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
    /// Set only for `exclusive_workspace` starts: the process holds the shared
    /// workspace write lease until it exits.
    #[serde(skip)]
    pub shared_workspace: Option<crate::RuntimeHandle>,
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
    shared.release_shared_write();
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


#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::supervisor::{ProcessIdentity, ProcessLaunchState};
    use std::sync::Barrier;

    #[test]
    fn exit_status_is_not_published_until_shared_write_is_released() {
        let runtime = crate::RuntimeHandle::new(
            crate::RunId::new(),
            crate::AgentId::new(),
            crate::RuntimeBus::new(),
        );
        let lease = runtime.team.acquire_shared_write(None).unwrap();
        lease.hand_to_job(1, "test process");
        let shared = Arc::new(Shared {
            status: Mutex::new(JobStatus::Running),
            workspace_write: Mutex::new((false, Some(lease))),
            output: Mutex::new(OutputBuffer::default()),
            child: Mutex::new(None),
            stdin: Mutex::new(None),
            finished_at: Mutex::new(None),
            supports_stdin: false,
            supervisor: Mutex::new(None),
        });
        // Hold the release lock to expose the terminal-status publication race.
        let held_release = shared.workspace_write.lock().unwrap();
        let started = Arc::new(Barrier::new(2));
        let callback_shared = Arc::clone(&shared);
        let callback_started = Arc::clone(&started);
        let finisher = std::thread::spawn(move || {
            callback_started.wait();
            finish_shared(
                &callback_shared,
                &ProcessExit {
                    identity: ProcessIdentity {
                        operation: None,
                        lifetime: uuid::Uuid::new_v4(),
                    },
                    launch_state: ProcessLaunchState::Exited,
                    sandbox: None,
                    code: Some(0),
                    stopped: false,
                    error: None,
                    output_complete: true,
                },
            );
        });
        started.wait();
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(
            *shared.status.lock().unwrap(),
            JobStatus::Running,
            "terminal job status was visible while workspace release was blocked"
        );

        drop(held_release);
        finisher.join().unwrap();
        assert_eq!(*shared.status.lock().unwrap(), JobStatus::Exited(0));
        assert!(runtime.team.acquire_shared_write(None).is_ok());
    }
}
