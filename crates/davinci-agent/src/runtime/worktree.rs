//! Git worktree manager isolating mutation agents in dedicated working trees.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::bus::RuntimeBus;
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

/// Thread-safe manager for ephemeral git worktrees.
#[derive(Clone)]
pub struct WorktreeManager {
    repo_root: PathBuf,
    worktree_root: PathBuf,
    leases: Arc<RwLock<HashMap<PathBuf, WorktreeLease>>>,
    bus: Option<RuntimeBus>,
    seq: Arc<AtomicU64>,
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
        self.validate_git_repo()?;

        // UUIDv7's leading characters encode time, so the first eight characters
        // collide for agents created near one another. Keep the full ID in both
        // the branch and path to make concurrent leases distinct.
        let agent_key = agent_id.to_string();
        let branch = explicit_branch
            .map(str::to_string)
            .unwrap_or_else(|| format!("davinci/agent/{agent_key}"));

        // 1. Validate branch is not held by another active lease
        {
            let leases = self.leases.read().map_err(|_| {
                WorktreeError::GitError("Failed to acquire read lock on leases".into())
            })?;
            for existing in leases.values() {
                if existing.branch == branch {
                    return Err(WorktreeError::BranchInUse(branch));
                }
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
        };

        // 5. Record active lease
        {
            let mut leases = self.leases.write().map_err(|_| {
                WorktreeError::GitError("Failed to acquire write lock on leases".into())
            })?;
            leases.insert(wt_path.clone(), lease.clone());
        }

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

    /// Check whether a worktree has uncommitted or unmerged changes.
    pub fn is_dirty(&self, lease: &WorktreeLease) -> bool {
        if !lease.path.exists() {
            return false;
        }

        // 1. Check uncommitted changes (git status --porcelain)
        let status = run_git(&lease.path, &["status", "--porcelain"]);
        if let Ok(st) = status {
            if !st.trim().is_empty() {
                return true;
            }
        }

        // 2. Check if current HEAD differs from base_head
        let current_head = run_git(&lease.path, &["rev-parse", "HEAD"]).unwrap_or_default();
        if !current_head.is_empty() && current_head != lease.base_head {
            // New commits were made. Check if they were merged into base_head
            let is_ancestor = Command::new("git")
                .current_dir(&self.repo_root)
                .args([
                    "merge-base",
                    "--is-ancestor",
                    &current_head,
                    &lease.base_head,
                ])
                .output()
                .map(|o| o.status.success())
                .unwrap_or(false);

            if !is_ancestor {
                return true;
            }
        }

        false
    }

    /// Release an existing worktree lease.
    /// If dirty and `force` is false, preserves the worktree directory and returns `DirtyWorktreePreserved`.
    pub fn release_lease(&self, lease: &WorktreeLease, force: bool) -> Result<(), WorktreeError> {
        let dirty = self.is_dirty(lease);
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
            let _ = run_git(
                &self.repo_root,
                &["worktree", "remove", "--force", wt_path_str],
            );
            // In case git worktree remove left artifacts, cleanup directory
            if lease.path.exists() {
                let _ = std::fs::remove_dir_all(&lease.path);
            }
        }

        // Delete branch if it starts with default prefix and force is true or clean
        if lease.branch.starts_with("davinci/agent/") {
            let _ = run_git(&self.repo_root, &["branch", "-D", &lease.branch]);
        }

        // Remove from leases
        {
            let mut leases = self.leases.write().map_err(|_| {
                WorktreeError::GitError("Failed to acquire write lock on leases".into())
            })?;
            leases.remove(&lease.path);
        }

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
}
