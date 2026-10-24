//! Where a worker is working, verified from the checkout itself.
//!
//! A branch name is shown only when `HEAD` of the worker's own worktree says
//! so. The lease's recorded branch is a promise made at creation: a worker can
//! check out something else, and a released or deleted worktree has no branch
//! at all. A worker in the shared workspace has no branch of its own and is
//! never given one.
//!
//! Reading `HEAD` is a few small file reads, but on a drive that stopped
//! answering even those hang, so snapshots go through [`WorkspaceCache`]: it
//! reads on its own thread and the UI only ever sees the last known value. The
//! dirty/changed-files probe runs `git`, so it has a timeout and the caller
//! runs it off the UI thread, for one worker at a time.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use super::events::AgentRecord;

/// The changed-files list never grows past this.
pub const MAX_CHANGED_PATHS: usize = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceKind {
    /// The lead's own checkout. No separate branch.
    #[default]
    Shared,
    /// A worktree of its own, found on disk.
    Worktree,
    /// A worktree was assigned but is gone or unreadable.
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct WorkerWorkspace {
    pub kind: WorkspaceKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    /// Verified from `HEAD`. `None` for shared, detached or unreadable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    /// Short commit id `HEAD` points at, when it can be resolved.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head: Option<String>,
    #[serde(default)]
    pub detached: bool,
    /// From the last probe; `None` until one has run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dirty: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub changed_paths: Vec<String>,
}

impl WorkerWorkspace {
    /// One line a person can read, never a guess.
    pub fn label(&self) -> String {
        match self.kind {
            WorkspaceKind::Shared => "shared workspace — no separate branch".into(),
            WorkspaceKind::Unavailable => "worktree unavailable".into(),
            WorkspaceKind::Worktree => {
                let state = match self.dirty {
                    Some(true) => " · dirty",
                    Some(false) => " · clean",
                    None => "",
                };
                match (&self.branch, self.detached) {
                    (Some(branch), _) => {
                        let head = self
                            .head
                            .as_deref()
                            .map(|head| format!(" @ {head}"))
                            .unwrap_or_default();
                        format!("{branch}{head}{state}")
                    }
                    (None, true) => format!(
                        "detached HEAD{}{state}",
                        self.head
                            .as_deref()
                            .map(|head| format!(" @ {head}"))
                            .unwrap_or_default()
                    ),
                    (None, false) => format!("worktree, branch unknown{state}"),
                }
            }
        }
    }
}

/// Where `record`'s worker works. Read-only, no subprocess.
pub fn snapshot_workspace(record: &AgentRecord) -> WorkerWorkspace {
    let Some(path) = record.worktree.as_ref() else {
        return WorkerWorkspace::default();
    };
    let unavailable = || WorkerWorkspace {
        kind: WorkspaceKind::Unavailable,
        path: Some(path.clone()),
        ..WorkerWorkspace::default()
    };
    if !path.is_dir() {
        return unavailable();
    }
    // Only this directory's own `.git`: walking up would report a parent
    // repository's branch for a worktree that is gone.
    let Some(git_dir) = own_git_dir(path) else {
        return unavailable();
    };
    let Ok(head_text) = std::fs::read_to_string(git_dir.join("HEAD")) else {
        return unavailable();
    };
    let head_text = head_text.trim();
    let mut workspace = WorkerWorkspace {
        kind: WorkspaceKind::Worktree,
        path: Some(path.clone()),
        ..WorkerWorkspace::default()
    };
    if let Some(reference) = head_text.strip_prefix("ref: ") {
        workspace.branch = reference
            .strip_prefix("refs/heads/")
            .map(str::to_string)
            .or_else(|| Some(reference.to_string()));
        workspace.head = resolve_ref(&git_dir, reference).map(|id| short(&id));
    } else if is_object_id(head_text) {
        workspace.detached = true;
        workspace.head = Some(short(head_text));
    }
    workspace
}

fn short(id: &str) -> String {
    id.chars().take(7).collect()
}

fn is_object_id(text: &str) -> bool {
    matches!(text.len(), 40 | 64) && text.chars().all(|ch| ch.is_ascii_hexdigit())
}

/// The directory holding this checkout's `HEAD`: `<path>/.git` itself, or the
/// per-worktree directory a `.git` file points at.
fn own_git_dir(path: &Path) -> Option<PathBuf> {
    let dot_git = path.join(".git");
    if dot_git.is_dir() {
        return Some(dot_git);
    }
    let pointer = std::fs::read_to_string(&dot_git).ok()?;
    let target = pointer.trim().strip_prefix("gitdir:")?.trim();
    let target = PathBuf::from(target);
    let target = if target.is_absolute() {
        target
    } else {
        path.join(target)
    };
    target.is_dir().then_some(target)
}

/// Resolve `refs/heads/x` to an object id from loose refs, then packed-refs,
/// in the worktree's git dir and in the common dir it points to.
fn resolve_ref(git_dir: &Path, reference: &str) -> Option<String> {
    let mut dirs = vec![git_dir.to_path_buf()];
    if let Ok(common) = std::fs::read_to_string(git_dir.join("commondir")) {
        let common = PathBuf::from(common.trim());
        dirs.push(if common.is_absolute() {
            common
        } else {
            git_dir.join(common)
        });
    }
    for dir in &dirs {
        if let Ok(text) = std::fs::read_to_string(dir.join(reference)) {
            let id = text.trim();
            if is_object_id(id) {
                return Some(id.to_string());
            }
        }
    }
    for dir in &dirs {
        if let Ok(packed) = std::fs::read_to_string(dir.join("packed-refs")) {
            for line in packed.lines() {
                if let Some((id, name)) = line.split_once(' ') {
                    if name == reference && is_object_id(id) {
                        return Some(id.to_string());
                    }
                }
            }
        }
    }
    None
}

/// `git status` for one worktree, with a timeout: `(dirty, changed paths)`.
/// `(None, [])` when git is missing, fails or does not answer in time. Run it
/// off the UI thread.
pub fn probe_changes(path: &Path, timeout: Duration) -> (Option<bool>, Vec<String>) {
    let mut command = std::process::Command::new("git");
    command
        .args(["-c", "safe.bareRepository=explicit", "-C"])
        .arg(path)
        .args(["status", "--porcelain=v1", "-z", "--untracked-files=normal"])
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let Ok(mut child) = command.spawn() else {
        return (None, Vec::new());
    };
    let Some(mut stdout) = child.stdout.take() else {
        let _ = child.kill();
        return (None, Vec::new());
    };
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        let _ = sender.send(bytes);
    });
    let Ok(bytes) = receiver.recv_timeout(timeout) else {
        let _ = child.kill();
        let _ = child.wait();
        return (None, Vec::new());
    };
    match child.wait() {
        Ok(status) if status.success() => {}
        _ => return (None, Vec::new()),
    }
    let text = String::from_utf8_lossy(&bytes);
    let mut paths = Vec::new();
    let mut entries = text.split('\0').filter(|entry| !entry.is_empty());
    while let Some(entry) = entries.next() {
        if entry.len() < 4 {
            continue;
        }
        let (status, file) = entry.split_at(3);
        // A rename lists its source as a second entry.
        if status.starts_with('R') || status.starts_with('C') {
            entries.next();
        }
        if paths.len() < MAX_CHANGED_PATHS {
            paths.push(super::worker_activity::safe_summary(file));
        }
    }
    let dirty = !text.trim_matches('\0').is_empty();
    (Some(dirty), paths)
}

