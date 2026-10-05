use davinci_agent::codemode::*;
use davinci_coding_agent::codemode_host::{
    assets::{AssetManifest, HostAssets},
    NodeCodeModeHost,
};
use std::path::Path;

fn fixture_host() -> NodeCodeModeHost {
    let root = std::env::var("DAVINCI_CODEMODE_FIXTURE_BUNDLE").unwrap();
    let node = std::env::var("DAVINCI_CODEMODE_FIXTURE_NODE").unwrap();
    let manifest: AssetManifest = serde_json::from_slice(
        &std::fs::read(std::env::var("DAVINCI_CODEMODE_FIXTURE_MANIFEST").unwrap()).unwrap(),
    )
    .unwrap();
    NodeCodeModeHost::new(
        Path::new(&node),
        HostAssets::validate(Path::new(&root), &manifest).unwrap(),
    )
    .unwrap()
}

struct NoTools;
impl CodeModeBroker for NoTools {
    fn search(&self, _: ToolQuery) -> Result<ToolPage, CodeModeError> {
        Ok(ToolPage {
            tools: vec![],
            total: 0,
            cursor: None,
        })
    }
    fn describe(&self, _: &str) -> Result<serde_json::Value, CodeModeError> {
        panic!("no metadata expected")
    }
    fn call(&self, _: CodeModeCall) -> Result<CodeModeToolValue, CodeModeError> {
        panic!("no calls expected")
    }
}

#[test]
#[ignore = "requires the disposable admitted Node and fixture bundle"]
fn native_host_executes_without_model_or_capability_access() {
    let root = std::env::var("DAVINCI_CODEMODE_FIXTURE_BUNDLE").unwrap();
    let node = std::env::var("DAVINCI_CODEMODE_FIXTURE_NODE").unwrap();
    let manifest: AssetManifest = serde_json::from_slice(
        &std::fs::read(std::env::var("DAVINCI_CODEMODE_FIXTURE_MANIFEST").unwrap()).unwrap(),
    )
    .unwrap();
    let assets = HostAssets::validate(Path::new(&root), &manifest).unwrap();
    let host = NodeCodeModeHost::new(Path::new(&node), assets).unwrap();
    let context = CodeModeRunContext {
        identity: CodeModeIdentity {
            invocation_id: "fixture".into(),
            session_id: None,
            branch_leaf: None,
            workspace_binding: "fixture-workspace".into(),
            runtime_run_id: "fixture-run".into(),
            parent_operation_ref: None,
        },
        mode: CodeModeMode::ReadOnly,
        limits: CodeModeLimits::default(),
        capability_revision: "fixture-revision".into(),
        cancellation: Default::default(),
        root_budget: None,
    };
    let outcome = host.execute(
        &CodeModeRequest {
            code: "return 42".into(),
            timeout_ms: Some(1000),
            max_output_bytes: None,
        },
        &context,
        std::sync::Arc::new(NoTools),
    );
    assert!(
        matches!(outcome.status, CodeModeStatus::Completed),
        "{outcome:?}"
    );
    assert!(outcome.output_text.contains("42"));
}

#[test]
#[ignore = "requires the disposable admitted Node and fixture bundle"]
fn native_root_deadline_does_not_restart_after_host_setup() {
    use davinci_agent::runtime::capacity::{BudgetLimits, RootBudget};
    use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
    let host = fixture_host();
    let workspace = tempfile::tempdir().unwrap();
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_millis() as u64;
    let budget = RootBudget::open(
        workspace.path().join("budget.json"),
        "native-root-deadline",
        BudgetLimits {
            max_requests: 1,
            max_output_tokens: Some(100),
            max_cost_microusd: None,
            codex_subscription: None,
            deadline_unix_ms: now + 1000,
        },
    )
    .unwrap();
    let parent = davinci_agent::runtime::CancellationToken::new();
    let context = CodeModeRunContext {
        identity: CodeModeIdentity {
            invocation_id: "root-deadline-fixture".into(),
            session_id: None,
            branch_leaf: None,
            workspace_binding: "fixture".into(),
            runtime_run_id: "fixture".into(),
            parent_operation_ref: None,
        },
        mode: CodeModeMode::ReadOnly,
        limits: CodeModeLimits::default(),
        capability_revision: "fixture".into(),
        cancellation: parent.child_token(),
        root_budget: Some(budget),
    };
    let started = Instant::now();
    let outcome = host.execute(
        &CodeModeRequest {
            code: "while (true) {}".into(),
            timeout_ms: None,
            max_output_bytes: None,
        },
        &context,
        std::sync::Arc::new(NoTools),
    );
    assert!(!outcome.script_completed);
    assert!(
        matches!(
            outcome.error.as_ref().map(|error| error.code.as_str()),
            Some("CANCELLED" | "TIMEOUT")
        ),
        "{outcome:?}"
    );
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "root deadline was replaced by the default wall allowance"
    );
    assert!(!parent.is_cancelled());
}

