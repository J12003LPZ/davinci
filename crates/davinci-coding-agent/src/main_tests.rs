
#[test]
fn contextual_dispatch_attachments_require_context_and_live_cancellation() {
    for shared in [false, true] {
        let mut agent = davinci_agent::Agent::new_builtin(davinci_agent::PromptProfile::Stable);
        let host = super::ExtensionHost::default();
        if shared {
            super::attach_shared_tool_executor(
                &mut agent,
                std::sync::Arc::new(std::sync::Mutex::new(host)),
            );
        } else {
            super::attach_tool_executor(&mut agent, &host);
        }
        let executor = agent.custom_tool_executor.as_ref().unwrap();
        let cwd = std::path::Path::new(".");
        let args = serde_json::json!({});
        assert!(
            matches!(executor.execute(cwd, "unregistered_extension", &args),
                Err(davinci_agent::ToolError::Failed(message))
                if message == "tool requires engine dispatch context")
        );
        let abort = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        let context = davinci_agent::ToolContext {
            abort: Some(abort.clone()),
            ..Default::default()
        };
        assert!(
            matches!(executor.execute_with_context(cwd, "unregistered_extension", &args, &context),
                Err(davinci_agent::ToolError::Failed(message))
                if message == "tool request cancelled")
        );
        abort.store(false, std::sync::atomic::Ordering::SeqCst);
        assert!(matches!(
            executor.execute_with_context(cwd, "unregistered_extension", &args, &context),
            Err(davinci_agent::ToolError::Unknown(_))
        ));
    }
}
use super::*;

static PROCESS_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn main_model_turn_setting_overrides_the_default_and_keeps_zero() {
    let mut agent = Agent::new("main turn limit fixture");
    super::apply_max_model_turns(&mut agent, &super::settings::Settings::default());
    assert_eq!(agent.max_model_turns, Some(200));

    let settings = super::settings::Settings {
        max_model_turns: Some(0),
        ..super::settings::Settings::default()
    };
    super::apply_max_model_turns(&mut agent, &settings);
    assert_eq!(agent.max_model_turns, Some(0));
}

#[test]
fn worker_agents_use_the_lower_model_turn_default() {
    let worker = super::new_worker_agent("worker turn limit fixture");
    assert_eq!(worker.max_model_turns, Some(60));
}

#[test]
fn provider_context_accounting_includes_runtime_identity() {
    let mut agent = Agent::new("base");
    agent.provider = "openai".into();
    agent.model_id = "gpt-test".into();
    let provider_system = "base\n\nYou are running as openai/gpt-test (thinking: off).".to_string();
    synchronize_provider_system_prompt(&mut agent);
    let schema_tokens =
        (serde_json::to_vec(&provider_tools(&agent)).unwrap().len() as u64).div_ceil(4);
    let expected = (provider_system.len() as u64).div_ceil(4) + schema_tokens;

    assert_eq!(agent.estimated_context_tokens(), expected);
}

#[test]
fn provider_context_includes_duplicate_repository_instructions_once() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = Agent::new("base");
    agent.provider = "openai".into();
    agent.model_id = "gpt-test".into();
    agent.context_files = vec![
        davinci_agent::ContextFile {
            path: dir.path().join("AGENTS.md"),
            name: "AGENTS.md".into(),
            body: "DISTINCTIVE_REPOSITORY_CONSTRAINT".into(),
        },
        davinci_agent::ContextFile {
            path: dir.path().join("CLAUDE.md"),
            name: "CLAUDE.md".into(),
            body: "DISTINCTIVE_REPOSITORY_CONSTRAINT".into(),
        },
    ];

    synchronize_provider_system_prompt(&mut agent);
    let prompt = agent.provider_system_prompt();

    assert_eq!(
        prompt.matches("DISTINCTIVE_REPOSITORY_CONSTRAINT").count(),
        1
    );
    assert!(prompt.contains("AGENTS.md"), "{prompt}");
    assert!(prompt.contains("CLAUDE.md"), "{prompt}");
}

#[test]
fn provider_context_tracks_turn_prompt_updates() {
    let mut agent = Agent::new("initial prompt");
    agent.provider = "openai".into();
    agent.model_id = "gpt-test".into();
    synchronize_provider_system_prompt(&mut agent);

    agent.system_prompt = "updated turn prompt".into();

    let prompt = agent.provider_system_prompt();
    assert!(prompt.starts_with("updated turn prompt"), "{prompt}");
    assert!(prompt.contains("openai/gpt-test"), "{prompt}");
}

struct EnvRestore {
    key: &'static str,
    previous: Option<std::ffi::OsString>,
}

impl EnvRestore {
    fn set(key: &'static str, value: &str) -> Self {
        let previous = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, previous }
    }
}

impl Drop for EnvRestore {
    fn drop(&mut self) {
        match self.previous.take() {
            Some(value) => std::env::set_var(self.key, value),
            None => std::env::remove_var(self.key),
        }
    }
}

#[test]
fn f03_graph_session_context_binds_current_parent_runtime() {
    let _env_lock = PROCESS_ENV_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let _config = EnvRestore::set("PI_CODING_AGENT_DIR", &dir.path().to_string_lossy());
    let _current = EnvRestore::set("DAVINCI_CODING_AGENT_DIR", &dir.path().to_string_lossy());
    let mut agent = Agent::new("fixture");
    agent.cwd = dir.path().to_path_buf();
    agent
        .load_from_session(JsonlSession::create(dir.path(), "fixture", None).unwrap())
        .unwrap();
    let host = ExtensionHost::default();
    apply_graph_session_context(&Args::default(), &agent, &host);
    let first_run = agent.runtime_for_session().unwrap().run_id;
    assert_eq!(
        host.native
            .lock()
            .unwrap()
            .graph
            .runtime
            .as_ref()
            .map(|rt| rt.run_id),
        Some(first_run)
    );
    agent
        .load_from_session(JsonlSession::create(dir.path(), "other", None).unwrap())
        .unwrap();
    apply_graph_session_context(&Args::default(), &agent, &host);
    let next_run = agent.runtime_for_session().unwrap().run_id;
    assert_ne!(next_run, first_run);
    assert_eq!(
        host.native
            .lock()
            .unwrap()
            .graph
            .runtime
            .as_ref()
            .map(|rt| rt.run_id),
        Some(next_run)
    );
}

#[test]
fn f05_graph_session_context_carries_active_task_contract() {
    let _env_lock = PROCESS_ENV_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let _config = EnvRestore::set("PI_CODING_AGENT_DIR", &dir.path().to_string_lossy());
    let _current = EnvRestore::set("DAVINCI_CODING_AGENT_DIR", &dir.path().to_string_lossy());
    let mut agent = Agent::new("fixture");
    agent.cwd = dir.path().to_path_buf();
    agent
        .load_from_session(JsonlSession::create(dir.path(), "fixture", None).unwrap())
        .unwrap();
    let contract = davinci_agent::runtime::TaskContract::new(
        "graph-session-contract",
        1,
        davinci_agent::TaskId::new(),
        1,
        vec!["crates/".into()],
        vec![".git/".into()],
        false,
        vec![],
        vec![],
        vec!["target/".into()],
    )
    .unwrap();
    let digest = contract.digest.clone();
    agent.set_active_contract(contract);
    let host = ExtensionHost::default();

    apply_graph_session_context(&Args::default(), &agent, &host);

    assert_eq!(
        host.native
            .lock()
            .unwrap()
            .graph
            .task_contract
            .as_ref()
            .map(|contract| contract.digest.as_str()),
        Some(digest.as_str())
    );
}

#[test]
fn f05_scope_expansion_report_preempts_permission_fallback_for_print_and_rpc() {
    let mut agent = Agent::new("fixture");
    let contract = davinci_agent::runtime::TaskContract::new(
        "scope-report",
        6,
        davinci_agent::TaskId::new(),
        2,
        vec!["src/".into()],
        vec![],
        false,
        vec![],
        vec![],
        vec![],
    )
    .unwrap();
    agent.set_active_contract(contract);
    let violation = davinci_agent::runtime::contracts::ScopeViolation {
        requested_target: "migrations/001.sql".into(),
        tool: "write".into(),
        writable_scope: vec!["src/".into()],
        protected_scope: vec![],
        reason: "target is outside writable scope".into(),
    };
    let events = vec![AgentEvent::ToolExecutionEnd {
        tool_call_id: "scope-call".into(),
        tool_name: "write".into(),
        result: serde_json::Value::Null,
        is_error: true,
        details: Some(serde_json::json!({"scope_violation": violation})),
    }];
    let generic = serde_json::json!({"type":"approval_required"});

    let report = blocking_host_report(&agent, &events, Some(generic)).unwrap();
    assert_eq!(report["type"], "scope_expansion_required");
    assert_eq!(report["preview"]["expected_revision"], 6);
    assert_eq!(report["preview"]["requested_target"], "migrations/001.sql");
    assert!(report["guidance"]
        .as_str()
        .unwrap()
        .contains("cannot be expanded by the model"));

    let preview = scope_expansion_preview_from_events(&agent, &events).unwrap();
    let (rpc_report, rpc_response) = rpc_scope_expansion_result(Some("rpc-scope".into()), &preview);
    assert_eq!(
        rpc_report["preview"]["preview_digest"],
        preview.preview_digest
    );
    assert_eq!(rpc_response.id.as_deref(), Some("rpc-scope"));
    assert!(!rpc_response.success);
    assert_eq!(
        rpc_response.error.as_deref(),
        Some("scope_expansion_required")
    );
    let encoded = serde_json::to_string(&rpc_response).unwrap();
    assert!(!encoded.contains("allow once"));
    assert!(!encoded.contains("always allow"));
    assert!(!encoded.contains("approved"));
}

#[test]
fn graph_transaction_owner_survives_multiple_prompt_turns() {
    let _env_lock = PROCESS_ENV_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let _config = EnvRestore::set("PI_CODING_AGENT_DIR", &dir.path().to_string_lossy());
    let _current = EnvRestore::set("DAVINCI_CODING_AGENT_DIR", &dir.path().to_string_lossy());
    let _tool = EnvRestore::set(
        "PI_OFFLINE_TOOL_CALL",
        r#"{"name":"write","arguments":{"path":"a.txt","content":"worker edit"}}"#,
    );
    let mut agent = Agent::new("offline Graph transaction fixture");
    agent.cwd = dir.path().to_path_buf();
    agent.tools = vec!["write".into(), "patch_rollback".into()];
    agent.tool_registry = agent.tools.clone();
    agent.set_permission_mode(davinci_agent::PermissionMode::AlwaysApprove);
    agent.tool_context.transaction_owner.graph_node = Some("writer-1".into());
    let owner = agent.tool_context.transaction_owner.agent_id;
    let parsed = Args {
        offline: true,
        no_extensions: true,
        ..Args::default()
    };
    let host = Arc::new(Mutex::new(ExtensionHost::default()));
    agent.prompt("write the file");
    let (_, first) = complete_prompt_with_host(&parsed, &mut agent, Some(host.clone()), false);
    assert_eq!(
        std::fs::read(dir.path().join("a.txt")).unwrap(),
        b"worker edit"
    );
    assert_eq!(agent.runtime.as_ref().unwrap().agent_id, owner);
    let id = first
        .iter()
        .find_map(|event| match event {
            AgentEvent::ToolExecutionEnd {
                is_error: false,
                details: Some(details),
                ..
            } => details["transaction"]["id"].as_str().map(str::to_owned),
            _ => None,
        })
        .expect("successful transaction");
    let _rollback = EnvRestore::set(
        "PI_OFFLINE_TOOL_CALL",
        &serde_json::json!({
            "name":"patch_rollback", "arguments":{"id":id,"paths":["a.txt"]}
        })
        .to_string(),
    );
    agent.prompt("undo the file");
    let (_, second) = complete_prompt_with_host(&parsed, &mut agent, Some(host), false);
    assert_eq!(agent.runtime.as_ref().unwrap().agent_id, owner);
    assert!(second.iter().any(|event| matches!(event,
            AgentEvent::ToolExecutionEnd { tool_name, is_error: false, .. } if tool_name == "patch_rollback")), "{second:?}");
    assert!(!dir.path().join("a.txt").exists());
}

