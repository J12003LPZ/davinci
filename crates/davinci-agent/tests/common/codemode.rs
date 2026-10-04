//! Fixture-only characterization of the existing production tool path.
//! Included under turn.rs so private dispatcher APIs remain private.
use crate::runtime::operations::{
    CallerType, EffectStatus, ExecutionOwner, ExecutionOwnerId, JournalId, JournalIdentity,
    OperationContext, OperationJournal, OperationState, RootNamespaceId, ToolOperationRuntime,
    WorkspaceId, WorkspaceIdentity,
};
use crate::runtime::{AgentId, RunId, RuntimeBus, RuntimeHandle};
use crate::{Agent, PostToolHook, PreToolHook, ToolExecutionMode};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

fn configured_agent() -> (Agent, tempfile::TempDir, Arc<OperationJournal>) {
    let workspace = tempfile::tempdir().unwrap();
    std::fs::write(
        workspace.path().join("input.txt"),
        "codemode baseline sentinel",
    )
    .unwrap();
    let root_namespace_id = RootNamespaceId::new();
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
        root_namespace_id,
        session_id: "codemode-baseline-fixture".into(),
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
        OperationJournal::open(
            &workspace.path().join("journal"),
            identity,
            root_namespace_id,
        )
        .unwrap(),
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
    let mut agent = Agent::new("codemode baseline fixture");
    agent.tools = vec!["read".into(), "write".into(), "batch".into()];
    agent.tool_execution_mode = ToolExecutionMode::Sequential;
    agent.cwd = workspace.path().to_path_buf();
    agent.set_runtime(runtime);
    (agent, workspace, journal)
}

fn request(batch: bool, tool: &str, args: Value) -> (String, String, Value) {
    if batch {
        (
            "baseline-batch".into(),
            "batch".into(),
            json!({"operations": [{"tool": tool, "args": args}]}),
        )
    } else {
        ("baseline-direct".into(), tool.into(), args)
    }
}

#[test]
fn direct_and_batch_reads_reach_real_hooks_and_durable_results() {
    for batch in [false, true] {
        let (mut agent, workspace, journal) = configured_agent();
        let trace = Arc::new(Mutex::new(Vec::new()));
        let pre_trace = trace.clone();
        agent.pre_tool = Some(PreToolHook(Arc::new(move |name, _| {
            if name == "read" {
                pre_trace.lock().unwrap().push("pre-read");
            }
            None
        })));
        let post_trace = trace.clone();
        agent.post_tool = Some(PostToolHook(Arc::new(move |_, _, name, _, result| {
            if name == "read" {
                post_trace.lock().unwrap().push("post-read");
            }
            result
        })));
        let messages = agent.execute_tool_batch(
            workspace.path(),
            vec![request(batch, "read", json!({"path": "input.txt"}))],
            &mut Vec::new(),
        );
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].is_error, Some(false));
        assert!(format!("{messages:?}").contains("codemode baseline sentinel"));
        assert_eq!(*trace.lock().unwrap(), vec!["pre-read", "post-read"]);
        assert_eq!(
            agent
                .counters
                .executed_leaf_operations
                .load(std::sync::atomic::Ordering::Relaxed),
            1
        );
        let snapshot = journal.snapshot().unwrap();
        assert_eq!(snapshot.operations.len(), if batch { 2 } else { 1 });
        assert!(snapshot
            .attempts
            .iter()
            .all(|attempt| attempt.state() == OperationState::Succeeded));
        if batch {
            assert_eq!(
                snapshot
                    .operations
                    .iter()
                    .filter(|operation| operation.context().parent_operation_id.is_some())
                    .count(),
                1
            );
        }
    }
}

#[test]
fn direct_and_batch_denied_mutations_have_zero_effects() {
    for batch in [false, true] {
        let (mut agent, workspace, journal) = configured_agent();
        agent.set_permission_mode(crate::PermissionMode::ReadOnly);
        agent.execute_tool_batch(
            workspace.path(),
            vec![request(
                batch,
                "write",
                json!({"path": "denied.txt", "content": "never"}),
            )],
            &mut Vec::new(),
        );
        assert!(!workspace.path().join("denied.txt").exists());
        assert_eq!(
            agent
                .counters
                .executed_leaf_operations
                .load(std::sync::atomic::Ordering::Relaxed),
            0
        );
        let snapshot = journal.snapshot().unwrap();
        assert!(snapshot.attempts.iter().any(|attempt| {
            attempt.state() == OperationState::Cancelled
                && attempt.effect_status() == EffectStatus::NotStarted
                && attempt.authorization().is_none()
        }));
    }
}

#[test]
fn sanitized_mcp_corpus_preserves_baseline_text_and_errors() {
    let corpus: Value = serde_json::from_str(include_str!(
        "../../../../fixtures/codemode/mcp-results.json"
    ))
    .unwrap();
    for case in corpus["results"].as_array().unwrap() {
        let result: davinci_mcp::CallToolResult =
            serde_json::from_value(case["wire"].clone()).unwrap();
        assert_eq!(result.text(), case["expectedText"].as_str().unwrap());
        assert_eq!(
            result.is_error.unwrap_or(false),
            case["expectedError"].as_bool().unwrap()
        );
    }
    let tool: davinci_mcp::ToolSpec = serde_json::from_value(corpus["tool"].clone()).unwrap();
    assert_eq!(tool.name, "fixture_items");
    assert!(tool.read_only());
    // C03 adds retention assertions. This baseline does not pretend that
    // structuredContent/outputSchema already survive the current decoder.
}

/// Reusable local transport: the mutation counter is changed before a lost
/// response, so later recovery tests can distinguish failure from no effect.
struct CountedMcpFixture {
    effects: Arc<std::sync::atomic::AtomicUsize>,
    delay: std::time::Duration,
    lose_response: bool,
}

impl davinci_mcp::RpcTransport for CountedMcpFixture {
    fn call(&mut self, method: &str, _params: Value) -> davinci_mcp::Result<Value> {
        match method {
            "initialize" => Ok(json!({
                "protocolVersion": "2025-03-26",
                "capabilities": {"tools": {}}
            })),
            "tools/list" => Ok(json!({
                "tools": [{"name": "counted_effect", "inputSchema": {"type": "object"}}]
            })),
            "tools/call" => {
                self.effects
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                std::thread::sleep(self.delay);
                if self.lose_response {
                    Err(davinci_mcp::Error::Transport(
                        "fixture response lost".into(),
                    ))
                } else {
                    Ok(json!({
                        "content": [{"type": "text", "text": "fixture effect recorded"}],
                        "structuredContent": {"count": 1}
                    }))
                }
            }
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
fn delayed_response_loss_is_not_no_effect_or_an_automatic_retry() {
    for lose_response in [false, true] {
        let effects = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut client = davinci_mcp::Client::connect_transport(
            "counted_fixture",
            Box::new(CountedMcpFixture {
                effects: effects.clone(),
                delay: std::time::Duration::from_millis(25),
                lose_response,
            }),
        )
        .unwrap();
        let result = client.call_tool("counted_effect", json!({}));
        assert_eq!(effects.load(std::sync::atomic::Ordering::SeqCst), 1);
        if lose_response {
            assert!(matches!(result, Err(davinci_mcp::Error::Transport(_))));
        } else {
            assert_eq!(result.unwrap().text(), "fixture effect recorded");
        }
    }
}
