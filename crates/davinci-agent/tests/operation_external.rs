use davinci_agent::runtime::operations::{
    CallerType, ExecutionOwner, ExecutionOwnerId, ExternalEndpointContract,
    ExternalEndpointIdentity, ExternalOperationAdapter, ExternalOperationError, JournalId,
    JournalIdentity, OperationContext, OperationJournal, OperationState, RootNamespaceId,
    ToolOperationRuntime, WorkspaceId, WorkspaceIdentity,
};
use davinci_agent::runtime::{AgentId, RunId};
use davinci_agent::tools::ToolResult;
use serde_json::json;
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Arc,
};
use tempfile::TempDir;

struct Fixture {
    _temp: TempDir,
    adapter: ExternalOperationAdapter,
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
            session_id: "external-session".into(),
            runtime_run_id: RunId::new(),
            parent_operation_id: None,
            agent_id: AgentId::new(),
            worker_id: None,
            task_id: None,
            graph: None,
            workspace: identity.workspace.clone(),
            caller: CallerType::Custom,
            wire_tool_call_id: None,
        };
        let journal = Arc::new(OperationJournal::open(&directory, identity, root).unwrap());
        let runtime = ToolOperationRuntime::new(
            journal,
            context,
            ExecutionOwner::new(ExecutionOwnerId::new(), 1).unwrap(),
            &temp_root,
        )
        .unwrap();
        Self {
            _temp: temp,
            adapter: ExternalOperationAdapter::new(runtime),
        }
    }

    fn endpoint(&self) -> ExternalEndpointIdentity {
        ExternalEndpointIdentity::new(
            "fixture://127.0.0.1:43123/mcp?token=must-not-persist",
            Some("fixture-server".into()),
            "mcp",
            Some("mcp-v1".into()),
            Some("tool-v2".into()),
            Some("tenant-a".into()),
        )
        .unwrap()
    }
}

