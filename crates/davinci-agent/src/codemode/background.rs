//! In-process ownership of detached Script runs.
//!
//! Detached runs are owned: their cancellation token and join handle live with
//! the agent, so closing the session cancels them, and a host that cannot stop
//! one says so instead of leaking a worker.
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::runtime::CancellationToken;

#[derive(Default)]
struct Inner {
    tokens: HashMap<String, CancellationToken>,
    handles: Vec<(String, std::thread::JoinHandle<()>)>,
}

/// Shared by every clone of an agent so a worker's host sees the same owned set.
#[derive(Clone, Default)]
pub struct BackgroundRuns {
    inner: Arc<Mutex<Inner>>,
}

impl std::fmt::Debug for BackgroundRuns {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "BackgroundRuns({} active)", self.active())
    }
}

impl BackgroundRuns {
    pub fn register(
        &self,
        run_id: &str,
        token: CancellationToken,
        handle: std::thread::JoinHandle<()>,
    ) {
        let mut inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        inner.tokens.insert(run_id.to_string(), token);
        inner.handles.push((run_id.to_string(), handle));
    }

    /// Cancel one owned run. Returns false when the handle is unknown.
    pub fn cancel(&self, run_id: &str) -> bool {
        let inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        match inner.tokens.get(run_id) {
            Some(token) => {
                token.cancel();
                true
            }
            None => false,
        }
    }

    pub fn cancel_all(&self) {
        let inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
        for token in inner.tokens.values() {
            token.cancel();
        }
    }

    /// Wait up to `grace` for cancelled runs to finish. Returns the ids that
    /// did not finish: those must be recorded as unresolved, never as success.
    pub fn wait_finished(&self, grace: Duration) -> Vec<String> {
        let deadline = Instant::now() + grace;
        loop {
            {
                let mut inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
                // Reap finished handles so a session does not accumulate them.
                let mut index = 0;
                while index < inner.handles.len() {
                    if inner.handles[index].1.is_finished() {
                        let (run_id, handle) = inner.handles.remove(index);
                        let _ = handle.join();
                        inner.tokens.remove(&run_id);
                    } else {
                        index += 1;
                    }
                }
                if inner.handles.is_empty() {
                    return Vec::new();
                }
            }
            if Instant::now() >= deadline {
                let inner = self.inner.lock().unwrap_or_else(|error| error.into_inner());
                return inner
                    .handles
                    .iter()
                    .map(|(run_id, _)| run_id.clone())
                    .collect();
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Runs still owned by this agent (diagnostics and drain checks).
    pub fn active(&self) -> usize {
        self.inner
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .handles
            .len()
    }
}
