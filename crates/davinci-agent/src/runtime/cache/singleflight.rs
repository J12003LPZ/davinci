use super::CacheError;
use crate::CancellationToken;
use std::any::Any;
use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

type Shared = Arc<dyn Any + Send + Sync>;
#[derive(Default)]
struct Flight {
    result: Mutex<Option<Result<Shared, CacheError>>>,
    ready: Condvar,
}

/// Bounded coordination only. Values live in the owning consumer, never here after completion.
#[derive(Default)]
pub struct SingleFlight {
    active: Mutex<HashMap<String, Arc<Flight>>>,
}
impl std::fmt::Debug for SingleFlight {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SingleFlight")
    }
}

impl SingleFlight {
    /// Run `compute` once per `key` across concurrent callers.
    ///
    /// The first caller leads and runs `compute` on its own thread, to
    /// completion: `compute` borrows the caller's state, so it cannot be
    /// abandoned midway, and its own I/O must carry its own deadline (the
    /// semantic launcher passes the same timeout to the server it starts).
    /// `timeout` bounds how long every other caller waits for the leader;
    /// a follower gives up with `Timeout` and the flight carries on.
    ///
    /// `cancel` belongs to its caller alone (WOR-101). A cancelled follower
    /// stops waiting; a leader cancelled during `compute` returns
    /// `Cancelled` but still hands the finished value to its followers.
    pub fn run<T: Send + Sync + 'static>(
        &self,
        key: &str,
        timeout: Duration,
        cancel: Option<&CancellationToken>,
        compute: impl FnOnce() -> Result<T, CacheError>,
    ) -> Result<(Arc<T>, bool), CacheError> {
        if cancel.is_some_and(CancellationToken::is_cancelled) {
            return Err(CacheError::Cancelled);
        }
        let (flight, leader) = {
            let mut active = self.active.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(flight) = active.get(key) {
                (flight.clone(), false)
            } else {
                if active.len() >= 256 {
                    return Err(CacheError::Busy);
                }
                let flight = Arc::new(Flight::default());
                active.insert(key.into(), flight.clone());
                (flight, true)
            }
        };
        if leader {
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(compute))
                .map_err(|_| CacheError::Panicked)
                .and_then(|r| r)
                .map(|value| Arc::new(value) as Shared);
            *flight.result.lock().unwrap_or_else(|e| e.into_inner()) = Some(result.clone());
            flight.ready.notify_all();
            self.active
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(key);
            if cancel.is_some_and(CancellationToken::is_cancelled) {
                return Err(CacheError::Cancelled);
            }
            return result
                .and_then(|v| v.downcast::<T>().map_err(|_| CacheError::TypeMismatch))
                .map(|value| (value, true));
        }
        let start = Instant::now();
        let mut result = flight.result.lock().unwrap_or_else(|e| e.into_inner());
        loop {
            if cancel.is_some_and(CancellationToken::is_cancelled) {
                return Err(CacheError::Cancelled);
            }
            if let Some(value) = result.as_ref() {
                return value
                    .clone()
                    .and_then(|v| v.downcast::<T>().map_err(|_| CacheError::TypeMismatch))
                    .map(|value| (value, false));
            }
            let remaining = timeout.saturating_sub(start.elapsed());
            if remaining.is_zero() {
                return Err(CacheError::Timeout);
            }
            result = flight
                .ready
                .wait_timeout(result, remaining.min(Duration::from_millis(20)))
                .unwrap_or_else(|e| e.into_inner())
                .0;
        }
    }
}
