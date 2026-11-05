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
        broker: Arc<dyn crate::codemode::CodeModeBroker>,
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
        broker: Arc<dyn crate::codemode::CodeModeBroker>,
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
            retained_output: None,
            omitted_bytes: 0,
            omitted_start_line: None,
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
        .push(crate::permission::PermissionRule::parameter(
            "read",
            "path",
            "input.txt",
        ));
    let source_denied = agent
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
    assert_eq!(
        source_denied.is_error,
        Some(true),
        "cached child data must obey current source permissions"
    );
    agent.permissions.lock().unwrap().deny.clear();
    let restored = agent
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
    assert_eq!(restored.is_error, Some(false));
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
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
fn completed_codemode_reopens_with_source_access_binding() {
    let (mut agent, workspace, journal) = configured_agent();
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    agent
        .enable_read_only_codemode(Arc::new(CountedReadOnlyFixtureHost(calls.clone())))
        .unwrap();
    let request = || {
        vec![(
            "reopened-parent".into(),
            "codemode".into(),
            json!({"code":"return 1"}),
        )]
    };
    let first = agent.execute_tool_batch(workspace.path(), request(), &mut Vec::new());
    assert_eq!(first[0].is_error, Some(false));
    let context = agent
        .runtime
        .as_ref()
        .unwrap()
        .operations
        .as_ref()
        .unwrap()
        .operation_context()
        .clone();
    let identity = journal.identity().clone();
    let root = journal.root_namespace_id();
    drop(agent);
    drop(journal);
    let journal = Arc::new(
        OperationJournal::open(&workspace.path().join("journal"), identity, root).unwrap(),
    );
    let operations = ToolOperationRuntime::new(
        journal.clone(),
        context.clone(),
        ExecutionOwner::new(ExecutionOwnerId::new(), 2).unwrap(),
        workspace.path(),
    )
    .unwrap();
    let runtime = RuntimeHandle::new(context.runtime_run_id, context.agent_id, RuntimeBus::new())
        .with_operation_runtime(operations);
    let mut agent = Agent::new("codemode reopened fixture");
    agent.tools = vec!["read".into(), "write".into(), "batch".into()];
    agent.tool_execution_mode = ToolExecutionMode::Sequential;
    agent.cwd = workspace.path().to_path_buf();
    agent.set_runtime(runtime);
    agent
        .enable_read_only_codemode(Arc::new(CountedReadOnlyFixtureHost(calls.clone())))
        .unwrap();
    agent
        .permissions
        .lock()
        .unwrap()
        .deny
        .push(crate::permission::PermissionRule::parameter(
            "read",
            "path",
            "input.txt",
        ));
    let denied = agent.execute_tool_batch(workspace.path(), request(), &mut Vec::new());
    assert_eq!(denied[0].is_error, Some(true));
    agent.permissions.lock().unwrap().deny.clear();
    let replay = agent.execute_tool_batch(workspace.path(), request(), &mut Vec::new());
    assert_eq!(replay[0].is_error, Some(false), "{replay:?}");
    assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert_eq!(journal.snapshot().unwrap().operations.len(), 3);
}

#[test]
fn staged_codemode_registers_on_each_turn_runtime_that_has_a_journal() {
    // Hosts admit Codemode at startup but attach the operation journal per
    // turn. Enabling at startup failed every time; staging must instead wait
    // for the journal and register on each new turn runtime exactly once.
    let mut early = Agent::new("startup without a journal");
    early.stage_read_only_codemode(Arc::new(ReadOnlyFixtureHost));
    assert!(early.activate_staged_codemode().is_err());

    let (mut agent, workspace, _journal) = configured_agent();
    agent.set_permission_mode(crate::PermissionMode::ReadOnly);
    let unstaged = agent.runtime.clone().unwrap();
    agent.stage_read_only_codemode(Arc::new(ReadOnlyFixtureHost));
    assert!(agent
        .runtime
        .as_ref()
        .unwrap()
        .capability_registry
        .get("codemode")
        .is_none());
    agent.activate_staged_codemode().unwrap();
    agent.activate_staged_codemode().unwrap(); // idempotent within one runtime
    assert!(agent
        .runtime
        .as_ref()
        .unwrap()
        .capability_registry
        .get("codemode")
        .is_some());

    // The next turn installs a fresh runtime without the capability.
    let mut next_turn = unstaged;
    next_turn.capability_registry = Default::default();
    agent.set_runtime(next_turn);
    assert!(agent
        .runtime
        .as_ref()
        .unwrap()
        .capability_registry
        .get("codemode")
        .is_none());
    agent.activate_staged_codemode().unwrap();
    assert!(agent
        .runtime
        .as_ref()
        .unwrap()
        .capability_registry
        .get("codemode")
        .is_some());

    let messages = agent.execute_tool_batch(
        workspace.path(),
        vec![(
            "staged".into(),
            "codemode".into(),
            json!({"code":"return 1"}),
        )],
        &mut Vec::new(),
    );
    assert_eq!(messages[0].is_error, Some(false), "{messages:?}");
}