/// How long a cached workspace is trusted before it is read again.
const FRESH_FOR: Duration = Duration::from_secs(2);
/// A read that has not finished after this long marks the worktree
/// unavailable, and is not started again while it is still stuck.
const READ_TIMEOUT: Duration = Duration::from_secs(3);

struct CachedWorkspace {
    path: PathBuf,
    /// Unique for this path assignment, so an old background read cannot
    /// overwrite a newer worktree (even if the path changes away and back).
    token: std::sync::Arc<()>,
    value: WorkerWorkspace,
    read_at: Option<std::time::Instant>,
    /// A read in flight, and when it began.
    reading_since: Option<std::time::Instant>,
}

impl CachedWorkspace {
    fn new(path: &Path) -> Self {
        Self {
            path: path.to_path_buf(),
            token: std::sync::Arc::new(()),
            value: WorkerWorkspace {
                kind: WorkspaceKind::Worktree,
                path: Some(path.to_path_buf()),
                ..WorkerWorkspace::default()
            },
            read_at: None,
            reading_since: None,
        }
    }
}

/// The last known workspace of each worker, refreshed off the caller's thread.
///
/// `snapshot_workspace` touches the disk; on a network drive that stopped
/// answering it would hold the caller for as long as the drive does. The UI
/// reads this cache instead: it never waits, shows the last value it has, and
/// shows `worktree unavailable` for a read that is stuck.
#[derive(Clone, Default)]
pub struct WorkspaceCache {
    entries: std::sync::Arc<
        std::sync::Mutex<std::collections::HashMap<super::ids::AgentId, CachedWorkspace>>,
    >,
}

