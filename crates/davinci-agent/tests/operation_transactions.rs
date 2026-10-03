use davinci_agent::runtime::operations::{
    reconcile_transaction_phase, AttemptId, ExecutionOwnerId, JournalId, OperationId,
    ProcessOperationBinding, TransactionProjectionLedger, TransactionRecovery,
};
use davinci_agent::runtime::transactions::{
    ProposedChange, TransactionCoordinator, TransactionOwner, TransactionState,
};
use std::fs;

fn binding() -> ProcessOperationBinding {
    ProcessOperationBinding {
        journal_id: JournalId::new(),
        operation_id: OperationId::new(),
        attempt_id: AttemptId::new(),
        owner_id: ExecutionOwnerId::new(),
        owner_generation: 12,
    }
}

#[test]
fn durable_phase_receipts_reconcile_without_replaying_a_transaction() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a.txt"), b"before").unwrap();
    let coordinator = TransactionCoordinator::new(root.path(), TransactionOwner::default())
        .unwrap()
        .with_operation_link(binding());
    let preview = coordinator
        .preview(vec![ProposedChange::write("a.txt", b"after".to_vec())])
        .unwrap();
    assert_eq!(
        reconcile_transaction_phase(&preview),
        TransactionRecovery::NotStarted
    );
    assert!(preview.phase_receipts.contains_key("previewed"));

    let applied = coordinator.apply(&preview.id, &|_| Ok(()), None).unwrap();
    assert_eq!(
        reconcile_transaction_phase(&applied),
        TransactionRecovery::AlreadyApplied
    );
    assert!(applied.phase_receipts.contains_key("applying"));
    assert!(applied.phase_receipts.contains_key("applied"));
    assert_eq!(applied.state, TransactionState::Applied);
}

#[test]
fn rollback_phase_is_distinct_and_compensation_is_not_parent_success() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a.txt"), b"before").unwrap();
    let coordinator =
        TransactionCoordinator::new(root.path(), TransactionOwner::default()).unwrap();
    let preview = coordinator
        .preview(vec![ProposedChange::write("a.txt", b"after".to_vec())])
        .unwrap();
    coordinator.apply(&preview.id, &|_| Ok(()), None).unwrap();
    let rolled_back = coordinator
        .rollback(&preview.id, &|_| Ok(()), None)
        .unwrap();
    assert_eq!(
        reconcile_transaction_phase(&rolled_back),
        TransactionRecovery::RolledBack
    );
    assert!(rolled_back.phase_receipts.contains_key("rolling_back"));
    assert!(rolled_back.phase_receipts.contains_key("rolled_back"));
    assert_ne!(rolled_back.state, TransactionState::Applied);
}

#[test]
fn projection_order_requires_transaction_receipt_then_result_then_session() {
    let mut ledger = TransactionProjectionLedger::default();
    assert_eq!(
        ledger.record_session_projection(),
        Err("session projection requires a durable operation result")
    );
    assert_eq!(
        ledger.record_operation_result(),
        Err("operation result requires a durable transaction receipt")
    );
    ledger.record_transaction_receipt();
    ledger.record_operation_result().unwrap();
    ledger.record_session_projection().unwrap();
    assert!(ledger.session_projection);
}

#[test]
fn legacy_summary_without_new_receipts_remains_an_observation() {
    let root = tempfile::tempdir().unwrap();
    fs::write(root.path().join("a.txt"), b"before").unwrap();
    let summary = TransactionCoordinator::new(root.path(), TransactionOwner::default())
        .unwrap()
        .preview(vec![ProposedChange::write("a.txt", b"after".to_vec())])
        .unwrap();
    let mut value = serde_json::to_value(&summary).unwrap();
    let object = value.as_object_mut().unwrap();
    object.remove("operation_id");
    object.remove("attempt_id");
    object.remove("operation_owner_id");
    object.remove("operation_owner_generation");
    object.remove("operation_workspace_identity");
    object.remove("phase_receipts");
    object.remove("path_receipts");
    let legacy: davinci_agent::runtime::transactions::TransactionSummary =
        serde_json::from_value(value).unwrap();
    assert!(legacy.operation_id.is_none());
    assert!(legacy.phase_receipts.is_empty());
    assert!(legacy.path_receipts.is_empty());
}