#[test]
#[ignore = "requires the disposable admitted Node and fixture bundle"]
fn native_host_runs_parallel_read_callbacks() {
    struct ParallelBroker {
        gate: std::sync::Arc<(std::sync::Mutex<usize>, std::sync::Condvar)>,
        active: std::sync::Arc<std::sync::atomic::AtomicUsize>,
        peak: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }
    impl CodeModeBroker for ParallelBroker {
        fn search(&self, _: ToolQuery) -> Result<ToolPage, CodeModeError> {
            Ok(ToolPage {
                tools: vec![ToolMetadata {
                    canonical_name: "parallel_read".into(),
                    js_name: "parallel_read".into(),
                    description: String::new(),
                    read_only: true,
                }],
                total: 1,
                cursor: None,
            })
        }
        fn describe(&self, _: &str) -> Result<serde_json::Value, CodeModeError> {
            unreachable!()
        }
        fn call(&self, call: CodeModeCall) -> Result<CodeModeToolValue, CodeModeError> {
            let now = self
                .active
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst)
                + 1;
            self.peak
                .fetch_max(now, std::sync::atomic::Ordering::SeqCst);
            let (lock, changed) = &*self.gate;
            let mut entered = lock.lock().unwrap();
            *entered += 1;
            if *entered >= 2 {
                changed.notify_all();
            } else {
                let (next, _) = changed
                    .wait_timeout(entered, std::time::Duration::from_millis(700))
                    .unwrap();
                entered = next;
            }
            drop(entered);
            self.active
                .fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
            Ok(CodeModeToolValue {
                text: call.request_id.to_string(),
                structured_content: None,
                complete: true,
                artifact: None,
                operation_ref: format!("fixture-child-{}", call.request_id),
            })
        }
    }

    let host = fixture_host();
    let peak = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let broker = ParallelBroker {
        gate: std::sync::Arc::new((std::sync::Mutex::new(0), std::sync::Condvar::new())),
        active: Default::default(),
        peak: peak.clone(),
    };
    let context = CodeModeRunContext {
        identity: CodeModeIdentity {
            invocation_id: "parallel-fixture".into(),
            session_id: None,
            branch_leaf: None,
            workspace_binding: "fixture".into(),
            runtime_run_id: "fixture".into(),
            parent_operation_ref: None,
        },
        mode: CodeModeMode::ReadOnly,
        limits: CodeModeLimits::default(),
        capability_revision: "fixture".into(),
        cancellation: Default::default(),
        root_budget: None,
    };
    let outcome = host.execute(
        &CodeModeRequest {
            code: "return await Promise.all([tools.parallel_read({}), tools.parallel_read({})])"
                .into(),
            timeout_ms: Some(2000),
            max_output_bytes: None,
        },
        &context,
        std::sync::Arc::new(broker),
    );
    assert!(
        matches!(outcome.status, CodeModeStatus::Completed),
        "{outcome:?}"
    );
    assert_eq!(
        peak.load(std::sync::atomic::Ordering::SeqCst),
        2,
        "the Rust supervisor serialized independent read callbacks"
    );
}

