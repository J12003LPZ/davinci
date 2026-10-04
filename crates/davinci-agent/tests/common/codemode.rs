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
    std::fs::write(workspace.path().join("input.txt"), "codemode baseline sentinel").unwrap();
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
    let corpus: Value =
        serde_json::from_str(include_str!("../../../../fixtures/codemode/mcp-results.json")).unwrap();
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
