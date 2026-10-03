use super::control::{
    reduce_control, ControlReceiptState, ControlTracker, GraphControl, GraphControlAction,
};
use super::recovery::{
    retry_recovery_gate, RetryDecision, RetryRecoveryInput, RetryRecoveryStatus,
};
use super::types::{
    ArtifactKind, GraphBudgets, GraphCounters, GraphLifecycle, GraphRun, GraphTaskState, Phase,
    Role, TaskStatus,
};
use davinci_agent::runtime::operations::{EffectStatus, OperationState};

fn recovery_input(
    operation_recovered: bool,
    operation_state: Option<OperationState>,
    effect_status: Option<EffectStatus>,
) -> RetryRecoveryInput {
    RetryRecoveryInput {
        recommendation: RetryDecision::RetryExtendedTimeout,
        attempt: 1,
        budget_available: true,
        prior_owner_quiescent: true,
        original_binding_valid: true,
        current_authority_valid: true,
        source_reconciled: true,
        operation_recovered,
        operation_completed: false,
        operation_state,
        effect_status,
    }
}

fn sample_run() -> GraphRun {
    GraphRun {
        version: 1,
        run_id: "run-operation-retry".into(),
        goal: "operation retry test".into(),
        cwd: ".".into(),
        phase: Phase::Investigate,
        forced: None,
        dry_run: false,
        execution_origin: None,
        definition_digest: None,
        saved_definition: None,
        definition: None,
        classification: None,
        milestones: None,
        current_milestone: None,
        tasks: Vec::new(),
        verification: None,
        verification_bundle: None,
        review_coverage: None,
        budgets: GraphBudgets::default(),
        counters: GraphCounters {
            workers_spawned: 0,
            revision_cycles: 0,
            replans: 0,
            cost_usd: 0.0,
            started_at: 0,
        },
        blocked_reason: None,
        resource_snapshot: None,
        ecosystem_stats: Default::default(),
        updated_at: 0,
        lifecycle: Some(GraphLifecycle::Running),
        revision: 0,
        control_history: Vec::new(),
        continuation: None,
    }
}

#[test]
fn timeout_after_committed_file_mutation_is_blocked() {
    let record = retry_recovery_gate(recovery_input(
        false,
        Some(OperationState::EffectPossible),
        Some(EffectStatus::EffectsObserved),
    ));

    assert!(!record.allowed);
    assert_eq!(record.status, RetryRecoveryStatus::OperationUnresolved);
    assert_eq!(record.operation_state, Some(OperationState::EffectPossible));
    assert_eq!(record.effect_status, Some(EffectStatus::EffectsObserved));
}

#[test]
fn missing_artifact_with_unknown_effects_is_blocked() {
    let record = retry_recovery_gate(recovery_input(
        false,
        Some(OperationState::Failed),
        Some(EffectStatus::Unknown),
    ));

    assert!(!record.allowed);
    assert_eq!(record.status, RetryRecoveryStatus::OperationUnresolved);
}

#[test]
fn reconciled_operation_allows_one_replacement_attempt() {
    let record = retry_recovery_gate(recovery_input(
        true,
        Some(OperationState::Failed),
        Some(EffectStatus::KnownNoEffect),
    ));

    assert!(record.allowed);
    assert_eq!(record.status, RetryRecoveryStatus::Allowed);
}

#[test]
fn successful_sibling_remains_eligible_when_other_retry_is_blocked() {
    let blocked = retry_recovery_gate(recovery_input(
        false,
        Some(OperationState::EffectPossible),
        Some(EffectStatus::Unknown),
    ));
    assert!(!blocked.allowed);

    let mut run = sample_run();
    let mut succeeded = GraphTaskState::new(
        "sibling-succeeded",
        Role::Researcher,
        ArtifactKind::Evidence,
        vec![],
        None,
    );
    succeeded.status = TaskStatus::Succeeded;
    succeeded.usage.input = 500;

    let mut failed = GraphTaskState::new(
        "retry-target",
        Role::Researcher,
        ArtifactKind::Evidence,
        vec![],
        None,
    );
    failed.status = TaskStatus::Failed;
    failed.attempts = 1;
    failed.error = Some("timeout after mutation".into());
    run.tasks = vec![succeeded, failed];

    let control = GraphControl {
        operation_id: "retry-target-control".into(),
        run_id: run.run_id.clone(),
        expected_run_revision: 0,
        node_id: Some("retry-target".into()),
        expected_attempt: Some(1),
        action: GraphControlAction::RetryNode,
    };
    let receipt = reduce_control(&mut run, &control, &mut ControlTracker::new(), 0, true);

    assert_eq!(receipt.state, ControlReceiptState::Applied);
    assert_eq!(
        run.tasks
            .iter()
            .find(|task| task.id == "sibling-succeeded")
            .map(|task| task.status),
        Some(TaskStatus::Succeeded)
    );
}

#[test]
fn retry_recovery_record_round_trips_after_restart() {
    let record = retry_recovery_gate(recovery_input(
        false,
        Some(OperationState::Failed),
        Some(EffectStatus::Unknown),
    ));
    let reopened: super::recovery::RetryRecoveryRecord =
        serde_json::from_slice(&serde_json::to_vec(&record).unwrap()).unwrap();

    assert_eq!(reopened, record);
}
