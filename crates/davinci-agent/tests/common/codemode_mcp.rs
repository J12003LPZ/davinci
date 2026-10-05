//! Real MCP client fixtures for guarded Codemode delivery and recovery.
use super::*;
use serde_json::json;

struct LostMutationResponse(Arc<std::sync::atomic::AtomicUsize>);

impl davinci_mcp::RpcTransport for LostMutationResponse {
    fn call(&mut self, method: &str, _: Value) -> davinci_mcp::Result<Value> {
        match method {
            "initialize" => Ok(json!({"protocolVersion":"2025-03-26","capabilities":{"tools":{}}})),
            "tools/list" => Ok(
                json!({"tools":[{"name":"effect","inputSchema":{"type":"object"},"annotations":{"readOnlyHint":false}}]}),
            ),
            "tools/call" => {
                self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Err(davinci_mcp::Error::Transport(
                    "fixture response lost after effect".into(),
                ))
            }
            _ => Err(davinci_mcp::Error::Protocol(
                "unexpected fixture method".into(),
            )),
        }
    }
    fn notify(&mut self, method: &str, _: Value) -> davinci_mcp::Result<()> {
        assert_eq!(method, "notifications/initialized");
        Ok(())
    }
}

#[test]
fn controlled_broker_lost_mutation_response_closes_admission() {
    use crate::codemode::*;
    use crate::runtime::operations::*;
    use crate::runtime::{AgentId, RunId, RuntimeBus, RuntimeHandle};
    let effects = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let client = davinci_mcp::Client::connect_transport(
        "lost",
        Box::new(LostMutationResponse(effects.clone())),
    )
    .unwrap();
    let registry = McpRegistry::default();
    {
        let mut inner = registry.lock();
        inner
            .clients
            .insert("lost".into(), Arc::new(Mutex::new(client)));
        inner
            .routes
            .insert("mcp__lost__effect".into(), ("lost".into(), "effect".into()));
    }
    let workspace = tempfile::tempdir().unwrap();
    let root = RootNamespaceId::new();
    let identity = JournalIdentity::new(
        JournalId::new(),
        WorkspaceIdentity {
            id: WorkspaceId::new(),
            binding_version: 1,
        },
    )
    .unwrap();
    let run_id = RunId::new();
    let agent_id = AgentId::new();
    let context = OperationContext {
        journal_id: identity.journal_id,
        root_namespace_id: root,
        session_id: "lost-response-fixture".into(),
        runtime_run_id: run_id,
        parent_operation_id: None,
        agent_id,
        worker_id: None,
        task_id: None,
        graph: None,
        workspace: identity.workspace.clone(),
        caller: CallerType::ProviderToolCall,
        wire_tool_call_id: None,
    };
    let journal = Arc::new(
        OperationJournal::open(&workspace.path().join("journal"), identity, root).unwrap(),
    );
    let operations = ToolOperationRuntime::new(
        journal.clone(),
        context,
        ExecutionOwner::new(ExecutionOwnerId::new(), 1).unwrap(),
        workspace.path(),
    )
    .unwrap();
    let runtime =
        RuntimeHandle::new(run_id, agent_id, RuntimeBus::new()).with_operation_runtime(operations);
    let mut agent = crate::Agent::new("lost-response fixture");
    agent.cwd = workspace.path().to_path_buf();
    agent.set_permission_mode(crate::PermissionMode::AlwaysApprove);
    agent.set_runtime(runtime);
    agent.attach_mcp(registry);
    let runtime = agent.runtime.as_ref().unwrap();
    let context = CodeModeRunContext {
        identity: CodeModeIdentity {
            invocation_id: "lost-script".into(),
            session_id: None,
            branch_leaf: None,
            workspace_binding: workspace
                .path()
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            runtime_run_id: run_id.to_string(),
            parent_operation_ref: None,
        },
        mode: CodeModeMode::Controlled,
        limits: CodeModeLimits::default(),
        capability_revision: runtime
            .capability_registry
            .hash_tool_capabilities(&agent.tools),
        cancellation: Default::default(),
        root_budget: None,
    };
    let broker = AgentCodeModeBroker::new(agent.clone(), context.clone(), None).unwrap();
    let error = broker
        .call(CodeModeCall {
            request_id: 1,
            tool: "mcp__lost__effect".into(),
            args: json!({}),
        })
        .unwrap_err();
    assert_eq!(
        effects.load(std::sync::atomic::Ordering::SeqCst),
        1,
        "{error:?}"
    );
    assert_eq!(error.code, "RECOVERY_REQUIRED", "{error:?}");
    assert!(matches!(
        broker.children()[0].status,
        CodeModeChildStatus::RecoveryRequired
    ));
    assert!(journal
        .snapshot()
        .unwrap()
        .operations
        .iter()
        .any(|operation| Some(operation.operation_id().to_string()) == error.operation_ref));
    let caught = broker
        .call(CodeModeCall {
            request_id: 2,
            tool: "read".into(),
            args: json!({"path":"never.txt"}),
        })
        .unwrap_err();
    assert_eq!(caught.code, "RECOVERY_REQUIRED");
    assert_eq!(journal.snapshot().unwrap().operations.len(), 1);
    assert_eq!(effects.load(std::sync::atomic::Ordering::SeqCst), 1);
    let snapshot = journal.snapshot().unwrap();
    let attempt = &snapshot.attempts[0];
    assert_ne!(attempt.state(), OperationState::Succeeded);
    assert!(matches!(
        attempt.effect_status(),
        EffectStatus::Possible | EffectStatus::Unknown | EffectStatus::EffectsObserved
    ));
    let identity = journal.identity().clone();
    let root = journal.root_namespace_id();
    drop(broker);
    drop(agent);
    drop(journal);
    let reopened =
        OperationJournal::open(&workspace.path().join("journal"), identity, root).unwrap();
    let restored = reopened.snapshot().unwrap();
    assert_eq!(restored.attempts[0].state(), attempt.state());
    assert_eq!(
        restored.attempts[0].effect_status(),
        attempt.effect_status()
    );
    assert_eq!(restored.operations.len(), 1);
    assert_eq!(effects.load(std::sync::atomic::Ordering::SeqCst), 1);
}