#[test]
#[ignore = "requires the disposable admitted Node and fixture bundle"]
fn native_deadline_returns_when_broker_ignores_cancellation() {
    struct NonCooperativeBroker;
    impl CodeModeBroker for NonCooperativeBroker {
        fn search(&self, _: ToolQuery) -> Result<ToolPage, CodeModeError> {
            Ok(ToolPage {
                tools: vec![ToolMetadata {
                    canonical_name: "stuck_read".into(),
                    js_name: "stuck_read".into(),
                    description: String::new(),
                    read_only: true,
                }],
                total: 1,
                cursor: None,
            })
        }
        fn describe(&self, _: &str) -> Result<serde_json::Value, CodeModeError> {
            unreachable!()
        }
        fn call(&self, _: CodeModeCall) -> Result<CodeModeToolValue, CodeModeError> {
            std::thread::sleep(std::time::Duration::from_millis(1500));
            Ok(CodeModeToolValue {
                text: "late".into(),
                structured_content: None,
                complete: true,
                artifact: None,
                operation_ref: "fixture-child".into(),
            })
        }
    }

    let host = fixture_host();
    let context = CodeModeRunContext {
        identity: CodeModeIdentity {
            invocation_id: "noncooperative-fixture".into(),
            session_id: None,
            branch_leaf: None,
            workspace_binding: "fixture".into(),
            runtime_run_id: "fixture".into(),
            parent_operation_ref: None,
        },
        mode: CodeModeMode::ReadOnly,
        limits: CodeModeLimits::default(),
        capability_revision: "fixture".into(),
        cancellation: Default::default(),
        root_budget: None,
    };
    let started = std::time::Instant::now();
    let outcome = host.execute(
        &CodeModeRequest {
            code: "return await tools.stuck_read({})".into(),
            timeout_ms: Some(100),
            max_output_bytes: None,
        },
        &context,
        std::sync::Arc::new(NonCooperativeBroker),
    );
    assert!(!outcome.script_completed);
    assert!(
        started.elapsed() < std::time::Duration::from_secs(1),
        "a non-cooperative read adapter held Codemode past its deadline: {:?}",
        started.elapsed()
    );
}

