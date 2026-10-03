use davinci_agent::runtime::operations::*;
use davinci_agent::runtime::{AgentId, RunId, TaskId};
use serde_json::{json, Value};

fn context() -> OperationContext {
    OperationContext {
        journal_id: JournalId::new(),
        root_namespace_id: RootNamespaceId::new(),
        session_id: "session-123".to_owned(),
        runtime_run_id: RunId::new(),
        parent_operation_id: None,
        agent_id: AgentId::new(),
        worker_id: Some("worker-2".to_owned()),
        task_id: Some(TaskId::new()),
        graph: Some(GraphRunBinding {
            graph_run_id: "graph-run-7".to_owned(),
            graph_task_id: Some("task-9".to_owned()),
        }),
        workspace: WorkspaceIdentity {
            id: WorkspaceId::new(),
            binding_version: 3,
        },
        caller: CallerType::ProviderToolCall,
        wire_tool_call_id: Some("call_abc".to_owned()),
    }
}

fn spec() -> OperationSpec {
    OperationSpec::new(
        context(),
        ScopedIdempotencyKey::new(IdempotencyScope::ProviderCall, "assistant-4/call_abc").unwrap(),
        OperationKind::ToolInvocation,
        EffectProfile {
            classification: EffectClass::IdempotentMutation,
            supports_idempotency_key: true,
            supports_postcondition_probe: true,
            supports_compensation: false,
            requires_live_owner: true,
        },
        json!({"path": "src/main.rs", "content": "updated"}),
        vec![],
    )
    .unwrap()
}

fn owner() -> ExecutionOwner {
    ExecutionOwner::new(ExecutionOwnerId::new(), 1).unwrap()
}

fn result_ref() -> ResultRef {
    ResultRef::new(PayloadDigest::of_bytes(b"structured result"))
}

fn attempt() -> OperationAttempt {
    OperationAttempt::new(spec().operation_id(), 1, owner()).unwrap()
}

fn advance(attempt: &OperationAttempt, event: OperationEvent) -> OperationAttempt {
    transition_attempt(attempt, attempt.revision(), event).unwrap()
}

fn authorized_and_queued(attempt: &OperationAttempt) -> OperationAttempt {
    let attempt = advance(attempt, OperationEvent::Persist);
    let receipt = AuthorizationReceipt::new(
        PayloadDigest::of_bytes(b"approved payload"),
        "policy-r17".to_owned(),
        Timestamp::from_unix_millis(10),
    );
    let attempt = advance(&attempt, OperationEvent::Authorize(receipt));
    advance(&attempt, OperationEvent::Queue)
}

fn running(attempt: &OperationAttempt) -> OperationAttempt {
    let attempt = authorized_and_queued(attempt);
    advance(
        &attempt,
        OperationEvent::Start {
            at: Timestamp::from_unix_millis(20),
        },
    )
}

#[test]
fn operation_context_round_trips_all_required_and_optional_bindings() {
    let context = context();
    let encoded = serde_json::to_value(&context).unwrap();

    for key in [
        "journal_id",
        "root_namespace_id",
        "session_id",
        "runtime_run_id",
        "parent_operation_id",
        "agent_id",
        "worker_id",
        "task_id",
        "graph",
        "workspace",
        "caller",
        "wire_tool_call_id",
    ] {
        assert!(
            encoded.get(key).is_some(),
            "missing serialized context field {key}"
        );
    }
    assert_eq!(encoded["graph"]["graph_run_id"], "graph-run-7");
    assert_eq!(encoded["wire_tool_call_id"], "call_abc");
    assert_eq!(
        serde_json::from_value::<OperationContext>(encoded).unwrap(),
        context
    );
}

#[test]
fn logical_operation_identity_survives_distinct_physical_attempts() {
    let operation = spec();
    let first = OperationAttempt::new(operation.operation_id(), 1, owner()).unwrap();
    let second = OperationAttempt::new(operation.operation_id(), 2, owner()).unwrap();

    assert_eq!(first.operation_id(), operation.operation_id());
    assert_eq!(second.operation_id(), operation.operation_id());
    assert_ne!(first.attempt_id(), second.attempt_id());
    assert_eq!(first.attempt_number(), 1);
    assert_eq!(second.attempt_number(), 2);
}

#[test]
fn unsupported_spec_schema_is_rejected() {
    let mut encoded = serde_json::to_value(spec()).unwrap();
    encoded["schema_version"] = json!(u32::MAX);

    assert!(serde_json::from_value::<OperationSpec>(encoded).is_err());
}

#[test]
fn attempt_without_required_owner_is_rejected() {
    let mut encoded = serde_json::to_value(attempt()).unwrap();
    encoded.as_object_mut().unwrap().remove("owner");

    assert!(serde_json::from_value::<OperationAttempt>(encoded).is_err());
}

