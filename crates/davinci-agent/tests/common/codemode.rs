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

#[test]
fn codemode_child_uses_real_admission_and_post_hook() {
    let (mut agent, workspace, journal) = configured_agent();
    let trace = Arc::new(Mutex::new(Vec::new()));
    let pre = trace.clone();
    agent.pre_tool = Some(PreToolHook(Arc::new(move |_, _| {
        pre.lock().unwrap().push("pre");
        None
    })));
    let post = trace.clone();
    agent.post_tool = Some(PostToolHook(Arc::new(move |_, _, _, _, mut result| {
        post.lock().unwrap().push("post");
        result.content = "sanitized".into();
        result
    })));
    let (result, structured, _) = agent.dispatch_script_child(
        workspace.path(),
        "script#1",
        "read",
        &json!({"path":"input.txt"}),
        None,
        1,
    );
    assert_eq!(result.content, "sanitized");
    assert!(structured.is_none());
    assert_eq!(*trace.lock().unwrap(), vec!["pre", "post"]);
    assert_eq!(journal.snapshot().unwrap().operations.len(), 1);
}

#[test]
fn denied_codemode_child_has_zero_effects() {
    let (mut agent, workspace, _) = configured_agent();
    agent.pre_tool = Some(PreToolHook(Arc::new(|_, _| Some("denied fixture".into()))));
    let (result, _, _) = agent.dispatch_script_child(
        workspace.path(),
        "script#1",
        "write",
        &json!({"path":"forbidden.txt","content":"effect"}),
        None,
        1,
    );
    assert!(result.is_error);
    assert!(!workspace.path().join("forbidden.txt").exists());
}

struct ReadOnlyFixtureHost;
struct CountedReadOnlyFixtureHost(Arc<std::sync::atomic::AtomicUsize>);
impl crate::codemode::CodeModeHost for CountedReadOnlyFixtureHost {
    fn execute(
        &self,
        request: &crate::codemode::CodeModeRequest,
        context: &crate::codemode::CodeModeRunContext,
        broker: &dyn crate::codemode::CodeModeBroker,
    ) -> crate::codemode::CodeModeOutcome {
        self.0.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        crate::codemode::CodeModeHost::execute(&ReadOnlyFixtureHost, request, context, broker)
    }
}
impl crate::codemode::CodeModeHost for ReadOnlyFixtureHost {
    fn execute(
        &self,
        _: &crate::codemode::CodeModeRequest,
        _context: &crate::codemode::CodeModeRunContext,
        broker: &dyn crate::codemode::CodeModeBroker,
    ) -> crate::codemode::CodeModeOutcome {
        use crate::codemode::*;
        for request_id in [1, 2] {
            assert!(
                broker
                    .call(CodeModeCall {
                        request_id,
                        tool: "read".into(),
                        args: json!({"path":"input.txt"})
                    })
                    .unwrap()
                    .complete
            );
        }
        CodeModeOutcome {
            status: CodeModeStatus::Completed,
            script_completed: true,
            output_text: "fixture".into(),
            output_complete: true,
            output_artifact: None,
            children: vec![],
            host_notes: vec![],
            operation_ref: "forged".into(),
            error: None,
        }
    }
}

#[test]
fn completed_codemode_parent_replays_without_running_the_host() {
    let (mut agent, workspace, journal) = configured_agent();
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    agent
        .enable_read_only_codemode(Arc::new(CountedReadOnlyFixtureHost(calls.clone())))
        .unwrap();
    let args = json!({"code":"return 1"});
    let first = agent
        .execute_tool_batch(
            workspace.path(),
            vec![("same-parent".into(), "codemode".into(), args.clone())],
            &mut Vec::new(),
        )
        .remove(0);
    assert_eq!(first.is_error, Some(false), "{first:?}");
    let second = agent
        .execute_tool_batch(
            workspace.path(),
            vec![("same-parent".into(), "codemode".into(), args)],
            &mut Vec::new(),
        )
        .remove(0);
    assert_eq!(second.is_error, Some(false), "{second:?}");
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(journal.snapshot().unwrap().operations.len(), 3);
    assert_eq!(
        format!("{:?}", first.content),
        format!("{:?}", second.content)
    );
    let collision = agent
        .execute_tool_batch(
            workspace.path(),
            vec![(
                "same-parent".into(),
                "codemode".into(),
                json!({"code":"return 2"}),
            )],
            &mut Vec::new(),
        )
        .remove(0);
    assert_eq!(collision.is_error, Some(true));
    agent
        .permissions
        .lock()
        .unwrap()
        .deny
        .push(crate::permission::PermissionRule::parse("codemode").unwrap());
    let denied = agent
        .execute_tool_batch(
            workspace.path(),
            vec![(
                "same-parent".into(),
                "codemode".into(),
                json!({"code":"return 1"}),
            )],
            &mut Vec::new(),
        )
        .remove(0);
    assert_eq!(denied.is_error, Some(true));
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
}