#[test]
#[ignore = "requires the disposable admitted Node and fixture bundle"]
fn native_deadline_remains_active_while_rust_waits_for_a_child() {
    struct WaitingBroker(davinci_agent::runtime::CancellationToken);
    impl CodeModeBroker for WaitingBroker {
        fn search(&self, _: ToolQuery) -> Result<ToolPage, CodeModeError> {
            Ok(ToolPage {
                tools: vec![ToolMetadata {
                    canonical_name: "waiting".into(),
                    js_name: "waiting".into(),
                    description: String::new(),
                    read_only: true,
                }],
                total: 1,
                cursor: None,
            })
        }
        fn describe(&self, _: &str) -> Result<serde_json::Value, CodeModeError> {
            unreachable!()
        }
        fn call(&self, _: CodeModeCall) -> Result<CodeModeToolValue, CodeModeError> {
            let started = std::time::Instant::now();
            while !self.0.is_cancelled() {
                assert!(
                    started.elapsed() < std::time::Duration::from_secs(2),
                    "watchdog did not cancel the child"
                );
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            Err(CodeModeError::new("CANCELLED", "fixture child cancelled"))
        }
    }
    let host = fixture_host();
    let parent = davinci_agent::runtime::CancellationToken::new();
    let cancellation = parent.child_token();
    let context = CodeModeRunContext {
        identity: CodeModeIdentity {
            invocation_id: "waiting-fixture".into(),
            session_id: None,
            branch_leaf: None,
            workspace_binding: "fixture".into(),
            runtime_run_id: "fixture".into(),
            parent_operation_ref: None,
        },
        mode: CodeModeMode::ReadOnly,
        limits: CodeModeLimits::default(),
        capability_revision: "fixture".into(),
        cancellation: cancellation.clone(),
        root_budget: None,
    };
    let outcome = host.execute(
        &CodeModeRequest {
            code: "return await tools.waiting({})".into(),
            timeout_ms: Some(500),
            max_output_bytes: None,
        },
        &context,
        std::sync::Arc::new(WaitingBroker(cancellation.clone())),
    );
    assert!(!outcome.script_completed);
    assert!(cancellation.is_cancelled());
    assert!(
        !parent.is_cancelled(),
        "script deadline must not cancel its parent"
    );
}

#[test]
#[ignore = "requires the disposable admitted Node and fixture bundle"]
fn read_only_real_quickjs_records_one_parent_and_two_guarded_children() {
    use davinci_agent::runtime::operations::*;
    use davinci_agent::runtime::{AgentId, RunId, RuntimeBus, RuntimeHandle};
    use serde_json::json;
    use std::sync::Arc;
    let workspace = tempfile::tempdir().unwrap();
    std::fs::write(
        workspace.path().join("catalog.json"),
        r#"{"path":"items.json"}"#,
    )
    .unwrap();
    std::fs::write(
        workspace.path().join("items.json"),
        r#"[{"id":"A","active":true},{"id":"B","active":false}]"#,
    )
    .unwrap();
    let namespace = RootNamespaceId::new();
    let sessions = tempfile::tempdir().unwrap();
    let session =
        davinci_session::JsonlSession::create(sessions.path(), "codemode-fixture", None).unwrap();
    let session_id = session.header.id.clone();
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
        root_namespace_id: namespace,
        session_id: session_id.clone(),
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
        OperationJournal::open(&workspace.path().join("journal"), identity, namespace).unwrap(),
    );
    let operations = ToolOperationRuntime::new(
        journal.clone(),
        context,
        ExecutionOwner::new(ExecutionOwnerId::new(), 1).unwrap(),
        workspace.path(),
    )
    .unwrap();
    let mut agent = davinci_agent::Agent::new("local Codemode fixture");
    agent.cwd = workspace.path().into();
    agent.tools = vec!["read".into()];
    agent.set_permission_mode(davinci_agent::PermissionMode::ReadOnly);
    agent.auto_compaction = false;
    agent.set_runtime(
        RuntimeHandle::new(run_id, agent_id, RuntimeBus::new())
            .with_session(session_id)
            .with_operation_runtime(operations),
    );
    agent.session = Some(session);
    agent
        .enable_read_only_codemode(Arc::new(fixture_host()))
        .unwrap();
    agent.prompt("Read the fixture and return active IDs");
    let code = r#"const first = await tools.read({path:"catalog.json"});
        const catalog = JSON.parse(first.text);
        const second = await tools.read({path:catalog.path});
        return JSON.parse(second.text).filter(item => item.active).map(item => item.id);"#;
    let mut completions = 0;
    agent.run_loop(|_| {
        completions += 1;
        assert!(completions <= 2, "unexpected fixture continuation");
        serde_json::from_value::<davinci_ai::AssistantMessage>(if completions == 1 {
            json!({"id":"script","role":"assistant","model":"fixture","stopReason":"toolUse",
                "content":[{"type":"toolCall","id":"real-script","name":"codemode","arguments":{"code":code}}]})
        } else {
            json!({"id":"done","role":"assistant","model":"fixture","stopReason":"stop",
                "content":[{"type":"text","text":"Done"}]})
        }).map_err(|error| error.to_string())
    }).unwrap();
    let message = agent
        .messages
        .iter()
        .find(|message| message.tool_call_id.as_deref() == Some("real-script"))
        .unwrap();
    assert_eq!(message.is_error, Some(false), "{message:?}");
    let outcome = &message.extra["details"]["codemode"];
    assert_eq!(outcome["outputText"], "[\"A\"]");
    assert_eq!(outcome["children"].as_array().unwrap().len(), 2);
    let snapshot = journal.snapshot().unwrap();
    assert_eq!(snapshot.operations.len(), 3);
    assert_eq!(
        snapshot
            .operations
            .iter()
            .filter(|operation| operation.context().parent_operation_id.is_some())
            .count(),
        2
    );
    assert_eq!(completions, 2); // Offline fixture replies, zero provider requests.
    let entries = &agent.session.as_ref().unwrap().entries;
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry.custom_type.as_deref() == Some("operation_result"))
            .count(),
        2
    );
    assert_eq!(
        agent
            .messages
            .iter()
            .filter(|message| message.role == "toolResult")
            .count(),
        1
    );
}

