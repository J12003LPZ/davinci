//! Worker workspace details come from real git worktrees, never from strings.

use davinci_agent::runtime::worker_workspace::{probe_changes, snapshot_workspace, WorkspaceKind};
use davinci_agent::runtime::{AgentId, AgentKind, AgentRecord, AgentState, RunId};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

fn git(dir: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.name=t",
            "-c",
            "user.email=t@example.com",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .expect("git runs");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout).trim().to_string()
}

fn repo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    std::fs::write(dir.path().join("a.txt"), "one\n").unwrap();
    git(dir.path(), &["add", "."]);
    git(dir.path(), &["commit", "-q", "-m", "first"]);
    dir
}

fn record(worktree: Option<PathBuf>) -> AgentRecord {
    AgentRecord {
        id: AgentId::new(),
        run_id: RunId::new(),
        parent: None,
        kind: AgentKind::Subagent,
        name: "w".into(),
        provider: "mock".into(),
        model_id: "mock".into(),
        cwd: PathBuf::from("."),
        state: AgentState::Running,
        task_id: None,
        worktree,
        started_ms: 1,
        updated_ms: 1,
        failure_reason: None,
    }
}

fn add_worktree(repo: &Path, name: &str, branch: &str) -> PathBuf {
    let path = repo.join("..").join(format!(
        "{}-{name}",
        repo.file_name().unwrap().to_string_lossy()
    ));
    git(
        repo,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            branch,
            path.to_str().unwrap(),
        ],
    );
    path
}

#[test]
fn isolated_worker_displays_verified_branch() {
    let repo = repo();
    let path = add_worktree(repo.path(), "iso", "davinci/agent/iso");
    let workspace = snapshot_workspace(&record(Some(path.clone())));
    assert_eq!(workspace.kind, WorkspaceKind::Worktree);
    assert_eq!(workspace.branch.as_deref(), Some("davinci/agent/iso"));
    let expected = git(
        repo.path(),
        &["rev-parse", "--short=7", "davinci/agent/iso"],
    );
    assert_eq!(workspace.head.as_deref(), Some(expected.as_str()));
    assert!(workspace.label().starts_with("davinci/agent/iso @ "));
    let _ = std::fs::remove_dir_all(path);
}

#[test]
fn shared_worker_has_no_fabricated_branch() {
    let workspace = snapshot_workspace(&record(None));
    assert_eq!(workspace.kind, WorkspaceKind::Shared);
    assert_eq!(workspace.branch, None);
    assert_eq!(workspace.label(), "shared workspace — no separate branch");
}

#[test]
fn worker_branch_updates_after_checkout() {
    let repo = repo();
    let path = add_worktree(repo.path(), "move", "davinci/agent/move");
    git(&path, &["checkout", "-q", "-b", "elsewhere"]);
    let workspace = snapshot_workspace(&record(Some(path.clone())));
    assert_eq!(workspace.branch.as_deref(), Some("elsewhere"));

    git(&path, &["checkout", "-q", "--detach"]);
    let detached = snapshot_workspace(&record(Some(path.clone())));
    assert!(detached.detached);
    assert_eq!(detached.branch, None);
    assert!(detached.label().starts_with("detached HEAD @ "));
    let _ = std::fs::remove_dir_all(path);
}

#[test]
fn deleted_worktree_displays_unavailable() {
    let repo = repo();
    let path = add_worktree(repo.path(), "gone", "davinci/agent/gone");
    std::fs::remove_dir_all(&path).unwrap();
    let workspace = snapshot_workspace(&record(Some(path)));
    assert_eq!(workspace.kind, WorkspaceKind::Unavailable);
    assert_eq!(workspace.branch, None);
    assert_eq!(workspace.label(), "worktree unavailable");
}

#[test]
fn a_directory_without_its_own_git_never_borrows_a_parents_branch() {
    // The worktree directory sits inside another repository and was
    // recreated empty: walking up would claim the parent's `main`.
    let repo = repo();
    let inside = repo.path().join("work").join("w");
    std::fs::create_dir_all(&inside).unwrap();
    let workspace = snapshot_workspace(&record(Some(inside)));
    assert_eq!(workspace.kind, WorkspaceKind::Unavailable);
    assert_eq!(workspace.branch, None);
}

#[test]
fn the_probe_reports_dirty_state_and_paths_with_a_timeout() {
    let repo = repo();
    let path = add_worktree(repo.path(), "dirty", "davinci/agent/dirty");
    let (clean, paths) = probe_changes(&path, Duration::from_secs(10));
    assert_eq!(clean, Some(false));
    assert!(paths.is_empty());

    std::fs::write(path.join("a.txt"), "changed\n").unwrap();
    std::fs::write(path.join("new file.txt"), "x\n").unwrap();
    let (dirty, paths) = probe_changes(&path, Duration::from_secs(10));
    assert_eq!(dirty, Some(true));
    assert!(paths.contains(&"a.txt".to_string()), "{paths:?}");
    assert!(paths.contains(&"new file.txt".to_string()), "{paths:?}");

    // Not a repository: no answer, not a claim of clean.
    let nowhere = tempfile::tempdir().unwrap();
    assert_eq!(
        probe_changes(nowhere.path(), Duration::from_secs(10)).0,
        None
    );
    let _ = std::fs::remove_dir_all(path);
}