#[test]
fn f03_sessionless_worker_prompt_uses_parent_coordinator() {
    let _env_lock = PROCESS_ENV_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let _config = EnvRestore::set("PI_CODING_AGENT_DIR", &dir.path().to_string_lossy());
    let _current = EnvRestore::set("DAVINCI_CODING_AGENT_DIR", &dir.path().to_string_lossy());
    let _teams = EnvRestore::set("DAVINCI_EXPERIMENTAL_AGENT_TEAMS", "1");
    let _tool = EnvRestore::set("PI_OFFLINE_TOOL_CALL", &serde_json::json!({"name":"task_create", "arguments":{"title":"shared worker task", "operation_id":uuid::Uuid::new_v4()}}).to_string());
    let mut parent = Agent::new("fixture");
    parent
        .load_from_session(JsonlSession::create(dir.path(), "fixture", None).unwrap())
        .unwrap();
    let runtime = parent.runtime_for_session().unwrap().clone();
    let run = runtime.run_id;
    let parent_id = runtime.agent_id;
    let tasks = runtime.task_registry.clone();
    let child_id = davinci_agent::AgentId::new();
    runtime
        .registry
        .register_agent(davinci_agent::AgentRecord {
            id: child_id,
            run_id: run,
            parent: Some(parent_id),
            kind: davinci_agent::AgentKind::Subagent,
            name: "fixture worker".into(),
            provider: "fixture".into(),
            model_id: "fixture".into(),
            cwd: dir.path().to_path_buf(),
            state: davinci_agent::AgentState::Running,
            task_id: None,
            worktree: None,
            started_ms: 0,
            updated_ms: 0,
            failure_reason: None,
        })
        .unwrap();
    let mut child_runtime = runtime;
    child_runtime.agent_id = child_id;
    child_runtime.parent_agent_id = Some(parent_id);
    let mut child = Agent::new("fixture");
    child.cwd = dir.path().to_path_buf();
    child.tools = vec!["task_create".into()];
    child.tool_registry = child.tools.clone();
    child.set_permission_mode(davinci_agent::PermissionMode::AlwaysApprove);
    child.set_runtime(child_runtime);
    child.prompt("create task");
    let (_, events) = complete_prompt_with_host(
        &Args {
            offline: true,
            no_extensions: true,
            ..Args::default()
        },
        &mut child,
        Some(Arc::new(Mutex::new(ExtensionHost::default()))),
        false,
    );
    assert!(
        events.iter().any(|event| matches!(
            event,
            AgentEvent::ToolExecutionEnd {
                is_error: false,
                ..
            }
        )),
        "{events:?}"
    );
    assert_eq!(child.runtime.as_ref().unwrap().agent_id, child_id);
    assert_eq!(child.runtime.as_ref().unwrap().run_id, run);
    assert!(child.session.is_none());
    assert_eq!(tasks.list_tasks(Some(run)).len(), 1);
    let _next_tool = EnvRestore::set("PI_OFFLINE_TOOL_CALL", &serde_json::json!({
            "name":"task_create", "arguments":{"title":"nested worker task", "operation_id":uuid::Uuid::new_v4()}
        }).to_string());
    run_nested_subagent(
        &Args {
            offline: true,
            no_extensions: true,
            ..Args::default()
        },
        dir.path(),
        &davinci_agent::McpRegistry::default(),
        &davinci_agent::SubagentRequest {
            prompt: "create another task".into(),
            tools: vec!["task_create".into()],
            runtime_agent_id: Some(child_id),
            runtime: child.runtime.clone(),
            worktree_path: Some(dir.path().to_path_buf()),
            ..Default::default()
        },
    )
    .unwrap();
    // The nested worker's edits policy still requires approval for task
    // mutations; sharing the coordinator must not bypass that boundary.
    assert_eq!(tasks.list_tasks(Some(run)).len(), 1);
    let unbound = run_nested_subagent(
        &Args {
            offline: true,
            no_extensions: true,
            ..Args::default()
        },
        dir.path(),
        &davinci_agent::McpRegistry::default(),
        &davinci_agent::SubagentRequest {
            prompt: "create task".into(),
            tools: vec!["task_create".into()],
            ..Default::default()
        },
    );
    assert_eq!(
        unbound.unwrap_err(),
        "worker task tools require a parent coordinator"
    );
}

#[test]
fn session_persistence_failure_reaches_host_reply_and_json_event() {
    let _env_lock = PROCESS_ENV_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let _config = EnvRestore::set("PI_CODING_AGENT_DIR", &dir.path().to_string_lossy());
    let _current = EnvRestore::set("DAVINCI_CODING_AGENT_DIR", &dir.path().to_string_lossy());
    let mut agent = Agent::new("fixture");
    agent.cwd = dir.path().to_path_buf();
    let session = JsonlSession::create(dir.path(), dir.path().to_str().unwrap(), None).unwrap();
    let path = session.path.clone();
    agent.load_from_session(session).unwrap();
    agent.record_assistant("previous success must not hide a failed turn");
    std::fs::rename(&path, path.with_extension("backup")).unwrap();
    std::fs::create_dir(&path).unwrap();
    agent.prompt("lost prompt");
    std::fs::remove_dir(&path).unwrap();
    std::fs::rename(path.with_extension("backup"), &path).unwrap();
    let (reply, events) = complete_prompt_with_host(
        &Args {
            offline: true,
            no_extensions: true,
            ..Args::default()
        },
        &mut agent,
        Some(Arc::new(Mutex::new(ExtensionHost::default()))),
        false,
    );
    assert!(reply.starts_with("Session recovery required:"), "{reply}");
    assert_eq!(print_text_exit(&events), (1, Some(reply.clone())));
    let event = to_json_print_event(events.last().unwrap()).unwrap();
    let mut state = native_extensions::graph::WorkerEventState::default();
    native_extensions::graph::parse_worker_event(&event.to_string(), &mut state, |_| {});
    assert_eq!(state.error_message.as_deref(), Some(reply.as_str()));
    assert_eq!(state.stop_reason.as_deref(), Some("session_persistence"));
}

#[test]
fn host_worker_session_preserves_parent_authority_across_turns() {
    let _env_lock = PROCESS_ENV_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let _config = EnvRestore::set("PI_CODING_AGENT_DIR", &dir.path().to_string_lossy());
    let _current = EnvRestore::set("DAVINCI_CODING_AGENT_DIR", &dir.path().to_string_lossy());
    let run = davinci_agent::RunId::new();
    let child = davinci_agent::AgentId::new();
    let parent = davinci_agent::AgentId::new();
    let mut agent = Agent::new("fixture").with_runtime(
        davinci_agent::RuntimeHandle::new(run, child, davinci_agent::RuntimeBus::new())
            .with_parent(parent),
    );
    agent.cwd = dir.path().to_path_buf();
    agent
        .load_from_session(JsonlSession::create(dir.path(), "worker", None).unwrap())
        .unwrap();
    let path = agent.session.as_ref().unwrap().path.clone();
    let parsed = Args {
        offline: true,
        no_extensions: true,
        ..Args::default()
    };
    for prompt in ["first", "second"] {
        agent.prompt(prompt);
        let (reply, _) = complete_prompt_with_host(
            &parsed,
            &mut agent,
            Some(Arc::new(Mutex::new(ExtensionHost::default()))),
            false,
        );
        assert!(!reply.contains("recovery required"), "{reply}");
        let runtime = agent.runtime_for_session().unwrap();
        assert_eq!(runtime.run_id, run);
        assert_eq!(runtime.agent_id, child);
        assert_eq!(runtime.parent_agent_id, Some(parent));
        assert!(!path.with_extension("tasks.jsonl").exists());
    }
    let events = davinci_session::read_runtime_log::<davinci_agent::RuntimeEventEnvelope>(
        &davinci_session::runtime_log_path(&path),
    )
    .unwrap();
    assert!(!events.is_empty());
    assert!(events
        .iter()
        .all(|event| event.parent_agent_id == Some(parent)));
    assert!(events
        .windows(2)
        .all(|pair| pair[0].sequence < pair[1].sequence));
}

#[test]
fn graph_worker_launch_opens_its_private_bound_conversation() {
    use crate::native_extensions::graph::{worker_session_fixtures as fixture, SESSION_ENV};
    let _env_lock = PROCESS_ENV_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let (spec, binding) = fixture::fixture(dir.path());
    let _config = EnvRestore::set(
        "PI_CODING_AGENT_DIR",
        &dir.path().join("config").to_string_lossy(),
    );
    let _current = EnvRestore::set(
        "DAVINCI_CODING_AGENT_DIR",
        &dir.path().join("config").to_string_lossy(),
    );
    let _role = EnvRestore::set("PI_GRAPH_ROLE", spec.role.as_str());
    let _node = EnvRestore::set("PI_GRAPH_NODE_ID", &spec.task_id);
    let _expect = EnvRestore::set("PI_GRAPH_EXPECT", spec.expect.as_str());
    let _artifact = EnvRestore::set(
        "PI_GRAPH_ARTIFACT_PATH",
        &spec.artifact_path.to_string_lossy(),
    );
    let _tools = EnvRestore::set(
        "PI_GRAPH_AUTHORIZED_TOOLS",
        &spec.authorized_tools.join(","),
    );
    let _agent = EnvRestore::set("DAVINCI_AGENT_ID", &binding.agent.to_string());
    let _binding = EnvRestore::set(SESSION_ENV, &serde_json::to_string(&binding).unwrap());
    let mut args = fixture::launch_args(&spec, dir.path());
    args.offline = true;
    let session_dir = binding.session_path.parent().unwrap();
    let mut agent = build_agent(&args, session_dir, dir.path()).unwrap();
    let runtime = agent.runtime_for_session().unwrap();
    assert_eq!(runtime.agent_id, binding.agent);
    assert_eq!(runtime.parent_agent_id, Some(binding.parent));
    assert_eq!(runtime.run_id, binding.runtime_run);
    assert_eq!(agent.session.as_ref().unwrap().path, binding.session_path);
    agent.prompt("retained worker prompt");
    agent.record_assistant("retained worker answer");
    assert!(build_agent(&args, session_dir, dir.path()).is_err());
    drop(agent);
    let recovered = build_agent(&args, session_dir, dir.path()).unwrap();
    assert_eq!(
        recovered.last_assistant_text().as_deref(),
        Some("retained worker answer")
    );
    assert!(!binding.session_path.with_extension("tasks.jsonl").exists());
    drop(recovered);
    args.no_session = true;
    assert!(build_agent(&args, session_dir, dir.path()).is_err());
}

#[test]
fn f03_post_turn_session_failure_reaches_reply_and_event_sink() {
    let _env_lock = PROCESS_ENV_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    for missing in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let _config = EnvRestore::set("PI_CODING_AGENT_DIR", &dir.path().to_string_lossy());
        let _current_config =
            EnvRestore::set("DAVINCI_CODING_AGENT_DIR", &dir.path().to_string_lossy());
        let mut agent = Agent::new("fixture");
        agent.cwd = dir.path().to_path_buf();
        let first = JsonlSession::create(dir.path(), "fixture", None).unwrap();
        let first_path = first.path.clone();
        agent.load_from_session(first).unwrap();
        agent.prompt("fixture prompt");
        let next = JsonlSession::create(dir.path(), "fixture", None).unwrap();
        std::fs::write(
            davinci_session::runtime_log_path(&next.path),
            b"{bad}\n{bad}\n",
        )
        .unwrap();
        let host = Arc::new(Mutex::new(ExtensionHost::default()));
        host.lock().unwrap().session_calls.push(serde_json::json!({
            "op": "switchSession", "sessionPath": if missing { dir.path().join("missing.jsonl") } else { next.path }
        }));
        host.lock().unwrap().session_calls.push(serde_json::json!({
            "op": "sendUserMessage", "text": "must not enter the old session"
        }));
        let observed = Arc::new(Mutex::new(Vec::new()));
        let sink = observed.clone();
        agent.event_sink = Some(EventSink(Arc::new(move |event| {
            sink.lock().unwrap().push(event.clone())
        })));
        let (reply, events) = complete_prompt_with_host(
            &Args {
                offline: true,
                no_extensions: true,
                ..Args::default()
            },
            &mut agent,
            Some(host),
            false,
        );
        assert!(reply.starts_with("Runtime recovery required:"), "{reply}");
        assert_eq!(agent.session.as_ref().unwrap().path, first_path);
        assert!(!serde_json::to_string(&agent.messages)
            .unwrap()
            .contains("must not enter the old session"));
        for events in [&events, &*observed.lock().unwrap()] {
            assert_eq!(
                events
                    .iter()
                    .filter(|event| matches!(
                        event,
                        AgentEvent::AgentEnd {
                            will_retry: false,
                            ..
                        }
                    ))
                    .count(),
                1
            );
            let Some(AgentEvent::AgentEnd { messages, .. }) = events.last() else {
                panic!("missing recovery event");
            };
            assert_eq!(content_text(&messages[0].content), reply);
        }
    }
}

#[test]
fn f03_prompt_recovery_failure_preserves_runtime_and_settles_host() {
    let _env_lock = PROCESS_ENV_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let _config = EnvRestore::set("PI_CODING_AGENT_DIR", &dir.path().to_string_lossy());
    let _current_config =
        EnvRestore::set("DAVINCI_CODING_AGENT_DIR", &dir.path().to_string_lossy());
    let mut agent = Agent::new("fixture");
    agent.cwd = dir.path().to_path_buf();
    let runtime = davinci_agent::RuntimeHandle::new(
        davinci_agent::RunId::new(),
        davinci_agent::AgentId::new(),
        davinci_agent::RuntimeBus::new(),
    );
    let original_run = runtime.run_id;
    agent.set_runtime(runtime);
    let session = JsonlSession::create(dir.path(), "fixture", None).unwrap();
    let log = davinci_session::runtime_log_path(&session.path);
    let corrupt = b"{broken record}\n{broken record}\n";
    std::fs::write(&log, corrupt).unwrap();
    agent.session = Some(session);
    let observed = Arc::new(Mutex::new(Vec::new()));
    let sink = observed.clone();
    agent.event_sink = Some(EventSink(Arc::new(move |event| {
        sink.lock().unwrap().push(event.clone());
    })));
    let host = Arc::new(Mutex::new(ExtensionHost::default()));
    let (reply, events) = complete_prompt_with_host(
        &Args {
            offline: true,
            no_extensions: true,
            ..Args::default()
        },
        &mut agent,
        Some(host.clone()),
        false,
    );
    assert!(reply.starts_with("Runtime recovery required:"), "{reply}");
    assert_eq!(agent.runtime.as_ref().unwrap().run_id, original_run);
    assert_eq!(std::fs::read(log).unwrap(), corrupt);
    assert!(matches!(
        events.as_slice(),
        [AgentEvent::AgentEnd {
            will_retry: false,
            ..
        }]
    ));
    assert!(matches!(
        observed.lock().unwrap().as_slice(),
        [AgentEvent::AgentEnd { .. }]
    ));
    let host = host.lock().unwrap();
    assert!(!host.events.iter().any(|event| matches!(
        event,
        ExtensionEvent::BeforeProviderRequest { .. }
            | ExtensionEvent::BeforeProviderHeaders { .. }
            | ExtensionEvent::AfterProviderResponse { .. }
    )));
    assert!(matches!(
        host.events.last(),
        Some(ExtensionEvent::AgentSettled)
    ));
}

