//! Parent-owned local memory reservations. These are estimates, not RSS caps.
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

/// Workers that may wait for capacity at once before the next is refused.
pub const DEFAULT_MAX_QUEUED_WORKERS: usize = 32;
/// How long a worker waits for capacity before it gives up.
pub const DEFAULT_WAIT_TIMEOUT_MS: u64 = 15 * 60 * 1000;

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkerResourceConfig {
    pub memory_budget_bytes: u64,
    pub estimated_worker_bytes: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_queued_workers: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub wait_timeout_ms: Option<u64>,
}
#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerResourceStats {
    pub memory_budget_bytes: Option<u64>,
    pub estimated_worker_bytes: u64,
    pub reserved_bytes: u64,
    pub waiting_workers: usize,
}
#[derive(Debug)]
struct Inner {
    error: Option<String>,
    limit: Option<u64>,
    estimate: u64,
    max_queued: usize,
    wait_timeout: Duration,
    state: Mutex<WorkerResourceStats>,
    changed: Condvar,
}
impl Default for Inner {
    fn default() -> Self {
        Self {
            error: None,
            limit: None,
            estimate: 0,
            max_queued: DEFAULT_MAX_QUEUED_WORKERS,
            wait_timeout: Duration::from_millis(DEFAULT_WAIT_TIMEOUT_MS),
            state: Mutex::default(),
            changed: Condvar::default(),
        }
    }
}
#[derive(Debug, Default, Clone)]
pub struct WorkerAdmission(Arc<Inner>);
impl WorkerAdmission {
    pub fn denied(error: String) -> Self {
        Self(Arc::new(Inner {
            error: Some(error),
            ..Default::default()
        }))
    }
    pub fn new(config: Option<WorkerResourceConfig>) -> Result<Self, String> {
        let Some(config) = config else {
            return Ok(Self::default());
        };
        if config.memory_budget_bytes == 0
            || config.estimated_worker_bytes == 0
            || config.estimated_worker_bytes > config.memory_budget_bytes
        {
            return Err(
                "workerResources requires positive estimatedWorkerBytes <= memoryBudgetBytes"
                    .into(),
            );
        }
        if config.wait_timeout_ms == Some(0) {
            return Err("workerResources waitTimeoutMs must be positive".into());
        }
        let defaults = Inner::default();
        Ok(Self(Arc::new(Inner {
            limit: Some(config.memory_budget_bytes),
            estimate: config.estimated_worker_bytes,
            max_queued: config.max_queued_workers.unwrap_or(defaults.max_queued),
            wait_timeout: config
                .wait_timeout_ms
                .map_or(defaults.wait_timeout, Duration::from_millis),
            ..defaults
        })))
    }
    pub fn stats(&self) -> WorkerResourceStats {
        let mut stats = self
            .0
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clone();
        stats.memory_budget_bytes = self.0.limit;
        stats.estimated_worker_bytes = self.0.estimate;
        stats
    }
    pub fn is_configured(&self) -> bool {
        self.0.limit.is_some() || self.0.error.is_some()
    }
    pub fn acquire_worker(&self, cancelled: impl Fn() -> bool) -> Result<WorkerLease, String> {
        self.acquire(self.0.estimate, cancelled)
    }
    /// A worker that already holds a parent reservation must not wait for
    /// capacity that can only be released when that same worker returns.
    pub fn acquire_worker_with_wait(
        &self,
        wait: bool,
        cancelled: impl Fn() -> bool,
    ) -> Result<WorkerLease, String> {
        self.acquire_inner(self.0.estimate, wait, cancelled)
    }
    pub fn acquire(&self, bytes: u64, cancelled: impl Fn() -> bool) -> Result<WorkerLease, String> {
        self.acquire_inner(bytes, true, cancelled)
    }
    fn acquire_inner(
        &self,
        bytes: u64,
        wait: bool,
        cancelled: impl Fn() -> bool,
    ) -> Result<WorkerLease, String> {
        if let Some(error) = &self.0.error {
            return Err(error.clone());
        }
        if self.0.limit.is_some_and(|limit| bytes > limit) {
            return Err("worker reservation exceeds local resource budget".into());
        }
        let bytes = if self.0.limit.is_some() { bytes } else { 0 };
        let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        let fits = |reserved: u64| {
            self.0
                .limit
                .is_none_or(|limit| reserved <= limit.saturating_sub(bytes))
        };
        if cancelled() {
            return Err("worker cancelled while waiting for local resources".into());
        }
        if fits(state.reserved_bytes) {
            return Ok(self.reserve(&mut state, bytes));
        }
        if !wait {
            return Err(
                "nested worker reservation cannot fit while its parent holds local resources"
                    .into(),
            );
        }
        if state.waiting_workers >= self.0.max_queued {
            return Err(format!(
                "worker admission queue is full: {} workers already wait for local resources \
                 (workerResources limits are estimates, not measured RSS)",
                self.0.max_queued
            ));
        }
        state.waiting_workers += 1;
        let deadline = Instant::now() + self.0.wait_timeout;
        loop {
            if cancelled() {
                state.waiting_workers -= 1;
                return Err("worker cancelled while waiting for local resources".into());
            }
            if fits(state.reserved_bytes) {
                state.waiting_workers -= 1;
                return Ok(self.reserve(&mut state, bytes));
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                state.waiting_workers -= 1;
                return Err(format!(
                    "worker timed out after {} ms waiting for local resources \
                     (workerResources limits are estimates, not measured RSS)",
                    self.0.wait_timeout.as_millis()
                ));
            }
            state = self
                .0
                .changed
                .wait_timeout(state, remaining.min(Duration::from_millis(50)))
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }
    fn reserve(&self, state: &mut WorkerResourceStats, bytes: u64) -> WorkerLease {
        state.reserved_bytes += bytes;
        WorkerLease {
            owner: self.clone(),
            bytes,
            alive: Arc::new(()),
        }
    }
}
#[derive(Debug, Clone, Default)]
pub struct WorkerLeaseContext(std::sync::Weak<()>);
impl WorkerLeaseContext {
    pub fn is_held(&self) -> bool {
        self.0.upgrade().is_some()
    }
}
#[derive(Debug)]
pub struct WorkerLease {
    owner: WorkerAdmission,
    bytes: u64,
    alive: Arc<()>,
}
impl WorkerLease {
    pub fn context(&self) -> WorkerLeaseContext {
        WorkerLeaseContext(Arc::downgrade(&self.alive))
    }
}
impl Drop for WorkerLease {
    fn drop(&mut self) {
        let mut state = self.owner.0.state.lock().unwrap_or_else(|e| e.into_inner());
        state.reserved_bytes = state.reserved_bytes.saturating_sub(self.bytes);
        self.owner.0.changed.notify_all();
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn budget() -> WorkerAdmission {
        WorkerAdmission::new(Some(WorkerResourceConfig {
            memory_budget_bytes: 8,
            estimated_worker_bytes: 8,
            ..Default::default()
        }))
        .unwrap()
    }
    fn bounded(max_queued_workers: usize, wait_timeout_ms: u64) -> WorkerAdmission {
        WorkerAdmission::new(Some(WorkerResourceConfig {
            memory_budget_bytes: 8,
            estimated_worker_bytes: 8,
            max_queued_workers: Some(max_queued_workers),
            wait_timeout_ms: Some(wait_timeout_ms),
        }))
        .unwrap()
    }
    fn wait_for_waiters(admission: &WorkerAdmission, count: usize) {
        let until = std::time::Instant::now() + Duration::from_secs(3);
        while admission.stats().waiting_workers < count {
            assert!(std::time::Instant::now() < until);
            std::thread::yield_now();
        }
    }
    #[test]
    fn a_single_worker_is_admitted_immediately_without_queueing() {
        let admission = bounded(0, 1);
        let lease = admission.acquire_worker(|| false).unwrap();
        assert_eq!(admission.stats().reserved_bytes, 8);
        assert_eq!(admission.stats().waiting_workers, 0);
        drop(lease);
    }
    #[test]
    fn default_bounds_are_32_waiters_and_15_minutes() {
        assert_eq!(DEFAULT_MAX_QUEUED_WORKERS, 32);
        assert_eq!(DEFAULT_WAIT_TIMEOUT_MS, 15 * 60 * 1000);
        let _ = budget();
    }
    #[test]
    fn a_full_queue_refuses_the_next_waiter_immediately() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let admission = bounded(2, 60_000);
        let held = admission.acquire_worker(|| false).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let waiters: Vec<_> = (0..2)
            .map(|_| {
                let (queued, flag) = (admission.clone(), cancel.clone());
                std::thread::spawn(move || queued.acquire_worker(|| flag.load(Ordering::Acquire)))
            })
            .collect();
        wait_for_waiters(&admission, 2);
        let started = std::time::Instant::now();
        let error = admission.acquire_worker(|| false).unwrap_err();
        assert!(started.elapsed() < Duration::from_secs(2), "must not wait");
        assert!(error.contains("queue is full"), "{error}");
        assert!(error.contains("estimate"), "{error}");
        assert_eq!(admission.stats().waiting_workers, 2);
        cancel.store(true, Ordering::Release);
        for waiter in waiters {
            assert!(waiter.join().unwrap().is_err());
        }
        drop(held);
        assert_eq!(admission.stats().reserved_bytes, 0);
        assert_eq!(admission.stats().waiting_workers, 0);
    }
    #[test]
    fn a_waiter_times_out_with_a_clear_error_and_leaks_nothing() {
        let admission = bounded(4, 100);
        let held = admission.acquire_worker(|| false).unwrap();
        let started = std::time::Instant::now();
        let error = admission.acquire_worker(|| false).unwrap_err();
        assert!(started.elapsed() >= Duration::from_millis(100));
        assert!(started.elapsed() < Duration::from_secs(5));
        assert!(error.contains("timed out"), "{error}");
        assert!(error.contains("not measured"), "{error}");
        assert_eq!(admission.stats().waiting_workers, 0);
        drop(held);
        assert!(admission.acquire_worker(|| false).is_ok());
    }
    #[test]
    fn cancellation_wins_over_the_timeout_while_waiting() {
        let admission = bounded(4, 60_000);
        let _held = admission.acquire_worker(|| false).unwrap();
        let error = admission.acquire_worker(|| true).unwrap_err();
        assert!(error.contains("cancelled"), "{error}");
        assert_eq!(admission.stats().waiting_workers, 0);
    }
    #[test]
    fn invalid_bounds_are_rejected() {
        let invalid = WorkerAdmission::new(Some(WorkerResourceConfig {
            memory_budget_bytes: 8,
            estimated_worker_bytes: 8,
            wait_timeout_ms: Some(0),
            ..Default::default()
        }));
        assert!(invalid.is_err());
    }
    #[test]
    fn nested_worker_context_retains_the_owner_and_refuses_saturated_launches() {
        use crate::runtime::{AgentId, RunId, RuntimeBus, RuntimeHandle};
        let admission = budget();
        let mut parent = RuntimeHandle::new(RunId::new(), AgentId::new(), RuntimeBus::new());
        parent.worker_admission = admission.clone();
        let runner = crate::SubagentRunner::new(|request| {
            let runtime = request.runtime.as_ref().unwrap();
            assert!(runtime.worker_resource_lease.is_held());
            let inherited = RuntimeHandle::new(RunId::new(), AgentId::new(), RuntimeBus::new())
                .with_worker_state_from(runtime);
            assert!(inherited.worker_resource_lease.is_held());
            assert_eq!(inherited.worker_admission.stats().reserved_bytes, 8);
            let mut agent = crate::Agent::new("local reservation fixture");
            agent.runtime = Some(inherited.clone());
            assert!(agent
                .workflow_launch(false)
                .worker_resource_parent
                .is_held());
            let nested = crate::SubagentRunner::new(|_| panic!("saturated child must not run"))
                .with_admission(inherited.worker_admission.clone());
            let request = crate::SubagentRequest {
                runtime: Some(inherited),
                abort: Some(Arc::new(std::sync::atomic::AtomicBool::new(false))),
                ..Default::default()
            };
            let abort = request.abort.clone().unwrap();
            let (tx, rx) = std::sync::mpsc::channel();
            let child = std::thread::spawn(move || tx.send(nested.run(&request)).unwrap());
            let result = rx.recv_timeout(Duration::from_millis(500));
            abort.store(true, std::sync::atomic::Ordering::Release);
            child.join().unwrap();
            let error = result
                .expect("nested launch must settle without waiting for its parent")
                .unwrap_err();
            assert!(error.contains("nested worker"), "{error}");
            Ok("complete".into())
        })
        .with_admission(admission.clone());
        runner
            .run(&crate::SubagentRequest {
                runtime: Some(parent),
                ..Default::default()
            })
            .unwrap();
        assert_eq!(admission.stats().reserved_bytes, 0);
        assert_eq!(admission.stats().waiting_workers, 0);
        let lease = admission.acquire_worker(|| false).unwrap();
        let background_context = lease.context();
        assert!(background_context.is_held());
        drop(lease);
        assert!(
            !background_context.is_held(),
            "background context must not retain its parent's lease"
        );
    }
    #[test]
    fn impossible_reservations_fail_and_unwinding_releases_a_lease() {
        let budget = budget();
        assert!(budget.acquire(9, || false).is_err());
        let _ = std::panic::catch_unwind(|| {
            let _lease = budget.acquire_worker(|| false).unwrap();
            panic!("worker");
        });
        assert_eq!(budget.stats().reserved_bytes, 0);
        assert!(budget.acquire_worker(|| false).is_ok());
    }
    #[test]
    fn saturation_waits_and_cancellation_settles_without_leaking_reservations() {
        use std::sync::atomic::{AtomicBool, Ordering};
        let budget = budget();
        let held = budget.acquire_worker(|| false).unwrap();
        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let queued = budget.clone();
        let worker =
            std::thread::spawn(move || queued.acquire_worker(|| flag.load(Ordering::Acquire)));
        let until = std::time::Instant::now() + Duration::from_secs(3);
        while budget.stats().waiting_workers == 0 {
            assert!(std::time::Instant::now() < until);
            std::thread::yield_now();
        }
        assert_eq!(budget.stats().reserved_bytes, 8);
        cancel.store(true, Ordering::Release);
        assert!(worker.join().unwrap().is_err());
        assert_eq!(budget.stats().waiting_workers, 0);
        drop(held);
        assert_eq!(budget.stats().reserved_bytes, 0);
    }
}
