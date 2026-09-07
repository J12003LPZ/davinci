//! Native-only session-owned lifecycle. Work must execute outside host locks.

use super::types::{RunProgress, RunStatus};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};

#[derive(Debug, Clone, Default)]
pub struct ScanCoordinator {
    current: Arc<Mutex<Option<RunHandle>>>,
}

#[derive(Debug, Clone)]
pub struct RunHandle {
    state: Arc<(Mutex<RunProgress>, Condvar)>,
    abort: Arc<AtomicBool>,
    deadline: Arc<Mutex<Option<std::time::Instant>>>,
    publishing: Arc<AtomicBool>,
    budget: Arc<Mutex<Option<super::budget::RequestBudget>>>,
    partial_report: Arc<Mutex<Option<serde_json::Value>>>,
}

impl RunHandle {
    pub fn set_partial_report(&self, value: serde_json::Value) -> Result<(), String> {
        let state = self.state.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.status.terminal()
            || value["scanId"] != state.scan_id
            || value["generation"] != state.generation
        {
            return Err("partial report belongs to an inactive generation".into());
        }
        *self
            .partial_report
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = Some(value);
        Ok(())
    }

    pub fn partial_report(&self) -> Option<serde_json::Value> {
        let mut value = self
            .partial_report
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()?;
        let status = self.status();
        value["status"] = serde_json::json!(status.status);
        if let Some(limitations) = value["limitations"].as_array_mut() {
            limitations.extend(
                status
                    .limitations
                    .into_iter()
                    .map(serde_json::Value::String),
            );
        }
        Some(value)
    }
    pub fn budget_summary(&self) -> serde_json::Value {
        self.budget
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map_or(
                serde_json::Value::Null,
                super::budget::RequestBudget::summary,
            )
    }
    pub fn bind_budget(
        &self,
        store: Option<super::store::Store>,
        turns: usize,
        tokens: u64,
        discovery_tokens: u64,
    ) -> Result<(), String> {
        let mut budget = self.budget.lock().unwrap_or_else(|e| e.into_inner());
        if budget.is_some() {
            return Err("scan budget already bound".into());
        }
        *budget = Some(super::budget::RequestBudget::open_scoped(
            store,
            self.status().scan_id,
            turns,
            tokens,
            discovery_tokens,
        )?);
        Ok(())
    }

    pub fn reserve_request(&self, tokens: u64) -> Result<Option<usize>, String> {
        if self.cancelled() {
            return Err("security review cancelled".into());
        }
        if let Some(budget) = self
            .budget
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_mut()
        {
            let discovery = matches!(
                self.status().status,
                RunStatus::Mapping | RunStatus::Investigating
            );
            return budget.reserve_scoped(tokens, discovery).map(Some);
        }
        Ok(None)
    }