#[test]
fn f03_custom_session_id_survives_durable_activation() {
    let dir = tempfile::tempdir().unwrap();
    let args = Args {
        session_id: Some("custom-session-id".into()),
        ..Args::default()
    };
    let session = resolve_or_create_session(&args, dir.path(), dir.path()).unwrap();
    let path = session.path.clone();
    let mut agent = Agent::new("fixture");
    agent.load_from_session(session).unwrap();
    let run = agent.runtime_for_session().unwrap().run_id;
    drop(agent);
    let reopened = JsonlSession::open(&path).unwrap();
    assert_eq!(reopened.header.id, "custom-session-id");
    let mut resumed = Agent::new("fixture");
    resumed.load_from_session(reopened).unwrap();
    assert_eq!(resumed.runtime_for_session().unwrap().run_id, run);
    assert_eq!(
        resolve_or_create_session(&args, dir.path(), dir.path())
            .unwrap()
            .path,
        path
    );
}

#[test]
fn codex_fixture_login_persists_exact_provider_without_aliasing() {
    let _lock = PROCESS_ENV_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let dir = tempfile::tempdir().expect("temp auth dir");
    let legacy = tempfile::tempdir().expect("legacy auth dir");
    let _agent_dir = EnvRestore::set("DAVINCI_CODING_AGENT_DIR", &dir.path().to_string_lossy());
    let _legacy_dir = EnvRestore::set("PI_CODING_AGENT_DIR", &legacy.path().to_string_lossy());
    let _oauth_code = EnvRestore::set("PI_OAUTH_CODE", "pi-fixture-code");

    assert_eq!(
        login_provider_with_wait("openai-codex", None, false),
        Ok(true)
    );

    let storage = AuthStorage::open(&dir.path().join("auth.json")).expect("persisted auth");
    assert!(!legacy.path().join("auth.json").exists());
    let credential = storage
        .get("openai-codex")
        .expect("credential stored under exact provider id");
    assert_eq!(credential.kind, CredentialKind::Oauth);
    assert!(storage.get("openai").is_none());

    assert!(
        davinci_ai::resolve_provider_auth(
            "openai-codex",
            &storage,
            &std::collections::HashMap::new(),
            false,
        )
        .is_none(),
        "plain test fixture token must not bypass Codex account-bound JWT validation"
    );
    let payload = base64::Engine::encode(
        &base64::engine::general_purpose::URL_SAFE_NO_PAD,
        br#"{"https://api.openai.com/auth":{"chatgpt_account_id":"test-account"}}"#,
    );
    let mut storage = AuthStorage::create().unwrap();
    storage
        .login_oauth(
            "openai-codex",
            format!("e30.{payload}.signature"),
            None,
            None,
        )
        .unwrap();
    let parsed = Args {
        no_extensions: true,
        ..Args::default()
    };
    let snapshot = load_model_runtime(&parsed);
    assert!(snapshot
        .available
        .iter()
        .any(|model| model.provider == "openai-codex"));
    let mut agent = Agent::new("fixture");
    apply_resolved_models(&parsed, &mut agent).unwrap();
    assert_eq!(agent.provider, "openai-codex");
    assert!(!agent.model_id.is_empty());
}

#[test]
fn model_runtime_snapshot_is_reused_until_an_input_changes() {
    let _lock = PROCESS_ENV_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let _config = EnvRestore::set("PI_CODING_AGENT_DIR", &dir.path().to_string_lossy());
    let _current = EnvRestore::set("DAVINCI_CODING_AGENT_DIR", &dir.path().to_string_lossy());
    let parsed = Args {
        no_extensions: true,
        ..Args::default()
    };
    clear_model_runtime_cache();
    let first = load_model_runtime(&parsed);
    assert!(!first.stored_providers.contains(&"anthropic".to_string()));

    // Unchanged inputs are served from the cache, not rebuilt: a marker
    // planted in the cached snapshot comes back.
    {
        let key = ModelRuntimeKey::current(&parsed);
        let mut cache = MODEL_RUNTIME_CACHE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let (_, snapshot) = cache
            .iter_mut()
            .find(|(cached, _)| *cached == key)
            .expect("first load fills the cache");
        snapshot
            .composition_errors
            .insert("cache-marker".into(), "reused".into());
    }
    assert!(load_model_runtime(&parsed)
        .composition_errors
        .contains_key("cache-marker"));

    // A login rewrites auth.json, so the next load is rebuilt from disk.
    let mut storage = AuthStorage::create().unwrap();
    storage.login_api_key("anthropic", "sk-test").unwrap();
    let after_login = load_model_runtime(&parsed);
    assert!(!after_login.composition_errors.contains_key("cache-marker"));
    assert!(after_login
        .stored_providers
        .contains(&"anthropic".to_string()));
    // The rebuild replaced the stale entry for the same inputs.
    let login_key = ModelRuntimeKey::current(&parsed);
    assert_eq!(
        MODEL_RUNTIME_CACHE
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .filter(|(cached, _)| cached.same_inputs(&login_key))
            .count(),
        1
    );

    // Environment and argument changes are part of the key too.
    let key = ModelRuntimeKey::current(&parsed);
    {
        let _env = EnvRestore::set("DAVINCI_MODEL_RUNTIME_KEY_PROBE", "1");
        assert_ne!(ModelRuntimeKey::current(&parsed), key);
    }
    let other_model = Args {
        model: Some("openai/gpt-test".into()),
        ..parsed.clone()
    };
    assert_ne!(ModelRuntimeKey::current(&other_model), key);

    clear_model_runtime_cache();
    assert!(!MODEL_RUNTIME_CACHE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .iter()
        .any(|(cached, _)| cached.same_inputs(&login_key)));
}

#[test]
fn the_offline_tool_call_fixture_scripts_one_call_then_the_usual_stub() {
    let _lock = PROCESS_ENV_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    std::env::remove_var("PI_OFFLINE_TOOL_CALL");
    let mut agent = Agent::new_builtin(davinci_agent::PromptProfile::Stable);
    agent.prompt("run git status");

    let plain = offline_stub_message(&agent, 14);
    assert!(
        matches!(plain.content.first(), Some(ContentBlock::Text { text }) if text == "(offline) received 14 characters")
    );
    assert_eq!(plain.stop_reason, Some(StopReason::Stop));

    let _fixture = EnvRestore::set(
        "PI_OFFLINE_TOOL_CALL",
        r#"{"name":"bash","arguments":{"command":"git status --short"}}"#,
    );
    let scripted = offline_stub_message(&agent, 14);
    assert_eq!(scripted.stop_reason, Some(StopReason::ToolUse));
    match scripted.content.first() {
        Some(ContentBlock::ToolCall {
            name, arguments, ..
        }) => {
            assert_eq!(name, "bash");
            assert_eq!(arguments["command"], "git status --short");
        }
        other => panic!("{other:?}"),
    }

    // After the tool answered, the fixture steps aside.
    agent
        .messages
        .push(davinci_ai::ChatMessage::text("toolResult", "ok"));
    let after = offline_stub_message(&agent, 14);
    assert_eq!(after.stop_reason, Some(StopReason::Stop));
    let mut reminder = davinci_ai::ChatMessage::text("user", "verify the edit");
    reminder.extra.insert(
        "davinciCapabilityReminder".into(),
        serde_json::json!("verification_required"),
    );
    agent.messages.push(reminder);
    assert_eq!(
        offline_stub_message(&agent, 15).stop_reason,
        Some(StopReason::Stop)
    );
}

#[test]
fn offline_tool_sequence_waits_for_results_and_stops_on_failure() {
    let _lock = PROCESS_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let _fixture = EnvRestore::set(
        "PI_OFFLINE_TOOL_CALL",
        r#"[{"name":"write","arguments":{}},{"name":"graph_submit","arguments":{}}]"#,
    );
    let mut agent = Agent::new("fixture");
    agent.prompt("edit and submit");
    let call_name = |agent: &Agent| match offline_stub_message(agent, 15).content.first() {
        Some(ContentBlock::ToolCall { name, .. }) => Some(name.clone()),
        _ => None,
    };
    assert_eq!(call_name(&agent).as_deref(), Some("write"));
    agent
        .messages
        .push(davinci_ai::ChatMessage::text("assistant", "waiting"));
    assert_eq!(call_name(&agent), None);
    agent.messages.push(davinci_ai::ChatMessage::tool_result(
        "one", "write", "ok", false,
    ));
    let mut reminder = davinci_ai::ChatMessage::text("user", "verify");
    reminder
        .extra
        .insert("davinciCapabilityReminder".into(), serde_json::json!(true));
    agent.messages.push(reminder);
    assert_eq!(call_name(&agent).as_deref(), Some("graph_submit"));
    agent.messages.push(davinci_ai::ChatMessage::tool_result(
        "two",
        "graph_submit",
        "ok",
        false,
    ));
    assert_eq!(call_name(&agent), None);
    agent.prompt("next real turn");
    assert_eq!(call_name(&agent).as_deref(), Some("write"));
    agent.messages.push(davinci_ai::ChatMessage::tool_result(
        "three", "write", "denied", true,
    ));
    assert_eq!(call_name(&agent), None);
}

#[test]
fn rpc_permission_questions_are_a_select_and_only_listed_answers_allow() {
    use davinci_agent::ToolApprovalDecision::*;
    let request = davinci_agent::ToolApprovalRequest {
        legal_choices: davinci_agent::approval::offer_scopes(false, true, true),
        tool_call_id: "call_1".into(),
        tool: "bash".into(),
        args: serde_json::json!({"command": "git status"}),
        subject: "git status".into(),
        summary: "bash · git status".into(),
        session_rule: "bash(git status *)".into(),
        outside_project: false,
        mode: davinci_agent::PermissionMode::Ask,
    };
    let call = rpc_approval_call(&request, true);
    assert_eq!(call["op"], "select");
    assert_eq!(call["title"], "Allow bash · git status?");
    assert_eq!(
        call["options"],
        serde_json::json!([
            "allow once",
            "allow for this session",
            "always allow in this project",
            "deny"
        ])
    );
    assert_eq!(
        rpc_approval_call(&request, false)["options"],
        serde_json::json!(["allow once", "allow for this session", "deny"])
    );

    let answer = |text: &str| serde_json::Value::String(text.into());
    assert_eq!(
        rpc_approval_decision(&answer("allow once"), true),
        AllowOnce
    );
    assert_eq!(
        rpc_approval_decision(&answer("allow for this session"), true),
        AllowForSession
    );
    assert_eq!(
        rpc_approval_decision(&answer("always allow in this project"), true),
        AllowAlways
    );
    assert_eq!(
        rpc_approval_decision(&answer("always allow in this project"), false),
        Deny
    );
    assert_eq!(rpc_approval_decision(&answer("deny"), true), Deny);
    assert_eq!(rpc_approval_decision(&answer("yes please"), true), Deny);
    assert_eq!(rpc_approval_decision(&serde_json::Value::Null, true), Deny);
}

/// Build offline first, then set DAVINCI_PRINT_TEST_EXECUTABLE to that binary.
#[test]
#[ignore = "requires a freshly built davinci executable via DAVINCI_PRINT_TEST_EXECUTABLE"]
fn f03_print_recovery_failure_exits_and_stops_prompts() {
    use std::process::{Command, Stdio};
    let binary = std::env::var_os("DAVINCI_PRINT_TEST_EXECUTABLE")
        .expect("set DAVINCI_PRINT_TEST_EXECUTABLE to the freshly built product");
    for json in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("config");
        std::fs::create_dir(&config).unwrap();
        let session =
            JsonlSession::create(dir.path(), &dir.path().to_string_lossy(), None).unwrap();
        let log = davinci_session::runtime_log_path(&session.path);
        let corrupt = b"{broken record}\n{broken record}\n";
        std::fs::write(&log, corrupt).unwrap();
        let stdout = dir.path().join("stdout");
        let stderr = dir.path().join("stderr");
        let mut command = Command::new(&binary);
        command
            .current_dir(dir.path())
            .args([
                "--offline",
                "--no-extensions",
                "--no-skills",
                "--no-prompt-templates",
                "--print",
                "first recovery fixture",
                "second recovery fixture",
                "--session",
            ])
            .arg(&session.path)
            .env("PI_CODING_AGENT_DIR", &config)
            .env("DAVINCI_CODING_AGENT_DIR", &config)
            .env("PI_OFFLINE", "1")
            .env("DAVINCI_OFFLINE", "1")
            .env("PI_DISABLE_NETWORK", "1")
            .env("PI_HOOKS_DRY_RUN", "1")
            .env(
                "PI_OFFLINE_TOOL_CALL",
                r#"{"name":"write","arguments":{"path":"must-not-exist.txt","content":"fixture"}}"#,
            )
            .stdin(Stdio::null())
            .stdout(std::fs::File::create(&stdout).unwrap())
            .stderr(std::fs::File::create(&stderr).unwrap());
        if json {
            command.args(["--mode", "json"]);
        }
        let mut child = command.spawn().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("recovery fixture did not terminate");
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        };
        let output = std::fs::read_to_string(if json { stdout } else { stderr }).unwrap();
        assert_eq!(status.code(), Some(1), "{output}");
        assert!(output.contains("Runtime recovery required:"), "{output}");
        assert_eq!(std::fs::read(log).unwrap(), corrupt);
        assert!(!dir.path().join("must-not-exist.txt").exists());
        let saved = std::fs::read_to_string(&session.path).unwrap();
        assert!(saved.contains("first recovery fixture"));
        assert!(!saved.contains("second recovery fixture"));
    }
}