#[test]
fn codemode_parent_uses_real_dispatch_and_authoritative_child_evidence() {
    let (mut agent, workspace, journal) = configured_agent();
    agent.set_permission_mode(crate::PermissionMode::ReadOnly);
    agent
        .enable_read_only_codemode(Arc::new(ReadOnlyFixtureHost))
        .unwrap();
    let messages = agent.execute_tool_batch(
        workspace.path(),
        vec![(
            "parent-script".into(),
            "codemode".into(),
            json!({"code":"return 1"}),
        )],
        &mut Vec::new(),
    );
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].is_error, Some(false));
    let snapshot = journal.snapshot().unwrap();
    assert_eq!(snapshot.operations.len(), 3);
    let parent = snapshot
        .operations
        .iter()
        .find(|operation| operation.context().wire_tool_call_id.as_deref() == Some("parent-script"))
        .unwrap();
    assert_eq!(
        snapshot
            .operations
            .iter()
            .filter(
                |operation| operation.context().parent_operation_id == Some(parent.operation_id())
            )
            .count(),
        2
    );
    let rendered = format!("{:?}", messages[0]);
    assert!(!rendered.contains("forged"));
    assert!(rendered.contains("children"));
}

#[test]
fn codemode_off_keeps_provider_schema_and_invalid_input_never_launches() {
    struct NeverHost;
    impl crate::codemode::CodeModeHost for NeverHost {
        fn execute(
            &self,
            _: &crate::codemode::CodeModeRequest,
            _: &crate::codemode::CodeModeRunContext,
            _: &dyn crate::codemode::CodeModeBroker,
        ) -> crate::codemode::CodeModeOutcome {
            panic!("invalid input must not launch the host")
        }
    }
    let (mut agent, workspace, journal) = configured_agent();
    assert!(!agent
        .provider_tool_specs()
        .iter()
        .any(|tool| tool.name == "codemode"));
    agent
        .enable_read_only_codemode(Arc::new(NeverHost))
        .unwrap();
    assert!(agent
        .provider_tool_specs()
        .iter()
        .any(|tool| tool.name == "codemode"));
    for args in [
        json!({"code":""}),
        json!({"code":"return 1","mode":"controlled"}),
    ] {
        let messages = agent.execute_tool_batch(
            workspace.path(),
            vec![(
                format!("invalid-{}", journal.snapshot().unwrap().operations.len()),
                "codemode".into(),
                args,
            )],
            &mut Vec::new(),
        );
        assert_eq!(messages[0].is_error, Some(true));
    }
    assert!(journal
        .snapshot()
        .unwrap()
        .operations
        .iter()
        .all(|operation| operation.context().parent_operation_id.is_none()));
}

fn broker_context(agent: &Agent) -> crate::codemode::CodeModeRunContext {
    use crate::codemode::*;
    let runtime = agent.runtime.as_ref().unwrap();
    CodeModeRunContext {
        identity: CodeModeIdentity {
            invocation_id: "broker-fixture".into(),
            session_id: None,
            branch_leaf: None,
            workspace_binding: agent
                .cwd
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .into_owned(),
            runtime_run_id: runtime.run_id.to_string(),
            parent_operation_ref: None,
        },
        mode: CodeModeMode::ReadOnly,
        limits: CodeModeLimits::default(),
        capability_revision: runtime
            .capability_registry
            .hash_tool_capabilities(&agent.tools),
        cancellation: Default::default(),
        root_budget: None,
    }
}