    pub fn settle_request(&self, ticket: Option<usize>, tokens: u64) -> Result<(), String> {
        if let Some(ticket) = ticket {
            self.budget
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .as_mut()
                .ok_or("scan budget unavailable")?
                .settle(ticket, tokens)?;
        }
        Ok(())
    }
    pub fn record_usage(&self, usage: super::usage::RequestUsage) -> u64 {
        let accounted = usage.accounted_tokens;
        self.state
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .usage
            .push(usage);
        accounted
    }
    pub fn begin_publication(&self) -> Result<(), String> {
        let state = self.state.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.status != RunStatus::Reporting || self.cancelled() {
            return Err("security publication cancelled or out of order".into());
        }
        self.publishing.store(true, Ordering::Release);
        Ok(())
    }
    pub fn set_generation(&self, generation: u64) -> Result<(), String> {
        let mut state = self.state.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.status != RunStatus::Preflight {
            return Err("generation is already bound".into());
        }
        state.generation = generation;
        Ok(())
    }
    pub fn set_deadline(&self, duration: std::time::Duration) {
        *self.deadline.lock().unwrap_or_else(|e| e.into_inner()) =
            Some(std::time::Instant::now() + duration);
        let state = Arc::downgrade(&self.state);
        let abort = self.abort.clone();
        let publishing = self.publishing.clone();
        std::thread::spawn(move || {
            let until = std::time::Instant::now() + duration;
            loop {
                let Some(state) = state.upgrade() else {
                    break;
                };
                let mut progress = state.0.lock().unwrap_or_else(|e| e.into_inner());
                if progress.status.terminal() {
                    break;
                }
                if std::time::Instant::now() >= until && !publishing.load(Ordering::Acquire) {
                    abort.store(true, Ordering::Release);
                    progress.status = RunStatus::Cancelling;
                    break;
                }
                drop(progress);
                drop(state);
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        });
    }
    pub fn abort_signal(&self) -> Arc<AtomicBool> {
        self.abort.clone()
    }

    pub fn advance(&self, status: RunStatus) -> Result<(), String> {
        let mut state = self.state.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.status.terminal() || self.cancelled() {
            return Err("security run is no longer active".into());
        }
        if status.terminal() || status == RunStatus::Cancelling {
            return Err("terminal transitions require finalization".into());
        }
        state.status = status;
        Ok(())
    }
    pub fn status(&self) -> RunProgress {
        self.state
            .0
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    pub fn cancelled(&self) -> bool {
        if !self.publishing.load(Ordering::Acquire)
            && self
                .deadline
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .is_some_and(|deadline| std::time::Instant::now() >= deadline)
        {
            self.abort.store(true, Ordering::Release);
        }
        self.abort.load(Ordering::Acquire)
    }

    pub fn finish(&self, result: Result<bool, String>) {
        self.budget.lock().unwrap_or_else(|e| e.into_inner()).take();
        let mut state = self.state.0.lock().unwrap_or_else(|e| e.into_inner());
        if state.status.terminal() {
            return;
        }
        if self.cancelled() {
            state.status = RunStatus::Cancelled;
            state.coverage_complete = false;
        } else {
            match result {
                Ok(complete) => {
                    state.status = RunStatus::Completed;
                    state.coverage_complete = complete;
                }
                Err(error) => {
                    state.status = RunStatus::Failed;
                    state.limitations.push(error);
                }
            }
        }
        self.state.1.notify_all();
    }

    pub fn wait(&self) {
        let state = self.state.0.lock().unwrap_or_else(|e| e.into_inner());
        drop(
            self.state
                .1
                .wait_while(state, |state| !state.status.terminal())
                .unwrap_or_else(|e| e.into_inner()),
        );
    }
}

impl ScanCoordinator {
    pub fn partial_report(&self) -> Option<serde_json::Value> {
        self.current
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .and_then(RunHandle::partial_report)
    }
    pub fn wait(&self) {
        let run = self
            .current
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        if let Some(run) = run {
            run.wait();
        }
    }

    pub fn status(&self) -> Option<RunProgress> {
        self.current
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .as_ref()
            .map(RunHandle::status)
    }

    pub fn start<F>(&self, task: F) -> Result<RunHandle, String>
    where
        F: FnOnce(RunHandle) + Send + 'static,
    {
        self.start_generation(uuid::Uuid::new_v4().to_string(), 1, task)
    }

    pub fn start_generation<F>(
        &self,
        scan_id: String,
        generation: u64,
        task: F,
    ) -> Result<RunHandle, String>
    where
        F: FnOnce(RunHandle) + Send + 'static,
    {
        let mut current = self.current.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(active) = current.as_ref().map(RunHandle::status) {
            if !active.status.terminal() {
                return Err(format!(
                    "security scan {} is already active",
                    active.scan_id
                ));
            }
        }
        let run = RunHandle {
            state: Arc::new((
                Mutex::new(RunProgress {
                    schema_version: 2,
                    scan_id,
                    generation,
                    status: RunStatus::Preflight,
                    coverage_complete: false,
                    experimental: true,
                    limitations: Vec::new(),
                    usage: Vec::new(),
                }),
                Condvar::new(),
            )),
            abort: Arc::new(AtomicBool::new(false)),
            deadline: Default::default(),
            publishing: Arc::new(AtomicBool::new(false)),
            budget: Default::default(),
            partial_report: Default::default(),
        };
        let worker = run.clone();
        std::thread::Builder::new()
            .name("security-scan".into())
            .spawn(move || {
                let result =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| task(worker.clone())));
                if result.is_err() {
                    worker.finish(Err("security worker panicked".into()));
                } else if !worker.status().status.terminal() {
                    worker.finish(Err("worker returned without a terminal result".into()));
                }
            })
            .map_err(|_| "could not start security worker".to_string())?;
        *current = Some(run.clone());
        Ok(run)
    }