#[test]
fn clearing_codemode_unregisters_it_and_allows_reenable_on_the_same_runtime() {
    let (mut agent, workspace, _journal) = configured_agent();
    agent.set_permission_mode(crate::PermissionMode::ReadOnly);
    agent.stage_read_only_codemode(Arc::new(ReadOnlyFixtureHost));
    agent.activate_staged_codemode().unwrap();
    assert!(agent.tools.iter().any(|name| name == "codemode"));

    agent.clear_codemode();
    let runtime = agent.runtime.as_ref().unwrap();
    assert!(runtime.capability_registry.get("codemode").is_none());
    assert!(!agent.tools.iter().any(|name| name == "codemode"));
    assert!(!agent
        .tool_context
        .tool_exposure
        .lock()
        .unwrap()
        .is_visible("codemode"));

    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    agent.stage_read_only_codemode(Arc::new(CountedReadOnlyFixtureHost(calls.clone())));
    agent.activate_staged_codemode().unwrap();
    let messages = agent.execute_tool_batch(
        workspace.path(),
        vec![(
            "reenabled".into(),
            "codemode".into(),
            json!({"code":"return 1"}),
        )],
        &mut Vec::new(),
    );
    assert_eq!(messages[0].is_error, Some(false), "{messages:?}");
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
fn in_flight_child_is_awaited_before_parent_evidence_is_taken() {
    use crate::codemode::*;
    struct ReturnsEarlyHost(
        Arc<Mutex<Option<std::thread::JoinHandle<()>>>>,
        Arc<std::sync::atomic::AtomicBool>,
    );
    impl CodeModeHost for ReturnsEarlyHost {
        fn execute(
            &self,
            _: &CodeModeRequest,
            _: &CodeModeRunContext,
            broker: Arc<dyn CodeModeBroker>,
        ) -> CodeModeOutcome {
            let worker = std::thread::spawn(move || {
                let _ = broker.call(CodeModeCall {
                    request_id: 1,
                    tool: "read".into(),
                    args: json!({"path":"input.txt"}),
                });
            });
            *self.0.lock().unwrap() = Some(worker);
            // The pre-tool hook raises the flag once the child is admitted.
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while !self.1.load(std::sync::atomic::Ordering::SeqCst)
                && std::time::Instant::now() < deadline
            {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            // Return while the child is still inside its (slow) dispatch, as a
            // host does after an early error.
            CodeModeOutcome {
                status: CodeModeStatus::Failed,
                script_completed: false,
                output_text: String::new(),
                output_complete: false,
                output_artifact: None,
                children: vec![],
                host_notes: vec![],
                operation_ref: "fixture".into(),
                error: Some(CodeModeError::new("LIMIT_EXCEEDED", "fixture early error")),
                retained_output: None,
                omitted_bytes: 0,
                omitted_start_line: None,
            }
        }
    }
    let worker = Arc::new(Mutex::new(None));
    let (mut agent, workspace, _) = configured_agent();
    agent.set_permission_mode(crate::PermissionMode::ReadOnly);
    let admitted = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = admitted.clone();
    agent.pre_tool = Some(PreToolHook(Arc::new(move |name, _| {
        if name == "read" {
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(300));
        }
        None
    })));
    agent
        .enable_read_only_codemode(Arc::new(ReturnsEarlyHost(worker.clone(), admitted.clone())))
        .unwrap();
    let messages = agent.execute_tool_batch(
        workspace.path(),
        vec![(
            "early-script".into(),
            "codemode".into(),
            json!({"code":"return 1"}),
        )],
        &mut Vec::new(),
    );
    worker.lock().unwrap().take().unwrap().join().unwrap();
    assert!(admitted.load(std::sync::atomic::Ordering::SeqCst));
    let rendered = format!("{:?}", messages[0]);
    let details = messages[0].extra["details"]["codemode"].clone();
    assert_eq!(
        details["children"].as_array().map(Vec::len),
        Some(1),
        "the in-flight child was missing from the parent evidence: {rendered}"
    );
}

#[test]
fn undrained_child_makes_the_run_recovery_required() {
    use crate::codemode::*;
    struct FinishesWhileChildRuns(
        Arc<Mutex<Option<std::thread::JoinHandle<()>>>>,
        Arc<std::sync::atomic::AtomicBool>,
    );
    impl CodeModeHost for FinishesWhileChildRuns {
        fn execute(
            &self,
            _: &CodeModeRequest,
            _: &CodeModeRunContext,
            broker: Arc<dyn CodeModeBroker>,
        ) -> CodeModeOutcome {
            let worker = std::thread::spawn(move || {
                let _ = broker.call(CodeModeCall {
                    request_id: 1,
                    tool: "read".into(),
                    args: json!({"path":"input.txt"}),
                });
            });
            *self.0.lock().unwrap() = Some(worker);
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
            while !self.1.load(std::sync::atomic::Ordering::SeqCst)
                && std::time::Instant::now() < deadline
            {
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
            CodeModeOutcome {
                status: CodeModeStatus::Completed,
                script_completed: true,
                output_text: "done".into(),
                output_complete: true,
                output_artifact: None,
                children: vec![],
                host_notes: vec![],
                operation_ref: "fixture".into(),
                error: None,
                retained_output: None,
                omitted_bytes: 0,
                omitted_start_line: None,
            }
        }
    }
    let worker = Arc::new(Mutex::new(None));
    let (mut agent, workspace, _) = configured_agent();
    agent.set_permission_mode(crate::PermissionMode::ReadOnly);
    let admitted = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let flag = admitted.clone();
    // The child ignores cancellation and outlasts the cleanup grace.
    let grace = CodeModeLimits::default().cleanup_grace_ms;
    agent.pre_tool = Some(PreToolHook(Arc::new(move |name, _| {
        if name == "read" {
            flag.store(true, std::sync::atomic::Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(grace + 600));
        }
        None
    })));
    agent
        .enable_read_only_codemode(Arc::new(FinishesWhileChildRuns(
            worker.clone(),
            admitted.clone(),
        )))
        .unwrap();
    let messages = agent.execute_tool_batch(
        workspace.path(),
        vec![(
            "undrained-script".into(),
            "codemode".into(),
            json!({"code":"return 1"}),
        )],
        &mut Vec::new(),
    );
    worker.lock().unwrap().take().unwrap().join().unwrap();
    let details = messages[0].extra["details"]["codemode"].clone();
    assert_eq!(
        details["status"],
        json!("recoveryRequired"),
        "an undrained broker must not look finished: {details}"
    );
}

#[test]
fn retained_broker_cannot_dispatch_after_parent_outcome() {
    use crate::codemode::*;
    struct RetainingHost(Arc<Mutex<Option<Arc<dyn CodeModeBroker>>>>);
    impl CodeModeHost for RetainingHost {
        fn execute(
            &self,
            request: &CodeModeRequest,
            context: &CodeModeRunContext,
            broker: Arc<dyn CodeModeBroker>,
        ) -> CodeModeOutcome {
            *self.0.lock().unwrap() = Some(broker.clone());
            CodeModeHost::execute(&ReadOnlyFixtureHost, request, context, broker)
        }
    }
    let retained = Arc::new(Mutex::new(None));
    let (mut agent, workspace, journal) = configured_agent();
    agent.set_permission_mode(crate::PermissionMode::ReadOnly);
    agent
        .enable_read_only_codemode(Arc::new(RetainingHost(retained.clone())))
        .unwrap();
    let messages = agent.execute_tool_batch(
        workspace.path(),
        vec![(
            "retaining-script".into(),
            "codemode".into(),
            json!({"code":"return 1"}),
        )],
        &mut Vec::new(),
    );
    assert_eq!(messages[0].is_error, Some(false));
    let operations = journal.snapshot().unwrap().operations.len();
    let broker = retained.lock().unwrap().take().unwrap();
    let error = broker
        .call(CodeModeCall {
            request_id: 3,
            tool: "read".into(),
            args: json!({"path":"input.txt"}),
        })
        .unwrap_err();
    assert_eq!(error.code, "CANCELLED", "{error:?}");
    assert_eq!(
        journal.snapshot().unwrap().operations.len(),
        operations,
        "a retained broker dispatched a child after the parent outcome"
    );
}

#[test]
fn codemode_inherits_root_deadline_before_host_launch() {
    struct DeadlineHost(Arc<std::sync::atomic::AtomicU64>);
    impl crate::codemode::CodeModeHost for DeadlineHost {
        fn execute(
            &self,
            request: &crate::codemode::CodeModeRequest,
            context: &crate::codemode::CodeModeRunContext,
            broker: Arc<dyn crate::codemode::CodeModeBroker>,
        ) -> crate::codemode::CodeModeOutcome {
            assert!(context.limits.wall_ms <= 60_000 && context.limits.wall_ms > 0);
            self.0
                .store(context.limits.wall_ms, std::sync::atomic::Ordering::SeqCst);
            crate::codemode::CodeModeHost::execute(&ReadOnlyFixtureHost, request, context, broker)
        }
    }
    for expired in [true, false] {
        let (mut agent, workspace, journal) = configured_agent();
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as u64;
        let budget = crate::runtime::capacity::RootBudget::open(
            workspace.path().join("budget.json"),
            "codemode-root-fixture",
            crate::runtime::capacity::BudgetLimits {
                max_requests: 1,
                max_output_tokens: Some(100),
                max_cost_microusd: None,
                codex_subscription: None,
                // Generous: the fixture's two broker reads must finish inside
                // it even on a loaded CI runner. Only inheritance is tested.
                deadline_unix_ms: if expired { 1 } else { now + 60_000 },
            },
        )
        .unwrap();
        agent.runtime.as_mut().unwrap().root_budget = Some(budget);
        let wall = Arc::new(std::sync::atomic::AtomicU64::new(0));
        agent
            .enable_read_only_codemode(Arc::new(DeadlineHost(wall.clone())))
            .unwrap();
        let result = agent.execute_tool_batch(
            workspace.path(),
            vec![(
                "root-deadline".into(),
                "codemode".into(),
                json!({"code":"return 1"}),
            )],
            &mut Vec::new(),
        );
        assert_eq!(result[0].is_error, Some(expired), "{result:?}");
        assert_eq!(wall.load(std::sync::atomic::Ordering::SeqCst) == 0, expired);
        assert_eq!(
            agent
                .counters
                .executed_leaf_operations
                .load(std::sync::atomic::Ordering::Relaxed),
            if expired { 0 } else { 2 }
        );
        if expired {
            let parent = journal.snapshot().unwrap().operations[0].operation_id();
            let result =
                agent.run_codemode("root-deadline", &json!({"code":"return 1"}), Some(parent));
            assert_eq!(
                result.details.as_ref().unwrap()["codemode"]["status"],
                "cancelled"
            );
            assert_eq!(
                crate::codemode::mandatory_facts(&result).unwrap()["operationRef"],
                parent.to_string()
            );
        }
    }
}

#[test]
fn codemode_off_keeps_provider_schema_and_invalid_input_never_launches() {
    struct NeverHost;
    impl crate::codemode::CodeModeHost for NeverHost {
        fn execute(
            &self,
            _: &crate::codemode::CodeModeRequest,
            _: &crate::codemode::CodeModeRunContext,
            _: Arc<dyn crate::codemode::CodeModeBroker>,
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
    let spec = agent
        .provider_tool_specs()
        .into_iter()
        .find(|tool| tool.name == "codemode")
        .unwrap();
    // The advertised ceilings are the effective ones, never silently clamped.
    let limits = crate::codemode::CodeModeLimits::default();
    let properties = &spec.parameters["properties"];
    assert_eq!(properties["timeoutMs"]["maximum"], json!(limits.wall_ms));
    assert_eq!(
        properties["maxOutputBytes"]["maximum"],
        json!(limits.output_bytes)
    );
    assert_eq!(properties["code"]["maxLength"], json!(limits.script_bytes));
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
        root_budget: runtime.root_budget.clone(),
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
        details: Some(json!({"_codemode_authority":"host-binding","codemode":{"status":"recoveryRequired","scriptCompleted":false,"operationRef":"parent-op","children":[{"requestId":1,"ordinal":1,"tool":"write","operationRef":"child-op","status":"recoveryRequired","error":{"message":"private child text"}}]}})),
    });
    assert!(result.is_error);
    assert!(result.content.contains("recoveryRequired"));
    assert!(result.content.contains("child-op"));
    assert!(!result.content.contains("private child text"));
    let details = result.details.unwrap();
    assert_eq!(details["_codemode_authority"], "host-binding");
    assert_eq!(details["codemode_facts"]["operationRef"], "parent-op");
}

#[test]
fn broker_hides_revoked_tools_without_expanding_initial_authority() {
    use crate::codemode::*;
    let (agent, _workspace, _) = configured_agent();
    let context = broker_context(&agent);
    let broker = AgentCodeModeBroker::new(agent.clone(), context.clone(), None).unwrap();
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
    let initially_denied = AgentCodeModeBroker::new(agent.clone(), context.clone(), None).unwrap();
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
fn broker_stays_inside_a_restricted_agents_tool_ceiling() {
    use crate::codemode::*;
    // A worker's tool list is narrower than what its runtime knows: the
    // capability registry still holds `write`, the worker's ceiling does not.
    let (mut agent, workspace, _) = configured_agent();
    agent.tools = vec!["read".into()];
    agent.sync_tool_authorization();
    let registry = &agent.runtime.as_ref().unwrap().capability_registry;
    assert!(registry.get("write").is_some() && registry.get("grep").is_some());
    let broker = AgentCodeModeBroker::new(agent.clone(), broker_context(&agent), None).unwrap();
    let call = |request_id, tool: &str, args| {
        broker.call(CodeModeCall {
            request_id,
            tool: tool.into(),
            args,
        })
    };
    assert!(
        call(1, "read", json!({"path":"input.txt"}))
            .unwrap()
            .complete
    );
    let denied = call(2, "write", json!({"path":"outside.txt","content":"effect"})).unwrap_err();
    assert!(
        denied.code == "DENIED" || denied.code == "RECOVERY_REQUIRED",
        "{denied:?}"
    );
    // Read-only is not the ceiling: a read-only tool outside the list is out too.
    let outside = call(3, "grep", json!({"pattern":"sentinel"})).unwrap_err();
    assert_eq!(outside.code, "DENIED");
    assert_eq!(broker.describe("grep").unwrap_err().code, "DENIED");
    assert!(broker
        .catalog(64)
        .unwrap()
        .iter()
        .all(|name| name != "write" && name != "grep"));
    assert!(!workspace.path().join("outside.txt").exists());
}

#[test]
fn broker_records_projection_failure_with_durable_child_reference() {
    use crate::codemode::*;
    let (agent, _workspace, journal) = configured_agent();
    let mut context = broker_context(&agent);
    context.limits.child_result_bytes = 1;
    let broker = AgentCodeModeBroker::new(agent.clone(), context.clone(), None).unwrap();
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
fn broker_child_limit_is_terminal_after_guest_catches_error() {
    use crate::codemode::*;
    let (agent, _workspace, journal) = configured_agent();
    let context = broker_context(&agent);
    let broker = AgentCodeModeBroker::new(agent.clone(), context.clone(), None).unwrap();
    for request_id in 1..=64 {
        let result = broker.call(CodeModeCall {
            request_id,
            tool: "read".into(),
            args: json!({"path":"input.txt"}),
        });
        // Deduplication may report unavailable exact data, but never renews admission.
        assert!(result.is_ok() || result.unwrap_err().code == "INCOMPLETE_DATA");
    }
    let operations = journal.snapshot().unwrap().operations.len();
    for request_id in [65, 66, 1] {
        assert_eq!(
            broker
                .call(CodeModeCall {
                    request_id,
                    tool: "read".into(),
                    args: json!({"path":"input.txt"})
                })
                .unwrap_err()
                .code,
            "LIMIT_EXCEEDED"
        );
    }
    assert_eq!(journal.snapshot().unwrap().operations.len(), operations);
    assert_eq!(broker.children().len(), 64);
}

#[test]
fn queued_codemode_child_rechecks_scoped_revocation() {
    use crate::codemode::*;
    let (mut agent, _workspace, journal) = configured_agent();
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let release_rx = Mutex::new(release_rx);
    let hooks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let hook_calls = hooks.clone();
    agent.pre_tool = Some(PreToolHook(Arc::new(move |_, _| {
        if hook_calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst) == 0 {
            entered_tx.send(()).unwrap();
            release_rx
                .lock()
                .unwrap()
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
        }
        None
    })));
    let mut context = broker_context(&agent);
    // A queued-child test must reserve the only lane. Parallel-safe reads
    // otherwise race the revocation and can legitimately finish beforehand.
    context.limits.parallelism = 1;
    let broker = AgentCodeModeBroker::new(agent.clone(), context.clone(), None).unwrap();
    std::thread::scope(|scope| {
        let first = scope.spawn(|| {
            broker.call(CodeModeCall {
                request_id: 1,
                tool: "read".into(),
                args: json!({"path":"input.txt"}),
            })
        });
        entered_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        let second = scope.spawn(|| {
            broker.call(CodeModeCall {
                request_id: 2,
                tool: "read".into(),
                args: json!({"path":"input.txt"}),
            })
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while broker.admitted_request_count() < 2 && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert_eq!(
            broker.admitted_request_count(),
            2,
            "second child must be queued before revocation"
        );
        agent
            .permissions
            .lock()
            .unwrap()
            .deny
            .push(crate::permission::PermissionRule::parameter(
                "read",
                "path",
                "input.txt",
            ));
        release_tx.send(()).unwrap();
        // The first call is still in its pre-hook, before permission admission.
        // Revocation therefore prevents both queued and pre-admission effects.
        assert_eq!(first.join().unwrap().unwrap_err().code, "DENIED");
        assert_eq!(second.join().unwrap().unwrap_err().code, "DENIED");
    });
    assert_eq!(
        agent
            .counters
            .executed_leaf_operations
            .load(std::sync::atomic::Ordering::Relaxed),
        0
    );
    assert!(journal
        .snapshot()
        .unwrap()
        .attempts
        .iter()
        .any(|attempt| attempt.state() == OperationState::Cancelled
            && attempt.effect_status() == EffectStatus::NotStarted));
}

#[test]
fn queued_codemode_child_cancels_before_active_child_returns() {
    use crate::codemode::*;
    let (mut agent, _workspace, _) = configured_agent();
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let release_rx = Mutex::new(release_rx);
    agent.pre_tool = Some(PreToolHook(Arc::new(move |_, _| {
        entered_tx.send(()).unwrap();
        release_rx
            .lock()
            .unwrap()
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        None
    })));
    let mut context = broker_context(&agent);
    // Parallel-safe reads otherwise run side by side; one lane makes the
    // second call genuinely queued behind the active one.
    context.limits.parallelism = 1;
    let broker = AgentCodeModeBroker::new(agent.clone(), context.clone(), None).unwrap();
    let (result_tx, result_rx) = std::sync::mpsc::channel();
    std::thread::scope(|scope| {
        let first = scope.spawn(|| {
            broker.call(CodeModeCall {
                request_id: 1,
                tool: "read".into(),
                args: json!({"path":"input.txt"}),
            })
        });
        entered_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        let second = scope.spawn(|| {
            result_tx
                .send(broker.call(CodeModeCall {
                    request_id: 2,
                    tool: "read".into(),
                    args: json!({"path":"input.txt"}),
                }))
                .unwrap();
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while broker.admitted_request_count() < 2 && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        assert_eq!(broker.admitted_request_count(), 2);
        context.cancellation.cancel();
        let queued_result = result_rx.recv_timeout(std::time::Duration::from_secs(1));
        // Release even on failure so the regression cannot strand its worker.
        release_tx.send(()).unwrap();
        let _ = first.join().unwrap();
        second.join().unwrap();
        assert_eq!(
            queued_result
                .expect("queued cancellation must not wait for the active callback")
                .unwrap_err()
                .code,
            "CANCELLED"
        );
    });
    assert!(!agent
        .runtime
        .as_ref()
        .unwrap()
        .cancellation_token
        .is_cancelled());
    // Both admitted requests leave evidence, including the one never dispatched.
    let children = broker.children();
    assert_eq!(children.len(), 2);
    assert_eq!(children[1].request_id, 2);
    assert!(matches!(children[1].status, CodeModeChildStatus::Cancelled));
}

#[test]
fn child_ordinals_follow_request_ids_not_arrival_order() {
    use crate::codemode::*;
    let (agent, _workspace, _) = configured_agent();
    let broker = AgentCodeModeBroker::new(agent.clone(), broker_context(&agent), None).unwrap();
    for request_id in [2, 1] {
        broker
            .call(CodeModeCall {
                request_id,
                tool: "read".into(),
                args: json!({"path":"input.txt"}),
            })
            .unwrap();
    }
    let children = broker.children();
    assert_eq!(
        children
            .iter()
            .map(|child| (child.request_id, child.ordinal))
            .collect::<Vec<_>>(),
        [(1, 1), (2, 2)]
    );
}

#[test]
fn host_catalog_bootstrap_does_not_spend_guest_metadata_budget() {
    use crate::codemode::*;
    let (agent, _workspace, _) = configured_agent();
    let mut context = broker_context(&agent);
    context.limits.metadata_calls = 1;
    let broker = AgentCodeModeBroker::new(agent.clone(), context, None).unwrap();
    for _ in 0..3 {
        assert!(broker.catalog(1024).unwrap().contains(&"read".to_string()));
    }
    let query = || ToolQuery {
        query: String::new(),
        limit: 20,
        cursor: None,
    };
    assert!(broker.search(query()).is_ok());
    assert_eq!(broker.search(query()).unwrap_err().code, "LIMIT_EXCEEDED");
}

#[test]
fn read_only_broker_allows_bounded_parallel_children() {
    use crate::codemode::*;
    let (mut agent, _workspace, _) = configured_agent();
    let (entered_tx, entered_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let release_rx = Arc::new(Mutex::new(release_rx));
    let active = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let peak = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let hook_active = active.clone();
    let hook_peak = peak.clone();
    let hook_release = release_rx.clone();
    agent.pre_tool = Some(PreToolHook(Arc::new(move |_, _| {
        let now = hook_active.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        hook_peak.fetch_max(now, std::sync::atomic::Ordering::SeqCst);
        entered_tx.send(()).unwrap();
        hook_release
            .lock()
            .unwrap()
            .recv_timeout(std::time::Duration::from_secs(15))
            .unwrap();
        hook_active.fetch_sub(1, std::sync::atomic::Ordering::SeqCst);
        None
    })));
    let mut context = broker_context(&agent);
    context.limits.parallelism = 2;
    let broker = AgentCodeModeBroker::new(agent.clone(), context.clone(), None).unwrap();
    let call = |request_id| {
        broker.call(CodeModeCall {
            request_id,
            tool: "read".into(),
            args: json!({"path":"input.txt"}),
        })
    };
    // Generous: a loaded CI runner may take seconds to schedule a thread. A
    // passing run returns as soon as the event arrives.
    let entered = || entered_rx.recv_timeout(std::time::Duration::from_secs(10));
    std::thread::scope(|scope| {
        let first = scope.spawn(|| call(1));
        entered().expect("first child enters");
        let second = scope.spawn(|| call(2));
        entered().expect("read-only children should overlap up to the parallelism limit");
        // Two are held, which is the limit: a third must wait for a free slot.
        let third = scope.spawn(|| call(3));
        assert!(
            entered_rx
                .recv_timeout(std::time::Duration::from_millis(300))
                .is_err(),
            "a third child ran beside two held ones, past parallelism 2"
        );
        // Freeing one slot lets the third in.
        release_tx.send(()).unwrap();
        entered().expect("the third child enters once a slot frees");
        release_tx.send(()).unwrap();
        release_tx.send(()).unwrap();
        assert!(first.join().unwrap().is_ok());
        assert!(second.join().unwrap().is_ok());
        assert!(third.join().unwrap().is_ok());
    });
    assert_eq!(peak.load(std::sync::atomic::Ordering::SeqCst), 2);
}

#[test]
fn truncated_child_result_is_recorded_as_incomplete_not_plain_success() {
    use crate::codemode::*;
    let (agent, workspace, _) = configured_agent();
    let lines: String = (0..20_000)
        .map(|n| {
            format!(
                "line {n}
"
            )
        })
        .collect();
    std::fs::write(workspace.path().join("big.txt"), lines).unwrap();
    let broker = AgentCodeModeBroker::new(agent.clone(), broker_context(&agent), None).unwrap();
    let call = |request_id, path: &str| CodeModeCall {
        request_id,
        tool: "read".into(),
        args: json!({"path":path}),
    };
    let whole = broker.call(call(1, "input.txt")).unwrap();
    assert!(whole.complete);
    let cut = broker.call(call(2, "big.txt")).unwrap();
    assert!(!cut.complete, "fixture must produce a truncated read");
    let children = broker.children();
    assert!(matches!(children[1].status, CodeModeChildStatus::Succeeded));
    assert!(!children[0].incomplete);
    assert!(children[1].incomplete);
    let wire = serde_json::to_value(&children).unwrap();
    assert!(
        wire[0].get("incomplete").is_none(),
        "false stays off the wire"
    );
    assert_eq!(wire[1]["incomplete"], json!(true));
}

struct TruncatedReadHost;
impl crate::codemode::CodeModeHost for TruncatedReadHost {
    fn execute(
        &self,
        _: &crate::codemode::CodeModeRequest,
        _: &crate::codemode::CodeModeRunContext,
        broker: Arc<dyn crate::codemode::CodeModeBroker>,
    ) -> crate::codemode::CodeModeOutcome {
        use crate::codemode::*;
        let read = |request_id, path: &str| {
            broker
                .call(CodeModeCall {
                    request_id,
                    tool: "read".into(),
                    args: json!({"path":path}),
                })
                .unwrap()
        };
        assert!(read(1, "input.txt").complete);
        assert!(!read(2, "big.txt").complete);
        CodeModeOutcome {
            status: CodeModeStatus::Completed,
            script_completed: true,
            output_text: "summary".into(),
            output_complete: true,
            output_artifact: None,
            children: vec![],
            host_notes: vec![],
            operation_ref: "forged".into(),
            error: None,
            retained_output: None,
            omitted_bytes: 0,
            omitted_start_line: None,
        }
    }
}

#[test]
fn parent_outcome_flags_truncated_children_and_survives_hook_rewrites() {
    let (mut agent, workspace, _) = configured_agent();
    let lines: String = (0..20_000).map(|n| format!("line {n}\n")).collect();
    std::fs::write(workspace.path().join("big.txt"), lines).unwrap();
    agent
        .enable_read_only_codemode(Arc::new(TruncatedReadHost))
        .unwrap();
    let message = agent
        .execute_tool_batch(
            workspace.path(),
            vec![(
                "incomplete-parent".into(),
                "codemode".into(),
                json!({"code":"return 1"}),
            )],
            &mut Vec::new(),
        )
        .remove(0);
    let text = match &message.content[0] {
        davinci_ai::MessageContent::Text { text } => text.clone(),
        other => panic!("unexpected content {other:?}"),
    };
    let outcome: serde_json::Value = serde_json::from_str(&text).unwrap();
    let outcome = &outcome;
    assert_eq!(outcome["status"], "completed", "{outcome}");
    let children = outcome["children"].as_array().unwrap();
    assert_eq!(children.len(), 2);
    assert!(children[0].get("incomplete").is_none());
    assert_eq!(children[1]["incomplete"], true);
    let notes = outcome["hostNotes"].as_array().unwrap();
    assert!(
        notes
            .iter()
            .any(|note| note.as_str().unwrap().starts_with("1 child result(s)")),
        "{notes:?}"
    );
    // A safety hook that rewrites the content keeps the flag as a mandatory fact.
    let facts = crate::codemode::mandatory_facts(&crate::ToolResult {
        content: "redacted".into(),
        is_error: false,
        details: Some(json!({ "codemode": outcome })),
    })
    .unwrap();
    assert_eq!(facts["children"][0]["incomplete"], false);
    assert_eq!(facts["children"][1]["incomplete"], true);
}

#[test]
fn enabled_codemode_tells_the_model_when_to_use_it_and_disabled_says_nothing() {
    let (mut agent, _workspace, _) = configured_agent();
    let off = crate::prompt::runtime_state::runtime_state_text(&agent.runtime_prompt_state());
    assert!(!agent.runtime_prompt_state().codemode_available);
    assert!(!off.contains("Codemode is available"), "{off}");
    agent
        .enable_read_only_codemode(Arc::new(ReadOnlyFixtureHost))
        .unwrap();
    let state = agent.runtime_prompt_state();
    assert!(state.codemode_available);
    let on = crate::prompt::runtime_state::runtime_state_text(&state);
    assert!(on.contains("Codemode is available"), "{on}");
    assert!(on.contains("instead of separate read, grep, find or ls calls"));
    let spec = agent
        .provider_tool_specs()
        .into_iter()
        .find(|tool| tool.name == "codemode")
        .unwrap();
    for needle in [
        "Prefer it to several separate",
        "tools.read(",
        "Not for a single lookup",
        "no undefined fields",
        "parse `.text`",
        "Output past 16 KB",
    ] {
        assert!(spec.description.contains(needle), "{}", spec.description);
    }
}

#[test]
fn staged_codemode_is_announced_before_the_runtime_registers_it() {
    // Print mode prepares the first prompt before `activate_staged_codemode`.
    let mut agent = Agent::new("staged fixture");
    assert!(!agent.runtime_prompt_state().codemode_available);
    agent.stage_read_only_codemode(Arc::new(ReadOnlyFixtureHost));
    assert!(agent.runtime_prompt_state().codemode_available);
    agent.clear_codemode();
    assert!(!agent.runtime_prompt_state().codemode_available);
}

struct EmptyAfterReadHost(&'static str);
impl crate::codemode::CodeModeHost for EmptyAfterReadHost {
    fn execute(
        &self,
        _: &crate::codemode::CodeModeRequest,
        _: &crate::codemode::CodeModeRunContext,
        broker: Arc<dyn crate::codemode::CodeModeBroker>,
    ) -> crate::codemode::CodeModeOutcome {
        use crate::codemode::*;
        broker
            .call(CodeModeCall {
                request_id: 1,
                tool: "read".into(),
                args: json!({"path":"input.txt"}),
            })
            .unwrap();
        CodeModeOutcome {
            status: CodeModeStatus::Completed,
            script_completed: true,
            output_text: self.0.into(),
            output_complete: true,
            output_artifact: None,
            children: vec![],
            host_notes: vec![],
            operation_ref: "forged".into(),
            error: None,
            retained_output: None,
            omitted_bytes: 0,
            omitted_start_line: None,
        }
    }
}

fn run_with_output(output: &'static str) -> (serde_json::Value, crate::stats::RunStats) {
    let (mut agent, workspace, _) = configured_agent();
    agent
        .enable_read_only_codemode(Arc::new(EmptyAfterReadHost(output)))
        .unwrap();
    let message = agent
        .execute_tool_batch(
            workspace.path(),
            vec![(
                "empty-check".into(),
                "codemode".into(),
                json!({"code":"return 1"}),
            )],
            &mut Vec::new(),
        )
        .remove(0);
    let text = match &message.content[0] {
        davinci_ai::MessageContent::Text { text } => text.clone(),
        other => panic!("unexpected content {other:?}"),
    };
    (serde_json::from_str(&text).unwrap(), agent.run_stats())
}

#[test]
fn a_request_refused_before_any_script_runs_is_still_counted() {
    let (mut agent, workspace, _) = configured_agent();
    agent
        .enable_read_only_codemode(Arc::new(EmptyAfterReadHost("[]")))
        .unwrap();
    // `code` must be a string: the request is refused at validation.
    let message = agent
        .execute_tool_batch(
            workspace.path(),
            vec![("bad".into(), "codemode".into(), json!({"code": 7}))],
            &mut Vec::new(),
        )
        .remove(0);
    assert_eq!(message.is_error, Some(true));
    let stats = agent.run_stats();
    assert_eq!((stats.codemode_runs, stats.codemode_unfinished), (1, 1));
}

#[test]
fn feature_use_reports_codemode_off_with_no_calls() {
    let agent = Agent::new("You are a coding agent.");
    assert_eq!(
        agent.feature_use()["codemode"],
        json!({"mode": "off", "available": false, "calls": 0})
    );
}

#[test]
fn feature_use_counts_a_dispatched_codemode_call() {
    let (mut agent, workspace, _) = configured_agent();
    agent
        .enable_read_only_codemode(Arc::new(EmptyAfterReadHost("[]")))
        .unwrap();
    assert_eq!(
        agent.feature_use()["codemode"],
        json!({"mode": "read-only", "available": true, "calls": 0})
    );
    agent.execute_tool_batch(
        workspace.path(),
        vec![("one".into(), "codemode".into(), json!({"code":"return 1"}))],
        &mut Vec::new(),
    );
    assert_eq!(
        agent.feature_use()["codemode"],
        json!({"mode": "read-only", "available": true, "calls": 1})
    );
}

#[test]
fn an_empty_value_after_tool_calls_is_flagged_and_counted() {
    for empty in ["[]", "{}", "null", "", "  "] {
        let (outcome, stats) = run_with_output(empty);
        let notes = outcome["hostNotes"].as_array().unwrap();
        assert!(
            notes
                .iter()
                .any(|n| n.as_str().unwrap().contains("field name")),
            "{empty:?}: {notes:?}"
        );
        assert_eq!(outcome["status"], "completed");
        assert_eq!(
            (
                stats.codemode_runs,
                stats.codemode_empty_results,
                stats.codemode_unfinished
            ),
            (1, 1, 0),
            "{empty:?}"
        );
    }
    // A real answer is not flagged.
    let (outcome, stats) = run_with_output("[\"a\"]");
    assert!(outcome["hostNotes"].as_array().unwrap().is_empty());
    assert_eq!((stats.codemode_runs, stats.codemode_empty_results), (1, 0));
}

#[test]
fn broker_rejects_duplicate_calls_and_mutations_before_effects() {
    use crate::codemode::*;
    let (agent, workspace, journal) = configured_agent();
    let context = broker_context(&agent);
    let broker = AgentCodeModeBroker::new(agent.clone(), context.clone(), None).unwrap();
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
            tool: "tools.Write".into(),
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
    let broker = AgentCodeModeBroker::new(agent.clone(), context.clone(), None).unwrap();
    let call = CodeModeCall {
        request_id: 1,
        tool: "tools.Write".into(),
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
    let broker = AgentCodeModeBroker::new(agent.clone(), context.clone(), None).unwrap();
    let error = broker
        .call(CodeModeCall {
            request_id: 1,
            tool: "tools.Write".into(),
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
            // The effect ran, but its reply was lost: the outcome is unknown,
            // not a plain transport failure the caller could safely retry.
            let error = result.expect_err("a lost reply cannot become a result");
            assert!(error.may_have_run(), "{error}");
            assert!(!error.definitely_not_delivered(), "{error}");
        } else {
            assert_eq!(result.unwrap().text(), "fixture effect recorded");
        }
    }
}

#[test]
fn revoked_while_queued_child_is_denied_before_effect() {
    use crate::codemode::{
        AgentCodeModeBroker, CodeModeBroker as _, CodeModeCall, CodeModeIdentity, CodeModeLimits,
        CodeModeMode, CodeModeRunContext,
    };
    let (mut agent, workspace, _) = configured_agent();
    agent.tools = vec!["read".into(), "write".into()];
    agent.sync_tool_authorization();
    let runtime = agent.runtime.as_ref().unwrap();
    let revision = runtime
        .capability_registry
        .hash_tool_capabilities(&agent.tools);
    let context = CodeModeRunContext {
        identity: CodeModeIdentity {
            invocation_id: "revocation-fixture".into(),
            session_id: runtime.session_id.clone(),
            branch_leaf: None,
            workspace_binding: workspace
                .path()
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .into(),
            runtime_run_id: runtime.run_id.to_string(),
            parent_operation_ref: None,
        },
        mode: CodeModeMode::Controlled,
        limits: CodeModeLimits::default(),
        capability_revision: revision,
        cancellation: runtime.cancellation_token.child_token(),
        root_budget: runtime.root_budget.clone(),
    };
    let broker = AgentCodeModeBroker::new(agent.clone(), context, None).unwrap();
    // The Droid alias resolves through the same admission path as the
    // canonical name, so a script can call tools.read without new authority.
    let aliased = broker
        .call(CodeModeCall {
            request_id: 1,
            tool: "tools.Read".into(),
            args: json!({"path":"input.txt"}),
        })
        .unwrap();
    assert!(aliased.text.contains("codemode baseline sentinel"));
    // Revoke the mutation while the child call waits for its lane.
    agent
        .runtime
        .as_ref()
        .unwrap()
        .capability_registry
        .unregister("write");
    let error = broker
        .call(CodeModeCall {
            request_id: 1,
            tool: "tools.Write".into(),
            args: json!({"path":"forbidden.txt","content":"effect"}),
        })
        .unwrap_err();
    assert!(
        error.code == "STALE_CAPABILITY" || error.code == "DENIED",
        "{error:?}"
    );
    assert!(!workspace.path().join("forbidden.txt").exists());
}

#[test]
fn controlled_script_applies_patch_once() {
    use crate::codemode::{
        AgentCodeModeBroker, CodeModeBroker as _, CodeModeCall, CodeModeIdentity, CodeModeLimits,
        CodeModeMode, CodeModeRunContext,
    };
    let (mut agent, workspace, _) = configured_agent();
    agent.tools = vec![
        "read".into(),
        "write".into(),
        "edit".into(),
        "apply_patch".into(),
    ];
    agent.set_permission_mode(crate::PermissionMode::AlwaysApprove);
    agent.sync_tool_authorization();
    let runtime = agent.runtime.as_ref().unwrap();
    let revision = runtime
        .capability_registry
        .hash_tool_capabilities(&agent.tools);
    let context = CodeModeRunContext {
        identity: CodeModeIdentity {
            invocation_id: "patch-fixture".into(),
            session_id: runtime.session_id.clone(),
            branch_leaf: None,
            workspace_binding: workspace
                .path()
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .into(),
            runtime_run_id: runtime.run_id.to_string(),
            parent_operation_ref: None,
        },
        mode: CodeModeMode::Controlled,
        limits: CodeModeLimits::default(),
        capability_revision: revision,
        cancellation: runtime.cancellation_token.child_token(),
        root_budget: runtime.root_budget.clone(),
    };
    let broker = AgentCodeModeBroker::new(agent.clone(), context, None).unwrap();
    let call = |request_id, tool: &str, args: Value| {
        broker.call(CodeModeCall {
            request_id,
            tool: tool.into(),
            args,
        })
    };
    // Droid spelling `tools.Write` reaches the canonical write tool.
    call(
        1,
        "tools.Write",
        json!({"path":"note.txt","content":"alpha\n"}),
    )
    .unwrap();
    assert_eq!(
        std::fs::read_to_string(workspace.path().join("note.txt")).unwrap(),
        "alpha\n"
    );
    // Droid `tools.ApplyPatch({patch})` is adapted to the native `{input}`.
    let patch = "*** Begin Patch\n*** Update File: note.txt\n@@\n-alpha\n+beta\n*** End Patch";
    call(2, "tools.ApplyPatch", json!({"patch": patch}))
        .expect("the Droid patch shorthand reaches apply_patch");
    let applied = std::fs::read_to_string(workspace.path().join("note.txt")).unwrap();
    assert_eq!(applied.matches("beta").count(), 1, "{applied:?}");
    assert!(!applied.contains("alpha"), "{applied:?}");
    // A denied path is untouched and a later failure never rolls the edit back.
    let denied = call(
        3,
        "tools.Write",
        json!({"path":"../escape.txt","content":"x"}),
    )
    .unwrap_err();
    assert!(
        denied.code == "DENIED" || denied.code == "RECOVERY_REQUIRED",
        "{denied:?}"
    );
    assert!(!workspace
        .path()
        .parent()
        .unwrap()
        .join("escape.txt")
        .exists());
    assert_eq!(
        std::fs::read_to_string(workspace.path().join("note.txt")).unwrap(),
        applied
    );
}

#[test]
fn controlled_script_execute_uses_the_guarded_shell() {
    use crate::codemode::{
        AgentCodeModeBroker, CodeModeBroker as _, CodeModeCall, CodeModeIdentity, CodeModeLimits,
        CodeModeMode, CodeModeRunContext,
    };
    let (mut agent, workspace, _) = configured_agent();
    agent.tools = vec!["exec_command".into()];
    agent.set_permission_mode(crate::PermissionMode::AlwaysApprove);
    agent.sync_tool_authorization();
    let runtime = agent.runtime.as_ref().unwrap();
    let revision = runtime
        .capability_registry
        .hash_tool_capabilities(&agent.tools);
    let context = CodeModeRunContext {
        identity: CodeModeIdentity {
            invocation_id: "execute-fixture".into(),
            session_id: runtime.session_id.clone(),
            branch_leaf: None,
            workspace_binding: workspace
                .path()
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .into(),
            runtime_run_id: runtime.run_id.to_string(),
            parent_operation_ref: None,
        },
        mode: CodeModeMode::Controlled,
        limits: CodeModeLimits::default(),
        capability_revision: revision,
        cancellation: runtime.cancellation_token.child_token(),
        root_budget: runtime.root_budget.clone(),
    };
    let broker = AgentCodeModeBroker::new(agent.clone(), context, None).unwrap();
    let ok = broker
        .call(CodeModeCall {
            request_id: 1,
            tool: "tools.Execute".into(),
            args: json!({"command":"echo codemode-execute"}),
        })
        .unwrap();
    assert!(ok.text.contains("codemode-execute"), "{}", ok.text);
    assert_eq!(
        ok.structured_content.as_ref().unwrap()["exitCode"],
        json!(0)
    );
    // A failing command is a normal result with a nonzero exit, not a
    // protocol error or a lost child.
    let failed = broker
        .call(CodeModeCall {
            request_id: 2,
            tool: "tools.Execute".into(),
            args: json!({"command":"exit 3"}),
        })
        .unwrap();
    assert_eq!(
        failed.structured_content.as_ref().unwrap()["exitCode"],
        json!(3)
    );
    assert_eq!(broker.children().len(), 2);
}

#[test]
fn failed_sibling_keeps_successful_script_evidence() {
    use crate::codemode::{
        AgentCodeModeBroker, CodeModeBroker as _, CodeModeCall, CodeModeChildStatus,
        CodeModeIdentity, CodeModeLimits, CodeModeMode, CodeModeRunContext,
    };
    let (agent, workspace, _) = configured_agent();
    let runtime = agent.runtime.as_ref().unwrap();
    let revision = runtime
        .capability_registry
        .hash_tool_capabilities(&agent.tools);
    let context = CodeModeRunContext {
        identity: CodeModeIdentity {
            invocation_id: "sibling-fixture".into(),
            session_id: runtime.session_id.clone(),
            branch_leaf: None,
            workspace_binding: workspace
                .path()
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .into(),
            runtime_run_id: runtime.run_id.to_string(),
            parent_operation_ref: None,
        },
        mode: CodeModeMode::ReadOnly,
        limits: CodeModeLimits::default(),
        capability_revision: revision,
        cancellation: runtime.cancellation_token.child_token(),
        root_budget: runtime.root_budget.clone(),
    };
    let broker = AgentCodeModeBroker::new(agent.clone(), context, None).unwrap();
    broker
        .call(CodeModeCall {
            request_id: 1,
            tool: "read".into(),
            args: json!({"path":"input.txt"}),
        })
        .unwrap();
    // A sibling that fails never erases the successful child's evidence.
    let _ = broker.call(CodeModeCall {
        request_id: 2,
        tool: "read".into(),
        args: json!({"path":"missing.txt"}),
    });
    let children = broker.children();
    assert_eq!(children.len(), 2);
    assert!(matches!(children[0].status, CodeModeChildStatus::Succeeded));
    assert!(!matches!(
        children[1].status,
        CodeModeChildStatus::Succeeded
    ));
    assert!(children[0].ordinal < children[1].ordinal);
}

#[test]
fn wait_for_script_scoped_handle_and_terminal_status() {
    use crate::codemode::{
        ScriptRunRecord, ScriptRunRegistry, ScriptRunState, WaitAction, WaitForScriptRequest,
    };
    let (mut agent, workspace, _) = configured_agent();
    let evidence = tempfile::tempdir().unwrap();
    agent.evidence = Some(crate::EvidenceStore::new(evidence.path()));
    let evidence_dir = evidence.path().to_path_buf();
    let binding: String = workspace
        .path()
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into();
    let session = agent.runtime.as_ref().unwrap().session_id.clone();
    let registry = ScriptRunRegistry::open(evidence.path().join("script-runs")).unwrap();
    let record = ScriptRunRecord::new(
        "run-wait",
        "call-wait",
        session.as_deref(),
        &binding,
        "revision",
    );
    registry.create(&record).unwrap();
    registry
        .transition("run-wait", ScriptRunState::Running)
        .unwrap();

    // A running handle reports running, never a false completion.
    let status = agent.wait_for_script(WaitForScriptRequest {
        tool_call_id: "run-wait".into(),
        action: WaitAction::Status,
    });
    assert!(!status.is_error, "{}", status.content);
    assert!(status.content.contains("running"), "{}", status.content);

    // A bounded wait on a non-terminal run returns its current state.
    let waited = agent.wait_for_script(WaitForScriptRequest {
        tool_call_id: "run-wait".into(),
        action: WaitAction::Wait,
    });
    assert!(!waited.is_error, "{}", waited.content);
    assert!(waited.content.contains("running"), "{}", waited.content);

    // This process owns the record but has no worker for it, so nothing was
    // observed stopping: Kill reports `recoveryRequired`, never `cancelled`.
    let killed = agent.wait_for_script(WaitForScriptRequest {
        tool_call_id: "run-wait".into(),
        action: WaitAction::Kill,
    });
    assert!(!killed.is_error, "{}", killed.content);
    assert!(
        killed.content.contains("recoveryRequired"),
        "{}",
        killed.content
    );
    assert_eq!(
        registry.get("run-wait").unwrap().state,
        ScriptRunState::RecoveryRequired
    );
    // Repeating it is refused with the unresolved-effect error and changes nothing.
    let again = agent.wait_for_script(WaitForScriptRequest {
        tool_call_id: "run-wait".into(),
        action: WaitAction::Kill,
    });
    assert!(again.is_error, "{}", again.content);
    assert_eq!(
        registry.get("run-wait").unwrap().state,
        ScriptRunState::RecoveryRequired
    );

    // A run another host process owns is refused and left alone.
    let mut foreign_owned = ScriptRunRecord::new(
        "run-other-host",
        "call-oh",
        session.as_deref(),
        &binding,
        "rev",
    );
    // Unique per run: owner locks live in the shared agent directory.
    let foreign_id = format!("1-another-host-{}", uuid::Uuid::new_v4().simple());
    foreign_owned.owner = Some(foreign_id.clone());
    registry.create(&foreign_owned).unwrap();
    // A live owner holds its lock through a handle of its own.
    let owners = davinci_session::default_agent_dir().join("script-owners");
    std::fs::create_dir_all(&owners).unwrap();
    let foreign_path = owners.join(format!("{foreign_id}.lock"));
    // A sweep probing the fresh file can briefly hold it; retry before failing.
    let foreign_lock = (0..10)
        .find_map(|attempt| {
            if attempt > 0 {
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
            crate::runtime::capacity::lock_file_exclusive(&foreign_path, true).unwrap()
        })
        .expect("the foreign lock is free to take");
    registry
        .transition("run-other-host", ScriptRunState::Running)
        .unwrap();
    let refused = agent.wait_for_script(WaitForScriptRequest {
        tool_call_id: "run-other-host".into(),
        action: WaitAction::Kill,
    });
    assert!(refused.is_error, "{}", refused.content);
    assert!(refused.content.contains("NOT_OWNER"), "{}", refused.content);
    assert_eq!(
        registry.get("run-other-host").unwrap().state,
        ScriptRunState::Running
    );
    // Once that owner dies, the same handle resolves instead of staying
    // `NOT_OWNER` forever.
    drop(foreign_lock);
    let resolved = agent.wait_for_script(WaitForScriptRequest {
        tool_call_id: "run-other-host".into(),
        action: WaitAction::Kill,
    });
    assert!(
        resolved.content.contains("recoveryRequired") || resolved.content.contains("RECOVERY"),
        "{}",
        resolved.content
    );
    assert_eq!(
        registry.get("run-other-host").unwrap().state,
        ScriptRunState::RecoveryRequired
    );
    // Resolving a dead owner already removed it; this is only a safety net.
    let _ = std::fs::remove_file(&foreign_path);

    // A handle from another session never resolves.
    let foreign = ScriptRunRegistry::open(evidence_dir.join("script-runs")).unwrap();
    let other = ScriptRunRecord::new(
        "run-foreign",
        "call-foreign",
        Some("another-session"),
        &binding,
        "revision",
    );
    foreign.create(&other).unwrap();
    let denied = agent.wait_for_script(WaitForScriptRequest {
        tool_call_id: "run-foreign".into(),
        action: WaitAction::Status,
    });
    assert!(denied.is_error, "{}", denied.content);
    assert!(denied.content.contains("another session") || denied.content.contains("DENIED"));
}

#[test]
fn reference_script_fixture_suite_passes() {
    use crate::codemode::{
        droid_script_fixture_status, AgentCodeModeBroker, CodeModeBroker as _, CodeModeCall,
        CodeModeChildStatus, CodeModeIdentity, CodeModeLimits, CodeModeMode, CodeModeRunContext,
    };
    // Every reference scenario is either proven here or labelled UNTESTED.
    let fixtures = droid_script_fixture_status();
    assert_eq!(fixtures.len(), 12);
    for fixture in &fixtures {
        assert!(
            matches!(fixture.evidence, "BEHAVIOR_PARITY" | "UNTESTED"),
            "{fixture:?}"
        );
    }
    // No scenario is left unproven: 05 and 09 are covered by the real-guest
    // fixtures in codemode_reference_fixtures.rs.
    let untested: Vec<&str> = fixtures
        .iter()
        .filter(|fixture| fixture.evidence == "UNTESTED")
        .map(|fixture| fixture.id)
        .collect();
    assert!(untested.is_empty(), "untested scenarios: {untested:?}");

    let (mut agent, workspace, _) = configured_agent();
    agent.tools = vec![
        "read".into(),
        "write".into(),
        "exec_command".into(),
        "apply_patch".into(),
    ];
    agent.set_permission_mode(crate::PermissionMode::AlwaysApprove);
    agent.sync_tool_authorization();
    let runtime = agent.runtime.as_ref().unwrap();
    let revision = runtime
        .capability_registry
        .hash_tool_capabilities(&agent.tools);
    let context = CodeModeRunContext {
        identity: CodeModeIdentity {
            invocation_id: "fixture-suite".into(),
            session_id: runtime.session_id.clone(),
            branch_leaf: None,
            workspace_binding: workspace
                .path()
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .into(),
            runtime_run_id: runtime.run_id.to_string(),
            parent_operation_ref: None,
        },
        mode: CodeModeMode::Controlled,
        limits: CodeModeLimits::default(),
        capability_revision: revision,
        cancellation: runtime.cancellation_token.child_token(),
        root_budget: runtime.root_budget.clone(),
    };
    let broker = AgentCodeModeBroker::new(agent.clone(), context, None).unwrap();
    let mut id = 0;
    let mut call = |tool: &str, args: serde_json::Value| {
        id += 1;
        broker.call(CodeModeCall {
            request_id: id,
            tool: tool.into(),
            args,
        })
    };

    // 01 nested read.
    assert!(call("tools.Read", json!({"path":"input.txt"}))
        .unwrap()
        .text
        .contains("codemode baseline sentinel"));
    // 02 guarded command.
    let command = call("tools.Execute", json!({"command":"echo fixture-02"})).unwrap();
    assert_eq!(
        command.structured_content.as_ref().unwrap()["exitCode"],
        json!(0)
    );
    // 03 parallel reads keep their ordinals even when issued together.
    let a = call("tools.Read", json!({"path":"input.txt"}));
    let b = call("tools.Read", json!({"path":"input.txt"}));
    assert!(a.is_ok() && b.is_ok());
    // 04 tool error is a delivered failure, not a lost child.
    let missing = call("tools.Read", json!({"path":"missing.txt"})).unwrap_err();
    assert_eq!(missing.code, "TOOL_FAILED");
    // 06 hostile input: unmapped alias and traversal both deny.
    assert!(call("tools.Delete", json!({})).is_err());
    assert!(call("tools.Read", json!({"path":"../escape.txt"})).is_err());
    // 08 sandbox escape: an out-of-workspace write leaves no file.
    let _ = call("tools.Write", json!({"path":"../escape.txt","content":"x"}));
    assert!(!workspace
        .path()
        .parent()
        .unwrap()
        .join("escape.txt")
        .exists());
    // 10 background notify: refused until the owner approves detached runs
    // (covered by background_script_returns_running_and_can_finish).

    let children = broker.children();
    assert!(children
        .iter()
        .any(|child| matches!(child.status, CodeModeChildStatus::Succeeded)));
    assert!(children
        .iter()
        .any(|child| !matches!(child.status, CodeModeChildStatus::Succeeded)));
}

#[test]
fn controlled_mode_advertises_script_and_wait_tools_with_the_prompt() {
    use crate::codemode::{CodeModeHost, CodeModeMode};

    struct ControlledHost;
    impl CodeModeHost for ControlledHost {
        fn admits_controlled_mode(&self) -> bool {
            true
        }
        fn execute(
            &self,
            _: &crate::codemode::CodeModeRequest,
            _: &crate::codemode::CodeModeRunContext,
            _: Arc<dyn crate::codemode::CodeModeBroker>,
        ) -> crate::codemode::CodeModeOutcome {
            unreachable!()
        }
    }

    let (mut agent, workspace, _) = configured_agent();
    agent.tools = vec!["read".into(), "exec_command".into()];
    agent.sync_tool_authorization();
    let evidence = tempfile::tempdir().unwrap();
    agent.evidence = Some(crate::EvidenceStore::new(evidence.path()));

    agent
        .enable_codemode(Arc::new(ControlledHost), CodeModeMode::Controlled)
        .unwrap();

    // Both model-facing tools are advertised in controlled mode.
    let specs = agent.provider_tool_specs();
    let names: Vec<&str> = specs.iter().map(|spec| spec.name.as_str()).collect();
    assert!(names.contains(&"codemode"), "{names:?}");
    assert!(names.contains(&"wait_for_script"), "{names:?}");

    // The prompt reference is mode-gated and describes the bound aliases.
    let prompt = agent
        .script_mode_prompt_text()
        .expect("controlled mode has a Script prompt");
    assert!(prompt.contains("<script_mode"), "{prompt}");
    assert!(prompt.contains("tools.Read"), "{prompt}");
    assert!(!prompt.contains("riskLevel"), "{prompt}");

    // A forged handle is refused rather than resolving another session's run.
    let denied = agent
        .execute_tool_batch(
            workspace.path(),
            vec![(
                "forged-call".into(),
                "wait_for_script".into(),
                json!({"toolCallId":"run-forged","action":"status"}),
            )],
            &mut Vec::new(),
        )
        .remove(0);
    assert_eq!(denied.is_error, Some(true), "{denied:?}");

    // Disabling Codemode removes both tools and the prompt.
    agent.clear_codemode();
    let after: Vec<String> = agent
        .provider_tool_specs()
        .into_iter()
        .map(|spec| spec.name)
        .collect();
    assert!(!after.iter().any(|name| name == "codemode"));
    assert!(!after.iter().any(|name| name == "wait_for_script"));
    assert!(agent.script_mode_prompt_text().is_none());
}

/// A host that finishes after `delay` and honours cancellation.
struct SlowScriptHost {
    delay: std::time::Duration,
    background: bool,
}
impl crate::codemode::CodeModeHost for SlowScriptHost {
    fn admits_controlled_mode(&self) -> bool {
        true
    }
    fn admits_background_scripts(&self) -> bool {
        self.background
    }
    fn execute(
        &self,
        _: &crate::codemode::CodeModeRequest,
        context: &crate::codemode::CodeModeRunContext,
        _: Arc<dyn crate::codemode::CodeModeBroker>,
    ) -> crate::codemode::CodeModeOutcome {
        use crate::codemode::*;
        let started = std::time::Instant::now();
        while started.elapsed() < self.delay {
            if context.cancellation.is_cancelled() {
                return CodeModeOutcome {
                    status: CodeModeStatus::Cancelled,
                    script_completed: false,
                    output_text: String::new(),
                    output_complete: false,
                    output_artifact: None,
                    children: vec![],
                    host_notes: vec![],
                    operation_ref: "fixture".into(),
                    error: Some(CodeModeError::new("CANCELLED", "worker cancelled")),
                    retained_output: None,
                    omitted_bytes: 0,
                    omitted_start_line: None,
                };
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        CodeModeOutcome {
            status: CodeModeStatus::Completed,
            script_completed: true,
            output_text: "background done".into(),
            output_complete: true,
            output_artifact: None,
            children: vec![],
            host_notes: vec![],
            operation_ref: "fixture".into(),
            error: None,
            retained_output: None,
            omitted_bytes: 0,
            omitted_start_line: None,
        }
    }
}

#[test]
fn retained_script_body_is_stored_and_named_in_the_completion_note() {
    use crate::codemode::{
        CodeModeBroker, CodeModeHost, CodeModeMode, CodeModeOutcome, CodeModeRequest,
        CodeModeRunContext, CodeModeStatus,
    };

    struct TruncatedHost;
    impl CodeModeHost for TruncatedHost {
        fn admits_controlled_mode(&self) -> bool {
            true
        }
        fn execute(
            &self,
            _: &CodeModeRequest,
            _: &CodeModeRunContext,
            _: Arc<dyn CodeModeBroker>,
        ) -> CodeModeOutcome {
            CodeModeOutcome {
                status: CodeModeStatus::Partial,
                script_completed: true,
                output_text: "HEAD\n[... 100 bytes omitted ...]\nTAIL".into(),
                output_complete: false,
                output_artifact: None,
                children: vec![],
                host_notes: vec![],
                operation_ref: "fixture".into(),
                error: None,
                retained_output: Some("HEAD\nNEEDLE\nTAIL".into()),
                omitted_bytes: 100,
                omitted_start_line: Some(2),
            }
        }
    }

    let (mut agent, _workspace, _) = configured_agent();
    let evidence = tempfile::tempdir().unwrap();
    let store = crate::EvidenceStore::new(evidence.path());
    agent.evidence = Some(store.clone());
    agent
        .enable_codemode(Arc::new(TruncatedHost), CodeModeMode::Controlled)
        .unwrap();
    let result = agent.run_codemode("call-retained", &json!({"code":"return 1"}), None);
    let details = result.details.as_ref().unwrap();
    let artifact_id = details["codemode"]["outputArtifact"]["id"]
        .as_str()
        .unwrap();
    assert!(
        artifact_id.starts_with(store.dir().to_string_lossy().as_ref()),
        "{artifact_id}"
    );
    let body = std::fs::read_to_string(artifact_id).unwrap();
    assert!(body.contains("NEEDLE"), "{body}");
    let notes = details["codemode"]["hostNotes"].as_array().unwrap();
    assert!(
        notes
            .iter()
            .any(|note| note.as_str().unwrap().contains("Full output retained at")),
        "{notes:?}"
    );
    assert!(
        notes.iter().any(|note| note
            .as_str()
            .unwrap()
            .contains("class: script_output; error: false")),
        "{notes:?}"
    );
    assert!(
        notes
            .iter()
            .any(|note| note.as_str().unwrap().contains("begins at line 2")),
        "{notes:?}"
    );
}

#[test]
fn no_store_marks_only_truncated_script_output_as_unretained() {
    use crate::codemode::{
        CodeModeBroker, CodeModeHost, CodeModeMode, CodeModeOutcome, CodeModeRequest,
        CodeModeRunContext, CodeModeStatus,
    };

    struct TruncatedHost;
    impl CodeModeHost for TruncatedHost {
        fn admits_controlled_mode(&self) -> bool {
            true
        }
        fn execute(
            &self,
            _: &CodeModeRequest,
            _: &CodeModeRunContext,
            _: Arc<dyn CodeModeBroker>,
        ) -> CodeModeOutcome {
            CodeModeOutcome {
                status: CodeModeStatus::Partial,
                script_completed: true,
                output_text: "HEAD\n[... 100 bytes omitted ...]\nTAIL".into(),
                output_complete: false,
                output_artifact: None,
                children: vec![],
                host_notes: vec![],
                operation_ref: "fixture".into(),
                error: None,
                retained_output: Some("HEAD\nNEEDLE\nTAIL".into()),
                omitted_bytes: 100,
                omitted_start_line: Some(2),
            }
        }
    }

    let (mut agent, _workspace, _) = configured_agent();
    agent.evidence = None;
    agent
        .enable_codemode(Arc::new(TruncatedHost), CodeModeMode::Controlled)
        .unwrap();
    let result = agent.run_codemode("call-no-store", &json!({"code":"return 1"}), None);
    let details = result.details.as_ref().unwrap();
    assert!(details["codemode"]["outputArtifact"].is_null());
    let notes = details["codemode"]["hostNotes"].as_array().unwrap();
    assert!(
        notes.iter().any(|note| note
            .as_str()
            .unwrap()
            .contains("no evidence store in this agent")),
        "{notes:?}"
    );
    assert!(
        notes.iter().any(|note| note
            .as_str()
            .unwrap()
            .contains("class: script_output; error: false")),
        "{notes:?}"
    );
    assert_eq!(details["codemode"]["status"], "partial");
}

#[test]
fn complete_no_store_script_output_has_no_omission_marker() {
    use crate::codemode::{
        CodeModeBroker, CodeModeHost, CodeModeMode, CodeModeOutcome, CodeModeRequest,
        CodeModeRunContext, CodeModeStatus,
    };

    struct CompleteHost;
    impl CodeModeHost for CompleteHost {
        fn admits_controlled_mode(&self) -> bool {
            true
        }
        fn execute(
            &self,
            _: &CodeModeRequest,
            _: &CodeModeRunContext,
            _: Arc<dyn CodeModeBroker>,
        ) -> CodeModeOutcome {
            CodeModeOutcome {
                status: CodeModeStatus::Completed,
                script_completed: true,
                output_text: "complete".into(),
                output_complete: true,
                output_artifact: None,
                children: vec![],
                host_notes: vec![],
                operation_ref: "fixture".into(),
                error: None,
                retained_output: None,
                omitted_bytes: 0,
                omitted_start_line: None,
            }
        }
    }

    let (mut agent, _workspace, _) = configured_agent();
    agent.evidence = None;
    agent
        .enable_codemode(Arc::new(CompleteHost), CodeModeMode::Controlled)
        .unwrap();
    let result = agent.run_codemode("call-complete", &json!({"code":"return 1"}), None);
    let details = result.details.as_ref().unwrap();
    assert!(details["codemode"]["outputArtifact"].is_null());
    let notes = details["codemode"]["hostNotes"]
        .as_array()
        .map(|notes| {
            notes
                .iter()
                .any(|note| note.as_str().unwrap().contains("not retained"))
        })
        .unwrap_or(false);
    assert!(!notes);
    assert_eq!(details["codemode"]["status"], "completed");
}

#[test]
fn failed_script_marks_the_omitted_class_as_error() {
    use crate::codemode::{
        CodeModeBroker, CodeModeError, CodeModeHost, CodeModeMode, CodeModeOutcome,
        CodeModeRequest, CodeModeRunContext, CodeModeStatus,
    };

    struct FailedHost;
    impl CodeModeHost for FailedHost {
        fn admits_controlled_mode(&self) -> bool {
            true
        }
        fn execute(
            &self,
            _: &CodeModeRequest,
            _: &CodeModeRunContext,
            _: Arc<dyn CodeModeBroker>,
        ) -> CodeModeOutcome {
            CodeModeOutcome {
                status: CodeModeStatus::Failed,
                script_completed: false,
                output_text: "HEAD\n[... 10 bytes omitted ...]\nTAIL".into(),
                output_complete: false,
                output_artifact: None,
                children: vec![],
                host_notes: vec![],
                operation_ref: "fixture".into(),
                error: Some(CodeModeError::new("FAILED", "boom")),
                retained_output: Some("HEAD\nNEEDLE\nTAIL".into()),
                omitted_bytes: 10,
                omitted_start_line: Some(2),
            }
        }
    }

    let (mut agent, _workspace, _) = configured_agent();
    let evidence = tempfile::tempdir().unwrap();
    agent.evidence = Some(crate::EvidenceStore::new(evidence.path()));
    agent
        .enable_codemode(Arc::new(FailedHost), CodeModeMode::Controlled)
        .unwrap();
    let result = agent.run_codemode("call-failed", &json!({"code":"throw new Error('x')"}), None);
    let details = result.details.as_ref().unwrap();
    let notes = details["codemode"]["hostNotes"].as_array().unwrap();
    assert!(
        notes.iter().any(|note| note
            .as_str()
            .unwrap()
            .contains("class: script_output; error: true")),
        "{notes:?}"
    );
}

#[test]
fn codemode_retained_body_is_recoverable_through_the_read_tool() {
    use crate::codemode::{
        CodeModeBroker, CodeModeHost, CodeModeMode, CodeModeOutcome, CodeModeRequest,
        CodeModeRunContext, CodeModeStatus,
    };
    use crate::{
        PermissionMode, PermissionPolicy, PermissionRule, PermissionState, ReadOutsideRootPolicy,
    };

    struct TruncatedHost;
    impl CodeModeHost for TruncatedHost {
        fn admits_controlled_mode(&self) -> bool {
            true
        }
        fn execute(
            &self,
            _: &CodeModeRequest,
            _: &CodeModeRunContext,
            _: Arc<dyn CodeModeBroker>,
        ) -> CodeModeOutcome {
            // The real script-output path: a long body whose needle sits past
            // the omitted start, so only the retained body holds it.
            let body = (0..400)
                .map(|i| {
                    if i == 249 {
                        format!("NEEDLE-E2E{}\n", "x".repeat(89))
                    } else {
                        format!("{:0>99}\n", i)
                    }
                })
                .collect::<String>();
            let items = vec![json!({"type": "text", "text": body})];
            let output =
                crate::codemode::projection::assemble_script_output(&items, 16 * 1024, 1024 * 1024)
                    .unwrap();
            assert!(!output.text.contains("NEEDLE-E2E"));
            CodeModeOutcome {
                status: CodeModeStatus::Partial,
                script_completed: true,
                output_text: output.text,
                output_complete: false,
                output_artifact: None,
                children: vec![],
                host_notes: vec![],
                operation_ref: "fixture".into(),
                error: None,
                retained_output: output.retained_body,
                omitted_bytes: output.omitted_bytes,
                omitted_start_line: output.omitted_start_line,
            }
        }
    }

    let (mut agent, workspace, _) = configured_agent();
    let evidence = tempfile::tempdir().unwrap();
    let store = crate::EvidenceStore::new(evidence.path());
    agent.evidence = Some(store.clone());
    agent.permissions = Arc::new(PermissionState::new(PermissionPolicy::new(
        PermissionMode::Ask,
    )));
    {
        let mut policy = agent.permissions.lock().unwrap();
        policy.set_worktree_boundary(workspace.path(), None);
        policy.set_read_outside_root_policy(ReadOutsideRootPolicy::Deny);
    }
    agent
        .enable_codemode(Arc::new(TruncatedHost), CodeModeMode::Controlled)
        .unwrap();
    let result = agent.run_codemode("call-e2e", &json!({"code":"return 1"}), None);
    let artifact = result.details.as_ref().unwrap()["codemode"]["outputArtifact"]["id"]
        .as_str()
        .unwrap()
        .to_string();

    // Read at the line the marker reports, as the model would.
    let notes = result.details.as_ref().unwrap()["codemode"]["hostNotes"]
        .as_array()
        .unwrap();
    let marker = notes
        .iter()
        .filter_map(|note| note.as_str())
        .find(|note| note.contains("the omitted region begins at line"))
        .unwrap();
    let offset: u64 = marker
        .rsplit("offset: ")
        .next()
        .unwrap()
        .split(',')
        .next()
        .unwrap()
        .parse()
        .unwrap();
    assert_eq!(offset, 110, "{marker}");

    let mut events = Vec::new();
    let messages = agent.execute_tool_batch(
        workspace.path(),
        vec![(
            "read-e2e".into(),
            "read".into(),
            json!({"path": artifact, "offset": offset, "limit": 2000}),
        )],
        &mut events,
    );
    assert_eq!(messages[0].is_error, Some(false), "{messages:?}");
    assert!(
        davinci_ai::content_text(&messages[0].content).contains("NEEDLE-E2E"),
        "{messages:?}"
    );

    // Expired: the sweep removes the retained body, so the read must fail.
    std::thread::sleep(std::time::Duration::from_millis(10));
    store.sweep(std::time::Duration::from_millis(1));
    assert!(!std::path::Path::new(&artifact).exists());
    let messages = agent.execute_tool_batch(
        workspace.path(),
        vec![(
            "read-expired".into(),
            "read".into(),
            json!({"path": artifact}),
        )],
        &mut events,
    );
    assert_eq!(messages[0].is_error, Some(true), "{messages:?}");

    // Missing: a path that was never retained.
    let missing = store
        .store_in_session("sessionless", "script-output", "MISSING")
        .unwrap();
    std::fs::remove_file(&missing).unwrap();
    let messages = agent.execute_tool_batch(
        workspace.path(),
        vec![(
            "read-missing".into(),
            "read".into(),
            json!({"path": missing}),
        )],
        &mut events,
    );
    assert_eq!(messages[0].is_error, Some(true), "{messages:?}");

    // Denied: the evidence exception still yields to an explicit deny rule.
    let denied = store
        .store_in_session("sessionless", "script-output", "DENIED")
        .unwrap();
    agent
        .permissions
        .lock()
        .unwrap()
        .deny
        .push(PermissionRule::bare("read"));
    let messages = agent.execute_tool_batch(
        workspace.path(),
        vec![("read-denied".into(), "read".into(), json!({"path": denied}))],
        &mut events,
    );
    assert_eq!(messages[0].is_error, Some(true), "{messages:?}");
}

#[test]
fn read_can_open_agent_evidence_under_deny_outside_root() {
    use crate::{PermissionMode, PermissionPolicy, PermissionState, ReadOutsideRootPolicy};
    let (mut agent, workspace, _) = configured_agent();
    let evidence = tempfile::tempdir().unwrap();
    let store = crate::EvidenceStore::new(evidence.path());
    let path = store
        .store_in_session("sessionless", "script-output", "RECOVERED EVIDENCE")
        .unwrap();
    agent.evidence = Some(store);
    agent.permissions = Arc::new(PermissionState::new(PermissionPolicy::new(
        PermissionMode::Ask,
    )));
    {
        let mut policy = agent.permissions.lock().unwrap();
        policy.set_worktree_boundary(workspace.path(), None);
        policy.set_read_outside_root_policy(ReadOutsideRootPolicy::Deny);
    }
    let mut events = Vec::new();
    let messages = agent.execute_tool_batch(
        workspace.path(),
        vec![(
            "evidence-read".into(),
            "read".into(),
            json!({"path": path.to_string_lossy().to_string()}),
        )],
        &mut events,
    );
    let message = &messages[0];
    assert_eq!(message.is_error, Some(false), "{message:?}");
    assert!(
        davinci_ai::content_text(&message.content).contains("RECOVERED EVIDENCE"),
        "{message:?}"
    );
}

/// Guards come before the agent so they drop after it: the workspace and
/// evidence directories must outlive background workers through shutdown.
fn background_agent(
    delay: std::time::Duration,
    admit: bool,
) -> (tempfile::TempDir, tempfile::TempDir, crate::Agent) {
    let (mut agent, workspace, _) = configured_agent();
    let evidence = tempfile::tempdir().unwrap();
    agent.evidence = Some(crate::EvidenceStore::new(evidence.path()));
    agent
        .enable_codemode(
            Arc::new(SlowScriptHost {
                delay,
                background: admit,
            }),
            crate::codemode::CodeModeMode::Controlled,
        )
        .unwrap();
    (workspace, evidence, agent)
}

fn background_request() -> serde_json::Value {
    json!({"code":"// @script: {\"startInBackground\": true}\nreturn 1;"})
}

#[test]
fn background_script_returns_running_and_can_finish() {
    use crate::codemode::{ScriptRunState, WaitAction, WaitForScriptRequest};

    let (_workspace, evidence, mut agent) =
        background_agent(std::time::Duration::from_millis(200), true);
    let started = agent.run_codemode("call-bg", &background_request(), None);
    assert!(!started.is_error, "{started:?}");
    let handle: serde_json::Value = serde_json::from_str(&started.content).unwrap();
    assert_eq!(handle["state"], json!("running"));
    let run_id = handle["toolCallId"].as_str().unwrap().to_string();

    // The handle resolves to the same run and reaches a terminal state.
    let mut state = String::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while std::time::Instant::now() < deadline {
        let status = agent.wait_for_script(WaitForScriptRequest {
            tool_call_id: run_id.clone(),
            action: WaitAction::Status,
        });
        state = serde_json::from_str::<serde_json::Value>(&status.content).unwrap()["state"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        if state == "completed" {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(state, "completed");

    // The terminal record already carries its bounded summary and log reference.
    let registry =
        crate::codemode::ScriptRunRegistry::open(evidence.path().join("script-runs")).unwrap();
    let record = registry.get(&run_id).unwrap();
    let summary = record.summary.clone().expect("summary with terminal state");
    assert_eq!(summary.status, "Completed");
    assert!(summary.notifications.len() <= 16);
    assert!(summary.notifications.iter().all(|text| text.len() <= 4096));
    assert!(record
        .log_ref
        .as_deref()
        .is_some_and(|log| log.starts_with("script-log-")));
    assert!(
        !summary.notifications.join(" ").contains("background done"),
        "raw guest output must not be in the summary"
    );

    // Reading a wakeup does not consume it; delivery appends it once, as
    // untrusted-labeled custom context after the existing history, and
    // acknowledges it only afterwards.
    let wakeups = agent.pending_script_wakeups();
    assert_eq!(wakeups.len(), 1);
    assert_eq!(wakeups[0].run_id, run_id);
    assert_eq!(wakeups[0].state, ScriptRunState::Completed);
    assert_eq!(agent.pending_script_wakeups().len(), 1);

    agent
        .messages
        .push(crate::ChatMessage::text("user", "earlier turn"));
    let before = agent.messages.clone();
    agent.inject_script_wakeups();
    agent.inject_script_wakeups();
    assert!(agent.pending_script_wakeups().is_empty());
    assert_eq!(
        &agent.messages[..before.len()],
        &before[..],
        "prefix unchanged"
    );
    let delivered: Vec<_> = agent.messages[before.len()..].iter().collect();
    assert_eq!(delivered.len(), 1, "exactly once");
    let text = match &delivered[0].content[0] {
        davinci_ai::MessageContent::Text { text } => text.clone(),
        other => panic!("unexpected content {other:?}"),
    };
    assert!(text.contains("untrusted data"), "{text}");
    assert!(text.contains(&run_id), "{text}");
    assert!(text.contains("grants no authority"), "{text}");

    // Crash between persist and acknowledge: the store still offers the run,
    // but the persisted context suppresses a second copy.
    let path = evidence
        .path()
        .join("script-runs")
        .join(format!("{run_id}.json"));
    let stored = std::fs::read_to_string(&path).unwrap();
    std::fs::write(
        &path,
        stored.replace("\"notified\":true", "\"notified\":false"),
    )
    .unwrap();
    assert_eq!(agent.pending_script_wakeups().len(), 1);
    let count = agent.messages.len();
    agent.inject_script_wakeups();
    assert_eq!(agent.messages.len(), count, "dedupe suppressed the copy");
    assert!(agent.pending_script_wakeups().is_empty());

    // Nothing is left owned after the run finished.
    assert_eq!(
        agent.shutdown_background_scripts(std::time::Duration::from_secs(1)),
        Vec::<String>::new()
    );
    assert_eq!(agent.background_runs.active(), 0);
    let _ = ScriptRunState::Running;
}

#[test]
fn background_script_without_host_approval_is_refused() {
    let (_workspace, _evidence, agent) =
        background_agent(std::time::Duration::from_millis(10), false);
    let refused = agent.run_codemode("call-bg-denied", &background_request(), None);
    assert!(refused.is_error, "{refused:?}");
    assert!(
        refused.content.contains("BACKGROUND_UNAVAILABLE"),
        "{}",
        refused.content
    );
    assert_eq!(agent.background_runs.active(), 0);
}

#[test]
fn killed_background_script_is_cancelled_and_not_replayed() {
    use crate::codemode::{ScriptRunRegistry, ScriptRunState, WaitAction, WaitForScriptRequest};

    let (_workspace, evidence, agent) = background_agent(std::time::Duration::from_secs(5), true);
    let started = agent.run_codemode("call-bg-kill", &background_request(), None);
    assert!(!started.is_error, "{started:?}");
    let run_id = serde_json::from_str::<serde_json::Value>(&started.content).unwrap()["toolCallId"]
        .as_str()
        .unwrap()
        .to_string();

    let kill = agent.wait_for_script(WaitForScriptRequest {
        tool_call_id: run_id.clone(),
        action: WaitAction::Kill,
    });
    assert!(!kill.is_error, "{kill:?}");
    assert!(kill.content.contains("cancelled"), "{}", kill.content);

    // Kill alone stopped the worker and the worker recorded its own outcome;
    // no shutdown was needed.
    assert_eq!(agent.background_runs.active(), 0);
    let registry = ScriptRunRegistry::open(evidence.path().join("script-runs")).unwrap();
    let record = registry.get(&run_id).unwrap();
    assert_eq!(record.state, ScriptRunState::Cancelled, "{record:?}");
    // Shutdown is safe afterwards, and safe twice.
    for _ in 0..2 {
        assert_eq!(
            agent.shutdown_background_scripts(std::time::Duration::from_secs(1)),
            Vec::<String>::new()
        );
    }
    // A cancelled run is terminal and is never replayed.
    assert!(record.state.terminal());
    let repeated = agent.wait_for_script(WaitForScriptRequest {
        tool_call_id: run_id.clone(),
        action: WaitAction::Kill,
    });
    assert!(!repeated.is_error, "{repeated:?}");
    assert_eq!(
        registry.get(&run_id).unwrap().state,
        ScriptRunState::Cancelled
    );
}

#[test]
fn replacing_the_session_stops_detached_scripts() {
    use crate::codemode::{ScriptRunRegistry, ScriptRunState};

    let (_workspace, evidence, mut agent) =
        background_agent(std::time::Duration::from_secs(5), true);
    let started = agent.run_codemode("call-bg-replace", &background_request(), None);
    assert!(!started.is_error, "{started:?}");
    let run_id = serde_json::from_str::<serde_json::Value>(&started.content).unwrap()["toolCallId"]
        .as_str()
        .unwrap()
        .to_string();
    assert_eq!(agent.background_runs.active(), 1);

    let root = tempfile::tempdir().unwrap();
    let replacement =
        davinci_session::JsonlSession::create(root.path(), "/tmp/replacement", None).unwrap();
    agent.load_from_session(replacement).unwrap();

    assert_eq!(agent.background_runs.active(), 0);
    let registry = ScriptRunRegistry::open(evidence.path().join("script-runs")).unwrap();
    assert_eq!(
        registry.get(&run_id).unwrap().state,
        ScriptRunState::Cancelled
    );
    // The agent still accepts runs for the new session.
    let next = agent.run_codemode("call-bg-after", &background_request(), None);
    assert!(!next.is_error, "{next:?}");
    agent.shutdown_session_scripts();
}

#[test]
fn drain_refuses_launches_and_reopens() {
    let (_workspace, _evidence, agent) = background_agent(std::time::Duration::from_secs(5), true);
    agent.background_runs.close();
    let refused = agent.run_codemode("call-bg-closing", &background_request(), None);
    assert!(refused.is_error, "{refused:?}");
    assert!(refused.content.contains("closing"), "{}", refused.content);
    assert_eq!(agent.background_runs.active(), 0);
    agent.background_runs.reopen();
    let started = agent.run_codemode("call-bg-open", &background_request(), None);
    assert!(!started.is_error, "{started:?}");
    agent.shutdown_session_scripts();
}

#[test]
fn explicit_branch_navigation_hides_runs_started_on_another_branch() {
    use crate::codemode::{WaitAction, WaitForScriptRequest};
    use davinci_session::SessionEntry;

    let (_workspace, _evidence, mut agent) =
        background_agent(std::time::Duration::from_secs(5), true);
    let root = tempfile::tempdir().unwrap();
    let mut session =
        davinci_session::JsonlSession::create(root.path(), "/tmp/branch", None).unwrap();
    for text in ["one", "two"] {
        session
            .append_entry(SessionEntry::message(
                "user",
                json!([{"type":"text","text":text}]),
            ))
            .unwrap();
    }
    let first = session.entries[0].id.clone();
    agent.session = Some(session);

    let started = agent.run_codemode("call-bg-branch", &background_request(), None);
    let run_id = serde_json::from_str::<serde_json::Value>(&started.content).unwrap()["toolCallId"]
        .as_str()
        .unwrap()
        .to_string();
    let status = |agent: &crate::Agent| {
        agent.wait_for_script(WaitForScriptRequest {
            tool_call_id: run_id.clone(),
            action: WaitAction::Status,
        })
    };
    assert!(!status(&agent).is_error);

    agent
        .navigate_tree_entry(&first, false, None, false, 16_384)
        .unwrap();
    let denied = status(&agent);
    assert!(denied.is_error, "{}", denied.content);
    assert!(denied.content.contains("DENIED"), "{}", denied.content);
    assert!(agent.pending_script_wakeups().is_empty());
    let second = agent.session.as_ref().unwrap().entries[1].id.clone();
    let session_path = agent.session.as_ref().unwrap().path.clone();
    agent.shutdown_session_scripts();

    // A restart keeps no in-memory counter: a fresh agent on the same session
    // applies the same rule, and returning to the branch restores access.
    let (mut fresh, _fresh_workspace, _) = configured_agent();
    fresh.cwd = agent.cwd.clone();
    fresh.evidence = Some(crate::EvidenceStore::new(_evidence.path()));
    let mut reopened = davinci_session::JsonlSession::open(&session_path).unwrap();
    reopened.set_leaf(Some(first));
    fresh.session = Some(reopened);
    assert!(status(&fresh).is_error, "abandoned branch stays hidden");
    fresh.session.as_mut().unwrap().set_leaf(Some(second));
    assert!(!status(&fresh).is_error, "the run's own branch resolves");
}

#[test]
fn script_todo_alias_requires_the_host_gate_end_to_end() {
    use crate::codemode::{
        AgentCodeModeBroker, CodeModeBroker as _, CodeModeCall, CodeModeIdentity, CodeModeLimits,
        CodeModeMode, CodeModeRunContext, ScriptToolGates,
    };
    let build_broker = |gates: ScriptToolGates| {
        let (mut agent, workspace, _) = configured_agent();
        agent.tools = vec!["read".into(), "todo".into()];
        agent.set_script_tool_gates(gates);
        agent.sync_tool_authorization();
        let runtime = agent.runtime.as_ref().unwrap();
        let revision = runtime
            .capability_registry
            .hash_tool_capabilities(&agent.tools);
        let context = CodeModeRunContext {
            identity: CodeModeIdentity {
                invocation_id: "todo-gate-fixture".into(),
                session_id: runtime.session_id.clone(),
                branch_leaf: None,
                workspace_binding: workspace
                    .path()
                    .canonicalize()
                    .unwrap()
                    .to_string_lossy()
                    .into(),
                runtime_run_id: runtime.run_id.to_string(),
                parent_operation_ref: None,
            },
            mode: CodeModeMode::Controlled,
            limits: CodeModeLimits::default(),
            capability_revision: revision,
            cancellation: runtime.cancellation_token.child_token(),
            root_budget: runtime.root_budget.clone(),
        };
        // The workspace guard comes first so it drops after the broker.
        (
            workspace,
            AgentCodeModeBroker::new(agent, context, None).unwrap(),
        )
    };

    // Closed by default: the alias does not resolve.
    let (_closed_workspace, closed) = build_broker(ScriptToolGates::default());
    let denied = closed
        .call(CodeModeCall {
            request_id: 1,
            tool: "tools.TodoWrite".into(),
            args: json!({"items":[{"text":"step","status":"pending"}]}),
        })
        .unwrap_err();
    assert_eq!(denied.code, "DENIED", "{denied:?}");

    // Explicitly opened, the same alias reaches the host-owned todo tool.
    let (_open_workspace, open) = build_broker(ScriptToolGates {
        allow_todo: true,
        allow_subagents: false,
    });
    let value = open
        .call(CodeModeCall {
            request_id: 1,
            tool: "tools.TodoWrite".into(),
            args: json!({"items":[{"text":"step","status":"pending"}]}),
        })
        .expect("the gated alias reaches the todo tool");
    assert!(value.operation_ref.starts_with("ephemeral:") || !value.operation_ref.is_empty());
}

#[test]
fn script_history_is_persisted_with_a_stable_wire_kind() {
    let (mut agent, workspace, _) = configured_agent();
    agent
        .enable_read_only_codemode(Arc::new(ReadOnlyFixtureHost))
        .unwrap();
    let messages = agent.execute_tool_batch(
        workspace.path(),
        vec![(
            "history-call".into(),
            "codemode".into(),
            json!({"code":"return 1"}),
        )],
        &mut Vec::new(),
    );
    // The persisted tool-result facts carry the append-only history item, so a
    // later mode switch or model change replays it instead of rewriting it.
    let facts = messages[0].extra["details"]["codemode_facts"].clone();
    let history = &facts["history"];
    assert_eq!(history["wireKind"], json!("function"), "{facts}");
    assert_eq!(history["wireName"], json!("codemode"), "{facts}");
    assert_eq!(history["version"], json!(1), "{facts}");
    assert_eq!(history["complete"], json!(true), "{facts}");
    assert_eq!(history["callId"], facts["operationRef"], "{facts}");
}

// Hybrid Context VM with Controlled CodeMode: an actual broker child runs, an
// automatic Hybrid fold replaces the transcript that held its result, and the
// result is recovered from the Context VM and the retained script log without
// running the script again.
const SCRIPT_MARK: &str = "SCRIPT-SENTINEL";

fn edit_input(old: &str, new: &str) -> Value {
    json!({"path":"input.txt","edits":[{"oldText":old,"newText":new}]})
}

/// Runs one scripted `edit` child per execution through the real broker and
/// returns a large output carrying `SCRIPT_MARK`.
struct HybridScriptHost {
    runs: Arc<std::sync::atomic::AtomicUsize>,
    edits: Mutex<std::collections::VecDeque<Value>>,
    child_errors: Mutex<Vec<String>>,
}

fn script_output(run: usize) -> String {
    (0..150)
        .map(|line| format!("{SCRIPT_MARK} run {run} line {line} {}\n", "x".repeat(40)))
        .collect()
}

impl crate::codemode::CodeModeHost for HybridScriptHost {
    fn admits_controlled_mode(&self) -> bool {
        true
    }
    fn execute(
        &self,
        _: &crate::codemode::CodeModeRequest,
        _: &crate::codemode::CodeModeRunContext,
        broker: Arc<dyn crate::codemode::CodeModeBroker>,
    ) -> crate::codemode::CodeModeOutcome {
        use crate::codemode::*;
        let run = self.runs.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        let args = self
            .edits
            .lock()
            .unwrap()
            .pop_front()
            .expect("scripted edit");
        let call = CodeModeCall {
            request_id: 1,
            tool: "edit".into(),
            args,
        };
        if let Err(error) = broker.call(call) {
            self.child_errors.lock().unwrap().push(error.code);
        }
        CodeModeOutcome {
            status: CodeModeStatus::Completed,
            script_completed: true,
            output_text: script_output(run),
            output_complete: true,
            output_artifact: None,
            children: vec![],
            host_notes: vec![],
            operation_ref: "forged".into(),
            error: None,
            retained_output: None,
            omitted_bytes: 0,
            omitted_start_line: None,
        }
    }
}

fn assistant_reply(
    content: Vec<davinci_ai::ContentBlock>,
    input: u64,
) -> davinci_ai::AssistantMessage {
    davinci_ai::AssistantMessage {
        extra: Default::default(),
        id: "hybrid-codemode-fixture".into(),
        role: "assistant".into(),
        model: "fixture".into(),
        usage: Some(davinci_protocol::Usage {
            input,
            output: 4,
            ..Default::default()
        }),
        error_message: None,
        stop_reason: Some(match content.first() {
            Some(davinci_ai::ContentBlock::ToolCall { .. }) => davinci_ai::StopReason::ToolUse,
            _ => davinci_ai::StopReason::Stop,
        }),
        content,
    }
}

/// The request's own size at four bytes a token, as the provider's count.
fn measured_tokens(agent: &Agent) -> u64 {
    serde_json::to_vec(&davinci_ai::openai_responses_input(
        &agent.messages_for_provider(),
    ))
    .unwrap()
    .len() as u64
        / 4
}

fn done_reply(agent: &Agent) -> davinci_ai::AssistantMessage {
    assistant_reply(
        vec![davinci_ai::ContentBlock::Text {
            text: "done".into(),
        }],
        measured_tokens(agent),
    )
}

/// One user turn in which the provider emits a single CodeMode call.
fn run_script_turn(agent: &mut Agent, call_id: &str) {
    agent.prompt("Run the script.");
    let mut sent = false;
    agent
        .run_loop(|agent: &Agent| {
            Ok::<_, String>(if std::mem::replace(&mut sent, true) {
                done_reply(agent)
            } else {
                assistant_reply(
                    vec![davinci_ai::ContentBlock::ToolCall {
                        id: call_id.into(),
                        name: "codemode".into(),
                        arguments: json!({"code":"return 1"}),
                    }],
                    measured_tokens(agent),
                )
            })
        })
        .expect("script turn runs");
}

fn folds(agent: &Agent) -> u64 {
    agent.runtime.as_ref().unwrap().context_vm.metrics().folds
}

fn filler_turn(agent: &mut Agent, turn: usize) {
    agent.prompt(&format!(
        "Continue with step {turn}. {}",
        "notes ".repeat(300)
    ));
    agent
        .run_loop(|agent: &Agent| Ok::<_, String>(done_reply(agent)))
        .expect("filler turn runs");
}

/// Plain turns until an automatic Hybrid fold has run and been applied;
/// returns how many it took.
fn fold_until_applied(agent: &mut Agent) -> usize {
    let (folds_before, applied_before) = (folds(agent), agent.hybrid_compaction_stats.applied);
    for turn in 0..200 {
        filler_turn(agent, turn);
        if folds(agent) > folds_before && agent.hybrid_compaction_stats.applied > applied_before {
            return turn + 1;
        }
    }
    panic!("no automatic Hybrid fold was applied");
}

/// Operations that are children of another operation, i.e. script children.
fn all_child_operations(journal: &OperationJournal) -> usize {
    let snapshot = journal.snapshot().unwrap();
    snapshot
        .operations
        .iter()
        .filter(|op| op.context().parent_operation_id.is_some())
        .count()
}

/// One A/B row, readable with `--nocapture`.
fn log_ab(label: &str, agent: &Agent, runs: usize, journal: &OperationJournal, recovered: usize) {
    eprintln!(
        "AB {label}: executions={runs} edit_children={} folds={} applied={} compactions={} recovered={recovered} marker_in_provider={}",
        all_child_operations(journal),
        folds(agent),
        agent.hybrid_compaction_stats.applied,
        agent.run_stats().compactions,
        provider_transcript(agent).contains(SCRIPT_MARK),
    );
}

fn provider_transcript(agent: &Agent) -> String {
    agent
        .messages_for_provider()
        .iter()
        .map(|message| davinci_ai::content_text(&message.content))
        .collect::<Vec<_>>()
        .join("\n")
}

fn tool_result_text(agent: &Agent, call_id: &str) -> String {
    let message = agent
        .messages
        .iter()
        .find(|message| message.tool_call_id.as_deref() == Some(call_id))
        .expect("tool result in the transcript");
    davinci_ai::content_text(&message.content)
}

/// Every result answers an earlier call and every call has its result.
fn assert_tool_pairs(messages: &[davinci_ai::ChatMessage]) {
    let mut open = std::collections::HashSet::new();
    for message in messages {
        for block in &message.content {
            if let davinci_ai::MessageContent::ToolCall { id, .. } = block {
                assert!(open.insert(id.clone()), "duplicate call {id}");
            }
        }
        if message.role == "toolResult" {
            let id = message.tool_call_id.as_ref().expect("result carries an id");
            assert!(open.remove(id), "result {id} without an earlier call");
        }
    }
    assert!(open.is_empty(), "calls without results: {open:?}");
}

struct HybridCodemodeFixture {
    agent: Agent,
    /// Keeps the session file alive.
    _sessions: tempfile::TempDir,
    workspace: tempfile::TempDir,
    evidence: tempfile::TempDir,
    journal: Arc<OperationJournal>,
    host: Arc<HybridScriptHost>,
}

/// Hybrid mode, automatic compaction, an evidence store and a summarizer that
/// cites every script result it is shown, with Controlled CodeMode.
///
/// Fixture limits: the summarizer always cites script results it sees, so the
/// Hybrid tests prove survival and recovery of cited evidence, not that a real
/// summarizer keeps the references. Script evidence is found the way a model
/// finds it, by reading the references in the provider view and calling
/// `retrieve_context`; whether a real model does so is not shown here.
fn hybrid_codemode_fixture(edits: Vec<Value>) -> HybridCodemodeFixture {
    use crate::runtime::ContextVmMode;
    let (mut agent, workspace, journal) = configured_agent();
    let evidence = tempfile::tempdir().unwrap();
    agent.evidence = Some(crate::EvidenceStore::new(evidence.path()));
    // A real session file, so every tool result is an authoritative entry the
    // Context VM can anchor evidence references to.
    let sessions = tempfile::tempdir().unwrap();
    let session = davinci_session::JsonlSession::create(
        sessions.path(),
        workspace.path().to_str().unwrap(),
        None,
    )
    .unwrap();
    let session_id = session.header.id.clone();
    agent.session = Some(session);
    let runtime = agent.runtime.clone().unwrap().with_session(session_id);
    agent.set_runtime(runtime);
    agent.tools = vec![
        "read".into(),
        "write".into(),
        "edit".into(),
        "batch".into(),
        "retrieve_context".into(),
    ];
    agent.set_permission_mode(crate::PermissionMode::AlwaysApprove);
    let host = Arc::new(HybridScriptHost {
        runs: Arc::default(),
        edits: Mutex::new(edits.into()),
        child_errors: Mutex::default(),
    });
    agent
        .enable_codemode(host.clone(), crate::codemode::CodeModeMode::Controlled)
        .unwrap();
    agent.set_context_vm_mode(ContextVmMode::Hybrid);
    agent.auto_compaction = true;
    agent.context_window = 32_000;
    agent.compaction = crate::CompactionSettings {
        reserve_tokens: 2_000,
        keep_recent_tokens: 1_000,
        ..crate::CompactionSettings::default()
    };
    agent.summarizer = Some(crate::Summarizer::new(|request| {
        let text = if request.label == "context state fold" {
            let cited = request
                .prompt
                .lines()
                .filter_map(|line| serde_json::from_str::<Value>(line).ok())
                .filter(|event| {
                    event["text"]
                        .as_str()
                        .is_some_and(|text| text.contains(SCRIPT_MARK))
                })
                .map(|event| {
                    json!({"value":"a CodeMode script ran","source_refs":[event["source_ref"]],
                        "provenance_kind":"tool_evidence"})
                })
                .collect::<Vec<_>>();
            json!({ "completed": cited }).to_string()
        } else {
            "## Goal\nContinue.".into()
        };
        Ok(crate::SummarizeResponse {
            text,
            usage: Some(Default::default()),
            stop_reason: Some(davinci_ai::StopReason::Stop),
            error_message: None,
            has_tool_call: false,
        })
    }));
    HybridCodemodeFixture {
        agent,
        _sessions: sessions,
        workspace,
        evidence,
        journal,
        host,
    }
}

/// One `retrieve_context` call through the tool surface, as the model makes it.
fn retrieve_context_call(
    agent: &mut Agent,
    cwd: &std::path::Path,
    call_id: &str,
    input: Value,
) -> (bool, String) {
    let out = agent.execute_tool_batch(
        cwd,
        vec![(call_id.into(), "retrieve_context".into(), input)],
        &mut Vec::new(),
    );
    (
        out[0].is_error == Some(true),
        davinci_ai::content_text(&out[0].content),
    )
}

/// Script evidence references named anywhere in `text`.
fn evidence_uris(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for scheme in [
        crate::evidence::SCRIPT_LOG_URI,
        crate::evidence::SCRIPT_OUTPUT_URI,
    ] {
        for (at, _) in text.match_indices(scheme) {
            let name: String = text[at + scheme.len()..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
                .collect();
            let uri = format!("{scheme}{name}");
            if !found.contains(&uri) {
                found.push(uri);
            }
        }
    }
    found
}

/// The text of every script result the surviving ledger cites, retrieved by
/// source reference the way the model would.
fn recovered_script_results(agent: &Agent) -> Vec<String> {
    use crate::runtime::context_vm::{cited_source_refs, RetrieveContextRequest};
    let vm = &agent.runtime.as_ref().unwrap().context_vm;
    let state = vm.load_state_from_root().unwrap();
    cited_source_refs(&state)
        .into_iter()
        .filter_map(|source_ref| {
            vm.retrieve(&RetrieveContextRequest {
                source_ref: Some(source_ref),
                ..Default::default()
            })
            .ok()
        })
        .map(|result| {
            assert!(!result.truncated, "one page holds the whole result");
            result.content
        })
        .filter(|content| content.contains(SCRIPT_MARK))
        .collect()
}

/// Operations whose parent is the tool call `call_id`.
fn child_operations(journal: &OperationJournal, call_id: &str) -> usize {
    let snapshot = journal.snapshot().unwrap();
    let parents = snapshot
        .operations
        .iter()
        .filter(|op| op.context().wire_tool_call_id.as_deref() == Some(call_id))
        .collect::<Vec<_>>();
    assert_eq!(parents.len(), 1, "one parent operation for {call_id}");
    let parent = parents[0].operation_id();
    snapshot
        .operations
        .iter()
        .filter(|op| op.context().parent_operation_id == Some(parent))
        .count()
}

#[test]
fn hybrid_fold_preserves_script_evidence_without_rerunning_the_script() {
    let mut fixture = hybrid_codemode_fixture(vec![edit_input(
        "codemode baseline sentinel",
        "edited by script",
    )]);
    let runs = fixture.host.runs.clone();
    let executions = || runs.load(std::sync::atomic::Ordering::SeqCst);
    let input = || std::fs::read_to_string(fixture.workspace.path().join("input.txt")).unwrap();
    run_script_turn(&mut fixture.agent, "script-call-1");
    let original = tool_result_text(&fixture.agent, "script-call-1");
    assert!(original.contains("run 1 line 149"));
    assert_eq!(executions(), 1);
    assert_eq!(input(), "edited by script");
    assert_eq!(child_operations(&fixture.journal, "script-call-1"), 1);
    let (folds_before, applied_before) = (
        folds(&fixture.agent),
        fixture.agent.hybrid_compaction_stats.applied,
    );

    let filler_turns = fold_until_applied(&mut fixture.agent);
    assert!(folds(&fixture.agent) > folds_before);
    assert!(fixture.agent.hybrid_compaction_stats.applied > applied_before);
    // The fold replaced the transcript that held the script result.
    assert!(
        !provider_transcript(&fixture.agent).contains(SCRIPT_MARK),
        "the script result must have left the provider transcript"
    );
    assert_tool_pairs(&fixture.agent.messages_for_provider());

    // Discovery: the surviving ledger cites the result; retrieval returns it exactly.
    assert_eq!(
        recovered_script_results(&fixture.agent),
        vec![original.clone()]
    );
    // The model's own view names the retained log; `retrieve_context` on that
    // reference returns the script's exact output.
    let view = provider_transcript(&fixture.agent);
    let log_uri = evidence_uris(&view)
        .into_iter()
        .find(|uri| uri.starts_with(crate::evidence::SCRIPT_LOG_URI))
        .expect("the provider view names the script log");
    let (is_error, text) = retrieve_context_call(
        &mut fixture.agent,
        fixture.workspace.path(),
        "retrieve-log",
        json!({"sourceRef": log_uri, "query": "run 1 line 149", "offset": 0, "limit": 120}),
    );
    assert!(!is_error, "{text}");
    let page: Value = serde_json::from_str(&text).unwrap();
    assert!(
        page["content"]
            .as_str()
            .unwrap()
            .contains(script_output(1).lines().last().unwrap()),
        "{text}"
    );

    // No re-execution: one host run, one child, the edit applied once.
    assert_eq!(executions(), 1);
    assert_eq!(child_operations(&fixture.journal, "script-call-1"), 1);
    assert_eq!(input(), "edited by script");
    assert!(fixture.host.child_errors.lock().unwrap().is_empty());
    assert_tool_pairs(&fixture.agent.messages_for_provider());
    log_ab(
        "hybrid+codemode",
        &fixture.agent,
        executions(),
        &fixture.journal,
        recovered_script_results(&fixture.agent).len(),
    );
    eprintln!("AB hybrid+codemode: filler_turns_to_fold={filler_turns}");
}

#[test]
fn hybrid_denied_operation_after_fold_remains_denied() {
    let mut fixture = hybrid_codemode_fixture(vec![
        edit_input("codemode baseline sentinel", "edited by script"),
        edit_input("edited by script", "denied edit"),
    ]);
    let runs = fixture.host.runs.clone();
    let executions = || runs.load(std::sync::atomic::Ordering::SeqCst);
    let input = || std::fs::read_to_string(fixture.workspace.path().join("input.txt")).unwrap();

    // The permitted edit takes effect.
    run_script_turn(&mut fixture.agent, "script-call-1");
    assert_eq!(input(), "edited by script");
    assert!(fixture.host.child_errors.lock().unwrap().is_empty());

    // The mode cycles between turns; the next tool batch sees it.
    assert_eq!(
        fixture.agent.permission_mode(),
        crate::PermissionMode::AlwaysApprove
    );
    let mut cycled = 0;
    while fixture.agent.cycle_permission_mode() != crate::PermissionMode::ReadOnly {
        cycled += 1;
        assert!(cycled < 5, "the mode never reached read-only");
    }
    run_script_turn(&mut fixture.agent, "script-call-2");
    let denied = tool_result_text(&fixture.agent, "script-call-2");
    let parsed: Value = serde_json::from_str(&denied).unwrap();
    assert_eq!(parsed["status"], json!("partial"));
    assert_eq!(parsed["children"][0]["status"], json!("notStarted"));
    assert_eq!(parsed["children"][0]["error"]["code"], json!("DENIED"));
    assert_eq!(*fixture.host.child_errors.lock().unwrap(), vec!["DENIED"]);
    assert_eq!(
        input(),
        "edited by script",
        "the denied edit changed nothing"
    );
    assert_eq!(executions(), 2);
    let denied_children = child_operations(&fixture.journal, "script-call-2");
    let (folds_before, applied_before) = (
        folds(&fixture.agent),
        fixture.agent.hybrid_compaction_stats.applied,
    );

    fold_until_applied(&mut fixture.agent);
    assert!(folds(&fixture.agent) > folds_before);
    assert!(fixture.agent.hybrid_compaction_stats.applied > applied_before);
    assert!(
        !provider_transcript(&fixture.agent).contains(SCRIPT_MARK),
        "both script results must have left the provider transcript"
    );
    assert_tool_pairs(&fixture.agent.messages_for_provider());

    // The denial is recovered from the surviving ledger exactly as it was
    // delivered, and the retained script log records the child as NotStarted.
    let recovered = recovered_script_results(&fixture.agent);
    assert_eq!(recovered.len(), 2, "both script results stay recoverable");
    assert_eq!(
        recovered
            .iter()
            .filter(|text| text.contains("run 2 line"))
            .collect::<Vec<_>>(),
        vec![&denied]
    );
    let view = provider_transcript(&fixture.agent);
    let log_uris = evidence_uris(&view)
        .into_iter()
        .filter(|uri| uri.starts_with(crate::evidence::SCRIPT_LOG_URI))
        .collect::<Vec<_>>();
    assert_eq!(log_uris.len(), 2, "the view names both script logs: {view}");
    let mut log = String::new();
    for (index, uri) in log_uris.iter().enumerate() {
        for query in ["NotStarted", "DENIED"] {
            let (is_error, text) = retrieve_context_call(
                &mut fixture.agent,
                fixture.workspace.path(),
                &format!("retrieve-log-{index}-{query}"),
                json!({"sourceRef": uri, "query": query, "offset": 0, "limit": 120}),
            );
            assert!(!is_error);
            log.push_str(&text);
        }
    }
    assert!(log.contains("edit request 1 ordinal 1 NotStarted"));
    assert!(log.contains("DENIED"));

    // Nothing ran again and nothing changed.
    assert_eq!(executions(), 2);
    assert_eq!(*fixture.host.child_errors.lock().unwrap(), vec!["DENIED"]);
    assert_eq!(
        child_operations(&fixture.journal, "script-call-2"),
        denied_children
    );
    assert_eq!(input(), "edited by script");
    assert_eq!(
        fixture.agent.permission_mode(),
        crate::PermissionMode::ReadOnly
    );
    assert_tool_pairs(&fixture.agent.messages_for_provider());
}

/// Control for the Hybrid test: the same fixture, provider script and context
/// growth with Context VM off and CodeMode off. The script cannot run, so there
/// is no script output to recover; the contrast asserted is that nothing ran,
/// no Hybrid fold happened, and the transcript kept every turn.
#[test]
fn context_vm_and_codemode_off_control_runs_no_script_and_folds_nothing() {
    use crate::runtime::ContextVmMode;
    let mut fixture = hybrid_codemode_fixture(vec![edit_input(
        "codemode baseline sentinel",
        "edited by script",
    )]);
    fixture.agent.clear_codemode();
    fixture.agent.set_context_vm_mode(ContextVmMode::Off);
    assert!(!fixture
        .agent
        .provider_tool_specs()
        .iter()
        .any(|tool| tool.name == "codemode"));
    let (folds_before, applied_before) = (
        folds(&fixture.agent),
        fixture.agent.hybrid_compaction_stats.applied,
    );
    let compactions_before = fixture.agent.run_stats().compactions;

    // The same provider script: a codemode call, then the same context growth.
    run_script_turn(&mut fixture.agent, "script-call-1");
    let refused = fixture
        .agent
        .messages
        .iter()
        .find(|message| message.tool_call_id.as_deref() == Some("script-call-1"))
        .expect("the call still gets a result");
    assert_eq!(refused.is_error, Some(true), "{refused:?}");
    for turn in 0..60 {
        filler_turn(&mut fixture.agent, turn);
    }

    let executions = fixture.host.runs.load(std::sync::atomic::Ordering::SeqCst);
    assert_eq!(executions, 0);
    assert_eq!(all_child_operations(&fixture.journal), 0);
    assert_eq!(
        std::fs::read_to_string(fixture.workspace.path().join("input.txt")).unwrap(),
        "codemode baseline sentinel"
    );
    assert_eq!(folds(&fixture.agent), folds_before);
    assert_eq!(
        fixture.agent.hybrid_compaction_stats.applied,
        applied_before
    );
    assert_eq!(fixture.agent.hybrid_compaction_stats.attempts, 0);
    assert_eq!(fixture.agent.run_stats().compactions, compactions_before);
    // Not folded: the first prompt and the last filler turn are both still there.
    let transcript = provider_transcript(&fixture.agent);
    assert!(transcript.contains("Run the script."));
    assert!(transcript.contains("Continue with step 59."));
    assert!(!transcript.contains(SCRIPT_MARK));
    assert_tool_pairs(&fixture.agent.messages_for_provider());
    log_ab("all-off", &fixture.agent, executions, &fixture.journal, 0);
}

/// One script turn, then plain turns until Hybrid has folded it away.
fn folded_script_fixture() -> HybridCodemodeFixture {
    let mut fixture = hybrid_codemode_fixture(vec![edit_input(
        "codemode baseline sentinel",
        "edited by script",
    )]);
    run_script_turn(&mut fixture.agent, "script-call-1");
    fold_until_applied(&mut fixture.agent);
    assert!(!provider_transcript(&fixture.agent).contains(SCRIPT_MARK));
    fixture
}

fn view_log_uri(agent: &Agent) -> String {
    evidence_uris(&provider_transcript(agent))
        .into_iter()
        .find(|uri| uri.starts_with(crate::evidence::SCRIPT_LOG_URI))
        .expect("the provider view names the script log")
}

fn retrieve_uri(
    fixture: &mut HybridCodemodeFixture,
    id: &str,
    uri: &str,
    extra: Value,
) -> (bool, String) {
    let mut input = json!({"sourceRef": uri, "offset": 0, "limit": 120});
    for (key, value) in extra.as_object().into_iter().flatten() {
        input[key] = value.clone();
    }
    retrieve_context_call(&mut fixture.agent, fixture.workspace.path(), id, input)
}

fn stored_log_names(fixture: &HybridCodemodeFixture) -> Vec<String> {
    std::fs::read_dir(fixture.evidence.path())
        .unwrap()
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .filter(|name| name.starts_with("script-log-"))
        .collect()
}

#[test]
fn hybrid_model_surface_recovers_script_evidence() {
    let mut fixture = hybrid_codemode_fixture(vec![edit_input(
        "codemode baseline sentinel",
        "edited by script",
    )]);
    run_script_turn(&mut fixture.agent, "script-call-1");
    // The delivered result never carries the reference; only the ledger does.
    let original = tool_result_text(&fixture.agent, "script-call-1");
    assert!(!original.contains(crate::evidence::SCRIPT_LOG_URI));
    assert!(!original.contains("scriptEvidence"));
    fold_until_applied(&mut fixture.agent);
    let uri = view_log_uri(&fixture.agent);
    let before = fixture.agent.runtime.as_ref().unwrap().context_vm.metrics();

    // Paging: the 150-line output spans pages, and the continuation reaches
    // every line exactly once.
    let mut collected = String::new();
    let mut offset = 0;
    let mut pages = 0;
    loop {
        let (is_error, text) = retrieve_uri(
            &mut fixture,
            &format!("page-{pages}"),
            &uri,
            json!({"offset": offset, "limit": 100}),
        );
        assert!(!is_error);
        let page: Value = serde_json::from_str(&text).unwrap();
        // Bounded: one page stays well under the per-page byte bound.
        assert!(text.len() < 40 * 1024);
        collected.push_str(page["content"].as_str().unwrap());
        collected.push('\n');
        pages += 1;
        match page["next_offset"].as_u64() {
            Some(next) => offset = next as usize,
            None => break,
        }
        assert!(pages < 10);
    }
    assert!(pages >= 2);
    for line in script_output(1).lines() {
        assert_eq!(collected.matches(line).count(), 1, "{line}");
    }
    // Every page is a page fault in the existing accounting.
    let after = fixture.agent.runtime.as_ref().unwrap().context_vm.metrics();
    assert_eq!(after.page_faults, before.page_faults + pages as u64);
    assert_eq!(after.page_fault_hits, before.page_fault_hits + pages as u64);

    // The ledger section is a handful of short lines.
    let view = provider_transcript(&fixture.agent);
    let section = view
        .split("## Script evidence")
        .nth(1)
        .expect("the ledger lists script evidence");
    let lines = section
        .lines()
        .skip(1)
        .take_while(|line| line.starts_with("- "));
    assert_eq!(lines.clone().count(), 1);
    assert!(lines.map(str::len).sum::<usize>() < 256);
}

#[test]
fn hybrid_script_evidence_refuses_unknown_and_foreign_references() {
    let mut fixture = folded_script_fixture();
    let own = view_log_uri(&fixture.agent);
    let other = folded_script_fixture();
    let foreign = view_log_uri(&other.agent);
    assert_ne!(own, foreign);
    for (id, uri) in [
        ("foreign", foreign.as_str()),
        ("guess", "script-log://script-log-000000000000"),
        ("traversal", "script-log://../../secret"),
        ("absolute", "script-log://C:/Windows/win"),
        (
            "output",
            "script-output://s000000000000-script-output-000000000000",
        ),
    ] {
        let (is_error, text) = retrieve_uri(&mut fixture, id, uri, json!({}));
        assert!(is_error, "{id}");
        assert!(text.contains("unknown context source_ref"), "{id}: {text}");
    }
    // The foreign fixture's own file exists and was not readable here.
    assert_eq!(stored_log_names(&other).len(), 1);
}

#[test]
fn hybrid_script_evidence_stops_when_permission_is_revoked() {
    let mut fixture = folded_script_fixture();
    let uri = view_log_uri(&fixture.agent);
    let (is_error, _) = retrieve_uri(&mut fixture, "allowed", &uri, json!({}));
    assert!(!is_error);
    fixture
        .agent
        .permissions
        .lock()
        .unwrap()
        .deny
        .push(crate::PermissionRule::bare("read"));
    let (is_error, text) = retrieve_uri(&mut fixture, "revoked", &uri, json!({}));
    assert!(is_error);
    assert!(text.contains("no longer allow reading"), "{text}");
    assert!(!text.contains(SCRIPT_MARK));
}

#[test]
fn hybrid_script_evidence_missing_file_is_reported_not_empty() {
    let mut fixture = folded_script_fixture();
    let uri = view_log_uri(&fixture.agent);
    for name in stored_log_names(&fixture) {
        std::fs::remove_file(fixture.evidence.path().join(name)).unwrap();
    }
    let (is_error, text) = retrieve_uri(&mut fixture, "missing", &uri, json!({}));
    assert!(is_error);
    assert!(text.contains("script evidence unavailable"), "{text}");
}

#[test]
fn hybrid_script_evidence_suppressed_by_a_hook_is_not_recoverable() {
    let mut fixture = hybrid_codemode_fixture(vec![edit_input(
        "codemode baseline sentinel",
        "edited by script",
    )]);
    fixture.agent.post_tool = Some(crate::PostToolHook(Arc::new(
        |_, _, name, _, mut result| {
            if name == "codemode" {
                result.content = "[withheld by policy]".into();
            }
            result
        },
    )));
    run_script_turn(&mut fixture.agent, "script-call-1");
    fold_until_applied(&mut fixture.agent);
    // The full log was stored before the hook ran, and is still on disk.
    let names = stored_log_names(&fixture);
    assert_eq!(names.len(), 1);
    assert!(evidence_uris(&provider_transcript(&fixture.agent)).is_empty());
    let stem = names[0].trim_end_matches(".jsonl");
    let uri = format!("{}{stem}", crate::evidence::SCRIPT_LOG_URI);
    let (is_error, text) = retrieve_uri(&mut fixture, "suppressed", &uri, json!({}));
    assert!(is_error);
    assert!(text.contains("unknown context source_ref"), "{text}");
    assert!(!text.contains(SCRIPT_MARK));
}

#[test]
fn hybrid_second_fold_and_reload_keep_every_script_reference() {
    let mut fixture = hybrid_codemode_fixture(vec![
        edit_input("codemode baseline sentinel", "edited by script"),
        edit_input("edited by script", "edited again"),
    ]);
    run_script_turn(&mut fixture.agent, "script-call-1");
    fold_until_applied(&mut fixture.agent);
    run_script_turn(&mut fixture.agent, "script-call-2");
    fold_until_applied(&mut fixture.agent);
    let uris = evidence_uris(&provider_transcript(&fixture.agent));
    assert_eq!(uris.len(), 2, "both references survive two folds");
    for (index, uri) in uris.iter().enumerate() {
        let (is_error, text) = retrieve_uri(
            &mut fixture,
            &format!("r{index}"),
            uri,
            json!({"query": "run"}),
        );
        assert!(!is_error);
        assert!(text.contains(SCRIPT_MARK));
    }
    // Reload: the references come from the session file alone.
    let path = fixture.agent.session.as_ref().unwrap().path.clone();
    let reopened = davinci_session::JsonlSession::open(&path).unwrap();
    let events = crate::runtime::context_vm::events_from_session_branch(
        &reopened.entries,
        reopened.leaf_id.as_deref(),
    );
    let mut reloaded: Vec<String> = events
        .iter()
        .flat_map(|event| {
            event
                .artifact_refs
                .iter()
                .map(|artifact| artifact.uri.clone())
        })
        .collect();
    reloaded.sort();
    let mut expected = uris.clone();
    expected.sort();
    assert_eq!(reloaded, expected);
}