#[test]
fn script_delivery_runs_authoritative_hook_without_model_presentation() {
    let (mut agent, workspace, _) = configured_agent();
    let model_calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let calls = model_calls.clone();
    agent.post_tool = Some(PostToolHook(Arc::new(move |_, _, _, _, mut result| {
        calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        result.content = "model digest".into();
        result
    })));
    agent.script_post_tool = Some(crate::ScriptPostToolHook(Arc::new(
        |_, _, _, _, mut result, mut structured| {
            result.content = result.content.replace("secret", "redacted");
            if let Some(value) = structured.as_mut() {
                value["value"] = json!("redacted");
            }
            (result, structured)
        },
    )));
    let result = crate::ToolResult {
        content: "secret".into(),
        is_error: false,
        details: None,
    };
    let mut structured = Some(json!({"value":"secret"}));
    let script = agent.finalize_tool_result_for_delivery(
        workspace.path(),
        "script",
        "read",
        &json!({}),
        result.clone(),
        Some(&mut structured),
    );
    assert_eq!(script.content, "redacted");
    assert_eq!(structured.unwrap()["value"], "redacted");
    assert_eq!(model_calls.load(std::sync::atomic::Ordering::Relaxed), 0);
    let model = agent.finalize_tool_result(workspace.path(), "model", "read", &json!({}), result);
    assert_eq!(model.content, "model digest");
    assert_eq!(model_calls.load(std::sync::atomic::Ordering::Relaxed), 1);
}

#[test]
fn legacy_post_hook_never_delivers_unredacted_structured_channel() {
    let (mut agent, workspace, _) = configured_agent();
    agent.post_tool = Some(PostToolHook(Arc::new(|_, _, _, _, mut result| {
        result.content = "redacted".into();
        result
    })));
    let mut structured = Some(json!({"value":"secret"}));
    let result = agent.finalize_tool_result_for_delivery(
        workspace.path(),
        "script",
        "read",
        &json!({}),
        crate::ToolResult {
            content: "secret".into(),
            is_error: false,
            details: None,
        },
        Some(&mut structured),
    );
    assert_eq!(result.content, "redacted");
    assert!(structured.is_none());
}

#[test]
fn replayed_script_child_reports_missing_exact_data() {
    let (agent, workspace, journal) = configured_agent();
    let args = json!({"path":"input.txt"});
    let (first, _, _) =
        agent.dispatch_script_child(workspace.path(), "same-child", "read", &args, None, 1);
    assert!(!first.is_error);
    let (replayed, structured, _) =
        agent.dispatch_script_child(workspace.path(), "same-child", "read", &args, None, 1);
    assert!(replayed.is_error);
    assert_eq!(replayed.details.unwrap()["codemode_incomplete"], true);
    assert!(structured.is_none());
    assert_eq!(journal.snapshot().unwrap().operations.len(), 1);
}

#[test]
fn codemode_failure_facts_survive_presentation_hooks() {
    let (mut agent, workspace, _) = configured_agent();
    agent.post_tool = Some(PostToolHook(Arc::new(|_, _, _, _, _| crate::ToolResult {
        content: "decorated output".into(),
        is_error: false,
        details: None,
    })));
    let result = agent.finalize_tool_result(workspace.path(), "parent", "codemode", &json!({}), crate::ToolResult {
        content: "private child text".into(), is_error: true,
        details: Some(json!({"codemode":{"status":"recoveryRequired","scriptCompleted":false,"operationRef":"parent-op","children":[{"requestId":1,"ordinal":1,"tool":"write","operationRef":"child-op","status":"recoveryRequired","error":{"message":"private child text"}}]}})),
    });
    assert!(result.is_error);
    assert!(result.content.contains("recoveryRequired"));
    assert!(result.content.contains("child-op"));
    assert!(!result.content.contains("private child text"));
    assert_eq!(
        result.details.unwrap()["codemode_facts"]["operationRef"],
        "parent-op"
    );
}

#[test]
fn broker_hides_revoked_tools_without_expanding_initial_authority() {
    use crate::codemode::*;
    let (agent, _workspace, _) = configured_agent();
    let context = broker_context(&agent);
    let broker = AgentCodeModeBroker::new(&agent, &context, None).unwrap();
    assert!(broker.describe("read").is_ok());
    agent
        .permissions
        .lock()
        .unwrap()
        .deny
        .push(crate::permission::PermissionRule::parse("read").unwrap());
    assert!(broker
        .search(ToolQuery {
            query: "read".into(),
            limit: 20,
            cursor: None
        })
        .unwrap()
        .tools
        .iter()
        .all(|tool| tool.canonical_name != "read"));
    assert_eq!(broker.describe("read").unwrap_err().code, "DENIED");
    assert_eq!(
        broker
            .call(CodeModeCall {
                request_id: 1,
                tool: "read".into(),
                args: json!({"path":"a.json"})
            })
            .unwrap_err()
            .code,
        "DENIED"
    );
    let initially_denied = AgentCodeModeBroker::new(&agent, &context, None).unwrap();
    agent.permissions.lock().unwrap().deny.clear();
    assert_eq!(
        initially_denied.describe("read").unwrap_err().code,
        "DENIED"
    );
    assert!(initially_denied
        .search(ToolQuery {
            query: "read".into(),
            limit: 20,
            cursor: None,
        })
        .unwrap()
        .tools
        .iter()
        .all(|tool| tool.canonical_name != "read"));
}

