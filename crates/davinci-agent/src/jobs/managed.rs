//! JobBook's supervised ownership primitive for trusted hosts.
//! This is not a tool dispatcher: callers must authorize every operation first.
//! Model arguments must never construct owners, scopes, or supervisor commands.
mod restarts;
use super::{
    supervisor::{ProcessConfig, ProcessEvent, ProcessExit, Supervisor, SupervisorCommand},
    Job, JobBook, JobStatus, OutputBuffer, Shared, LIVE_JOBS,
};
pub use restarts::{Provenance, RestartCheck, RestartPolicy, StartOptions};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeSet, HashMap},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Condvar, Mutex, Weak,
    },
    time::{Duration, Instant},
};
use uuid::Uuid;

const MAX_RUNNING: usize = 16;
const MAX_RECORDS: usize = 64;
const MAX_LEASES: usize = 64;

pub(super) struct Record {
    key: String,
    scope: Uuid,
    owners: Mutex<Owners>,
    command: ProcessConfig,
    environment_digest: String,
    started_ms: u64,
    options: StartOptions,
    stopped: AtomicBool,
    restart: Mutex<restarts::RestartState>,
}

impl Record {
    pub(super) fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
    }
}

#[derive(Default)]
struct Owners {
    known: BTreeSet<Uuid>,
    active: BTreeSet<Uuid>,
}

#[derive(Default)]
pub(super) struct Flight {
    result: Mutex<Option<Result<u32, String>>>,
    changed: Condvar,
    waiters: AtomicUsize,
}

struct Leader<'a> {
    scope: &'a Scope,
    key: String,
    flight: Arc<Flight>,
}

impl Leader<'_> {
    fn finish(&self, result: Result<u32, String>) {
        let mut book = self.scope.jobs.lock().unwrap_or_else(|e| e.into_inner());
        // Remove only our reservation, including during panic unwinding.
        if book
            .managed_starts
            .get(&self.key)
            .is_some_and(|flight| Arc::ptr_eq(flight, &self.flight))
        {
            book.managed_starts.remove(&self.key);
            *self.flight.result.lock().unwrap_or_else(|e| e.into_inner()) = Some(result);
            self.flight.changed.notify_all();
        }
    }
}

impl Drop for Leader<'_> {
    fn drop(&mut self) {
        self.finish(Err("process startup was interrupted".into()));
    }
}

struct Waiter<'a>(&'a AtomicUsize);
impl Drop for Waiter<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

struct Scope {
    id: Uuid,
    closed: Arc<AtomicBool>,
    workspaces: Arc<Mutex<HashMap<PathBuf, Weak<Scope>>>>,
    workspace: PathBuf,
    jobs: Arc<Mutex<JobBook>>,
    host: SupervisorCommand,
}

struct Owner {
    id: Uuid,
    scope: Arc<Scope>,
}

/// A session owner or a child lease explicitly issued by that owner.
#[derive(Clone)]
pub struct ManagedOwner(Arc<Owner>);

/// Host-only lifetime observation. It neither grants execution authority nor
/// keeps the owner (and its processes) alive after session/worker teardown.
#[derive(Clone)]
pub struct ManagedProcessLease {
    owner: Weak<Owner>,
    process_id: u32,
    lifetime: Uuid,
}

impl std::fmt::Debug for ManagedProcessLease {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManagedProcessLease")
            .field("process_id", &self.process_id)
            .field("lifetime", &self.lifetime)
            .finish_non_exhaustive()
    }
}

impl PartialEq for ManagedProcessLease {
    fn eq(&self, other: &Self) -> bool {
        self.owner.ptr_eq(&other.owner)
            && self.process_id == other.process_id
            && self.lifetime == other.lifetime
    }
}
impl Eq for ManagedProcessLease {}

impl ManagedProcessLease {
    pub fn is_live(&self) -> bool {
        self.owner.upgrade().is_some_and(|owner| {
            ManagedOwner(owner)
                .active_snapshot(self.process_id)
                .is_ok_and(|snapshot| snapshot.lifetime == self.lifetime)
        })
    }
}

impl std::fmt::Debug for ManagedOwner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ManagedOwner")
            .field("owner", &self.0.id)
            .field("workspace", &self.0.scope.workspace)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Serialize)]