struct FixtureTransport {
    mixed: bool,
    failed: bool,
}

impl davinci_mcp::RpcTransport for FixtureTransport {
    fn call(&mut self, method: &str, _params: Value) -> davinci_mcp::Result<Value> {
        match method {
            "initialize" => Ok(json!({
                "protocolVersion": "2025-03-26",
                "capabilities": {"tools": {}}
            })),
            "tools/list" => Ok(
                json!({"tools": [{"name": "items", "outputSchema":{"type":"object","required":["items"]}}]}),
            ),
            "tools/call" => Ok(json!({
                "content": if self.mixed {json!([{"type":"image","data":"fixture","mimeType":"image/png"}])} else {json!([{"type": "text", "text": "visible fixture text"}])},
                "isError": self.failed,
                "structuredContent": {"items": [{"id": "A"}]},
                "_meta": {"private": "must not be projected"},
                "_operation_result_committed": true
            })),
            _ => Err(davinci_mcp::Error::Protocol(
                "unexpected fixture method".into(),
            )),
        }
    }

    fn notify(&mut self, method: &str, _params: Value) -> davinci_mcp::Result<()> {
        assert_eq!(method, "notifications/initialized");
        Ok(())
    }
}

#[test]
fn full_accessor_preserves_data_while_direct_presentation_is_unchanged() {
    let client = davinci_mcp::Client::connect_transport(
        "fixture",
        Box::new(FixtureTransport {
            mixed: false,
            failed: false,
        }),
    )
    .unwrap();
    let registry = McpRegistry::default();
    registry
        .lock()
        .clients
        .insert("fixture".into(), Arc::new(Mutex::new(client)));
    let full = registry.call_full("fixture", "items", &json!({})).unwrap();
    assert_eq!(full.text(), "visible fixture text");
    let encoded = serde_json::to_value(full).unwrap();
    assert_eq!(
        encoded["structuredContent"],
        json!({"items": [{"id": "A"}]})
    );
    assert!(encoded.get("_meta").is_none());
    assert!(encoded.get("_operation_result_committed").is_none());
    let direct = registry.call("fixture", "items", &json!({})).unwrap();
    assert_eq!(direct.content, "visible fixture text");
    assert!(!direct.is_error);
    assert!(direct.details.is_none());
}

#[test]
fn guarded_mcp_delivery_retains_schema_and_rejects_incomplete_or_failed_payloads() {
    for (mixed, failed) in [(false, false), (true, false), (false, true)] {
        let client = davinci_mcp::Client::connect_transport(
            "fixture",
            Box::new(FixtureTransport { mixed, failed }),
        )
        .unwrap();
        let registry = McpRegistry::default();
        {
            let mut inner = registry.lock();
            inner
                .clients
                .insert("fixture".into(), Arc::new(Mutex::new(client)));
            inner.routes.insert(
                "mcp__fixture__items".into(),
                ("fixture".into(), "items".into()),
            );
        }
        assert_eq!(
            registry.output_schema("mcp__fixture__items").unwrap()["required"],
            json!(["items"])
        );
        let workspace = tempfile::tempdir().unwrap();
        let mut agent = crate::Agent::new("guarded MCP fixture");
        agent.cwd = workspace.path().to_path_buf();
        agent.set_permission_mode(crate::PermissionMode::AlwaysApprove);
        agent.attach_mcp(registry);
        let (result, structured, _) = agent.dispatch_script_child(
            workspace.path(),
            "mcp-child",
            "mcp__fixture__items",
            &json!({}),
            None,
            1,
        );
        assert_eq!(result.is_error, mixed || failed, "{result:?}");
        if mixed || failed {
            assert!(structured.is_none());
        } else {
            assert_eq!(structured.unwrap(), json!({"items":[{"id":"A"}]}));
            assert_eq!(result.content, "visible fixture text");
        }
        assert!(!format!("{result:?}").contains("must not be projected"));
    }
}
