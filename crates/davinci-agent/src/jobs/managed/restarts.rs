//! Bounded reconciliation of one JobBook record, never a second process table.
use super::*;
use serde::{Deserialize, Serialize};
use std::sync::Weak;

pub type RestartCheck = Arc<dyn Fn() -> Result<(), String> + Send + Sync>;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RestartPolicy {
    pub max_restarts: u8,
    pub backoff_ms: u64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct StartOptions {
    pub restart: RestartPolicy,
    pub ports: Vec<u16>,
    #[serde(skip)]
    pub counters: Arc<crate::SharedCounters>,
    #[serde(skip)]
    pub provenance: Provenance,
}

/// Host-provided correlation, never accepted from model arguments or used as authority.
#[derive(Clone, Debug, Default, Serialize)]
pub struct Provenance {
    pub session_id: Option<String>,
    pub agent_id: Option<crate::AgentId>,
    pub task_id: Option<crate::TaskId>,
    pub graph_node: Option<String>,
}

impl StartOptions {
    pub fn validate(&self) -> Result<(), String> {
        if self.restart.max_restarts > 3
            || self.restart.max_restarts > 0 && !(50..=5000).contains(&self.restart.backoff_ms)
            || self.restart.backoff_ms > 5000
            || self.ports.len() > 8
            || self.ports.contains(&0)
            || self.ports.iter().collect::<BTreeSet<_>>().len() != self.ports.len()
        {
            return Err("invalid restart policy or declared ports".into());
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub(super) struct RestartState {
    pub attempts: u8,
    pub error: Option<String>,
    #[serde(skip)]
    pub terminal: bool,
    #[serde(skip)]
    pub backoff: bool,
}

pub(super) fn callback(
    shared: Weak<Shared>,
    restarting: bool,
) -> Arc<dyn Fn(ProcessEvent) + Send + Sync> {
    Arc::new(move |event| {
        let Some(shared) = shared.upgrade() else {
            return;
        };
        match event {
            ProcessEvent::Output(bytes) => shared
                .output
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .append(&bytes),
            ProcessEvent::Finished(exit) if !restarting => finish_shared(&shared, &exit),
            ProcessEvent::Finished(_) => {}
        }
    })
}

fn finish_shared(shared: &Shared, exit: &ProcessExit) {
    *shared.status.lock().unwrap_or_else(|e| e.into_inner()) = if exit.stopped {
        JobStatus::Killed
    } else {
        JobStatus::Exited(exit.code.unwrap_or(-1))
    };
    *shared.finished_at.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
}

fn finish(shared: &Shared, record: &Record, mut exit: ProcessExit) {
    exit.stopped |= record.stopped.load(Ordering::SeqCst);
    record
        .restart
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .terminal = true;
    finish_shared(shared, &exit);
}

fn cancelled(scope: &Scope, record: &Record) -> bool {
    let unowned = record
        .owners
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .active
        .is_empty();
    scope.closed.load(Ordering::SeqCst)
        || record.stopped.load(Ordering::SeqCst)
        || unowned
        || ensure_active(
            &scope.jobs.lock().unwrap_or_else(|e| e.into_inner()),
            &AtomicBool::new(false),
        )
        .is_err()
}

fn authorize(scope: &Scope, record: &Record, validate: &RestartCheck) -> Result<(), String> {
    if cancelled(scope, record) {
        return Err("process restart cancelled".into());
    }
    if record.command.cwd.canonicalize().ok().as_ref() != Some(&record.command.cwd)
        || scope.workspace.canonicalize().ok().as_ref() != Some(&scope.workspace)
    {
        return Err("process workspace changed before restart".into());
    }
    validate()
}

pub(super) fn supervise(
    scope: Weak<Scope>,
    shared: Weak<Shared>,
    record: Weak<Record>,
    id: u32,
    validate: RestartCheck,
) {
    std::thread::spawn(move || loop {
        let (Some(scope), Some(shared), Some(record)) =
            (scope.upgrade(), shared.upgrade(), record.upgrade())
        else {
            return;
        };
        let Some(current) = shared
            .supervisor
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
        else {
            return;
        };
        if cancelled(&scope, &record) {
            record.stopped.store(true, Ordering::SeqCst);
            current.stop();
        }
        let Some(exit) = current.wait(Duration::from_millis(20)) else {
            continue;
        };
        if exit.stopped
            || exit.code == Some(0)
            || cancelled(&scope, &record)
            || record
                .restart
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .attempts
                >= record.options.restart.max_restarts
        {
            finish(&shared, &record, exit);
            return;
        }
        record
            .restart
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .backoff = true;
        let deadline = Instant::now() + Duration::from_millis(record.options.restart.backoff_ms);
        loop {
            if let Err(error) = authorize(&scope, &record, &validate) {
                record
                    .restart
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .error = Some(error.to_string());
                finish(&shared, &record, exit);
                return;
            }
            if Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        record
            .restart
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .attempts += 1;
        let next = match Supervisor::spawn(
            &scope.host,
            record.command.clone(),
            callback(Arc::downgrade(&shared), true),
        ) {
            Ok(next) => Arc::new(next),
            Err(error) => {
                record
                    .restart
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .error = Some(error.to_string());
                finish(&shared, &record, exit);
                return;
            }
        };
        crate::SharedCounters::add(&record.options.counters.process_startups, 1);
        crate::SharedCounters::add(&record.options.counters.process_restarts, 1);
        // A stop/revocation during spawn must also stop the newly owned lifetime.
        if let Err(error) = authorize(&scope, &record, &validate) {
            record.stopped.store(true, Ordering::SeqCst);
            record
                .restart
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .error = Some(error);
        }
        {
            let owners = record.owners.lock().unwrap_or_else(|e| e.into_inner());
            if owners.active.is_empty() {
                record.stopped.store(true, Ordering::SeqCst);
            }
            *shared.supervisor.lock().unwrap_or_else(|e| e.into_inner()) = Some(next.clone());
        }
        if record.stopped.load(Ordering::SeqCst) || scope.closed.load(Ordering::SeqCst) {
            next.stop();
        }
        record
            .restart
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .backoff = false;
        if let Some(job) = scope
            .jobs
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .jobs
            .iter_mut()
            .find(|job| job.id == id)
        {
            job.pid = next.child_pid();
        };
    });
}
