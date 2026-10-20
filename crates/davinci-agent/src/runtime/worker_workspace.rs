//! Where a worker is working, verified from the checkout itself.
//!
//! A branch name is shown only when `HEAD` of the worker's own worktree says
//! so. The lease's recorded branch is a promise made at creation: a worker can
//! check out something else, and a released or deleted worktree has no branch
//! at all. A worker in the shared workspace has no branch of its own and is
//! never given one.
//!
//! Reading `HEAD` is a few small file reads and is safe on any thread at any
//! rate. The dirty/changed-files probe runs `git`, so it has a timeout and the
//! caller runs it off the UI thread, for one worker at a time.

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
        .arg("-C")
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
