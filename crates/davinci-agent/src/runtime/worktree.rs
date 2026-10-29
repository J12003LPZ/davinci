//! Git worktree manager isolating mutation agents in dedicated working trees.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::bus::RuntimeBus;

/// The repository top-level containing `path`, or `None` outside a git
/// checkout. Worktree leases must be rooted here: rooting them at a
/// subdirectory the user launched from produced leases of the wrong tree.
pub fn git_toplevel(path: &Path) -> Option<PathBuf> {
    let output = Command::new("git")
        .args(["-c", "safe.bareRepository=explicit"])
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(path)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let root = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!root.is_empty()).then(|| PathBuf::from(root))
}
use super::events::{RuntimeEvent, RuntimeEventEnvelope};
use super::ids::{AgentId, RunId};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorktreeLease {
    pub agent_id: AgentId,
    pub run_id: RunId,
    pub path: PathBuf,
    pub branch: String,
    pub base_head: String,
    pub created_at_ms: i64,
    #[serde(default)]
    pub repo_root: Option<PathBuf>,
    /// Distinguishes two leases of the same run and agent, which share a
    /// deterministic path. Assigned by the manager; a release must present the
    /// lease the manager recorded, not a stale copy of an earlier one.
    #[serde(default)]
    pub generation: u64,
}

#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum WorktreeError {
    #[error("not a git repository: {0}")]
    NotAGitRepository(PathBuf),
    #[error("branch already in use by another active lease: {0}")]
    BranchInUse(String),
    #[error("worktree path already exists: {0}")]
    PathAlreadyExists(PathBuf),
    #[error("lease not found: {0}")]
    LeaseNotFound(PathBuf),
    #[error("lease is stale: a newer lease owns {0}")]
    StaleLease(PathBuf),
    #[error("worktree has uncommitted or unmerged changes: {path}")]
    DirtyWorktreePreserved { path: PathBuf },
    #[error("worktree cleanup incomplete ({detail}); left behind: {}", left_behind(.path, .branch))]
    CleanupIncomplete {
        /// The worktree directory or its git registration, if still present.
        path: Option<PathBuf>,
        /// The agent branch, if it still exists.
        branch: Option<String>,
        detail: String,
    },
    #[error("git command failed: {0}")]
    GitError(String),
    #[error("io error: {0}")]
    IoError(String),
}

fn left_behind(path: &Option<PathBuf>, branch: &Option<String>) -> String {
    let mut parts = Vec::new();
    if let Some(path) = path {
        parts.push(format!("path {}", path.display()));
    }
    if let Some(branch) = branch {
        parts.push(format!("branch {branch}"));
    }
    parts.join(", ")
}

/// Something under the managed prefix that no live lease owns. Reported only:
/// it may hold someone's unmerged work, so nothing deletes it automatically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorktreeOrphan {
    Directory(PathBuf),
    Branch(String),
    /// Git still lists a worktree here, whether or not its directory exists.
    Registration(PathBuf),
}

/// Comparable form of a path that may not exist: git prints forward slashes
/// and no `\\?\` prefix, while `canonicalize` on Windows adds one.
fn path_key(path: &Path) -> String {
    let resolved = path.canonicalize().unwrap_or_else(|_| {
        match (
            path.parent().and_then(|p| p.canonicalize().ok()),
            path.file_name(),
        ) {
            (Some(parent), Some(name)) => parent.join(name),
            _ => path.to_path_buf(),
        }
    });
    let text = resolved.to_string_lossy().replace('\\', "/");
    let text = text.strip_prefix("//?/").unwrap_or(&text).to_string();
    if cfg!(windows) {
        text.to_lowercase()
    } else {
        text
    }
}