pub struct ProcessSnapshot {
    pub id: u32,
    pub owner: Uuid,
    pub session: Uuid,
    pub origin: Provenance,
    pub workspace: PathBuf,
    pub executable: PathBuf,
    pub argv: Vec<String>,
    pub cwd: PathBuf,
    pub environment_digest: String,
    pub environment_names: Vec<String>,
    pub pid: u32,
    pub lifetime: Uuid,
    pub started_ms: u64,
    pub state: String,
    pub exit_code: Option<i32>,
    pub error: Option<String>,
    pub output_complete: Option<bool>,
    pub active_leases: usize,
    pub restart: serde_json::Value,
    pub ports: Vec<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct OutputPage {
    pub text: String,
    pub earliest_cursor: u64,
    pub next_cursor: u64,
    pub total_bytes: u64,
    pub remaining_bytes: u64,
    pub truncated: bool,
}

impl ManagedOwner {
    pub fn new(
        workspace: &Path,
        jobs: Arc<Mutex<JobBook>>,
        host: SupervisorCommand,
    ) -> Result<Self, String> {
        let workspace = workspace
            .canonicalize()
            .map_err(|_| "process workspace is unavailable")?;
        if !workspace.is_dir() {
            return Err("process workspace is not a directory".into());
        }
        let scope = Arc::new(Scope {
            id: Uuid::new_v4(),
            closed: Arc::new(AtomicBool::new(false)),
            workspaces: Arc::new(Mutex::new(HashMap::new())),
            workspace,
            jobs,
            host,
        });
        scope
            .workspaces
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(scope.workspace.clone(), Arc::downgrade(&scope));
        Ok(Self(Arc::new(Owner {
            id: Uuid::new_v4(),
            scope,
        })))
    }

    /// Trusted parent capability. A worker receives this lease, never its parent
    /// owner ID. Releasing it cannot stop another live lease's process.
    pub fn child_lease(&self) -> Self {
        Self(Arc::new(Owner {
            id: Uuid::new_v4(),
            scope: self.0.scope.clone(),
        }))
    }

    /// Host-selected workspace view in the same session and job book.
    /// Weak views keep same-workspace restart supervision shared without
    /// retaining departed workers or granting access to another owner's jobs.
    pub fn child_lease_for_workspace(&self, workspace: &Path) -> Result<Self, String> {
        self.ensure_open()?;
        let workspace = workspace
            .canonicalize()
            .map_err(|_| "process workspace unavailable")?;
        if !workspace.is_dir() {
            return Err("process workspace is not a directory".into());
        }
        let parent = &self.0.scope;
        let mut views = parent.workspaces.lock().unwrap_or_else(|e| e.into_inner());
        views.retain(|_, scope| scope.strong_count() > 0);
        let scope = match views.get(&workspace).and_then(Weak::upgrade) {
            Some(scope) => scope,
            None => {
                if views.len() >= MAX_LEASES {
                    return Err("process workspace limit reached".into());
                }
                let scope = Arc::new(Scope {
                    id: parent.id,
                    closed: parent.closed.clone(),
                    workspaces: parent.workspaces.clone(),
                    workspace: workspace.clone(),
                    jobs: parent.jobs.clone(),
                    host: parent.host.clone(),
                });
                views.insert(workspace, Arc::downgrade(&scope));
                scope
            }
        };
        Ok(Self(Arc::new(Owner {
            id: Uuid::new_v4(),
            scope,
        })))
    }

    pub fn id(&self) -> Uuid {
        self.0.id
    }

    /// End the parent session, including child leases still held by workers.
    pub fn shutdown(&self) {
        self.0.scope.closed.store(true, Ordering::SeqCst);
        let book = self.0.scope.jobs.lock().unwrap_or_else(|e| e.into_inner());
        for job in &book.jobs {
            if job
                .managed
                .as_ref()
                .is_some_and(|record| record.scope == self.0.scope.id)
            {
                job.kill();
            }
        }
    }

    pub fn new_session(&self) -> Result<Self, String> {
        let next = Self::new(
            &self.0.scope.workspace,
            self.0.scope.jobs.clone(),
            self.0.scope.host.clone(),
        )?;
        self.shutdown();
        Ok(next)
    }