#[test]
#[ignore = "requires a freshly built davinci executable via DAVINCI_PRINT_TEST_EXECUTABLE"]
fn f03_graph_process_rejects_unbound_task_tools() {
    use std::process::{Command, Stdio};
    let binary = std::env::var_os("DAVINCI_PRINT_TEST_EXECUTABLE")
        .expect("set DAVINCI_PRINT_TEST_EXECUTABLE to the freshly built product");
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config");
    std::fs::create_dir(&config).unwrap();
    for tool in ["task_create", "task_update", "task_list", "task_get"] {
        let stdout = dir.path().join(format!("{tool}.stdout"));
        let stderr = dir.path().join(format!("{tool}.stderr"));
        let mut child = Command::new(&binary)
                .current_dir(dir.path())
                .args([
                    "--offline", "--no-session", "--no-extensions", "--no-skills",
                    "--no-prompt-templates", "--permission-mode", "always-approve",
                    "--tools", "task_create,task_update,task_list,task_get",
                    "--mode", "json", "--print", "graph coordinator fixture",
                ])
                .env("PI_CODING_AGENT_DIR", &config)
                .env("DAVINCI_CODING_AGENT_DIR", &config)
                .env("PI_OFFLINE", "1")
                .env("DAVINCI_OFFLINE", "1")
                .env("PI_DISABLE_NETWORK", "1")
                .env("PI_HOOKS_DRY_RUN", "1")
                .env("DAVINCI_EXPERIMENTAL_AGENT_TEAMS", "1")
                .env("PI_GRAPH_ROLE", "writer")
                .env("PI_GRAPH_EXPECT", "patch-report")
                .env("PI_GRAPH_ARTIFACT_PATH", dir.path().join("artifact.json"))
                .env("PI_GRAPH_EXTRA_TOOLS", "task_create,task_update,task_list,task_get")
                .env("PI_OFFLINE_TOOL_CALL", serde_json::json!({
                    "name": tool, "arguments": {"title":"must not exist", "operation_id":uuid::Uuid::new_v4()}
                }).to_string())
                .stdin(Stdio::null())
                .stdout(std::fs::File::create(&stdout).unwrap())
                .stderr(std::fs::File::create(&stderr).unwrap())
                .spawn().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("graph coordinator fixture did not terminate");
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        };
        let output = std::fs::read_to_string(&stdout).unwrap();
        assert!(
            status.success(),
            "{}",
            std::fs::read_to_string(stderr).unwrap()
        );
        let results: Vec<serde_json::Value> = output
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .filter(|event: &serde_json::Value| {
                event["type"] == "tool_execution_end" && event["toolName"] == tool
            })
            .collect();
        assert_eq!(results.len(), 1, "{output}");
        assert_eq!(results[0]["isError"], true, "{output}");
        assert!(
            results[0]
                .to_string()
                .contains("parent coordinator transport"),
            "{output}"
        );
        assert!(!dir.path().join("artifact.json").exists());
    }
}

#[test]
#[ignore = "requires a freshly built davinci executable via DAVINCI_PRINT_TEST_EXECUTABLE"]
fn f03_print_process_resumes_task_journal() {
    use serde_json::json;
    use std::process::{Command, Stdio};
    let binary = std::env::var_os("DAVINCI_PRINT_TEST_EXECUTABLE")
        .expect("set DAVINCI_PRINT_TEST_EXECUTABLE to the freshly built product");
    let dir = tempfile::tempdir().unwrap();
    let config = dir.path().join("config");
    std::fs::create_dir(&config).unwrap();
    let session = JsonlSession::create(dir.path(), &dir.path().to_string_lossy(), None).unwrap();
    let journal = session.path.with_extension("tasks.jsonl");
    let key = serde_json::to_string(&(
        std::fs::canonicalize(&session.path).unwrap(),
        &session.header.id,
    ))
    .unwrap();
    let mut expected = None;
    for tool in ["task_create", "task_list"] {
        let stdout = dir.path().join(format!("{tool}.stdout"));
        let stderr = dir.path().join(format!("{tool}.stderr"));
        let arguments = if tool == "task_create" {
            json!({"title":"persisted print task", "operation_id": uuid::Uuid::new_v4()})
        } else {
            json!({})
        };
        let mut child = Command::new(&binary)
            .current_dir(dir.path())
            .args([
                "--offline",
                "--no-extensions",
                "--no-skills",
                "--no-prompt-templates",
                "--permission-mode",
                "always-approve",
                "--tools",
                "task_create,task_list",
                "--mode",
                "json",
                "--print",
                "task fixture",
                "--session",
            ])
            .arg(&session.path)
            .env("PI_CODING_AGENT_DIR", &config)
            .env("DAVINCI_CODING_AGENT_DIR", &config)
            .env("PI_OFFLINE", "1")
            .env("DAVINCI_OFFLINE", "1")
            .env("PI_DISABLE_NETWORK", "1")
            .env("PI_HOOKS_DRY_RUN", "1")
            .env("DAVINCI_EXPERIMENTAL_AGENT_TEAMS", "1")
            .env(
                "PI_OFFLINE_TOOL_CALL",
                json!({"name":tool,"arguments":arguments}).to_string(),
            )
            .stdin(Stdio::null())
            .stdout(std::fs::File::create(&stdout).unwrap())
            .stderr(std::fs::File::create(&stderr).unwrap())
            .spawn()
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("task journal print fixture did not terminate");
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        };
        let output = std::fs::read_to_string(&stdout).unwrap();
        assert!(
            status.success(),
            "{}",
            std::fs::read_to_string(&stderr).unwrap()
        );
        assert!(output.contains("persisted print task"), "{output}");
        let results: Vec<serde_json::Value> = output
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .filter(|event: &serde_json::Value| {
                event["type"] == "tool_execution_end" && event["toolName"] == tool
            })
            .collect();
        assert_eq!(results.len(), 1, "{output}");
        assert_eq!(results[0]["isError"], false, "{output}");
        assert!(
            results[0].to_string().contains("persisted print task"),
            "{output}"
        );
        let (registry, run) =
            davinci_agent::TaskRegistry::open_session_durable(&journal, &key).unwrap();
        let tasks = registry.list_tasks(Some(run));
        assert_eq!(tasks.len(), 1, "{output}");
        assert_eq!(tasks[0].run_id, run);
        if let Some((saved_run, saved_task)) = &expected {
            assert_eq!(run, *saved_run);
            assert_eq!(&tasks[0], saved_task);
        } else {
            expected = Some((run, tasks[0].clone()));
        }
    }
}

/// Build offline first, then set DAVINCI_PRINT_TEST_EXECUTABLE to that binary.
#[test]
#[ignore = "requires a freshly built davinci executable via DAVINCI_PRINT_TEST_EXECUTABLE"]
fn f01_print_process_output_and_exit() {
    use std::process::{Command, Stdio};
    let binary = std::env::var_os("DAVINCI_PRINT_TEST_EXECUTABLE")
        .expect("set DAVINCI_PRINT_TEST_EXECUTABLE to the freshly built product");
    assert!(Path::new(&binary).is_file());
    for (json, allow, legacy) in [
        (false, false, false),
        (true, false, false),
        (true, true, false),
        (true, false, true),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let config = dir.path().join("agent");
        std::fs::create_dir(&config).unwrap();
        let stdout_path = dir.path().join("stdout.jsonl");
        let stderr_path = dir.path().join("stderr.txt");
        let mut command = Command::new(&binary);
        command
            .current_dir(dir.path())
            .args([
                "--offline",
                "--no-session",
                "--no-extensions",
                "--no-skills",
                "--no-prompt-templates",
                "--permission-mode",
                if allow { "always-approve" } else { "manual" },
                "--print",
                "first fixture prompt",
                "second fixture prompt",
            ])
            .env("PI_CODING_AGENT_DIR", &config)
            .env("PI_OFFLINE", "1")
            .env("DAVINCI_OFFLINE", "1")
            .env("PI_DISABLE_NETWORK", "1")
            .env("PI_HOOKS_DRY_RUN", "1")
            .env(
                "PI_OFFLINE_TOOL_CALL",
                r#"{"name":"write","arguments":{"path":"must-not-exist.txt","content":"fixture"}}"#,
            )
            .stdin(Stdio::null())
            .stdout(std::fs::File::create(&stdout_path).unwrap())
            .stderr(std::fs::File::create(&stderr_path).unwrap());
        if legacy {
            command.env_remove("DAVINCI_CODING_AGENT_DIR");
        } else {
            command.env("DAVINCI_CODING_AGENT_DIR", &config);
        }
        if json {
            command.args(["--mode", "json"]);
        }
        let mut child = command.spawn().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            if std::time::Instant::now() >= deadline {
                let _ = child.kill();
                let _ = child.wait();
                panic!("print fixture did not terminate on EOF");
            }
            std::thread::sleep(std::time::Duration::from_millis(25));
        };
        let stdout = std::fs::read_to_string(stdout_path).unwrap();
        assert_eq!(
            status.code(),
            Some(if allow { 0 } else { 1 }),
            "{}",
            std::fs::read_to_string(stderr_path).unwrap()
        );
        let rows: Vec<serde_json::Value> = stdout
            .lines()
            .filter(|line| !line.is_empty())
            .map(|line| serde_json::from_str(line).expect("every stdout line must be JSON"))
            .collect();
        let required: Vec<_> = rows
            .iter()
            .filter(|row| row["type"] == "approval_required")
            .collect();
        assert_eq!(required.len(), usize::from(!allow));
        assert_eq!(dir.path().join("must-not-exist.txt").exists(), allow);
        assert!(!config.join("settings.json").exists());
        if !allow {
            assert_eq!(required[0]["action"], "write");
            assert!(required[0]["target"]
                .as_str()
                .unwrap()
                .ends_with("must-not-exist.txt"));
            assert_eq!(
                required[0]["configuration_path"],
                config.join("settings.json").to_string_lossy().as_ref()
            );
            assert!(!stdout.contains("second fixture prompt"));
            if !json {
                assert_eq!(rows.len(), 1);
            }
        }
    }
}

#[test]
fn f01_noninteractive_fail_closed() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = Agent::new("offline print approval");
    agent.cwd = dir.path().to_path_buf();
    agent.tools = vec!["write".into()];
    agent.permissions = Arc::new(davinci_agent::PermissionState::new(
        davinci_agent::PermissionPolicy::new(davinci_agent::PermissionMode::Ask),
    ));
    agent.prompt("write the file");
    let path = dir.path().join("agent/settings.json");
    let mut calls = 0;
    let (events, required) = with_print_approval(&mut agent, &path, |agent| {
        agent.run_loop(|_| {
                calls += 1;
                assert_eq!(calls, 1, "unresolved approval must stop further provider calls");
                Ok(AssistantMessage {
                    id: "fixture".into(), role: "assistant".into(),
                    content: vec![ContentBlock::ToolCall {
                        id: "write-1".into(), name: "write".into(),
                        arguments: serde_json::json!({"path": "must-not-exist.txt", "content": "blocked"}),
                    }],
                    model: "fixture".into(), usage: None,
                    stop_reason: Some(StopReason::ToolUse), error_message: None,
                })
            }).unwrap()
    });
    let required = required.expect("print must report unresolved approval");
    assert_eq!(required["type"], "approval_required");
    assert_eq!(required["tool_call_id"], "write-1");
    assert_eq!(required["action"], "write");
    assert!(required["target"]
        .as_str()
        .unwrap_or_default()
        .ends_with("must-not-exist.txt"));
    assert_eq!(
        required["configuration_path"],
        path.to_string_lossy().as_ref()
    );
    assert!(!dir.path().join("must-not-exist.txt").exists());
    assert!(!path.exists());
    assert!(agent.permissions.lock().unwrap().session_allow.is_empty());
    assert!(!agent.abort_requested());
    assert!(agent.approval_responder.is_none());
    assert!(events
        .iter()
        .any(|event| matches!(event, AgentEvent::ToolExecutionEnd { is_error: true, .. })));
    for event in events {
        serde_json::from_str::<serde_json::Value>(
            &serde_json::to_string(&to_json_print_event(&event).unwrap()).unwrap(),
        )
        .unwrap();
    }
}