/// Git prints raw bytes; Windows paths from git are UTF-8.
fn path_from_git_bytes(bytes: &[u8]) -> PathBuf {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt;
        PathBuf::from(std::ffi::OsStr::from_bytes(bytes))
    }
    #[cfg(not(unix))]
    {
        PathBuf::from(String::from_utf8_lossy(bytes).into_owned())
    }
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn run_git(cwd: &Path, args: &[&str]) -> Result<String, WorktreeError> {
    let output = Command::new("git")
        .current_dir(cwd)
        .args(["-c", "safe.bareRepository=explicit"])
        .args(args)
        .output()
        .map_err(|e| WorktreeError::GitError(format!("Failed to execute git: {e}")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(WorktreeError::GitError(format!(
            "git {} failed (status {:?}): {}",
            args.join(" "),
            output.status.code(),
            stderr.trim()
        )));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

/// Return whether `cwd` currently has tracked or untracked Git changes.
/// Non-repositories and unavailable Git are treated as clean routing context.
pub fn has_uncommitted_changes(cwd: &Path) -> bool {
    run_git(cwd, &["status", "--porcelain"])
        .map(|status| !status.is_empty())
        .unwrap_or(false)
}

/// Inspect an already isolated worktree for a host-approved coding handoff.
/// This owner never switches branches or hides unrelated working changes.
pub fn handoff_snapshot(cwd: &Path, paths: &[String]) -> Result<String, WorktreeError> {
    let git_dir = run_git(cwd, &["rev-parse", "--path-format=absolute", "--git-dir"])?;
    let common = run_git(
        cwd,
        &["rev-parse", "--path-format=absolute", "--git-common-dir"],
    )?;
    if git_dir == common {
        return Err(WorktreeError::GitError(
            "Open the coding session in an isolated worktree before applying a design".into(),
        ));
    }
    if paths.is_empty()
        || paths.len() > 32
        || paths
            .iter()
            .any(|p| p.is_empty() || p.starts_with('-') || p.contains('\0'))
    {
        return Err(WorktreeError::GitError(
            "handoff needs bounded target paths".into(),
        ));
    }
    let mut args = vec!["status", "--porcelain", "--untracked-files=all", "--"];
    args.extend(paths.iter().map(String::as_str));
    if !run_git(cwd, &args)?.is_empty() {
        return Err(WorktreeError::DirtyWorktreePreserved {
            path: cwd.to_path_buf(),
        });
    }
    Ok(format!(
        "{}\n{}\n{}",
        git_dir,
        run_git(cwd, &["rev-parse", "HEAD"])?,
        run_git(cwd, &["symbolic-ref", "--short", "HEAD"])?
    ))
}

/// Thread-safe manager for ephemeral git worktrees.
#[derive(Clone)]
pub struct WorktreeManager {
    repo_root: PathBuf,
    worktree_root: PathBuf,
    leases: Arc<RwLock<HashMap<PathBuf, WorktreeLease>>>,
    bus: Option<RuntimeBus>,
    seq: Arc<AtomicU64>,
    generations: Arc<AtomicU64>,
}

impl std::fmt::Debug for WorktreeManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorktreeManager")
            .field("repo_root", &self.repo_root)
            .field("worktree_root", &self.worktree_root)
            .finish()
    }
}

impl WorktreeManager {
    pub fn new(repo_root: impl Into<PathBuf>, worktree_root: impl Into<PathBuf>) -> Self {
        Self {
            repo_root: repo_root.into(),
            worktree_root: worktree_root.into(),
            leases: Arc::new(RwLock::new(HashMap::new())),
            bus: None,
            seq: Arc::new(AtomicU64::new(0)),
            generations: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn with_bus(mut self, bus: RuntimeBus) -> Self {
        self.bus = Some(bus);
        self
    }

    /// Validates that `repo_root` is inside a git working tree.
    pub fn validate_git_repo(&self) -> Result<(), WorktreeError> {
        let is_wt = run_git(&self.repo_root, &["rev-parse", "--is-inside-work-tree"])?;
        if is_wt != "true" {
            return Err(WorktreeError::NotAGitRepository(self.repo_root.clone()));
        }
        Ok(())
    }

    /// Create an isolated worktree lease for an agent.
    pub fn create_lease(
        &self,
        run_id: RunId,
        agent_id: AgentId,
        explicit_branch: Option<&str>,
    ) -> Result<WorktreeLease, WorktreeError> {
        // Git scans other worktree entries during add/remove, including entries
        // another add has not finished writing. Keep administration and lease
        // publication atomic across clones of this manager.
        let mut leases = self.leases.write().map_err(|_| {
            WorktreeError::GitError("Failed to acquire write lock on leases".into())
        })?;
        self.validate_git_repo()?;

        // UUIDv7's leading characters encode time, so the first eight characters
        // collide for agents created near one another. Keep the full ID in both
        // the branch and path to make concurrent leases distinct.
        let agent_key = agent_id.to_string();
        let branch = explicit_branch
            .map(str::to_string)
            .unwrap_or_else(|| format!("davinci/agent/{agent_key}"));

        // 1. Validate branch is not held by another active lease
        for existing in leases.values() {
            if existing.branch == branch {
                return Err(WorktreeError::BranchInUse(branch));
            }
        }

        // 2. Query current base head sha
        let base_head = run_git(&self.repo_root, &["rev-parse", "HEAD"])?;

        // 3. Determine deterministic worktree path outside tracked paths
        let wt_path = self
            .worktree_root
            .join(format!("wt-{}-{agent_key}", &run_id.to_string()[..8]));

        if wt_path.exists() {
            return Err(WorktreeError::PathAlreadyExists(wt_path));
        }

        // Ensure parent directory exists
        if let Some(parent) = wt_path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| WorktreeError::IoError(format!("Failed to create parent dir: {e}")))?;
        }

        // 4. Create git worktree
        let wt_path_str = wt_path
            .to_str()
            .ok_or_else(|| WorktreeError::IoError("Worktree path is not valid unicode".into()))?;

        // Check if branch already exists in git
        let branch_exists = Command::new("git")
            .current_dir(&self.repo_root)
            .args(["-c", "safe.bareRepository=explicit"])
            .args(["rev-parse", "--verify", &branch])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);

        if branch_exists {
            run_git(&self.repo_root, &["worktree", "add", wt_path_str, &branch])?;
        } else {
            run_git(
                &self.repo_root,
                &["worktree", "add", "-b", &branch, wt_path_str, &base_head],
            )?;
        }

        let lease = WorktreeLease {
            agent_id,
            run_id,
            path: wt_path.clone(),
            branch,
            base_head,
            created_at_ms: now_ms(),
            repo_root: Some(self.repo_root.clone()),
            generation: self.generations.fetch_add(1, Ordering::SeqCst) + 1,
        };

        // 5. Record active lease, then unlock before invoking subscribers.
        leases.insert(wt_path.clone(), lease.clone());
        drop(leases);

        // 6. Emit WorktreeCreated event
        if let Some(bus) = &self.bus {
            let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
            let envelope = RuntimeEventEnvelope::new(
                seq,
                run_id,
                None,
                Some(agent_id),
                None,
                RuntimeEvent::WorktreeCreated { path: wt_path },
            );
            bus.emit_observe(envelope);
        }

        Ok(lease)
    }

    /// Snapshot committed, staged, unstaged and untracked code without changing
    /// the worker's index or checkout. A ref keeps the handoff alive across GC.
    pub fn snapshot_code(&self, lease: &WorktreeLease) -> Result<String, WorktreeError> {
        if self.get_lease(&lease.path).as_ref() != Some(lease) {
            return Err(WorktreeError::StaleLease(lease.path.clone()));
        }
        let git_dir = run_git(&lease.path, &["rev-parse", "--absolute-git-dir"])?;
        let index = PathBuf::from(git_dir).join(format!("handoff-{}.index", uuid::Uuid::new_v4()));
        struct IndexFile(PathBuf);
        impl Drop for IndexFile {
            fn drop(&mut self) {
                let _ = std::fs::remove_file(&self.0);
            }
        }
        let _index_file = IndexFile(index.clone());
        let git = |args: &[&str]| -> Result<String, WorktreeError> {
            let output = Command::new("git")
                .current_dir(&lease.path)
                .env("GIT_INDEX_FILE", &index)
                .args(["-c", "safe.bareRepository=explicit"])
                .args(args)
                .output()
                .map_err(|e| WorktreeError::GitError(e.to_string()))?;
            if !output.status.success() {
                return Err(WorktreeError::GitError(
                    String::from_utf8_lossy(&output.stderr).into_owned(),
                ));
            }
            Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
        };
        git(&["read-tree", "HEAD"])?;
        git(&["add", "-A", "--", "."])?;
        let tree = git(&["write-tree"])?;
        let head = run_git(&lease.path, &["rev-parse", "HEAD"])?;
        let commit = if tree == run_git(&lease.path, &["rev-parse", "HEAD^{tree}"])? {
            head
        } else {
            git(&[
                "-c",
                "user.name=Da Vinci",
                "-c",
                "user.email=davinci@localhost",
                "commit-tree",
                &tree,
                "-p",
                &head,
                "-m",
                "Workflow code handoff",
            ])?
        };
        let reference = format!("refs/davinci/handoffs/{}/{}", lease.run_id, lease.agent_id);
        run_git(&lease.path, &["update-ref", &reference, &commit])?;
        Ok(commit)
    }

    /// Integrate only into the dependent isolated checkout. Conflicts stop the
    /// launch and remain available for review; the lead checkout is untouched.
    pub fn integrate_code(
        &self,
        lease: &WorktreeLease,
        commits: &[String],
    ) -> Result<(), WorktreeError> {
        if self.get_lease(&lease.path).as_ref() != Some(lease) {
            return Err(WorktreeError::StaleLease(lease.path.clone()));
        }
        for commit in commits {
            if !matches!(commit.len(), 40 | 64) || !commit.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err(WorktreeError::GitError(
                    "invalid code handoff commit".into(),
                ));
            }
            run_git(
                &lease.path,
                &[
                    "-c",
                    "user.name=Da Vinci",
                    "-c",
                    "user.email=davinci@localhost",
                    "-c",
                    if cfg!(windows) {
                        "core.hooksPath=NUL"
                    } else {
                        "core.hooksPath=/dev/null"
                    },
                    "-c",
                    "commit.gpgSign=false",
                    "-c",
                    "merge.autoStash=false",
                    "-c",
                    "rerere.enabled=false",
                    "merge",
                    "--no-edit",
                    "--no-ff",
                    "--no-verify",
                    commit,
                ],
            )?;
            run_git(
                &lease.path,
                &["merge-base", "--is-ancestor", commit, "HEAD"],
            )?;
        }
        Ok(())
    }

    /// Check whether a worktree has uncommitted or unmerged changes.
    /// Evidence that cannot be read counts as dirty: see [`Self::check_dirty`].
    pub fn is_dirty(&self, lease: &WorktreeLease) -> bool {
        self.check_dirty(lease).unwrap_or(true)
    }

    /// Whether the worktree holds work that would be lost by removing it.
    ///
    /// A failing `git status` or `rev-parse` is an error, never "clean":
    /// treating unreadable state as clean let a forced cleanup delete
    /// uncommitted work. Commits made in the worktree count as integrated once
    /// they are reachable from the repository's current HEAD (the branch they
    /// were merged into), or once every one of their patches already exists
    /// there (rebase, cherry-pick or squash). Comparing against the lease's
    /// recorded `base_head` instead made every worker commit look unmerged
    /// forever, since a commit made after the lease is never an ancestor of
    /// the commit the lease started from.
    pub fn check_dirty(&self, lease: &WorktreeLease) -> Result<bool, WorktreeError> {
        if !lease.path.exists() {
            return Ok(false);
        }
        if !run_git(&lease.path, &["status", "--porcelain"])?.is_empty() {
            return Ok(true);
        }
        let current_head = run_git(&lease.path, &["rev-parse", "HEAD"])?;
        if current_head.is_empty() || current_head == lease.base_head {
            return Ok(false);
        }
        let integrated = Command::new("git")
            .current_dir(&self.repo_root)
            .args(["-c", "safe.bareRepository=explicit"])
            .args(["merge-base", "--is-ancestor", &current_head, "HEAD"])
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false);
        if integrated {
            return Ok(false);
        }
        // Not an ancestor: still integrated if every patch already landed.
        let cherry = run_git(&self.repo_root, &["cherry", "HEAD", &current_head])?;
        Ok(cherry.lines().any(|line| line.starts_with('+')))
    }

    /// Release an existing worktree lease.
    /// A dirty worktree is preserved and returns `DirtyWorktreePreserved`;
    /// state that cannot be read counts as dirty. `force` does not override
    /// that: uncommitted work is never deleted. No production caller passes
    /// `true`, so the parameter is kept only for the signature.
    pub fn release_lease(&self, lease: &WorktreeLease, _force: bool) -> Result<(), WorktreeError> {
        let mut leases = self.leases.write().map_err(|_| {
            WorktreeError::GitError("Failed to acquire write lock on leases".into())
        })?;
        // The manager's record is the authority, not the caller's struct: a
        // newer lease on the same path must survive a stale holder's release,
        // even a forced one.
        if leases
            .get(&lease.path)
            .is_some_and(|recorded| recorded != lease)
        {
            return Err(WorktreeError::StaleLease(lease.path.clone()));
        }
        // Unreadable state preserves the worktree too.
        if self.check_dirty(lease)? {
            return Err(WorktreeError::DirtyWorktreePreserved {
                path: lease.path.clone(),
            });
        }

        // Remove worktree via git
        let wt_path_str = lease
            .path
            .to_str()
            .ok_or_else(|| WorktreeError::IoError("Worktree path is not valid unicode".into()))?;

        // Anything that survives teardown is reported and the lease is kept, so
        // the caller can retry and nothing is forgotten while still on disk.
        let mut left_path = None;
        let mut left_branch = None;
        let mut details = Vec::new();

        let removal = lease.path.exists().then(|| {
            run_git(
                &self.repo_root,
                &["worktree", "remove", "--force", wt_path_str],
            )
        });
        // Registration is checked whatever happened to the directory: git can
        // delete the files and still fail to delete its own metadata. A failed
        // check is not proof of absence, so it keeps the lease like a leftover.
        match self.is_registered(&lease.path) {
            Ok(false) => {
                // Git leaving files behind after dropping its registration is
                // cleaned up here.
                if lease.path.exists() {
                    if let Err(error) = std::fs::remove_dir_all(&lease.path) {
                        details.push(format!("cannot delete directory: {error}"));
                    }
                }
            }
            Ok(true) if lease.path.exists() => {
                // A worktree git still owns (locked, in use) is left alone:
                // deleting its files would orphan the registration.
                details.push(match &removal {
                    Some(Err(error)) => error.to_string(),
                    _ => "git still lists the worktree".into(),
                });
            }
            Ok(true) => {
                if let Err(error) = self.remove_registration(&lease.path, wt_path_str) {
                    details.push(error);
                }
                match self.is_registered(&lease.path) {
                    Ok(false) => {}
                    Ok(true) => left_path = Some(lease.path.clone()),
                    Err(error) => {
                        details.push(error.to_string());
                        left_path = Some(lease.path.clone());
                    }
                }
            }
            Err(error) => {
                details.push(error.to_string());
                left_path = Some(lease.path.clone());
            }
        }
        if lease.path.exists() {
            left_path = Some(lease.path.clone());
        }

        // Delete branch if it starts with default prefix and force is true or clean
        if lease.branch.starts_with("davinci/agent/") {
            match self.branch_exists(&lease.branch) {
                Ok(false) => {}
                Ok(true) if left_path.is_some() => left_branch = Some(lease.branch.clone()),
                Ok(true) => {
                    if let Err(error) = run_git(&self.repo_root, &["branch", "-D", &lease.branch]) {
                        details.push(error.to_string());
                        left_branch = Some(lease.branch.clone());
                    }
                }
                Err(error) => {
                    details.push(error.to_string());
                    left_branch = Some(lease.branch.clone());
                }
            }
        }

        if left_path.is_some() || left_branch.is_some() || !details.is_empty() {
            return Err(WorktreeError::CleanupIncomplete {
                path: left_path,
                branch: left_branch,
                detail: details.join("; "),
            });
        }

        // Remove from leases, then unlock before invoking subscribers. A
        // repeated release of a cleaned lease finds nothing recorded and
        // stays quiet.
        let was_recorded = leases.remove(&lease.path).is_some();
        drop(leases);

        // Emit WorktreeRemoved event
        if let Some(bus) = self.bus.as_ref().filter(|_| was_recorded) {
            let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
            let envelope = RuntimeEventEnvelope::new(
                seq,
                lease.run_id,
                None,
                Some(lease.agent_id),
                None,
                RuntimeEvent::WorktreeRemoved {
                    path: lease.path.clone(),
                },
            );
            bus.emit_observe(envelope);
        }

        Ok(())
    }

    /// Get an active lease by path.
    pub fn get_lease(&self, path: &Path) -> Option<WorktreeLease> {
        self.leases.read().ok()?.get(path).cloned()
    }

    /// List all currently active leases.
    pub fn list_leases(&self) -> Vec<WorktreeLease> {
        let Ok(leases) = self.leases.read() else {
            return Vec::new();
        };
        leases.values().cloned().collect()
    }

    /// Managed `wt-*` directories and `davinci/agent/*` branches that no live
    /// lease owns, for example after a crash. Report only: it never deletes,
    /// since an orphan may hold unmerged work.
    pub fn scan_orphans(&self) -> Result<Vec<WorktreeOrphan>, WorktreeError> {
        // Creation holds the write lock while it adds the worktree, so a scan
        // never sees a half-created lease as an orphan.
        let leases = self
            .leases
            .read()
            .map_err(|_| WorktreeError::GitError("Failed to acquire read lock on leases".into()))?;
        let mut orphans = Vec::new();
        match std::fs::read_dir(&self.worktree_root) {
            Ok(entries) => {
                for entry in entries {
                    let entry = entry.map_err(|e| WorktreeError::IoError(e.to_string()))?;
                    let is_dir = entry.file_type().is_ok_and(|kind| kind.is_dir());
                    let managed = entry.file_name().to_string_lossy().starts_with("wt-");
                    if is_dir && managed && !leases.contains_key(&entry.path()) {
                        orphans.push(WorktreeOrphan::Directory(entry.path()));
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(WorktreeError::IoError(error.to_string())),
        }
        let refs = run_git(
            &self.repo_root,
            &[
                "for-each-ref",
                "--format=%(refname)",
                "refs/heads/davinci/agent/",
            ],
        )?;
        for branch in refs.lines().filter_map(|r| r.strip_prefix("refs/heads/")) {
            if !leases.values().any(|lease| lease.branch == branch) {
                orphans.push(WorktreeOrphan::Branch(branch.to_string()));
            }
        }
        // Git's own registrations under the managed prefix, which outlive a
        // deleted directory and keep its branch checked out.
        let root = format!("{}/", path_key(&self.worktree_root));
        let leased: HashSet<String> = leases.keys().map(|path| path_key(path)).collect();
        for listed in self.registered_worktrees()? {
            let key = path_key(&listed);
            if key.starts_with(&root) && !leased.contains(&key) {
                orphans.push(WorktreeOrphan::Registration(listed));
            }
        }
        orphans.sort_by_key(|orphan| format!("{orphan:?}"));
        Ok(orphans)
    }

    /// `Ok(false)` only when git reports the ref missing (exit 1, no message).
    /// Any other failure is an error: a broken check is not an absent branch.
    fn branch_exists(&self, branch: &str) -> Result<bool, WorktreeError> {
        let output = Command::new("git")
            .current_dir(&self.repo_root)
            .args(["-c", "safe.bareRepository=explicit"])
            .args(["rev-parse", "--verify", "--quiet"])
            .arg(format!("refs/heads/{branch}"))
            .output()
            .map_err(|e| WorktreeError::GitError(format!("Failed to execute git: {e}")))?;
        if output.status.success() {
            return Ok(true);
        }
        if output.status.code() == Some(1) && output.stderr.iter().all(u8::is_ascii_whitespace) {
            return Ok(false);
        }
        Err(WorktreeError::GitError(format!(
            "cannot check branch {branch} (status {:?}): {}",
            output.status.code(),
            String::from_utf8_lossy(&output.stderr).trim()
        )))
    }

    /// Drop the registration of one worktree whose directory is gone. Never
    /// `git worktree prune`: it deletes every stale registration in the
    /// repository, including a worktree whose directory is only temporarily
    /// missing. Asks git first; failing that, deletes the one admin directory
    /// whose `gitdir` file points at `path`, unless it is locked.
    fn remove_registration(&self, path: &Path, path_str: &str) -> Result<(), String> {
        let refused = match run_git(
            &self.repo_root,
            &["worktree", "remove", "--force", path_str],
        ) {
            Ok(_) => return Ok(()),
            Err(error) => error.to_string(),
        };
        let common = run_git(&self.repo_root, &["rev-parse", "--git-common-dir"])
            .map_err(|error| format!("{refused}; {error}"))?;
        let admin_root = self.repo_root.join(common).join("worktrees");
        let wanted = path_key(path);
        let entries = std::fs::read_dir(&admin_root)
            .map_err(|error| format!("{refused}; cannot read {}: {error}", admin_root.display()))?;
        for admin in entries.flatten().map(|entry| entry.path()) {
            let Ok(gitdir) = std::fs::read(admin.join("gitdir")) else {
                continue;
            };
            // A relative `gitdir` is relative to the admin directory that
            // holds it; `path_key` would resolve it against the process cwd.
            let gitdir = path_from_git_bytes(gitdir.trim_ascii_end());
            let gitdir = if gitdir.is_relative() {
                admin.join(gitdir)
            } else {
                gitdir
            };
            let points_here = gitdir.parent().is_some_and(|dir| path_key(dir) == wanted);
            if !points_here {
                continue;
            }
            if admin.join("locked").exists() {
                return Err(format!("{refused}; the registration is locked"));
            }
            return std::fs::remove_dir_all(&admin)
                .map_err(|error| format!("{refused}; cannot delete {}: {error}", admin.display()));
        }
        Err(format!("{refused}; no registration points at the lease"))
    }

    /// Whether git still lists `path` as a worktree of the repository.
    /// Reads the NUL-separated listing: plain `--porcelain` quotes unusual
    /// path characters, which would never compare equal to the real path.
    fn is_registered(&self, path: &Path) -> Result<bool, WorktreeError> {
        let wanted = path_key(path);
        Ok(self
            .registered_worktrees()?
            .iter()
            .any(|listed| path_key(listed) == wanted))
    }

    /// Every worktree path git lists for the repository, the main one included.
    fn registered_worktrees(&self) -> Result<Vec<PathBuf>, WorktreeError> {
        let output = Command::new("git")
            .current_dir(&self.repo_root)
            .args(["-c", "safe.bareRepository=explicit"])
            .args(["worktree", "list", "--porcelain", "-z"])
            .output()
            .map_err(|e| WorktreeError::GitError(format!("Failed to execute git: {e}")))?;
        if !output.status.success() {
            return Err(WorktreeError::GitError(format!(
                "cannot list worktrees (status {:?}): {}",
                output.status.code(),
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        Ok(output
            .stdout
            .split(|byte| *byte == 0)
            .filter_map(|field| field.strip_prefix(b"worktree "))
            .map(path_from_git_bytes)
            .collect())
    }

    pub fn repo_root(&self) -> &Path {
        &self.repo_root
    }

    pub fn worktree_root(&self) -> &Path {
        &self.worktree_root
    }
}

impl WorktreeLease {
    /// Create a filesystem boundary policy enforcing mutations to this worktree
    /// while preserving git metadata access back to the repository root.
    pub fn boundary_policy(
        &self,
        read_policy: crate::permission::ReadOutsideRootPolicy,
    ) -> crate::permission::FilesystemBoundaryPolicy {
        crate::permission::FilesystemBoundaryPolicy {
            root: Some(self.path.clone()),
            repo_root: self.repo_root.clone(),
            read_outside_root: read_policy,
            enforce_root_for_mutations: true,
            allow_git_metadata: true,
            // A lease is isolated: the user's extra roots never follow it.
            extra_roots: Vec::new(),
        }
    }

    /// Check if target path is safely within this worktree's filesystem boundary.
    pub fn is_path_within_boundary(&self, target: &Path) -> bool {
        let (outside_lexical, symlink_escape) =
            crate::permission::check_path_boundary(&self.path, target);
        !outside_lexical && !symlink_escape
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::bus::{RuntimeDecision, RuntimeSubscriber};
    use std::sync::Mutex;
    use tempfile::tempdir;

    struct EventCapture {
        events: Arc<Mutex<Vec<RuntimeEvent>>>,
    }

    impl RuntimeSubscriber for EventCapture {
        fn on_event(&self, event: &RuntimeEventEnvelope) -> RuntimeDecision {
            self.events.lock().unwrap().push(event.payload.clone());
            RuntimeDecision::Continue
        }
    }

    #[test]
    fn code_snapshot_preserves_source_index_and_hands_off_binary_deletions_and_commits() {
        let repo = init_temp_git_repo();
        let trees = tempdir().unwrap();
        let manager = WorktreeManager::new(repo.path(), trees.path());
        let run = RunId::new();
        let source = manager.create_lease(run, AgentId::new(), None).unwrap();
        std::fs::write(source.path.join("committed.txt"), "committed").unwrap();
        run_git(&source.path, &["add", "."]).unwrap();
        run_git(&source.path, &["commit", "-m", "worker commit"]).unwrap();
        std::fs::write(source.path.join("staged.txt"), "staged").unwrap();
        run_git(&source.path, &["add", "."]).unwrap();
        std::fs::write(source.path.join("staged.txt"), "unstaged").unwrap();
        std::fs::remove_file(source.path.join("README.md")).unwrap();
        std::fs::write(source.path.join("binary.dat"), [0, 255, 10, 13]).unwrap();
        let before = run_git(&source.path, &["status", "--porcelain"]).unwrap();
        let head = run_git(&source.path, &["rev-parse", "HEAD"]).unwrap();
        let commit = manager.snapshot_code(&source).unwrap();
        assert_eq!(
            run_git(&source.path, &["status", "--porcelain"]).unwrap(),
            before
        );
        assert_eq!(run_git(&source.path, &["rev-parse", "HEAD"]).unwrap(), head);
        assert_eq!(
            run_git(&source.path, &["show", ":staged.txt"]).unwrap(),
            "staged"
        );
        let target = manager.create_lease(run, AgentId::new(), None).unwrap();
        manager.integrate_code(&target, &[commit]).unwrap();
        assert_eq!(
            std::fs::read_to_string(target.path.join("committed.txt")).unwrap(),
            "committed"
        );
        assert_eq!(
            std::fs::read_to_string(target.path.join("staged.txt")).unwrap(),
            "unstaged"
        );
        assert_eq!(
            std::fs::read(target.path.join("binary.dat")).unwrap(),
            [0, 255, 10, 13]
        );
        assert!(!target.path.join("README.md").exists());
    }

    #[test]
    fn git_toplevel_resolves_a_subdirectory_to_the_repository_root() {
        let repo = init_temp_git_repo();
        let nested = repo.path().join("crates").join("inner");
        std::fs::create_dir_all(&nested).unwrap();
        let root = git_toplevel(&nested).expect("inside a checkout");
        assert_eq!(
            std::fs::canonicalize(root).unwrap(),
            std::fs::canonicalize(repo.path()).unwrap()
        );
        let outside = tempdir().unwrap();
        assert!(git_toplevel(outside.path()).is_none());
    }

    fn init_temp_git_repo() -> tempfile::TempDir {
        init_git_repo_in(tempdir().unwrap())
    }

    fn init_git_repo_in(dir: tempfile::TempDir) -> tempfile::TempDir {
        let path = dir.path();

        Command::new("git")
            .current_dir(path)
            .args(["init"])
            .output()
            .unwrap();
        Command::new("git")
            .current_dir(path)
            .args(["config", "user.name", "Davinci Test"])
            .output()
            .unwrap();
        Command::new("git")
            .current_dir(path)
            .args(["config", "user.email", "test@davinci.local"])
            .output()
            .unwrap();

        let readme = path.join("README.md");
        std::fs::write(&readme, "# Main Repo\n").unwrap();

        Command::new("git")
            .current_dir(path)
            .args(["add", "README.md"])
            .output()
            .unwrap();
        Command::new("git")
            .current_dir(path)
            .args(["commit", "-m", "Initial commit"])
            .output()
            .unwrap();

        dir
    }

    #[test]
    fn parallel_leases_cannot_claim_the_same_branch() {
        let repo = init_temp_git_repo();
        let trees = tempdir().unwrap();
        let manager = WorktreeManager::new(repo.path(), trees.path());
        let barrier = Arc::new(std::sync::Barrier::new(8));
        let results = std::thread::scope(|scope| {
            let workers: Vec<_> = (0..8)
                .map(|_| {
                    let manager = manager.clone();
                    let barrier = barrier.clone();
                    scope.spawn(move || {
                        barrier.wait();
                        manager.create_lease(RunId::new(), AgentId::new(), Some("shared-worker"))
                    })
                })
                .collect();
            workers
                .into_iter()
                .map(|worker| worker.join().unwrap())
                .collect::<Vec<_>>()
        });
        assert_eq!(
            results.iter().filter(|result| result.is_ok()).count(),
            1,
            "{results:?}"
        );
        for result in results {
            match result {
                Ok(lease) => manager.release_lease(&lease, false).unwrap(),
                Err(error) => assert_eq!(error, WorktreeError::BranchInUse("shared-worker".into())),
            }
        }
        assert!(manager.list_leases().is_empty());
    }

    #[test]
    fn parallel_creates_and_removals_leave_no_partial_git_metadata() {
        let repo = init_temp_git_repo();
        let trees = tempdir().unwrap();
        let manager = WorktreeManager::new(repo.path(), trees.path());
        let barrier = Arc::new(std::sync::Barrier::new(4));
        std::thread::scope(|scope| {
            let workers: Vec<_> = (0..4)
                .map(|_| {
                    let manager = manager.clone();
                    let barrier = barrier.clone();
                    scope.spawn(move || {
                        barrier.wait();
                        for _ in 0..3 {
                            let lease = manager
                                .create_lease(RunId::new(), AgentId::new(), None)
                                .unwrap();
                            manager.release_lease(&lease, false).unwrap();
                        }
                    })
                })
                .collect();
            for worker in workers {
                worker.join().unwrap();
            }
        });
        assert!(manager.list_leases().is_empty());
        let listed = run_git(repo.path(), &["worktree", "list", "--porcelain"]).unwrap();
        assert_eq!(
            listed
                .lines()
                .filter(|line| line.starts_with("worktree "))
                .count(),
            1,
            "{listed}"
        );
    }

    #[test]
    fn test_create_and_release_clean_lease_with_temp_repo() {
        let repo_dir = init_temp_git_repo();
        let wt_dir = tempdir().unwrap();

        let bus = RuntimeBus::new();
        let events = Arc::new(Mutex::new(Vec::new()));
        bus.subscribe(Arc::new(EventCapture {
            events: Arc::clone(&events),
        }));

        let manager = WorktreeManager::new(repo_dir.path(), wt_dir.path()).with_bus(bus);
        let run_id = RunId::new();
        let agent_id = AgentId::new();

        // 1. Create lease
        let lease = manager.create_lease(run_id, agent_id, None).unwrap();
        assert!(lease.path.exists());
        assert!(lease.path.join("README.md").exists());
        assert!(lease.branch.starts_with("davinci/agent/"));

        let captured_create = events.lock().unwrap();
        assert!(captured_create
            .iter()
            .any(|e| matches!(e, RuntimeEvent::WorktreeCreated { path } if path == &lease.path)));
        drop(captured_create);

        // 2. Release clean lease
        manager.release_lease(&lease, false).unwrap();
        assert!(!lease.path.exists());

        let captured_remove = events.lock().unwrap();
        assert!(captured_remove
            .iter()
            .any(|e| matches!(e, RuntimeEvent::WorktreeRemoved { path } if path == &lease.path)));
    }

    #[test]
    fn uuid_v7_agents_with_the_same_timestamp_get_distinct_worktrees() {
        let repo_dir = init_temp_git_repo();
        let wt_dir = tempdir().unwrap();
        let manager = WorktreeManager::new(repo_dir.path(), wt_dir.path());
        let run_id = RunId::from_uuid(
            uuid::Uuid::parse_str("01901234-3333-7333-8333-333333333333").unwrap(),
        );
        let agent1 = AgentId::from_uuid(
            uuid::Uuid::parse_str("01901234-1111-7111-8111-111111111111").unwrap(),
        );
        let agent2 = AgentId::from_uuid(
            uuid::Uuid::parse_str("01901234-2222-7222-8222-222222222222").unwrap(),
        );

        let lease1 = manager.create_lease(run_id, agent1, None).unwrap();
        let lease2 = manager.create_lease(run_id, agent2, None).unwrap();

        assert_ne!(lease1.branch, lease2.branch);
        assert_ne!(lease1.path, lease2.path);
        manager.release_lease(&lease1, false).unwrap();
        manager.release_lease(&lease2, false).unwrap();
    }

    #[test]
    fn test_conflict_when_two_agents_request_same_branch() {
        let repo_dir = init_temp_git_repo();
        let wt_dir = tempdir().unwrap();

        let manager = WorktreeManager::new(repo_dir.path(), wt_dir.path());
        let run_id = RunId::new();
        let agent1 = AgentId::new();
        let agent2 = AgentId::new();

        let explicit_branch = "feature-parallel-work";

        let lease1 = manager
            .create_lease(run_id, agent1, Some(explicit_branch))
            .unwrap();
        assert_eq!(lease1.branch, explicit_branch);

        // Second agent requests same branch -> rejected deterministically
        let err = manager
            .create_lease(run_id, agent2, Some(explicit_branch))
            .unwrap_err();
        assert_eq!(err, WorktreeError::BranchInUse(explicit_branch.to_string()));

        // Cleanup lease1
        manager.release_lease(&lease1, true).unwrap();
    }

    #[test]
    fn test_dirty_worktree_preservation() {
        let repo_dir = init_temp_git_repo();
        let wt_dir = tempdir().unwrap();

        let manager = WorktreeManager::new(repo_dir.path(), wt_dir.path());
        let run_id = RunId::new();
        let agent = AgentId::new();

        let lease = manager.create_lease(run_id, agent, None).unwrap();

        // Introduce uncommitted change in worktree
        let dirty_file = lease.path.join("uncommitted.txt");
        std::fs::write(&dirty_file, "Work in progress...").unwrap();

        assert!(manager.is_dirty(&lease));

        // Attempt to release without force: must return DirtyWorktreePreserved and NOT delete directory
        let err = manager.release_lease(&lease, false).unwrap_err();
        assert_eq!(
            err,
            WorktreeError::DirtyWorktreePreserved {
                path: lease.path.clone()
            }
        );
        assert!(lease.path.exists());
        assert!(dirty_file.exists());

        // `force` never overrides uncommitted work either.
        let err = manager.release_lease(&lease, true).unwrap_err();
        assert!(
            matches!(err, WorktreeError::DirtyWorktreePreserved { .. }),
            "{err:?}"
        );
        assert!(dirty_file.exists());

        // Once the work is gone the same lease releases, forced or not.
        std::fs::remove_file(&dirty_file).unwrap();
        manager.release_lease(&lease, true).unwrap();
        assert!(!lease.path.exists());
    }

    fn git_in(path: &Path, args: &[&str]) {
        let out = Command::new("git")
            .current_dir(path)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// WOR-53: a status that cannot run must preserve the lease, not read as clean.
    #[test]
    fn a_failing_git_status_preserves_the_lease_instead_of_reading_clean() {
        let repo_dir = init_temp_git_repo();
        let wt_dir = tempdir().unwrap();
        let manager = WorktreeManager::new(repo_dir.path(), wt_dir.path());
        let lease = manager
            .create_lease(RunId::new(), AgentId::new(), None)
            .unwrap();
        let precious = lease.path.join("wip.txt");
        std::fs::write(&precious, "uncommitted").unwrap();
        // Break the worktree's git link so every git command in it fails.
        std::fs::write(lease.path.join(".git"), "gitdir: /nonexistent/gitdir\n").unwrap();
        assert!(manager.check_dirty(&lease).is_err());
        assert!(manager.is_dirty(&lease), "unreadable state is dirty");
        let err = manager.release_lease(&lease, false).unwrap_err();
        assert!(matches!(err, WorktreeError::GitError(_)), "{err:?}");
        assert!(precious.exists(), "nothing was deleted");
        assert!(manager.get_lease(&lease.path).is_none() || lease.path.exists());
    }

    fn branch_exists(repo: &Path, branch: &str) -> bool {
        Command::new("git")
            .current_dir(repo)
            .args(["rev-parse", "--verify", "--quiet"])
            .arg(format!("refs/heads/{branch}"))
            .output()
            .unwrap()
            .status
            .success()
    }

    /// G-03: git refusing to remove a worktree is not a successful teardown.
    #[test]
    fn a_failing_worktree_removal_keeps_the_lease_and_names_what_is_left() {
        let repo_dir = init_temp_git_repo();
        let wt_dir = tempdir().unwrap();
        let manager = WorktreeManager::new(repo_dir.path(), wt_dir.path());
        let lease = manager
            .create_lease(RunId::new(), AgentId::new(), None)
            .unwrap();
        // A locked worktree makes `git worktree remove` fail even when forced once.
        git_in(
            repo_dir.path(),
            &["worktree", "lock", lease.path.to_str().unwrap()],
        );

        let err = manager.release_lease(&lease, true).unwrap_err();
        match &err {
            WorktreeError::CleanupIncomplete { path, branch, .. } => {
                assert_eq!(path.as_deref(), Some(lease.path.as_path()));
                assert_eq!(branch.as_deref(), Some(lease.branch.as_str()));
            }
            other => panic!("expected CleanupIncomplete, got {other:?}"),
        }
        assert!(err.to_string().contains(&lease.branch), "{err}");
        assert!(lease.path.exists(), "a refused removal deletes nothing");
        assert_eq!(manager.get_lease(&lease.path).as_ref(), Some(&lease));

        // Once the obstacle is gone the same lease can be released.
        git_in(
            repo_dir.path(),
            &["worktree", "unlock", lease.path.to_str().unwrap()],
        );
        manager.release_lease(&lease, true).unwrap();
        assert!(!lease.path.exists());
        assert!(!branch_exists(repo_dir.path(), &lease.branch));
        assert!(manager.get_lease(&lease.path).is_none());
    }

    /// Git prints unusual path characters quoted in plain `--porcelain`; a
    /// locked worktree under such a path must still be recognised as owned.
    #[test]
    fn a_locked_worktree_under_a_path_git_quotes_is_still_recognised() {
        let repo_dir = init_temp_git_repo();
        let wt_dir = tempdir().unwrap();
        // A user-level `core.quotePath=false` would hide the quoting.
        git_in(repo_dir.path(), &["config", "core.quotePath", "true"]);
        // A TAB is the character most likely to be quoted in plain
        // `--porcelain` on unix. Windows filenames cannot hold one.
        let name = if cfg!(unix) {
            "sp ace\tq \u{e9}\u{4e16}"
        } else {
            "sp ace \u{e9}\u{4e16}"
        };
        let root = wt_dir.path().join(name);
        let manager = WorktreeManager::new(repo_dir.path(), &root);
        let lease = manager
            .create_lease(RunId::new(), AgentId::new(), None)
            .unwrap();
        // Whether plain `--porcelain` quotes the path depends on the git
        // version; the `-z` listing is parsed independent of that policy.
        assert!(manager.is_registered(&lease.path).unwrap());
        git_in(
            repo_dir.path(),
            &["worktree", "lock", lease.path.to_str().unwrap()],
        );

        let err = manager.release_lease(&lease, true).unwrap_err();
        assert!(
            matches!(err, WorktreeError::CleanupIncomplete { .. }),
            "{err:?}"
        );
        assert!(lease.path.join("README.md").exists(), "locked files stay");
        assert_eq!(manager.get_lease(&lease.path).as_ref(), Some(&lease));

        git_in(
            repo_dir.path(),
            &["worktree", "unlock", lease.path.to_str().unwrap()],
        );
        manager.release_lease(&lease, true).unwrap();
        assert!(!lease.path.exists());
    }

    /// Keeps git from deleting the metadata of the one registered worktree:
    /// open handles on Windows, a read-only directory on unix. Returns `None`
    /// where the block cannot work (unix as root).
    fn block_admin_deletion(repo: &Path) -> Option<Box<dyn std::any::Any>> {
        let admin = std::fs::read_dir(repo.join(".git").join("worktrees"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            // FILE_SHARE_READ without FILE_SHARE_DELETE: git's unlink fails.
            let held: Vec<_> = std::fs::read_dir(&admin)
                .unwrap()
                .map(|entry| entry.unwrap().path())
                .filter(|path| path.is_file())
                .map(|path| {
                    std::fs::OpenOptions::new()
                        .read(true)
                        .share_mode(1)
                        .open(path)
                        .unwrap()
                })
                .collect();
            Some(Box::new(held))
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            struct Restore(PathBuf);
            impl Drop for Restore {
                fn drop(&mut self) {
                    let _ = std::fs::set_permissions(&self.0, PermissionsExt::from_mode(0o755));
                }
            }
            std::fs::set_permissions(&admin, PermissionsExt::from_mode(0o555)).unwrap();
            if std::fs::File::create(admin.join("probe")).is_ok() {
                std::fs::remove_file(admin.join("probe")).unwrap();
                return None;
            }
            Some(Box::new(Restore(admin)))
        }
    }

    /// Git can delete a worktree's files and still fail to delete its own
    /// metadata; that registration must keep the lease.
    #[test]
    fn a_surviving_registration_keeps_the_lease_even_when_the_directory_is_gone() {
        let repo_dir = init_temp_git_repo();
        let wt_dir = tempdir().unwrap();
        let manager = WorktreeManager::new(repo_dir.path(), wt_dir.path());
        let lease = manager
            .create_lease(RunId::new(), AgentId::new(), None)
            .unwrap();
        let Some(block) = block_admin_deletion(repo_dir.path()) else {
            eprintln!("skipped: permissions cannot block git here");
            return;
        };

        let err = manager.release_lease(&lease, false).unwrap_err();
        match &err {
            WorktreeError::CleanupIncomplete { path, .. } => {
                assert!(path.is_some(), "the registration survived: {err}");
            }
            other => panic!("expected CleanupIncomplete, got {other:?}"),
        }
        assert_eq!(manager.get_lease(&lease.path).as_ref(), Some(&lease));

        drop(block);
        manager.release_lease(&lease, true).unwrap();
        assert!(!manager.is_registered(&lease.path).unwrap());
        assert!(!branch_exists(repo_dir.path(), &lease.branch));
        assert!(manager.get_lease(&lease.path).is_none());
    }

    /// Releasing one lease whose directory is gone must not delete the
    /// registration of an unrelated worktree that is merely missing too.
    #[test]
    fn releasing_a_lease_with_a_missing_directory_keeps_unrelated_registrations() {
        let repo_dir = init_temp_git_repo();
        let wt_dir = tempdir().unwrap();
        let manager = WorktreeManager::new(repo_dir.path(), wt_dir.path());
        let lease = manager
            .create_lease(RunId::new(), AgentId::new(), None)
            .unwrap();
        let bystander = manager
            .create_lease(RunId::new(), AgentId::new(), None)
            .unwrap();
        std::fs::remove_dir_all(&lease.path).unwrap();
        std::fs::remove_dir_all(&bystander.path).unwrap();

        manager.release_lease(&lease, true).unwrap();

        assert!(!manager.is_registered(&lease.path).unwrap());
        assert!(!branch_exists(repo_dir.path(), &lease.branch));
        assert!(
            manager.is_registered(&bystander.path).unwrap(),
            "the other worktree keeps its registration"
        );
        assert!(branch_exists(repo_dir.path(), &bystander.branch));
        assert_eq!(
            manager.get_lease(&bystander.path).as_ref(),
            Some(&bystander)
        );
    }

    /// A relative `gitdir` is relative to the registration's own directory,
    /// not to the process cwd. Resolved against the cwd, a bystander's path
    /// aliases the lease and its registration is deleted, while the lease's
    /// own locked registration is never recognised.
    #[test]
    fn a_relative_gitdir_is_resolved_against_its_registration_not_the_cwd() {
        let cwd = std::env::current_dir().unwrap();
        let repo_dir = init_git_repo_in(tempfile::tempdir_in(&cwd).unwrap());
        let wt_dir = tempfile::tempdir_in(&cwd).unwrap();
        let manager = WorktreeManager::new(repo_dir.path(), wt_dir.path());
        let lease = manager
            .create_lease(RunId::new(), AgentId::new(), None)
            .unwrap();
        let bystander = manager
            .create_lease(RunId::new(), AgentId::new(), None)
            .unwrap();
        git_in(
            repo_dir.path(),
            &["worktree", "lock", lease.path.to_str().unwrap()],
        );
        std::fs::remove_dir_all(&lease.path).unwrap();
        std::fs::remove_dir_all(&bystander.path).unwrap();

        // Both files become relative. The lease's resolves from its admin
        // directory to the lease; the bystander's is written so that it would
        // reach the lease only from the cwd.
        let admin_root = repo_dir.path().join(".git").join("worktrees");
        let (mut lease_admin, mut bystander_admin) = (None, None);
        for admin in std::fs::read_dir(&admin_root).unwrap() {
            let admin = admin.unwrap().path();
            if admin.join("locked").exists() {
                lease_admin = Some(admin);
            } else {
                bystander_admin = Some(admin);
            }
        }
        let (lease_admin, bystander_admin) = (lease_admin.unwrap(), bystander_admin.unwrap());
        let up = |admin: &Path| {
            let depth = admin.strip_prefix(&cwd).unwrap().components().count();
            PathBuf::from_iter(std::iter::repeat_n("..", depth))
        };
        let lease_git = lease.path.join(".git");
        let from_admin = up(&lease_admin).join(lease_git.strip_prefix(&cwd).unwrap());
        let from_cwd = lease_git.strip_prefix(&cwd).unwrap();
        std::fs::write(
            lease_admin.join("gitdir"),
            format!("{}\n", from_admin.to_string_lossy().replace('\\', "/")),
        )
        .unwrap();
        std::fs::write(
            bystander_admin.join("gitdir"),
            format!("{}\n", from_cwd.to_string_lossy().replace('\\', "/")),
        )
        .unwrap();

        let err = manager.release_lease(&lease, true).unwrap_err();
        assert!(
            matches!(&err, WorktreeError::CleanupIncomplete { detail, .. }
                if detail.contains("the registration is locked")),
            "{err:?}"
        );
        assert!(
            lease_admin.exists(),
            "a locked registration is never deleted"
        );
        assert!(
            bystander_admin.exists(),
            "a relative path that only the cwd would resolve to the lease is not the lease"
        );
        assert_eq!(manager.get_lease(&lease.path).as_ref(), Some(&lease));
    }

    /// A git command that fails is not evidence that the thing is gone.
    #[test]
    fn a_failing_inspection_keeps_the_lease_instead_of_assuming_absence() {
        let repo_dir = init_temp_git_repo();
        let wt_dir = tempdir().unwrap();
        let manager = WorktreeManager::new(repo_dir.path(), wt_dir.path());
        let lease = manager
            .create_lease(RunId::new(), AgentId::new(), None)
            .unwrap();
        // Directory already gone, registration and branch still present.
        std::fs::remove_dir_all(&lease.path).unwrap();
        let head_path = repo_dir.path().join(".git").join("HEAD");
        let head = std::fs::read(&head_path).unwrap();
        std::fs::write(&head_path, "garbage\n").unwrap();

        let err = manager.release_lease(&lease, true).unwrap_err();
        match &err {
            WorktreeError::CleanupIncomplete { path, detail, .. } => {
                assert!(path.is_some(), "registration state is unknown: {err}");
                assert!(detail.contains("git"), "names the failed check: {err}");
            }
            other => panic!("expected CleanupIncomplete, got {other:?}"),
        }
        assert_eq!(manager.get_lease(&lease.path).as_ref(), Some(&lease));

        std::fs::write(&head_path, head).unwrap();
        manager.release_lease(&lease, true).unwrap();
        assert!(!branch_exists(repo_dir.path(), &lease.branch));
        assert!(manager.get_lease(&lease.path).is_none());
    }

    #[test]
    fn branch_existence_tells_a_missing_ref_from_a_failed_check() {
        let repo_dir = init_temp_git_repo();
        let wt_dir = tempdir().unwrap();
        let manager = WorktreeManager::new(repo_dir.path(), wt_dir.path());
        assert!(!manager
            .branch_exists("davinci/agent/never-created")
            .unwrap());
        let head_path = repo_dir.path().join(".git").join("HEAD");
        let head = std::fs::read(&head_path).unwrap();
        std::fs::write(&head_path, "garbage\n").unwrap();
        assert!(manager
            .branch_exists("davinci/agent/never-created")
            .is_err());
        std::fs::write(&head_path, head).unwrap();
    }

    #[test]
    fn a_failing_branch_delete_is_reported_and_keeps_the_lease() {
        let repo_dir = init_temp_git_repo();
        let wt_dir = tempdir().unwrap();
        let manager = WorktreeManager::new(repo_dir.path(), wt_dir.path());
        let lease = manager
            .create_lease(RunId::new(), AgentId::new(), None)
            .unwrap();
        git_in(
            repo_dir.path(),
            &[
                "worktree",
                "remove",
                "--force",
                lease.path.to_str().unwrap(),
            ],
        );
        // Git refuses to delete the branch checked out in the main checkout.
        git_in(repo_dir.path(), &["checkout", &lease.branch]);

        let err = manager.release_lease(&lease, true).unwrap_err();
        match &err {
            WorktreeError::CleanupIncomplete { path, branch, .. } => {
                assert_eq!(path, &None, "the directory is already gone");
                assert_eq!(branch.as_deref(), Some(lease.branch.as_str()));
            }
            other => panic!("expected CleanupIncomplete, got {other:?}"),
        }
        assert!(branch_exists(repo_dir.path(), &lease.branch));
        assert_eq!(manager.get_lease(&lease.path).as_ref(), Some(&lease));

        git_in(repo_dir.path(), &["checkout", "-"]);
        manager.release_lease(&lease, true).unwrap();
        assert!(!branch_exists(repo_dir.path(), &lease.branch));
        assert!(manager.get_lease(&lease.path).is_none());
    }

    #[test]
    fn releasing_an_already_cleaned_lease_is_idempotent() {
        let repo_dir = init_temp_git_repo();
        let wt_dir = tempdir().unwrap();
        let manager = WorktreeManager::new(repo_dir.path(), wt_dir.path());
        let lease = manager
            .create_lease(RunId::new(), AgentId::new(), None)
            .unwrap();
        manager.release_lease(&lease, false).unwrap();
        manager.release_lease(&lease, false).unwrap();
        manager.release_lease(&lease, true).unwrap();
    }

    #[test]
    fn orphan_scan_reports_unleased_directories_and_branches_without_deleting() {
        let repo_dir = init_temp_git_repo();
        let wt_dir = tempdir().unwrap();
        let manager = WorktreeManager::new(repo_dir.path(), wt_dir.path());
        let leased = manager
            .create_lease(RunId::new(), AgentId::new(), None)
            .unwrap();
        let orphan_dir = wt_dir.path().join("wt-deadbeef-orphan");
        std::fs::create_dir_all(&orphan_dir).unwrap();
        std::fs::write(orphan_dir.join("wip.txt"), "unknown work").unwrap();
        let unrelated = wt_dir.path().join("notes");
        std::fs::create_dir_all(&unrelated).unwrap();
        git_in(repo_dir.path(), &["branch", "davinci/agent/orphan"]);
        git_in(repo_dir.path(), &["branch", "feature/mine"]);

        let orphans = manager.scan_orphans().unwrap();

        assert_eq!(orphans.len(), 2, "{orphans:?}");
        assert!(orphans.contains(&WorktreeOrphan::Directory(orphan_dir.clone())));
        assert!(orphans.contains(&WorktreeOrphan::Branch("davinci/agent/orphan".into())));
        assert!(orphan_dir.join("wip.txt").exists(), "report only");
        assert!(branch_exists(repo_dir.path(), "davinci/agent/orphan"));
        assert!(leased.path.exists());
        assert!(branch_exists(repo_dir.path(), &leased.branch));
    }

    #[test]
    fn orphan_scan_reports_a_registration_whose_directory_and_branch_are_gone() {
        let repo_dir = init_temp_git_repo();
        let wt_dir = tempdir().unwrap();
        let first = WorktreeManager::new(repo_dir.path(), wt_dir.path());
        let lease = first
            .create_lease(RunId::new(), AgentId::new(), None)
            .unwrap();
        std::fs::remove_dir_all(&lease.path).unwrap();
        git_in(
            repo_dir.path(),
            &["update-ref", "-d", &format!("refs/heads/{}", lease.branch)],
        );
        // A fresh manager, as after a crash: no live lease owns the registration.
        let manager = WorktreeManager::new(repo_dir.path(), wt_dir.path());

        let orphans = manager.scan_orphans().unwrap();

        assert_eq!(orphans.len(), 1, "{orphans:?}");
        assert!(
            matches!(&orphans[0], WorktreeOrphan::Registration(path)
                if path_key(path) == path_key(&lease.path)),
            "{orphans:?}"
        );
        assert!(manager.is_registered(&lease.path).unwrap(), "report only");
        // A leased registration is not an orphan.
        assert!(first.scan_orphans().unwrap().is_empty());
    }

    /// WOR-54: once the worker's commit is merged, the lease is clean.
    #[test]
    fn integrated_worker_commits_do_not_keep_the_lease_dirty() {
        let repo_dir = init_temp_git_repo();
        let wt_dir = tempdir().unwrap();
        let manager = WorktreeManager::new(repo_dir.path(), wt_dir.path());
        let lease = manager
            .create_lease(RunId::new(), AgentId::new(), None)
            .unwrap();
        std::fs::write(lease.path.join("work.txt"), "done").unwrap();
        git_in(&lease.path, &["add", "work.txt"]);
        git_in(&lease.path, &["config", "user.name", "w"]);
        git_in(&lease.path, &["config", "user.email", "w@x"]);
        git_in(&lease.path, &["commit", "-m", "worker change"]);
        // Committed but not integrated: still holds unmerged work.
        assert!(manager.is_dirty(&lease));
        assert!(matches!(
            manager.release_lease(&lease, false),
            Err(WorktreeError::DirtyWorktreePreserved { .. })
        ));
        // Integrate into the repository's checked-out branch.
        git_in(
            repo_dir.path(),
            &["merge", "--no-ff", "-m", "merge worker", &lease.branch],
        );
        assert!(!manager.is_dirty(&lease), "merged work is not dirty");
        manager.release_lease(&lease, false).unwrap();
        assert!(!lease.path.exists());
    }

    #[test]
    fn a_squash_integrated_worker_commit_is_also_clean() {
        let repo_dir = init_temp_git_repo();
        let wt_dir = tempdir().unwrap();
        let manager = WorktreeManager::new(repo_dir.path(), wt_dir.path());
        let lease = manager
            .create_lease(RunId::new(), AgentId::new(), None)
            .unwrap();
        std::fs::write(lease.path.join("sq.txt"), "x").unwrap();
        git_in(&lease.path, &["add", "sq.txt"]);
        git_in(&lease.path, &["config", "user.name", "w"]);
        git_in(&lease.path, &["config", "user.email", "w@x"]);
        git_in(&lease.path, &["commit", "-m", "squash me"]);
        git_in(repo_dir.path(), &["merge", "--squash", &lease.branch]);
        git_in(repo_dir.path(), &["commit", "-m", "squashed"]);
        assert!(!manager.is_dirty(&lease));
        manager.release_lease(&lease, false).unwrap();
    }
}
