//! Git worktree manager isolating mutation agents in dedicated working trees.

use std::collections::HashMap;
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
    #[error("git command failed: {0}")]
    GitError(String),
    #[error("io error: {0}")]
    IoError(String),
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
        // Patch IDs used by `git cherry` ignore whitespace and merge-only
        // changes. Instead, compute the exact result of merging the worker
        // HEAD into the main checkout. Only a byte-identical tree proves that
        // cleanup cannot discard committed work. A conflict or unsupported Git
        // command is ambiguous and must preserve the worker worktree.
        let merged_tree = match run_git(
            &self.repo_root,
            &["merge-tree", "--write-tree", "--no-messages", "HEAD", &current_head],
        ) {
            Ok(tree) => tree,
            Err(_) => return Ok(true),
        };
        let main_tree = run_git(&self.repo_root, &["rev-parse", "HEAD^{tree}"])?;
        Ok(merged_tree != main_tree)
    }

    /// Release an existing worktree lease.
    /// If dirty and `force` is false, preserves the worktree directory and returns `DirtyWorktreePreserved`.
    pub fn release_lease(&self, lease: &WorktreeLease, force: bool) -> Result<(), WorktreeError> {
        let mut leases = self.leases.write().map_err(|_| {
            WorktreeError::GitError("Failed to acquire write lock on leases".into())
        })?;
        // The manager's record is the authority, not the caller's struct: a
        // newer lease on the same path must survive a stale holder's release,
        // even a forced one.
        match leases.get(&lease.path) {
            Some(recorded) if recorded == lease => {}
            Some(_) => return Err(WorktreeError::StaleLease(lease.path.clone())),
            None => return Err(WorktreeError::LeaseNotFound(lease.path.clone())),
        }
        // Unreadable state preserves the worktree unless the caller forces.
        let dirty = match self.check_dirty(lease) {
            Ok(dirty) => dirty,
            Err(error) if !force => return Err(error),
            Err(_) => true,
        };
        if dirty && !force {
            return Err(WorktreeError::DirtyWorktreePreserved {
                path: lease.path.clone(),
            });
        }

        // Remove worktree via git
        let wt_path_str = lease
            .path
            .to_str()
            .ok_or_else(|| WorktreeError::IoError("Worktree path is not valid unicode".into()))?;

        if lease.path.exists() {
            // Do not bypass Git's worktree locks or erase a directory that Git
            // refused to remove. Keep the lease on any failed cleanup so it
            // can be inspected and retried safely.
            run_git(
                &self.repo_root,
                &["worktree", "remove", "--force", wt_path_str],
            )?;
            if lease.path.exists() {
                return Err(WorktreeError::GitError(format!(
                    "git removed the worktree registration but {} still exists",
                    lease.path.display()
                )));
            }
        }

        // A failed branch removal is not a successful release: keep the
        // record so the owner can reconcile the incomplete cleanup.
        if lease.branch.starts_with("davinci/agent/") {
            run_git(&self.repo_root, &["branch", "-D", &lease.branch])?;
        }

        // Remove from leases, then unlock before invoking subscribers.
        leases.remove(&lease.path);
        drop(leases);

        // Emit WorktreeRemoved event
        if let Some(bus) = &self.bus {
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
        manager.release_lease(&source, true).unwrap();
        manager.release_lease(&target, true).unwrap();
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
        let dir = tempdir().unwrap();
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

        // Now release with force: true
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

    #[test]
    fn unregistered_lease_cannot_delete_another_managers_worktree() {
        let repo = init_temp_git_repo();
        let trees = tempdir().unwrap();
        let owner = WorktreeManager::new(repo.path(), trees.path());
        let stranger = WorktreeManager::new(repo.path(), trees.path());
        let lease = owner
            .create_lease(RunId::new(), AgentId::new(), None)
            .unwrap();
        let err = stranger.release_lease(&lease, true).unwrap_err();
        assert_eq!(err, WorktreeError::LeaseNotFound(lease.path.clone()));
        assert!(lease.path.join("README.md").exists());
        assert_eq!(owner.get_lease(&lease.path), Some(lease.clone()));
        owner.release_lease(&lease, false).unwrap();
    }

    #[test]
    fn locked_worktree_cannot_be_removed_even_with_force() {
        let repo = init_temp_git_repo();
        let trees = tempdir().unwrap();
        let manager = WorktreeManager::new(repo.path(), trees.path());
        let lease = manager
            .create_lease(RunId::new(), AgentId::new(), None)
            .unwrap();
        let worktree = lease.path.to_str().unwrap();
        git_in(repo.path(), &["worktree", "lock", worktree]);
        let err = manager.release_lease(&lease, true).unwrap_err();
        assert!(matches!(err, WorktreeError::GitError(_)), "{err:?}");
        assert!(lease.path.join("README.md").exists());
        assert_eq!(manager.get_lease(&lease.path), Some(lease.clone()));
        git_in(repo.path(), &["worktree", "unlock", worktree]);
        manager.release_lease(&lease, false).unwrap();
    }

    #[test]
    fn merge_only_changes_are_preserved_even_if_every_non_merge_patch_landed() {
        let repo = init_temp_git_repo();
        let trees = tempdir().unwrap();
        let manager = WorktreeManager::new(repo.path(), trees.path());
        let lease = manager
            .create_lease(RunId::new(), AgentId::new(), None)
            .unwrap();
        git_in(&lease.path, &["config", "user.name", "w"]);
        git_in(&lease.path, &["config", "user.email", "w@x"]);

        git_in(&lease.path, &["checkout", "-b", "side"]);
        std::fs::write(lease.path.join("side.txt"), "side\n").unwrap();
        git_in(&lease.path, &["add", "side.txt"]);
        git_in(&lease.path, &["commit", "-m", "side patch"]);
        let side = run_git(&lease.path, &["rev-parse", "HEAD"]).unwrap();

        git_in(&lease.path, &["checkout", &lease.branch]);
        std::fs::write(lease.path.join("worker.txt"), "worker\n").unwrap();
        git_in(&lease.path, &["add", "worker.txt"]);
        git_in(&lease.path, &["commit", "-m", "worker patch"]);
        let worker = run_git(&lease.path, &["rev-parse", "HEAD"]).unwrap();

        git_in(&lease.path, &["merge", "--no-ff", "-m", "worker merge", "side"]);
        std::fs::write(lease.path.join("merge-only.txt"), "unique merge change\n").unwrap();
        git_in(&lease.path, &["add", "merge-only.txt"]);
        git_in(&lease.path, &["commit", "--amend", "--no-edit"]);

        git_in(repo.path(), &["cherry-pick", &worker]);
        git_in(repo.path(), &["cherry-pick", &side]);
        assert!(!repo.path().join("merge-only.txt").exists());
        assert!(manager.is_dirty(&lease), "merge-only edits must survive cleanup");
        assert!(matches!(
            manager.release_lease(&lease, false),
            Err(WorktreeError::DirtyWorktreePreserved { .. })
        ));
        assert_eq!(
            std::fs::read_to_string(lease.path.join("merge-only.txt")).unwrap(),
            "unique merge change\n"
        );
        manager.release_lease(&lease, true).unwrap();
    }

    #[test]
    fn whitespace_different_patch_is_not_equivalent_for_cleanup() {
        let repo = init_temp_git_repo();
        let trees = tempdir().unwrap();
        let manager = WorktreeManager::new(repo.path(), trees.path());
        let lease = manager
            .create_lease(RunId::new(), AgentId::new(), None)
            .unwrap();
        git_in(&lease.path, &["config", "user.name", "w"]);
        git_in(&lease.path, &["config", "user.email", "w@x"]);
        std::fs::write(lease.path.join("README.md"), "# Main Repo\n\toutput\n").unwrap();
        git_in(&lease.path, &["add", "README.md"]);
        git_in(&lease.path, &["commit", "-m", "worker whitespace"]);
        std::fs::write(repo.path().join("README.md"), "# Main Repo\n    output\n").unwrap();
        git_in(repo.path(), &["add", "README.md"]);
        git_in(repo.path(), &["commit", "-m", "main whitespace"]);
        assert!(manager.is_dirty(&lease), "patch-id ignores whitespace differences");
        assert!(matches!(
            manager.release_lease(&lease, false),
            Err(WorktreeError::DirtyWorktreePreserved { .. })
        ));
        assert!(lease.path.exists());
        manager.release_lease(&lease, true).unwrap();
    }

}