    pub(crate) fn ensure_open(&self) -> Result<(), String> {
        if self.0.scope.closed.load(Ordering::SeqCst) {
            Err("managed process session ended".into())
        } else {
            Ok(())
        }
    }

    /// `validate` rechecks request authority and cancellation outside book locks.
    /// The revision participates in reuse; the callback must reject a stale one.
    pub fn start_authorized(
        &self,
        config: ProcessConfig,
        revision: u64,
        abort: &AtomicBool,
        validate: impl Fn() -> Result<(), String>,
    ) -> Result<(u32, bool), String> {
        self.start_inner(
            config,
            revision,
            abort,
            StartOptions::default(),
            &validate,
            None,
        )
    }

    pub fn start_with_options(
        &self,
        config: ProcessConfig,
        revision: u64,
        abort: &AtomicBool,
        options: StartOptions,
        validate: impl Fn() -> Result<(), String>,
        restart_check: RestartCheck,
    ) -> Result<(u32, bool), String> {
        self.start_inner(
            config,
            revision,
            abort,
            options,
            &validate,
            Some(restart_check),
        )
    }

    fn start_inner(
        &self,
        mut config: ProcessConfig,
        revision: u64,
        abort: &AtomicBool,
        options: StartOptions,
        validate: &dyn Fn() -> Result<(), String>,
        restart_check: Option<RestartCheck>,
    ) -> Result<(u32, bool), String> {
        options.validate()?;
        self.ensure_open()?;
        validate()?;
        if abort.load(Ordering::SeqCst) {
            return Err("process startup cancelled".into());
        }
        config.cwd = config
            .cwd
            .canonicalize()
            .map_err(|_| "process cwd is unavailable")?;
        if !config.cwd.starts_with(&self.0.scope.workspace) {
            return Err("process cwd is outside its workspace".into());
        }
        let identity = serde_json::to_vec(&(
            &config,
            revision,
            self.0.scope.id,
            &self.0.scope.workspace,
            &options,
        ))
        .map_err(|_| "invalid command identity")?;
        let key = format!("{:x}", Sha256::digest(identity));
        let (flight, leader) = {
            let mut book = self.0.scope.jobs.lock().unwrap_or_else(|e| e.into_inner());
            self.ensure_open()?;
            ensure_active(&book, abort)?;
            for job in &book.jobs {
                if job.status().is_running()
                    && job.managed.as_ref().is_some_and(|record| record.key == key)
                    && acquire_live(job, self.0.id)?
                {
                    crate::SharedCounters::add(&options.counters.process_reuses, 1);
                    return Ok((job.id, true));
                }
            }
            if let Some(flight) = book.managed_starts.get(&key) {
                (flight.clone(), false)
            } else {
                if book.managed_starts.len()
                    + book
                        .jobs
                        .iter()
                        .filter(|job| job.managed.is_some() && job.status().is_running())
                        .count()
                    >= MAX_RUNNING
                {
                    return Err("managed process limit reached".into());
                }
                let flight = Arc::new(Flight::default());
                book.managed_starts.insert(key.clone(), flight.clone());
                (flight, true)
            }
        };
        if !leader {
            let count = flight.waiters.fetch_add(1, Ordering::SeqCst);
            let _waiter = Waiter(&flight.waiters);
            if count >= 32 {
                return Err("process startup waiter limit reached".into());
            }
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                self.ensure_open()?;
                validate()?;
                if abort.load(Ordering::SeqCst) {
                    return Err("process startup cancelled".into());
                }
                let result = flight
                    .result
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .clone();
                if let Some(result) = result {
                    let id = result?;
                    let book = self.0.scope.jobs.lock().unwrap_or_else(|e| e.into_inner());
                    ensure_active(&book, abort)?;
                    let job = book.get(id).ok_or("process startup record expired")?;
                    match job.status() {
                        JobStatus::Running if acquire_live(job, self.0.id)? => {}
                        JobStatus::Exited(_) => {
                            let record =
                                job.managed.as_ref().ok_or("process owner is unavailable")?;
                            let mut owners =
                                record.owners.lock().unwrap_or_else(|e| e.into_inner());
                            acquire_locked(&mut owners, self.0.id)?;
                            owners.active.remove(&self.0.id);
                        }
                        _ => return Err("process was stopped during startup".into()),
                    }
                    crate::SharedCounters::add(&options.counters.process_reuses, 1);
                    return Ok((id, true));
                }
                if Instant::now() >= deadline {
                    return Err("process startup wait timed out".into());
                }
                let guard = flight.result.lock().unwrap_or_else(|e| e.into_inner());
                let _ = flight
                    .changed
                    .wait_timeout(guard, Duration::from_millis(20));
            }
        }
        let leader = Leader {
            scope: &self.0.scope,
            key: key.clone(),
            flight,
        };
        let result = self.spawn_reserved(config, &key, abort, validate, options, restart_check);
        leader.finish(result.clone());
        result.map(|id| (id, false))
    }