#[test]
fn f01_rpc_abort_releases_pending_dialog_before_late_approval() {
    let (tx, rx) = std::sync::mpsc::channel();
    tx.send(r#"{"type":"abort","id":"stop"}"#.to_string())
        .unwrap();
    tx.send(r#"{"type":"extension_ui_response","id":"pending","value":"allow once"}"#.to_string())
        .unwrap();
    let leftover = Mutex::new(std::collections::VecDeque::new());
    let rx = Mutex::new(rx);
    let (answer, intercepted) = rpc_wait_ui_response(
        "pending",
        serde_json::Value::Null,
        Some(1000),
        &leftover,
        &rx,
        true,
    );
    assert_eq!(
        answer,
        serde_json::Value::Null,
        "abort must reject the pending dialog before consuming a later approval"
    );
    assert_eq!(intercepted.unwrap().id.as_deref(), Some("stop"));
    assert!(leftover.lock().unwrap().is_empty());
    assert!(rx
        .lock()
        .unwrap()
        .try_recv()
        .unwrap()
        .contains("allow once"));
}

#[test]
fn f01_rpc_dialog_abort_signals_turn_and_is_not_replayed() {
    let abort = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let mut agent = Agent::new("test");
    agent.abort_signal = Some(abort.clone());
    let active = Mutex::new(Some(abort.clone()));
    let (tx, rx) = std::sync::mpsc::channel();
    let before = r#"{"type":"get_state","id":"before"}"#;
    let after = r#"{"type":"prompt","id":"after","message":"next"}"#;
    let leftover = Mutex::new(std::collections::VecDeque::from([
        before.to_string(),
        r#"{"type":"abort","id":"stop"}"#.to_string(),
        after.to_string(),
    ]));
    let rx = Mutex::new(rx);
    let call = serde_json::json!({"op": "confirm", "title": "Allow?", "timeout": 60_000});
    assert_eq!(
        rpc_emit_and_wait_ui(&call, &leftover, &rx, &active),
        serde_json::json!(false)
    );
    assert!(agent.abort_requested());
    agent.prompt("cancelled turn");
    let events = agent
        .run_loop(|_| -> Result<davinci_ai::AssistantMessage, String> {
            panic!("a cancelled RPC turn must not call the provider")
        })
        .unwrap();
    assert!(events
        .iter()
        .any(|event| matches!(event, davinci_agent::AgentEvent::AgentEnd { .. })));
    assert_eq!(
        *leftover.lock().unwrap(),
        [before.to_string(), after.to_string()]
    );
    // An already cancelled turn must not open another dialog or consume queued work.
    assert_eq!(
        rpc_emit_and_wait_ui(&call, &leftover, &rx, &active),
        serde_json::json!(false)
    );
    assert_eq!(
        *leftover.lock().unwrap(),
        [before.to_string(), after.to_string()]
    );
    drop(tx);
}

#[test]
fn f01_rpc_dialog_abort_scope_restores_previous_signal() {
    let mut agent = Agent::new("test");
    let previous = Arc::new(std::sync::atomic::AtomicBool::new(false));
    agent.abort_signal = Some(previous.clone());
    let active = Mutex::new(None);
    for _ in 0..2 {
        rpc_with_ui_abort(&mut agent, &active, |agent| {
            assert!(!agent.abort_requested());
            let signal = active.lock().unwrap().clone().unwrap();
            assert!(Arc::ptr_eq(agent.abort_signal.as_ref().unwrap(), &signal));
            signal.store(true, std::sync::atomic::Ordering::Relaxed);
            assert!(agent.abort_requested());
        });
        assert!(active.lock().unwrap().is_none());
        assert!(Arc::ptr_eq(agent.abort_signal.as_ref().unwrap(), &previous));
        assert!(!agent.abort_requested());
    }
    previous.store(true, std::sync::atomic::Ordering::Relaxed);
    rpc_with_ui_abort(
        &mut agent,
        &active,
        |agent| assert!(agent.abort_requested()),
    );
    assert!(agent.abort_requested());
    assert!(Arc::ptr_eq(agent.abort_signal.as_ref().unwrap(), &previous));
    // Idle extension dialogs retain ordinary command handling outside an active turn.
    let (tx, rx) = std::sync::mpsc::channel();
    let line = r#"{"type":"abort","id":"idle"}"#;
    tx.send(line.to_string()).unwrap();
    drop(tx);
    let leftover = Mutex::new(std::collections::VecDeque::new());
    let (_, intercepted) = rpc_wait_ui_response(
        "pending",
        serde_json::Value::Null,
        None,
        &leftover,
        &Mutex::new(rx),
        false,
    );
    assert!(intercepted.is_none());
    assert_eq!(leftover.lock().unwrap().front().unwrap(), line);
}

#[test]
fn f01_rpc_disconnect_releases_wait_and_preserves_commands() {
    let (tx, rx) = std::sync::mpsc::channel();
    let commands = [
        r#"{"type":"get_state","id":"first"}"#,
        r#"{"type":"get_state","id":"second"}"#,
    ];
    tx.send(commands[1].to_string()).unwrap();
    drop(tx);
    let leftover = Mutex::new(std::collections::VecDeque::from([commands[0].to_string()]));
    let start = std::time::Instant::now();
    assert_eq!(
        rpc_wait_ui_response(
            "pending",
            serde_json::Value::Null,
            Some(1000),
            &leftover,
            &Mutex::new(rx),
            false,
        )
        .0,
        serde_json::Value::Null
    );
    assert!(
        start.elapsed() < std::time::Duration::from_millis(500),
        "disconnect must terminate without waiting for the deadline"
    );
    assert_eq!(*leftover.lock().unwrap(), commands.map(String::from));
}

#[test]
fn f01_rpc_wait_matches_only_current_valid_response() {
    for (line, expected, consumed) in [
        (
            r#"{"type":"extension_ui_response","id":"pending","value":"allow once"}"#,
            serde_json::json!("allow once"),
            true,
        ),
        (
            r#"{"type":"extension_ui_response","id":"pending","cancelled":true,"value":"allow once"}"#,
            serde_json::Value::Null,
            true,
        ),
        (
            r#"{"type":"extension_ui_response","id":"old","value":"allow once"}"#,
            serde_json::Value::Null,
            false,
        ),
        (
            r#"{"type":"extension_ui_response","id":"pending","value":42}"#,
            serde_json::Value::Null,
            false,
        ),
    ] {
        let (tx, rx) = std::sync::mpsc::channel();
        tx.send(line.to_string()).unwrap();
        drop(tx);
        let leftover = Mutex::new(std::collections::VecDeque::new());
        assert_eq!(
            rpc_wait_ui_response(
                "pending",
                serde_json::Value::Null,
                None,
                &leftover,
                &Mutex::new(rx),
                false,
            )
            .0,
            expected
        );
        assert_eq!(leftover.lock().unwrap().is_empty(), consumed);
    }
    let (_tx, rx) = std::sync::mpsc::channel();
    let leftover = Mutex::new(std::collections::VecDeque::from(["queued".to_string()]));
    assert_eq!(
        rpc_wait_ui_response(
            "pending",
            serde_json::json!(false),
            Some(0),
            &leftover,
            &Mutex::new(rx),
            false,
        )
        .0,
        serde_json::json!(false)
    );
    assert_eq!(leftover.lock().unwrap().front().unwrap(), "queued");
}

#[test]
fn f01_rpc_denial_instructions_use_confirmed_bounded_dialogs() {
    use davinci_agent::approval::{ApprovalChallenge, GrantScope};
    for case in [
        "accept",
        "cancel_input",
        "empty",
        "oversize",
        "reject_confirm",
        "malformed_confirm",
        "stale_confirm",
        "unoffered",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let policy = davinci_agent::PermissionState::new(davinci_agent::PermissionPolicy::new(
            davinci_agent::PermissionMode::Ask,
        ));
        let davinci_agent::PermissionVerdict::Ask(request) = policy.lock().unwrap().decide(
            "call",
            "write",
            &serde_json::json!({"path":"ordinary.txt", "content":"fixture"}),
            dir.path(),
        ) else {
            panic!("expected Ask")
        };
        let mut challenge = ApprovalChallenge {
            schema_version: 1,
            id: uuid::Uuid::new_v4(),
            call_id: request.tool_call_id.clone(),
            action_digest: "fixture".into(),
            policy_revision: policy.lock().unwrap().revision().unwrap(),
            contract_revision: None,
            mode: "ask".into(),
            action_label: "write".into(),
            display_target: request.subject.clone(),
            reason: request.summary.clone(),
            legal_choices: request.legal_choices.clone(),
            expires_at_ms: davinci_session::now_ms() + 60_000,
        };
        if case == "unoffered" {
            challenge
                .legal_choices
                .retain(|choice| choice.scope != GrantScope::DenyWithInstructions);
        }
        let mut calls = 0;
        let reply =
            rpc_resolve_challenge(&request, &challenge, false, dir.path(), &policy, |call| {
                calls += 1;
                assert!(call["timeout"].as_u64().is_some_and(|ms| ms <= 60_000));
                let wire = rpc::extension_ui_requests_from_calls(&[call.clone()]);
                assert_eq!(wire.len(), 1);
                assert_eq!(wire[0]["method"], call["op"]);
                match calls {
                    1 => {
                        assert_eq!(
                            call["options"]
                                .as_array()
                                .unwrap()
                                .iter()
                                .any(|option| option == "deny with instructions"),
                            case != "unoffered"
                        );
                        serde_json::json!("deny with instructions")
                    }
                    2 => {
                        assert_eq!(call["op"], "input");
                        match case {
                            "cancel_input" => serde_json::Value::Null,
                            "empty" => serde_json::json!("   "),
                            "oversize" => serde_json::json!("x".repeat(4097)),
                            _ => serde_json::json!("read the docs first"),
                        }
                    }
                    3 => {
                        assert_eq!(call["op"], "confirm");
                        assert_eq!(call["message"], "read the docs first");
                        if case == "stale_confirm" {
                            policy.lock().unwrap().mode = davinci_agent::PermissionMode::Auto;
                        }
                        if case == "malformed_confirm" {
                            serde_json::json!("true")
                        } else {
                            serde_json::json!(case != "reject_confirm")
                        }
                    }
                    _ => panic!("unexpected dialog"),
                }
            });
        assert_eq!(reply.challenge_id, challenge.id);
        assert_eq!(
            reply.choice_id,
            if case == "accept" {
                "deny_with_instructions"
            } else {
                "deny"
            }
        );
        assert_eq!(
            reply.instructions.as_deref(),
            if case == "accept" {
                Some("read the docs first")
            } else {
                None
            }
        );
        assert_eq!(
            calls,
            match case {
                "unoffered" => 1,
                "cancel_input" | "empty" | "oversize" => 2,
                _ => 3,
            }
        );
        assert!(!dir.path().join("ordinary.txt").exists());
        assert!(!dir.path().join(".davinci/settings.json").exists());
        assert!(policy.lock().unwrap().session_allow.is_empty());
    }
}

#[test]
fn f01_rpc_typed_challenge_bounds_wait_and_rejects_stale_save() {
    use davinci_agent::approval::ApprovalChallenge;
    for case in ["current", "expired", "policy_changed"] {
        let dir = tempfile::tempdir().unwrap();
        let mut initial = davinci_agent::PermissionPolicy::new(davinci_agent::PermissionMode::Ask);
        initial.project_trusted = true;
        let policy = davinci_agent::PermissionState::new(initial);
        let davinci_agent::PermissionVerdict::Ask(request) = policy.lock().unwrap().decide(
            "call",
            "write",
            &serde_json::json!({"path":"ordinary.txt", "content":"fixture"}),
            dir.path(),
        ) else {
            panic!("expected Ask")
        };
        let challenge = ApprovalChallenge {
            schema_version: 1,
            id: uuid::Uuid::new_v4(),
            call_id: request.tool_call_id.clone(),
            action_digest: "engine digest fixture".into(),
            policy_revision: policy.lock().unwrap().revision().unwrap(),
            contract_revision: None,
            mode: "ask".into(),
            action_label: "write".into(),
            display_target: request.subject.clone(),
            reason: request.summary.clone(),
            legal_choices: request.legal_choices.clone(),
            expires_at_ms: if case == "expired" {
                0
            } else {
                davinci_session::now_ms() + 60_000
            },
        };
        let mut asks = 0;
        let reply =
            rpc_resolve_challenge(&request, &challenge, true, dir.path(), &policy, |call| {
                asks += 1;
                assert!(call["timeout"].as_u64().is_some_and(|ms| ms <= 60_000));
                if case == "policy_changed" {
                    let mut state = policy.lock().unwrap();
                    state.mode = davinci_agent::PermissionMode::Auto;
                    state.mode = davinci_agent::PermissionMode::Ask;
                }
                serde_json::json!(RPC_APPROVAL_ALWAYS)
            });
        assert_eq!(reply.challenge_id, challenge.id);
        assert_eq!(
            reply.choice_id,
            if case == "current" { "project" } else { "deny" }
        );
        assert_eq!(asks, usize::from(case != "expired"));
        assert_eq!(
            dir.path().join(".davinci/settings.json").exists(),
            case == "current"
        );
    }
}

#[test]
fn rpc_permission_cannot_add_scopes_to_policy_choices() {
    let dir = tempfile::tempdir().unwrap();
    let policy = davinci_agent::PermissionPolicy::new(davinci_agent::PermissionMode::Ask);
    let davinci_agent::PermissionVerdict::Ask(request) = policy.decide(
        "risky",
        "write",
        &serde_json::json!({"path":".env", "content":"fixture"}),
        dir.path(),
    ) else {
        panic!("expected Ask")
    };
    assert_eq!(
        rpc_approval_call(&request, true)["options"],
        serde_json::json!([RPC_APPROVAL_ONCE, RPC_APPROVAL_DENY])
    );
    for answer in [RPC_APPROVAL_SESSION, RPC_APPROVAL_ALWAYS] {
        assert_eq!(
            rpc_resolve_approval(&request, true, dir.path(), |_| serde_json::json!(answer)),
            davinci_agent::ToolApprovalDecision::Deny
        );
    }
    assert!(!dir.path().join(".davinci/settings.json").exists());
}

#[test]
fn rpc_permission_save_failure_requires_another_explicit_choice() {
    use davinci_agent::ToolApprovalDecision::*;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join(".davinci"), "not a directory").unwrap();
    let request = davinci_agent::ToolApprovalRequest {
        legal_choices: davinci_agent::approval::offer_scopes(false, true, true),
        tool_call_id: "save-failure".into(),
        tool: "write".into(),
        args: serde_json::json!({}),
        subject: "file.txt".into(),
        summary: "write file.txt".into(),
        session_rule: "write(file.txt)".into(),
        outside_project: false,
        mode: davinci_agent::PermissionMode::Ask,
    };
    for (answer, expected) in [
        (RPC_APPROVAL_ONCE, AllowOnce),
        (RPC_APPROVAL_SESSION, AllowForSession),
        (RPC_APPROVAL_DENY, Deny),
        (RPC_APPROVAL_ALWAYS, Deny),
    ] {
        let mut calls = 0;
        let decision = rpc_resolve_approval(&request, true, dir.path(), |call| {
            calls += 1;
            if calls == 1 {
                serde_json::json!(RPC_APPROVAL_ALWAYS)
            } else {
                assert!(call["title"]
                    .as_str()
                    .unwrap()
                    .contains("could not be saved"));
                assert_eq!(
                    call["options"],
                    rpc_approval_call(&request, false)["options"]
                );
                serde_json::json!(answer)
            }
        });
        assert_eq!(calls, 2);
        assert_eq!(decision, expected);
    }
    let saved = tempfile::tempdir().unwrap();
    let mut calls = 0;
    assert_eq!(
        rpc_resolve_approval(&request, true, saved.path(), |_| {
            calls += 1;
            serde_json::json!(RPC_APPROVAL_ALWAYS)
        }),
        AllowAlways
    );
    assert_eq!(calls, 1);
    let settings: serde_json::Value = serde_json::from_slice(
        &std::fs::read(saved.path().join(".davinci/settings.json")).unwrap(),
    )
    .unwrap();
    assert_eq!(
        settings["permissions"]["allow"],
        serde_json::json!([request.session_rule])
    );
    assert_eq!(
        rpc_resolve_approval(&request, true, dir.path(), |call| {
            if call["options"].as_array().unwrap().len() == 4 {
                serde_json::json!(RPC_APPROVAL_ALWAYS)
            } else {
                serde_json::Value::Null
            }
        }),
        Deny
    );
}

#[test]
fn rpc_prompt_auth_error_matches_ts_no_model_copy() {
    let runtime = RpcRuntime::new(
        Agent::new_builtin(davinci_agent::PromptProfile::Stable),
        PathBuf::from("/tmp"),
        PathBuf::from("/tmp"),
    );
    let error = rpc_prompt_auth_error(&runtime).expect("no model");
    assert!(error.starts_with("No model selected."));
    assert!(error.contains("Then use /model to select a model."));
    assert!(error.contains("Use /login to log into a provider via OAuth or API key. See:"));
}

#[test]
fn offline_flag_sets_ts_env_vars() {
    let _env_lock = PROCESS_ENV_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let previous_offline = std::env::var("PI_OFFLINE").ok();
    let previous_skip = std::env::var("PI_SKIP_VERSION_CHECK").ok();
    std::env::remove_var("PI_OFFLINE");
    std::env::remove_var("PI_SKIP_VERSION_CHECK");
    apply_offline_mode(&["--offline".into()]);
    assert_eq!(std::env::var("PI_OFFLINE").as_deref(), Ok("1"));
    assert_eq!(std::env::var("PI_SKIP_VERSION_CHECK").as_deref(), Ok("1"));
    match previous_offline {
        Some(value) => std::env::set_var("PI_OFFLINE", value),
        None => std::env::remove_var("PI_OFFLINE"),
    }
    match previous_skip {
        Some(value) => std::env::set_var("PI_SKIP_VERSION_CHECK", value),
        None => std::env::remove_var("PI_SKIP_VERSION_CHECK"),
    }
}

#[test]
fn parses_print_and_thinking_like_ts() {
    let parsed = parse_args(&[
        "-p".into(),
        "hello".into(),
        "--thinking".into(),
        "high".into(),
    ]);
    assert!(parsed.print);
    assert_eq!(parsed.messages, ["hello"]);
    assert_eq!(parsed.thinking, Some(davinci_protocol::ThinkingLevel::High));
}

#[test]
fn help_lists_product_commands() {
    let help = print_help();
    for needle in [
        "install", "remove", "update", "list", "config", "auth", "server", "client", "--print",
        "--resume",
    ] {
        assert!(help.contains(needle), "missing {needle}");
    }
    assert!(help.contains("Extensions can register additional flags"));
    assert!(help.contains("Examples:"));
    assert!(help.contains("Environment Variables:"));
    assert!(help.contains("PI_CODING_AGENT_DIR"));
    assert!(help.contains("PI_SESSION_DIR"));
    let with_flags =
        args::print_help_with_extension_flags(&[("plan".into(), "/tmp/plan.js".into())]);
    assert!(with_flags.contains("Extension CLI Flags:"));
    assert!(with_flags.contains("--plan"));
    assert_eq!(
        args::normalize_session_name("  demo  ").as_deref(),
        Some("demo")
    );
    let usage = auth_cmd::get_auth_command_usage(auth_cmd::AuthCommandKind::Check);
    assert!(usage.contains("auth check"));
    let command = auth_cmd::parse_auth_command(&[
        "auth".into(),
        "check".into(),
        "--provider".into(),
        "openai".into(),
        "--no-refresh".into(),
    ])
    .unwrap()
    .unwrap();
    assert!(command.no_refresh);
    assert!(auth_cmd::parsed_auth_args(&command).provider.is_some());
    assert!(command.min_expiry_ms.is_none());
    assert!(matches!(
        slash::parse_line("/quit"),
        slash::SlashAction::Quit
    ));
    assert!(matches!(
        slash::parse_line("hello"),
        slash::SlashAction::Prompt(_)
    ));
    assert!(matches!(
        slash::parse_line("/model"),
        slash::SlashAction::OpenModel
    ));
    assert!(matches!(
        slash::parse_line("/thinking"),
        slash::SlashAction::SetThinking(level) if level.is_empty()
    ));
    assert!(matches!(
        slash::parse_line("/thinking high"),
        slash::SlashAction::SetThinking(level) if level == "high"
    ));
    assert_eq!(
            unknown_thinking_error("nope"),
            "Unknown thinking level \"nope\". Available levels: off, minimal, low, medium, high, xhigh, max."
        );
    assert!(matches!(
        slash::parse_line("/tree"),
        slash::SlashAction::Tree
    ));
    assert!(matches!(
        slash::parse_line("/import ./foo.jsonl"),
        slash::SlashAction::Import(_)
    ));
    assert!(matches!(
        slash::parse_line("/share"),
        slash::SlashAction::Share
    ));
    assert!(matches!(slash::parse_line("/mcp"), slash::SlashAction::Mcp));
    assert!(matches!(
        slash::parse_line("/cost"),
        slash::SlashAction::ShowCost
    ));
    assert!(matches!(
        slash::parse_line("/status"),
        slash::SlashAction::ShowStatus
    ));
    assert!(matches!(
        slash::parse_line("/agents"),
        slash::SlashAction::Agents
    ));
    assert!(matches!(
        slash::parse_line("/tasks"),
        slash::SlashAction::Tasks
    ));
    assert_eq!(
        slash::parse_line("/review src/index.ts"),
        slash::SlashAction::Prompt("/review src/index.ts".into())
    );
    assert_eq!(
        slash::parse_line("/skill:test explain this"),
        slash::SlashAction::Prompt("/skill:test explain this".into())
    );
}

#[test]
fn status_text_not_automatically_appended_to_model_context() {
    let agent = Agent::new("sys");
    let initial_len = agent.messages.len();
    let parsed = Args::default();
    let status = format_session_status(&parsed, &agent);
    assert!(!status.is_empty());
    assert_eq!(
        agent.messages.len(),
        initial_len,
        "status output must not append to agent messages"
    );
}

#[test]
fn status_includes_behavior_telemetry_metrics_when_runs_exist() {
    davinci_telemetry::clear_behavior_telemetry();
    for _ in 0..5 {
        davinci_telemetry::record_behavior_telemetry(davinci_telemetry::BehaviorTelemetry {
            prompt_profile: "telemetry-status-fixture".to_string(),
            prompt_version: 2,
            prompt_stable_hash_prefix: "a1b2c3d4".to_string(),
            model_family: "claude".to_string(),
            model_policy: "default".to_string(),
            model_policy_version: 0,
            model_turns: 6,
            tool_calls: 10,
            permission_prompts: 1,
            permission_denials: 0,
            files_changed_count: 2,
            verification_commands_run: 2,
            verification_failures: 1,
            capability_incomplete_evidence: 0,
            aborted: false,
            user_steers: 1,
        });
    }

    let mut agent = Agent::new("sys");
    agent.prompt_manifest = Some(davinci_agent::prompt::manifest::PromptManifest::from_parts(
        "telemetry-status-fixture",
        2,
        &[],
        "stable system prompt prefix",
        "full system prompt",
    ));
    let parsed = Args::default();
    let status = format_session_status(&parsed, &agent);
    assert!(status.contains("Prompt profile: telemetry-status-fixture"));
    assert!(status.contains("Runs: 5"));
    assert!(status.contains("Median turns: 6"));
    assert!(status.contains("Verification failures recovered: 5"));
}

#[test]
fn radius_share_uses_fixture_url() {
    let _env_lock = PROCESS_ENV_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    std::env::set_var("PI_RADIUS_TOKEN", "fixture-token");
    std::env::set_var("PI_RADIUS_ARTIFACT_URL", "https://example.test/session/abc");
    std::env::remove_var("PI_SHARE_DRY_RUN");
    std::env::remove_var("PI_SHARE_URL");
    let agent = Agent::new("sys");
    let shared = share_current_session(&agent).expect("share");
    assert_eq!(shared, "Share URL: https://example.test/session/abc");
    std::env::set_var(
        "PI_RADIUS_ARTIFACT_REPLY",
        r#"{"artifact":{"canonical_url":"https://radius.example/a"}}"#,
    );
    std::env::remove_var("PI_RADIUS_ARTIFACT_URL");
    let shared = share_current_session(&agent).expect("share reply");
    assert_eq!(shared, "Share URL: https://radius.example/a");
    std::env::remove_var("PI_RADIUS_TOKEN");
    std::env::remove_var("PI_RADIUS_ARTIFACT_REPLY");
}

#[test]
fn list_models_table_matches_ts_columns() {
    assert_eq!(format_token_count(200_000), "200K");
    assert_eq!(format_token_count(1_000_000), "1M");
    assert_eq!(format_token_count(1_500_000), "1.5M");
    assert_eq!(format_token_count(500), "500");
    let model = davinci_ai::Model {
        id: "sonnet".into(),
        name: "Sonnet".into(),
        api: "anthropic-messages".into(),
        provider: "anthropic".into(),
        base_url: None,
        reasoning: true,
        input: vec!["text".into(), "image".into()],
        cost: davinci_ai::ModelCost {
            input: 0.0,
            output: 0.0,
            cache_read: 0.0,
            cache_write: 0.0,
        },
        context_window: 200_000,
        max_tokens: 16_384,
        compat: serde_json::json!(null),
        headers: Default::default(),
        thinking_level_map: Default::default(),
    };
    let table = render_models_table(&[&model]);
    let header = table.lines().next().unwrap();
    assert!(header.contains("provider"));
    assert!(header.contains("model"));
    assert!(header.contains("context"));
    assert!(header.contains("max-out"));
    assert!(header.contains("thinking"));
    assert!(header.contains("images"));
    let row = table.lines().nth(1).unwrap();
    assert!(row.contains("anthropic"));
    assert!(row.contains("sonnet"));
    assert!(row.contains("200K"));
    assert!(row.contains("16.4K") || row.contains("16384") || row.contains("16K"));
    assert!(row.contains("yes"));
}

#[test]
fn f01_new_session_hosts_revoke_consent_only_after_creation() {
    for host in ["native", "legacy", "extension"] {
        for fail_creation in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let session_dir = dir.path().join("sessions");
            if fail_creation {
                std::fs::write(&session_dir, "occupied").unwrap();
            }
            let parsed = Args {
                session_dir: Some(session_dir.display().to_string()),
                ..Args::default()
            };
            let mut agent = Agent::new("fixture");
            agent.cwd = dir.path().into();
            let original = JsonlSession::create(
                &dir.path().join("original"),
                &agent.cwd.to_string_lossy(),
                None,
            )
            .unwrap();
            let original_id = original.header.id.clone();
            agent.load_from_session(original).unwrap();
            agent.set_permission_mode(davinci_agent::PermissionMode::Ask);
            agent
                .permissions
                .lock()
                .unwrap()
                .remember("write(ordinary.txt)");
            let revision = agent.permissions.lock().unwrap().revision();
            let mut session =
                InteractiveSession::new(builtin_themes()[0].clone(), "fixture", vec![]);
            match host {
                "native" => {
                    use davinci_tui::davinci::{
                        model::Model,
                        theme::{ColorDepth, Theme},
                    };
                    let mut model =
                        Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 80, 24, false);
                    let result = davinci_interactive::perform(
                        &parsed,
                        &mut agent,
                        &mut model,
                        SlashAction::NewSession,
                    );
                    assert_eq!(result.is_err(), fail_creation);
                }
                "legacy" => {
                    let result = handle_user_line(&parsed, &mut agent, &mut session, "/new", None);
                    assert_eq!(result.is_err(), fail_creation);
                }
                _ => {
                    apply_session_calls(
                        Some(&parsed),
                        &mut agent,
                        SessionCallUi::Chrome(&mut session.chrome),
                        &[serde_json::json!({"op":"newSession"})],
                        false,
                    );
                }
            }
            let policy = agent.permissions.lock().unwrap();
            assert_eq!(policy.session_allow.is_empty(), !fail_creation, "{host}");
            assert_eq!(policy.revision() == revision, fail_creation, "{host}");
            assert_eq!(
                agent.session.as_ref().unwrap().header.id == original_id,
                fail_creation,
                "{host}"
            );
            assert_eq!(policy.mode, davinci_agent::PermissionMode::Ask);
        }
    }
}