    pub fn abort(&self, scan_id: Option<&str>) -> Result<RunProgress, String> {
        let current = self.current.lock().unwrap_or_else(|e| e.into_inner());
        let run = current.as_ref().ok_or("no active security scan")?;
        let mut state = run.state.0.lock().unwrap_or_else(|e| e.into_inner());
        if scan_id.is_some_and(|id| id != state.scan_id) {
            return Err("scan identity does not match this session".into());
        }
        if !state.status.terminal() {
            if run.publishing.load(Ordering::Acquire) {
                return Err("immutable report publication has begun".into());
            }
            run.abort.store(true, Ordering::Release);
            state.status = RunStatus::Cancelling;
        }
        Ok(state.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;
    use std::time::Duration;

    #[test]
    fn security_scan_status_and_abort_do_not_wait_on_worker_lock() {
        let controller = ScanCoordinator::default();
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let handle = controller
            .start(move |run| {
                started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                run.finish(Ok(false));
            })
            .unwrap();
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(!controller.status().unwrap().status.terminal());
        controller.abort(Some(&handle.status().scan_id)).unwrap();
        assert!(handle.cancelled());
        release_tx.send(()).unwrap();
        handle.wait();
        assert_eq!(handle.status().status, RunStatus::Cancelled);
    }

    #[test]
    fn security_scan_same_session_conflict_is_explicit() {
        let controller = ScanCoordinator::default();
        let (tx, rx) = mpsc::channel();
        let handle = controller
            .start(move |run| {
                rx.recv().unwrap();
                run.finish(Ok(false));
            })
            .unwrap();
        let error = controller.start(|_| {}).unwrap_err();
        assert!(error.contains(&handle.status().scan_id));
        assert!(controller.abort(Some("other-run")).is_err());
        tx.send(()).unwrap();
        handle.wait();
        assert_eq!(handle.status().status, RunStatus::Completed);
        assert!(!handle.status().coverage_complete);
    }

    #[test]
    fn security_scan_worker_panic_is_not_success() {
        let controller = ScanCoordinator::default();
        let run = controller.start(|_| panic!("fixture crash")).unwrap();
        run.wait();
        assert_eq!(run.status().status, RunStatus::Failed);
    }

    #[test]
    fn security_complete_and_cancel_race_has_one_terminal_outcome() {
        let controller = ScanCoordinator::default();
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let handle = controller
            .start(move |run| {
                run.advance(RunStatus::Reporting).unwrap();
                run.begin_publication().unwrap();
                started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                run.finish(Ok(true));
            })
            .unwrap();
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(
            controller
                .abort(Some(&handle.status().scan_id))
                .unwrap_err(),
            "immutable report publication has begun"
        );
        assert!(!handle.status().status.terminal());
        release_tx.send(()).unwrap();
        handle.wait();
        assert_eq!(handle.status().status, RunStatus::Completed);
        assert!(handle.status().coverage_complete);
        assert_ne!(handle.status().status, RunStatus::Cancelled);
    }
}
