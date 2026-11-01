//! In-process ownership of detached Script runs.
//!
//! Detached runs are owned: their cancellation token and join handle live with
//! the agent, so closing the session cancels them, and a host that cannot stop
//! one says so instead of leaking a worker.
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::runtime::CancellationToken;

#[derive(Default)]
struct Inner {
    tokens: HashMap<String, CancellationToken>,
    handles: Vec<(String, std::thread::JoinHandle<()>)>,
    /// Drains in progress. While any is, no new run is admitted, so a launch
    /// racing a shutdown is refused instead of escaping it.
    closing: usize,
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
    fn lock(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|error| error.into_inner())
    }

    /// Admit a run before its worker exists, so a shutdown that starts now
    /// already sees (and cancels) it. False while a drain is in progress.
    pub fn reserve(&self, run_id: &str, token: CancellationToken) -> bool {
        let mut inner = self.lock();
        if inner.closing > 0 {
            return false;
        }
        inner.tokens.insert(run_id.to_string(), token);
        true
    }

    /// Give up a reservation whose worker never started.
    pub fn release(&self, run_id: &str) {
        self.lock().tokens.remove(run_id);
    }

    /// Attach the worker to a reserved run.
    pub fn attach(&self, run_id: &str, handle: std::thread::JoinHandle<()>) {
        self.lock().handles.push((run_id.to_string(), handle));
    }

    /// Refuse new runs until the matching `reopen`. Drains may overlap.
    pub fn close(&self) {
        self.lock().closing += 1;
    }

    pub fn reopen(&self) {
        let mut inner = self.lock();
        inner.closing = inner.closing.saturating_sub(1);
    }

    /// Cancel one owned run. Returns false when the handle is unknown.
    pub fn cancel(&self, run_id: &str) -> bool {
        match self.lock().tokens.get(run_id) {
            Some(token) => {
                token.cancel();
                true
            }
            None => false,
        }
    }

    pub fn cancel_all(&self) {
        for token in self.lock().tokens.values() {
            token.cancel();
        }
    }

    /// Reap finished workers. Joining a finished thread does not block.
    fn reap(inner: &mut Inner) {
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
    }

    /// Wait up to `grace` for cancelled runs to finish. Returns the ids that
    /// did not finish: those must be recorded as unresolved, never as success.
    /// A reserved run whose worker is still starting counts as unfinished.
    pub fn wait_finished(&self, grace: Duration) -> Vec<String> {
        self.wait_until(grace, None)
    }

    /// Wait up to `grace` for one run's worker. True when it finished.
    pub fn wait_run(&self, run_id: &str, grace: Duration) -> bool {
        self.wait_until(grace, Some(run_id)).is_empty()
    }

    fn wait_until(&self, grace: Duration, only: Option<&str>) -> Vec<String> {
        let deadline = Instant::now() + grace;
        loop {
            // The lock is dropped between polls, so a worker or a launch that
            // needs it is never blocked by the wait.
            let unfinished: Vec<String> = {
                let mut inner = self.lock();
                Self::reap(&mut inner);
                inner
                    .tokens
                    .keys()
                    .filter(|id| only.is_none_or(|run| run == id.as_str()))
                    .cloned()
                    .collect()
            };
            if unfinished.is_empty() || Instant::now() >= deadline {
                return unfinished;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    /// Runs still owned by this agent (diagnostics and drain checks).
    pub fn active(&self) -> usize {
        self.lock().tokens.len()
    }

    /// Remember this set, with its registry directory, so process exit can
    /// stop it without a handle to the agent. Sets nobody else holds any more
    /// are dropped here. False once process-wide shutdown has begun: the
    /// caller must refuse the launch.
    pub(crate) fn track(&self, registry_dir: &Path) -> bool {
        tracked()
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .track(self, registry_dir)
    }
}

/// The sets process exit must stop, and the permanent flag saying exit has
/// begun. Both sit under one lock, so a launch is either tracked before the
/// shutdown snapshot (and cancelled) or sees the flag (and is refused).
#[derive(Default)]
struct Tracking {
    shutdown: bool,
    sets: Vec<(BackgroundRuns, PathBuf)>,
}

impl Tracking {
    fn track(&mut self, runs: &BackgroundRuns, registry_dir: &Path) -> bool {
        if self.shutdown {
            return false;
        }
        self.sets
            .retain(|(held, _)| Arc::strong_count(&held.inner) > 1);
        if !self
            .sets
            .iter()
            .any(|(held, _)| Arc::ptr_eq(&held.inner, &runs.inner))
        {
            self.sets.push((runs.clone(), registry_dir.to_path_buf()));
        }
        true
    }

    /// Set the flag and snapshot the sets in one step.
    fn begin_shutdown(&mut self) -> Vec<(BackgroundRuns, PathBuf)> {
        self.shutdown = true;
        self.sets.clone()
    }
}

fn tracked() -> &'static Mutex<Tracking> {
    static TRACKED: OnceLock<Mutex<Tracking>> = OnceLock::new();
    TRACKED.get_or_init(Default::default)
}

/// Runs that did not finish are unresolved, never cancelled or successful.
pub(crate) fn mark_unresolved(registry_dir: &Path, run_ids: &[String]) {
    if run_ids.is_empty() {
        return;
    }
    if let Ok(registry) = super::ScriptRunRegistry::open(registry_dir) {
        for run_id in run_ids {
            let _ = registry.transition(run_id, super::ScriptRunState::RecoveryRequired);
        }
    }
}

/// Stop every detached Script run of every agent in this process, within one
/// shared `grace`, for process exit. New launches stay refused afterwards.
/// Safe to call more than once.
pub fn shutdown_all_background_scripts(grace: Duration) -> Vec<String> {
    let all = tracked()
        .lock()
        .unwrap_or_else(|error| error.into_inner())
        .begin_shutdown();
    let deadline = Instant::now() + grace;
    for (runs, _) in &all {
        runs.close();
        runs.cancel_all();
    }
    let mut unfinished = Vec::new();
    for (runs, dir) in &all {
        let ids = runs.wait_finished(deadline.saturating_duration_since(Instant::now()));
        mark_unresolved(dir, &ids);
        unfinished.extend(ids);
    }
    unfinished
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A launch tracked before the shutdown snapshot is in it; one tracked
    /// after is refused. Uses a private `Tracking`, so the process-wide flag
    /// is never set by a test.
    #[test]
    fn launch_after_shutdown_snapshot_is_refused() {
        let mut tracking = Tracking::default();
        let early = BackgroundRuns::default();
        assert!(tracking.track(&early, Path::new("early")));
        let snapshot = tracking.begin_shutdown();
        assert_eq!(snapshot.len(), 1);
        assert!(Arc::ptr_eq(&snapshot[0].0.inner, &early.inner));

        let late = BackgroundRuns::default();
        assert!(!tracking.track(&late, Path::new("late")));
    }
}