impl WorkspaceCache {
    pub fn get(&self, record: &AgentRecord) -> WorkerWorkspace {
        self.get_with(record, snapshot_workspace, FRESH_FOR, READ_TIMEOUT)
    }

    fn get_with(
        &self,
        record: &AgentRecord,
        read: fn(&AgentRecord) -> WorkerWorkspace,
        fresh_for: Duration,
        timeout: Duration,
    ) -> WorkerWorkspace {
        // A removed worktree invalidates the old read even if this agent ID
        // receives a new path before the background task finishes.
        let Some(path) = record.worktree.as_ref() else {
            self.entries
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&record.id);
            return WorkerWorkspace::default();
        };
        let now = std::time::Instant::now();
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        let entry = entries
            .entry(record.id)
            .or_insert_with(|| CachedWorkspace::new(path));
        if entry.path.as_path() != path.as_path() {
            // Do not present another worktree's branch while a new read runs.
            *entry = CachedWorkspace::new(path);
        }
        if let Some(since) = entry.reading_since {
            if now.duration_since(since) >= timeout {
                entry.value = WorkerWorkspace {
                    kind: WorkspaceKind::Unavailable,
                    path: Some(path.clone()),
                    ..WorkerWorkspace::default()
                };
            }
            return entry.value.clone();
        }
        let stale = entry
            .read_at
            .is_none_or(|at| now.duration_since(at) >= fresh_for);
        if stale {
            entry.reading_since = Some(now);
            let (cache, record, token) = (self.clone(), record.clone(), entry.token.clone());
            let spawned = std::thread::Builder::new()
                .name("davinci-workspace-read".into())
                .spawn(move || {
                    let value = read(&record);
                    let mut entries = cache.entries.lock().unwrap_or_else(|e| e.into_inner());
                    if let Some(entry) = entries.get_mut(&record.id) {
                        if std::sync::Arc::ptr_eq(&entry.token, &token) {
                            entry.value = value;
                            entry.read_at = Some(std::time::Instant::now());
                            entry.reading_since = None;
                        }
                    }
                });
            if spawned.is_err() {
                entry.reading_since = None;
            }
        }
        entry.value.clone()
    }
}

/// The process-wide cache the snapshots use.
pub fn cached_workspace(record: &AgentRecord) -> WorkerWorkspace {
    static CACHE: std::sync::OnceLock<WorkspaceCache> = std::sync::OnceLock::new();
    CACHE.get_or_init(WorkspaceCache::default).get(record)
}

#[cfg(test)]
mod cache_tests {
    use super::*;
    use crate::runtime::{AgentId, AgentKind, AgentState, RunId};

    fn record(worktree: Option<PathBuf>) -> AgentRecord {
        AgentRecord {
            id: AgentId::new(),
            run_id: RunId::new(),
            parent: None,
            kind: AgentKind::Subagent,
            name: "w".into(),
            provider: String::new(),
            model_id: String::new(),
            cwd: PathBuf::from("."),
            state: AgentState::Running,
            task_id: None,
            worktree,
            started_ms: 0,
            updated_ms: 0,
            failure_reason: None,
        }
    }

    fn slow_read(record: &AgentRecord) -> WorkerWorkspace {
        std::thread::sleep(Duration::from_millis(400));
        WorkerWorkspace {
            kind: WorkspaceKind::Worktree,
            path: record.worktree.clone(),
            branch: Some("late".into()),
            ..WorkerWorkspace::default()
        }
    }