    fn spawn_reserved(
        &self,
        config: ProcessConfig,
        key: &str,
        abort: &AtomicBool,
        validate: &dyn Fn() -> Result<(), String>,
        options: StartOptions,
        restart_check: Option<RestartCheck>,
    ) -> Result<u32, String> {
        validate()?;
        if abort.load(Ordering::SeqCst) {
            return Err("process startup cancelled".into());
        }
        let shared = Arc::new(Shared {
            status: Mutex::new(JobStatus::Running),
            output: Mutex::new(OutputBuffer::default()),
            child: Mutex::new(None),
            stdin: Mutex::new(None),
            finished_at: Mutex::new(None),
            supports_stdin: true,
            supervisor: Mutex::new(None),
        });
        let supervisor = Arc::new(Supervisor::spawn(
            &self.0.scope.host,
            config.clone(),
            restarts::callback(Arc::downgrade(&shared), options.restart.max_restarts > 0),
        )?);
        crate::SharedCounters::add(&options.counters.process_startups, 1);
        // Unpublished ownership is dropped (and stopped) on every failure path.
        validate()?;
        let mut book = self.0.scope.jobs.lock().unwrap_or_else(|e| e.into_inner());
        self.ensure_open()?;
        ensure_active(&book, abort)?;
        book.next_id = book.next_id.checked_add(1).ok_or("job IDs exhausted")?;
        let id = book.next_id;
        if book.jobs.iter().filter(|job| job.managed.is_some()).count() >= MAX_RECORDS {
            if let Some(index) = book
                .jobs
                .iter()
                .position(|job| job.managed.is_some() && !job.status().is_running())
            {
                book.jobs.remove(index);
            }
        }
        *shared.supervisor.lock().unwrap_or_else(|e| e.into_inner()) = Some(supervisor.clone());
        let record = Arc::new(Record {
            key: key.into(),
            scope: self.0.scope.id,
            environment_digest: format!(
                "{:x}",
                Sha256::digest(
                    serde_json::to_vec(&config.environment).map_err(|_| "invalid environment")?
                )
            ),
            command: config,
            owners: Mutex::new(Owners::default()),
            started_ms: davinci_session::now_ms(),
            options,
            stopped: AtomicBool::new(false),
            restart: Mutex::new(restarts::RestartState::default()),
        });
        acquire(&record, self.0.id)?;
        let mut live = LIVE_JOBS.lock().unwrap_or_else(|e| e.into_inner());
        live.retain(|(_, weak)| weak.strong_count() > 0);
        live.push((supervisor.child_pid(), Arc::downgrade(&shared)));
        drop(live);
        book.jobs.push(Job {
            id,
            command: format!("managed process {id}"),
            pid: supervisor.child_pid(),
            started: Instant::now(),
            task_id: None,
            agent_id: None,
            generation: None,
            shared: shared.clone(),
            managed: Some(record.clone()),
            announced: false,
            seen: false,
        });
        if record.options.restart.max_restarts > 0 {
            restarts::supervise(
                Arc::downgrade(&self.0.scope),
                Arc::downgrade(&shared),
                Arc::downgrade(&record),
                id,
                restart_check.expect("restart authority supplied"),
            );
        }
        Ok(id)
    }

