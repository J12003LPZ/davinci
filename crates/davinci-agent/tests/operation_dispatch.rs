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
        let temp_root = std::fs::canonicalize(temp.path()).unwrap();
        let directory = temp_root.join("operation-journal");
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
            operation,
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

fn edit_plan(
    fixture: &Fixture,
    call_id: &str,
    path: &str,
) -> davinci_agent::runtime::operations::PlannedToolOperation {
    let edit = RuntimeCapabilityRegistry::with_builtins()
        .get("edit")
        .unwrap();
    ToolOperationPlanner::provider_call(
        fixture.context.clone(),
        call_id,
        "edit",
        &json!({"path": path, "edits": [{"oldText": "a", "newText": "b"}]}),
        Some(&edit),
        None,
    )
    .unwrap()
}

/// A mutating tool that returns an error (a rejected edit, a missing
/// `oldText`) has finished running. Its effect stays `Unknown`, so it is
/// never replayed, but it must not keep the workspace claim: before this
/// fix one rejected edit blocked every later edit in the workspace, from
/// every session, until the journal was deleted.
#[test]
fn a_returned_tool_error_releases_its_claim_for_the_next_mutation() {
    let fixture = Fixture::new();
    let plan = edit_plan(&fixture, "provider-edit-rejected", "math.js");
    let admitted = new_operation(fixture.dispatcher.admit(plan.clone(), 7).unwrap());
    let result = fixture
        .dispatcher
        .dispatch(
            &admitted,
            &plan,
            Some(7),
            false,
            || Ok(()),
            || davinci_agent::ToolResult {
                content: "edits[1].oldText must not be empty in math.js.".into(),
                is_error: true,
                details: None,
            },
        )
        .unwrap();
    fixture
        .dispatcher
        .complete_for_session(&admitted, &result, false)
        .unwrap();
    let attempt = fixture
        .journal
        .load_attempt(admitted.attempt.attempt_id())
        .unwrap();
    assert_eq!(attempt.state(), OperationState::Failed);
    assert_eq!(
        attempt.effect_status(),
        davinci_agent::runtime::operations::EffectStatus::Unknown
    );

    let next = edit_plan(&fixture, "provider-edit-retry", "math.js");
    let admitted = fixture.dispatcher.admit(next, 7);
    assert!(
        matches!(admitted, Ok(OperationAdmission::New(_))),
        "the next edit must be admitted, got {admitted:?}"
    );
}

/// The scheduler runs up to eight read-class calls on parallel threads. Each
/// writes the journal briefly (admit, claim, latch, complete); contention
/// must wait for the writer rather than fail the tool call.
#[test]
fn parallel_read_dispatch_waits_for_the_journal_writer() {
    let fixture = Fixture::new();
    let read = fixture.read_capability();
    let threads = 8;
    let rounds = 12;
    let barrier = std::sync::Barrier::new(threads);
    let failures = std::sync::Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for thread in 0..threads {
            let (fixture, read, barrier, failures) = (&fixture, &read, &barrier, &failures);
            scope.spawn(move || {
                for round in 0..rounds {
                    barrier.wait();
                    let outcome = (|| -> Result<(), String> {
                        let plan = ToolOperationPlanner::provider_call(
                            fixture.context.clone(),
                            &format!("provider-read-{thread}-{round}"),
                            "read",
                            &json!({"path": format!("src/{thread}.rs")}),
                            Some(read),
                            None,
                        )
                        .map_err(|error| format!("{error:?}"))?;
                        let admitted = new_operation(
                            fixture
                                .dispatcher
                                .admit(plan.clone(), 7)
                                .map_err(|error| error.to_string())?,
                        );
                        let result = fixture
                            .dispatcher
                            .dispatch(
                                &admitted,
                                &plan,
                                Some(7),
                                false,
                                || Ok(()),
                                || davinci_agent::ToolResult {
                                    content: "ok".into(),
                                    is_error: false,
                                    details: None,
                                },
                            )
                            .map_err(|error| error.to_string())?;
                        fixture
                            .dispatcher
                            .complete_for_session(&admitted, &result, false)
                            .map_err(|error| error.to_string())
                    })();
                    if let Err(error) = outcome {
                        failures.lock().unwrap().push(error);
                    }
                }
            });
        }
    });
    let failures = failures.into_inner().unwrap();
    assert!(
        failures.is_empty(),
        "{} failed: {failures:?}",
        failures.len()
    );
}