#[test]
fn fixture_success_and_result_replay_never_call_endpoint_twice() {
    let fixture = Fixture::new();
    let calls = AtomicUsize::new(0);
    let args = json!({"path": "remote/item"});
    let first = fixture
        .adapter
        .start(
            "fixture-call-1",
            "mcp/read",
            &args,
            fixture.endpoint(),
            ExternalEndpointContract::unverified(),
            false,
            7,
        )
        .unwrap();
    first
        .begin(false, || Ok(()), || calls.fetch_add(1, Ordering::SeqCst))
        .unwrap();
    first
        .complete(&ToolResult {
            content: "fixture response".into(),
            is_error: false,
            details: Some(json!({"receipt": "fixture-receipt-1"})),
        })
        .unwrap();
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    let replay = fixture
        .adapter
        .start(
            "fixture-call-1",
            "mcp/read",
            &args,
            fixture.endpoint(),
            ExternalEndpointContract::unverified(),
            false,
            7,
        )
        .unwrap();
    assert_eq!(
        replay.disposition(),
        davinci_agent::runtime::operations::ExternalOperationDisposition::ExistingResult
    );
    let result = replay.replay_result().unwrap();
    assert!(result
        .details
        .as_ref()
        .and_then(|details| details.get("replayed_from_operation_journal"))
        .is_some());
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn duplicate_delivery_and_intentional_new_call_are_distinct() {
    let fixture = Fixture::new();
    let args = json!({"mutation": "once"});
    let first = fixture
        .adapter
        .start(
            "duplicate-call",
            "mcp/write",
            &args,
            fixture.endpoint(),
            ExternalEndpointContract::unverified(),
            false,
            7,
        )
        .unwrap();
    let duplicate = fixture
        .adapter
        .start(
            "duplicate-call",
            "mcp/write",
            &args,
            fixture.endpoint(),
            ExternalEndpointContract::unverified(),
            false,
            7,
        )
        .unwrap();
    assert_eq!(
        duplicate.disposition(),
        davinci_agent::runtime::operations::ExternalOperationDisposition::ExistingInFlight
    );
    assert_eq!(first.operation_id(), duplicate.operation_id());

    let deliberate_new = fixture
        .adapter
        .start(
            "new-authorized-call",
            "mcp/write",
            &args,
            fixture.endpoint(),
            ExternalEndpointContract::unverified(),
            false,
            7,
        )
        .unwrap();
    assert_eq!(
        deliberate_new.disposition(),
        davinci_agent::runtime::operations::ExternalOperationDisposition::New
    );
    assert_ne!(first.operation_id(), deliberate_new.operation_id());
}

#[test]
fn unverified_remote_key_and_response_loss_fail_closed() {
    let fixture = Fixture::new();
    let mut unverified = ExternalEndpointContract::unverified();
    unverified.remote_idempotency_key = Some("remote-key".into());
    assert!(matches!(
        fixture.adapter.start(
            "unverified-key",
            "mcp/write",
            &json!({"x": 1}),
            fixture.endpoint(),
            unverified,
            false,
            7,
        ),
        Err(ExternalOperationError::RemoteKeyRequiresVerifiedContract)
    ));

    let operation = fixture
        .adapter
        .start(
            "response-loss",
            "mcp/write",
            &json!({"x": 1}),
            fixture.endpoint(),
            ExternalEndpointContract::unverified(),
            false,
            7,
        )
        .unwrap();
    operation.begin(false, || Ok(()), || ()).unwrap();
    operation.response_lost().unwrap();
    let attempt = fixture
        .adapter
        .runtime()
        .dispatcher()
        .journal()
        .load_attempt(operation.attempt_id())
        .unwrap();
    assert_eq!(attempt.state(), OperationState::RecoveryRequired);
    assert!(matches!(
        operation.replay_result(),
        Err(ExternalOperationError::NotReplayable)
    ));
}

#[test]
fn endpoint_identity_and_metadata_are_redacted() {
    let fixture = Fixture::new();
    let endpoint = fixture.endpoint();
    assert!(!serde_json::to_string(&endpoint)
        .unwrap()
        .contains("must-not-persist"));
    let contract = ExternalEndpointContract::verified_with_remote_key("remote-key")
        .unwrap()
        .with_receipt_refs(vec!["receipt-1".into()])
        .unwrap();
    let operation = fixture
        .adapter
        .start(
            "redaction-call",
            "http/post",
            &json!({"password": "do-not-store"}),
            endpoint,
            contract,
            true,
            7,
        )
        .unwrap();
    let metadata = serde_json::to_string(&operation.metadata()).unwrap();
    assert!(!metadata.contains("do-not-store"));
    assert!(!metadata.contains("must-not-persist"));
    operation.begin(false, || Ok(()), || ()).unwrap();
    operation
        .complete(&ToolResult {
            content: "authorization=do-not-store".into(),
            is_error: false,
            details: Some(json!({"token": "do-not-store", "receipt": "receipt-1"})),
        })
        .unwrap();
    let replay = fixture
        .adapter
        .start(
            "redaction-call",
            "http/post",
            &json!({"password": "do-not-store"}),
            fixture.endpoint(),
            ExternalEndpointContract::verified_with_remote_key("remote-key")
                .unwrap()
                .with_receipt_refs(vec!["receipt-1".into()])
                .unwrap(),
            true,
            7,
        )
        .unwrap()
        .replay_result()
        .unwrap();
    assert!(!serde_json::to_string(&replay)
        .unwrap()
        .contains("do-not-store"));
}

#[test]
fn endpoint_change_with_same_call_id_is_a_collision() {
    let fixture = Fixture::new();
    let first = fixture
        .adapter
        .start(
            "endpoint-change",
            "mcp/read",
            &json!({"x": 1}),
            fixture.endpoint(),
            ExternalEndpointContract::unverified(),
            false,
            7,
        )
        .unwrap();
    let changed = ExternalEndpointIdentity::new(
        "fixture://127.0.0.1:43124/mcp",
        Some("fixture-server".into()),
        "mcp",
        Some("mcp-v1".into()),
        Some("tool-v2".into()),
        Some("tenant-a".into()),
    )
    .unwrap();
    assert!(matches!(
        fixture.adapter.start(
            "endpoint-change",
            "mcp/read",
            &json!({"x": 1}),
            changed,
            ExternalEndpointContract::unverified(),
            false,
            7,
        ),
        Err(ExternalOperationError::Journal(
            davinci_agent::runtime::operations::JournalError::IdempotencyCollision
        ))
    ));
    assert_eq!(
        first.disposition(),
        davinci_agent::runtime::operations::ExternalOperationDisposition::New
    );
}
