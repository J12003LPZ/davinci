use davinci_agent::runtime::transactions::{
    ProposedChange, TransactionCoordinator, TransactionOwner, TransactionState,
};
use std::{fs, path::Path, process::Command};

fn git(root: &Path, args: &[&str]) -> String {
    let output = Command::new("git")
        .current_dir(root)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env(
            "GIT_CONFIG_GLOBAL",
            if cfg!(windows) { "NUL" } else { "/dev/null" },
        )
        .args([
            "-c",
            "user.name=Fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout).unwrap().trim().into()
}

#[test]
fn transaction_commit_requires_exact_git_objects_and_current_authority() {
    for outcome in ["committed", "uncommitted", "changed", "denied"] {
        let root = tempfile::tempdir().unwrap();
        git(root.path(), &["init", "--quiet"]);
        fs::write(root.path().join("a.txt"), b"before").unwrap();
        git(root.path(), &["add", "--", "a.txt"]);
        git(root.path(), &["commit", "--quiet", "-m", "baseline"]);
        let coordinator =
            TransactionCoordinator::new(root.path(), TransactionOwner::default()).unwrap();
        let preview = coordinator
            .preview(vec![ProposedChange::write("a.txt", b"after".to_vec())])
            .unwrap();
        coordinator.apply(&preview.id, &|_| Ok(()), None).unwrap();
        if outcome != "uncommitted" {
            git(root.path(), &["add", "--", "a.txt"]);
            git(root.path(), &["commit", "--quiet", "-m", "transaction"]);
        }
        if outcome == "changed" {
            fs::write(root.path().join("a.txt"), b"other").unwrap();
        }
        let result = coordinator.observe_commit(&preview.id, &|_| {
            if outcome == "denied" {
                Err("revoked".into())
            } else {
                Ok(())
            }
        });
        if outcome == "committed" {
            let summary = result.unwrap();
            assert_eq!(summary.state, TransactionState::Committed);
            assert_eq!(
                summary.commit_revision.as_deref(),
                Some(git(root.path(), &["rev-parse", "HEAD"]).as_str())
            );
        } else {
            assert!(result.is_err(), "{outcome}");
            assert_eq!(
                coordinator.status(&preview.id).unwrap().state,
                TransactionState::Applied
            );
        }
        assert_eq!(
            fs::read(root.path().join("a.txt")).unwrap(),
            if outcome == "changed" {
                b"other"
            } else {
                b"after"
            }
        );
    }
}

#[test]
fn commit_observation_handles_create_delete_and_preserves_unrelated_files() {
    let root = tempfile::tempdir().unwrap();
    git(root.path(), &["init", "--quiet"]);
    fs::write(root.path().join("old name.txt"), "old").unwrap();
    git(root.path(), &["add", "--", "old name.txt"]);
    git(root.path(), &["commit", "--quiet", "-m", "baseline"]);
    let base = git(root.path(), &["rev-parse", "HEAD"]);
    let coordinator = TransactionCoordinator::new(root.path(), TransactionOwner::default())
        .unwrap()
        .with_base_revision(Some(base.clone()));
    let preview = coordinator
        .preview(vec![
            ProposedChange::delete("old name.txt"),
            ProposedChange::write("new name.txt", b"new".to_vec()),
        ])
        .unwrap();
    coordinator.apply(&preview.id, &|_| Ok(()), None).unwrap();
    git(root.path(), &["add", "--", "old name.txt", "new name.txt"]);
    git(root.path(), &["commit", "--quiet", "-m", "transaction"]);
    fs::write(root.path().join("unrelated.txt"), b"user content").unwrap();
    let committed = coordinator
        .observe_commit(&preview.id, &|_| Ok(()))
        .unwrap();
    assert_eq!(committed.state, TransactionState::Committed);
    assert_eq!(committed.base_revision.as_deref(), Some(base.as_str()));
    assert_eq!(
        coordinator.status(&preview.id).unwrap().commit_revision,
        committed.commit_revision
    );
    assert!(!root.path().join("old name.txt").exists());
    assert_eq!(
        fs::read(root.path().join("unrelated.txt")).unwrap(),
        b"user content"
    );
    assert!(coordinator
        .rollback(&preview.id, &|_| Ok(()), None)
        .is_err());
}

#[test]
fn commit_observation_never_fetches_a_missing_promisor_blob() {
    let root = tempfile::tempdir().unwrap();
    let remote = tempfile::tempdir().unwrap();
    git(root.path(), &["init", "--quiet"]);
    fs::write(root.path().join("a.txt"), b"before").unwrap();
    git(root.path(), &["add", "--", "a.txt"]);
    git(root.path(), &["commit", "--quiet", "-m", "baseline"]);
    let coordinator =
        TransactionCoordinator::new(root.path(), TransactionOwner::default()).unwrap();
    let preview = coordinator
        .preview(vec![ProposedChange::write("a.txt", b"after".to_vec())])
        .unwrap();
    coordinator.apply(&preview.id, &|_| Ok(()), None).unwrap();
    git(root.path(), &["add", "--", "a.txt"]);
    git(root.path(), &["commit", "--quiet", "-m", "transaction"]);
    // An independent local remote has the object available. Observation must
    // still fail locally, without letting Git repair the missing object.
    git(
        root.path(),
        &[
            "clone",
            "--quiet",
            "--no-local",
            "--bare",
            ".",
            remote.path().to_str().unwrap(),
        ],
    );
    git(
        root.path(),
        &["remote", "add", "origin", remote.path().to_str().unwrap()],
    );
    git(root.path(), &["config", "remote.origin.promisor", "true"]);
    let blob = git(root.path(), &["rev-parse", "HEAD:a.txt"]);
    assert_eq!(blob.len(), 40);
    assert!(blob.bytes().all(|byte| byte.is_ascii_hexdigit()));
    let object = root
        .path()
        .join(".git/objects")
        .join(&blob[..2])
        .join(&blob[2..]);
    #[cfg(windows)]
    {
        let mut permissions = fs::metadata(&object).unwrap().permissions();
        // Windows-only: clear the disposable Git object's DOS read-only bit.
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        fs::set_permissions(&object, permissions).unwrap();
    }
    fs::remove_file(&object).unwrap();
    assert!(coordinator
        .observe_commit(&preview.id, &|_| Ok(()))
        .is_err());
    assert!(!object.exists());
    assert!(!root.path().join(".git/FETCH_HEAD").exists());
    assert_eq!(
        coordinator.status(&preview.id).unwrap().state,
        TransactionState::Applied
    );
    assert_eq!(fs::read(root.path().join("a.txt")).unwrap(), b"after");
}
