//! R4: worktree lease ownership. Production passes no explicit branch (branch
//! and path embed the agent id), so same-branch races are not reachable from
//! product paths; these tests pin the guarantees the manager itself makes.

use davinci_agent::runtime::{AgentId, RunId, WorktreeError, WorktreeManager};
use std::path::Path;
use std::process::Command;
use std::sync::{Arc, Barrier};
use tempfile::TempDir;

fn git(path: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(path)
        .args(args)
        .output()
        .unwrap();
    assert!(output.status.success(), "git {args:?}: {output:?}");
}

fn temp_repo() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init"]);
    git(dir.path(), &["config", "user.name", "Davinci Test"]);
    git(dir.path(), &["config", "user.email", "test@davinci.local"]);
    std::fs::write(dir.path().join("README.md"), "# repo\n").unwrap();
    git(dir.path(), &["add", "README.md"]);
    git(dir.path(), &["commit", "-m", "Initial commit"]);
    dir
}

#[test]
fn same_branch_concurrent_acquisition_has_one_winner() {
    const CONTENDERS: usize = 4;
    let repo = temp_repo();
    let roots = tempfile::tempdir().unwrap();
    let manager = WorktreeManager::new(repo.path(), roots.path());
    let run_id = RunId::new();
    let start = Arc::new(Barrier::new(CONTENDERS));
    let handles: Vec<_> = (0..CONTENDERS)
        .map(|_| {
            let (manager, start) = (manager.clone(), start.clone());
            std::thread::spawn(move || {
                start.wait();
                manager.create_lease(run_id, AgentId::new(), Some("shared/contested"))
            })
        })
        .collect();
    let outcomes: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    // Losers may see `BranchInUse` or a git error depending on timing; only
    // the number of winners is guaranteed.
    assert_eq!(
        outcomes.iter().filter(|outcome| outcome.is_ok()).count(),
        1,
        "{outcomes:?}"
    );
    let leases = manager.list_leases();
    assert_eq!(leases.len(), 1);
    assert!(leases[0].path.exists());
}

#[test]
fn stale_owner_cannot_release_new_lease() {
    let repo = temp_repo();
    let roots = tempfile::tempdir().unwrap();
    let manager = WorktreeManager::new(repo.path(), roots.path());
    let (run_id, agent_id) = (RunId::new(), AgentId::new());

    let stale = manager.create_lease(run_id, agent_id, None).unwrap();
    manager.release_lease(&stale, false).unwrap();
    // Same run and agent: the deterministic path and branch are reused.
    let current = manager.create_lease(run_id, agent_id, None).unwrap();
    assert_eq!(stale.path, current.path);
    std::fs::write(current.path.join("work.txt"), "uncommitted work").unwrap();

    // A caller holding the first lease must not tear down the second, even
    // with force.
    let error = manager.release_lease(&stale, true).unwrap_err();
    assert!(matches!(error, WorktreeError::StaleLease(_)), "{error:?}");
    assert!(current.path.join("work.txt").exists());
    assert_eq!(manager.get_lease(&current.path), Some(current.clone()));

    // The real owner still releases normally.
    manager.release_lease(&current, true).unwrap();
    assert!(manager.get_lease(&current.path).is_none());
}