#[test]
fn checked_revision_accepts_a_valid_execution_lifecycle() {
    let attempt = running(&attempt());
    assert_eq!(attempt.state(), OperationState::Running);

    let attempt = advance(&attempt, OperationEvent::MarkEffectPossible);
    assert_eq!(attempt.state(), OperationState::EffectPossible);
    assert_eq!(attempt.effect_status(), EffectStatus::Possible);

    let attempt = advance(
        &attempt,
        OperationEvent::CompleteSuccess {
            result: result_ref(),
            effect_status: EffectStatus::EffectsObserved,
            finished_at: Timestamp::from_unix_millis(30),
        },
    );
    assert_eq!(attempt.state(), OperationState::Succeeded);
    assert_eq!(attempt.effect_status(), EffectStatus::EffectsObserved);
    assert!(attempt.result().is_some());
    assert!(attempt.revision() > 0);

    let stale = transition_attempt(&attempt, 0, OperationEvent::RequestCancellation);
    assert!(matches!(stale, Err(TransitionError::StaleRevision { .. })));
}

#[test]
fn terminal_execution_cannot_regress_to_a_nonterminal_state() {
    let attempt = running(&attempt());
    let attempt = advance(&attempt, OperationEvent::MarkEffectPossible);
    let succeeded = advance(
        &attempt,
        OperationEvent::CompleteSuccess {
            result: result_ref(),
            effect_status: EffectStatus::Unknown,
            finished_at: Timestamp::from_unix_millis(30),
        },
    );

    let original_result = succeeded.result().unwrap().clone();
    assert!(transition_attempt(&succeeded, succeeded.revision(), OperationEvent::Persist).is_err());
    assert!(transition_attempt(
        &succeeded,
        succeeded.revision(),
        OperationEvent::CompleteSuccess {
            result: result_ref(),
            effect_status: EffectStatus::EffectsObserved,
            finished_at: Timestamp::from_unix_millis(40),
        },
    )
    .is_err());
    assert_eq!(succeeded.result(), Some(&original_result));
    assert_eq!(succeeded.state(), OperationState::Succeeded);
    assert_ne!(succeeded.effect_status(), EffectStatus::NotStarted);
}

#[test]
fn cancellation_request_does_not_claim_an_effect_was_undone() {
    let attempt = running(&attempt());
    let attempt = advance(&attempt, OperationEvent::MarkEffectPossible);
    let cancellation = advance(&attempt, OperationEvent::RequestCancellation);

    assert!(cancellation.cancellation_requested());
    assert_eq!(cancellation.state(), OperationState::EffectPossible);
    assert_eq!(cancellation.effect_status(), EffectStatus::Possible);
}

#[test]
fn legacy_optional_metadata_round_trips_without_inventing_effect_certainty() {
    let mut encoded: Value = serde_json::to_value(attempt()).unwrap();
    let object = encoded.as_object_mut().unwrap();
    for optional_field in [
        "authorization",
        "started_at",
        "finished_at",
        "result",
        "failure",
        "verification",
        "publications",
        "cancellation_requested",
        "effect_status",
    ] {
        object.remove(optional_field);
    }

    let decoded: OperationAttempt = serde_json::from_value(encoded).unwrap();
    assert_eq!(decoded.effect_status(), EffectStatus::Unknown);
    assert_eq!(decoded.state(), OperationState::Created);
    assert!(decoded.authorization().is_none());
    assert!(decoded.result().is_none());

    let round_trip = serde_json::to_value(decoded).unwrap();
    assert_eq!(round_trip["effect_status"], json!("unknown"));
}

#[test]
fn successful_execution_cannot_transition_back_to_not_started_effect_status() {
    let attempt = running(&attempt());
    let attempt = advance(&attempt, OperationEvent::MarkEffectPossible);
    let succeeded = advance(
        &attempt,
        OperationEvent::CompleteSuccess {
            result: result_ref(),
            effect_status: EffectStatus::KnownNoEffect,
            finished_at: Timestamp::from_unix_millis(30),
        },
    );

    let later_events = [
        OperationEvent::Persist,
        OperationEvent::Queue,
        OperationEvent::RequestCancellation,
        OperationEvent::MarkVerification(VerificationState::Pending),
        OperationEvent::Publish {
            consumer: "session".to_owned(),
            state: PublicationState::Pending,
        },
        OperationEvent::Interrupt {
            at: Timestamp::from_unix_millis(40),
        },
    ];
    for event in later_events {
        if let Ok(updated) = transition_attempt(&succeeded, succeeded.revision(), event) {
            assert_eq!(updated.state(), OperationState::Succeeded);
            assert_ne!(updated.effect_status(), EffectStatus::NotStarted);
        }
    }
}