#[test]
#[ignore = "requires the disposable admitted Node and fixture bundle"]
fn native_budget_failure_cancels_in_flight_children() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    struct BigBroker(std::sync::Arc<AtomicUsize>);
    impl CodeModeBroker for BigBroker {
        fn search(&self, _: ToolQuery) -> Result<ToolPage, CodeModeError> {
            Ok(ToolPage {
                tools: vec![ToolMetadata {
                    canonical_name: "big".into(),
                    js_name: "big".into(),
                    description: String::new(),
                    read_only: true,
                }],
                total: 1,
                cursor: None,
            })
        }
        fn describe(&self, _: &str) -> Result<serde_json::Value, CodeModeError> {
            unreachable!()
        }
        fn call(&self, _: CodeModeCall) -> Result<CodeModeToolValue, CodeModeError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(25));
            Ok(CodeModeToolValue {
                text: "x".repeat(900_000),
                structured_content: None,
                complete: true,
                artifact: None,
                operation_ref: "fixture-child".into(),
            })
        }
    }

    let host = fixture_host();
    let calls = std::sync::Arc::new(AtomicUsize::new(0));
    let context = CodeModeRunContext {
        identity: CodeModeIdentity {
            invocation_id: "budget-fixture".into(),
            session_id: None,
            branch_leaf: None,
            workspace_binding: "fixture".into(),
            runtime_run_id: "fixture".into(),
            parent_operation_ref: None,
        },
        mode: CodeModeMode::ReadOnly,
        limits: CodeModeLimits::default(),
        capability_revision: "fixture".into(),
        cancellation: Default::default(),
        root_budget: None,
    };
    let outcome = host.execute(
        &CodeModeRequest {
            // 30 x 900KB exceeds the 8MB aggregate budget after about 9 replies.
            code: "await Promise.all(Array.from({length: 30}, () => tools.big({}))); return 1"
                .into(),
            timeout_ms: Some(20_000),
            max_output_bytes: None,
        },
        &context,
        std::sync::Arc::new(BigBroker(calls.clone())),
    );
    assert_eq!(
        outcome.error.as_ref().map(|error| error.code.as_str()),
        Some("LIMIT_EXCEEDED"),
        "{outcome:?}"
    );
    // In-flight children observe the run token; an early host error must
    // cancel it, not leave them running against a finished parent.
    assert!(
        context.cancellation.is_cancelled(),
        "run token stayed live after the parent outcome returned"
    );
    let at_return = calls.load(Ordering::SeqCst);
    std::thread::sleep(std::time::Duration::from_millis(500));
    let settled = calls.load(Ordering::SeqCst);
    assert!(
        at_return < 30,
        "budget failure arrived after every child ran"
    );
    // A worker that passed the closed check just before the lane closed may
    // still start one call; at most one per worker, never the queue.
    assert!(
        settled <= at_return + 4,
        "queued children kept dispatching after the parent outcome returned: {at_return} -> {settled}"
    );
}

#[test]
#[ignore = "requires the disposable admitted Node and fixture bundle"]
fn native_escaped_output_beyond_frame_reports_limit_exceeded() {
    let host = fixture_host();
    let context = CodeModeRunContext {
        identity: CodeModeIdentity {
            invocation_id: "escaped-fixture".into(),
            session_id: None,
            branch_leaf: None,
            workspace_binding: "fixture".into(),
            runtime_run_id: "fixture".into(),
            parent_operation_ref: None,
        },
        mode: CodeModeMode::ReadOnly,
        limits: CodeModeLimits::default(),
        capability_revision: "fixture".into(),
        cancellation: Default::default(),
        root_budget: None,
    };
    let outcome = host.execute(
        &CodeModeRequest {
            // Fits the 1MiB collector, but JSON-escapes past the 2MiB frame.
            code: "text(String.fromCharCode(1).repeat(1000000)); return 1".into(),
            timeout_ms: Some(5000),
            max_output_bytes: None,
        },
        &context,
        std::sync::Arc::new(NoTools),
    );
    assert_eq!(
        outcome.error.as_ref().map(|error| error.code.as_str()),
        Some("LIMIT_EXCEEDED"),
        "{outcome:?}"
    );
}
