use davinci_agent::runtime::transactions::{
    ProposedChange, TransactionCoordinator, TransactionOwner, TransactionState,
};
use std::fs;
use std::path::Path;
use std::process::Command;

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(root)
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "git {:?}: {}",
        args,
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

#[test]
fn commit_observation_is_read_only_and_records_exact_git_evidence() {
    let root = tempfile::tempdir().unwrap();
    git(root.path(), &["init"]);
    git(root.path(), &["config", "user.email", "test@example.invalid"]);
    git(root.path(), &["config", "user.name", "DaVinci Test"]);
    fs::write(root.path().join("a.txt"), b"before").unwrap();
    git(root.path(), &["add", "a.txt"]);
    git(root.path(), &["commit", "-m", "before"]);

    let coordinator = TransactionCoordinator::new(root.path(), TransactionOwner::default()).unwrap();
    let preview = coordinator
        .preview(vec![ProposedChange::write("a.txt", b"after".to_vec())])
        .unwrap();
    coordinator.apply(&preview.id, &|_| Ok(()), None).unwrap();
    git(root.path(), &["add", "a.txt"]);
    git(root.path(), &["commit", "-m", "after"]);
    let head_before = git(root.path(), &["rev-parse", "HEAD"]);
    let status_before = git(root.path(), &["status", "--porcelain"]);

    let committed = coordinator
        .observe_commit(&preview.id, &|_| Ok(()))
        .unwrap();
    assert_eq!(committed.state, TransactionState::Committed);
    assert_eq!(committed.commit_revision.as_deref(), Some(head_before.as_str()));
    assert_eq!(git(root.path(), &["rev-parse", "HEAD"]), head_before);
    assert_eq!(git(root.path(), &["status", "--porcelain"]), status_before);
}

#[test]
fn commit_observation_refuses_unmatched_tree_without_writing_git() {
    let root = tempfile::tempdir().unwrap();
    git(root.path(), &["init"]);
    git(root.path(), &["config", "user.email", "test@example.invalid"]);
    git(root.path(), &["config", "user.name", "DaVinci Test"]);
    fs::write(root.path().join("a.txt"), b"before").unwrap();
    git(root.path(), &["add", "a.txt"]);
    git(root.path(), &["commit", "-m", "before"]);

    let coordinator = TransactionCoordinator::new(root.path(), TransactionOwner::default()).unwrap();
    let preview = coordinator
        .preview(vec![ProposedChange::write("a.txt", b"after".to_vec())])
        .unwrap();
    coordinator.apply(&preview.id, &|_| Ok(()), None).unwrap();
    fs::write(root.path().join("a.txt"), b"unrelated").unwrap();
    let head_before = git(root.path(), &["rev-parse", "HEAD"]);
    let error = coordinator
        .observe_commit(&preview.id, &|_| Ok(()))
        .unwrap_err();
    assert!(error.contains("verification") || error.contains("changed"));
    assert_eq!(git(root.path(), &["rev-parse", "HEAD"]), head_before);
}