    fn quick_read(record: &AgentRecord) -> WorkerWorkspace {
        WorkerWorkspace {
            kind: WorkspaceKind::Worktree,
            path: record.worktree.clone(),
            branch: Some("fast".into()),
            ..WorkerWorkspace::default()
        }
    }

    #[test]
    fn a_stuck_read_never_holds_the_caller_and_ends_as_unavailable() {
        let cache = WorkspaceCache::default();
        let record = record(Some(PathBuf::from("/net/worktree")));
        let started = std::time::Instant::now();
        let first = cache.get_with(
            &record,
            slow_read,
            Duration::from_secs(1),
            Duration::from_millis(50),
        );
        assert!(
            started.elapsed() < Duration::from_millis(200),
            "the caller waited"
        );
        assert_eq!(
            first.branch, None,
            "nothing is known yet, and none is invented"
        );
        std::thread::sleep(Duration::from_millis(80));
        let second = cache.get_with(
            &record,
            slow_read,
            Duration::from_secs(1),
            Duration::from_millis(50),
        );
        assert_eq!(second.kind, WorkspaceKind::Unavailable);
        assert!(
            started.elapsed() < Duration::from_millis(350),
            "still did not wait"
        );
        // The slow read finally lands and the real value replaces the marker.
        std::thread::sleep(Duration::from_millis(500));
        let third = cache.get_with(
            &record,
            slow_read,
            Duration::from_secs(60),
            Duration::from_secs(60),
        );
        assert_eq!(third.branch.as_deref(), Some("late"));
    }

    #[test]
    fn changed_worktree_path_never_shows_or_restores_the_previous_branch() {
        let cache = WorkspaceCache::default();
        let original = record(Some(PathBuf::from("/worktree/old")));
        let initial = cache.get_with(
            &original,
            slow_read,
            Duration::from_secs(60),
            Duration::from_secs(2),
        );
        assert_eq!(initial.path, original.worktree);

        let mut moved = original.clone();
        moved.worktree = Some(PathBuf::from("/worktree/new"));
        let pending = cache.get_with(
            &moved,
            quick_read,
            Duration::from_secs(60),
            Duration::from_secs(2),
        );
        assert_eq!(pending.path, moved.worktree);
        assert!(pending.branch.is_none());

        let mut current = pending;
        for _ in 0..100 {
            current = cache.get_with(
                &moved,
                quick_read,
                Duration::from_secs(60),
                Duration::from_secs(2),
            );
            if current.branch.as_deref() == Some("fast") {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(current.branch.as_deref(), Some("fast"));
        // Wait for the slow read of the original path; it must not clobber
        // the newer worker assignment or its branch.
        std::thread::sleep(Duration::from_millis(450));
        let final_value = cache.get_with(
            &moved,
            quick_read,
            Duration::from_secs(60),
            Duration::from_secs(2),
        );
        assert_eq!(final_value.path, moved.worktree);
        assert_eq!(final_value.branch.as_deref(), Some("fast"));

        // Clearing the assignment also invalidates outstanding cache state.
        moved.worktree = None;
        assert_eq!(cache.get(&moved), WorkerWorkspace::default());
        moved.worktree = Some(PathBuf::from("/worktree/new"));
        assert!(cache
            .get_with(
                &moved,
                quick_read,
                Duration::from_secs(60),
                Duration::from_secs(2),
            )
            .branch
            .is_none());
    }

    #[test]
    fn a_fresh_value_is_reused_and_a_stale_one_is_read_again() {
        let cache = WorkspaceCache::default();
        let record = record(Some(PathBuf::from("/w")));
        let _ = cache.get_with(
            &record,
            quick_read,
            Duration::from_millis(100),
            Duration::from_secs(5),
        );
        std::thread::sleep(Duration::from_millis(60));
        let value = cache.get_with(
            &record,
            quick_read,
            Duration::from_millis(100),
            Duration::from_secs(5),
        );
        assert_eq!(value.branch.as_deref(), Some("fast"));
        // A shared worker never reads the disk at all.
        assert_eq!(cache.get(&self::record(None)), WorkerWorkspace::default());
    }
}
