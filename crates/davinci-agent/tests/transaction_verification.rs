use davinci_agent::runtime::{
    checkpoints::compute_sha256,
    evidence_store::ExecutionReceipt,
    transactions::{ProposedChange, TransactionCoordinator, TransactionOwner, TransactionState},
    EvidenceId,
};
use std::{fs, process::Command};

fn receipt(root: &std::path::Path) -> ExecutionReceipt {
    let started = now();
    let output = Command::new(std::env::current_exe().unwrap())
        .arg("--list")
        .current_dir(root)
        .output()
        .unwrap();
    ExecutionReceipt {
        receipt_id: EvidenceId::new(),
        operation_id: "verification-fixture".into(),
        tool_name: "fixture_process".into(),
        argv: vec!["--list".into()],
        cwd: root.to_string_lossy().into(),
        started: true,
        exit_code: output.status.code(),
        stdout_hash: Some(compute_sha256(&output.stdout)),
        stderr_hash: Some(compute_sha256(&output.stderr)),
        started_at_ms: started,
        finished_at_ms: now(),
        ..Default::default()
    }
}
fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64
}

#[test]
fn actual_receipt_verifies_only_unchanged_transaction_images() {
    let root = tempfile::tempdir().unwrap();
    let coordinator =
        TransactionCoordinator::new(root.path(), TransactionOwner::default()).unwrap();
    let preview = coordinator
        .preview(vec![ProposedChange::write("a.txt", b"after".to_vec())])
        .unwrap();
    coordinator.apply(&preview.id, &|_| Ok(()), None).unwrap();
    let observation = coordinator
        .begin_verification(&preview.id, &|_| Ok(()))
        .unwrap();
    let verified = coordinator
        .finish_verification(observation, receipt(root.path()), &|_| Ok(()))
        .unwrap();
    assert_eq!(verified.state, TransactionState::Verified);
    assert!(verified.verification.is_some());
    fs::write(root.path().join("a.txt"), b"other").unwrap();
    let stale = coordinator.status(&preview.id).unwrap();
    assert_eq!(stale.state, TransactionState::Applied);
    assert!(stale.verification_state.contains("stale"));
    assert_eq!(fs::read(root.path().join("a.txt")).unwrap(), b"other");
}

#[test]
fn verification_rejects_source_changes_simulation_and_revoked_authority() {
    for failure in ["changed", "simulated", "denied"] {
        let root = tempfile::tempdir().unwrap();
        let coordinator =
            TransactionCoordinator::new(root.path(), TransactionOwner::default()).unwrap();
        let preview = coordinator
            .preview(vec![ProposedChange::write("a.txt", b"after".to_vec())])
            .unwrap();
        coordinator.apply(&preview.id, &|_| Ok(()), None).unwrap();
        let observation = coordinator
            .begin_verification(&preview.id, &|_| Ok(()))
            .unwrap();
        let mut evidence = receipt(root.path());
        if failure == "changed" {
            fs::write(root.path().join("a.txt"), b"other").unwrap();
        }
        if failure == "simulated" {
            evidence.simulated = true;
        }
        let authority = |_: &std::path::Path| {
            if failure == "denied" {
                Err("revoked".into())
            } else {
                Ok(())
            }
        };
        assert!(coordinator
            .finish_verification(observation, evidence, &authority)
            .is_err());
        assert_eq!(
            coordinator.status(&preview.id).unwrap().state,
            TransactionState::Applied
        );
    }
}

#[test]
fn recheck_invalidates_previous_success_and_superseded_observations() {
    let root = tempfile::tempdir().unwrap();
    let coordinator =
        TransactionCoordinator::new(root.path(), TransactionOwner::default()).unwrap();
    let preview = coordinator
        .preview(vec![ProposedChange::write("a.txt", b"after".to_vec())])
        .unwrap();
    coordinator.apply(&preview.id, &|_| Ok(()), None).unwrap();
    let first = coordinator
        .begin_verification(&preview.id, &|_| Ok(()))
        .unwrap();
    coordinator
        .finish_verification(first, receipt(root.path()), &|_| Ok(()))
        .unwrap();
    let superseded = coordinator
        .begin_verification(&preview.id, &|_| Ok(()))
        .unwrap();
    let pending = coordinator.status(&preview.id).unwrap();
    assert_eq!(pending.state, TransactionState::Applied);
    assert_eq!(pending.verification_state, "verification pending");
    assert!(
        pending.verification.is_some(),
        "historical evidence remains available"
    );
    let current = coordinator
        .begin_verification(&preview.id, &|_| Ok(()))
        .unwrap();
    assert!(coordinator
        .finish_verification(superseded, receipt(root.path()), &|_| Ok(()))
        .is_err());
    let mut failed = receipt(root.path());
    failed.exit_code = Some(1);
    assert!(coordinator
        .finish_verification(current, failed, &|_| Ok(()))
        .is_err());
    assert_eq!(
        coordinator.status(&preview.id).unwrap().state,
        TransactionState::Applied
    );
}

#[test]
fn persisted_verification_rejects_a_different_workspace() {
    let root = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let coordinator =
        TransactionCoordinator::new(root.path(), TransactionOwner::default()).unwrap();
    let preview = coordinator
        .preview(vec![ProposedChange::write("a.txt", b"after".to_vec())])
        .unwrap();
    coordinator.apply(&preview.id, &|_| Ok(()), None).unwrap();
    let observation = coordinator
        .begin_verification(&preview.id, &|_| Ok(()))
        .unwrap();
    coordinator
        .finish_verification(observation, receipt(root.path()), &|_| Ok(()))
        .unwrap();
    let path = root
        .path()
        .join(".davinci-transactions")
        .join(format!("{}.json", preview.id));
    let mut record: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    record["summary"]["verification"]["receipt"]["cwd"] = serde_json::json!(other.path());
    fs::write(path, serde_json::to_vec(&record).unwrap()).unwrap();
    assert!(
        coordinator.status(&preview.id).is_err(),
        "foreign workspace receipt must not remain verified"
    );
}
