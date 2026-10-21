//! Parent-owned local memory reservations. These are estimates, not RSS caps.
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkerResourceConfig {
    pub memory_budget_bytes: u64,
    pub estimated_worker_bytes: u64,
}
#[derive(Debug, Default, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkerResourceStats {
    pub memory_budget_bytes: Option<u64>,
    pub estimated_worker_bytes: u64,
    pub reserved_bytes: u64,
    pub waiting_workers: usize,
}
#[derive(Debug, Default)]
struct Inner {
    error: Option<String>,
    limit: Option<u64>,
    estimate: u64,
    state: Mutex<WorkerResourceStats>,
    changed: Condvar,
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
        Ok(Self(Arc::new(Inner {
            limit: Some(config.memory_budget_bytes),
            estimate: config.estimated_worker_bytes,
            ..Default::default()
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
    pub fn acquire(&self, bytes: u64, cancelled: impl Fn() -> bool) -> Result<WorkerLease, String> {
        if let Some(error) = &self.0.error {
            return Err(error.clone());
        }
        if self.0.limit.is_some_and(|limit| bytes > limit) {
            return Err("worker reservation exceeds local resource budget".into());
        }
        let bytes = if self.0.limit.is_some() { bytes } else { 0 };
        let mut state = self.0.state.lock().unwrap_or_else(|e| e.into_inner());
        state.waiting_workers += 1;
        loop {
            if cancelled() {
                state.waiting_workers -= 1;
                return Err("worker cancelled while waiting for local resources".into());
            }
            if self
                .0
                .limit
                .is_none_or(|limit| state.reserved_bytes <= limit.saturating_sub(bytes))
            {
                state.waiting_workers -= 1;
                state.reserved_bytes += bytes;
                return Ok(WorkerLease {
                    owner: self.clone(),
                    bytes,
                });
            }
            state = self
                .0
                .changed
                .wait_timeout(state, Duration::from_millis(50))
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }
}
#[derive(Debug)]
pub struct WorkerLease {
    owner: WorkerAdmission,
    bytes: u64,
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
        }))
        .unwrap()
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