#[test]
fn apply_session_calls_creates_session_switches_model_and_steers() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = Agent::new("x");
    agent.cwd = dir.path().to_path_buf();
    let parsed = Args {
        session_dir: Some(dir.path().join("sessions").display().to_string()),
        ..Args::default()
    };
    let mut chrome = ChatChrome::new(builtin_themes()[0].clone(), "pi");
    apply_session_calls(
        Some(&parsed),
        &mut agent,
        SessionCallUi::Chrome(&mut chrome),
        &[
            serde_json::json!({"op":"setModel","model":"sonnet","provider":"anthropic"}),
            serde_json::json!({"op":"sendUserMessage","text":"hi","options":{"deliverAs":"steer"}}),
            serde_json::json!({"op":"exec","command":"echo","stdout":"ok"}),
            serde_json::json!({"op":"newSession"}),
        ],
        false,
    );
    assert_eq!(agent.provider, "anthropic");
    assert_eq!(agent.model_id, "sonnet");
    assert_eq!(agent.queues.steer.len(), 1);
    assert!(agent.session.is_some());
    assert!(agent.messages.is_empty());
    assert!(chrome
        .transcript
        .lines
        .iter()
        .any(|line| line.role == "exec" && line.text == "ok"));
    assert!(chrome.status.contains("newSession") || chrome.status.contains("model="));
}

#[test]
fn visual_verification_prompt_state_requires_registered_native_backend() {
    let mut agent = Agent::new("x");
    let host = ExtensionHost::default();

    sync_visual_verification_availability(&mut agent, &host);
    assert!(!agent.runtime_prompt_state().visual_verification_available);

    agent.apply_extension_tools(&[String::from("visual_snapshot")]);
    sync_visual_verification_availability(&mut agent, &host);
    assert!(!agent.runtime_prompt_state().visual_verification_available);

    {
        let mut native = host.native.lock().unwrap_or_else(|err| err.into_inner());
        native.visual_verification_available = true;
    }
    sync_visual_verification_availability(&mut agent, &host);
    assert!(!agent.runtime_prompt_state().visual_verification_available);
}

