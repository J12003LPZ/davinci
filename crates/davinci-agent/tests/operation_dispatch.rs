use davinci_agent::runtime::operations::{
    EffectClass, ExecutionOwner, ExecutionOwnerId, IdempotencyScope, JournalId, JournalIdentity,
    OperationAdmission, OperationContext, OperationJournal, OperationState, RootNamespaceId,
    ToolOperationDispatcher, ToolOperationPlanner, WorkspaceId, WorkspaceIdentity,
    JOURNAL_DATABASE_FILE_NAME,
};
use davinci_agent::runtime::{
    AgentId, CapabilitySource, ReplayPolicy, RunId, RuntimeCapabilityRegistry,
};
use serde_json::json;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tempfile::TempDir;

struct Fixture {
    _temp: TempDir,
    directory: std::path::PathBuf,
    journal: Arc<OperationJournal>,
    context: OperationContext,
    dispatcher: ToolOperationDispatcher,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let directory = temp.path().join("operation-journal");
        let root = RootNamespaceId::new();
        let identity = JournalIdentity::new(
            JournalId::new(),
            WorkspaceIdentity {
                id: WorkspaceId::new(),
                binding_version: 1,
            },
        )
        .unwrap();
        let context = OperationContext {
            journal_id: identity.journal_id,
            root_namespace_id: root,
            session_id: "dispatch-session".into(),
            runtime_run_id: RunId::new(),
            parent_operation_id: None,
            agent_id: AgentId::new(),
            worker_id: None,
            task_id: None,
            graph: None,
            workspace: identity.workspace.clone(),
            caller: davinci_agent::runtime::operations::CallerType::ProviderToolCall,
            wire_tool_call_id: None,
        };
        let journal = Arc::new(OperationJournal::open(&directory, identity, root).unwrap());
        let dispatcher = ToolOperationDispatcher::new(
            journal.clone(),
            ExecutionOwner::new(ExecutionOwnerId::new(), 1).unwrap(),
        );
        Self {
            _temp: temp,
            directory,
            journal,
            context,
            dispatcher,
        }
    }

    fn read_capability(&self) -> davinci_agent::runtime::RuntimeCapability {
        RuntimeCapabilityRegistry::with_builtins()
            .get("read")
            .unwrap()
    }
}

fn new_operation(
    admission: OperationAdmission,
) -> davinci_agent::runtime::operations::AdmittedOperation {
    match admission {
        OperationAdmission::New(operation) => operation,
        other => panic!("expected a new operation, got {other:?}"),
    }
}

#[test]
fn direct_and_batch_children_share_durable_admission_and_replay_stable_keys() {
    let fixture = Fixture::new();
    let read = fixture.read_capability();
    let direct = ToolOperationPlanner::provider_call(
        fixture.context.clone(),
        "provider-call-1",
        "read",
        &json!({"path": "src/lib.rs"}),
        Some(&read),
        None,
    )
    .unwrap();
    let direct_admitted = new_operation(fixture.dispatcher.admit(direct, 7).unwrap());

    let first_child_plan = ToolOperationPlanner::batch_child(
        fixture.context.clone(),
        direct_admitted.spec.operation_id(),
        1,
        "read",
        &json!({"path": "Cargo.toml"}),
        Some(&read),
        None,
    )
    .unwrap();
    let first_child_key = first_child_plan.spec().caller_key().clone();
    assert_eq!(first_child_key.scope, IdempotencyScope::BatchChild);

    let first_child = new_operation(
        fixture
            .dispatcher
            .admit(first_child_plan.clone(), 7)
            .unwrap(),
    );
    let replay = fixture.dispatcher.admit(first_child_plan, 7).unwrap();
    let replayed = match replay {
        OperationAdmission::ExistingInFlight(operation) => operation,
        other => panic!("expected existing child operation, got {other:?}"),
    };

    assert_eq!(
        replayed.spec.operation_id(),
        first_child.spec.operation_id()
    );
    assert_eq!(replayed.spec.caller_key(), &first_child_key);
    assert_eq!(direct_admitted.attempt.state(), OperationState::Queued);
    assert_eq!(first_child.attempt.state(), OperationState::Queued);
    let snapshot = fixture.journal.snapshot().unwrap();
    assert_eq!(snapshot.operations.len(), 2);
    assert_eq!(snapshot.attempts.len(), 2);
}

#[test]
fn planning_a_tool_call_does_not_run_its_adapter_or_touch_the_workspace() {
    let fixture = Fixture::new();
    let capability = RuntimeCapabilityRegistry::with_builtins()
        .get("exec_command")
        .unwrap();
    let adapter_calls = AtomicUsize::new(0);
    let plan = ToolOperationPlanner::provider_call(
        fixture.context.clone(),
        "provider-command-1",
        "exec_command",
        &json!({"command": "touch should-not-exist"}),
        Some(&capability),
        None,
    )
    .unwrap();

    assert_eq!(adapter_calls.load(Ordering::SeqCst), 0);
    assert!(plan.spec().payload().is_object());
    assert!(!fixture.directory.join("should-not-exist").exists());
}

