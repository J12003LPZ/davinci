use davinci_agent::runtime::operations::{
    classify_file_observation, AttemptId, ExecutionOwnerId, FilesystemEffectObservation, JournalId,
    OperationId, ProcessOperationBinding,
};
use davinci_agent::runtime::transactions::{
    ProposedChange, TransactionCoordinator, TransactionOwner, TransactionState,
};
use std::fs;

fn binding(generation: u64) -> ProcessOperationBinding {
    ProcessOperationBinding {
        journal_id: JournalId::new(),
        operation_id: OperationId::new(),
        attempt_id: AttemptId::new(),
        owner_id: ExecutionOwnerId::new(),
        owner_generation: generation,
    }
}

#[test]
fn transaction_record_contains_operation_attempt_and_path_receipts() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a.txt"), b"before").unwrap();
    let link = binding(7);
    let summary = TransactionCoordinator::new(root.path(), TransactionOwner::default())
        .unwrap()
        .with_operation_link(link.clone())
        .preview(vec![ProposedChange::write("a.txt", b"after".to_vec())])
        .unwrap();

    assert_eq!(
        summary.operation_id.as_deref(),
        Some(link.operation_id.to_string().as_str())
    );
    assert_eq!(
        summary.attempt_id.as_deref(),
        Some(link.attempt_id.to_string().as_str())
    );
    assert_eq!(summary.operation_owner_generation, Some(7));
    let receipt = summary.path_receipts.get("a.txt").unwrap();
    assert_eq!(receipt.before_hash, summary.before_hashes["a.txt"]);
    assert_eq!(receipt.proposed_hash, summary.proposed_hashes["a.txt"]);
    assert_eq!(receipt.applied_hash, None);
    assert_eq!(receipt.state, "previewed");
}

#[test]
fn apply_and_rollback_persist_path_receipts() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a.txt"), b"before").unwrap();
    let coordinator = TransactionCoordinator::new(root.path(), TransactionOwner::default())
        .unwrap()
        .with_operation_link(binding(3));
    let preview = coordinator
        .preview(vec![ProposedChange::write("a.txt", b"after".to_vec())])
        .unwrap();
    let applied = coordinator.apply(&preview.id, &|_| Ok(()), None).unwrap();
    assert_eq!(applied.state, TransactionState::Applied);
    assert_eq!(applied.path_receipts["a.txt"].state, "applied");
    assert_eq!(
        applied.path_receipts["a.txt"].applied_hash,
        applied.applied_hashes["a.txt"]
    );
    assert_eq!(fs::read(root.path().join("a.txt")).unwrap(), b"after");

    let rolled_back = coordinator
        .rollback(&preview.id, &|_| Ok(()), None)
        .unwrap();
    assert_eq!(rolled_back.state, TransactionState::RolledBack);
    assert_eq!(rolled_back.path_receipts["a.txt"].state, "rolled_back");
    assert_eq!(fs::read(root.path().join("a.txt")).unwrap(), b"before");
}

#[test]
fn intervening_user_change_is_retained_as_conflict() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a.txt"), b"before").unwrap();
    let coordinator =
        TransactionCoordinator::new(root.path(), TransactionOwner::default()).unwrap();
    let preview = coordinator
        .preview(vec![ProposedChange::write("a.txt", b"after".to_vec())])
        .unwrap();
    fs::write(root.path().join("a.txt"), b"user change").unwrap();

    let error = coordinator
        .apply(&preview.id, &|_| Ok(()), None)
        .unwrap_err();
    assert!(error.contains("changed since preview"));
    assert_eq!(fs::read(root.path().join("a.txt")).unwrap(), b"user change");
    let status = coordinator.status(&preview.id).unwrap();
    assert_eq!(status.state, TransactionState::Conflicted);
    assert_eq!(status.path_receipts["a.txt"].state, "previewed");
}

#[test]
fn filesystem_observation_does_not_invent_command_history() {
    assert_eq!(
        classify_file_observation(Some("before"), Some("after"), Some("before"), false),
        FilesystemEffectObservation::NotStarted
    );
    assert_eq!(
        classify_file_observation(Some("before"), Some("after"), Some("after"), false),
        FilesystemEffectObservation::AppliedPostimage
    );
    assert_eq!(
        classify_file_observation(Some("before"), Some("after"), Some("other"), true),
        FilesystemEffectObservation::Partial
    );
    assert_eq!(
        classify_file_observation(Some("before"), Some("after"), Some("other"), false),
        FilesystemEffectObservation::ThirdParty
    );
}