    fn owned(&self, id: u32) -> Result<(Arc<Shared>, Arc<Record>), String> {
        self.ensure_open()?;
        let book = self.0.scope.jobs.lock().unwrap_or_else(|e| e.into_inner());
        let job = book.get(id).ok_or("unknown managed process")?;
        let record = job
            .managed
            .as_ref()
            .filter(|record| record.scope == self.0.scope.id)
            .ok_or("managed process belongs to another owner")?;
        if !record
            .owners
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .known
            .contains(&self.0.id)
        {
            return Err("managed process lease is not owned by this caller".into());
        }
        Ok((job.shared.clone(), record.clone()))
    }

    pub fn snapshot(&self, id: u32) -> Result<ProcessSnapshot, String> {
        let (shared, record) = self.owned(id)?;
        let supervisor = shared
            .supervisor
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .ok_or("process lifetime missing")?;
        let exit = supervisor.wait(Duration::ZERO);
        let restart = record
            .restart
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        let restarting = record.options.restart.max_restarts > 0 && !restart.terminal;
        let state = if restarting && (restart.backoff || exit.is_some()) {
            "restarting"
        } else if exit.is_some() {
            "exited"
        } else if supervisor.is_stopping() {
            "stopping"
        } else {
            "running"
        }
        .to_string();
        let active_leases = record
            .owners
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .active
            .len();
        Ok(ProcessSnapshot {
            id,
            owner: self.0.id,
            session: self.0.scope.id,
            origin: record.options.provenance.clone(),
            workspace: self.0.scope.workspace.clone(),
            executable: record.command.executable.clone(),
            argv: record.command.argv.clone(),
            cwd: record.command.cwd.clone(),
            environment_digest: record.environment_digest.clone(),
            environment_names: record.command.environment.keys().cloned().collect(),
            pid: supervisor.child_pid(),
            lifetime: supervisor.identity(),
            started_ms: record.started_ms,
            state,
            exit_code: exit.as_ref().and_then(|exit| exit.code),
            output_complete: exit.as_ref().map(|exit| exit.output_complete),
            error: exit.and_then(|exit| exit.error),
            active_leases,
            restart: serde_json::json!({"max_restarts":record.options.restart.max_restarts, "backoff_ms":record.options.restart.backoff_ms, "attempts":restart.attempts, "error":restart.error}),
            ports: record
                .options
                .ports
                .iter()
                .map(|port| serde_json::json!({"port":port,"source":"request","verified":false}))
                .collect(),
        })
    }

    /// Host resource attachment requires this caller's active lease. Historical
    /// status access remains available after release through `snapshot`.
    pub fn active_snapshot(&self, id: u32) -> Result<ProcessSnapshot, String> {
        let (_, record) = self.owned(id)?;
        if !record
            .owners
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .active
            .contains(&self.0.id)
        {
            return Err("managed process lease has been released".into());
        }
        let snapshot = self.snapshot(id)?;
        if snapshot.state != "running" {
            return Err("managed process is not running".into());
        }
        Ok(snapshot)
    }

    pub fn active_lease(&self, id: u32) -> Result<(ProcessSnapshot, ManagedProcessLease), String> {
        let snapshot = self.active_snapshot(id)?;
        let lease = ManagedProcessLease {
            owner: Arc::downgrade(&self.0),
            process_id: id,
            lifetime: snapshot.lifetime,
        };
        Ok((snapshot, lease))
    }

    pub fn ids(&self) -> Vec<u32> {
        let book = self.0.scope.jobs.lock().unwrap_or_else(|e| e.into_inner());
        book.jobs
            .iter()
            .filter(|job| {
                job.managed.as_ref().is_some_and(|record| {
                    record.scope == self.0.scope.id
                        && record
                            .owners
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .known
                            .contains(&self.0.id)
                })
            })
            .map(|job| job.id)
            .collect()
    }

    pub fn output(&self, id: u32, cursor: Option<u64>, limit: usize) -> Result<OutputPage, String> {
        let (shared, _) = self.owned(id)?;
        let buffer = shared.output.lock().unwrap_or_else(|e| e.into_inner());
        let earliest_cursor = buffer.total_bytes.saturating_sub(buffer.bytes.len() as u64);
        let requested = cursor.unwrap_or(earliest_cursor);
        if requested > buffer.total_bytes || limit == 0 || limit > 64 * 1024 {
            return Err("invalid process output cursor or limit".into());
        }
        let start = requested.max(earliest_cursor);
        let offset = (start - earliest_cursor) as usize;
        let end = buffer.bytes.len().min(offset.saturating_add(limit));
        let next_cursor = earliest_cursor + end as u64;
        Ok(OutputPage {
            text: String::from_utf8_lossy(&buffer.bytes[offset..end]).into(),
            earliest_cursor,
            next_cursor,
            total_bytes: buffer.total_bytes,
            remaining_bytes: buffer.total_bytes - next_cursor,
            truncated: requested < earliest_cursor,
        })
    }

