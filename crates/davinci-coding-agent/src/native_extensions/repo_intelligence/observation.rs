//! In-process change hints. A watcher is not an atomic filesystem snapshot.
//! Inventory/stamps are reconciled on every query and contents periodically or
//! explicitly. No observer state is trusted after process restart or overflow.
use super::scanner::{self, FileStamp};
use notify::{Event, EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use serde_json::{json, Value};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{sync_channel, Receiver},
        Arc,
    },
    time::{Duration, Instant},
};

const MAX_EVENTS: usize = 4096;
const MAX_DIRECTORIES: usize = 4096;
const FULL_REFRESH_INTERVAL: Duration = Duration::from_secs(30);

#[derive(Default)]
pub(super) struct Observation {
    started: bool,
    watcher: Option<RecommendedWatcher>,
    receiver: Option<Receiver<Event>>,
    lost: Arc<AtomicBool>,
    failed: Arc<AtomicBool>,
    watched: BTreeSet<PathBuf>,
    reason: Option<&'static str>,
    pub stamps: BTreeMap<String, FileStamp>,
    ignore_hashes: BTreeMap<String, String>,
    pub needs_full: bool,
    last_full: Option<Instant>,
}

impl std::fmt::Debug for Observation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.status().fmt(f)
    }
}

#[derive(Default)]
pub(super) struct Changes {
    pub paths: BTreeSet<String>,
    pub rescan: bool,
}

impl Observation {
    pub fn start(&mut self, root: &Path, enabled: bool) {
        if self.started {
            return;
        }
        self.started = true;
        self.needs_full = true;
        if !enabled {
            self.reason = Some("disabled; full content reconciliation");
            return;
        }
        let (send, receive) = sync_channel(MAX_EVENTS);
        let lost = self.lost.clone();
        let failed = self.failed.clone();
        let watched_root = root.to_path_buf();
        let watcher = notify::recommended_watcher(move |event: notify::Result<Event>| {
            let Ok(mut event) = event else {
                failed.store(true, Ordering::Release);
                return;
            };
            if event.need_rescan() {
                lost.store(true, Ordering::Release);
            }
            if matches!(event.kind, EventKind::Access(_)) {
                return;
            }
            if event.paths.len() > 16 {
                lost.store(true, Ordering::Release);
                return;
            }
            event.paths.retain(|path| {
                path.strip_prefix(&watched_root)
                    .is_ok_and(|relative| !scanner::excluded(relative))
            });
            if event.paths.is_empty() {
                return;
            }
            if send.try_send(event).is_err() {
                lost.store(true, Ordering::Release);
            }
        });
        match watcher {
            Ok(watcher) => {
                self.watcher = Some(watcher);
                self.receiver = Some(receive);
            }
            Err(_) => {
                self.reason = Some("native watcher unavailable; full content reconciliation");
                return;
            }
        }
        // Watch only directories actually traversed, never recursively register
        // node_modules or ignored trees. Parent notices root deletion/replacement.
        if let Some(parent) = root.parent() {
            self.watch_directory(parent);
        }
        self.watch_directory(root);
    }

    pub fn watch_directory(&mut self, path: &Path) {
        if self.watched.contains(path) || self.watcher.is_none() {
            return;
        }
        if self.watched.len() >= MAX_DIRECTORIES {
            self.disable("watch directory limit; full content reconciliation");
            return;
        }
        if self
            .watcher
            .as_mut()
            .unwrap()
            .watch(path, RecursiveMode::NonRecursive)
            .is_err()
        {
            self.disable("watch registration failed; full content reconciliation");
        } else {
            self.watched.insert(path.to_path_buf());
        }
    }

    fn disable(&mut self, reason: &'static str) {
        self.watcher = None;
        self.receiver = None;
        self.watched.clear();
        self.reason = Some(reason);
        self.needs_full = true;
    }

    pub fn drain(&mut self, root: &Path) -> Changes {
        if self.failed.load(Ordering::Acquire) {
            self.disable("watcher failed; full content reconciliation");
        }
        let mut changes = Changes {
            rescan: self.lost.swap(false, Ordering::AcqRel),
            ..Default::default()
        };
        if let Some(receiver) = &self.receiver {
            for event in receiver.try_iter().take(MAX_EVENTS) {
                changes.rescan |= event.need_rescan();
                for path in event.paths {
                    if let Ok(relative) = path.strip_prefix(root) {
                        let relative = relative.to_string_lossy().replace('\\', "/");
                        if relative.is_empty()
                            || relative.ends_with(".gitignore")
                            || scanner::config_file(&relative)
                        {
                            changes.rescan = true;
                        }
                        // Directory replacement can invalidate a native watch
                        // while retaining the same pathname. Re-register it.
                        if self.watched.contains(&path) {
                            if let Some(watcher) = &mut self.watcher {
                                let _ = watcher.unwatch(&path);
                            }
                            self.watched.remove(&path);
                            changes.rescan = true;
                        }
                        changes.paths.insert(relative);
                    }
                }
            }
        }
        self.needs_full |= changes.rescan;
        changes
    }

    pub fn full_required(&self, ignore_hashes: &BTreeMap<String, String>) -> bool {
        self.needs_full
            || self.ignore_hashes != *ignore_hashes
            || self.watcher.is_none()
            || self
                .last_full
                .is_none_or(|time| time.elapsed() >= FULL_REFRESH_INTERVAL)
    }

    pub fn publish(
        &mut self,
        root: &Path,
        directories: &BTreeSet<PathBuf>,
        stamps: BTreeMap<String, FileStamp>,
        ignore_hashes: BTreeMap<String, String>,
        full: bool,
    ) {
        let stale: Vec<_> = self
            .watched
            .iter()
            .filter(|p| Some(p.as_path()) != root.parent() && !directories.contains(*p))
            .cloned()
            .collect();
        for path in stale {
            if let Some(watcher) = &mut self.watcher {
                let _ = watcher.unwatch(&path);
            }
            self.watched.remove(&path);
        }
        self.stamps = stamps;
        self.ignore_hashes = ignore_hashes;
        self.needs_full = false;
        if full {
            self.last_full = Some(Instant::now());
        }
    }

    pub fn status(&self) -> Value {
        json!({"started":self.started,"active":self.watcher.is_some(),"watched_directories":self.watched.len(),
            "fallback":self.reason,"reconcile_interval_ms":FULL_REFRESH_INTERVAL.as_millis(),
            "consistency":"observed changes; explicit full refresh required for completion evidence"})
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overflow_and_backend_failure_require_full_content_reconciliation() {
        let root = tempfile::tempdir().unwrap();
        let mut observer = Observation::default();
        observer.start(root.path(), true);
        observer.needs_full = false;
        observer.last_full = Some(Instant::now());
        observer.lost.store(true, Ordering::Release);
        assert!(observer.drain(root.path()).rescan);
        assert!(observer.full_required(&BTreeMap::new()));
        observer.failed.store(true, Ordering::Release);
        observer.drain(root.path());
        assert!(observer.full_required(&BTreeMap::new()));
        assert!(observer.watcher.is_none());
    }
}