#[test]
fn broker_records_projection_failure_with_durable_child_reference() {
    use crate::codemode::*;
    let (agent, _workspace, journal) = configured_agent();
    let mut context = broker_context(&agent);
    context.limits.child_result_bytes = 1;
    let broker = AgentCodeModeBroker::new(&agent, &context, None).unwrap();
    let error = broker
        .call(CodeModeCall {
            request_id: 1,
            tool: "read".into(),
            args: json!({"path":"input.txt"}),
        })
        .unwrap_err();
    assert_eq!(error.code, "INCOMPLETE_DATA");
    assert!(error.operation_ref.is_some());
    let children = broker.children();
    assert_eq!(children.len(), 1);
    assert!(matches!(children[0].status, CodeModeChildStatus::Failed));
    assert_eq!(children[0].operation_ref, error.operation_ref.unwrap());
    assert_eq!(journal.snapshot().unwrap().operations.len(), 1);
}

#[test]
fn broker_rejects_duplicate_calls_and_mutations_before_effects() {
    use crate::codemode::*;
    let (agent, workspace, journal) = configured_agent();
    let context = broker_context(&agent);
    let broker = AgentCodeModeBroker::new(&agent, &context, None).unwrap();
    let call = CodeModeCall {
        request_id: 1,
        tool: "read".into(),
        args: json!({"path":"input.txt"}),
    };
    assert!(broker.call(call.clone()).is_ok());
    assert_eq!(broker.call(call).unwrap_err().code, "LIMIT_EXCEEDED");
    assert!(broker
        .call(CodeModeCall {
            request_id: 2,
            tool: "write".into(),
            args: json!({"path":"forbidden.txt","content":"effect"})
        })
        .is_err());
    assert!(!workspace.path().join("forbidden.txt").exists());
    assert_eq!(journal.snapshot().unwrap().operations.len(), 1);
}

#[test]
fn controlled_broker_uses_real_permissions_and_never_replays_a_mutation() {
    use crate::codemode::*;
    let (mut agent, workspace, journal) = configured_agent();
    agent.set_permission_mode(crate::PermissionMode::AlwaysApprove);
    let mut context = broker_context(&agent);
    context.mode = CodeModeMode::Controlled;
    let broker = AgentCodeModeBroker::new(&agent, &context, None).unwrap();
    let call = CodeModeCall {
        request_id: 1,
        tool: "write".into(),
        args: json!({"path":"effect.txt","content":"once"}),
    };
    assert!(broker.call(call.clone()).is_ok());
    assert_eq!(
        std::fs::read_to_string(workspace.path().join("effect.txt")).unwrap(),
        "once"
    );
    assert!(broker.call(call).is_err());
    assert_eq!(journal.snapshot().unwrap().operations.len(), 1);
    assert_eq!(broker.children().len(), 1);
}

#[test]
fn controlled_broker_denial_is_not_uncertain_mutation_evidence() {
    use crate::codemode::*;
    let (mut agent, workspace, journal) = configured_agent();
    agent.set_permission_mode(crate::PermissionMode::ReadOnly);
    let mut context = broker_context(&agent);
    context.mode = CodeModeMode::Controlled;
    let broker = AgentCodeModeBroker::new(&agent, &context, None).unwrap();
    let error = broker
        .call(CodeModeCall {
            request_id: 1,
            tool: "write".into(),
            args: json!({"path":"denied.txt","content":"never"}),
        })
        .unwrap_err();
    assert_eq!(error.code, "DENIED");
    assert!(!workspace.path().join("denied.txt").exists());
    assert!(matches!(
        broker.children()[0].status,
        CodeModeChildStatus::NotStarted
    ));
    assert!(journal
        .snapshot()
        .unwrap()
        .operations
        .iter()
        .any(|operation| Some(operation.operation_id().to_string()) == error.operation_ref));
    assert!(broker
        .call(CodeModeCall {
            request_id: 2,
            tool: "read".into(),
            args: json!({"path":"input.txt"})
        })
        .is_ok());
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