#[test]
fn unknown_and_custom_tools_use_conservative_effect_and_replay_defaults() {
    let fixture = Fixture::new();
    let unknown = ToolOperationPlanner::provider_call(
        fixture.context.clone(),
        "provider-custom-1",
        "custom_external_tool",
        &json!({"target": "remote"}),
        None,
        None,
    )
    .unwrap();
    assert_eq!(
        unknown.spec().effects().classification,
        EffectClass::ExternalMutation
    );
    assert_eq!(unknown.replay_policy(), ReplayPolicy::NeverAutoReplay);

    let custom = davinci_agent::runtime::RuntimeCapability::new(
        "custom_external_tool",
        CapabilitySource::Mcp,
        davinci_agent::ToolClass::Read,
        true,
        &json!({"type": "object"}),
        None,
    );
    let custom_plan = ToolOperationPlanner::provider_call(
        fixture.context.clone(),
        "provider-custom-2",
        "custom_external_tool",
        &json!({"target": "remote"}),
        Some(&custom),
        None,
    )
    .unwrap();
    assert_eq!(
        custom_plan.spec().effects().classification,
        EffectClass::ExternalMutation
    );
    assert_eq!(custom_plan.replay_policy(), ReplayPolicy::NeverAutoReplay);
}

#[test]
fn intent_persistence_failure_cannot_invoke_the_adapter() {
    let fixture = Fixture::new();
    let connection =
        rusqlite::Connection::open(fixture.directory.join(JOURNAL_DATABASE_FILE_NAME)).unwrap();
    connection
        .execute_batch(
            "CREATE TRIGGER reject_operation_intent
             BEFORE INSERT ON operations
             BEGIN SELECT RAISE(FAIL, 'injected intent persistence failure'); END;",
        )
        .unwrap();
    drop(connection);

    let read = fixture.read_capability();
    let plan = ToolOperationPlanner::provider_call(
        fixture.context.clone(),
        "provider-call-failed-write",
        "read",
        &json!({"path": "README.md"}),
        Some(&read),
        None,
    )
    .unwrap();
    let adapter_calls = AtomicUsize::new(0);
    let result = fixture.dispatcher.admit(plan.clone(), 7);
    if let Ok(OperationAdmission::New(operation)) = result.as_ref() {
        let _ = fixture.dispatcher.dispatch(
            &operation,
            &plan,
            Some(7),
            false,
            || Ok(()),
            || adapter_calls.fetch_add(1, Ordering::SeqCst),
        );
    }

    assert!(result.is_err());
    assert_eq!(adapter_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn durable_intent_precedes_authorization_and_can_be_cancelled_without_effect() {
    let fixture = Fixture::new();
    let read = fixture.read_capability();
    let plan = ToolOperationPlanner::provider_call(
        fixture.context.clone(),
        "provider-call-preauth-cancel",
        "read",
        &json!({"path": "README.md"}),
        Some(&read),
        None,
    )
    .unwrap();

    let admitted = new_operation(fixture.dispatcher.persist_intent(plan).unwrap());
    assert_eq!(admitted.attempt.state(), OperationState::Persisted);
    assert!(admitted.attempt.authorization().is_none());

    fixture
        .dispatcher
        .cancel_before_start(&admitted, "permission denied")
        .unwrap();

    let attempt = fixture
        .journal
        .load_attempt(admitted.attempt.attempt_id())
        .unwrap();
    assert_eq!(attempt.state(), OperationState::Cancelled);
    assert_eq!(
        attempt.effect_status(),
        davinci_agent::runtime::operations::EffectStatus::NotStarted
    );
}

#[test]
fn persisted_intent_becomes_dispatchable_only_after_authorization() {
    let fixture = Fixture::new();
    let read = fixture.read_capability();
    let plan = ToolOperationPlanner::provider_call(
        fixture.context.clone(),
        "provider-call-preauth-queue",
        "read",
        &json!({"path": "README.md"}),
        Some(&read),
        None,
    )
    .unwrap();

    let admitted = new_operation(fixture.dispatcher.persist_intent(plan).unwrap());
    let queued = fixture
        .dispatcher
        .authorize_and_queue(&admitted, 11)
        .unwrap();

    assert_eq!(queued.attempt.state(), OperationState::Queued);
    assert_eq!(
        queued
            .attempt
            .authorization()
            .map(|receipt| receipt.policy_revision.as_str()),
        Some("11")
    );
}

#[test]
fn a_post_admission_denial_records_a_not_executed_attempt() {
    let fixture = Fixture::new();
    let read = fixture.read_capability();
    let plan = ToolOperationPlanner::provider_call(
        fixture.context.clone(),
        "provider-call-policy-changed",
        "read",
        &json!({"path": "README.md"}),
        Some(&read),
        None,
    )
    .unwrap();
    let admitted = new_operation(fixture.dispatcher.admit(plan.clone(), 7).unwrap());
    let adapter_calls = AtomicUsize::new(0);
    let result = fixture.dispatcher.dispatch(
        &admitted,
        &plan,
        Some(8),
        false,
        || Ok(()),
        || adapter_calls.fetch_add(1, Ordering::SeqCst),
    );

    assert!(result.is_err());
    assert_eq!(adapter_calls.load(Ordering::SeqCst), 0);
    let attempt = fixture
        .journal
        .load_attempt(admitted.attempt.attempt_id())
        .unwrap();
    assert_eq!(attempt.state(), OperationState::Cancelled);
    assert_eq!(
        attempt.effect_status(),
        davinci_agent::runtime::operations::EffectStatus::NotStarted
    );
}