#[test]
fn rebind_print_extensions_rediscovers_skills_and_emits_session_start() {
    let _env_lock = PROCESS_ENV_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let _agent_dir = EnvRestore::set(
        "PI_CODING_AGENT_DIR",
        &dir.path().join("agent").to_string_lossy(),
    );
    let _current_dir = EnvRestore::set(
        "DAVINCI_CODING_AGENT_DIR",
        &dir.path().join("agent").to_string_lossy(),
    );
    let skill_dir = dir.path().join(".pi").join("skills").join("demo");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: demo\ndescription: demo skill\n---\n# Demo\n",
    )
    .unwrap();
    let mut agent = Agent::new("x");
    agent.cwd = dir.path().to_path_buf();
    let parsed = Args {
        project_trust_override: Some(true),
        ..Args::default()
    };
    let mut host = ExtensionHost::default();
    rebind_print_extensions(&parsed, &mut agent, &mut host);
    assert!(
        agent.skills.iter().any(|skill| skill.name == "demo"),
        "print-mode rebind should rediscover project skills: {:?}",
        agent
            .skills
            .iter()
            .map(|skill| &skill.name)
            .collect::<Vec<_>>()
    );
    assert!(host
        .events
        .iter()
        .any(|event| { matches!(event, crate::extension_host::ExtensionEvent::SessionStart) }));
}

#[test]
fn show_loaded_resources_lists_context_skills_and_expands() {
    let _env_lock = PROCESS_ENV_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let _agent_dir = EnvRestore::set(
        "PI_CODING_AGENT_DIR",
        &dir.path().join("agent").to_string_lossy(),
    );
    let _current_dir = EnvRestore::set(
        "DAVINCI_CODING_AGENT_DIR",
        &dir.path().join("agent").to_string_lossy(),
    );
    let skill_dir = dir.path().join(".pi").join("skills").join("demo");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: demo\ndescription: demo skill\n---\n# Demo\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("AGENTS.md"), "# agents\n").unwrap();
    let mut agent = Agent::new("x");
    agent.cwd = dir.path().to_path_buf();
    let parsed = Args {
        project_trust_override: Some(true),
        ..Args::default()
    };
    apply_discovered_resources(&parsed, &mut agent);
    let mut session = InteractiveSession::new(
        builtin_themes()[0].clone(),
        "pi",
        vec!["google/gemini".into()],
    );
    show_loaded_resources(&mut session, &agent, &ExtensionHost::default(), &parsed);
    let collapsed = session.chrome.render_document(80).join("\n");
    assert!(collapsed.contains("[Skills]"), "{collapsed}");
    assert!(collapsed.contains("demo"), "{collapsed}");
    assert!(collapsed.contains("[Context]"), "{collapsed}");
    assert!(collapsed.contains("AGENTS.md"), "{collapsed}");
    session.chrome.set_tools_expanded(true);
    let expanded = session.chrome.render_document(80).join("\n");
    assert!(
        expanded.contains("SKILL.md") || expanded.contains("demo"),
        "{expanded}"
    );
    session.quiet_startup = true;
    show_loaded_resources(&mut session, &agent, &ExtensionHost::default(), &parsed);
    let quiet = session.chrome.render_document(80).join("\n");
    assert!(!quiet.contains("[Skills]"), "{quiet}");
}

#[test]
fn startup_header_and_model_scope_match_ts() {
    let theme = builtin_themes()[0].clone();
    let mut session = InteractiveSession::new(theme, "pi", vec!["anthropic/sonnet".into()]);
    session.enabled_model_ids = Some(vec!["anthropic/sonnet".into()]);
    session
        .scoped_thinking_levels
        .insert("anthropic/sonnet".into(), "high".into());
    apply_startup_header(&mut session, false);
    let collapsed = session.chrome.render_document(80).join("\n");
    assert!(
        collapsed.contains("full startup help and loaded resources"),
        "{collapsed}"
    );
    assert!(collapsed.contains("ctrl+o"), "{collapsed}");
    session.chrome.set_tools_expanded(true);
    let expanded = session.chrome.render_document(80).join("\n");
    assert!(expanded.contains("to expand tools"), "{expanded}");
    let line = model_scope_startup_line(&session).expect("scope");
    assert!(line.contains("Model scope:"), "{line}");
    assert!(line.contains("sonnet:high"), "{line}");
    session.quiet_startup = true;
    apply_startup_header(&mut session, false);
    assert!(session.chrome.startup_header.is_none());
    assert!(model_scope_startup_line(&session).is_none());
}

#[test]
fn streaming_turn_delivers_offline_reply_to_transcript() {
    let _env_lock = PROCESS_ENV_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let _agent_dir = EnvRestore::set("PI_CODING_AGENT_DIR", &dir.path().to_string_lossy());
    let _current_dir = EnvRestore::set("DAVINCI_CODING_AGENT_DIR", &dir.path().to_string_lossy());
    let theme = builtin_themes()[0].clone();
    let mut session = InteractiveSession::new(theme, "davinci", vec![]);
    session.width = 100;
    let mut agent = Agent::new("test system prompt");
    agent.cwd = dir.path().to_path_buf();
    agent.provider = "openai-codex".into();
    agent.model_id = "gpt-5.6-sol".into();
    let parsed = Args {
        offline: true,
        no_extensions: true,
        ..Args::default()
    };
    let panes = ChromePanes::new(Vec::new(), Vec::new());
    let options = InteractiveTuiOptions {
        tui_mode: TuiMode::Regular,
        show_hardware_cursor: false,
        log_directory: dir.path().to_path_buf(),
        terminal: Box::new(davinci_tui::MemoryTerminal::new(100, 30)),
        theme: builtin_themes()[0].clone(),
        copy_on_select: false,
        open_url: None,
        on_right_click_paste: None,
        copy_selection: None,
    };
    let mut tui = create_interactive_tui(options);
    remount_chrome_panes(&mut tui, &panes);
    ACTIVE_PANES.with(|slot| *slot.borrow_mut() = Some(panes.clone()));
    let ok = submit_user_message(
        &parsed,
        &mut agent,
        &mut session,
        "hello there",
        &[],
        Some(&mut tui),
    )
    .unwrap();
    ACTIVE_PANES.with(|slot| *slot.borrow_mut() = None);
    assert!(ok);
    let doc = session.chrome.render_document(100).join("\n");
    let plain = davinci_tui::strip_terminal_sequences(&doc);
    assert!(plain.contains("> hello there"), "{plain}");
    assert!(
        plain.contains("(offline) received"),
        "assistant reply missing from transcript: {plain}"
    );
    assert!(session.chrome.working_message.is_none());
}

#[test]
fn keystroke_pipeline_stays_fast_on_long_transcripts() {
    let theme = builtin_themes()[0].clone();
    let mut session =
        InteractiveSession::new(theme, "davinci", vec!["openai-codex/gpt-5.6-sol".into()]);
    session.width = 120;
    for i in 0..400 {
        session
            .chrome
            .transcript
            .push("user", format!("prompt {i}"));
        session.chrome.transcript.push(
            "assistant",
            format!("Answer {i} with some *markdown* and `code` and a\nsecond line."),
        );
        session
            .chrome
            .transcript
            .push("tool", format!("✓ manus · cargo check step {i}  0.3{i}s"));
    }
    session.chrome.footer_cwd = Some("C:\\dev\\davinci-rust".into());
    session.chrome.footer_model = Some("openai-codex/gpt-5.6-sol".into());
    session.chrome.footer_context = Some((47_000, 200_000));
    // Warm the render memo like a running session.
    let _ = session.chrome.render_document(120);
    let started = std::time::Instant::now();
    let keys = 60;
    for i in 0..keys {
        let ch = char::from(b'a' + (i % 26) as u8);
        let _ = session.handle_bytes(&ch.to_string());
        let _ = session.chrome.render_document(120);
        let _ = session.chrome.render_dock(120);
    }
    let per_key = started.elapsed() / keys;
    assert!(
        per_key < std::time::Duration::from_millis(12),
        "keystroke pipeline too slow: {per_key:?} per key"
    );
}

#[test]
fn davinci_frame_composes_transcript_composer_and_status_bar() {
    let theme = builtin_themes()[0].clone();
    let mut session =
        InteractiveSession::new(theme, "davinci", vec!["openai-codex/gpt-5.6-sol".into()]);
    session.width = 100;
    session.chrome.transcript.agent_label = "davinci".into();
    session.chrome.transcript.push("user", "run the tests");
    session
        .chrome
        .transcript
        .push("tool", "✓ manus · cargo test -p davinci-agent  1.84s");
    session.chrome.transcript.push(
            "tool",
            "× manus · cargo test -p pi-session  0.42s\n  ! error[E0308] mismatched types · store.rs:118",
        );
    session.chrome.transcript.push(
        "assistant",
        "The failing case builds a path with a forward slash.",
    );
    push_native_panel(
        &mut session,
        "governor-status",
        &serde_json::json!({
            "enabled": true,
            "compressedOutputs": 14,
            "deduplicatedReads": 6,
            "blockedCalls": 0,
        }),
    );
    session.chrome.footer_cwd = Some("C:\\dev\\davinci-rust".into());
    session.chrome.footer_branch = Some("main".into());
    session.chrome.footer_model = Some("openai-codex/gpt-5.6-sol".into());
    session.chrome.footer_context = Some((47_000, 200_000));
    session.chrome.footer_delta = Some((3, 42, 11));
    let document = session.chrome.render_document(100).join("\n");
    let dock = session.chrome.render_dock(100).join("\n");
    println!("─ document ─\n{document}\n─ dock ─\n{dock}");
    let plain_doc = davinci_tui::strip_terminal_sequences(&document);
    let plain_dock = davinci_tui::strip_terminal_sequences(&dock);
    assert!(plain_doc.contains("> run the tests"), "{plain_doc}");
    assert!(plain_doc.contains("◆ davinci"), "{plain_doc}");
    assert!(plain_doc.contains("✓ manus · cargo test -p davinci-agent"));
    assert!(plain_doc.contains("! error[E0308]"), "{plain_doc}");
    assert!(
        plain_doc.contains("MENSURA · TOKEN GOVERNOR"),
        "{plain_doc}"
    );
    assert!(plain_doc.contains("compressed outputs"), "{plain_doc}");
    assert!(plain_dock.contains("›"), "{plain_dock}");
    assert!(plain_dock.contains("enter send"), "{plain_dock}");
    assert!(plain_dock.contains("47k/200k"), "{plain_dock}");
    assert!(plain_dock.contains("Δ3 +42 -11"), "{plain_dock}");
    assert!(plain_dock.contains("main"), "{plain_dock}");
}

#[test]
fn apply_discovered_resources_keeps_cli_skill_paths() {
    let _env_lock = PROCESS_ENV_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let _agent_dir = EnvRestore::set(
        "PI_CODING_AGENT_DIR",
        &dir.path().join("agent").to_string_lossy(),
    );
    let _current_dir = EnvRestore::set(
        "DAVINCI_CODING_AGENT_DIR",
        &dir.path().join("agent").to_string_lossy(),
    );
    let extra = dir.path().join("extra-skill.md");
    std::fs::write(
        &extra,
        "---\nname: cli-skill\ndescription: from --skill\n---\n# Extra\n",
    )
    .unwrap();
    let mut agent = Agent::new("x");
    agent.cwd = dir.path().to_path_buf();
    let parsed = Args {
        skills: vec![extra.display().to_string()],
        project_trust_override: Some(false),
        ..Args::default()
    };
    apply_discovered_resources(&parsed, &mut agent);
    assert!(
        agent.skills.iter().any(|skill| skill.name == "cli-skill"),
        "CLI --skill paths must survive reload/rebind: {:?}",
        agent
            .skills
            .iter()
            .map(|skill| &skill.name)
            .collect::<Vec<_>>()
    );
}

#[test]
fn extension_reload_does_not_load_untrusted_project_skills() {
    let _env_lock = PROCESS_ENV_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let _agent_dir = EnvRestore::set(
        "PI_CODING_AGENT_DIR",
        &dir.path().join("agent").to_string_lossy(),
    );
    let _davinci_agent_dir = EnvRestore::set(
        "DAVINCI_CODING_AGENT_DIR",
        &dir.path().join("agent").to_string_lossy(),
    );
    let skill_dir = dir.path().join(".pi").join("skills").join("planted");
    std::fs::create_dir_all(&skill_dir).unwrap();
    std::fs::write(
        skill_dir.join("SKILL.md"),
        "---\nname: planted\ndescription: planted skill\n---\n# Planted\n",
    )
    .unwrap();
    let mut agent = Agent::new("x");
    agent.cwd = dir.path().to_path_buf();
    apply_session_calls(
        None,
        &mut agent,
        SessionCallUi::Silent,
        &[serde_json::json!({"op":"reload"})],
        false,
    );
    assert!(agent.skills.iter().all(|skill| skill.name != "planted"));
}

#[test]
fn suspend_dry_run_sets_status() {
    let _env_lock = PROCESS_ENV_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let theme = builtin_themes().into_iter().next().expect("theme");
    let mut session = InteractiveSession::new(theme, "pi", vec!["google/gemini".into()]);
    std::env::set_var("PI_SUSPEND_DRY_RUN", "1");
    apply_suspend(&mut session, false);
    std::env::remove_var("PI_SUSPEND_DRY_RUN");
    if cfg!(windows) {
        assert_eq!(
            session.chrome.status,
            "Suspend to background is not supported on Windows"
        );
    } else {
        assert_eq!(session.chrome.status, "Suspended");
    }
}

fn test_session() -> (Args, Agent, InteractiveSession) {
    let theme = builtin_themes().into_iter().next().expect("theme");
    (
        Args::default(),
        Agent::new("sys"),
        InteractiveSession::new(theme, "pi", vec!["anthropic/claude-fable-5".into()]),
    )
}

#[test]
fn bare_login_opens_auth_type_selector() {
    let (parsed, mut agent, mut session) = test_session();
    handle_user_line(&parsed, &mut agent, &mut session, "/login", None).unwrap();
    let selector = session
        .chrome
        .extension_selector
        .as_ref()
        .expect("auth-type selector");
    assert_eq!(selector.title, "Select authentication method:");
    assert!(selector
        .options
        .iter()
        .any(|item| item == "Sign in with an account"));
    assert!(selector
        .options
        .iter()
        .any(|item| item == "Sign in with an API key"));
}

