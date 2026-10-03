use davinci_agent::runtime::{
    checkpoints::compute_sha256,
    evidence_store::ExecutionReceipt,
    operations::{AttemptId, ExecutionOwnerId, JournalId, OperationId, ProcessOperationBinding},
    transactions::{ProposedChange, TransactionCoordinator, TransactionOwner, TransactionState},
    EvidenceId,
};
use std::fs;

fn binding() -> ProcessOperationBinding {
    ProcessOperationBinding {
        journal_id: JournalId::new(),
        operation_id: OperationId::new(),
        attempt_id: AttemptId::new(),
        owner_id: ExecutionOwnerId::new(),
        owner_generation: 2,
    }
}

fn receipt(
    root: &std::path::Path,
    operation_id: String,
    attempt_id: Option<String>,
) -> ExecutionReceipt {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    ExecutionReceipt {
        receipt_id: EvidenceId::new(),
        operation_id,
        attempt_id,
        tool_name: "fixture-test".into(),
        argv: vec!["fixture-test".into()],
        cwd: root.to_string_lossy().into(),
        started: true,
        exit_code: Some(0),
        stdout_hash: Some(compute_sha256(b"stdout")),
        stderr_hash: Some(compute_sha256(b"stderr")),
        started_at_ms: now,
        finished_at_ms: now + 1,
        ..Default::default()
    }
}

#[test]
fn verification_evidence_is_bound_to_the_exact_operation_attempt() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a.txt"), b"before").unwrap();
    let link = binding();
    let coordinator = TransactionCoordinator::new(root.path(), TransactionOwner::default())
        .unwrap()
        .with_operation_link(link.clone());
    let preview = coordinator
        .preview(vec![ProposedChange::write("a.txt", b"after".to_vec())])
        .unwrap();
    coordinator.apply(&preview.id, &|_| Ok(()), None).unwrap();
    let observation = coordinator
        .begin_verification(&preview.id, &|_| Ok(()))
        .unwrap();

    let wrong = receipt(
        root.path(),
        OperationId::new().to_string(),
        Some(link.attempt_id.to_string()),
    );
    assert!(coordinator
        .finish_verification(observation, wrong, &|_| Ok(()))
        .unwrap_err()
        .contains("operation"));

    let observation = coordinator
        .begin_verification(&preview.id, &|_| Ok(()))
        .unwrap();
    let verified = coordinator
        .finish_verification(
            observation,
            receipt(
                root.path(),
                link.operation_id.to_string(),
                Some(link.attempt_id.to_string()),
            ),
            &|_| Ok(()),
        )
        .unwrap();
    let evidence = verified.verification.unwrap();
    assert_eq!(
        evidence.operation_id.as_deref(),
        Some(link.operation_id.to_string().as_str())
    );
    assert_eq!(
        evidence.attempt_id.as_deref(),
        Some(link.attempt_id.to_string().as_str())
    );
    assert!(evidence.output_complete);
    assert_eq!(verified.state, TransactionState::Verified);
}

#[test]
fn stale_source_keeps_the_write_successful_but_blocks_verification() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a.txt"), b"before").unwrap();
    let coordinator =
        TransactionCoordinator::new(root.path(), TransactionOwner::default()).unwrap();
    let preview = coordinator
        .preview(vec![ProposedChange::write("a.txt", b"after".to_vec())])
        .unwrap();
    coordinator.apply(&preview.id, &|_| Ok(()), None).unwrap();
    let observation = coordinator
        .begin_verification(&preview.id, &|_| Ok(()))
        .unwrap();
    fs::write(root.path().join("a.txt"), b"third-party").unwrap();
    assert!(coordinator
        .finish_verification(
            observation,
            receipt(root.path(), "verification".into(), None),
            &|_| Ok(())
        )
        .is_err());
    assert_eq!(
        coordinator.status(&preview.id).unwrap().state,
        TransactionState::Applied
    );
}

#[test]
fn failed_or_incomplete_verification_never_resets_the_original_operation() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a.txt"), b"before").unwrap();
    let coordinator =
        TransactionCoordinator::new(root.path(), TransactionOwner::default()).unwrap();
    let preview = coordinator
        .preview(vec![ProposedChange::write("a.txt", b"after".to_vec())])
        .unwrap();
    coordinator.apply(&preview.id, &|_| Ok(()), None).unwrap();
    let observation = coordinator
        .begin_verification(&preview.id, &|_| Ok(()))
        .unwrap();
    let mut failed = receipt(root.path(), "verification".into(), None);
    failed.exit_code = Some(1);
    assert!(coordinator
        .finish_verification(observation, failed, &|_| Ok(()))
        .is_err());
    assert_eq!(
        coordinator.status(&preview.id).unwrap().state,
        TransactionState::Applied
    );
    assert_eq!(fs::read(root.path().join("a.txt")).unwrap(), b"after");
}

#[test]
fn artifact_digests_are_recorded_without_persisting_output_contents() {
    let root = tempfile::tempdir().unwrap();
    let store = davinci_agent::runtime::evidence_store::VerificationEvidenceStore::new(root.path());
    let artifact = store
        .store_artifact("text/plain", b"fixture output")
        .unwrap();
    let mut value = receipt(root.path(), "verification".into(), None);
    value.stdout_artifact = Some(artifact.clone());
    let binding = davinci_agent::runtime::operations::bind_verification_evidence(
        &davinci_agent::runtime::transactions::TransactionCoordinator::new(
            root.path(),
            TransactionOwner::default(),
        )
        .unwrap()
        .preview(vec![ProposedChange::write("source.txt", b"after".to_vec())])
        .unwrap(),
        &value,
        "source-manifest",
    )
    .unwrap();
    assert_eq!(binding.artifact_digests, vec![artifact.sha256]);
    assert!(binding.output_complete);
    assert!(!serde_json::to_string(&binding)
        .unwrap()
        .contains("fixture output"));
}