    pub fn write(&self, id: u32, bytes: &[u8]) -> Result<usize, String> {
        let (shared, record) = self.owned(id)?;
        if !record
            .owners
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .active
            .contains(&self.0.id)
        {
            return Err("process lease was released".into());
        }
        let supervisor = shared
            .supervisor
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .ok_or("process lifetime missing")?;
        supervisor.write(bytes)
    }

    pub fn release(&self, id: u32) -> Result<(), String> {
        let (shared, record) = self.owned(id)?;
        let mut owners = record.owners.lock().unwrap_or_else(|e| e.into_inner());
        owners.active.remove(&self.0.id);
        if owners.active.is_empty() {
            record.stop();
            if let Some(supervisor) = shared
                .supervisor
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_ref()
            {
                supervisor.stop();
            }
        }
        Ok(())
    }

    pub fn wait(&self, id: u32, timeout: Duration) -> Result<Option<ProcessExit>, String> {
        let (shared, record) = self.owned(id)?;
        let started = Instant::now();
        // Reconciliation owns the logical lifetime. An individual child exiting
        // during backoff is not completion of a restart-enabled record.
        while record.options.restart.max_restarts > 0
            && !record
                .restart
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .terminal
        {
            let remaining = timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                return Ok(None);
            }
            std::thread::sleep(remaining.min(Duration::from_millis(20)));
        }
        let supervisor = shared
            .supervisor
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
            .ok_or("process lifetime missing")?;
        Ok(supervisor.wait(timeout.saturating_sub(started.elapsed())))
    }
}

fn acquire(record: &Record, owner: Uuid) -> Result<(), String> {
    let mut owners = record.owners.lock().unwrap_or_else(|e| e.into_inner());
    acquire_locked(&mut owners, owner)
}

fn acquire_live(job: &Job, owner: Uuid) -> Result<bool, String> {
    let record = job
        .managed
        .as_ref()
        .ok_or("managed process metadata missing")?;
    let mut owners = record.owners.lock().unwrap_or_else(|e| e.into_inner());
    // This lock also guards final release: a stopped lifetime cannot acquire
    // another owner between checking stop and inserting the lease.
    if record.stopped.load(Ordering::SeqCst) {
        return Ok(false);
    }
    let restarting = record.options.restart.max_restarts > 0
        && !record
            .restart
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .terminal;
    if !restarting
        && job
            .shared
            .supervisor
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .is_none_or(|supervisor| supervisor.is_stopping())
    {
        return Ok(false);
    }
    acquire_locked(&mut owners, owner)?;
    Ok(true)
}

fn acquire_locked(owners: &mut Owners, owner: Uuid) -> Result<(), String> {
    if !owners.known.contains(&owner) && owners.known.len() >= MAX_LEASES {
        return Err("process lease limit reached".into());
    }
    owners.known.insert(owner);
    owners.active.insert(owner);
    Ok(())
}

fn ensure_active(book: &JobBook, abort: &AtomicBool) -> Result<(), String> {
    if abort.load(Ordering::SeqCst)
        || book
            .cancellation_owner
            .as_ref()
            .and_then(std::sync::Weak::upgrade)
            .is_some_and(|flag| flag.load(Ordering::SeqCst))
    {
        Err("process startup cancelled".into())
    } else {
        Ok(())
    }
}

impl Drop for Owner {
    fn drop(&mut self) {
        let book = self.scope.jobs.lock().unwrap_or_else(|e| e.into_inner());
        for job in &book.jobs {
            if let Some(record) = &job.managed {
                if record.scope == self.scope.id {
                    let mut owners = record.owners.lock().unwrap_or_else(|e| e.into_inner());
                    owners.active.remove(&self.id);
                    if owners.active.is_empty() {
                        job.kill();
                    }
                }
            }
        }
    }
}