#[test]
fn login_anthropic_opens_auth_type_when_both_methods_exist() {
    let (parsed, mut agent, mut session) = test_session();
    handle_user_line(&parsed, &mut agent, &mut session, "/login anthropic", None).unwrap();
    let selector = session
        .chrome
        .extension_selector
        .as_ref()
        .expect("provider auth-type");
    assert_eq!(
        selector.title,
        "Select authentication method for Anthropic:"
    );
    assert_eq!(session.login_auth_options.len(), 2);
    assert!(session.login_auth_type_labels.is_some());
}

#[test]
fn login_bedrock_opens_api_key_dialog() {
    let (parsed, mut agent, mut session) = test_session();
    handle_user_line(
        &parsed,
        &mut agent,
        &mut session,
        "/login amazon-bedrock",
        None,
    )
    .unwrap();
    assert!(session.chrome.login_dialog.is_some());
    assert!(session.chrome.extension_selector.is_none());
    let rendered = session
        .chrome
        .login_dialog
        .as_ref()
        .unwrap()
        .render(80)
        .join("\n");
    assert!(rendered.contains("AWS profile") || rendered.contains("Enter API key"));
}

#[test]
fn ambient_login_shows_configured_outside_dialog() {
    let theme = builtin_themes().into_iter().next().expect("theme");
    let mut session = InteractiveSession::new(theme, "pi", vec!["google/gemini".into()]);
    start_provider_login(&mut session, "custom-ambient", "api_key", None).unwrap();
    let dialog = session
        .chrome
        .login_dialog
        .as_ref()
        .expect("ambient dialog");
    let rendered = dialog.render(80).join("\n");
    assert!(rendered.contains(&format!("Authentication is configured outside {APP_NAME}.")));
}

#[test]
fn bare_logout_without_stored_credentials_matches_ts() {
    let _env_lock = PROCESS_ENV_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let _agent_dir = EnvRestore::set("PI_CODING_AGENT_DIR", &dir.path().to_string_lossy());
    let _current_dir = EnvRestore::set("DAVINCI_CODING_AGENT_DIR", &dir.path().to_string_lossy());
    let (parsed, mut agent, mut session) = test_session();
    handle_user_line(&parsed, &mut agent, &mut session, "/logout", None).unwrap();
    assert!(session.chrome.oauth_selector.is_none());
    assert!(session
        .chrome
        .status
        .contains("No stored credentials to remove"));
}

#[test]
fn print_mode_exits_nonzero_on_assistant_error() {
    let message = AssistantMessage {
        id: "m1".into(),
        role: "assistant".into(),
        content: vec![],
        model: "x".into(),
        usage: None,
        stop_reason: Some(StopReason::Error),
        error_message: Some("provider failure".into()),
    };
    let events = vec![AgentEvent::MessageUpdate {
        message: std::sync::Arc::new(davinci_ai::ChatMessage::text("assistant", "")),
        assistant_message_event: davinci_ai::AssistantMessageEvent::Error {
            reason: StopReason::Error,
            error: message,
        },
    }];
    assert_eq!(
        print_text_exit(&events),
        (1, Some("provider failure".into()))
    );
    let aborted = AssistantMessage {
        id: "m2".into(),
        role: "assistant".into(),
        content: vec![],
        model: "x".into(),
        usage: None,
        stop_reason: Some(StopReason::Aborted),
        error_message: None,
    };
    let events = vec![AgentEvent::MessageUpdate {
        message: std::sync::Arc::new(davinci_ai::ChatMessage::text("assistant", "")),
        assistant_message_event: davinci_ai::AssistantMessageEvent::Error {
            reason: StopReason::Aborted,
            error: aborted,
        },
    }];
    assert_eq!(
        print_text_exit(&events),
        (1, Some("Request aborted".into()))
    );
}

#[test]
fn print_json_event_strips_partial_and_adds_toolcall_ids() {
    let message = AssistantMessage {
        id: "m1".into(),
        role: "assistant".into(),
        content: vec![ContentBlock::ToolCall {
            id: "call-1".into(),
            name: "bash".into(),
            arguments: serde_json::json!({}),
        }],
        model: "x".into(),
        usage: None,
        stop_reason: None,
        error_message: None,
    };
    let json = to_json_print_event(&AgentEvent::MessageUpdate {
        message: std::sync::Arc::new(davinci_ai::ChatMessage::text("assistant", "")),
        assistant_message_event: davinci_ai::AssistantMessageEvent::ToolcallStart {
            content_index: 0,
            partial: std::sync::Arc::new(message),
        },
    })
    .unwrap();
    assert_eq!(json["type"], "message_update");
    assert!(json["assistantMessageEvent"].get("partial").is_none());
    assert_eq!(json["assistantMessageEvent"]["id"], "call-1");
    assert_eq!(json["assistantMessageEvent"]["toolName"], "bash");
    assert_eq!(json["assistantMessageEvent"]["type"], "toolcall_start");
}

#[test]
fn print_json_fast_path_matches_full_serialization_minus_partial() {
    use davinci_ai::AssistantMessageEvent as Ev;
    let partial = std::sync::Arc::new(AssistantMessage {
        id: "m1".into(),
        role: "assistant".into(),
        content: vec![ContentBlock::Text { text: "big".into() }],
        model: "x".into(),
        usage: None,
        stop_reason: None,
        error_message: None,
    });
    let tool_call = ContentBlock::ToolCall {
        id: "call-1".into(),
        name: "read".into(),
        arguments: serde_json::json!({"path": "a"}),
    };
    let events = vec![
        Ev::Start {
            partial: partial.clone(),
        },
        Ev::TextStart {
            content_index: 0,
            partial: partial.clone(),
        },
        Ev::TextDelta {
            content_index: 0,
            delta: "d".into(),
            partial: partial.clone(),
        },
        Ev::TextEnd {
            content_index: 0,
            content: "c".into(),
            partial: partial.clone(),
        },
        Ev::ThinkingStart {
            content_index: 1,
            partial: partial.clone(),
        },
        Ev::ThinkingDelta {
            content_index: 1,
            delta: "d".into(),
            partial: partial.clone(),
        },
        Ev::ThinkingEnd {
            content_index: 1,
            content: "c".into(),
            partial: partial.clone(),
        },
        Ev::ToolcallDelta {
            content_index: 2,
            delta: "{".into(),
            partial: partial.clone(),
        },
        Ev::ToolcallEnd {
            content_index: 2,
            tool_call,
            partial: partial.clone(),
        },
    ];
    for event in events {
        let fast = to_json_print_event(&AgentEvent::MessageUpdate {
            message: std::sync::Arc::new(davinci_ai::ChatMessage::text("assistant", "")),
            assistant_message_event: event.clone(),
        })
        .unwrap();
        let mut reference = serde_json::to_value(&event).unwrap();
        reference.as_object_mut().unwrap().remove("partial");
        assert_eq!(
            fast["assistantMessageEvent"], reference,
            "fast path diverged for {reference:?}"
        );
    }
}

#[test]
fn json_mode_help_takes_over_stdout() {
    let help = parse_args(&["--help".into()]);
    assert!(!should_take_over_stdout(&help));
    let json_help = parse_args(&["--mode".into(), "json".into(), "--help".into()]);
    assert!(should_take_over_stdout(&json_help));
    let print_help = parse_args(&["-p".into(), "--help".into()]);
    assert!(should_take_over_stdout(&print_help));
}

#[test]
fn apply_resolved_models_fuzzy_and_thinking_suffix() {
    let parsed = Args {
        provider: Some("anthropic".into()),
        model: Some("sonnet:high".into()),
        ..Args::default()
    };
    let mut agent = Agent::new_builtin(davinci_agent::PromptProfile::Stable);
    apply_resolved_models(&parsed, &mut agent).expect("resolve");
    assert!(agent.model_id.to_ascii_lowercase().contains("sonnet"));
    assert_eq!(agent.thinking_level, davinci_protocol::ThinkingLevel::High);
    assert_eq!(agent.provider, "anthropic");
}

#[test]
fn apply_resolved_models_unknown_is_error() {
    let parsed = Args {
        model: Some("definitely-not-a-real-model-xyz".into()),
        ..Args::default()
    };
    let mut agent = Agent::new_builtin(davinci_agent::PromptProfile::Stable);
    let err = apply_resolved_models(&parsed, &mut agent).unwrap_err();
    assert!(err.contains("not found"));
}

#[test]
fn resume_fixture_opens_selected_session() {
    let _env_lock = PROCESS_ENV_LOCK
        .lock()
        .unwrap_or_else(|error| error.into_inner());
    let dir = tempfile::tempdir().unwrap();
    let session = JsonlSession::create(dir.path(), "/tmp/resume-pick", Some("picked")).unwrap();
    std::env::set_var("PI_RESUME_SESSION", session.path.display().to_string());
    let parsed = Args {
        resume: true,
        ..Args::default()
    };
    let selected =
        select_resume_session(&parsed, dir.path(), Path::new("/tmp/resume-pick")).expect("select");
    std::env::remove_var("PI_RESUME_SESSION");
    assert_eq!(selected.unwrap(), session.path);
}

#[test]
fn prompt_profile_rollback_test() {
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("sessions");
    let cwd = dir.path().join("cwd");
    std::fs::create_dir_all(&session_dir).unwrap();
    std::fs::create_dir_all(&cwd).unwrap();

    let preview_args = Args {
        prompt_profile: Some(davinci_agent::PromptProfile::Preview),
        ..Args::default()
    };
    let preview_agent = build_agent(&preview_args, &session_dir, &cwd).unwrap();
    let preview_manifest = preview_agent.prompt_manifest.as_ref().unwrap();
    assert_eq!(preview_manifest.profile, "preview");
    assert_eq!(preview_manifest.profile_version, 3);

    let rollback_args = Args {
        prompt_profile: Some(davinci_agent::PromptProfile::LegacyV1),
        ..Args::default()
    };
    let rollback_agent = build_agent(&rollback_args, &session_dir, &cwd).unwrap();
    let rollback_manifest = rollback_agent.prompt_manifest.as_ref().unwrap();
    assert_eq!(rollback_manifest.profile, "legacy-v1");
    assert_eq!(rollback_manifest.profile_version, 1);

    assert_ne!(
        preview_manifest.stable_sha256,
        rollback_manifest.stable_sha256
    );

    let status_preview = format_session_status(&preview_args, &preview_agent);
    assert!(status_preview.contains("prompt: preview v3"));

    let status_rollback = format_session_status(&rollback_args, &rollback_agent);
    assert!(status_rollback.contains("prompt: legacy-v1 v1"));
}

#[test]
fn build_agent_rejects_invalid_selected_prompt_profile() {
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("sessions");
    let cwd = dir.path().join("cwd");
    std::fs::create_dir_all(cwd.join(".davinci")).unwrap();
    std::fs::write(
        cwd.join(".davinci").join("settings.json"),
        r#"{"promptProfile":"experimental"}"#,
    )
    .unwrap();
    let parsed = Args {
        project_trust_override: Some(true),
        ..Args::default()
    };

    let error = match build_agent(&parsed, &session_dir, &cwd) {
        Ok(_) => panic!("an invalid selected project profile must reach the host boundary"),
        Err(error) => error,
    };
    assert_eq!(
        error,
        "Invalid prompt profile 'experimental'. Valid profiles: stable, preview, legacy-v1"
    );
}

#[test]
fn status_includes_astra_model_policy_identity() {
    let mut agent = Agent::new_builtin(davinci_agent::PromptProfile::Stable);
    let manifest = agent.prompt_manifest.as_mut().expect("manifest");
    manifest.model_policy = "gpt6-astra".to_string();
    manifest.model_policy_version = 1;

    let status = format_session_status(&Args::default(), &agent);
    assert!(status.contains("prompt: stable v2 · gpt6-astra v1 ·"));
    assert!(!status.contains(&agent.system_prompt));
}

#[test]
fn json_mode_and_status_expose_safe_prompt_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let session_dir = dir.path().join("sessions");
    let cwd = dir.path().join("cwd");
    std::fs::create_dir_all(&session_dir).unwrap();
    std::fs::create_dir_all(&cwd).unwrap();

    let parsed = Args {
        mode: Some(Mode::Json),
        ..Args::default()
    };
    let agent = build_agent(&parsed, &session_dir, &cwd).unwrap();
    assert!(agent.tool_context.semantic.is_some());
    assert!(agent.prompt_session.is_builtin());
    assert_eq!(
        agent.prompt_session.profile(),
        Some(davinci_agent::PromptProfile::Stable)
    );
    let manifest = agent.prompt_manifest.as_ref().expect("manifest");
    assert_eq!(manifest.profile, "stable");
    assert!(!manifest.stable_sha256.is_empty());

    let status = format_session_status(&parsed, &agent);
    assert!(status.contains("prompt: stable"));
    assert!(!status.contains(&agent.system_prompt));

    let mut preview_agent = build_agent(
        &Args {
            prompt_profile: Some(davinci_agent::PromptProfile::Preview),
            ..Args::default()
        },
        &session_dir,
        &cwd,
    )
    .unwrap();
    preview_agent.prompt_session.candidate_id = Some("cand-99".into());
    preview_agent.prompt_session.transition_diagnostic =
        Some("Prompt hash transition on resume".into());
    let preview_status = format_session_status(&parsed, &preview_agent);
    assert!(preview_status.contains("prompt: preview v3 [cand-99]"));
    assert!(preview_status.contains("transition: Prompt hash transition on resume"));
    assert!(!preview_status.contains(&preview_agent.system_prompt));
}

#[test]
fn json_prompt_manifest_event_exposes_identity_without_prompt_text() {
    let agent = Agent::new_builtin(davinci_agent::PromptProfile::Preview);
    let event = prompt_manifest_json_event(&agent).expect("prompt manifest");

    assert_eq!(event["type"], "prompt_manifest");
    assert_eq!(event["promptManifest"]["profile"], "preview");
    assert!(!event.to_string().contains(&agent.system_prompt));
}
