use super::*;

#[test]
fn lean_surface_exposes_only_core_tools_on_cache_routes() {
    let visible_with = |surface: crate::ToolSurface| {
        let mut agent = Agent::new_builtin(crate::prompt::PromptProfile::Stable);
        agent.turn_context_placement_override =
            Some(crate::turn_context::TurnContextPlacement::Appended);
        agent.apply_extension_tools(&["ticket".into()]);
        agent.tool_surface = surface;
        agent.freeze_tools_for_cache();
        agent.visible_tool_names()
    };
    let lean = visible_with(crate::ToolSurface::Lean);
    let full = visible_with(crate::ToolSurface::Full);
    assert!(
        lean.contains("read") && lean.contains("tool_search"),
        "{lean:?}"
    );
    assert!(!lean.contains("ticket"), "{lean:?}");
    assert!(lean
        .iter()
        .all(|name| crate::tools::LEAN_TOOLS.contains(&name.as_str())));
    assert!(full.len() > lean.len(), "full={full:?} lean={lean:?}");
}

#[test]
fn lean_surface_discovery_changes_schema_once_and_respects_authorization() {
    let mut agent = Agent::new("x");
    agent.set_runtime(RuntimeHandle::new(
        RunId::new(),
        AgentId::new(),
        RuntimeBus::new(),
    ));
    agent.turn_context_placement_override =
        Some(crate::turn_context::TurnContextPlacement::Appended);
    agent.tool_surface = crate::ToolSurface::Lean;
    agent.freeze_tools_for_cache();
    assert!(!agent.is_tool_visible("code_outline"));
    let before = agent.provider_tool_schema_identity();
    execute_tool_with(
        Path::new("."),
        "tool_search",
        &serde_json::json!({"query":"code_outline"}),
        &agent.tool_context,
    )
    .unwrap();
    assert!(agent.is_tool_visible("code_outline"));
    let after = agent.provider_tool_schema_identity();
    assert_ne!(before, after);
    agent.freeze_tools_for_cache();
    execute_tool_with(
        Path::new("."),
        "tool_search",
        &serde_json::json!({"query":"code_outline"}),
        &agent.tool_context,
    )
    .unwrap();
    assert_eq!(after, agent.provider_tool_schema_identity());
    agent.set_active_tools_by_name(&["read".into(), "tool_search".into()]);
    agent.freeze_tools_for_cache();
    execute_tool_with(
        Path::new("."),
        "tool_search",
        &serde_json::json!({"query":"code_outline"}),
        &agent.tool_context,
    )
    .unwrap();
    assert!(!agent.is_tool_visible("code_outline"));
    assert!(!agent.is_tool_visible("write"));
}

#[test]
fn lean_surface_does_not_change_non_cache_routes() {
    let mut agent = Agent::new("x");
    agent.turn_context_placement_override =
        Some(crate::turn_context::TurnContextPlacement::SystemPrompt);
    let before = agent.visible_tool_names();
    agent.tool_surface = crate::ToolSurface::Lean;
    agent.freeze_tools_for_cache();
    assert_eq!(before, agent.visible_tool_names());
}

#[test]
fn effort_signals_track_real_turns_and_preserve_configured_identity() {
    let mut agent = Agent::new("stable system");
    agent.prompt("fix it");
    agent
        .messages
        .push(ChatMessage::tool_result("c1", "write", "ok", false));
    agent
        .messages
        .push(ChatMessage::tool_result("c2", "bash", "boom", true));
    agent
        .messages
        .push(ChatMessage::tool_result("c3", "bash", "boom", true));
    let mut reminder = ChatMessage::text("user", "Verify before finishing");
    reminder
        .extra
        .insert("davinciCapabilityReminder".into(), Value::Bool(true));
    agent.messages.push(reminder);
    agent.messages.push(ChatMessage::text("custom", "context"));
    agent.prompt_with("injected context", &[]);
    assert_eq!(
        agent.effort_signals(),
        crate::effort::EffortSignals {
            mutations: 1,
            consecutive_failures: 2
        }
    );
    agent.thinking_level = ThinkingLevel::Medium;
    assert_eq!(agent.request_thinking_level(), ThinkingLevel::Medium);
    agent.effort_policy = crate::effort::EffortPolicy::Adaptive;
    assert_eq!(agent.request_thinking_level(), ThinkingLevel::High);
    assert_eq!(agent.thinking_level, ThinkingLevel::Medium);
    assert_eq!(agent.system_prompt, "stable system");
    agent.prompt("a new task");
    assert_eq!(
        agent.effort_signals(),
        crate::effort::EffortSignals::default()
    );
    assert_eq!(agent.request_thinking_level(), ThinkingLevel::Low);
}

#[test]
fn effort_signals_count_batch_children_and_ignore_skipped_operations() {
    let mut agent = Agent::new("x");
    agent.prompt("fix it");
    let mut result = ChatMessage::tool_result("batch-1", "batch", "summary", false);
    result.extra.insert(
        "details".into(),
        serde_json::json!({"operations": [
            {"tool": "write", "status": "ok"},
            {"tool": "edit", "status": "error"},
            {"tool": "bash", "status": "error"},
            {"tool": "read", "status": "skipped"}
        ]}),
    );
    agent.messages.push(result);
    assert_eq!(
        agent.effort_signals(),
        crate::effort::EffortSignals {
            mutations: 1,
            consecutive_failures: 2
        }
    );
    agent
        .messages
        .push(ChatMessage::tool_result("c2", "read", "ok", false));
    assert_eq!(
        agent.effort_signals(),
        crate::effort::EffortSignals {
            mutations: 1,
            consecutive_failures: 0
        }
    );
}

#[test]
fn f03_equivalent_session_path_reuses_live_runtime() {
    let dir = tempfile::tempdir().unwrap();
    let session = JsonlSession::create(dir.path(), "fixture", None).unwrap();
    let intermediate = session.path.parent().unwrap().join("alias");
    std::fs::create_dir(&intermediate).unwrap();
    let alias = intermediate
        .join("..")
        .join(session.path.file_name().unwrap());
    let mut agent = Agent::new("fixture");
    agent.load_from_session(session).unwrap();
    let run = agent.runtime_for_session().unwrap().run_id;
    agent
        .load_from_session(JsonlSession::open(&alias).unwrap())
        .unwrap();
    assert_eq!(agent.runtime_for_session().unwrap().run_id, run);
}

#[test]
fn f03_session_reload_preserves_live_claim_and_writer_lease() {
    let dir = tempfile::tempdir().unwrap();
    let session = JsonlSession::create(dir.path(), "fixture", None).unwrap();
    let path = session.path.clone();
    let mut agent = Agent::new("fixture");
    agent.load_from_session(session).unwrap();
    let worker = agent.runtime_for_session().unwrap().clone();
    let id = worker
        .task_registry
        .create_task(TaskRecord::new(worker.run_id, "live"))
        .unwrap();
    let before = worker.task_registry.get_task(&id).unwrap();
    worker
        .task_registry
        .claim_task(id, worker.run_id, worker.agent_id, before.revision)
        .unwrap();
    let claimed = worker.task_registry.get_task(&id).unwrap();
    agent
        .load_from_session(JsonlSession::open(&path).unwrap())
        .unwrap();
    assert_eq!(
        agent
            .runtime_for_session()
            .unwrap()
            .task_registry
            .get_task(&id),
        Some(claimed)
    );
    assert!(!worker.cancellation_token.is_cancelled());
    let mut competing = Agent::new("fixture");
    let error = competing
        .load_from_session(JsonlSession::open(&path).unwrap())
        .unwrap_err();
    // A live second writer is not corruption: say where the session is and
    // what to do, not "Runtime recovery required" with a raw OS error.
    assert!(
        error.starts_with(runtime::session::SESSION_IN_USE),
        "{error}"
    );
    assert!(!error.contains("recovery required"), "{error}");
    assert!(!error.contains("os error"), "{error}");
    assert!(competing.session.is_none());
    drop(worker);
    drop(agent);
    competing
        .load_from_session(JsonlSession::open(&path).unwrap())
        .unwrap();
    let recovered = competing
        .runtime_for_session()
        .unwrap()
        .task_registry
        .get_task(&id)
        .unwrap();
    assert_eq!(recovered.state, TaskState::Failed);
}

#[test]
fn switching_to_a_session_held_elsewhere_keeps_the_current_one() {
    let dir = tempfile::tempdir().unwrap();
    let held = JsonlSession::create(dir.path(), "fixture", None).unwrap();
    let held_path = held.path.clone();
    let mut holder = Agent::new("fixture");
    holder.load_from_session(held).unwrap();

    let mut agent = Agent::new("fixture");
    agent
        .load_from_session(JsonlSession::create(dir.path(), "fixture", None).unwrap())
        .unwrap();
    let own_path = agent.session.as_ref().unwrap().path.clone();
    let run = agent.runtime_for_session().unwrap().run_id;

    let error = agent
        .load_from_session(JsonlSession::open(&held_path).unwrap())
        .unwrap_err();
    assert!(
        error.starts_with(runtime::session::SESSION_IN_USE),
        "{error}"
    );
    assert!(error.contains(&held_path.display().to_string()), "{error}");
    assert_eq!(agent.session.as_ref().unwrap().path, own_path);
    assert_eq!(agent.runtime_for_session().unwrap().run_id, run);

    // Once the holder lets go, the same switch succeeds.
    drop(holder);
    agent
        .load_from_session(JsonlSession::open(&held_path).unwrap())
        .unwrap();
    assert_eq!(agent.session.as_ref().unwrap().path, held_path);
}

#[test]
fn f03_corrupt_task_journal_prevents_session_switch() {
    let dir = tempfile::tempdir().unwrap();
    let first = JsonlSession::create(dir.path(), "fixture", None).unwrap();
    let mut agent = Agent::new("fixture");
    agent.load_from_session(first).unwrap();
    let first_path = agent.session.as_ref().unwrap().path.clone();
    let run = agent.runtime_for_session().unwrap().run_id;
    let next = JsonlSession::create(dir.path(), "fixture", None).unwrap();
    let journal = next.path.with_extension("tasks.jsonl");
    let corrupt = b"invalid task journal\n";
    std::fs::write(&journal, corrupt).unwrap();
    let error = agent.load_from_session(next).unwrap_err();
    assert!(
        error.contains("task journal could not be opened"),
        "{error}"
    );
    assert_eq!(agent.session.as_ref().unwrap().path, first_path);
    assert_eq!(agent.runtime_for_session().unwrap().run_id, run);
    assert_eq!(std::fs::read(journal).unwrap(), corrupt);
}

#[test]
fn f03_session_load_activates_durable_tasks_before_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let session = JsonlSession::create(dir.path(), "fixture", None).unwrap();
    let path = session.path.clone();
    let mut agent = Agent::new("fixture");
    agent.load_from_session(session).unwrap();
    assert!(path.with_extension("tasks.jsonl").is_file());
    let runtime = agent.runtime_for_session().unwrap();
    let run = runtime.run_id;
    let id = runtime
        .task_registry
        .create_task(TaskRecord::new(run, "before prompt"))
        .unwrap();
    let expected = runtime.task_registry.get_task(&id).unwrap();
    agent
        .load_from_session(JsonlSession::open(&path).unwrap())
        .unwrap();
    assert_eq!(agent.runtime_for_session().unwrap().run_id, run);
    assert_eq!(
        agent.runtime.as_ref().unwrap().task_registry.get_task(&id),
        Some(expected.clone())
    );
    drop(agent);
    let mut resumed = Agent::new("fixture");
    resumed
        .load_from_session(JsonlSession::open(&path).unwrap())
        .unwrap();
    assert_eq!(resumed.runtime_for_session().unwrap().run_id, run);
    assert_eq!(
        resumed
            .runtime
            .as_ref()
            .unwrap()
            .task_registry
            .get_task(&id),
        Some(expected)
    );
    let events = davinci_session::read_runtime_log::<RuntimeEventEnvelope>(
        &davinci_session::runtime_log_path(&path),
    )
    .unwrap();
    assert_eq!(
        events
            .iter()
            .filter(|event| matches!(event.payload, RuntimeEvent::TaskCreated { .. }))
            .count(),
        1
    );
}

#[test]
fn f03_failed_session_load_preserves_active_state() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = Agent::new("fixture");
    let first = JsonlSession::create(dir.path(), "fixture", None).unwrap();
    let first_path = first.path.clone();
    let runtime = RuntimeHandle::new(RunId::new(), AgentId::new(), RuntimeBus::new())
        .with_session(&first.header.id);
    agent.session = Some(first);
    agent.set_runtime(runtime.clone());
    agent.prompt("keep this prompt");
    let before = serde_json::to_value(&agent.messages).unwrap();
    let next = JsonlSession::create(dir.path(), "fixture", None).unwrap();
    let path = davinci_session::runtime_log_path(&next.path);
    let corrupt = b"{broken record}\n{broken record}\n";
    std::fs::write(&path, corrupt).unwrap();
    assert!(agent.load_from_session(next).is_err());
    assert_eq!(agent.session.as_ref().unwrap().path, first_path);
    assert_eq!(serde_json::to_value(&agent.messages).unwrap(), before);
    assert_eq!(agent.runtime.as_ref().unwrap().run_id, runtime.run_id);
    assert!(agent.runtime_for_session().is_some());
    assert_eq!(std::fs::read(path).unwrap(), corrupt);
}

#[test]
fn f03_session_load_isolates_previous_worker_handles() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = Agent::new("fixture");
    let first = JsonlSession::create(dir.path(), "fixture", None).unwrap();
    let old = RuntimeHandle::new(RunId::new(), AgentId::new(), RuntimeBus::new())
        .with_session(&first.header.id);
    let old_task = old
        .task_registry
        .create_task(TaskRecord::new(old.run_id, "old task"))
        .unwrap();
    agent.session = Some(first);
    agent.set_runtime(old.clone());
    let next = JsonlSession::create(dir.path(), "fixture", None).unwrap();
    let next_id = next.header.id.clone();
    agent.load_from_session(next).unwrap();
    let current = agent.runtime.as_ref().unwrap();
    assert_eq!(current.session_id.as_deref(), Some(next_id.as_str()));
    assert!(current.task_registry.get_task(&old_task).is_none());
    assert!(old.task_registry.get_task(&old_task).is_some());
    assert_ne!(old.session_id, current.session_id);
    old.cancellation_token.cancel();
    assert!(!agent.abort_requested());
    assert_eq!(
        agent.tool_context.runtime.as_ref().unwrap().session_id,
        current.session_id
    );
}

#[test]
fn f03_session_runtime_reuse_checks_file_and_binding_identity() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = Agent::new("fixture");
    let first = JsonlSession::create(dir.path(), "fixture", None).unwrap();
    let first_path = first.path.clone();
    let id = first.header.id.clone();
    agent.session = Some(first);
    let runtime =
        RuntimeHandle::new(RunId::new(), AgentId::new(), RuntimeBus::new()).with_session(&id);
    agent.set_runtime(runtime.clone());
    assert!(agent.runtime_for_session().is_some());
    let mut other = JsonlSession::create(dir.path(), "fixture", None).unwrap();
    other.header.id = id.clone();
    agent.session = Some(other);
    assert!(
        agent.runtime_for_session().is_none(),
        "same ID in a different file is not the same binding"
    );
    agent.session.as_mut().unwrap().path = first_path;
    assert!(agent.runtime_for_session().is_some());
    agent.runtime.as_mut().unwrap().run_id = RunId::new();
    assert!(
        agent.runtime_for_session().is_none(),
        "unbound runtime replacement cannot inherit ownership"
    );
    agent.set_runtime(runtime);
    let session = agent.session.take().unwrap();
    agent.load_from_session(session).unwrap();
    assert!(
        agent.runtime_for_session().is_some(),
        "explicit reload completes reconciliation before returning"
    );
}

#[test]
fn f03_runtime_rebind_refreshes_only_owned_abort_signal() {
    for external in [false, true] {
        let mut agent = Agent::new("runtime binding fixture");
        let old = RuntimeHandle::new(RunId::new(), AgentId::new(), RuntimeBus::new());
        agent.set_runtime(old.clone());
        let ui_signal = Arc::new(std::sync::atomic::AtomicBool::new(false));
        if external {
            agent.abort_signal = Some(ui_signal.clone());
        }
        old.cancellation_token.cancel();
        let next = RuntimeHandle::new(RunId::new(), AgentId::new(), RuntimeBus::new());
        agent.set_runtime(next.clone());
        assert!(
            !agent.abort_requested(),
            "old runtime must not cancel the new binding"
        );
        let expected = if external {
            ui_signal.clone()
        } else {
            next.cancellation_token.as_atomic_bool()
        };
        assert!(Arc::ptr_eq(agent.abort_signal.as_ref().unwrap(), &expected));
        assert_eq!(agent.runtime.as_ref().unwrap().run_id, next.run_id);
        assert_eq!(
            agent.tool_context.runtime.as_ref().unwrap().run_id,
            next.run_id
        );
        if external {
            ui_signal.store(true, std::sync::atomic::Ordering::SeqCst);
            assert!(agent.abort_requested(), "UI cancellation remains effective");
            ui_signal.store(false, std::sync::atomic::Ordering::SeqCst);
        }
        next.cancellation_token.cancel();
        assert!(
            agent.abort_requested(),
            "new runtime cancellation remains effective"
        );
        agent.set_runtime(next);
        assert!(
            agent.abort_requested(),
            "rebinding the same runtime cannot clear cancellation"
        );
    }
}

#[test]
fn a_chat_entry_keeps_the_message_extras() {
    let mut extra = serde_json::Map::new();
    extra.insert("customType".into(), serde_json::json!(JOB_NOTICE_TYPE));
    extra.insert("jobId".into(), serde_json::json!(3));
    let entry = chat_entry("user", serde_json::json!("hi"), &extra);
    let message = entry.message.unwrap();
    assert_eq!(message["role"], "user");
    assert_eq!(message["content"], "hi");
    assert_eq!(message["customType"], JOB_NOTICE_TYPE);
    assert_eq!(message["jobId"], 3);
}

#[test]
fn queues_and_compaction_match_ts_modes() {
    let mut agent = Agent::new(default_system_prompt());
    agent.prompt(&"aaaaaaaaaa bbbbbbbbbb ".repeat(80));
    agent.record_assistant(&"cccccccccc ".repeat(400));
    agent.prompt("go on");
    agent.queues.enqueue_steer("steer me");
    agent.queues.enqueue_follow_up("follow");
    assert_eq!(agent.queues.steer.len(), 1);
    let drained = agent.queues.drain_steer(QueueMode::OneAtATime);
    assert_eq!(drained.len(), 1);
    let compacted = agent.compact(Some("keep decisions"));
    assert!(compacted.summary.contains("keep decisions"));
    assert!(!agent.messages.is_empty());
}

#[test]
fn compaction_emits_pre_and_post_compact_on_runtime() {
    use crate::runtime::{
        bus::RuntimeSubscriber, AgentId, RunId, RuntimeBus, RuntimeDecision, RuntimeEvent,
        RuntimeEventEnvelope, RuntimeHandle,
    };
    use std::sync::Mutex;

    #[derive(Default)]
    struct EventCollector {
        events: Mutex<Vec<RuntimeEvent>>,
    }
    impl RuntimeSubscriber for EventCollector {
        fn on_event(&self, event: &RuntimeEventEnvelope) -> RuntimeDecision {
            self.events.lock().unwrap().push(event.payload.clone());
            RuntimeDecision::Continue
        }
    }

    let bus = RuntimeBus::new();
    let collector = Arc::new(EventCollector::default());
    bus.subscribe(collector.clone());

    let runtime = RuntimeHandle::new(RunId::new(), AgentId::new(), bus);
    let mut agent = Agent::new("base prompt").with_runtime(runtime);

    agent.prompt(&"message 1 for conversation history ".repeat(60));
    agent.record_assistant(&"assistant response 1 ".repeat(60));
    agent.prompt("message 2 for conversation history");
    agent.record_assistant("assistant response 2");

    let result = agent.compact(Some("custom summary"));
    assert!(result.compacted);

    let events = collector.events.lock().unwrap().clone();
    let has_pre = events.iter().any(
        |e| matches!(e, RuntimeEvent::PreCompact { estimated_tokens } if *estimated_tokens > 0),
    );
    let has_post = events.iter().any(
            |e| matches!(e, RuntimeEvent::PostCompact { before_tokens, after_tokens } if *before_tokens > 0 && *after_tokens > 0),
        );

    assert!(has_pre, "Must emit PreCompact event with estimated tokens");
    assert!(
        has_post,
        "Must emit PostCompact event with before and after tokens"
    );
}

#[test]
fn plan_mode_has_one_authoritative_permission_state() {
    let mut agent = Agent::new("base prompt");
    agent.permissions.lock().unwrap().mode = PermissionMode::Edits;
    agent.set_plan_mode(true);
    assert_eq!(
        agent.permissions.lock().unwrap().mode,
        PermissionMode::ReadOnly
    );
    assert!(agent.system_prompt.contains(crate::PLAN_MODE_APPENDIX));
    agent.set_plan_mode(true);
    agent.set_plan_mode(false);
    assert_eq!(
        agent.permissions.lock().unwrap().mode,
        PermissionMode::Edits
    );
    assert_eq!(agent.system_prompt, "base prompt");
}

#[test]
fn update_plan_accepts_legacy_steps_without_approving_structured_plan() {
    let context = ToolContext::default();
    let result = execute_tool_with(
            Path::new("."), "update_plan",
            &serde_json::json!({"plan": [{"step": "Inspect parser", "status": "in_progress"}], "explanation": "Start from source"}),
            &context,
        ).unwrap();
    assert!(
        result.content.contains("Inspect parser"),
        "{}",
        result.content
    );
    assert_eq!(context.todos.lock().unwrap().items.len(), 1);
    assert_eq!(context.living_plan.lock().unwrap().approved_revision, None);
}

#[test]
fn system_prompt_reset_restores_the_base_prompt() {
    let mut agent = Agent::new("base prompt");
    agent.system_prompt = "extension override".into();

    agent.reset_system_prompt_to_base();

    assert_eq!(agent.system_prompt, "base prompt");
}

#[test]
fn agent_new_compatibility_distinguishes_custom_and_legacy() {
    let custom = Agent::new("custom prompt");
    assert!(custom.prompt_session.is_custom());
    assert!(custom.prompt_manifest.is_none());
    assert_eq!(custom.system_prompt, "custom prompt");

    let legacy = Agent::new(default_system_prompt());
    assert!(legacy.prompt_session.is_builtin());
    assert_eq!(
        legacy.prompt_session.profile(),
        Some(prompt::PromptProfile::LegacyV1)
    );
    assert!(legacy.prompt_manifest.is_some());
    assert_eq!(
        legacy.prompt_manifest.as_ref().unwrap().profile,
        "legacy-v1"
    );
}

#[test]
fn agent_new_builtin_activates_specified_profile() {
    let agent = Agent::new_builtin(prompt::PromptProfile::Stable);
    assert!(agent.prompt_session.is_builtin());
    assert_eq!(
        agent.prompt_session.profile(),
        Some(prompt::PromptProfile::Stable)
    );
    assert!(agent.prompt_manifest.is_some());
    let manifest = agent.prompt_manifest.as_ref().unwrap();
    assert_eq!(manifest.profile, "stable");
    assert_eq!(
        agent.prompt_session.stable_bundle_hash.as_deref(),
        Some(manifest.stable_sha256.as_str())
    );
    assert!(agent.system_prompt.contains("DaVinci"));
}

#[test]
fn prompt_session_identity_persists_and_resumes_across_session_reload() {
    let dir = tempfile::tempdir().unwrap();
    let session = JsonlSession::create(dir.path(), "fixture", None).unwrap();
    let path = session.path.clone();

    let mut agent = Agent::new_builtin(prompt::PromptProfile::Stable);
    agent.session = Some(session);
    agent.persist_prompt_session().unwrap();
    let entry_count = agent.session.as_ref().unwrap().entries.len();
    agent.persist_prompt_session().unwrap();
    assert_eq!(agent.session.as_ref().unwrap().entries.len(), entry_count);

    let mut resumed = Agent::new("placeholder");
    resumed
        .load_from_session(JsonlSession::open(&path).unwrap())
        .unwrap();

    assert!(resumed.prompt_session.is_builtin());
    assert_eq!(
        resumed.prompt_session.profile(),
        Some(prompt::PromptProfile::Stable)
    );
    let manifest = resumed.prompt_manifest.as_ref().expect("manifest");
    assert_eq!(manifest.profile, "stable");
    assert_eq!(manifest.profile_version, 2);
}

#[test]
fn real_user_routing_history_survives_resume_without_mailbox_contamination() {
    let dir = tempfile::tempdir().unwrap();

    let session = JsonlSession::create(dir.path(), "routing-history", None).unwrap();
    let path = session.path.clone();
    let mut agent = Agent::new_builtin(prompt::PromptProfile::Stable);
    agent.session = Some(session);
    agent.prompt_user_with(
        "Redesign the UI dashboard layout with visual hierarchy.",
        &[],
    );
    agent.prompt_with("Internal mailbox note: routine status only.", &[]);

    let mut resumed = Agent::new_builtin(prompt::PromptProfile::Stable);
    resumed
        .load_from_session(JsonlSession::open(&path).unwrap())
        .unwrap();
    resumed.prompt_user_with("cards", &[]);
    assert!(
        resumed.system_prompt.contains("frontend_design_policy"),
        "the last persisted real user request must remain eligible history after resume"
    );

    let isolated = JsonlSession::create(dir.path(), "mailbox-isolation", None).unwrap();
    let isolated_path = isolated.path.clone();
    let mut agent = Agent::new_builtin(prompt::PromptProfile::Stable);
    agent.session = Some(isolated);
    agent.prompt_user_with("Say hello.", &[]);
    agent.prompt_with(
        "Redesign the UI dashboard layout with visual hierarchy and cards.",
        &[],
    );

    let mut resumed = Agent::new_builtin(prompt::PromptProfile::Stable);
    resumed
        .load_from_session(JsonlSession::open(&isolated_path).unwrap())
        .unwrap();
    resumed.prompt_user_with("cards", &[]);
    assert!(
        !resumed.system_prompt.contains("frontend_design_policy"),
        "unmarked internal mailbox text must remain ineligible routing history after resume"
    );
}

#[test]
fn internal_prompts_are_never_goals_or_mandatory() {
    use crate::runtime::context_manifest::ProvenanceKind;
    use crate::runtime::context_vm::ContextEventKind;
    let mut agent = Agent::new("base");
    agent.prompt_user_with("Fix the parser.", &[]);
    agent.prompt_internal("teammate says: done", InternalPromptOrigin::ToolEvidence);
    agent.prompt_internal("schema errors: /a", InternalPromptOrigin::AgentInference);
    agent.prompt_with("host steering", &[]);
    let events = agent.context_vm_events_for_test();
    let find = |needle: &str| {
        events
            .iter()
            .find(|event| event.visible_text.contains(needle))
            .unwrap_or_else(|| panic!("{needle}: {events:#?}"))
    };
    assert_eq!(find("Fix the parser").kind, ContextEventKind::User);
    let mailbox = find("teammate says");
    assert_eq!(mailbox.kind, ContextEventKind::Custom);
    assert_eq!(mailbox.provenance_kind, ProvenanceKind::ToolEvidence);
    let repair = find("schema errors");
    assert_eq!(repair.kind, ContextEventKind::Custom);
    assert_eq!(repair.provenance_kind, ProvenanceKind::AgentInference);
    // Unmarked host steering keeps its meaning: a person's words.
    assert_eq!(find("host steering").kind, ContextEventKind::User);
    // The marker is an extra field: the provider still sees plain user text.
    let provider = agent.messages_for_provider();
    assert!(
        provider
            .iter()
            .any(|m| m.role == "user"
                && davinci_ai::content_text(&m.content) == "teammate says: done")
    );
}

#[test]
fn prompt_session_explicit_legacy_pin_preserved_on_resume() {
    let dir = tempfile::tempdir().unwrap();
    let session = JsonlSession::create(dir.path(), "fixture", None).unwrap();
    let path = session.path.clone();

    let mut agent = Agent::new_builtin(prompt::PromptProfile::LegacyV1);
    agent.session = Some(session);
    agent.persist_prompt_session().unwrap();

    let mut resumed = Agent::new_builtin(prompt::PromptProfile::Stable);
    resumed
        .load_from_session(JsonlSession::open(&path).unwrap())
        .unwrap();

    assert_eq!(
        resumed.prompt_session.profile(),
        Some(prompt::PromptProfile::LegacyV1)
    );
    let manifest = resumed.prompt_manifest.as_ref().expect("manifest");
    assert_eq!(manifest.profile, "legacy-v1");
    assert_eq!(manifest.profile_version, 1);
}

#[test]
fn context_budget_counts_system_and_active_tool_schemas() {
    let mut agent = Agent::new("");
    agent.tools.clear();
    agent.expose_active_tools();
    let empty = agent.estimated_context_tokens();
    agent.system_prompt = "x".repeat(4_000);
    assert!(agent.estimated_context_tokens() >= empty + 1_000);
    let without_tools = agent.estimated_context_tokens();
    agent.tools.push("read".into());
    agent.expose_active_tools();
    assert!(agent.estimated_context_tokens() > without_tools);
    agent.tools.clear();
    agent.expose_active_tools();
    assert_eq!(agent.estimated_context_tokens(), without_tools);
    agent.set_provider_context_overhead_tokens(Some(3_000));
    assert_eq!(agent.estimated_context_tokens(), 4_000);
    agent.set_provider_context_overhead_tokens(None);
    assert_eq!(agent.estimated_context_tokens(), without_tools);
}

#[test]
fn a_host_schema_estimator_counts_tools_like_the_rest_of_the_estimate() {
    // Hosts register a byte-ceiling estimator for admission; the compaction
    // estimate must still weigh the schemas at four bytes a token.
    let mut agent = Agent::new("");
    let schema_bytes = serde_json::to_vec(&agent.provider_tool_specs())
        .unwrap()
        .len() as u64;
    let heuristic = agent.estimated_context_tokens();
    agent.set_provider_context_overhead_estimator(|agent| {
        serde_json::to_vec(&agent.provider_tool_specs())
            .unwrap()
            .len() as u64
            + 128
    });
    assert_eq!(
        agent.estimated_context_tokens(),
        heuristic - schema_bytes.div_ceil(4) + (schema_bytes + 128).div_ceil(4)
    );
    // The admission budget keeps its conservative byte ceiling.
    assert_eq!(agent.provider_context_budget().tools, schema_bytes + 128);
}

#[test]
fn context_budget_counts_only_provider_visible_tool_schemas() {
    let agent = Agent::new("");
    let provider_schema_tokens = (serde_json::to_vec(&agent.provider_tool_specs())
        .unwrap()
        .len() as u64)
        .div_ceil(4);

    assert_eq!(agent.estimated_context_tokens(), provider_schema_tokens);
}

#[test]
fn context_manifest_describes_only_provider_visible_tool_schemas() {
    let mut agent = Agent::new("");
    let provider_schema = serde_json::to_string(&agent.provider_tool_schema_value()).unwrap();
    let expected_tokens = (provider_schema.len() as u64).div_ceil(4);
    let expected_hash =
        runtime::context_manifest::ContextManifestEntry::hash_content(&provider_schema);

    let manifest = agent.prepare_context_manifest("request", RunId::new(), 1, 1);
    let tool_schemas = manifest
        .entries
        .iter()
        .find(|entry| entry.id == "tool_schemas")
        .unwrap();

    assert_eq!(tool_schemas.token_estimate, expected_tokens);
    assert_eq!(tool_schemas.content_hash, expected_hash);
}

#[test]
fn root_context_report_includes_provider_system_suffix() {
    let mut agent = Agent::new("base");
    agent.set_provider_system_prompt_suffix(Some("runtime identity".into()));

    let report = agent.root_context_budget_report(u64::MAX);
    let suffix = report
        .contributions
        .iter()
        .find(|entry| entry.source == "provider_system_prompt_suffix")
        .unwrap();

    assert!(!suffix.stable);
    assert_eq!(suffix.priority, ContextPriority::Mandatory);
    assert!(suffix.selected);
}

#[test]
fn root_context_report_sizes_skills_from_metadata_without_reading_bodies() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("SKILL.md");
    // Not UTF-8: reading the body fails and would answer with the tiny inline
    // text, so only the file's size can explain the estimate.
    std::fs::write(&path, vec![0xff_u8; 4_000]).unwrap();
    let mut agent = Agent::new("base");
    agent.skills.push(crate::Skill {
        name: "big".into(),
        path,
        description: String::new(),
        body: "tiny".into(),
        base_dir: dir.path().to_path_buf(),
        namespace: None,
    });

    let report = agent.root_context_budget_report(u64::MAX);
    let skill = report
        .contributions
        .iter()
        .find(|entry| entry.source == "skill::big")
        .unwrap();
    assert_eq!(skill.estimated_tokens, 1_000);
    assert_eq!(skill.priority, ContextPriority::Deferred);
}

#[test]
fn context_overhead_triggers_pruning_before_the_provider_request() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = Agent::new("x".repeat(24_000));
    agent.tools.clear();
    agent.tools.push("read".into());
    agent.evidence = Some(EvidenceStore::new(dir.path()));
    agent.context_window = 10_000;
    agent.prune_settings.keep_recent = 0;
    let mut result = ChatMessage::text("toolResult", "output".repeat(1_000));
    result.tool_call_id = Some("old-read".into());
    agent.messages.push(result);
    agent.prune_context();
    assert!(agent.pruned_tool_results().contains("old-read"));
    assert_eq!(agent.stats.pruned_results, 1);
}

#[test]
fn pruning_requires_readable_evidence_and_recovers_after_expiry() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = Agent::new("");
    agent.context_window = 100;
    agent.prune_settings.keep_recent = 0;
    let original = ChatMessage::tool_result("effect", "bash", "done".repeat(2000), false);
    agent.messages.push(original.clone());
    agent.prune_context();
    assert!(agent.pruned_tool_results().is_empty());
    agent.evidence = Some(EvidenceStore::new(dir.path()));
    agent.tools.retain(|tool| tool != "read");
    agent.prune_context();
    assert!(agent.pruned_tool_results().is_empty());
    agent.tools.push("read".into());
    agent.prune_context();
    assert!(agent.pruned_tool_results().contains("effect"));
    let path = std::fs::read_dir(dir.path())
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        content_text(&original.content)
    );
    assert!(content_text(&agent.messages_for_provider()[0].content)
        .contains(&path.display().to_string()));
    let reduced = agent.estimated_context_tokens();
    std::fs::remove_file(path).unwrap();
    assert_eq!(agent.messages_for_provider()[0].content, original.content);
    assert!(agent.estimated_context_tokens() > reduced);
}

#[test]
fn evidence_write_failure_keeps_recoverable_context() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("not-a-directory");
    std::fs::write(&file, "occupied").unwrap();
    let mut agent = Agent::new("");
    agent.evidence = Some(EvidenceStore::new(file));
    agent.context_window = 100;
    agent.prune_settings.keep_recent = 0;
    let original = ChatMessage::tool_result("effect", "bash", "done".repeat(2000), false);
    agent.messages.push(original.clone());
    agent.prune_context();
    assert!(agent.pruned_tool_results().is_empty());
    assert_eq!(agent.messages_for_provider()[0].content, original.content);
    assert_eq!(agent.run_stats().evidence_files, 0);
}

#[test]
fn revoked_evidence_permission_restores_original_output() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = Agent::new("");
    agent.evidence = Some(EvidenceStore::new(dir.path()));
    agent.context_window = 100;
    agent.prune_settings.keep_recent = 0;
    let original = ChatMessage::tool_result("effect", "bash", "done".repeat(2000), true);
    agent.messages.push(original.clone());
    agent.prune_context();
    assert!(agent.pruned_tool_results().contains("effect"));
    agent.prune_context();
    assert_eq!(agent.run_stats().evidence_files, 1);
    let projected = agent.messages_for_provider();
    assert!(content_text(&projected[0].content).contains("outcome error"));
    agent
        .permissions
        .lock()
        .unwrap()
        .deny
        .push(PermissionRule::bare("read"));
    assert_eq!(agent.messages_for_provider()[0].content, original.content);
}

#[test]
fn retry_cancellation_during_backoff_prevents_another_request() {
    use davinci_ai::AssistantMessage;
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    };
    let signal = Arc::new(AtomicBool::new(false));
    let mut agent = Agent::new("");
    agent.abort_signal = Some(signal.clone());
    agent.retry_attempts = 2;
    agent.retry_base_delay_ms = 5_000;
    agent.prompt("retry cancellation");
    let mut calls = 0;
    let mut interrupter = None;
    let started = std::time::Instant::now();
    agent
        .run_loop(|_| -> Result<AssistantMessage, String> {
            calls += 1;
            if calls == 1 {
                let signal = signal.clone();
                interrupter = Some(std::thread::spawn(move || {
                    std::thread::sleep(std::time::Duration::from_millis(50));
                    signal.store(true, Ordering::Relaxed);
                }));
            }
            Err("overloaded_error".into())
        })
        .unwrap();
    interrupter.unwrap().join().unwrap();
    assert_eq!(calls, 1);
    assert_eq!(agent.stats.provider_retries, 0);
    assert!(started.elapsed() < std::time::Duration::from_secs(2));
}

#[test]
fn retry_never_repeats_a_permanent_request_failure() {
    let mut agent = Agent::new("");
    agent.retry_base_delay_ms = 0;
    agent.prompt("bad request");
    let mut calls = 0;
    let result = agent.run_loop(|_| -> Result<davinci_ai::AssistantMessage, String> {
        calls += 1;
        Err("400 bad request".into())
    });
    assert!(result.is_err());
    assert_eq!(calls, 1);
    assert_eq!(agent.stats.provider_retries, 0);
}

#[test]
fn retry_aborted_provider_response_is_not_reported_as_success() {
    let mut agent = Agent::new("");
    agent.retry_base_delay_ms = 0;
    agent.prompt("cancel recovery");
    let mut calls = 0;
    let events = agent
        .run_loop(|_| {
            calls += 1;
            if calls == 1 {
                return Err("overloaded_error".into());
            }
            Ok(davinci_ai::AssistantMessage {
                extra: Default::default(),
                id: "abort".into(),
                role: "assistant".into(),
                content: Vec::new(),
                model: "fixture".into(),
                usage: None,
                stop_reason: Some(davinci_ai::StopReason::Aborted),
                error_message: Some("cancelled".into()),
            })
        })
        .unwrap();
    assert!(events
        .iter()
        .any(|event| matches!(event, AgentEvent::AutoRetryEnd { success: false, .. })));
    assert_eq!(agent.stats.provider_retries, 1);
}

#[test]
fn auto_retry_emits_ts_session_events() {
    use davinci_ai::{AssistantMessage, ContentBlock, StopReason};

    let mut agent = Agent::new(default_system_prompt());
    agent.retry_attempts = 2;
    agent.retry_base_delay_ms = 0;
    agent.prompt("retry me");
    let mut calls = 0;
    let events = agent
        .run_loop(|_| {
            calls += 1;
            if calls == 1 {
                return Ok(AssistantMessage {
                    extra: Default::default(),
                    id: "e1".into(),
                    role: "assistant".into(),
                    content: Vec::new(),
                    model: "fixture".into(),
                    usage: None,
                    stop_reason: Some(StopReason::Error),
                    error_message: Some("overloaded_error".into()),
                });
            }
            Ok(AssistantMessage {
                extra: Default::default(),
                id: "ok".into(),
                role: "assistant".into(),
                content: vec![ContentBlock::Text {
                    text: "recovered".into(),
                }],
                model: "fixture".into(),
                usage: None,
                stop_reason: Some(StopReason::Stop),
                error_message: None,
            })
        })
        .unwrap();
    assert_eq!(agent.stats.provider_retries, 1);
    let kinds: Vec<_> = events.iter().map(AgentEvent::kind).collect();
    assert!(kinds.contains(&"auto_retry_start"));
    assert!(kinds.contains(&"auto_retry_end"));
    assert_eq!(calls, 2);
    assert_eq!(agent.last_assistant_text().as_deref(), Some("recovered"));
}

#[test]
fn a_closure_that_streamed_live_is_recorded_but_not_resent_to_the_sink() {
    use davinci_ai::{AssistantMessage, AssistantMessageEvent, ContentBlock, StopReason};
    use std::sync::{Arc, Mutex};

    let seen: Arc<Mutex<Vec<&'static str>>> = Arc::new(Mutex::new(Vec::new()));
    let sink_seen = seen.clone();
    let mut agent = Agent::new(default_system_prompt());
    agent.event_sink = Some(EventSink(Arc::new(move |event: &AgentEvent| {
        sink_seen.lock().unwrap().push(event.kind());
    })));
    agent.prompt("stream me");
    let events = agent
        .run_loop(|current| {
            let message = AssistantMessage {
                extra: Default::default(),
                id: "live".into(),
                role: "assistant".into(),
                content: vec![ContentBlock::Text {
                    text: "hi there".into(),
                }],
                model: "fixture".into(),
                usage: None,
                stop_reason: Some(StopReason::Stop),
                error_message: None,
            };
            let stream_events = davinci_ai::events_from_complete(&message);
            let chat = std::sync::Arc::new(davinci_ai::assistant_to_chat(&message));
            current.emit_live(AgentEvent::MessageStart {
                message: (*chat).clone(),
            });
            for event in &stream_events {
                current.emit_live(AgentEvent::MessageUpdate {
                    message: chat.clone(),
                    assistant_message_event: event.clone(),
                });
            }
            let _: &AssistantMessageEvent = &stream_events[0];
            Ok(CompleteOutput {
                message,
                stream_events: Some(stream_events),
                native_responses_resume: None,
                streamed_live: true,
            })
        })
        .unwrap();
    let recorded: Vec<_> = events.iter().map(AgentEvent::kind).collect();
    let sunk = seen.lock().unwrap().clone();
    // The record and the sink saw the same sequence, once each: no
    // update was sent twice and none was dropped.
    assert_eq!(recorded, sunk);
    assert_eq!(
        recorded
            .iter()
            .filter(|kind| **kind == "message_start")
            .count(),
        2 // the user prompt and the assistant reply
    );
    assert_eq!(
        recorded
            .iter()
            .filter(|kind| **kind == "message_update")
            .count(),
        stream_events_len("hi there")
    );
    assert_eq!(agent.last_assistant_text().as_deref(), Some("hi there"));
}

fn stream_events_len(text: &str) -> usize {
    use davinci_ai::{AssistantMessage, ContentBlock, StopReason};
    davinci_ai::events_from_complete(&AssistantMessage {
        extra: Default::default(),
        id: "n".into(),
        role: "assistant".into(),
        content: vec![ContentBlock::Text { text: text.into() }],
        model: "fixture".into(),
        usage: None,
        stop_reason: Some(StopReason::Stop),
        error_message: None,
    })
    .len()
}

#[test]
fn agent_loop_emits_ts_event_names_and_runs_tools() {
    use davinci_ai::{AssistantMessage, ContentBlock, StopReason};

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("note.txt"), "hello").unwrap();
    let mut agent = Agent::new(default_system_prompt());
    agent.cwd = dir.path().to_path_buf();
    agent.prompt("read the note");
    let events = agent
        .run_loop(|current| {
            if current.messages.iter().any(|m| m.role == "toolResult") {
                return Ok(AssistantMessage {
                    extra: Default::default(),
                    id: "a2".into(),
                    role: "assistant".into(),
                    content: vec![ContentBlock::Text {
                        text: "done".into(),
                    }],
                    model: "fixture".into(),
                    usage: None,
                    stop_reason: Some(StopReason::Stop),
                    error_message: None,
                });
            }
            Ok(AssistantMessage {
                extra: Default::default(),
                id: "a1".into(),
                role: "assistant".into(),
                content: vec![ContentBlock::ToolCall {
                    id: "call_1".into(),
                    name: "read".into(),
                    arguments: serde_json::json!({"path": "note.txt"}),
                }],
                model: "fixture".into(),
                usage: None,
                stop_reason: Some(StopReason::ToolUse),
                error_message: None,
            })
        })
        .unwrap();
    let kinds: Vec<_> = events.iter().map(AgentEvent::kind).collect();
    assert_eq!(kinds.first().copied(), Some("agent_start"));
    assert!(kinds.contains(&"tool_execution_start"));
    assert!(kinds.contains(&"tool_execution_end"));
    assert!(kinds.contains(&"message_update"));
    let update_types: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            AgentEvent::MessageUpdate {
                assistant_message_event,
                ..
            } => Some(match assistant_message_event {
                davinci_ai::AssistantMessageEvent::TextDelta { .. } => "text_delta",
                davinci_ai::AssistantMessageEvent::ThinkingDelta { .. } => "thinking_delta",
                davinci_ai::AssistantMessageEvent::ToolcallStart { .. } => "toolcall_start",
                davinci_ai::AssistantMessageEvent::ToolcallEnd { .. } => "toolcall_end",
                _ => "other",
            }),
            _ => None,
        })
        .collect();
    assert!(update_types.contains(&"toolcall_start"));
    assert!(update_types.contains(&"text_delta"));
    assert_eq!(kinds.last().copied(), Some("agent_end"));
    assert_eq!(agent.last_assistant_text().as_deref(), Some("done"));
}

/// A scripted model: one tool call per entry, then `done`.
fn scripted_tool_calls(
    calls: Vec<(&'static str, serde_json::Value)>,
) -> impl FnMut(&Agent) -> Result<davinci_ai::AssistantMessage, String> {
    use davinci_ai::{AssistantMessage, ContentBlock, StopReason};
    let mut remaining = calls.into_iter();
    let mut index = 0;
    move |_current| {
        index += 1;
        match remaining.next() {
            Some((name, arguments)) => Ok(AssistantMessage {
                extra: Default::default(),
                id: format!("a{index}"),
                role: "assistant".into(),
                content: vec![ContentBlock::ToolCall {
                    id: format!("call_{index}"),
                    name: name.into(),
                    arguments,
                }],
                model: "fixture".into(),
                usage: None,
                stop_reason: Some(StopReason::ToolUse),
                error_message: None,
            }),
            None => Ok(AssistantMessage {
                extra: Default::default(),
                id: format!("a{index}"),
                role: "assistant".into(),
                content: vec![ContentBlock::Text {
                    text: "done".into(),
                }],
                model: "fixture".into(),
                usage: None,
                stop_reason: Some(StopReason::Stop),
                error_message: None,
            }),
        }
    }
}

fn tool_outcomes(events: &[AgentEvent]) -> Vec<(String, bool, String)> {
    events
        .iter()
        .filter_map(|event| match event {
            AgentEvent::ToolExecutionEnd {
                tool_name,
                is_error,
                result,
                ..
            } => Some((
                tool_name.clone(),
                *is_error,
                result.as_str().unwrap_or_default().to_string(),
            )),
            _ => None,
        })
        .collect()
}

/// A scripted model that emits every call of one entry in a single
/// assistant message, then `done`.
fn scripted_batches(
    batches: Vec<Vec<(&'static str, serde_json::Value)>>,
) -> impl FnMut(&Agent) -> Result<davinci_ai::AssistantMessage, String> {
    use davinci_ai::{AssistantMessage, ContentBlock, StopReason};
    let mut remaining = batches.into_iter();
    let mut index = 0;
    move |_current| {
        index += 1;
        match remaining.next() {
            Some(calls) => Ok(AssistantMessage {
                extra: Default::default(),
                id: format!("a{index}"),
                role: "assistant".into(),
                content: calls
                    .into_iter()
                    .enumerate()
                    .map(|(n, (name, arguments))| ContentBlock::ToolCall {
                        id: format!("call_{index}_{n}"),
                        name: name.into(),
                        arguments,
                    })
                    .collect(),
                model: "fixture".into(),
                usage: None,
                stop_reason: Some(StopReason::ToolUse),
                error_message: None,
            }),
            None => Ok(AssistantMessage {
                extra: Default::default(),
                id: format!("a{index}"),
                role: "assistant".into(),
                content: vec![ContentBlock::Text {
                    text: "done".into(),
                }],
                model: "fixture".into(),
                usage: None,
                stop_reason: Some(StopReason::Stop),
                error_message: None,
            }),
        }
    }
}

fn truncated_call_response(index: usize) -> davinci_ai::AssistantMessage {
    let mut extra = serde_json::Map::new();
    extra.insert(
        davinci_ai::DROPPED_TOOL_CALLS_KEY.into(),
        serde_json::Value::Bool(true),
    );
    davinci_ai::AssistantMessage {
        extra,
        id: format!("a{index}"),
        role: "assistant".into(),
        content: Vec::new(),
        model: "fixture".into(),
        usage: None,
        stop_reason: Some(davinci_ai::StopReason::Length),
        error_message: None,
    }
}

#[test]
fn output_limit_truncated_call_gets_one_corrective_turn() {
    let mut agent = Agent::new(default_system_prompt());
    agent.prompt("write a large file");
    let mut calls = 0;
    agent
        .run_loop(|_current| {
            calls += 1;
            if calls == 1 {
                return Ok(truncated_call_response(calls));
            }
            Ok(davinci_ai::AssistantMessage {
                extra: Default::default(),
                id: format!("a{calls}"),
                role: "assistant".into(),
                content: vec![davinci_ai::ContentBlock::Text {
                    text: "done".into(),
                }],
                model: "fixture".into(),
                usage: None,
                stop_reason: Some(davinci_ai::StopReason::Stop),
                error_message: None,
            })
        })
        .unwrap();
    assert_eq!(calls, 2, "the model must get a turn to retry smaller");
    assert!(agent.messages.iter().any(|message| {
        message.extra.get("davinciCapabilityReminder")
            == Some(&serde_json::Value::String("output_truncated".into()))
    }));
    assert!(agent.messages.iter().all(|message| !message
        .extra
        .contains_key(davinci_ai::DROPPED_TOOL_CALLS_KEY)));
    assert_eq!(agent.last_assistant_text().as_deref(), Some("done"));
}

#[test]
fn repeated_output_limit_truncation_ends_the_run() {
    let mut agent = Agent::new(default_system_prompt());
    agent.prompt("write a large file");
    let mut calls = 0;
    agent
        .run_loop(|_current| {
            calls += 1;
            Ok(truncated_call_response(calls))
        })
        .unwrap();
    assert_eq!(calls, 3, "one initial request plus two bounded retries");
}

#[test]
fn parallel_workers_in_one_message_overlap_and_answer_in_order() {
    use std::sync::{Condvar, Mutex};
    use std::time::Duration;
    let mut agent = Agent::new(default_system_prompt());
    agent.set_permission_mode(PermissionMode::Ask);
    agent.approver = Some(ToolApprover(Arc::new(|_| ToolApprovalDecision::AllowOnce)));
    // Rendezvous instead of a wall-clock budget: each worker waits until all
    // three are inside the runner. Concurrent workers meet at once; workers
    // run one after another never do, and the first one times out. A timing
    // threshold failed on loaded runners even when the workers did overlap.
    let arrived = Arc::new((Mutex::new(0usize), Condvar::new()));
    let gate = Arc::clone(&arrived);
    agent.subagent_runner = Some(SubagentRunner::new(move |req| {
        let (count, all_in) = &*gate;
        let mut count = count.lock().unwrap();
        *count += 1;
        all_in.notify_all();
        let (count, timeout) = all_in
            .wait_timeout_while(count, Duration::from_secs(30), |count| *count < 3)
            .unwrap();
        if timeout.timed_out() {
            return Ok(format!("serial:{}:{}", req.prompt, *count));
        }
        Ok(format!("answer:{}", req.prompt))
    }));
    agent.prompt("go");
    let events = agent
        .run_loop(scripted_batches(vec![vec![
            ("agent", serde_json::json!({"prompt": "one"})),
            ("agent", serde_json::json!({"prompt": "two"})),
            ("agent", serde_json::json!({"prompt": "three"})),
        ]]))
        .unwrap();
    let results: Vec<String> = agent
        .messages
        .iter()
        .filter(|message| message.role == "toolResult")
        .map(|message| content_text(&message.content))
        .collect();
    assert_eq!(results.len(), 3);
    for (result, expected) in results
        .iter()
        .zip(["answer:one", "answer:two", "answer:three"])
    {
        assert!(
            result.starts_with(expected),
            "expected {expected}, got {result}"
        );
        assert!(
            result.contains("[verification receipt] status: unavailable"),
            "a worker with no host observation must be unavailable: {result}"
        );
    }
    // Every start precedes every end, and the ends are recorded in
    // source order even though the workers finished together.
    let kinds: Vec<(&str, String)> = events
        .iter()
        .filter_map(|event| match event {
            AgentEvent::ToolExecutionStart { tool_call_id, .. } => {
                Some(("start", tool_call_id.clone()))
            }
            AgentEvent::ToolExecutionEnd { tool_call_id, .. } => {
                Some(("end", tool_call_id.clone()))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        kinds,
        [
            ("start", "call_1_0".to_string()),
            ("start", "call_1_1".to_string()),
            ("start", "call_1_2".to_string()),
            ("end", "call_1_0".to_string()),
            ("end", "call_1_1".to_string()),
            ("end", "call_1_2".to_string()),
        ]
    );
    let stats = agent.run_stats();
    assert_eq!(stats.model_turns, 2);
    assert_eq!(stats.tool_batches, 1);
    assert_eq!(stats.tool_calls, 3);
    assert_eq!(stats.max_batch_width, 3);
    assert_eq!(stats.parallel_groups, 1);
    assert_eq!(stats.subagents, 3);
}

#[test]
fn sequential_mode_runs_workers_one_at_a_time() {
    use std::time::{Duration, Instant};
    let mut agent = Agent::new(default_system_prompt());
    agent.tool_execution_mode = ToolExecutionMode::Sequential;
    agent.subagent_runner = Some(SubagentRunner::new(|_| {
        std::thread::sleep(Duration::from_millis(40));
        Ok("x".into())
    }));
    agent.prompt("go");
    let start = Instant::now();
    agent
        .run_loop(scripted_batches(vec![vec![
            ("agent", serde_json::json!({"prompt": "one"})),
            ("agent", serde_json::json!({"prompt": "two"})),
        ]]))
        .unwrap();
    assert!(start.elapsed() >= Duration::from_millis(80));
    assert_eq!(agent.run_stats().parallel_groups, 0);
}

#[test]
fn an_edit_is_a_barrier_so_the_read_after_it_sees_the_change() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "before").unwrap();
    let mut agent = Agent::new(default_system_prompt());
    agent.cwd = dir.path().to_path_buf();
    agent.prompt("go");
    agent
        .run_loop(scripted_batches(vec![vec![
            ("read", serde_json::json!({"path": "a.txt"})),
            (
                "edit",
                serde_json::json!({"path": "a.txt", "oldText": "before", "newText": "after"}),
            ),
            ("read", serde_json::json!({"path": "a.txt"})),
        ]]))
        .unwrap();
    let results: Vec<String> = agent
        .messages
        .iter()
        .filter(|message| message.role == "toolResult")
        .map(|message| content_text(&message.content))
        .collect();
    assert_eq!(results.len(), 3, "{results:?}");
    assert!(results[0].contains("before"), "{results:?}");
    assert!(results[2].contains("after"), "{results:?}");
}

#[test]
fn a_batch_runs_its_operations_behind_one_result() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "alpha\nneedle\n").unwrap();
    std::fs::write(dir.path().join("b.txt"), "beta\n").unwrap();
    let mut agent = Agent::new(default_system_prompt());
    agent.cwd = dir.path().to_path_buf();
    agent.prompt("go");
    let events = agent
        .run_loop(scripted_batches(vec![vec![(
            "batch",
            serde_json::json!({"operations": [
                {"tool": "read", "args": {"path": "a.txt"}},
                {"tool": "grep", "args": {"pattern": "needle", "path": "."}},
                {"tool": "ls", "args": {"path": "."}},
                {"tool": "batch", "args": {"operations": []}},
                {"tool": "nope", "args": {}},
            ]}),
        )]]))
        .unwrap();
    let outcomes = tool_outcomes(&events);
    assert_eq!(outcomes.len(), 1, "{outcomes:?}");
    let (name, is_error, text) = &outcomes[0];
    assert_eq!(name, "batch");
    assert!(!is_error);
    assert!(text.starts_with("batch: 5/5 operations ran"), "{text}");
    assert!(text.contains("[1] read path=\"a.txt\" → ok"), "{text}");
    assert!(text.contains("alpha"), "{text}");
    assert!(
        text.contains("[2] grep path=\".\" pattern=\"needle\""),
        "{text}"
    );
    assert!(text.contains("a.txt:2: needle"), "{text}");
    assert!(text.contains("[3] ls"), "{text}");
    assert!(text.contains("b.txt"), "{text}");
    assert!(text.contains("[4] batch operations=… → error"), "{text}");
    assert!(text.contains("cannot run inside a batch"), "{text}");
    assert!(text.contains("[5] nope → error"), "{text}");
    assert!(text.contains("Unknown tool: nope"), "{text}");
    let stats = agent.run_stats();
    assert_eq!(stats.tool_calls, 1);
    assert_eq!(stats.batch_operations, 5);
    // One model-visible tool result for five operations.
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
fn a_batch_operation_is_gated_like_a_direct_call() {
    use std::sync::Arc;
    let dir = tempfile::tempdir().unwrap();
    let mut agent = Agent::new(default_system_prompt());
    agent.cwd = dir.path().to_path_buf();
    agent.permissions = Arc::new(PermissionState::new(PermissionPolicy::new(
        PermissionMode::ReadOnly,
    )));
    agent.prompt("go");
    let events = agent
        .run_loop(scripted_batches(vec![vec![(
            "batch",
            serde_json::json!({"operations": [
                {"tool": "write", "args": {"path": "x.txt", "content": "no"}},
                {"tool": "ls", "args": {}},
            ]}),
        )]]))
        .unwrap();
    let outcomes = tool_outcomes(&events);
    let text = &outcomes[0].2;
    assert!(
        text.contains("[1] write path=\"x.txt\" content=\"no\" → error"),
        "{text}"
    );
    assert!(text.contains(crate::PLAN_MODE_DENIAL), "{text}");
    assert!(!dir.path().join("x.txt").exists());
    assert!(text.contains("[2] ls → ok"), "{text}");
}

#[test]
fn overflowing_batch_output_goes_to_the_evidence_store() {
    let dir = tempfile::tempdir().unwrap();
    let big = "z".repeat(20 * 1024);
    std::fs::write(dir.path().join("big.txt"), &big).unwrap();
    let mut agent = Agent::new(default_system_prompt());
    agent.cwd = dir.path().to_path_buf();
    agent.evidence = Some(EvidenceStore::new(dir.path().join("evidence")));
    agent.prompt("go");
    let events = agent
        .run_loop(scripted_batches(vec![vec![(
            "batch",
            serde_json::json!({"operations": [
                {"tool": "read", "args": {"path": "big.txt"}},
            ]}),
        )]]))
        .unwrap();
    let text = &tool_outcomes(&events)[0].2;
    assert!(
        text.len() < 14 * 1024,
        "visible result is {} bytes",
        text.len()
    );
    assert!(text.contains("full output saved to"), "{text}");
    let path = text
        .split("full output saved to ")
        .nth(1)
        .and_then(|rest| rest.split(" — ").next())
        .unwrap();
    let saved = std::fs::read_to_string(path.trim()).unwrap();
    assert!(saved.contains(&big));
    assert_eq!(agent.run_stats().evidence_files, 1);
}

#[test]
fn old_tool_output_is_pruned_from_the_provider_view_but_kept_in_history() {
    let dir = tempfile::tempdir().unwrap();
    let body = "q".repeat(4_000);
    std::fs::write(dir.path().join("f.txt"), &body).unwrap();
    let mut agent = Agent::new(default_system_prompt());
    agent.cwd = dir.path().to_path_buf();
    agent.evidence = Some(EvidenceStore::new(dir.path().join("evidence")));
    agent.auto_compaction = false;
    // ~1000 tokens per read; a 6k window starts pruning past 3k.
    agent.context_window = 6_000;
    agent.prune_settings.keep_recent = 2;
    agent.prompt("go");
    let batches: Vec<Vec<(&'static str, serde_json::Value)>> = (0..6)
        .map(|_| vec![("read", serde_json::json!({"path": "f.txt"}))])
        .collect();
    agent.run_loop(scripted_batches(batches)).unwrap();
    let stats = agent.run_stats();
    assert!(stats.pruned_results >= 2, "{stats:?}");
    assert!(stats.pruned_chars >= 8_000, "{stats:?}");
    // History is whole.
    let full: Vec<&ChatMessage> = agent
        .messages
        .iter()
        .filter(|message| message.role == "toolResult")
        .collect();
    assert_eq!(full.len(), 6);
    assert!(full
        .iter()
        .all(|message| content_text(&message.content).contains(&body)));
    // The provider view is not.
    let projected = agent.messages_for_provider();
    let pruned = projected
        .iter()
        .filter(|message| message.role == "toolResult")
        .filter(|message| content_text(&message.content).contains("pruned to save context"))
        .count();
    assert_eq!(pruned as u64, stats.pruned_results);
    // The newest results are untouched.
    let last = projected
        .iter()
        .rev()
        .find(|message| message.role == "toolResult")
        .unwrap();
    assert!(content_text(&last.content).contains(&body));
    // Compare like-for-like estimates: both include system prompt and tool
    // schema overhead, which can exceed the saved history tokens by itself.
    assert!(
        crate::estimate_context_tokens(&projected)
            < crate::estimate_context_tokens(&agent.messages)
    );
    let pruned_estimate = agent.estimated_context_tokens();
    let saved = std::mem::take(&mut agent.pruned_tool_results);
    let unpruned_estimate = agent.estimated_context_tokens();
    agent.pruned_tool_results = saved;
    assert!(pruned_estimate < unpruned_estimate);
}

#[test]
fn ask_mode_asks_the_approver_for_edits_and_never_for_reads() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("note.txt"), "hello").unwrap();
    let asked = Arc::new(AtomicUsize::new(0));
    let seen = asked.clone();
    let mut agent = Agent::new(default_system_prompt());
    agent.cwd = dir.path().to_path_buf();
    agent.permissions = Arc::new(PermissionState::new(PermissionPolicy::new(
        PermissionMode::Ask,
    )));
    agent.approver = Some(ToolApprover(Arc::new(move |request| {
        seen.fetch_add(1, Ordering::SeqCst);
        assert_eq!(request.tool, "write");
        assert_eq!(request.summary, "write · out.txt");
        ToolApprovalDecision::AllowForSession
    })));
    agent.prompt("go");
    let events = agent
        .run_loop(scripted_tool_calls(vec![
            ("read", serde_json::json!({"path": "note.txt"})),
            (
                "write",
                serde_json::json!({"path": "out.txt", "content": "a"}),
            ),
            (
                "write",
                serde_json::json!({"path": "out.txt", "content": "b"}),
            ),
        ]))
        .unwrap();
    let outcomes = tool_outcomes(&events);
    assert_eq!(outcomes.len(), 3, "{outcomes:?}");
    assert!(
        outcomes.iter().all(|(_, is_error, _)| !is_error),
        "{outcomes:?}"
    );
    // The read never asked; the first write did and the second was
    // covered by the session rule the answer added.
    assert_eq!(asked.load(Ordering::SeqCst), 1);
    assert_eq!(
        std::fs::read_to_string(dir.path().join("out.txt")).unwrap(),
        "b"
    );
    assert_eq!(
        agent
            .permissions
            .lock()
            .unwrap()
            .session_allow
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>(),
        ["write(out.txt)"]
    );
}

#[test]
fn read_only_refuses_an_edit_without_asking_and_the_loop_goes_on() {
    use std::sync::Arc;

    let dir = tempfile::tempdir().unwrap();
    let mut agent = Agent::new(default_system_prompt());
    agent.cwd = dir.path().to_path_buf();
    agent.permissions = Arc::new(PermissionState::new(PermissionPolicy::new(
        PermissionMode::ReadOnly,
    )));
    agent.approver = Some(ToolApprover(Arc::new(|_request| {
        panic!("read-only never asks")
    })));
    agent.prompt("go");
    let events = agent
        .run_loop(scripted_tool_calls(vec![(
            "write",
            serde_json::json!({"path": "out.txt", "content": "a"}),
        )]))
        .unwrap();
    let outcomes = tool_outcomes(&events);
    assert_eq!(outcomes.len(), 1);
    assert!(outcomes[0].1, "{outcomes:?}");
    assert!(
        outcomes[0].2.contains(crate::PLAN_MODE_DENIAL),
        "{}",
        outcomes[0].2
    );
    assert!(!dir.path().join("out.txt").exists());
    assert_eq!(agent.last_assistant_text().as_deref(), Some("done"));
}

#[test]
fn a_declined_call_tells_the_model_the_user_said_no() {
    use std::sync::Arc;

    let dir = tempfile::tempdir().unwrap();
    let mut agent = Agent::new(default_system_prompt());
    agent.cwd = dir.path().to_path_buf();
    agent.permissions = Arc::new(PermissionState::new(PermissionPolicy::new(
        PermissionMode::Ask,
    )));
    agent.approver = Some(ToolApprover(Arc::new(|_request| {
        ToolApprovalDecision::Deny
    })));
    agent.prompt("go");
    let events = agent
        .run_loop(scripted_tool_calls(vec![(
            "write",
            serde_json::json!({"path": "out.txt", "content": "a"}),
        )]))
        .unwrap();
    let outcomes = tool_outcomes(&events);
    assert_eq!(
        outcomes[0].2,
        "Permission denied: the user declined `write · out.txt`."
    );
    assert!(!dir.path().join("out.txt").exists());
}

#[test]
fn without_an_approver_ask_mode_fails_closed_and_says_how_to_open_it() {
    use std::sync::Arc;

    let dir = tempfile::tempdir().unwrap();
    let mut agent = Agent::new(default_system_prompt());
    agent.cwd = dir.path().to_path_buf();
    agent.permissions = Arc::new(PermissionState::new(PermissionPolicy::new(
        PermissionMode::Ask,
    )));
    agent.prompt("go");
    let events = agent
        .run_loop(scripted_tool_calls(vec![(
            "bash",
            serde_json::json!({"command": "git status"}),
        )]))
        .unwrap();
    let outcomes = tool_outcomes(&events);
    assert!(outcomes[0].1);
    assert!(
            outcomes[0].2.starts_with(
                "Permission denied: `bash · git status` needs approval in permission mode `ask`, and this run cannot ask."
            ),
            "{}",
            outcomes[0].2
        );
    assert!(
        outcomes[0].2.contains("`bash(git status)`"),
        "{}",
        outcomes[0].2
    );
}

#[test]
fn the_library_default_runs_every_tool_as_vendor_pi_does() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = Agent::new(default_system_prompt());
    agent.cwd = dir.path().to_path_buf();
    agent.prompt("go");
    let events = agent
        .run_loop(scripted_tool_calls(vec![(
            "write",
            serde_json::json!({"path": "out.txt", "content": "a"}),
        )]))
        .unwrap();
    assert!(tool_outcomes(&events)
        .iter()
        .all(|(_, is_error, _)| !is_error));
    assert!(dir.path().join("out.txt").exists());
}

#[test]
fn event_sink_receives_events_as_run_loop_emits_them() {
    use davinci_ai::{AssistantMessage, ContentBlock, StopReason};

    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let sink_seen = seen.clone();
    let mut agent = Agent::new(default_system_prompt());
    agent.event_sink = Some(EventSink(std::sync::Arc::new(move |event| {
        sink_seen.lock().unwrap().push(event.kind().to_string());
    })));
    agent.prompt("hello");
    let events = agent
        .run_loop(|_| {
            Ok(AssistantMessage {
                extra: Default::default(),
                id: "a1".into(),
                role: "assistant".into(),
                content: vec![ContentBlock::Text { text: "ok".into() }],
                model: "fixture".into(),
                usage: None,
                stop_reason: Some(StopReason::Stop),
                error_message: None,
            })
        })
        .unwrap();
    let kinds = seen.lock().unwrap().clone();
    assert!(
        kinds.contains(&"agent_start".to_string()),
        "sink should observe agent_start before run_loop returns: {kinds:?}"
    );
    assert_eq!(
        kinds,
        events
            .iter()
            .map(|event| event.kind().to_string())
            .collect::<Vec<_>>()
    );
}

#[test]
fn continue_loop_matches_ts_errors() {
    let mut agent = Agent::new("x");
    assert_eq!(
        agent
            .continue_loop::<_, AssistantMessage>(|_| unreachable!())
            .unwrap_err(),
        "Cannot continue: no messages in context"
    );
    agent.record_assistant("hi");
    assert_eq!(
        agent
            .continue_loop::<_, AssistantMessage>(|_| unreachable!())
            .unwrap_err(),
        "Cannot continue from message role: assistant"
    );
}

#[test]
fn custom_tool_executor_context_requires_engine_context() {
    let executor = CustomToolExecutor::new_with_context(|_, _, _, _| {
        panic!("context-dependent callback must not run without engine context")
    });
    assert!(executor
        .execute(Path::new("."), "browser_open", &serde_json::json!({}))
        .is_err());
}

#[test]
fn custom_tool_executor_context_receives_dispatch_cancellation_and_jobs() {
    let mut agent = Agent::new("context fixture");
    agent.permissions = Arc::new(PermissionState::new(PermissionPolicy::new(
        PermissionMode::Ask,
    )));
    agent.tools.push("context_fixture".into());
    agent.set_runtime(RuntimeHandle::new(
        RunId::new(),
        AgentId::new(),
        runtime::RuntimeBus::new(),
    ));
    agent.approval_responder = Some(approval::ApprovalResponder(Arc::new(|_, challenge| {
        approval::ApprovalReply {
            challenge_id: challenge.id,
            choice_id: "once".into(),
            instructions: None,
        }
    })));
    let abort = agent
        .runtime
        .as_ref()
        .unwrap()
        .cancellation_token
        .as_atomic_bool();
    let jobs = agent.tool_context.jobs.clone();
    let permissions = agent.permissions.clone();
    let observed = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let captured = observed.clone();
    agent.custom_tool_executor = Some(CustomToolExecutor::new_with_context(
        move |cwd, name, args, context| {
            assert_eq!(name, "context_fixture");
            assert!(context.dispatch_permit.is_some());
            let revision = permissions.lock().unwrap().revision();
            let permit = context.dispatch_permit.as_ref().unwrap();
            permit
                .consume(&permissions, revision, cwd, name, args)
                .unwrap();
            assert!(permit
                .consume(&permissions, revision, cwd, name, args)
                .is_err());
            assert!(Arc::ptr_eq(context.abort.as_ref().unwrap(), &abort));
            assert!(Arc::ptr_eq(&context.jobs, &jobs));
            captured.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(ToolResult {
                content: "context observed".into(),
                is_error: false,
                details: None,
            })
        },
    ));
    let cwd = agent.cwd.clone();
    assert!(matches!(
        agent.prepare_tool_call(
            &cwd,
            "context-call",
            "context_fixture",
            &serde_json::json!({}),
            0
        ),
        turn::Preparation::Ready { .. }
    ));
    let result = agent.run_prepared_call(
        &cwd,
        "context-call",
        "context_fixture",
        &serde_json::json!({}),
        0,
    );
    assert!(!result.is_error, "{}", result.content);
    assert!(observed.load(std::sync::atomic::Ordering::SeqCst));
    assert!(agent.permissions.lock().unwrap().session_allow.is_empty());
}

#[test]
fn custom_tool_executor_runs_unknown_builtin_names() {
    use davinci_ai::{AssistantMessage, ContentBlock, StopReason};

    let mut agent = Agent::new(default_system_prompt());
    agent.tools.push("ticket".into());
    agent.custom_tool_executor = Some(CustomToolExecutor::new(|_cwd, name, args| {
        Ok(ToolResult {
            content: format!("{name}:{}", args["id"].as_str().unwrap_or("")),
            is_error: false,
            details: None,
        })
    }));
    agent.prompt("lookup");
    let events = agent
        .run_loop(|current| {
            if current.messages.iter().any(|m| m.role == "toolResult") {
                return Ok(AssistantMessage {
                    extra: Default::default(),
                    id: "a2".into(),
                    role: "assistant".into(),
                    content: vec![ContentBlock::Text {
                        text: "done".into(),
                    }],
                    model: "fixture".into(),
                    usage: None,
                    stop_reason: Some(StopReason::Stop),
                    error_message: None,
                });
            }
            Ok(AssistantMessage {
                extra: Default::default(),
                id: "a1".into(),
                role: "assistant".into(),
                content: vec![ContentBlock::ToolCall {
                    id: "call_1".into(),
                    name: "ticket".into(),
                    arguments: serde_json::json!({"id": "42"}),
                }],
                model: "fixture".into(),
                usage: None,
                stop_reason: Some(StopReason::ToolUse),
                error_message: None,
            })
        })
        .unwrap();
    assert!(events
        .iter()
        .any(|event| event.kind() == "tool_execution_end"));
    assert_eq!(agent.last_assistant_text().as_deref(), Some("done"));
}

#[test]
fn block_images_and_abort_retry_match_ts() {
    use davinci_ai::MessageContent;
    let mut agent = Agent::new("x");
    agent.block_images = true;
    agent.prompt_with(
        "hi",
        &[MessageContent::Image {
            data: "e30=".into(),
            mime_type: "image/png".into(),
        }],
    );
    let for_llm = agent.messages_for_provider();
    assert!(for_llm[0].content.iter().any(|block| matches!(
        block,
        MessageContent::Text { text } if text == IMAGE_READING_DISABLED
    )));
    agent.abort_retry();
    assert!(agent.retry_aborted);
}

#[test]
fn ephemeral_context_precedes_latest_prompt_without_persistence() {
    let mut agent = Agent::new("x");
    agent.prompt("active question");
    agent.set_ephemeral_context(vec![ChatMessage::text("custom", "supporting memory")]);

    let provider_messages = agent.messages_for_provider();
    assert_eq!(provider_messages.len(), 2);
    assert_eq!(provider_messages[0].role, "user");
    assert_eq!(
        content_text(&provider_messages[0].content),
        "supporting memory"
    );
    assert_eq!(
        content_text(&provider_messages[1].content),
        "active question"
    );
    assert_eq!(agent.messages.len(), 1);

    agent.clear_ephemeral_context();
    assert_eq!(agent.messages_for_provider().len(), 1);
}

#[test]
fn retry_delay_matches_ts_exponential_backoff() {
    assert_eq!(retry_delay_ms(2000, 0), 2000);
    assert_eq!(retry_delay_ms(2000, 1), 4000);
    assert_eq!(retry_delay_ms(2000, 2), 8000);
    assert_eq!(retry_delay_ms(1, 3), 8);
}

#[test]
fn navigate_tree_appends_llm_branch_summary() {
    let dir = tempfile::tempdir().unwrap();
    let mut session = JsonlSession::create(dir.path(), "/tmp/project", Some("branch")).unwrap();
    session
        .append_entry(SessionEntry::message(
            "user",
            serde_json::json!([{"type":"text","text":"start"}]),
        ))
        .unwrap();
    session
        .append_entry(SessionEntry::message(
            "assistant",
            serde_json::json!([{"type":"text","text":"ok"}]),
        ))
        .unwrap();
    session
        .append_entry(SessionEntry::message(
            "user",
            serde_json::json!([{"type":"text","text":"abandoned"}]),
        ))
        .unwrap();
    let abandoned = session.leaf_id.clone().unwrap();
    session.set_leaf(Some(session.entries[1].id.clone()));
    session
        .append_entry(SessionEntry::message(
            "user",
            serde_json::json!([{"type":"text","text":"other"}]),
        ))
        .unwrap();

    let mut agent = Agent::new("x");
    agent.summarizer = Some(Summarizer::new(|request| {
        assert_eq!(request.system, SUMMARIZATION_SYSTEM_PROMPT);
        assert!(request.prompt.contains("## Goal"));
        assert_eq!(request.max_tokens, 2048);
        Ok(SummarizeResponse {
            text: "## Goal\nabandoned work".into(),
            usage: Some(davinci_protocol::Usage {
                input: 1,
                output: 2,
                total_tokens: 3,
                ..davinci_protocol::Usage::default()
            }),
            stop_reason: Some(davinci_ai::StopReason::Stop),
            error_message: None,
            has_tool_call: false,
        })
    }));
    agent.load_from_session(session).unwrap();
    let result = agent
        .navigate_tree_entry(&abandoned, true, None, false, 16_384)
        .unwrap();
    assert_eq!(result.editor_text.as_deref(), Some("abandoned"));
    assert!(result
        .summary
        .as_deref()
        .unwrap()
        .starts_with(BRANCH_SUMMARY_PREAMBLE));
    assert!(result
        .summary
        .as_deref()
        .unwrap()
        .contains("## Goal\nabandoned work"));
    let stored = agent.session.as_ref().unwrap();
    assert!(stored
        .entries
        .iter()
        .any(|entry| entry.entry_type == "branch_summary"
            && entry.extra.get("fromId").and_then(Value::as_str)
                == stored
                    .entries
                    .iter()
                    .find(|e| {
                        e.entry_type == "message"
                            && e.message
                                .as_ref()
                                .and_then(|m| m.get("content"))
                                .and_then(|c| c.as_array())
                                .and_then(|items| items[0].get("text"))
                                .and_then(Value::as_str)
                                == Some("other")
                    })
                    .map(|e| e.id.as_str())));
    assert!(agent
        .messages
        .iter()
        .any(|message| content_text(&message.content).contains("abandoned work")));
    assert!(!agent
        .messages
        .iter()
        .any(|message| content_text(&message.content) == "other"));
}

#[test]
fn pre_tool_hook_blocks_before_execution() {
    use davinci_ai::{AssistantMessage, ContentBlock, StopReason};
    let mut agent = Agent::new("x");
    agent.pre_tool = Some(PreToolHook(Arc::new(|name, _| {
        if name == "bash" {
            Some("blocked by extension".into())
        } else {
            None
        }
    })));
    let seen = Arc::new(std::sync::Mutex::new(None));
    let seen_post = seen.clone();
    agent.post_tool = Some(PostToolHook(Arc::new(move |_, _, _, _, result| {
        *seen_post.lock().unwrap() = result.details.clone();
        result
    })));
    agent.prompt("run");
    let events = agent
        .run_loop(|current| {
            if current
                .messages
                .iter()
                .any(|message| message.role == "toolResult")
            {
                return Ok(AssistantMessage {
                    extra: Default::default(),
                    id: "a2".into(),
                    role: "assistant".into(),
                    content: vec![ContentBlock::Text {
                        text: "stopped".into(),
                    }],
                    model: "fixture".into(),
                    usage: None,
                    stop_reason: Some(StopReason::Stop),
                    error_message: None,
                });
            }
            Ok(AssistantMessage {
                extra: Default::default(),
                id: "a1".into(),
                role: "assistant".into(),
                content: vec![ContentBlock::ToolCall {
                    id: "c1".into(),
                    name: "bash".into(),
                    arguments: serde_json::json!({"command": "echo hi"}),
                }],
                model: "fixture".into(),
                usage: None,
                stop_reason: Some(StopReason::ToolUse),
                error_message: None,
            })
        })
        .unwrap();
    assert!(events
        .iter()
        .any(|event| matches!(event, AgentEvent::ToolExecutionEnd { is_error: true, .. })));
    let result = agent
        .messages
        .iter()
        .rev()
        .find(|message| message.role == "toolResult")
        .map(|message| content_text(&message.content))
        .unwrap_or_default();
    assert!(result.contains("blocked by extension"));
    // The post hook learns the call never ran (plugins skip PostToolUse).
    assert_eq!(
        *seen.lock().unwrap(),
        Some(serde_json::json!({ "preToolBlocked": true }))
    );
}

#[test]
fn record_bash_result_persists_ts_message_and_excludes_double_bang_from_llm() {
    let dir = tempfile::tempdir().unwrap();
    let session = JsonlSession::create(dir.path(), "/tmp/project", Some("bash")).unwrap();
    let mut agent = Agent::new("x");
    agent.session = Some(session);

    agent.record_bash_result(
        "printf hi",
        &serde_json::json!({
            "output": "hi",
            "exitCode": 0,
            "cancelled": false,
            "truncated": false
        }),
        false,
    );

    let bash = agent.messages.last().expect("bash message");
    assert_eq!(bash.role, "bashExecution");
    assert_eq!(
        bash.extra.get("command"),
        Some(&serde_json::json!("printf hi"))
    );
    assert_eq!(bash.extra.get("output"), Some(&serde_json::json!("hi")));
    assert_eq!(bash.extra.get("exitCode"), Some(&serde_json::json!(0)));
    assert_eq!(bash.extra.get("cancelled"), Some(&serde_json::json!(false)));
    assert_eq!(bash.extra.get("truncated"), Some(&serde_json::json!(false)));

    let stored = agent.session.as_ref().unwrap().entries.last().unwrap();
    let stored_message = stored.message.as_ref().unwrap();
    assert_eq!(
        stored_message.get("role").and_then(Value::as_str),
        Some("bashExecution")
    );
    assert_eq!(
        stored_message.get("command").and_then(Value::as_str),
        Some("printf hi")
    );
    assert_eq!(
        stored_message.get("output").and_then(Value::as_str),
        Some("hi")
    );

    let llm = agent.messages_for_provider();
    assert_eq!(llm.last().unwrap().role, "user");
    assert_eq!(
        content_text(&llm.last().unwrap().content),
        "Ran `printf hi`\n```\nhi\n```"
    );

    agent.record_bash_result(
        "secret",
        &serde_json::json!({
            "output": "hidden",
            "exitCode": 0,
            "cancelled": false,
            "truncated": false
        }),
        true,
    );
    assert_eq!(
        agent
            .messages
            .last()
            .unwrap()
            .extra
            .get("excludeFromContext")
            .and_then(Value::as_bool),
        Some(true)
    );
    assert!(!agent
        .messages_for_provider()
        .iter()
        .any(|message| content_text(&message.content).contains("secret")));
}

#[test]
fn pending_bash_results_flush_before_the_next_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let session = JsonlSession::create(dir.path(), "/tmp/project", Some("bash-pending")).unwrap();
    let mut agent = Agent::new("x");
    agent.session = Some(session);
    agent.is_streaming = true;

    agent.record_bash_result(
        "echo queued",
        &serde_json::json!({
            "output": "queued",
            "exitCode": 0,
            "cancelled": false,
            "truncated": false
        }),
        false,
    );
    assert!(agent.messages.is_empty());
    assert!(agent.session.as_ref().unwrap().entries.is_empty());

    agent.is_streaming = false;
    agent.prompt("next");
    assert_eq!(agent.messages[0].role, "bashExecution");
    assert_eq!(agent.messages[1].role, "user");
    let entries = &agent.session.as_ref().unwrap().entries;
    assert_eq!(
        entries
            .iter()
            .filter(|entry| entry.entry_type == "message")
            .count(),
        2
    );
    assert_eq!(
        entries[0]
            .message
            .as_ref()
            .and_then(|message| message.get("role"))
            .and_then(Value::as_str),
        Some("bashExecution")
    );
}

#[test]
fn record_custom_message_persists_before_agent_start_shape() {
    let dir = tempfile::tempdir().unwrap();
    let session = JsonlSession::create(dir.path(), "/tmp/project", Some("custom")).unwrap();
    let mut agent = Agent::new("x");
    agent.session = Some(session);

    agent.record_custom_message(&serde_json::json!({
        "customType": "hint",
        "content": [{"type":"text","text":"extension context"}],
        "display": false,
        "details": {"source":"before_agent_start"}
    }));

    let message = agent.messages.last().unwrap();
    assert_eq!(message.role, "custom");
    assert_eq!(content_text(&message.content), "extension context");
    assert_eq!(
        message.extra.get("customType").and_then(Value::as_str),
        Some("hint")
    );
    assert_eq!(
        message.extra.get("display").and_then(Value::as_bool),
        Some(false)
    );
    let stored = agent.session.as_ref().unwrap().entries.last().unwrap();
    assert_eq!(stored.entry_type, "custom_message");
    assert_eq!(stored.custom_type.as_deref(), Some("hint"));
    assert_eq!(
        stored.extra.get("content"),
        Some(&serde_json::json!([{"type":"text","text":"extension context"}]))
    );
    assert_eq!(stored.extra.get("display"), Some(&serde_json::json!(false)));
    assert_eq!(
        stored.extra.get("details"),
        Some(&serde_json::json!({"source":"before_agent_start"}))
    );
}

#[test]
fn before_agent_start_custom_messages_are_emitted_in_current_turn_and_reload() {
    use davinci_ai::{AssistantMessage, ContentBlock, StopReason};

    let dir = tempfile::tempdir().unwrap();
    let session = JsonlSession::create(dir.path(), "/tmp/project", Some("prompt")).unwrap();
    let mut agent = Agent::new("x");
    agent.session = Some(session);
    agent.prompt("hello");
    agent.record_custom_message(&serde_json::json!({
        "customType": "hint",
        "content": [
            {"type":"text","text":"extension context"},
            {"type":"image","data":"e30=","mimeType":"image/png"}
        ],
        "display": false,
        "details": {"source":"before_agent_start"}
    }));

    let events = agent
        .run_loop(|_| {
            Ok(AssistantMessage {
                extra: Default::default(),
                id: "assistant-1".into(),
                role: "assistant".into(),
                content: vec![ContentBlock::Text {
                    text: "done".into(),
                }],
                model: "fixture".into(),
                usage: None,
                stop_reason: Some(StopReason::Stop),
                error_message: None,
            })
        })
        .unwrap();

    let message_starts: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            AgentEvent::MessageStart { message } => Some(message),
            _ => None,
        })
        .collect();
    assert_eq!(
        message_starts
            .iter()
            .map(|message| message.role.as_str())
            .collect::<Vec<_>>(),
        vec!["user", "custom", "assistant"]
    );
    assert_eq!(
        message_starts[1].extra.get("customType"),
        Some(&serde_json::json!("hint"))
    );
    assert!(matches!(
        message_starts[1].content.get(1),
        Some(davinci_ai::MessageContent::Image { .. })
    ));

    let agent_end = events
        .iter()
        .find_map(|event| match event {
            AgentEvent::AgentEnd { messages, .. } => Some(messages),
            _ => None,
        })
        .unwrap();
    assert_eq!(
        agent_end
            .iter()
            .map(|message| message.role.as_str())
            .collect::<Vec<_>>(),
        vec!["user", "custom", "assistant"]
    );

    let session = agent.session.as_ref().unwrap();
    let custom_entry = session
        .entries
        .iter()
        .find(|entry| entry.entry_type == "custom_message")
        .unwrap();
    assert_eq!(custom_entry.custom_type.as_deref(), Some("hint"));
    assert_eq!(
        custom_entry.extra.get("content"),
        Some(&serde_json::json!([
            {"type":"text","text":"extension context"},
            {"type":"image","data":"e30=","mimeType":"image/png"}
        ]))
    );

    let reloaded = messages_from_session(session);
    let reloaded_custom = reloaded
        .iter()
        .find(|message| message.role == "custom")
        .unwrap();
    assert_eq!(content_text(&reloaded_custom.content), "extension context");
    assert_eq!(
        reloaded_custom.extra.get("customType"),
        Some(&serde_json::json!("hint"))
    );
    assert!(matches!(
        reloaded_custom.content.get(1),
        Some(davinci_ai::MessageContent::Image { .. })
    ));
}

#[test]
fn a_background_job_is_announced_to_the_model_before_its_next_completion() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = Agent::new(default_system_prompt());
    agent.cwd = dir.path().to_path_buf();
    agent.tool_context.foreground_supervisor = Some(crate::jobs::supervisor::SupervisorCommand {
        executable: std::env::current_exe().unwrap(),
        argv: vec![
            "--exact".into(),
            "tools::foreground::tests::foreground_supervisor_fixture".into(),
            "--nocapture".into(),
        ],
    });
    agent.prompt("build it");
    let events = agent
        .run_loop(scripted_tool_calls(vec![
            (
                "bash",
                serde_json::json!({"command": "echo built", "background": true}),
            ),
            // Give the job time to exit; the next completion sees it.
            ("job_output", serde_json::json!({"jobId": 1, "wait": 10})),
        ]))
        .unwrap();
    let outcomes = tool_outcomes(&events);
    assert!(
        outcomes[0]
            .2
            .starts_with("Started background job 1: `echo built`"),
        "{:?}",
        outcomes[0]
    );
    assert!(outcomes[1].2.contains("built"), "{:?}", outcomes[1]);
    assert!(outcomes[1].2.contains("[job 1 exit 0"), "{:?}", outcomes[1]);
    // The notice is a user message with its custom type, injected once,
    // before the completion that followed the job's end.
    let notices: Vec<&ChatMessage> = agent
        .messages
        .iter()
        .filter(|message| {
            message.extra.get("customType") == Some(&serde_json::json!(JOB_NOTICE_TYPE))
        })
        .collect();
    assert_eq!(notices.len(), 1);
    let text = content_text(&notices[0].content);
    assert!(
        text.starts_with("[background job 1 finished · exit 0 · "),
        "{text}"
    );
    assert!(text.contains("] echo built\n    built"), "{text}");
    let notice_index = agent
        .messages
        .iter()
        .position(|message| message.extra.contains_key("customType"))
        .unwrap();
    let last_assistant = agent
        .messages
        .iter()
        .rposition(|message| message.role == "assistant")
        .unwrap();
    assert!(
        notice_index < last_assistant,
        "the notice came before the final reply"
    );
    // Nothing left to announce, and the events carried it as a message.
    assert!(agent.job_notice_messages().is_empty());
    assert!(events.iter().any(|event| matches!(
        event,
        AgentEvent::MessageStart { message } if message.extra.contains_key("customType")
    )));
}

#[test]
fn a_job_that_ends_between_turns_is_prepended_to_the_next_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = Agent::new(default_system_prompt());
    agent.cwd = dir.path().to_path_buf();
    let result = execute_tool_with(
        dir.path(),
        "bash",
        &serde_json::json!({"command": "echo later", "background": true}),
        &agent.tool_context,
    )
    .unwrap();
    assert!(result.content.starts_with("Started background job 1"));
    execute_tool_with(
        dir.path(),
        "job_output",
        &serde_json::json!({"jobId": 1, "wait": 10}),
        &agent.tool_context,
    )
    .unwrap();
    agent.prompt("what happened?");
    let roles: Vec<(String, bool)> = agent
        .messages
        .iter()
        .map(|message| {
            (
                message.role.clone(),
                message.extra.contains_key("customType"),
            )
        })
        .collect();
    assert_eq!(
        roles,
        vec![("user".to_string(), true), ("user".to_string(), false)]
    );
    assert_eq!(agent.pending_prompt_messages.len(), 2);
}

#[test]
fn permission_mode_cycle_keeps_policy_and_native_prompt_in_sync() {
    let mut agent = Agent::new("unchanged base");
    agent.set_permission_mode(PermissionMode::Ask);
    for expected in [
        PermissionMode::Edits,
        PermissionMode::ReadOnly,
        PermissionMode::Auto,
        PermissionMode::AlwaysApprove,
        PermissionMode::Ask,
    ] {
        assert_eq!(agent.cycle_permission_mode(), expected);
        assert_eq!(agent.permission_mode(), expected);
        assert_eq!(
            agent.system_prompt.contains(PLAN_MODE_APPENDIX),
            expected == PermissionMode::ReadOnly
        );
    }
    assert_eq!(agent.system_prompt, "unchanged base");
}

#[test]
fn structured_plan_storage_failure_emits_only_failure_and_rolls_back() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("src.rs"), "fn existing() {}\n").unwrap();
    let mut agent = Agent::new(default_system_prompt());
    agent.cwd = dir.path().to_path_buf();
    let session =
        JsonlSession::create(dir.path(), &dir.path().display().to_string(), None).unwrap();
    let path = session.path.clone();
    agent.session = Some(session);
    agent.set_permission_mode(PermissionMode::ReadOnly);
    agent.prompt("Propose a change");
    agent.post_tool = Some(PostToolHook(Arc::new(move |_, _, _, _, result| {
        std::fs::rename(&path, path.with_extension("backup")).unwrap();
        std::fs::create_dir(&path).unwrap();
        result
    })));
    let observed = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = observed.clone();
    agent.event_sink = Some(EventSink(Arc::new(move |event| {
        sink.lock().unwrap().push(event.clone());
    })));
    let error = agent.run_loop(scripted_tool_calls(vec![("propose_plan", serde_json::json!({
            "expected_revision":0, "goal":"Add behavior", "evidence":[{"path":"src.rs","finding":"Existing function"}],
            "steps":[{"id":"implement","change":"Extend existing function","files":["src.rs"],"why":"Requested behavior","verify":["cargo test --offline"]}]
        }))])).unwrap_err();
    assert!(error.contains("Session recovery required"));
    let events = observed.lock().unwrap();
    let ends: Vec<_> = events
        .iter()
        .filter_map(|event| match event {
            AgentEvent::ToolExecutionEnd {
                tool_name,
                is_error,
                ..
            } if tool_name == "propose_plan" => Some(*is_error),
            _ => None,
        })
        .collect();
    assert_eq!(ends, vec![true]);
    assert_eq!(agent.tool_context.living_plan.lock().unwrap().revision, 0);
    assert!(agent.is_plan_mode());
    assert!(agent.render_plan().contains("storage error"));
}

#[test]
fn structured_plan_post_hook_failure_does_not_commit_the_plan() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("src.rs"), "fn existing() {}\n").unwrap();
    let mut agent = Agent::new(default_system_prompt());
    agent.cwd = dir.path().to_path_buf();
    agent.session =
        Some(JsonlSession::create(dir.path(), &dir.path().display().to_string(), None).unwrap());
    agent.set_permission_mode(PermissionMode::ReadOnly);
    let hooks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let hook_count = hooks.clone();
    agent.post_tool = Some(PostToolHook(Arc::new(move |_, _, name, _, mut result| {
        if name == "propose_plan" {
            hook_count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            result.is_error = true;
            result.content = "Plan rejected by validation hook".into();
        }
        result
    })));
    agent.prompt("Propose a change");
    let events = agent.run_loop(scripted_tool_calls(vec![("propose_plan", serde_json::json!({
            "expected_revision":0, "goal":"Add behavior", "evidence":[{"path":"src.rs","finding":"Existing function"}],
            "steps":[{"id":"implement","change":"Extend function","files":["src.rs"],"why":"Requested behavior","verify":["cargo test --offline"]}]
        }))])).unwrap();
    assert_eq!(hooks.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(events.iter().any(|event| matches!(event, AgentEvent::ToolExecutionEnd {tool_name, is_error:true, ..} if tool_name == "propose_plan")));
    assert_eq!(agent.tool_context.living_plan.lock().unwrap().revision, 0);
    let raw = std::fs::read_to_string(&agent.session.as_ref().unwrap().path).unwrap();
    assert!(!raw.contains("\"customType\":\"living_plan\""));
}

#[test]
fn structured_plan_tool_is_read_only_and_persists_in_the_session() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("src.rs"), "fn existing() {}\n").unwrap();
    let mut agent = Agent::new(default_system_prompt());
    agent.cwd = dir.path().to_path_buf();
    agent.session = Some(
        JsonlSession::create(dir.path(), &dir.path().display().to_string(), Some("plan")).unwrap(),
    );
    agent.set_permission_mode(PermissionMode::ReadOnly);
    agent.prompt("Investigate and propose a change without implementing it");
    let events = agent.run_loop(scripted_tool_calls(vec![("propose_plan", serde_json::json!({
            "expected_revision":0, "goal":"Add behavior", "evidence":[{"path":"src.rs","finding":"Existing function"}],
            "steps":[{"id":"implement","change":"Extend the existing function","files":["src.rs"],"why":"Requested behavior","verify":["cargo test --offline"]}]
        }))])).unwrap();
    assert!(
        tool_outcomes(&events)[0].2.contains("Revision 1"),
        "{:?}",
        tool_outcomes(&events)
    );
    let raw = std::fs::read_to_string(&agent.session.as_ref().unwrap().path).unwrap();
    assert!(raw.contains("\"customType\":\"living_plan\""), "{raw}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("src.rs")).unwrap(),
        "fn existing() {}\n"
    );
}

#[test]
fn the_todo_ledger_is_kept_on_the_agent_and_written_to_the_session() {
    let dir = tempfile::tempdir().unwrap();
    let session = JsonlSession::create(dir.path(), "/tmp/project", Some("todo")).unwrap();
    let mut agent = Agent::new(default_system_prompt());
    agent.session = Some(session);
    agent.cwd = dir.path().to_path_buf();
    agent.prompt("plan it");
    let events = agent
        .run_loop(scripted_tool_calls(vec![(
            "todo",
            serde_json::json!({"items": [
                {"text": "read the parser", "status": "done"},
                {"text": "add the branch", "status": "active"},
                {"text": "run the tests", "status": "pending"}
            ]}),
        )]))
        .unwrap();
    let outcomes = tool_outcomes(&events);
    assert_eq!(
        outcomes[0].2,
        "3 items · 1 done · 1 active\n✓ read the parser\n◉ add the branch\n○ run the tests"
    );
    assert_eq!(
        agent.tool_context.todos.lock().unwrap().summary(),
        "1 of 3 done"
    );
    let path = agent.session.as_ref().unwrap().path.clone();
    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(raw.contains("\"customType\":\"todo\""), "{raw}");

    // A fresh agent on the same session finds the ledger again.
    let reopened = JsonlSession::open(&path).unwrap();
    let mut again = Agent::new("x");
    again.session = Some(reopened);
    assert!(again.restore_todos());
    assert_eq!(again.tool_context.todos.lock().unwrap().items.len(), 3);
    let mut empty = Agent::new("x");
    assert!(!empty.restore_todos());
}

#[test]
fn set_active_tools_by_name_ignores_unknown_and_rebuilds_active_set() {
    let mut agent = Agent::new("x");
    agent.apply_extension_tools(&["ticket".into()]);
    assert!(agent.tools.contains(&"ticket".into()));
    agent.set_active_tools_by_name(&["read".into(), "missing".into(), "ticket".into()]);
    assert_eq!(agent.tools, vec!["read".to_string(), "ticket".to_string()]);
    agent.set_active_tools_by_name(&["bash".into(), "read".into()]);
    assert_eq!(agent.tools, vec!["bash".to_string(), "read".to_string()]);
    assert!(agent.tool_registry.contains(&"ticket".into()));
}

#[test]
fn mutation_without_verification_requests_evidence() {
    let agent = Agent::new("x");

    agent.record_successful_mutation();

    assert_eq!(agent.completion_evidence(), CompletionEvidence::Unverified);
}

#[test]
fn verification_generation_accumulates_only_fresh_applicable_coverage() {
    let agent = Agent::new("x");
    let a = PathBuf::from("a.py");
    let b = PathBuf::from("b.py");
    agent.record_successful_mutation_paths(vec![a.clone(), b.clone()]);
    let assessment = |path| crate::verification::Assessment {
        kind: crate::verification::CheckKind::TargetedScript,
        covered: vec![path],
        complete: false,
        full_workspace: false,
        reason: "test",
        ran_in: Some(agent.cwd.clone()),
    };
    agent.record_verification_assessment(1, "check a", &assessment(a.clone()), None);
    assert_eq!(agent.completion_evidence(), CompletionEvidence::Unverified);
    agent.record_verification_assessment(1, "check a", &assessment(a.clone()), Some(true));
    assert_eq!(agent.completion_evidence(), CompletionEvidence::Partial);
    agent.record_verification_assessment(1, "check b", &assessment(b.clone()), Some(true));
    assert_eq!(agent.completion_evidence(), CompletionEvidence::Verified);
    agent.record_successful_mutation_paths(vec![a.clone()]);
    agent.record_verification_assessment(1, "stale", &assessment(a.clone()), Some(true));
    assert_eq!(agent.completion_evidence(), CompletionEvidence::Unverified);
    agent.record_verification_assessment(2, "fresh", &assessment(a.clone()), Some(true));
    assert_eq!(agent.completion_evidence(), CompletionEvidence::Verified);
    agent.record_verification_assessment(2, "failure", &assessment(a), Some(false));
    assert_eq!(
        agent.completion_evidence(),
        CompletionEvidence::VerificationFailed
    );
}

#[test]
fn partial_verification_preserves_outstanding_paths_after_mutation() {
    let agent = Agent::new("x");
    agent.record_successful_mutation_paths(vec![PathBuf::from("a.py"), PathBuf::from("b.py")]);
    agent.record_verification_assessment(
        1,
        "check a",
        &crate::verification::Assessment {
            kind: crate::verification::CheckKind::TargetedScript,
            covered: vec![PathBuf::from("a.py")],
            complete: false,
            full_workspace: false,
            reason: "test",
            ran_in: None,
        },
        Some(true),
    );
    agent.record_successful_mutation_paths(vec![PathBuf::from("c.py")]);
    assert_eq!(agent.mutation_verification_state().mutation_paths.len(), 3);
    assert!(agent.mutation_verification_state().covered_paths.is_empty());
}

#[test]
fn unknown_mutation_paths_prevent_full_targeted_verification() {
    let agent = Agent::new("x");
    agent.record_successful_mutation_paths(vec![PathBuf::from("a.py")]);
    agent.record_successful_mutation();
    let assessment = crate::verification::Assessment {
        kind: crate::verification::CheckKind::TargetedScript,
        covered: vec![PathBuf::from("a.py")],
        complete: true,
        full_workspace: false,
        reason: "test",
        ran_in: None,
    };
    agent.record_verification_assessment(2, "check a", &assessment, Some(true));
    assert_eq!(agent.completion_evidence(), CompletionEvidence::Partial);
    agent.record_verification_assessment(2, "check a fails", &assessment, Some(false));
    let unrelated = crate::verification::Assessment {
        covered: vec![],
        ..assessment
    };
    agent.record_verification_assessment(2, "unrelated success", &unrelated, Some(true));
    assert_eq!(
        agent.completion_evidence(),
        CompletionEvidence::VerificationFailed
    );
}

#[test]
fn partial_success_preserves_a_failure_until_its_scope_passes() {
    let agent = Agent::new("x");
    let paths = [PathBuf::from("a.py"), PathBuf::from("b.py")];
    agent.record_successful_mutation_paths(paths.to_vec());
    let assessment = |path: &PathBuf| crate::verification::Assessment {
        kind: crate::verification::CheckKind::TargetedScript,
        covered: vec![path.clone()],
        complete: false,
        full_workspace: false,
        reason: "test",
        ran_in: None,
    };
    agent.record_verification_assessment(1, "b fails", &assessment(&paths[1]), Some(false));
    agent.record_verification_assessment(1, "a passes", &assessment(&paths[0]), Some(true));
    assert_eq!(
        agent.completion_evidence(),
        CompletionEvidence::VerificationFailed
    );
    agent.record_verification_assessment(1, "b passes", &assessment(&paths[1]), Some(true));
    assert_eq!(agent.completion_evidence(), CompletionEvidence::Verified);
}

#[test]
fn fresh_full_workspace_suite_recovers_unknown_mutations_and_unscoped_failures() {
    let root = tempfile::tempdir().unwrap();
    let agent = Agent::new("x");
    agent.record_successful_mutation();
    let suite = crate::verification::classify("bash", "pytest -q", root.path(), &[]);
    agent.record_verification_assessment(1, "pytest", &suite, Some(false));
    assert_eq!(
        agent.completion_evidence(),
        CompletionEvidence::VerificationFailed
    );
    agent.record_successful_mutation();
    agent.record_verification_assessment(1, "stale pytest", &suite, Some(true));
    assert!(agent.mutation_verification_state().unknown_mutation_paths);
    agent.record_verification_assessment(2, "fresh pytest", &suite, Some(true));
    assert_eq!(agent.completion_evidence(), CompletionEvidence::Verified);
    assert!(!agent.mutation_verification_state().unknown_mutation_paths);
}

#[test]
fn shell_edits_invalidate_an_earlier_check_generation() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = verifying_agent(dir.path());
    agent.prompt("write, verify, then modify through a script");
    let script = scripted_tool_calls(vec![
        (
            "write",
            serde_json::json!({"path":"a.txt", "content":"one"}),
        ),
        (
            shell_tool(),
            serde_json::json!({"command":"python -c \"from pathlib import Path; assert Path('a.txt').read_text() == 'one'\""}),
        ),
        (
            shell_tool(),
            serde_json::json!({"command":"python -c \"from pathlib import Path; Path('a.txt').write_text('two changed')\""}),
        ),
    ]);
    agent.run_loop(script).unwrap();
    assert_eq!(agent.mutation_verification_state().mutation_generation, 2);
    assert_eq!(
        agent.completion_evidence(),
        CompletionEvidence::VerificationFailed
    );
    assert_eq!(harness_runs(&agent), 1);
}

#[test]
fn resumed_session_keeps_unverified_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let sessions = tempfile::tempdir().unwrap();
    let mut agent = verifying_agent(dir.path());
    agent
        .load_from_session(JsonlSession::create(sessions.path(), "mutation", None).unwrap())
        .unwrap();
    agent.cwd = dir.path().to_path_buf();
    agent.prompt("edit a file and stop");
    agent
        .run_loop(scripted_tool_calls(vec![(
            "write",
            serde_json::json!({"path":"a.txt", "content":"one"}),
        )]))
        .unwrap();
    assert_eq!(agent.mutation_verification_state().mutation_generation, 1);
    assert_eq!(agent.completion_evidence(), CompletionEvidence::Unverified);
    let path = agent.session.as_ref().unwrap().path.clone();
    drop(agent);

    // A new process: nothing is in memory but the session file.
    let mut resumed = verifying_agent(dir.path());
    resumed
        .load_from_session(JsonlSession::open(&path).unwrap())
        .unwrap();
    resumed.cwd = dir.path().to_path_buf();
    let state = resumed.mutation_verification_state();
    assert_eq!(state.mutation_generation, 1);
    assert_eq!(state.mutation_paths, vec![PathBuf::from("a.txt")]);
    assert_eq!(
        resumed.completion_evidence(),
        CompletionEvidence::Unverified
    );
}

#[test]
fn sessions_saved_with_the_old_visible_set_still_restore_discovered_tools() {
    let dir = tempfile::tempdir().unwrap();
    let sessions = tempfile::tempdir().unwrap();
    let probe = verifying_agent(dir.path());
    let initial = probe
        .tool_context
        .tool_exposure
        .lock()
        .unwrap()
        .visible_names()
        .clone();
    let deferred = probe
        .tools
        .iter()
        .find(|name| !initial.contains(*name))
        .cloned()
        .expect("a tool that starts deferred");
    drop(probe);
    // The earlier format stored the whole visible set: host-initial tools
    // plus anything discovered, and whatever else had been visible.
    let mut visible: Vec<String> = initial.iter().cloned().collect();
    visible.push(deferred.clone());
    visible.push("tool_that_no_longer_exists".into());
    let mut session = JsonlSession::create(sessions.path(), "old-exposure", None).unwrap();
    let mut extra = serde_json::Map::new();
    extra.insert("data".into(), serde_json::json!({ "visible": visible }));
    session
        .append_entry(davinci_session::SessionEntry {
            id: String::new(),
            entry_type: "custom".into(),
            parent_id: None,
            seq: 0,
            timestamp: 0,
            message: None,
            custom_type: Some("tool_exposure".into()),
            extra,
        })
        .unwrap();
    let path = session.path.clone();
    drop(session);

    let mut resumed = verifying_agent(dir.path());
    resumed
        .load_from_session(JsonlSession::open(&path).unwrap())
        .unwrap();
    let exposure = resumed.tool_context.tool_exposure.lock().unwrap();
    assert!(
        exposure.is_visible(&deferred),
        "the discovered tool returns"
    );
    assert!(
        !exposure.is_visible("tool_that_no_longer_exists"),
        "authorization is re-checked, never trusted from the file"
    );
    assert!(exposure.discovered_names().contains(&deferred));
    assert!(
        !initial
            .iter()
            .any(|name| exposure.discovered_names().contains(name)),
        "host-initial tools are not counted as discovered"
    );
}

#[test]
fn unreadable_mutation_state_fails_closed_on_resume() {
    let dir = tempfile::tempdir().unwrap();
    let sessions = tempfile::tempdir().unwrap();
    let mut session = JsonlSession::create(sessions.path(), "corrupt", None).unwrap();
    let mut extra = serde_json::Map::new();
    // Valid JSON of the wrong shape: schema drift or a hand edit.
    extra.insert(
        "data".into(),
        serde_json::json!({"mutation_generation": "three"}),
    );
    session
        .append_entry(davinci_session::SessionEntry {
            id: String::new(),
            entry_type: "custom".into(),
            parent_id: None,
            seq: 0,
            timestamp: 0,
            message: None,
            custom_type: Some("mutation_verification".into()),
            extra,
        })
        .unwrap();
    let path = session.path.clone();
    drop(session);
    let mut resumed = verifying_agent(dir.path());
    resumed
        .load_from_session(JsonlSession::open(&path).unwrap())
        .unwrap();
    // Not "nothing happened": an edit may be pending and nothing verified it.
    assert_eq!(
        resumed.completion_evidence(),
        CompletionEvidence::Unverified
    );
    assert!(resumed.mutation_verification_state().unknown_mutation_paths);
}

#[test]
fn last_mutation_generation_survives() {
    let dir = tempfile::tempdir().unwrap();
    let sessions = tempfile::tempdir().unwrap();
    let mut agent = verifying_agent(dir.path());
    agent
        .load_from_session(JsonlSession::create(sessions.path(), "fold", None).unwrap())
        .unwrap();
    agent.cwd = dir.path().to_path_buf();
    agent.compaction.keep_recent_tokens = 1;
    agent.summarizer = Some(Summarizer::new(|_| {
        Ok(SummarizeResponse {
            text: "Lossy summary that says nothing about edits.".into(),
            usage: Some(Default::default()),
            stop_reason: Some(davinci_ai::StopReason::Stop),
            error_message: None,
            has_tool_call: false,
        })
    }));
    agent.prompt(&format!("edit a file and stop {}", "x".repeat(400)));
    agent
        .run_loop(scripted_tool_calls(vec![(
            "write",
            serde_json::json!({"path":"a.txt", "content":"one"}),
        )]))
        .unwrap();
    for turn in 0..4 {
        agent.prompt(&format!("question {turn} {}", "x".repeat(400)));
        agent.record_assistant(&format!("answer {turn} {}", "y".repeat(400)));
    }
    assert!(agent.compact(None).compacted, "the fold must happen");
    let before = agent.mutation_verification_state();
    assert_eq!(before.mutation_generation, 1);
    let path = agent.session.as_ref().unwrap().path.clone();
    drop(agent);

    let mut resumed = verifying_agent(dir.path());
    resumed
        .load_from_session(JsonlSession::open(&path).unwrap())
        .unwrap();
    assert_eq!(resumed.mutation_verification_state(), before);
    assert_eq!(
        resumed.completion_evidence(),
        CompletionEvidence::Unverified
    );
}

#[test]
fn source_edits_during_a_passing_check_cannot_verify_the_new_generation() {
    let dir = tempfile::tempdir().unwrap();
    let agent = verifying_agent(dir.path());
    std::fs::write(dir.path().join("changed.py"), "x = 1").unwrap();
    agent.record_successful_mutation_paths(vec![PathBuf::from("changed.py")]);
    agent
        .verification_starts
        .lock()
        .unwrap()
        .insert("check".into(), 1);
    agent.shell_mutation_snapshots.lock().unwrap().insert(
        "check".into(),
        crate::verification::workspace::Snapshot::capture(dir.path()),
    );
    std::fs::write(dir.path().join("changed.py"), "x = 'changed'").unwrap();
    let result = ToolResult {
        content: "passed".into(),
        is_error: false,
        details: Some(serde_json::json!({"exitCode":0})),
    };
    agent.observe_shell_verification(
        "check",
        dir.path(),
        shell_tool(),
        &serde_json::json!({"command":"pytest -q"}),
        &result,
        &result,
    );
    assert_eq!(agent.mutation_verification_state().mutation_generation, 2);
    assert_eq!(agent.completion_evidence(), CompletionEvidence::Unverified);
}

#[test]
fn exhausted_inventory_recovers_with_a_fresh_suite_but_rejects_edits_during_it() {
    let dir = tempfile::tempdir().unwrap();
    let agent = verifying_agent(dir.path());
    for id in ["suite", "editing-suite", "fresh-suite"] {
        insert_suite_receipt_fixture(&agent, id);
    }
    let paths = [PathBuf::from("changed.py")];
    std::fs::write(dir.path().join(&paths[0]), "x = 1").unwrap();
    agent.record_successful_mutation_paths(paths.to_vec());
    let result = ToolResult {
        content: "passed".into(),
        is_error: false,
        details: Some(serde_json::json!({"exitCode":0})),
    };
    let args = serde_json::json!({"command":"pytest -q"});
    let snapshot = crate::verification::workspace::Snapshot::limited_for_test(dir.path(), &paths);
    agent.record_shell_verification_start("suite", snapshot);
    assert!(agent.mutation_verification_state().unknown_mutation_paths);
    agent.observe_shell_verification("suite", dir.path(), shell_tool(), &args, &result, &result);
    assert_eq!(agent.completion_evidence(), CompletionEvidence::Verified);

    let snapshot = crate::verification::workspace::Snapshot::limited_for_test(dir.path(), &paths);
    agent.record_shell_verification_start("editing-suite", snapshot);
    std::fs::write(dir.path().join(&paths[0]), "x = 2").unwrap();
    agent.observe_shell_verification(
        "editing-suite",
        dir.path(),
        shell_tool(),
        &args,
        &result,
        &result,
    );
    assert_eq!(agent.completion_evidence(), CompletionEvidence::Unverified);
    let snapshot = crate::verification::workspace::Snapshot::limited_for_test(dir.path(), &paths);
    agent.record_shell_verification_start("fresh-suite", snapshot);
    agent.observe_shell_verification(
        "fresh-suite",
        dir.path(),
        shell_tool(),
        &args,
        &result,
        &result,
    );
    assert_eq!(agent.completion_evidence(), CompletionEvidence::Verified);
}

#[test]
fn background_shell_invalidates_coverage_until_terminal_state_and_fresh_verification() {
    use std::process::{Command, Stdio};
    let dir = tempfile::tempdir().unwrap();
    let agent = verifying_agent(dir.path());
    std::fs::write(dir.path().join("changed.py"), "old").unwrap();
    agent.record_successful_mutation_paths(vec![PathBuf::from("changed.py")]);
    agent.record_verification_result(true);
    let child = Command::new("python").args(["-c", "import sys; from pathlib import Path; sys.stdin.readline(); Path('changed.py').write_text('new')"])
        .current_dir(dir.path()).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn().unwrap();
    let job_id = agent
        .tool_context
        .jobs
        .lock()
        .unwrap()
        .register("background edit", child);
    agent
        .verification_starts
        .lock()
        .unwrap()
        .insert("background".into(), 1);
    let result = crate::jobs::started_result(job_id, 0, "background edit");
    agent.observe_shell_verification(
        "background",
        dir.path(),
        shell_tool(),
        &serde_json::json!({"command":"python editor.py", "background":true}),
        &result,
        &result,
    );
    assert_eq!(agent.completion_evidence(), CompletionEvidence::Unverified);
    let suite = crate::verification::classify("bash", "pytest -q", dir.path(), &[]);
    let generation = agent.mutation_verification_state().mutation_generation;
    agent.record_verification_assessment(generation, "pytest", &suite, Some(true));
    assert_eq!(agent.completion_evidence(), CompletionEvidence::Unverified);
    agent
        .tool_context
        .jobs
        .lock()
        .unwrap()
        .write_stdin(job_id, "go\n")
        .unwrap();
    // `wait` returns as soon as the job exits; the bound only has to outlast
    // Python startup on a loaded Windows runner, where 2 s was not enough.
    let result = crate::jobs::output_tool(
        &agent.tool_context.jobs,
        &serde_json::json!({"jobId":job_id, "wait":30}),
        None,
    )
    .unwrap();
    assert_eq!(result.details.unwrap()["exitCode"], 0);
    assert_eq!(agent.completion_evidence(), CompletionEvidence::Unverified);
    let generation = agent.mutation_verification_state().mutation_generation;
    agent.record_verification_assessment(generation, "fresh pytest", &suite, Some(true));
    assert_eq!(agent.completion_evidence(), CompletionEvidence::Verified);
}

#[test]
fn oversized_workspace_keeps_automatic_verification_and_reminders_bounded() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = verifying_agent(dir.path());
    agent.prompt("verify then create a large artifact");
    let mut calls = 0;
    let mut script = scripted_tool_calls(vec![
        (
            "write",
            serde_json::json!({"path":"a.txt", "content":"one"}),
        ),
        (
            shell_tool(),
            serde_json::json!({"command":"python -c \"from pathlib import Path; assert Path('a.txt').read_text() == 'one'\""}),
        ),
        (
            shell_tool(),
            serde_json::json!({"command":"python -c \"from pathlib import Path; Path('large.bin').open('wb').truncate(67108865)\""}),
        ),
    ]);
    agent
        .run_loop(|current| {
            calls += 1;
            assert!(
                calls <= 5,
                "automatic verification must not create a reminder loop"
            );
            script(current)
        })
        .unwrap();
    assert_eq!(calls, 5);
    assert_eq!(harness_runs(&agent), 1);
    assert_eq!(reminders(&agent).len(), 1);
    assert_eq!(agent.completion_evidence(), CompletionEvidence::Unverified);
}

#[test]
fn passing_harness_verification_does_not_relabel_another_scopes_failure() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = verifying_agent(dir.path());
    std::fs::write(dir.path().join("a.txt"), "one").unwrap();
    agent.record_successful_mutation_paths(vec![PathBuf::from("a.txt"), PathBuf::from("b.py")]);
    let failed = crate::verification::Assessment {
        kind: crate::verification::CheckKind::TargetedScript,
        covered: vec![PathBuf::from("b.py")],
        complete: false,
        full_workspace: false,
        reason: "test",
        ran_in: None,
    };
    agent.record_verification_assessment(1, "check b", &failed, Some(false));
    agent.remember_verification_call(shell_tool(), &serde_json::json!({"command":"python -c \"from pathlib import Path; assert Path('a.txt').read_text() == 'one'\""}), dir.path());
    agent.record_successful_mutation_paths(vec![PathBuf::from("a.txt")]);
    agent.prompt("finish");
    agent.run_loop(scripted_tool_calls(Vec::new())).unwrap();
    assert_eq!(harness_runs(&agent), 1);
    let reminders = reminders(&agent);
    assert_eq!(reminders.len(), 1);
    assert!(
        reminders[0].contains("failure remains unresolved"),
        "{}",
        reminders[0]
    );
    assert!(!reminders[0].contains("and it failed"), "{}", reminders[0]);
    assert_eq!(
        agent.completion_evidence(),
        CompletionEvidence::VerificationFailed
    );
}

#[test]
fn overlapping_shell_mutations_record_all_paths_before_rejecting_stale_verification() {
    let dir = tempfile::tempdir().unwrap();
    let agent = verifying_agent(dir.path());
    agent.record_shell_verification_start(
        "a",
        crate::verification::workspace::Snapshot::capture(dir.path()),
    );
    agent.record_shell_verification_start(
        "b",
        crate::verification::workspace::Snapshot::capture(dir.path()),
    );
    let result = ToolResult {
        content: "ok".into(),
        is_error: false,
        details: Some(serde_json::json!({"exitCode":0})),
    };
    let args = serde_json::json!({"command":"python editor.py"});
    std::fs::write(dir.path().join("a.py"), "one").unwrap();
    agent.observe_shell_verification("a", dir.path(), shell_tool(), &args, &result, &result);
    std::fs::write(dir.path().join("b.py"), "two").unwrap();
    agent.observe_shell_verification("b", dir.path(), shell_tool(), &args, &result, &result);
    let state = agent.mutation_verification_state();
    assert!(state.mutation_paths.contains(&PathBuf::from("a.py")));
    assert!(state.mutation_paths.contains(&PathBuf::from("b.py")));
    let assessment = crate::verification::Assessment {
        kind: crate::verification::CheckKind::TargetedScript,
        covered: vec![PathBuf::from("a.py")],
        complete: false,
        full_workspace: false,
        reason: "test",
        ran_in: None,
    };
    agent.record_verification_assessment(
        state.mutation_generation,
        "check a",
        &assessment,
        Some(true),
    );
    assert_eq!(agent.completion_evidence(), CompletionEvidence::Partial);
}

#[test]
fn unsupported_move_marker_does_not_create_mutation_targets() {
    let patch = "*** Begin Patch\n*** Update File: old.py\n*** Move to: new.py\n@@\n-x\n+y\n*** Delete File: removed.py\n*** End Patch";
    let error = crate::apply_patch::parse_codex_patch(patch).unwrap_err();
    assert!(error.to_string().contains("Move to"), "{error}");

    let paths = crate::turn::mutation_paths_from_tool(
        "apply_patch",
        &serde_json::json!({
            "patch": patch
        }),
    );
    assert!(paths.is_empty());
}

#[test]
fn mutation_then_successful_verification_is_verified() {
    let agent = Agent::new("x");

    agent.record_successful_mutation();
    agent.record_verification_result(true);

    assert_eq!(agent.completion_evidence(), CompletionEvidence::Verified);
}

#[test]
fn later_mutation_invalidates_previous_verification() {
    let agent = Agent::new("x");

    agent.record_successful_mutation();
    agent.record_verification_result(true);
    agent.record_successful_mutation();

    assert_eq!(agent.completion_evidence(), CompletionEvidence::Unverified);
}

#[test]
fn last_verification_command_survives_a_later_mutation() {
    let agent = Agent::new("x");
    agent.record_successful_mutation();
    agent.remember_verification_command("bash", "cargo check --help");
    agent.record_verification_command("cargo check --help", true);
    agent.record_successful_mutation();
    assert_eq!(agent.completion_evidence(), CompletionEvidence::Unverified);
    let last = agent
        .mutation_verification_state()
        .last_verification
        .unwrap();
    assert_eq!(last.tool, "bash");
    assert_eq!(last.command, "cargo check --help");
}

#[test]
fn last_verification_preserves_arguments_and_working_directory() {
    let agent = Agent::new("x");
    let cwd = tempfile::tempdir().unwrap();
    let args = serde_json::json!({"command":"cargo check --help", "timeout":17, "workdir":"child"});
    agent.remember_verification_call("powershell", &args, cwd.path());
    agent.record_successful_mutation();
    let last = agent
        .mutation_verification_state()
        .last_verification
        .unwrap();
    assert_eq!(last.arguments, args);
    assert_eq!(last.cwd.as_deref(), Some(cwd.path()));
}

#[test]
fn read_only_turn_does_not_require_verification() {
    let agent = Agent::new("x");

    assert_eq!(agent.completion_evidence(), CompletionEvidence::NotRequired);
}

#[test]
fn tool_search_activates_authorized_deferred_schema() {
    let mut agent = Agent::new("x");
    agent.set_runtime(RuntimeHandle::new(
        RunId::new(),
        AgentId::new(),
        RuntimeBus::new(),
    ));

    assert!(!agent
        .provider_tool_specs()
        .iter()
        .any(|tool| tool.name == "web_search"));

    let result = execute_tool_with(
        Path::new("."),
        "tool_search",
        &serde_json::json!({"query": "web_search"}),
        &agent.tool_context,
    )
    .unwrap();
    let activated = result
        .details
        .as_ref()
        .and_then(|details| details.get("activated"))
        .and_then(serde_json::Value::as_array)
        .unwrap();

    assert!(activated.iter().any(|name| name == "web_search"));
    assert!(agent
        .provider_tool_specs()
        .iter()
        .any(|tool| tool.name == "web_search"));
}

#[test]
fn tool_search_cannot_activate_denied_tool() {
    let mut agent = Agent::new("x");
    agent.set_runtime(RuntimeHandle::new(
        RunId::new(),
        AgentId::new(),
        RuntimeBus::new(),
    ));
    agent.set_active_tools_by_name(&["read".into(), "tool_search".into()]);
    agent.sync_tool_authorization();

    let result = execute_tool_with(
        Path::new("."),
        "tool_search",
        &serde_json::json!({"query": "web_search"}),
        &agent.tool_context,
    )
    .unwrap();
    let activated = result
        .details
        .as_ref()
        .and_then(|details| details.get("activated"))
        .and_then(serde_json::Value::as_array)
        .unwrap();

    assert!(!activated.iter().any(|name| name == "web_search"));
    assert!(!agent
        .provider_tool_specs()
        .iter()
        .any(|tool| tool.name == "web_search"));
}

#[test]
fn tool_activation_changes_schema_identity_once() {
    let mut agent = Agent::new("x");
    agent.set_runtime(RuntimeHandle::new(
        RunId::new(),
        AgentId::new(),
        RuntimeBus::new(),
    ));
    let before = agent.provider_tool_schema_identity();

    execute_tool_with(
        Path::new("."),
        "tool_search",
        &serde_json::json!({"query": "web_search"}),
        &agent.tool_context,
    )
    .unwrap();
    let after = agent.provider_tool_schema_identity();
    assert_ne!(before, after);

    execute_tool_with(
        Path::new("."),
        "tool_search",
        &serde_json::json!({"query": "web_search"}),
        &agent.tool_context,
    )
    .unwrap();
    assert_eq!(after, agent.provider_tool_schema_identity());
}

#[test]
fn deferred_root_schema_ablation_reports_serialized_reduction() {
    let mut agent = Agent::new("x");
    agent.set_runtime(RuntimeHandle::new(
        RunId::new(),
        AgentId::new(),
        RuntimeBus::new(),
    ));

    let deferred = serde_json::to_vec(&agent.provider_tool_specs()).unwrap();
    let deferred_names = agent.visible_tool_names();
    agent.expose_active_tools();
    let full = serde_json::to_vec(&agent.provider_tool_specs()).unwrap();
    let full_names = agent.visible_tool_names();

    assert!(full.len() > deferred.len());
    assert!(deferred_names.is_subset(&full_names));
    assert!(
        deferred.len() * 100 <= full.len() * 70,
        "deferred schemas must save at least 30%: deferred={}, full={}",
        deferred.len(),
        full.len()
    );
    println!(
            "schema_ab deferred_tools={} full_tools={} deferred_bytes={} full_bytes={} withheld_bytes={}",
            deferred_names.len(),
            full_names.len(),
            deferred.len(),
            full.len(),
            full.len().saturating_sub(deferred.len())
        );
}

#[test]
fn batch_duplicate_mutating_calls_executes_once() {
    use davinci_ai::{AssistantMessage, ContentBlock, StopReason};
    use std::sync::atomic::{AtomicUsize, Ordering};

    let counter = Arc::new(AtomicUsize::new(0));
    let c = Arc::clone(&counter);
    let mut agent = Agent::new(default_system_prompt());
    agent.custom_tool_executor = Some(CustomToolExecutor::new(move |_cwd, _tool, _args| {
        c.fetch_add(1, Ordering::SeqCst);
        Ok(crate::ToolResult {
            content: "mutated".into(),
            is_error: false,
            details: None,
        })
    }));
    agent.tools.push("custom_mutating".into());
    agent.prompt("run duplicate batch");

    let mut turn = 0;
    let _ = agent.run_loop(|_| {
        turn += 1;
        if turn == 1 {
            Ok(AssistantMessage {
                extra: Default::default(),
                id: "a1".into(),
                role: "assistant".into(),
                content: vec![
                    ContentBlock::ToolCall {
                        id: "dup_call_1".into(),
                        name: "custom_mutating".into(),
                        arguments: serde_json::json!({"action": "write"}),
                    },
                    ContentBlock::ToolCall {
                        id: "dup_call_1".into(),
                        name: "custom_mutating".into(),
                        arguments: serde_json::json!({"action": "write"}),
                    },
                ],
                model: "fixture".into(),
                usage: None,
                stop_reason: Some(StopReason::ToolUse),
                error_message: None,
            })
        } else {
            Ok(AssistantMessage {
                extra: Default::default(),
                id: "a2".into(),
                role: "assistant".into(),
                content: vec![ContentBlock::Text {
                    text: "done".into(),
                }],
                model: "fixture".into(),
                usage: None,
                stop_reason: Some(StopReason::Stop),
                error_message: None,
            })
        }
    });

    // The mutating custom tool must have executed exactly once!
    assert_eq!(counter.load(Ordering::SeqCst), 1);
}

#[test]
fn concurrent_identical_calls_followers_receive_terminal_result() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration;

    let counter = Arc::new(AtomicUsize::new(0));
    let c = Arc::clone(&counter);
    let mut agent = Agent::new(default_system_prompt());
    agent.custom_tool_executor = Some(CustomToolExecutor::new(move |_cwd, _tool, _args| {
        c.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(50));
        Ok(crate::ToolResult {
            content: "finished_concurrent".into(),
            is_error: false,
            details: None,
        })
    }));
    agent.tools.push("custom_slow".into());
    let agent = Arc::new(agent);

    let mut handles = Vec::new();
    for _ in 0..4 {
        let a = Arc::clone(&agent);
        handles.push(std::thread::spawn(move || {
            let cwd = std::path::PathBuf::from(".");
            a.prepare_and_run_call(
                &cwd,
                "concurrent_call_id",
                "custom_slow",
                &serde_json::json!({"test": 1}),
                0,
            )
        }));
    }

    for h in handles {
        let res = h.join().unwrap();
        assert_eq!(res.content, "finished_concurrent");
        assert!(!res.is_error);
    }

    // Executed exactly once!
    assert_eq!(counter.load(Ordering::SeqCst), 1);
}

#[test]
fn prepare_builtin_prompt_for_user_turn_activates_dormant_capabilities() {
    let mut agent = Agent::new_builtin(PromptProfile::Stable);
    assert!(
        !agent.system_prompt.contains("frontend_design_policy"),
        "Initial stable prompt should not contain dormant capability policy"
    );

    // User asks for visual redesign
    let prepared = agent
        .prepare_builtin_prompt_for_user_turn(
            "Redesign this dashboard so it feels premium and intentional.",
        )
        .unwrap();

    assert!(prepared
        .capabilities
        .is_active(prompt::capabilities::NativeBehaviorCapability::FrontendDesign));
    assert!(agent.system_prompt.contains("frontend_design_policy"));
    assert!(agent
        .prompt_manifest
        .as_ref()
        .unwrap()
        .modules
        .iter()
        .any(|m| m.id == "capability.frontend-design"));

    // Stable prefix and hash MUST remain invariant
    assert_eq!(
        agent.prompt_manifest.as_ref().unwrap().stable_sha256,
        PromptProfile::Stable.bundle().stable_sha256()
    );
}

#[test]
fn runtime_prompt_state_reads_live_plan_and_permission_owners() {
    let mut agent = Agent::new_builtin(PromptProfile::Stable);
    agent.set_permission_mode(PermissionMode::Edits);
    {
        let mut plan = agent.tool_context.living_plan.lock().unwrap();
        plan.revision = 7;
        plan.approved_revision = Some(7);
    }

    let state = agent.runtime_prompt_state();

    assert_eq!(state.permission_mode, PermissionMode::Edits);
    assert_eq!(state.plan_revision, Some(7));
    assert!(state.plan_approved);
    assert!(!state.visual_verification_available);

    agent.set_visual_verification_available(true);
    let prepared = agent
        .prepare_builtin_prompt_for_user_turn("Check the current runtime state.")
        .unwrap();
    assert!(prepared.runtime_state.visual_verification_available);
    assert!(prepared
        .composed
        .dynamic_text
        .contains("Visual verification backend: available."));
}

#[test]
fn runtime_prompt_state_reads_live_contract_owner_not_execution_mode() {
    let mut agent = Agent::new_builtin(PromptProfile::Stable);
    agent.set_permission_mode(PermissionMode::ReadOnly);
    assert!(!agent.runtime_prompt_state().active_contract);

    let contract = crate::runtime::TaskContract::new(
        "runtime-prompt-state-contract",
        1,
        crate::TaskId::new(),
        1,
        vec!["src/".into()],
        vec!["secret.env".into()],
        false,
        vec![],
        vec![],
        vec![],
    )
    .unwrap();
    agent.set_active_contract(contract);

    let prepared = agent
        .prepare_builtin_prompt_for_user_turn("Continue the approved task.")
        .unwrap();
    assert!(prepared.runtime_state.active_contract);
}

#[test]
fn capability_run_state_resets_on_each_real_user_turn_and_observes_events() {
    let mut agent = Agent::new_builtin(PromptProfile::Stable);
    agent.set_visual_verification_available(true);
    agent
        .prepare_builtin_prompt_for_user_turn(
            "Redesign this dashboard so it feels premium and intentional.",
        )
        .unwrap();

    let mut events = Vec::new();
    agent.push_event(
        &mut events,
        AgentEvent::ToolExecutionStart {
            tool_call_id: "edit-1".into(),
            tool_name: "edit".into(),
            args: serde_json::json!({"path": "src/App.tsx"}),
        },
    );
    agent.emit_live(AgentEvent::ToolExecutionEnd {
        tool_call_id: "edit-1".into(),
        tool_name: "edit".into(),
        result: serde_json::json!({}),
        is_error: false,
        details: None,
    });

    let state = agent.capability_run_state();
    assert!(state
        .frontend
        .as_ref()
        .is_some_and(|frontend| frontend.frontend_edit_seen));

    agent
        .prepare_builtin_prompt_for_user_turn("Diagnose the root cause of this failure.")
        .unwrap();
    let state = agent.capability_run_state();
    assert!(state.frontend.is_none());
    assert!(state.debugging.is_some());
    assert!(!state.debugging.as_ref().unwrap().failure_signal_seen);
}

#[test]
fn prepare_builtin_prompt_routes_contextual_review_for_dirty_repo() {
    let dir = tempfile::tempdir().unwrap();
    let status = std::process::Command::new("git")
        .args(["init", "--quiet"])
        .current_dir(dir.path())
        .status()
        .unwrap();
    assert!(status.success());
    std::fs::write(dir.path().join("dirty.rs"), "fn dirty() {}\n").unwrap();

    let mut agent = Agent::new_builtin(PromptProfile::Stable);
    agent.cwd = dir.path().to_path_buf();
    let prepared = agent
        .prepare_builtin_prompt_for_user_turn("sanity check what i just wrote")
        .unwrap();

    assert!(prepared
        .capabilities
        .is_active(prompt::capabilities::NativeBehaviorCapability::CodeReview));
}

#[test]
fn internal_mailbox_messages_do_not_route_capabilities() {
    let mut agent = Agent::new_builtin(PromptProfile::Stable);
    assert!(!agent.system_prompt.contains("code_review_policy"));

    // Internal message via prompt_with (mimicking mailbox drain)
    let _msg = agent.prompt_with(
        "Perform a code review of this PR focusing on security risks.",
        &[],
    );

    // Capabilities must NOT have been activated on the internal prompt_with
    assert!(
        !agent.system_prompt.contains("code_review_policy"),
        "Internal prompt_with must not route capabilities"
    );
    assert!(!agent
        .prompt_manifest
        .as_ref()
        .unwrap()
        .modules
        .iter()
        .any(|m| m.id == "capability.code-review"));

    // Real user turn via prompt_user_with DOES route capabilities
    let _user_msg = agent.prompt_user_with(
        "Perform a code review of this PR focusing on security risks.",
        &[],
    );
    assert!(
        agent.system_prompt.contains("code_review_policy"),
        "Real user turn prompt_user_with must route capabilities"
    );
    assert!(agent
        .prompt_manifest
        .as_ref()
        .unwrap()
        .modules
        .iter()
        .any(|m| m.id == "capability.code-review"));
}

#[test]
fn mailbox_history_is_not_previous_real_user_evidence_for_capability_routing() {
    let run_id = runtime::RunId::new();
    let agent_id = runtime::AgentId::new();
    let sender_id = runtime::AgentId::new();
    let mailbox = runtime::AgentMailbox::new();
    mailbox
        .send(runtime::AgentMessage::new(
            run_id,
            sender_id,
            agent_id,
            "Redesign the UI dashboard layout with visual hierarchy and cards.",
        ))
        .unwrap();
    let runtime = runtime::RuntimeHandle::new(run_id, agent_id, runtime::RuntimeBus::new())
        .with_mailbox(mailbox);

    let mut agent = Agent::new_builtin(PromptProfile::Stable);
    agent.set_runtime(runtime);
    agent.prompt_user_with("Say hello.", &[]);
    agent
        .run_loop(|_| {
            Ok(davinci_ai::AssistantMessage {
                extra: Default::default(),
                id: "mailbox-fixture".into(),
                role: "assistant".into(),
                content: vec![davinci_ai::ContentBlock::Text { text: "ok".into() }],
                model: "fixture".into(),
                usage: None,
                stop_reason: Some(davinci_ai::StopReason::Stop),
                error_message: None,
            })
        })
        .unwrap();

    agent.prompt_user_with("cards", &[]);
    assert!(
        !agent.system_prompt.contains("frontend_design_policy"),
        "internal mailbox text must not become previous_user_request evidence"
    );
}

#[test]
fn custom_replacement_prompt_does_not_route_capabilities() {
    let mut agent = Agent::new("CUSTOM SYSTEM PROMPT");
    let result = agent.prepare_builtin_prompt_for_user_turn(
        "Redesign this dashboard so it feels premium and intentional.",
    );
    assert!(result.is_err());
    assert_eq!(agent.system_prompt, "CUSTOM SYSTEM PROMPT");

    // Calling prompt_user_with also preserves custom replacement
    agent.prompt_user_with(
        "Redesign this dashboard so it feels premium and intentional.",
        &[],
    );
    assert_eq!(agent.system_prompt, "CUSTOM SYSTEM PROMPT");
}

#[test]
fn custom_replacement_real_user_turn_clears_capability_run_state() {
    let mut agent = Agent::new_builtin(PromptProfile::Stable);
    agent
        .prepare_builtin_prompt_for_user_turn(
            "Redesign this dashboard so it feels premium and intentional.",
        )
        .unwrap();
    agent.push_event(
        &mut Vec::new(),
        AgentEvent::ToolExecutionStart {
            tool_call_id: "edit-1".into(),
            tool_name: "edit".into(),
            args: serde_json::json!({"path": "src/App.tsx"}),
        },
    );
    assert!(agent.capability_run_state().frontend.is_some());

    agent.prompt_session = PromptSessionState::custom("CUSTOM SYSTEM PROMPT");
    agent.prompt_user_with("Keep the custom prompt.", &[]);

    let state = agent.capability_run_state();
    assert!(state.active.is_empty());
    assert!(state.frontend.is_none());
    assert!(state.debugging.is_none());
    assert!(state.review.is_none());
}

#[test]
fn subsequent_non_capability_turn_deactivates_capability() {
    let mut agent = Agent::new_builtin(PromptProfile::Stable);

    // Turn 1: Redesign triggers FrontendDesign
    agent.prompt_user_with(
        "Redesign this dashboard so it feels premium and intentional.",
        &[],
    );
    assert!(agent.system_prompt.contains("frontend_design_policy"));

    // Turn 2: Non-capability request deactivates FrontendDesign
    agent.prompt_user_with("What is 2 + 2?", &[]);
    assert!(
        !agent.system_prompt.contains("frontend_design_policy"),
        "Non-capability turn should not carry forward previous capability"
    );
}

#[test]
fn unrelated_successful_test_does_not_verify_mutation() {
    let agent = Agent::new("x");
    agent.record_successful_mutation_paths(vec![PathBuf::from("crates/davinci-agent/src/lib.rs")]);
    agent.record_verification_command("cargo test -p unrelated-crate", true);
    assert_eq!(agent.completion_evidence(), CompletionEvidence::Unverified);
    let state = agent.mutation_verification_state();
    // Production ignores an unrelated success; it cannot change the
    // latest applicable evidence or satisfy the mutation's test coverage.
    assert!(state.latest_evidence.is_none());
    assert!(state.tests_passed_generation.is_none());
}

fn shell_tool() -> &'static str {
    if cfg!(windows) {
        "powershell"
    } else {
        "bash"
    }
}

fn verifying_agent(dir: &std::path::Path) -> Agent {
    let mut agent = Agent::new(default_system_prompt());
    agent.tool_context.foreground_supervisor = Some(crate::command_receipt::test_supervisor());
    agent.cwd = dir.to_path_buf();
    agent.set_permission_mode(PermissionMode::Ask);
    agent.approver = Some(ToolApprover(Arc::new(|_| ToolApprovalDecision::AllowOnce)));
    agent
}

// Unit fixture for coverage-state transitions; actual process receipt creation
// is exercised separately through the supervised command tests.
fn insert_suite_receipt_fixture(agent: &Agent, operation_id: &str) {
    agent.command_receipts.lock().unwrap().push_back(
        crate::runtime::evidence_store::ExecutionReceipt {
            operation_id: operation_id.into(),
            argv: vec!["pytest -q".into()],
            started: true,
            exit_code: Some(0),
            assertion_counts: Some(crate::runtime::AssertionCounts {
                total: 1,
                passed: 1,
                failed: 0,
                skipped: 0,
            }),
            ..Default::default()
        },
    );
}

fn harness_runs(agent: &Agent) -> usize {
    agent
        .messages
        .iter()
        .filter(|message| message.extra.contains_key("davinciHarnessVerification"))
        .count()
}

fn reminders(agent: &Agent) -> Vec<String> {
    agent
        .messages
        .iter()
        .filter(|message| message.extra.contains_key("davinciCapabilityReminder"))
        .map(|message| davinci_ai::content_text(&message.content))
        .collect()
}

#[test]
fn harness_reruns_the_last_verification_after_a_later_edit() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = verifying_agent(dir.path());
    agent.prompt("edit, verify, edit again");
    let mut model_calls = 0;
    let mut script = scripted_tool_calls(vec![
        (
            "write",
            serde_json::json!({"path": "a.txt", "content": "one"}),
        ),
        (
            shell_tool(),
            serde_json::json!({"command": "python -c \"from pathlib import Path; assert Path('a.txt').read_text() in ('one', 'two')\""}),
        ),
        (
            "write",
            serde_json::json!({"path": "a.txt", "content": "two"}),
        ),
    ]);
    agent
        .run_loop(|current| {
            model_calls += 1;
            script(current)
        })
        .unwrap();

    assert_eq!(agent.run_stats().completion_requirement_reminders, 1);
    assert_eq!(
        model_calls, 5,
        "write, verify, write, done, requirement review"
    );
    assert_eq!(harness_runs(&agent), 1);
    assert!(reminders(&agent).is_empty(), "{:?}", reminders(&agent));
    assert_eq!(agent.completion_evidence(), CompletionEvidence::Verified);
    assert_eq!(
        agent.last_assistant_text().as_deref(),
        Some("done"),
        "the harness tool call must not replace the model's reply"
    );
}

#[test]
fn inline_check_finishes_without_extra_model_or_verifier_execution() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = verifying_agent(dir.path());
    agent.prompt("write and check");
    let mut calls = 0;
    let mut script = scripted_tool_calls(vec![
        (
            "write",
            serde_json::json!({"path":"changed.py", "content":"def f(x): return x * 2\n"}),
        ),
        (
            shell_tool(),
            serde_json::json!({"command":"python -c \"from changed import f; assert f(2) == 4\""}),
        ),
    ]);
    agent
        .run_loop(|current| {
            calls += 1;
            script(current)
        })
        .unwrap();
    assert_eq!(
        calls,
        3,
        "{:?}; {:?}",
        agent.mutation_verification_state(),
        reminders(&agent)
    );
    assert_eq!(harness_runs(&agent), 0);
    assert!(reminders(&agent).is_empty());
    assert_eq!(agent.completion_evidence(), CompletionEvidence::Verified);
}

#[test]
fn environment_prefixed_pytest_without_discovery_cannot_verify_source() {
    let dir = tempfile::tempdir().unwrap();
    let agent = verifying_agent(dir.path());
    agent.record_successful_mutation_paths(vec!["changed.py".into()]);
    let generation = agent.mutation_verification_state().mutation_generation;
    let command = "PYTHONDONTWRITEBYTECODE=1 python -m pytest tests/test_changed.py -q";
    let args = serde_json::json!({"command":command});
    agent
        .verification_starts
        .lock()
        .unwrap()
        .insert("check".into(), generation);
    agent.command_receipts.lock().unwrap().push_back(
        crate::runtime::evidence_store::ExecutionReceipt {
            operation_id: "check".into(),
            argv: vec![command.into()],
            started: true,
            exit_code: Some(0),
            ..Default::default()
        },
    );
    let result = ToolResult {
        content: "runner output unavailable".into(),
        is_error: false,
        details: Some(serde_json::json!({"exitCode":0})),
    };
    agent.observe_shell_verification("check", dir.path(), "bash", &args, &result, &result);
    assert_eq!(agent.completion_evidence(), CompletionEvidence::Unverified);
}

#[test]
fn shell_verification_requires_original_terminal_status_and_fresh_dispatch() {
    let dir = tempfile::tempdir().unwrap();
    let agent = verifying_agent(dir.path());
    agent.record_successful_mutation_paths(vec![PathBuf::from("a.py")]);
    let args = serde_json::json!({"command":"pytest -q"});
    for details in [
        None,
        Some(serde_json::json!({"jobId":"running"})),
        Some(serde_json::json!({"exitCode":0,"cancelled":true})),
    ] {
        agent
            .verification_starts
            .lock()
            .unwrap()
            .insert("check".into(), 1);
        let result = ToolResult {
            content: "ok".into(),
            is_error: false,
            details,
        };
        agent.observe_shell_verification(
            "check",
            dir.path(),
            shell_tool(),
            &args,
            &result,
            &result,
        );
        assert_eq!(agent.completion_evidence(), CompletionEvidence::Unverified);
    }
    let result = ToolResult {
        content: "ok".into(),
        is_error: false,
        details: Some(serde_json::json!({"exitCode":0})),
    };
    agent.observe_shell_verification("replay", dir.path(), shell_tool(), &args, &result, &result);
    assert_eq!(agent.completion_evidence(), CompletionEvidence::Unverified);
    agent
        .verification_starts
        .lock()
        .unwrap()
        .insert("check".into(), 1);
    let veto = ToolResult {
        is_error: true,
        ..result.clone()
    };
    agent.observe_shell_verification("check", dir.path(), shell_tool(), &args, &result, &veto);
    assert_eq!(agent.completion_evidence(), CompletionEvidence::Unverified);
    insert_suite_receipt_fixture(&agent, "check");
    agent
        .verification_starts
        .lock()
        .unwrap()
        .insert("check".into(), 1);
    agent.observe_shell_verification("check", dir.path(), shell_tool(), &args, &result, &result);
    assert_eq!(agent.completion_evidence(), CompletionEvidence::Verified);
}

/// Runs one shell result through the production turn path
/// (`observe_shell_verification`) with a terminal receipt.
fn observe_shell_result(
    agent: &Agent,
    id: &str,
    cwd: &std::path::Path,
    command: &str,
    exit_code: i32,
    with_assertions: bool,
) {
    observe_shell_result_with(agent, id, cwd, command, exit_code, with_assertions, |_| {});
}

/// As [`observe_shell_result`], with a hook to mark the receipt (cancelled,
/// denied, simulated, hook-vetoed).
fn observe_shell_result_with(
    agent: &Agent,
    id: &str,
    cwd: &std::path::Path,
    command: &str,
    exit_code: i32,
    with_assertions: bool,
    mark: impl FnOnce(&mut crate::runtime::evidence_store::ExecutionReceipt),
) {
    observe_shell_result_as(
        "bash",
        agent,
        id,
        cwd,
        command,
        exit_code,
        with_assertions,
        mark,
    );
}

/// As [`observe_shell_result_with`], for a named shell tool (`powershell`).
#[allow(clippy::too_many_arguments)]
fn observe_shell_result_as(
    tool: &str,
    agent: &Agent,
    id: &str,
    cwd: &std::path::Path,
    command: &str,
    exit_code: i32,
    with_assertions: bool,
    mark: impl FnOnce(&mut crate::runtime::evidence_store::ExecutionReceipt),
) {
    let generation = agent.mutation_verification_state().mutation_generation;
    agent
        .verification_starts
        .lock()
        .unwrap()
        .insert(id.into(), generation);
    let mut receipt = crate::runtime::evidence_store::ExecutionReceipt {
        operation_id: id.into(),
        argv: vec![command.into()],
        started: true,
        exit_code: Some(exit_code),
        assertion_counts: with_assertions.then_some(crate::runtime::AssertionCounts {
            total: 1,
            passed: u32::from(exit_code == 0),
            failed: u32::from(exit_code != 0),
            skipped: 0,
        }),
        ..Default::default()
    };
    mark(&mut receipt);
    agent.command_receipts.lock().unwrap().push_back(receipt);
    let result = ToolResult {
        content: if exit_code == 0 { "ok" } else { "failed" }.into(),
        is_error: exit_code != 0,
        details: Some(serde_json::json!({"exitCode":exit_code})),
    };
    agent.observe_shell_verification(
        id,
        cwd,
        tool,
        &serde_json::json!({"command":command}),
        &result,
        &result,
    );
}

#[test]
fn failed_syntax_check_revokes_test_coverage_through_the_shell_path() {
    let dir = tempfile::tempdir().unwrap();
    let agent = verifying_agent(dir.path());
    agent.record_successful_mutation_paths(vec!["app.py".into(), "lib.py".into()]);
    let generation = agent.mutation_verification_state().mutation_generation;
    observe_shell_result(
        &agent,
        "t1",
        dir.path(),
        "pytest tests/test_app.py",
        0,
        true,
    );
    assert_eq!(
        agent.mutation_verification_state().tested_paths,
        [PathBuf::from("app.py")]
    );
    // A successful syntax-only check adds no coverage and retires nothing.
    observe_shell_result(
        &agent,
        "s0",
        dir.path(),
        "python -m py_compile app.py",
        0,
        false,
    );
    assert_eq!(
        agent.mutation_verification_state().tested_paths,
        [PathBuf::from("app.py")]
    );
    observe_shell_result(
        &agent,
        "s1",
        dir.path(),
        "python -m py_compile missing.py",
        1,
        false,
    );
    let state = agent.mutation_verification_state();
    assert!(state.tested_paths.is_empty(), "{:?}", state.tested_paths);
    assert_eq!(state.tests_passed_generation, None);
    observe_shell_result(
        &agent,
        "t2",
        dir.path(),
        "pytest tests/test_lib.py",
        0,
        true,
    );
    let state = agent.mutation_verification_state();
    assert_eq!(state.tested_paths, [PathBuf::from("lib.py")]);
    assert_ne!(state.tests_passed_generation, Some(generation));
}

#[test]
fn failed_unknown_check_revokes_test_coverage_through_the_shell_path() {
    let dir = tempfile::tempdir().unwrap();
    let agent = verifying_agent(dir.path());
    agent.record_successful_mutation_paths(vec!["app.py".into(), "lib.py".into()]);
    observe_shell_result(
        &agent,
        "t1",
        dir.path(),
        "pytest tests/test_app.py",
        0,
        true,
    );
    observe_shell_result(&agent, "u0", dir.path(), "make lint", 0, false);
    assert_eq!(
        agent.mutation_verification_state().tested_paths,
        [PathBuf::from("app.py")]
    );
    observe_shell_result(&agent, "u1", dir.path(), "make lint", 1, false);
    assert!(agent.mutation_verification_state().tested_paths.is_empty());
}

#[test]
fn failed_inspection_command_keeps_test_coverage_through_the_shell_path() {
    let dir = tempfile::tempdir().unwrap();
    let agent = verifying_agent(dir.path());
    agent.record_successful_mutation_paths(vec!["app.py".into(), "lib.py".into()]);
    observe_shell_result(
        &agent,
        "t1",
        dir.path(),
        "pytest tests/test_app.py",
        0,
        true,
    );
    let before = agent.mutation_verification_state();
    for (id, command) in [
        ("g1", "grep -r foo ."),
        ("g2", "ls missing"),
        ("g3", "./inspect.sh"),
    ] {
        observe_shell_result(&agent, id, dir.path(), command, 1, false);
        let state = agent.mutation_verification_state();
        assert_eq!(state.tested_paths, [PathBuf::from("app.py")], "{command}");
        assert_eq!(
            state.tests_passed_generation, before.tests_passed_generation,
            "{command}"
        );
    }
}

#[test]
fn failed_unclassified_checker_without_a_summary_revokes_test_coverage_through_the_shell_path() {
    let dir = tempfile::tempdir().unwrap();
    let agent = verifying_agent(dir.path());
    agent.record_successful_mutation_paths(vec!["app.py".into(), "lib.py".into()]);
    observe_shell_result(&agent, "ok", dir.path(), "pytest -q", 0, true);
    assert_eq!(agent.mutation_verification_state().tested_paths.len(), 2);
    // `--no-run` makes this an Unknown check, and a bad feature fails the
    // build before any test summary exists.
    let command = "cargo test --no-run --features nonexistent";
    assert_eq!(
        crate::verification::classify("bash", command, dir.path(), &["app.py".into()]).kind,
        crate::verification::CheckKind::Unknown
    );
    observe_shell_result(&agent, "bad", dir.path(), command, 101, false);
    let state = agent.mutation_verification_state();
    assert!(state.tested_paths.is_empty(), "{:?}", state.tested_paths);
    assert_eq!(state.tests_passed_generation, None);
}

#[test]
fn inline_script_in_a_subdirectory_does_not_credit_exempt_docs_outside_it() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    std::fs::write(dir.path().join("sub/lib.py"), "def f():\n    return 1\n").unwrap();
    let agent = verifying_agent(dir.path());
    agent.record_successful_mutation_paths(vec!["sub/lib.py".into(), "README.md".into()]);
    observe_shell_result(
        &agent,
        "inline",
        &dir.path().join("sub"),
        "python -c \"import lib; assert lib.f() == 1\"",
        0,
        false,
    );
    let state = agent.mutation_verification_state();
    assert_eq!(state.tested_paths, [PathBuf::from("sub/lib.py")]);
    assert_eq!(state.tests_passed_generation, None);
    // Run from the root, the same check does sit above the README.
    std::fs::write(dir.path().join("lib.py"), "def f():\n    return 1\n").unwrap();
    agent.record_successful_mutation_paths(vec!["lib.py".into(), "README.md".into()]);
    observe_shell_result(
        &agent,
        "root",
        dir.path(),
        "python -c \"import lib; assert lib.f() == 1\"",
        0,
        false,
    );
    let state = agent.mutation_verification_state();
    assert!(state.tested_paths.contains(&PathBuf::from("README.md")));
}

#[test]
fn heredoc_check_after_cd_credits_only_the_directory_the_classifier_used() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    std::fs::write(dir.path().join("sub/lib.py"), "def f():\n    return 1\n").unwrap();
    let agent = verifying_agent(dir.path());
    agent.record_successful_mutation_paths(vec!["sub/lib.py".into(), "README.md".into()]);
    let command = "cd sub && python - <<'PY'\nimport lib\nassert lib.f() == 1\nPY";
    observe_shell_result(&agent, "heredoc", dir.path(), command, 0, false);
    let state = agent.mutation_verification_state();
    assert_eq!(state.tested_paths, [PathBuf::from("sub/lib.py")]);
    assert_eq!(state.tests_passed_generation, None);
}

#[test]
fn check_whose_directory_cannot_be_determined_credits_nothing() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    std::fs::write(dir.path().join("sub/lib.py"), "def f():\n    return 1\n").unwrap();
    let agent = verifying_agent(dir.path());
    agent.record_successful_mutation_paths(vec!["sub/lib.py".into(), "README.md".into()]);
    for (id, command) in [
        (
            "var",
            "cd \"$SUBDIR\" && python -c \"import lib; assert lib.f() == 1\"",
        ),
        (
            "late",
            "cd sub && cd . && python -c \"import lib; assert lib.f() == 1\"",
        ),
    ] {
        observe_shell_result(&agent, id, dir.path(), command, 0, false);
        let state = agent.mutation_verification_state();
        assert!(
            state.tested_paths.is_empty(),
            "{command}: {:?}",
            state.tested_paths
        );
    }
    // The coverage step itself refuses to guess a directory.
    let unknown = crate::verification::Assessment {
        kind: crate::verification::CheckKind::TargetedScript,
        covered: vec!["sub/lib.py".into(), "README.md".into()],
        complete: true,
        full_workspace: false,
        reason: "test",
        ran_in: None,
    };
    assert!(crate::verification::targeted_tested_coverage(&unknown, dir.path()).is_empty());
}

#[test]
fn failed_checker_chained_with_an_unclassified_command_revokes_test_coverage() {
    for command in [
        "tsc --noEmit && echo checked",
        "eslint . && echo ok",
        "npx biome check . && echo ok",
        "FOO=1 tsgo -p . && echo ok",
        "python -m py_compile x.py && echo ok",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let agent = verifying_agent(dir.path());
        agent.record_successful_mutation_paths(vec!["app.py".into(), "lib.py".into()]);
        observe_shell_result(
            &agent,
            "t1",
            dir.path(),
            "pytest tests/test_app.py",
            0,
            true,
        );
        assert_eq!(
            agent.mutation_verification_state().tested_paths,
            [PathBuf::from("app.py")]
        );
        // The same chain passing proves nothing and changes nothing.
        observe_shell_result(&agent, "ok", dir.path(), command, 0, false);
        assert_eq!(
            agent.mutation_verification_state().tested_paths,
            [PathBuf::from("app.py")],
            "{command}"
        );
        observe_shell_result(&agent, "bad", dir.path(), command, 2, false);
        let state = agent.mutation_verification_state();
        assert!(
            state.tested_paths.is_empty(),
            "{command}: {:?}",
            state.tested_paths
        );
        assert_eq!(state.tests_passed_generation, None, "{command}");
    }
}

#[test]
fn failed_path_qualified_checker_revokes_test_coverage() {
    for command in [
        "./node_modules/.bin/tsc --noEmit",
        "./node_modules/.bin/eslint .",
        r"node_modules\.bin\tsc.cmd --noEmit",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let agent = verifying_agent(dir.path());
        agent.record_successful_mutation_paths(vec!["app.py".into(), "lib.py".into()]);
        observe_shell_result(
            &agent,
            "t1",
            dir.path(),
            "cd . && pytest tests/test_app.py",
            0,
            true,
        );
        assert_eq!(
            agent.mutation_verification_state().tested_paths,
            [PathBuf::from("app.py")],
            "{command}"
        );
        observe_shell_result(&agent, "bad", dir.path(), command, 2, false);
        let state = agent.mutation_verification_state();
        assert!(
            state.tested_paths.is_empty(),
            "{command}: {:?}",
            state.tested_paths
        );
        observe_shell_result(
            &agent,
            "t2",
            dir.path(),
            "cd . && pytest tests/test_lib.py",
            0,
            true,
        );
        assert_eq!(
            agent.mutation_verification_state().tests_passed_generation,
            None,
            "{command}"
        );
    }
}

#[test]
fn unrecognized_wrapper_script_named_pytest_earns_no_coverage() {
    let dir = tempfile::tempdir().unwrap();
    let agent = verifying_agent(dir.path());
    agent.record_successful_mutation_paths(vec!["app.py".into(), "lib.py".into()]);
    observe_shell_result(
        &agent,
        "w1",
        dir.path(),
        "./tools/pytest tests/test_app.py tests/test_lib.py",
        0,
        true,
    );
    let state = agent.mutation_verification_state();
    assert!(state.tested_paths.is_empty(), "{:?}", state.tested_paths);
    assert_eq!(state.tests_passed_generation, None);
}

// Captured from bench-pubcmp-20261016: t8-calc-r0 and t7-cli-r2.
const PYTEST_PASSED: &str =
    "......                                                                   [100%]\r\n6 passed in 0.01s\r\n";
const UNITTEST_PASSED: &str = "test_top_requires_a_positive_integer (test_wordcount.WordcountTests.test_top_requires_a_positive_integer) ... ok\r\n\r\n----------------------------------------------------------------------\r\nRan 4 tests in 0.242s\r\n\r\nOK\r\n";

/// As [`observe_shell_result`], but the assertion counts come from the
/// captured runner output through the production discovery parser.
fn observe_captured_run(
    agent: &Agent,
    id: &str,
    cwd: &std::path::Path,
    command: &str,
    exit_code: i32,
    stdout: &str,
) {
    observe_shell_result_with(agent, id, cwd, command, exit_code, false, |receipt| {
        receipt.assertion_counts =
            crate::verification::discovery::counts(command, stdout.as_bytes(), b"");
    });
}

#[test]
fn passing_pytest_summary_earns_test_credit_and_skips_the_requirement_review() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["app.py", "lib.py"] {
        std::fs::write(dir.path().join(name), "before").unwrap();
    }
    let mut agent = verifying_agent(dir.path());
    agent.prompt("Handle normal input, reject invalid input; preserve the format.");
    for name in ["app.py", "lib.py"] {
        std::fs::write(dir.path().join(name), "after").unwrap();
    }
    agent.record_successful_mutation_paths(vec!["app.py".into(), "lib.py".into()]);
    let generation = agent.mutation_verification_state().mutation_generation;
    observe_captured_run(&agent, "p1", dir.path(), "pytest -q", 0, PYTEST_PASSED);
    let state = agent.mutation_verification_state();
    assert_eq!(state.tests_passed_generation, Some(generation));
    assert_eq!(agent.completion_evidence(), CompletionEvidence::Verified);
    assert!(!agent.queue_requirement_completion(&mut Vec::new()));
    assert_eq!(agent.stats.completion_requirement_reviews_skipped, 1);
}

#[test]
fn filtered_pytest_runs_earn_no_coverage_and_keep_the_review() {
    for (command, output) in [
        (
            "pytest -q tests/ -k smoke",
            "1 passed, 2 deselected in 0.01s\r\n",
        ),
        ("pytest -q tests/ -k smoke", PYTEST_PASSED),
        ("pytest -q -m fast tests/", PYTEST_PASSED),
        ("pytest -q --lf", PYTEST_PASSED),
    ] {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("tests")).unwrap();
        for name in ["app.py", "lib.py"] {
            std::fs::write(dir.path().join(name), "before").unwrap();
        }
        let mut agent = verifying_agent(dir.path());
        agent.prompt("Handle normal input, reject invalid input; preserve the format.");
        for name in ["app.py", "lib.py"] {
            std::fs::write(dir.path().join(name), "after").unwrap();
        }
        agent.record_successful_mutation_paths(vec!["app.py".into(), "lib.py".into()]);
        observe_captured_run(&agent, "f1", dir.path(), command, 0, output);
        let state = agent.mutation_verification_state();
        assert!(
            state.tested_paths.is_empty(),
            "{command}: {:?}",
            state.tested_paths
        );
        assert!(
            agent.queue_requirement_completion(&mut Vec::new()),
            "{command}"
        );
        assert_eq!(agent.stats.completion_requirement_reviews_skipped, 0);
    }
    // Control: the unfiltered directory run still covers the edited paths.
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("tests")).unwrap();
    for name in ["app.py", "lib.py"] {
        std::fs::write(dir.path().join(name), "before").unwrap();
    }
    let mut agent = verifying_agent(dir.path());
    agent.prompt("Handle normal input, reject invalid input; preserve the format.");
    for name in ["app.py", "lib.py"] {
        std::fs::write(dir.path().join(name), "after").unwrap();
    }
    agent.record_successful_mutation_paths(vec!["app.py".into(), "lib.py".into()]);
    observe_captured_run(
        &agent,
        "f2",
        dir.path(),
        "pytest -q tests/",
        0,
        PYTEST_PASSED,
    );
    assert!(!agent.queue_requirement_completion(&mut Vec::new()));
    assert_eq!(agent.stats.completion_requirement_reviews_skipped, 1);
}

#[test]
fn pytest_without_a_passing_summary_or_exact_runner_earns_no_credit() {
    let dir = tempfile::tempdir().unwrap();
    let agent = verifying_agent(dir.path());
    agent.record_successful_mutation_paths(vec!["app.py".into(), "lib.py".into()]);
    observe_captured_run(
        &agent,
        "w1",
        dir.path(),
        "./tools/pytest -q",
        0,
        PYTEST_PASSED,
    );
    // Exit 5 with nothing collected, and a zero-test summary on a zero exit.
    observe_captured_run(
        &agent,
        "n1",
        dir.path(),
        "pytest -q",
        5,
        "no tests ran in 0.01s\r\n",
    );
    observe_captured_run(
        &agent,
        "n2",
        dir.path(),
        "pytest -q",
        0,
        "no tests ran in 0.01s\r\n",
    );
    // No parsable summary at all.
    observe_captured_run(
        &agent,
        "n3",
        dir.path(),
        "pytest -q",
        0,
        "......  [100%]\r\n",
    );
    let state = agent.mutation_verification_state();
    assert!(state.tested_paths.is_empty(), "{:?}", state.tested_paths);
    assert_eq!(state.tests_passed_generation, None);
    assert_ne!(agent.completion_evidence(), CompletionEvidence::Verified);
}

#[test]
fn failed_pytest_summary_revokes_earlier_coverage() {
    let dir = tempfile::tempdir().unwrap();
    let agent = verifying_agent(dir.path());
    agent.record_successful_mutation_paths(vec!["app.py".into(), "lib.py".into()]);
    observe_captured_run(&agent, "p1", dir.path(), "pytest -q", 0, PYTEST_PASSED);
    assert_eq!(agent.mutation_verification_state().tested_paths.len(), 2);
    observe_captured_run(
        &agent,
        "p2",
        dir.path(),
        "pytest -q",
        1,
        "F.....                                                                   [100%]\r\n1 failed, 5 passed in 0.03s\r\n",
    );
    let state = agent.mutation_verification_state();
    assert!(state.tested_paths.is_empty(), "{:?}", state.tested_paths);
    assert_eq!(state.tests_passed_generation, None);
}

#[test]
fn default_unittest_discovery_covers_edited_paths_but_a_module_selector_does_not() {
    let dir = tempfile::tempdir().unwrap();
    let agent = verifying_agent(dir.path());
    agent.record_successful_mutation_paths(vec!["app.py".into(), "lib.py".into()]);
    observe_captured_run(
        &agent,
        "u1",
        dir.path(),
        "python -m unittest -v test_mod",
        0,
        UNITTEST_PASSED,
    );
    assert!(agent.mutation_verification_state().tested_paths.is_empty());
    observe_captured_run(
        &agent,
        "u2",
        dir.path(),
        "python -m unittest -v",
        0,
        UNITTEST_PASSED,
    );
    let state = agent.mutation_verification_state();
    assert_eq!(
        state.tested_paths,
        [PathBuf::from("app.py"), PathBuf::from("lib.py")]
    );
    assert!(state.tests_passed_generation.is_some());
}

#[test]
fn suffixed_python_does_not_bypass_the_filtered_unittest_exclusion() {
    for program in ["Python.exe", "python.cmd"] {
        let dir = tempfile::tempdir().unwrap();
        let agent = verifying_agent(dir.path());
        agent.record_successful_mutation_paths(vec!["app.py".into(), "lib.py".into()]);
        observe_shell_result(
            &agent,
            "u1",
            dir.path(),
            &format!("{program} -m unittest discover -s tests/ --pattern=test_lib.py"),
            0,
            true,
        );
        let state = agent.mutation_verification_state();
        assert!(
            !state.tested_paths.contains(&PathBuf::from("app.py")),
            "{program}: {:?}",
            state.tested_paths
        );
    }
}

#[test]
fn failed_loosely_spelled_checker_revokes_test_coverage() {
    for (tool, command) in [
        ("bash", "/usr/bin/env -- ./node_modules/.bin/eslint ."),
        ("powershell", r".\node_modules\.bin\eslint.cmd ."),
        ("bash", "env ./node_modules/.bin/eslint ."),
        ("bash", "env ./node_modules/.bin/biome 'check' ."),
        ("bash", "env -- ./node_modules/.bin/eslint ."),
        ("bash", "env -u FOO ./node_modules/.bin/eslint ."),
        ("powershell", r"& '.\tools with spaces\eslint.cmd' ."),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let agent = verifying_agent(dir.path());
        agent.record_successful_mutation_paths(vec!["a.ts".into(), "b.ts".into()]);
        observe_shell_result(
            &agent,
            "t1",
            dir.path(),
            "cd . && npm test -- tests/a.test.ts",
            0,
            true,
        );
        assert!(
            !agent.mutation_verification_state().tested_paths.is_empty(),
            "{command}: precondition"
        );
        observe_shell_result_as(tool, &agent, "bad", dir.path(), command, 2, false, |_| {});
        let state = agent.mutation_verification_state();
        assert!(
            state.tested_paths.is_empty(),
            "{command}: {:?}",
            state.tested_paths
        );
    }
}

#[test]
fn compound_inline_assertion_and_suite_credit_both_paths() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("app.py"), "def f():\n    return 1\n").unwrap();
    let agent = verifying_agent(dir.path());
    agent.record_successful_mutation_paths(vec!["app.py".into(), "lib.py".into()]);
    let generation = agent.mutation_verification_state().mutation_generation;
    observe_shell_result(
        &agent,
        "c1",
        dir.path(),
        "python -c \"import app; assert app.f() == 1\" && pytest tests/test_lib.py",
        0,
        true,
    );
    let state = agent.mutation_verification_state();
    assert!(state.tested_paths.contains(&PathBuf::from("app.py")));
    assert!(state.tested_paths.contains(&PathBuf::from("lib.py")));
    assert_eq!(state.tests_passed_generation, Some(generation));
}

#[test]
fn set_eu_prelude_still_earns_suite_coverage() {
    let dir = tempfile::tempdir().unwrap();
    let agent = verifying_agent(dir.path());
    agent.record_successful_mutation_paths(vec!["app.py".into(), "lib.py".into()]);
    let generation = agent.mutation_verification_state().mutation_generation;
    observe_shell_result(
        &agent,
        "p1",
        dir.path(),
        "set -eu\ncargo test --workspace",
        0,
        true,
    );
    let state = agent.mutation_verification_state();
    assert_eq!(state.tested_paths.len(), 2, "{:?}", state.tested_paths);
    assert_eq!(state.tests_passed_generation, Some(generation));
}

#[test]
fn runs_that_prove_nothing_leave_test_coverage_unchanged_through_the_shell_path() {
    type Mark = fn(&mut crate::runtime::evidence_store::ExecutionReceipt);
    let marks: [(&str, Mark); 4] = [
        ("cancelled", |r| r.cancelled = true),
        ("denied", |r| r.permission_denied = true),
        ("hook vetoed", |r| r.hook_vetoed = true),
        ("simulated", |r| r.simulated = true),
    ];
    for command in [
        "pytest tests/test_lib.py",
        "python -m py_compile lib.py",
        "make lint",
    ] {
        for (label, mark) in marks {
            let dir = tempfile::tempdir().unwrap();
            let agent = verifying_agent(dir.path());
            agent.record_successful_mutation_paths(vec!["app.py".into(), "lib.py".into()]);
            observe_shell_result(
                &agent,
                "t1",
                dir.path(),
                "pytest tests/test_app.py",
                0,
                true,
            );
            let before = agent.mutation_verification_state();
            assert_eq!(before.tested_paths, [PathBuf::from("app.py")]);
            // Started with a nonzero exit code, so only the flag keeps the
            // coverage: it must neither be cleared nor extended.
            observe_shell_result_with(&agent, "x", dir.path(), command, 1, false, mark);
            let after = agent.mutation_verification_state();
            assert_eq!(
                after.tested_paths, before.tested_paths,
                "{label}: {command}"
            );
            assert_eq!(
                after.tests_passed_generation, before.tests_passed_generation,
                "{label}: {command}"
            );
        }
    }
}

#[test]
fn suite_run_in_a_subdirectory_covers_only_paths_under_it() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    let agent = verifying_agent(dir.path());
    agent.record_successful_mutation_paths(vec!["app.py".into(), "sub/lib.py".into()]);
    observe_shell_result(&agent, "sub", &dir.path().join("sub"), "pytest -q", 0, true);
    let state = agent.mutation_verification_state();
    assert_eq!(state.tested_paths, [PathBuf::from("sub/lib.py")]);
    assert_eq!(state.tests_passed_generation, None);
}

#[test]
fn nonzero_test_run_without_a_summary_revokes_test_coverage_through_the_shell_path() {
    let dir = tempfile::tempdir().unwrap();
    let agent = verifying_agent(dir.path());
    agent.record_successful_mutation_paths(vec!["app.py".into(), "lib.py".into()]);
    let generation = agent.mutation_verification_state().mutation_generation;
    observe_shell_result(&agent, "ok", dir.path(), "pytest -q", 0, true);
    let state = agent.mutation_verification_state();
    assert_eq!(state.tests_passed_generation, Some(generation));
    assert_eq!(state.tested_paths.len(), 2);
    // No test summary (a collection error, an unknown package): the exit code
    // is the only evidence, and it is a failure.
    observe_shell_result(
        &agent,
        "bad",
        dir.path(),
        "pytest tests/missing.py",
        4,
        false,
    );
    let state = agent.mutation_verification_state();
    assert!(state.tested_paths.is_empty(), "{:?}", state.tested_paths);
    assert_eq!(state.tests_passed_generation, None);
}

#[test]
fn suite_run_in_a_subdirectory_does_not_credit_exempt_docs_outside_it() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    let agent = verifying_agent(dir.path());
    agent.record_successful_mutation_paths(vec!["sub/lib.py".into(), "README.md".into()]);
    observe_shell_result(&agent, "sub", &dir.path().join("sub"), "pytest -q", 0, true);
    let state = agent.mutation_verification_state();
    assert_eq!(state.tested_paths, [PathBuf::from("sub/lib.py")]);
    assert_eq!(state.tests_passed_generation, None);
    // A root run does test the whole workspace, README included.
    let generation = state.mutation_generation;
    observe_shell_result(&agent, "root", dir.path(), "pytest -q", 0, true);
    let state = agent.mutation_verification_state();
    assert_eq!(state.tests_passed_generation, Some(generation));
    assert!(state.tested_paths.contains(&PathBuf::from("README.md")));
}

#[test]
fn suite_run_at_the_workspace_root_still_covers_every_edited_path() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    let agent = verifying_agent(dir.path());
    agent.record_successful_mutation_paths(vec!["app.py".into(), "sub/lib.py".into()]);
    let generation = agent.mutation_verification_state().mutation_generation;
    observe_shell_result(&agent, "root", dir.path(), "pytest -q", 0, true);
    let state = agent.mutation_verification_state();
    assert_eq!(state.tests_passed_generation, Some(generation));
    assert!(state.tested_paths.contains(&PathBuf::from("app.py")));
    assert!(state.tested_paths.contains(&PathBuf::from("sub/lib.py")));
}

#[test]
fn full_workspace_suite_does_not_credit_a_path_outside_the_workspace_root() {
    let base = tempfile::tempdir().unwrap();
    let repo = base.path().join("repo");
    let shared = base.path().join("shared");
    std::fs::create_dir(&repo).unwrap();
    std::fs::create_dir(&shared).unwrap();
    std::fs::write(repo.join("app.py"), "def f():\n    return 1\n").unwrap();
    std::fs::write(shared.join("config.json"), "ok").unwrap();
    let config = shared.join("config.json");
    let agent = verifying_agent(&repo);
    agent.record_successful_mutation_paths(vec!["app.py".into(), config.clone()]);
    let generation = agent.mutation_verification_state().mutation_generation;
    observe_shell_result(
        &agent,
        "inline",
        &repo,
        "python -c \"import app; assert app.f() == 1; assert open('../shared/config.json').read() == 'ok'\"",
        0,
        false,
    );
    let state = agent.mutation_verification_state();
    assert_eq!(state.tested_paths, [PathBuf::from("app.py")]);
    assert_eq!(state.tests_passed_generation, None);
    // A root run tests everything under the root and nothing beside it.
    observe_shell_result(&agent, "root", &repo, "cargo test", 0, true);
    let state = agent.mutation_verification_state();
    assert_eq!(state.tested_paths, [PathBuf::from("app.py")]);
    assert_eq!(state.tests_passed_generation, None);
    // The extra root earns its own passing run from inside it.
    observe_shell_result(&agent, "shared", &shared, "pytest -q", 0, true);
    let state = agent.mutation_verification_state();
    assert!(
        state.tested_paths.contains(&config),
        "{:?}",
        state.tested_paths
    );
    assert_eq!(state.tests_passed_generation, Some(generation));
}

#[test]
fn batch_check_observes_generation_after_earlier_batch_mutation() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = verifying_agent(dir.path());
    agent.prompt("write and check together");
    let mut calls = 0;
    let mut script = scripted_tool_calls(vec![(
        "batch",
        serde_json::json!({"operations":[
            {"tool":"write", "args":{"path":"changed.py", "content":"def f(x): return x * 2\n"}},
            {"tool":shell_tool(), "args":{"command":"python -c \"from changed import f; assert f(2) == 4\""}}
        ]}),
    )]);
    agent
        .run_loop(|current| {
            calls += 1;
            script(current)
        })
        .unwrap();
    assert_eq!(calls, 2, "{:?}", agent.mutation_verification_state());
    assert_eq!(agent.completion_evidence(), CompletionEvidence::Verified);
    assert_eq!(harness_runs(&agent), 0);
}

#[test]
fn partial_check_discloses_unchecked_paths_after_requirement_review() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = verifying_agent(dir.path());
    agent.prompt("edit two modules and check one");
    let mut calls = 0;
    let mut script = scripted_tool_calls(vec![
        (
            "write",
            serde_json::json!({"path":"changed.py", "content":"def f(x): return x * 2\n"}),
        ),
        (
            "write",
            serde_json::json!({"path":"other.py", "content":"x = 1\n"}),
        ),
        (
            shell_tool(),
            serde_json::json!({"command":"python -c \"from changed import f; assert f(2) == 4\""}),
        ),
    ]);
    let events = agent
        .run_loop(|current| {
            calls += 1;
            script(current)
        })
        .unwrap();
    assert_eq!(agent.run_stats().completion_requirement_reminders, 1);
    assert_eq!(calls, 5);
    assert_eq!(agent.completion_evidence(), CompletionEvidence::Partial);
    assert_eq!(harness_runs(&agent), 0);
    assert!(reminders(&agent).is_empty());
    assert!(events.iter().any(|event| matches!(event,
        crate::AgentEvent::VerificationNotice { status: crate::CompletionStatus::Partial, text, .. }
            if text.contains("unchecked"))));
    assert!(!agent
        .messages
        .iter()
        .any(|message| message.extra.contains_key("davinciVerificationStatus")));
}

#[test]
fn a_passing_rerun_that_misses_the_change_is_not_reported_as_failed() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("unrelated.py"), "value = 1\n").unwrap();
    let mut agent = verifying_agent(dir.path());
    agent.prompt("edit, verify, edit again");
    let mut model_calls = 0;
    let mut script = scripted_tool_calls(vec![
        (
            "write",
            serde_json::json!({"path": "a.txt", "content": "one"}),
        ),
        (
            shell_tool(),
            serde_json::json!({"command": "python -c \"import unrelated; assert unrelated.value == 1\""}),
        ),
        (
            "write",
            serde_json::json!({"path": "a.txt", "content": "two"}),
        ),
    ]);
    agent
        .run_loop(|current| {
            model_calls += 1;
            script(current)
        })
        .unwrap();

    assert_eq!(agent.run_stats().completion_requirement_reminders, 1);
    assert_eq!(harness_runs(&agent), 1, "messages: {:?}", agent.messages);
    let reminders = reminders(&agent);
    assert_eq!(reminders.len(), 1, "{reminders:?}");
    assert!(
        reminders[0].contains("have not completed a verification command"),
        "{}",
        reminders[0]
    );
    assert!(!reminders[0].contains("failed"), "{}", reminders[0]);
    assert_eq!(model_calls, 6);
}

#[test]
fn non_cargo_verifiers_cover_the_files_they_test() {
    let cases = [
        ("pytest -q", true, CompletionEvidence::Verified),
        ("python -m pytest -q", true, CompletionEvidence::Verified),
        (
            "python -m pytest tests/test_intervals.py -q",
            true,
            CompletionEvidence::Verified,
        ),
        ("pytest tests/", true, CompletionEvidence::Verified),
        ("pytest .\\tests\\", true, CompletionEvidence::Verified),
        (
            "pytest tests/test_intervals.py::test_touching",
            true,
            CompletionEvidence::Verified,
        ),
        ("go test ./...", true, CompletionEvidence::Verified),
        ("npm test", true, CompletionEvidence::Verified),
        (
            "pytest tests/test_checkout.py",
            true,
            CompletionEvidence::Unverified,
        ),
        ("pytest -q", false, CompletionEvidence::VerificationFailed),
    ];
    for (command, succeeded, expected) in cases {
        let agent = Agent::new("x");
        agent.record_successful_mutation_paths(vec![PathBuf::from("src/intervals.py")]);
        agent.record_verification_command(command, succeeded);
        assert_eq!(agent.completion_evidence(), expected, "{command}");
    }
}

#[test]
fn a_failing_harness_rerun_is_reported_to_the_model() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = verifying_agent(dir.path());
    agent.prompt("edit, verify, edit again");
    let mut model_calls = 0;
    let mut script = scripted_tool_calls(vec![
        (
            "write",
            serde_json::json!({"path": "a.txt", "content": "one"}),
        ),
        (
            shell_tool(),
            serde_json::json!({"command": "cargo check --definitely-not-a-flag"}),
        ),
        (
            "write",
            serde_json::json!({"path": "a.txt", "content": "two"}),
        ),
    ]);
    agent
        .run_loop(|current| {
            model_calls += 1;
            script(current)
        })
        .unwrap();

    assert_eq!(agent.run_stats().completion_requirement_reminders, 1);
    assert_eq!(harness_runs(&agent), 1);
    let reminders = reminders(&agent);
    assert_eq!(reminders.len(), 1, "{reminders:?}");
    assert!(
        reminders[0].contains("the harness re-ran"),
        "{}",
        reminders[0]
    );
    assert_eq!(
        model_calls, 6,
        "one requirement review and one verification-failure reminder"
    );
}

#[test]
fn auto_verify_off_keeps_the_plain_reminder() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = verifying_agent(dir.path());
    agent.auto_verify = false;
    agent.prompt("edit, verify, edit again");
    let mut script = scripted_tool_calls(vec![
        (
            "write",
            serde_json::json!({"path": "a.txt", "content": "one"}),
        ),
        (
            shell_tool(),
            serde_json::json!({"command": "cargo check --help"}),
        ),
        (
            "write",
            serde_json::json!({"path": "a.txt", "content": "two"}),
        ),
    ]);
    agent.run_loop(|current| script(current)).unwrap();

    assert_eq!(harness_runs(&agent), 0);
    let reminders = reminders(&agent);
    assert_eq!(reminders.len(), 1);
    assert!(reminders[0].contains("have not completed a verification command"));
}

#[test]
fn harness_rerun_respects_permission_denial() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = verifying_agent(dir.path());
    agent.approver = Some(ToolApprover(Arc::new(|_| ToolApprovalDecision::Deny)));
    agent.prompt("finish the change");
    agent.remember_verification_command(shell_tool(), "cargo check --help");
    agent.record_successful_mutation();
    let events = agent.run_loop(scripted_tool_calls(vec![])).unwrap();
    let outcomes = tool_outcomes(&events);
    assert_eq!(outcomes.len(), 1);
    assert!(outcomes[0].1);
    assert!(outcomes[0].2.contains("Permission denied"), "{outcomes:?}");
    assert_eq!(harness_runs(&agent), 1);
    assert_ne!(agent.completion_evidence(), CompletionEvidence::Verified);
}

#[test]
fn failed_command_retains_exit_status_and_relevant_output() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = verifying_agent(dir.path());
    agent.prompt("run the failing command");
    let mut script = scripted_tool_calls(vec![(
        shell_tool(),
        serde_json::json!({"command":"python -c \"import sys; print('widget 7 is missing'); sys.exit(3)\""}),
    )]);
    // What the provider is actually sent on each request.
    let requests = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let seen = requests.clone();
    agent
        .run_loop(move |current| {
            let wire = current
                .messages_for_provider()
                .iter()
                .filter(|message| message.role == "toolResult")
                .map(|message| content_text(&message.content))
                .collect::<Vec<_>>()
                .join(
                    "
---
",
                );
            seen.lock().unwrap().push(wire);
            script(current)
        })
        .unwrap();
    let requests = requests.lock().unwrap();
    let last = requests.last().expect("a request followed the tool result");
    // The output that matters and the exit status both reach the model.
    assert!(last.contains("widget 7 is missing"), "{last}");
    let result = agent
        .messages
        .iter()
        .find(|message| message.role == "toolResult")
        .unwrap();
    assert_eq!(result.is_error, Some(true));
    let exit_code = result
        .extra
        .get("details")
        .and_then(|details| details.get("exitCode"))
        .and_then(serde_json::Value::as_i64)
        .expect("the exit status stays in the tool result details");
    assert_ne!(exit_code, 0);
    assert!(
        last.trim_end()
            .ends_with(&format!("Command exited with code {exit_code}")),
        "{last}"
    );
}

#[test]
fn permission_denied_never_auto_retries_privileged_tool() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = verifying_agent(dir.path());
    let asked = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = asked.clone();
    agent.approver = Some(ToolApprover(Arc::new(move |_| {
        counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        ToolApprovalDecision::Deny
    })));
    agent.prompt("publish the release");
    let marker = dir.path().join("published.txt");
    let command = format!(
        "python -c \"open(r'{}', 'w').write('x')\"",
        marker.display()
    );
    agent
        .run_loop(scripted_tool_calls(vec![(
            shell_tool(),
            serde_json::json!({ "command": command }),
        )]))
        .unwrap();
    assert_eq!(asked.load(std::sync::atomic::Ordering::SeqCst), 1);
    assert!(!marker.exists(), "a denied tool must not run, now or later");
    let denied: Vec<_> = agent
        .messages
        .iter()
        .filter(|message| message.role == "toolResult")
        .collect();
    assert_eq!(denied.len(), 1);
    assert!(content_text(&denied[0].content).contains("Permission denied"));
}

#[test]
fn harness_rerun_preserves_context_and_stops_on_cancellation() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let original = tempfile::tempdir().unwrap();
    let current = tempfile::tempdir().unwrap();
    let mut agent = verifying_agent(current.path());
    agent.prompt("finish the change");
    let args = serde_json::json!({"command":"cargo check --help", "timeout":17});
    agent.remember_verification_call(shell_tool(), &args, original.path());
    agent.record_successful_mutation();
    let cancelled = Arc::new(AtomicBool::new(false));
    agent.abort_signal = Some(cancelled.clone());
    let observed = Arc::new(std::sync::Mutex::new(Vec::new()));
    let capture = observed.clone();
    agent.post_tool = Some(PostToolHook(Arc::new(move |_, cwd, name, args, result| {
        capture
            .lock()
            .unwrap()
            .push((cwd.to_path_buf(), name.to_string(), args.clone()));
        cancelled.store(true, Ordering::SeqCst);
        result
    })));
    let mut model_calls = 0;
    let mut script = scripted_tool_calls(vec![]);
    agent
        .run_loop(|current| {
            model_calls += 1;
            script(current)
        })
        .unwrap();
    assert_eq!(model_calls, 1);
    assert!(agent.abort_requested());
    assert_eq!(
        *observed.lock().unwrap(),
        vec![(
            original.path().to_path_buf(),
            shell_tool().to_string(),
            args
        )]
    );
    assert_eq!(harness_runs(&agent), 1);
    assert!(reminders(&agent).is_empty());
}

#[test]
fn tool_name_scripted_aliases_need_no_recovery_request() {
    for batched in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut agent = verifying_agent(dir.path());
        agent.prompt("write and check");
        let check = if cfg!(windows) {
            "functions.powershell"
        } else {
            "functions.bash"
        };
        let edit_args =
            serde_json::json!({"path":"changed.py", "content":"def f(x): return x * 2\n"});
        let check_args =
            serde_json::json!({"command":"python -c \"from changed import f; assert f(2) == 4\""});
        let operations = if batched {
            vec![(
                "functions.batch",
                serde_json::json!({"operations":[
                    {"tool":"functions.write","args":edit_args},
                    {"tool":check,"args":check_args}
                ]}),
            )]
        } else {
            vec![("functions.write", edit_args), (check, check_args)]
        };
        let mut script = scripted_tool_calls(operations);
        let mut calls = 0;
        let outcome = agent
            .run_loop(|current| {
                calls += 1;
                script(current)
            })
            .unwrap();
        assert_eq!(calls, if batched { 2 } else { 3 });
        assert_eq!(harness_runs(&agent), 0);
        assert!(reminders(&agent).is_empty());
        assert_eq!(agent.completion_evidence(), CompletionEvidence::Verified);
        assert!(tool_outcomes(&outcome).iter().all(|(_, error, _)| !error));
        assert!(format!("{:?}", agent.messages).contains("functions."));
    }
}

#[test]
fn switching_runs_shuts_down_the_previous_team() {
    use crate::runtime::{AgentId, RunId, RuntimeBus, RuntimeHandle};
    let mut agent = crate::Agent::new_builtin(crate::PromptProfile::Stable);
    let first = RuntimeHandle::new(RunId::new(), AgentId::new(), RuntimeBus::new());
    let token = first.team.admit(AgentId::new());
    agent.set_runtime(first);
    agent.set_runtime(RuntimeHandle::new(
        RunId::new(),
        AgentId::new(),
        RuntimeBus::new(),
    ));
    assert!(token.is_cancelled());
}

#[test]
fn a_session_less_conversation_continues_its_runtime_and_team() {
    use crate::runtime::{AgentId, RunId, RuntimeBus, RuntimeHandle};
    let mut agent = crate::Agent::new_builtin(crate::PromptProfile::Stable);
    assert!(agent.session.is_none());
    assert!(agent.runtime_for_next_prompt().is_none());
    let first = RuntimeHandle::new(RunId::new(), AgentId::new(), RuntimeBus::new());
    let teammate = first.team.admit(AgentId::new());
    agent.set_runtime(first.clone());
    let previous = agent
        .runtime_for_next_prompt()
        .expect("--no-session keeps the previous prompt's runtime");
    assert_eq!(previous.run_id, first.run_id);
    // The host builds the next prompt's handle from it, as main.rs does.
    let next = RuntimeHandle::new(RunId::new(), AgentId::new(), RuntimeBus::new())
        .with_session_state_from(previous);
    assert_eq!(next.run_id, first.run_id);
    agent.set_runtime(next);
    assert!(
        !teammate.is_cancelled(),
        "the next prompt must not shut the team down"
    );
}

#[test]
fn a_nested_worker_runtime_is_not_continued_as_a_conversation() {
    use crate::runtime::{AgentId, RunId, RuntimeBus, RuntimeHandle};
    let mut agent = crate::Agent::new_builtin(crate::PromptProfile::Stable);
    let mut worker = RuntimeHandle::new(RunId::new(), AgentId::new(), RuntimeBus::new());
    worker.parent_agent_id = Some(AgentId::new());
    agent.set_runtime(worker);
    assert!(agent.runtime_for_next_prompt().is_none());
}

#[test]
fn a_resumed_conversation_recovers_the_users_delegation_refusal() {
    let user = |text: &str| {
        let mut message = davinci_ai::ChatMessage::text("user", text);
        message.extra.insert(
            "davinciRealUserOrigin".into(),
            serde_json::Value::Bool(true),
        );
        message
    };
    let relayed = davinci_ai::ChatMessage::text("user", "you can use subagents again");
    let refused = [user("fix it. no subagents please"), relayed.clone()];
    assert!(super::delegation_forbidden_from_messages(&refused));
    let reversed = [
        user("fix it. no subagents please"),
        user("you can use subagents again"),
    ];
    assert!(!super::delegation_forbidden_from_messages(&reversed));
}

fn tool_result_texts(agent: &Agent) -> Vec<String> {
    agent
        .messages
        .iter()
        .filter(|message| message.role == "toolResult")
        .map(|message| davinci_ai::content_text(&message.content))
        .collect()
}

#[test]
fn a_print_only_check_is_told_inline_why_it_does_not_count() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = verifying_agent(dir.path());
    agent.auto_verify = false;
    agent.prompt("write and check");
    let mut script = scripted_tool_calls(vec![
        (
            "write",
            serde_json::json!({"path":"changed.py", "content":"def f(x): return x * 2\n"}),
        ),
        (
            shell_tool(),
            serde_json::json!({"command":"python -c \"from changed import f; print(f(2))\""}),
        ),
        (
            shell_tool(),
            serde_json::json!({"command":"python -c \"from changed import f; print(f(3))\""}),
        ),
    ]);
    agent.run_loop(|current| script(current)).unwrap();
    let results = tool_result_texts(&agent);
    let noted: Vec<&String> = results
        .iter()
        .filter(|text| text.contains("[harness: this command does not count as verification"))
        .collect();
    assert_eq!(noted.len(), 1, "once per change: {results:?}");
    assert!(noted[0].contains("changed.py"), "{}", noted[0]);
    assert!(noted[0].contains("asserts nothing"), "{}", noted[0]);
    assert!(noted[0].contains("python - <<'PY'"), "{}", noted[0]);
    // The completion gate stays the backstop and now says why.
    let reminder = agent
        .messages
        .iter()
        .find(|message| message.extra.contains_key("davinciCapabilityReminder"))
        .expect("gate still fires");
    assert_eq!(
        reminder.extra.get("davinciCapabilityReminderReason"),
        Some(&serde_json::json!("no_applicable_check"))
    );
}

#[test]
fn an_asserting_check_gets_no_inline_note() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = verifying_agent(dir.path());
    agent.prompt("write and check");
    let mut script = scripted_tool_calls(vec![
        (
            "write",
            serde_json::json!({"path":"changed.py", "content":"def f(x): return x * 2\n"}),
        ),
        (
            shell_tool(),
            serde_json::json!({"command":"python -c \"from changed import f; assert f(2) == 4\""}),
        ),
    ]);
    agent.run_loop(|current| script(current)).unwrap();
    assert!(tool_result_texts(&agent)
        .iter()
        .all(|text| !text.contains("[harness:")));
    assert!(reminders(&agent).is_empty());
}

#[test]
fn a_failing_command_gets_no_inline_note() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = verifying_agent(dir.path());
    agent.auto_verify = false;
    agent.prompt("write and check");
    let mut script = scripted_tool_calls(vec![
        (
            "write",
            serde_json::json!({"path":"changed.py", "content":"def f(x): return x * 2\n"}),
        ),
        (
            shell_tool(),
            serde_json::json!({"command":"python -c \"import changed; raise SystemExit(3)\""}),
        ),
    ]);
    agent.run_loop(|current| script(current)).unwrap();
    assert!(tool_result_texts(&agent)
        .iter()
        .all(|text| !text.contains("[harness:")));
}

#[test]
fn an_inspection_command_does_not_use_up_the_inline_note() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = verifying_agent(dir.path());
    agent.auto_verify = false;
    agent.prompt("write and check");
    let mut script = scripted_tool_calls(vec![
        (
            "write",
            serde_json::json!({"path":"changed.py", "content":"def f(x): return x * 2\n"}),
        ),
        (
            shell_tool(),
            serde_json::json!({"command":"cat changed.py"}),
        ),
        (
            shell_tool(),
            serde_json::json!({"command":"python -c \"from changed import f; print(f(2))\""}),
        ),
        (
            shell_tool(),
            serde_json::json!({"command":"cat changed.py"}),
        ),
    ]);
    agent.run_loop(|current| script(current)).unwrap();
    let results = tool_result_texts(&agent);
    let noted: Vec<usize> = results
        .iter()
        .enumerate()
        .filter(|(_, text)| text.contains("[harness: this command does not count"))
        .map(|(index, _)| index)
        .collect();
    // Results: write, cat, python, cat. Only the python check is told.
    assert_eq!(noted, vec![2], "{results:?}");
    let reminder = agent
        .messages
        .iter()
        .find(|message| message.extra.contains_key("davinciCapabilityReminder"))
        .expect("gate still fires");
    // The trailing `cat` does not overwrite the reason of the real check.
    assert_eq!(
        reminder.extra.get("davinciCapabilityReminderReason"),
        Some(&serde_json::json!("no_applicable_check"))
    );
}

#[test]
fn checker_programs_match_only_as_the_program_word() {
    use crate::turn::invokes_checker;
    for command in [
        "python -c 'print(1)'",
        "python3.12 -m pytest -q",
        "cd app && npm test",
        "PYTHONPATH=. pytest tests/test_a.py",
        "./venv/bin/python check.py",
        r"C:\Python312\python.exe check.py",
        "uv run pytest",
        "cat a.py | python -",
        "& python x.py",
        "go test ./...",
    ] {
        assert!(invokes_checker(command), "{command}");
    }
    for command in [
        "cat pricing.py",
        "git diff pricing.py",
        "ls node_modules",
        "rg cargo Cargo.toml",
        "echo python",
        "Get-Content app.py",
    ] {
        assert!(!invokes_checker(command), "{command}");
    }
}

#[test]
fn loose_checker_predicate_is_wider_than_the_strict_ones_but_not_a_catch_all() {
    use crate::shell_policy::{invokes_verifier, invokes_verifier_or_checker_loosely};
    use crate::turn::invokes_checker;
    for command in [
        "/usr/bin/env -- ./node_modules/.bin/eslint .",
        r".\node_modules\.bin\eslint.cmd .",
        "env ./node_modules/.bin/eslint .",
        r"node_modules\.bin\TSC.CMD --noEmit",
        "./node_modules/.bin/tsc --noEmit",
        r"C:\Python312\python.bat check.py",
        "FOO=1 ./node_modules/.bin/eslint .",
        "env ./node_modules/.bin/biome 'check' .",
        "env -- ./node_modules/.bin/eslint .",
        "env -u FOO ./node_modules/.bin/eslint .",
        "env --unset=FOO ./node_modules/.bin/eslint .",
        r"& '.\tools with spaces\eslint.cmd' .",
    ] {
        assert!(invokes_verifier_or_checker_loosely(command), "{command}");
        assert!(
            !invokes_verifier(command) && !invokes_checker(command),
            "{command} is already recognized strictly"
        );
    }
    for command in [
        "./tools/cat file",
        "grep x",
        "echo eslint",
        "cat tsc.cmd",
        "grep pytest log.txt",
        "grep 'pytest' log.txt",
    ] {
        assert!(!invokes_verifier_or_checker_loosely(command), "{command}");
    }
}

#[test]
fn gate_reason_distinguishes_failed_and_incomplete_checks() {
    for (command, expected) in [
        (
            "python -c \"from alpha import f; assert f() == 1\"",
            "coverage_incomplete",
        ),
        (
            "python -c \"from alpha import f; assert f() == 99\"",
            "check_failed",
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let mut agent = verifying_agent(dir.path());
        agent.auto_verify = false;
        agent.prompt("write and check");
        let mut script = scripted_tool_calls(vec![
            (
                "write",
                serde_json::json!({"path":"alpha.py","content":"def f(): return 1\n"}),
            ),
            (
                "write",
                serde_json::json!({"path":"beta.py","content":"def g(): return 2\n"}),
            ),
            (shell_tool(), serde_json::json!({"command":command})),
        ]);
        agent.run_loop(|current| script(current)).unwrap();
        assert_eq!(
            agent
                .mutation_verification_state()
                .last_classification_reason
                .as_deref(),
            Some(expected)
        );
    }
}

#[test]
fn execution_isolation_requires_a_confining_sandbox() {
    use davinci_protocol::{SandboxBackendKind, SandboxId, SandboxMode, SandboxSpec};
    let spec = |mode, backend| SandboxSpec {
        id: SandboxId("isolation-fixture".into()),
        mode,
        backend,
        container: None,
        workspace: ".".into(),
        filesystem: Default::default(),
        network: Default::default(),
        environment: Default::default(),
        resources: Default::default(),
        process: Default::default(),
        required_capabilities: Default::default(),
    };
    let mut agent = Agent::new("x");
    assert!(!agent.execution_isolated());
    for (mode, backend, isolated) in [
        (
            SandboxMode::WorkspaceWrite,
            SandboxBackendKind::LinuxBubblewrap,
            true,
        ),
        (
            SandboxMode::Restricted,
            SandboxBackendKind::MacosSeatbelt,
            true,
        ),
        (SandboxMode::WorkspaceWrite, SandboxBackendKind::Host, false),
        (
            SandboxMode::FullAccess,
            SandboxBackendKind::LinuxBubblewrap,
            false,
        ),
    ] {
        agent.tool_context.sandbox = Some(spec(mode, backend));
        assert_eq!(agent.execution_isolated(), isolated, "{mode:?} {backend:?}");
    }
}

#[test]
fn provider_attribution_is_off_unless_the_host_opts_in() {
    // Hosts copy the user's setting onto the agents they configure. Any agent
    // they do not configure, such as a subagent, must not attribute by default.
    assert!(!Agent::new("x").install_telemetry);
}

fn legacy_compaction_agent(dir: &std::path::Path) -> Agent {
    let mut agent = Agent::new("fixture");
    agent.set_context_vm_mode(ContextVmMode::Off);
    agent
        .load_from_session(JsonlSession::create(dir, "fixture", None).unwrap())
        .unwrap();
    agent.set_context_vm_mode(ContextVmMode::Off);
    agent.compaction.keep_recent_tokens = 1;
    agent
}

fn chat_texts(messages: &[ChatMessage]) -> Vec<String> {
    messages
        .iter()
        .map(|message| format!("{}:{}", message.role, content_text(&message.content)))
        .collect()
}

#[test]
fn wor21_repeated_compaction_reloads_the_same_branch_history() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = legacy_compaction_agent(dir.path());
    for turn in 0..4 {
        agent.prompt(&format!("first round question {turn} {}", "x".repeat(400)));
        agent.record_assistant(&format!("first round answer {turn} {}", "y".repeat(400)));
    }
    assert!(agent.compact(None).compacted);
    for turn in 0..4 {
        agent.prompt(&format!("second round question {turn} {}", "x".repeat(400)));
        agent.record_assistant(&format!("second round answer {turn} {}", "y".repeat(400)));
    }
    let second = agent.compact(None);
    assert!(second.compacted, "{}", second.summary);
    let live = chat_texts(&agent.messages);

    let path = agent.session.as_ref().unwrap().path.clone();
    let reopened = JsonlSession::open(&path).unwrap();
    assert_eq!(chat_texts(&messages_from_session(&reopened)), live);
}

/// An agent whose only compaction entry predates the carried-requirements
/// metadata: its summary text is the sole record of what the user restricted.
fn agent_with_legacy_compaction(dir: &std::path::Path, restriction: &str) -> Agent {
    let mut agent = legacy_compaction_agent(dir);
    agent.summarizer = Some(Summarizer::new(|_| {
        Ok(crate::compaction::SummarizeResponse {
            text: "## Goal\nship the import".into(),
            usage: Default::default(),
            stop_reason: Some(davinci_ai::StopReason::Stop),
            error_message: None,
            has_tool_call: false,
        })
    }));
    agent.prompt(&format!("{restriction} {}", "x".repeat(400)));
    agent.record_assistant(&format!("answer {}", "y".repeat(400)));
    for turn in 0..3 {
        agent.prompt(&format!("question {turn} {}", "x".repeat(400)));
        agent.record_assistant(&format!("answer {turn} {}", "y".repeat(400)));
    }
    let first = agent.compact(None);
    assert!(first.compacted, "{}", first.summary);
    let session = agent.session.as_mut().unwrap();
    let entry = session
        .entries
        .iter_mut()
        .find(|entry| entry.entry_type == "compaction")
        .unwrap();
    entry.extra.remove("carriedUserRequirements");
    agent.messages = messages_from_session(agent.session.as_ref().unwrap());
    for turn in 0..3 {
        agent.prompt(&format!("later question {turn} {}", "x".repeat(400)));
        agent.record_assistant(&format!("later answer {turn} {}", "y".repeat(400)));
    }
    agent
}

#[test]
fn a_restriction_recorded_only_in_a_legacy_compaction_summary_survives_the_next_compaction() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = agent_with_legacy_compaction(dir.path(), "Never delete vendor/");
    let second = agent.compact(None);
    assert!(second.compacted, "{}", second.summary);
    let meta =
        crate::compaction_summary_meta(&agent.messages[0]).expect("generated summary metadata");
    // Model-written legacy text is carried verbatim, but never as a user requirement.
    assert!(
        !meta
            .carried_user_requirements
            .iter()
            .any(|entry| entry.contains("Never delete vendor/")),
        "{:?}",
        meta.carried_user_requirements
    );
    assert!(
        meta.carried_legacy_summary
            .as_deref()
            .is_some_and(|text| text.contains("Never delete vendor/")),
        "{meta:?}"
    );
    // And it keeps surviving: the entry is now ordinary carried metadata.
    let path = agent.session.as_ref().unwrap().path.clone();
    let reopened = JsonlSession::open(&path).unwrap();
    let reloaded = messages_from_session(&reopened)
        .iter()
        .find_map(crate::compaction_summary_meta)
        .unwrap();
    assert_eq!(reloaded, meta);
}

#[test]
fn compaction_metadata_survives_a_session_reopen_and_legacy_entries_load_without_it() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = legacy_compaction_agent(dir.path());
    for turn in 0..4 {
        agent.prompt(&format!("question {turn} {}", "x".repeat(400)));
        agent.record_assistant(&format!("answer {turn} {}", "y".repeat(400)));
    }
    let result = agent.compact(None);
    assert!(result.compacted, "{}", result.summary);
    let live = crate::compaction_summary_meta(&agent.messages[0]).expect("live metadata");
    assert!(!live.carried_user_requirements.is_empty());

    let path = agent.session.as_ref().unwrap().path.clone();
    let reopened = JsonlSession::open(&path).unwrap();
    let messages = messages_from_session(&reopened);
    let reloaded = messages
        .iter()
        .find_map(crate::compaction_summary_meta)
        .expect("reloaded metadata");
    assert_eq!(reloaded, live);

    // An entry written before the metadata existed still loads, carrying none.
    let mut entry = reopened
        .entries
        .iter()
        .find(|entry| entry.entry_type == "compaction")
        .unwrap()
        .clone();
    entry.extra.remove("carriedUserRequirements");
    let legacy = crate::entry_to_chat(&entry).expect("legacy entry loads");
    assert!(crate::compaction_summary_meta(&legacy)
        .unwrap()
        .carried_user_requirements
        .is_empty());
}

#[test]
fn wor20_failed_compaction_checkpoint_keeps_live_history() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = legacy_compaction_agent(dir.path());
    for turn in 0..4 {
        agent.prompt(&format!("question {turn} {}", "x".repeat(400)));
        agent.record_assistant(&format!("answer {turn} {}", "y".repeat(400)));
    }
    let before = chat_texts(&agent.messages);
    let path = agent.session.as_ref().unwrap().path.clone();
    let mut readonly = std::fs::metadata(&path).unwrap().permissions();
    readonly.set_readonly(true);
    std::fs::set_permissions(&path, readonly).unwrap();

    let result = agent.compact(None);

    let mut writable = std::fs::metadata(&path).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    writable.set_readonly(false);
    std::fs::set_permissions(&path, writable).unwrap();
    assert!(!result.compacted, "unsaved compaction reported success");
    assert!(result.summary.contains("not saved"), "{}", result.summary);
    assert_eq!(chat_texts(&agent.messages), before);
}

#[test]
fn hybrid_compaction_whose_ledger_cannot_follow_keeps_live_history() {
    use std::sync::atomic::Ordering;
    let mut agent = Agent::new("You are a coding agent.");
    agent.set_runtime(crate::RuntimeHandle::new(
        crate::RunId::new(),
        crate::AgentId::new(),
        crate::RuntimeBus::new(),
    ));
    agent.set_context_vm_mode(crate::runtime::ContextVmMode::Hybrid);
    for turn in 0..6 {
        agent.prompt(&format!("question {turn} {}", "x".repeat(4_000)));
        agent.record_assistant("ok");
    }
    let before = agent.messages.clone();
    let prepared = agent.hybrid_compaction(None, true, 0);
    assert!(prepared.compacted, "{}", prepared.summary);
    let vm = agent.runtime.as_ref().unwrap().context_vm.clone();
    let root = vm.root();
    let resident = vm.resident_source_bytes();

    vm.store.fail_saves.store(true, Ordering::SeqCst);
    let result = agent.commit_compaction(prepared);
    vm.store.fail_saves.store(false, Ordering::SeqCst);

    assert!(!result.compacted, "unrebased compaction reported success");
    assert!(result.summary.contains("not applied"), "{}", result.summary);
    assert_eq!(agent.messages, before);
    assert_eq!(vm.root(), root);
    assert_eq!(vm.resident_source_bytes(), resident);
}

#[test]
fn wor32_abandoned_branch_summary_does_not_seed_the_active_branch() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = legacy_compaction_agent(dir.path());
    // Echo the focus line and the previous summary so each summary shows
    // exactly which previous summary the compaction was seeded with, while
    // staying far smaller than the history it replaces.
    agent.summarizer = Some(crate::compaction::Summarizer::new(|request| {
        let echoed = request
            .prompt
            .lines()
            .filter(|line| line.contains("MARKER") || line.contains("previous-summary"))
            .collect::<Vec<_>>()
            .join(
                "
",
            );
        Ok(crate::compaction::SummarizeResponse {
            text: format!(
                "## Goal
checkpoint
{echoed}"
            ),
            usage: Some(Default::default()),
            stop_reason: Some(davinci_ai::StopReason::Stop),
            error_message: None,
            has_tool_call: false,
        })
    }));
    for turn in 0..3 {
        agent.prompt(&format!("shared question {turn} {}", "x".repeat(400)));
        agent.record_assistant(&format!("shared answer {turn} {}", "y".repeat(400)));
    }
    let branch_point = agent.session.as_ref().unwrap().leaf_id.clone();
    for turn in 0..3 {
        agent.prompt(&format!("branch b question {turn} {}", "x".repeat(400)));
        agent.record_assistant(&format!("branch b answer {turn} {}", "y".repeat(400)));
    }
    let abandoned = agent.compact(Some("ABANDONED-BRANCH-MARKER"));
    assert!(abandoned.compacted);
    assert!(abandoned.summary.contains("ABANDONED-BRANCH-MARKER"));

    let session = agent.session.as_mut().unwrap();
    session.set_leaf(branch_point);
    agent.messages = messages_from_session(agent.session.as_ref().unwrap());
    for turn in 0..3 {
        agent.prompt(&format!("branch a question {turn} {}", "x".repeat(400)));
        agent.record_assistant(&format!("branch a answer {turn} {}", "y".repeat(400)));
    }
    let active = agent.compact(None);
    assert!(active.compacted, "{}", active.summary);
    assert!(
        !active.summary.contains("ABANDONED-BRANCH-MARKER"),
        "abandoned branch summary leaked: {}",
        active.summary
    );
}

fn append_resume_record(agent: &mut Agent, response_id: &str, provider_messages: &[ChatMessage]) {
    let prepared = davinci_ai::PreparedProviderRequest::new(serde_json::json!({
        "model": "fixture",
        "input": [],
    }));
    let turn = davinci_ai::NativeResponsesTurn::from_prepared(
        &prepared,
        davinci_ai::NativeResponsesOutput {
            response_id: Some(response_id.into()),
            output_items: vec![],
            final_response: None,
            terminal_event_type: "response.completed".into(),
        },
    )
    .unwrap();
    let record = davinci_ai::NativeResponsesResumeRecord {
        turn,
        resume_provider_message_count: provider_messages.len(),
        resume_provider_messages_fingerprint: davinci_ai::provider_messages_fingerprint(
            provider_messages,
        ),
    };
    let mut extra = serde_json::Map::new();
    extra.insert("data".into(), serde_json::to_value(record).unwrap());
    agent
        .session
        .as_mut()
        .unwrap()
        .append_entry(SessionEntry {
            id: String::new(),
            entry_type: "custom".into(),
            parent_id: None,
            seq: 0,
            timestamp: 0,
            message: None,
            custom_type: Some(davinci_ai::NATIVE_RESPONSES_TURN_ENTRY_TYPE.into()),
            extra,
        })
        .unwrap();
}

fn resume_response_id(record: Option<davinci_ai::NativeResponsesResumeRecord>) -> Option<String> {
    record.and_then(|record| record.turn.output.response_id)
}

#[test]
fn wor28_native_resume_record_comes_from_the_active_branch() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = legacy_compaction_agent(dir.path());
    agent.prompt("shared question");
    agent.record_assistant("shared answer");
    let provider_a = agent.messages.clone();
    append_resume_record(&mut agent, "resp_branch_a", &provider_a);
    let branch_a_leaf = agent.session.as_ref().unwrap().leaf_id.clone();

    agent.prompt("branch b question");
    agent.record_assistant("branch b answer");
    let provider_b = agent.messages.clone();
    append_resume_record(&mut agent, "resp_branch_b", &provider_b);
    assert_eq!(
        resume_response_id(agent.native_responses_resume_record()).as_deref(),
        Some("resp_branch_b")
    );

    agent.session.as_mut().unwrap().set_leaf(branch_a_leaf);
    agent.messages = messages_from_session(agent.session.as_ref().unwrap());
    assert_eq!(
        resume_response_id(agent.native_responses_resume_record()).as_deref(),
        Some("resp_branch_a"),
        "a newer record from the abandoned branch hid this branch's own one"
    );
    assert_eq!(
        resume_response_id(agent.native_responses_resume_record_for(&provider_a)).as_deref(),
        Some("resp_branch_a")
    );
}

#[test]
fn wor39_native_resume_falls_back_to_a_compatible_ancestor_record() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = legacy_compaction_agent(dir.path());
    agent.prompt("first question");
    agent.record_assistant("first answer");
    let compatible = agent.messages.clone();
    append_resume_record(&mut agent, "resp_compatible", &compatible);
    agent.prompt("second question");
    agent.record_assistant("second answer");
    let mut rewritten = agent.messages.clone();
    rewritten[2] = ChatMessage::text("user", "a projection the next request will not contain");
    append_resume_record(&mut agent, "resp_stale", &rewritten);

    let request = agent.messages.clone();
    assert_eq!(
        resume_response_id(agent.native_responses_resume_record()).as_deref(),
        Some("resp_stale")
    );
    assert_eq!(
        resume_response_id(agent.native_responses_resume_record_for(&request)).as_deref(),
        Some("resp_compatible")
    );
    assert_eq!(
        agent.native_responses_resume_record_for(&request[..1]),
        None
    );
}

#[test]
fn a_full_context_with_failed_auto_compaction_blocks_the_request() {
    let mut agent = Agent::new("fixture");
    agent.context_window = 2_000;
    agent.compaction.enabled = true;
    agent.compaction.reserve_tokens = 100;
    agent.compaction.keep_recent_tokens = 1;
    agent.summarizer = Some(Summarizer::new(|_| Err("provider timed out".into())));
    for i in 0..6 {
        agent.prompt(&format!("request {i}: {}", "background ".repeat(250)));
        agent.record_assistant(&format!("reply {i}: {}", "detail ".repeat(400)));
    }
    let before = agent.messages.len();
    let mut dispatched = false;
    let error = agent
        .run_loop(|_| {
            dispatched = true;
            Err::<crate::CompleteOutput, String>("must not be reached".into())
        })
        .unwrap_err();
    assert!(!dispatched, "no provider request may follow the failure");
    assert!(error.contains("automatic compaction failed"), "{error}");
    // The history is real user text over the model's window: it cannot be
    // capped, so the summarizer is never asked and the request is refused.
    assert!(error.contains("CONTEXT_ADMISSION_FAILED"), "{error}");
    assert_eq!(agent.messages.len(), before, "history is untouched");
}

#[test]
fn failed_auto_compaction_with_headroom_still_sends_the_request() {
    let mut agent = Agent::new("fixture");
    agent.context_window = 200_000;
    agent.compaction.enabled = true;
    agent.compaction.reserve_tokens = 199_000;
    agent.summarizer = Some(Summarizer::new(|_| Err("provider timed out".into())));
    agent.prompt(&"background ".repeat(2_000));
    let mut dispatched = false;
    let _ = agent.run_loop(|_| {
        dispatched = true;
        Err::<crate::CompleteOutput, String>("stop here".into())
    });
    assert!(dispatched, "headroom left: the turn proceeds");
}

#[test]
fn switching_the_tree_branch_restores_that_branchs_verification_and_exposure() {
    let dir = tempfile::tempdir().unwrap();
    let sessions = tempfile::tempdir().unwrap();
    let mut agent = verifying_agent(dir.path());
    agent
        .load_from_session(JsonlSession::create(sessions.path(), "tree", None).unwrap())
        .unwrap();
    agent.prompt("start");
    agent.record_assistant("first answer");
    let fork = agent
        .session
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .find(|entry| {
            entry.entry_type == "message"
                && entry
                    .message
                    .as_ref()
                    .and_then(|message| message.get("role"))
                    .and_then(|role| role.as_str())
                    == Some("assistant")
        })
        .map(|entry| entry.id.clone())
        .expect("an assistant entry to fork from");
    let initial = agent
        .tool_context
        .tool_exposure
        .lock()
        .unwrap()
        .visible_names()
        .clone();
    let deferred = agent
        .tools
        .iter()
        .find(|name| !initial.contains(*name))
        .cloned()
        .expect("a tool that starts deferred");

    // Work done after the fork: an unverified edit and a discovered tool.
    agent.prompt("edit a file");
    agent.record_assistant("second answer");
    std::fs::write(dir.path().join("a.txt"), "one").unwrap();
    agent.record_successful_mutation_paths(vec![PathBuf::from("a.txt")]);
    agent
        .tool_context
        .tool_exposure
        .lock()
        .unwrap()
        .activate_authorized(&deferred, true);
    agent.persist_mutation_verification();
    agent.persist_tool_exposure();
    assert_eq!(agent.mutation_verification_state().mutation_generation, 1);
    assert!(agent
        .tool_context
        .tool_exposure
        .lock()
        .unwrap()
        .discovered_names()
        .contains(&deferred));

    // Back to the fork: that branch never saw the edit or the tool.
    agent
        .navigate_tree_entry(&fork, false, None, false, 16_384)
        .unwrap();
    assert_eq!(
        agent.mutation_verification_state(),
        MutationVerificationState::default(),
        "the abandoned branch's edit is still live"
    );
    assert!(
        !agent
            .tool_context
            .tool_exposure
            .lock()
            .unwrap()
            .discovered_names()
            .contains(&deferred),
        "the abandoned branch's discovered tool is still exposed"
    );
    // Nothing from the other branch is written onto this one.
    let customs = |agent: &Agent| {
        agent
            .session
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .filter(|entry| entry.entry_type == "custom")
            .count()
    };
    let before = customs(&agent);
    agent.persist_mutation_verification();
    agent.persist_tool_exposure();
    assert_eq!(customs(&agent), before);
}

fn effect_marker(custom_type: &str, id: &str) -> SessionEntry {
    let mut entry = SessionEntry::message("user", serde_json::json!([]));
    entry.entry_type = "custom".into();
    entry.message = None;
    entry.custom_type = Some(custom_type.into());
    entry
        .extra
        .insert("data".into(), serde_json::json!({ "id": id }));
    entry
}

fn tool_call_entry(id: &str, name: &str) -> SessionEntry {
    SessionEntry::message(
        "assistant",
        serde_json::json!([{"type":"toolCall","id":id,"name":name,"arguments":{}}]),
    )
}

fn tool_result_entry(id: &str) -> SessionEntry {
    let mut entry = SessionEntry::message("toolResult", serde_json::json!([]));
    entry.message.as_mut().unwrap()["toolCallId"] = id.into();
    entry
}

fn results_for<'a>(agent: &'a Agent, id: &str) -> Vec<&'a ChatMessage> {
    agent
        .messages
        .iter()
        .filter(|m| m.role == "toolResult" && m.tool_call_id.as_deref() == Some(id))
        .collect()
}

#[test]
fn b3_an_unresolved_call_is_closed_before_the_next_request_and_never_rerun() {
    // (ledger saw the call start, expected recovery outcome, expected text)
    for (started, outcome, text) in [
        (true, "unknown", "Outcome unknown"),
        (false, "not_executed", "Not executed"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = seeded_session(
            dir.path(),
            vec![
                SessionEntry::message(
                    "user",
                    serde_json::json!([{"type": "text", "text": "write it"}]),
                ),
                tool_call_entry("c1", "write"),
            ],
        );
        let args = serde_json::json!({"path": "x.txt", "content": "y"});
        let mut agent = resume(&path);
        agent.cwd = dir.path().to_path_buf();
        agent.set_permission_mode(PermissionMode::AlwaysApprove);
        if started {
            agent
                .tool_ledger
                .lock()
                .unwrap()
                .record_start("c1", "write", &args);
        }

        agent.prompt("continue");

        let closed = results_for(&agent, "c1");
        assert_eq!(closed.len(), 1, "started={started}: exactly one result");
        assert_eq!(closed[0].is_error, Some(true));
        assert!(
            davinci_ai::content_text(&closed[0].content).contains(text),
            "started={started}"
        );
        assert_eq!(
            closed[0].extra["details"]["toolRecovery"]["outcome"], outcome,
            "started={started}"
        );
        // The call and its closing result stay adjacent, and the result is
        // durable in the session file, not only in memory.
        let call_at = agent
            .messages
            .iter()
            .position(|m| m.role == "assistant")
            .unwrap();
        assert_eq!(agent.messages[call_at + 1].role, "toolResult");
        let reopened = JsonlSession::open(&path).unwrap();
        assert!(reopened
            .entries
            .iter()
            .any(|e| e.message.as_ref().is_some_and(|m| m["toolCallId"] == "c1")));

        if started {
            // A possibly-committed call is not run again under its old id: the
            // ledger still holds it as in flight, so a re-issue is no leader.
            let again = agent
                .tool_ledger
                .lock()
                .unwrap()
                .reserve_call("c1", "write", &args)
                .unwrap();
            assert!(
                !matches!(again, crate::tool_ledger::ReservationOutcome::Reserved),
                "a possibly-committed call could be executed again"
            );
            assert!(!dir.path().join("x.txt").exists());
        }
    }
}

#[test]
fn b3_an_orphan_tool_result_is_refused_before_any_provider_request() {
    let dir = tempfile::tempdir().unwrap();
    let path = seeded_session(
        dir.path(),
        vec![
            SessionEntry::message(
                "user",
                serde_json::json!([{"type": "text", "text": "hello"}]),
            ),
            tool_result_entry("ghost"),
        ],
    );
    let mut agent = resume(&path);
    agent.prompt("next");
    let requests = std::cell::Cell::new(0);
    let mut model = scripted_tool_calls(vec![]);
    let error = agent
        .run_loop(|current| {
            requests.set(requests.get() + 1);
            model(current)
        })
        .unwrap_err();
    assert!(
        error.contains("ghost") && error.contains("no matching tool call"),
        "{error}"
    );
    assert_eq!(
        requests.get(),
        0,
        "the unpaired history reached the provider"
    );
}

#[test]
fn b3_a_fork_does_not_inherit_the_parents_ledger_records() {
    let dir = tempfile::tempdir().unwrap();
    let path = seeded_session(
        dir.path(),
        vec![
            SessionEntry::message("user", serde_json::json!([{"type": "text", "text": "go"}])),
            tool_call_entry("live", "write"),
            tool_result_entry("done"),
        ],
    );
    let args = serde_json::json!({"path": "x.txt", "content": "y"});
    let parent = resume(&path);
    {
        let mut ledger = parent.tool_ledger.lock().unwrap();
        ledger.reserve_call("live", "write", &args).unwrap();
        ledger.record_start("live", "write", &args);
        ledger.reserve_call("done", "write", &args).unwrap();
        ledger.record_start("done", "write", &args);
        ledger.record_completion("done", "ok", false);
        assert!(ledger.get_completed_result("done").is_some());
    }

    let source = JsonlSession::open(&path).unwrap();
    let leaf = source.leaf_id.clone().unwrap();
    let fork = source.fork(&leaf, &dir.path().join("forks")).unwrap();
    assert_ne!(fork.header.id, source.header.id);
    assert_eq!(
        fork.header.parent_session_id.as_deref(),
        Some(source.header.id.as_str())
    );
    assert_ne!(
        fork.path.with_extension("tool-ledger.json"),
        path.with_extension("tool-ledger.json"),
        "a fork must not share the parent's ledger file"
    );
    let child = resume(&fork.path.clone());

    let ledger = child.tool_ledger.lock().unwrap();
    assert!(
        ledger.records().is_empty(),
        "the fork inherited ledger records"
    );
    assert!(ledger.get_completed_result("done").is_none());
}

/// Evidence that classifies as `Verified`, not merely a recorded generation.
fn verified_state(generation: u64) -> MutationVerificationState {
    MutationVerificationState {
        mutation_generation: generation,
        verified_generation: Some(generation),
        last_verification_succeeded: true,
        latest_evidence: Some(VerificationEvidence {
            generation,
            command: "cargo test".into(),
            succeeded: true,
            mutation_paths: Vec::new(),
            verification_targets: Vec::new(),
            coverage: VerificationCoverage::Broad,
        }),
        ..MutationVerificationState::default()
    }
}

fn verified_state_entry(generation: u64) -> SessionEntry {
    let mut entry = effect_marker(MUTATION_VERIFICATION_ENTRY_TYPE, "unused");
    entry.extra.insert(
        "data".into(),
        serde_json::to_value(verified_state(generation)).unwrap(),
    );
    entry
}

/// Write `entries` to a fresh session and return its path.
fn seeded_session(dir: &Path, entries: Vec<SessionEntry>) -> PathBuf {
    let mut session = JsonlSession::create(dir, "effects", None).unwrap();
    for entry in entries {
        session.append_entry(entry).unwrap();
    }
    session.path.clone()
}

fn resume(path: &Path) -> Agent {
    let mut agent = Agent::new(default_system_prompt());
    agent
        .load_from_session(JsonlSession::open(path).unwrap())
        .unwrap();
    agent
}

#[test]
fn resume_leaves_verification_alone_for_read_only_unresolved_calls() {
    let dir = tempfile::tempdir().unwrap();
    let path = seeded_session(
        dir.path(),
        vec![verified_state_entry(2), tool_call_entry("c1", "read")],
    );
    let agent = resume(&path);
    let state = agent.mutation_verification_state();
    assert_eq!(state.mutation_generation, 2);
    assert_eq!(state.verified_generation, Some(2));
    assert_eq!(agent.completion_evidence(), CompletionEvidence::Verified);
}

#[test]
fn resume_invalidates_for_unresolved_calls_that_may_mutate_and_bumps_once() {
    // `batch` and `codemode` run other tools, an unregistered name is unknown,
    // and `mcp_read` and `lsp_*` are read-class yet run external code.
    for tool in [
        "write",
        "bash",
        "mystery_tool",
        "batch",
        "codemode",
        "mcp_read",
        "lsp_hover",
    ] {
        let dir = tempfile::tempdir().unwrap();
        let path = seeded_session(
            dir.path(),
            vec![verified_state_entry(2), tool_call_entry("c1", tool)],
        );
        let seeded = seeded_session(dir.path(), vec![verified_state_entry(2)]);
        assert_eq!(
            resume(&seeded).completion_evidence(),
            CompletionEvidence::Verified,
            "{tool}: the seed must start verified"
        );
        for _ in 0..3 {
            let agent = resume(&path);
            let state = agent.mutation_verification_state();
            assert_eq!(state.mutation_generation, 3, "{tool}");
            assert_eq!(state.verified_generation, None, "{tool}");
            assert!(state.unknown_mutation_paths, "{tool}");
            assert_eq!(
                agent.completion_evidence(),
                CompletionEvidence::Unverified,
                "{tool}"
            );
        }
    }
}

#[test]
fn resume_invalidates_for_a_pending_effect_marker_and_bumps_once() {
    let dir = tempfile::tempdir().unwrap();
    let path = seeded_session(
        dir.path(),
        vec![
            verified_state_entry(4),
            effect_marker(MUTATION_EFFECT_PENDING_ENTRY_TYPE, "effect-1"),
        ],
    );
    let seeded = seeded_session(dir.path(), vec![verified_state_entry(4)]);
    assert_eq!(
        resume(&seeded).completion_evidence(),
        CompletionEvidence::Verified
    );
    for _ in 0..3 {
        let agent = resume(&path);
        assert_eq!(agent.mutation_verification_state().mutation_generation, 5);
        assert_eq!(agent.completion_evidence(), CompletionEvidence::Unverified);
    }
}

#[test]
fn an_interrupted_effect_revokes_earlier_test_credit() {
    let dir = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let paths = vec![PathBuf::from("a.rs"), PathBuf::from("b.rs")];
    let mut tested = verified_state(7);
    tested.mutation_paths = paths.clone();
    tested.covered_paths = paths.clone();
    tested.tested_paths = paths;
    tested.tests_passed_generation = Some(7);
    let mut seed = effect_marker(MUTATION_VERIFICATION_ENTRY_TYPE, "unused");
    seed.extra
        .insert("data".into(), serde_json::to_value(tested).unwrap());
    let path = seeded_session(
        dir.path(),
        vec![
            seed,
            effect_marker(MUTATION_EFFECT_PENDING_ENTRY_TYPE, "effect-1"),
        ],
    );
    let mut agent = verifying_agent(work.path());
    agent
        .load_from_session(JsonlSession::open(&path).unwrap())
        .unwrap();
    let state = agent.mutation_verification_state();
    assert_eq!(state.mutation_generation, 8);
    assert!(state.tested_paths.is_empty());
    assert_eq!(state.tests_passed_generation, None);
    // A later build-only check must not promote the stale test credit.
    observe_shell_result(
        &agent,
        "c1",
        work.path(),
        "cargo check --workspace",
        0,
        false,
    );
    let state = agent.mutation_verification_state();
    assert_eq!(state.tests_passed_generation, None);
}

#[test]
fn a_reused_call_id_cannot_hide_a_mutating_call() {
    for calls in [
        [("bash", "x"), ("read", "x")],
        [("read", "x"), ("bash", "x")],
    ] {
        let dir = tempfile::tempdir().unwrap();
        let content: Vec<_> = calls
            .iter()
            .map(|(name, id)| serde_json::json!({"type":"toolCall","id":id,"name":name,"arguments":{}}))
            .collect();
        let path = seeded_session(
            dir.path(),
            vec![
                verified_state_entry(2),
                SessionEntry::message("assistant", serde_json::Value::Array(content)),
            ],
        );
        let agent = resume(&path);
        assert_eq!(agent.mutation_verification_state().mutation_generation, 3);
        assert_eq!(agent.completion_evidence(), CompletionEvidence::Unverified);
    }
}

#[test]
fn resume_ignores_resolved_effects_and_answered_calls() {
    let dir = tempfile::tempdir().unwrap();
    let path = seeded_session(
        dir.path(),
        vec![
            verified_state_entry(2),
            effect_marker(MUTATION_EFFECT_PENDING_ENTRY_TYPE, "effect-1"),
            effect_marker(MUTATION_EFFECT_RESOLVED_ENTRY_TYPE, "effect-1"),
            tool_call_entry("c1", "write"),
            tool_result_entry("c1"),
        ],
    );
    let agent = resume(&path);
    let state = agent.mutation_verification_state();
    assert_eq!(state.mutation_generation, 2);
    assert_eq!(state.verified_generation, Some(2));
}

#[test]
fn an_interrupted_effect_from_generation_zero_is_unverified_not_unrequired() {
    let dir = tempfile::tempdir().unwrap();
    let path = seeded_session(dir.path(), vec![tool_call_entry("c1", "write")]);
    let agent = resume(&path);
    assert_eq!(agent.mutation_verification_state().mutation_generation, 1);
    assert_eq!(agent.completion_evidence(), CompletionEvidence::Unverified);
}

mod hybrid_fold_economics {
    //! Hybrid compaction must not pay for a summarizer request it cannot use.
    use super::*;
    use crate::compaction::{SummarizeResponse, Summarizer};
    use std::cell::Cell;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    thread_local! {
        /// Request bytes the test estimator reports (the estimator is a plain
        /// `fn`). Per test thread, so parallel tests cannot see each other.
        static OVERHEAD_BYTES: Cell<u64> = const { Cell::new(0) };
    }

    fn overhead_estimator(_: &Agent) -> u64 {
        OVERHEAD_BYTES.with(Cell::get)
    }

    fn summarizer(
        calls: Arc<AtomicUsize>,
        on_call: impl Fn() + Send + Sync + 'static,
    ) -> Summarizer {
        Summarizer::new(move |request| {
            calls.fetch_add(1, Ordering::SeqCst);
            on_call();
            Ok(SummarizeResponse {
                text: if request.label == "context state fold" {
                    "{}".into()
                } else {
                    "## Goal\nContinue.".into()
                },
                usage: Some(Default::default()),
                stop_reason: Some(davinci_ai::StopReason::Stop),
                error_message: None,
                has_tool_call: false,
            })
        })
    }

    /// Hybrid agent: compacts at 10k tokens, keeps 1k verbatim; the capacity
    /// limit (window minus reserve) is 38k.
    fn agent(calls: Arc<AtomicUsize>, on_call: impl Fn() + Send + Sync + 'static) -> Agent {
        configure(Agent::new("You are a coding agent."), calls, on_call)
    }

    fn configure(
        mut agent: Agent,
        calls: Arc<AtomicUsize>,
        on_call: impl Fn() + Send + Sync + 'static,
    ) -> Agent {
        agent.set_runtime(crate::RuntimeHandle::new(
            crate::RunId::new(),
            crate::AgentId::new(),
            crate::RuntimeBus::new(),
        ));
        agent.set_context_vm_mode(crate::runtime::ContextVmMode::Hybrid);
        agent.context_window = 40_000;
        agent.compaction = CompactionSettings {
            reserve_tokens: 2_000,
            keep_recent_tokens: 1_000,
            threshold: Some(crate::compaction::CompactionThreshold::Tokens(10_000)),
            ..CompactionSettings::default()
        };
        agent.summarizer = Some(summarizer(calls, on_call));
        agent
    }

    /// `turns` exchanges of about `tokens` tokens each, after a constraint.
    fn fill(agent: &mut Agent, turns: usize, tokens: usize) {
        agent.prompt("Build the scheduler. Never modify files under migrations/.");
        agent.record_assistant("ok");
        for turn in 0..turns {
            agent.prompt(&format!("step {turn} {}", "x".repeat(tokens * 4)));
            agent.record_assistant("ok");
        }
    }

    fn hybrid_stats(agent: &Agent) -> serde_json::Value {
        agent.feature_use()["contextVm"]["hybridCompaction"].clone()
    }

    #[test]
    fn a_replacement_that_cannot_fit_never_reaches_the_summarizer() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut agent = agent(calls.clone(), || {});
        fill(&mut agent, 4, 400);
        // The newest request alone is over the capacity limit: no cut can fit it.
        agent.prompt(&"y".repeat(39_000 * 4));
        let before = agent.messages.clone();

        let result = agent.compact_automatic(15_000);

        assert!(!result.compacted);
        assert_eq!(calls.load(Ordering::SeqCst), 0, "a fold was paid for");
        assert_eq!(agent.messages, before);
        assert!(
            result.summary.contains("skipped before the summarizer")
                && result.summary.contains("does not fit"),
            "{}",
            result.summary
        );
        let stats = hybrid_stats(&agent);
        assert_eq!(stats["attempts"], 1, "{stats}");
        assert_eq!(stats["skippedNoFit"], 1, "{stats}");
        assert_eq!(stats["applied"], 0, "{stats}");
        // Free skips never start the backoff.
        assert_eq!(agent.hybrid_fold_backoff, None);
    }

    #[test]
    fn a_history_without_a_cut_point_never_reaches_the_summarizer() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut agent = agent(calls.clone(), || {});
        agent.prompt("Build the scheduler. Never modify files under migrations/.");

        let result = agent.compact_automatic(15_000);

        assert!(!result.compacted);
        assert_eq!(calls.load(Ordering::SeqCst), 0, "a fold was paid for");
        assert!(
            result.summary.contains("no earlier turn to replace"),
            "{}",
            result.summary
        );
        let stats = hybrid_stats(&agent);
        assert_eq!(
            (stats["attempts"].as_u64(), stats["skippedNoFit"].as_u64()),
            (Some(1), Some(1))
        );
    }

    #[test]
    fn a_replacement_that_fits_is_folded_applied_and_counted() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut agent = agent(calls.clone(), || {});
        fill(&mut agent, 30, 400);

        let result = agent.compact(None);

        assert!(result.compacted, "{}", result.summary);
        assert!(calls.load(Ordering::SeqCst) >= 1, "the fold never ran");
        assert!(result.tokens_after < result.tokens_before);
        let stats = hybrid_stats(&agent);
        assert_eq!(stats["attempts"], 1, "{stats}");
        assert_eq!(stats["applied"], 1, "{stats}");
        assert_eq!(stats["skippedNoFit"], 0, "{stats}");
        assert_eq!(
            stats["contextTokensBefore"], result.tokens_before,
            "{stats}"
        );
        assert_eq!(stats["contextTokensAfter"], result.tokens_after, "{stats}");
        assert_eq!(agent.hybrid_fold_backoff, None);
    }

    /// A fold that is paid for and then not applied: the overhead jumps while
    /// the summarizer runs, so the budget recomputed afterwards is too small.
    fn unapplied_after_a_paid_fold() -> (Agent, Arc<AtomicUsize>) {
        OVERHEAD_BYTES.with(|bytes| bytes.set(400));
        let calls = Arc::new(AtomicUsize::new(0));
        let mut agent = agent(calls.clone(), || {
            OVERHEAD_BYTES.with(|bytes| bytes.set(60_000))
        });
        agent.set_provider_context_overhead_estimator(overhead_estimator);
        fill(&mut agent, 30, 400);
        let result = agent.compact_automatic(15_000);
        assert!(!result.compacted, "{}", result.summary);
        assert!(calls.load(Ordering::SeqCst) >= 1, "the fold never ran");
        // The condition that blocked the fit has passed; only the backoff holds.
        OVERHEAD_BYTES.with(|bytes| bytes.set(400));
        (agent, calls)
    }

    #[test]
    fn a_paid_fold_that_is_not_applied_defers_the_next_automatic_attempt() {
        let (mut agent, calls) = unapplied_after_a_paid_fold();
        let paid = calls.load(Ordering::SeqCst);
        let failed_at = agent.hybrid_fold_backoff.expect("backoff started");
        let keep = agent.compaction.keep_recent_tokens;
        let stats = hybrid_stats(&agent);
        assert_eq!(stats["notAppliedAfterFold"], 1, "{stats}");
        assert_eq!(stats["applied"], 0, "{stats}");

        // No growth, and one token short of `keep_recent_tokens`: deferred.
        for tokens in [failed_at, failed_at + keep - 1] {
            let result = agent.compact_automatic(tokens);
            assert!(!result.compacted);
            assert!(result.summary.contains("deferred"), "{}", result.summary);
        }
        assert_eq!(
            calls.load(Ordering::SeqCst),
            paid,
            "backoff paid for a fold"
        );
        let stats = hybrid_stats(&agent);
        assert_eq!(stats["skippedBackoff"], 2, "{stats}");
        assert_eq!(stats["attempts"], 3, "{stats}");

        // Grown by `keep_recent_tokens`: allowed, and it applies now.
        let result = agent.compact_automatic(failed_at + keep);
        assert!(result.compacted, "{}", result.summary);
        assert_eq!(
            agent.hybrid_fold_backoff, None,
            "applied compaction resets it"
        );
        let stats = hybrid_stats(&agent);
        assert_eq!(
            (stats["attempts"].as_u64(), stats["applied"].as_u64()),
            (Some(4), Some(1))
        );
    }

    #[test]
    fn the_backoff_defers_below_capacity_and_never_at_or_above_it() {
        // context_window = 40_000, capacity = 38_000, keep_recent = 1_000.
        let (mut agent, _) = unapplied_after_a_paid_fold();
        agent.arm_hybrid_backoff(36_500);
        let result = agent.compact_automatic(37_000);
        assert!(
            !result.compacted && result.summary.contains("deferred"),
            "{}",
            result.summary
        );

        // From the capacity limit up the attempt is a recovery: a stale backoff,
        // even one recorded just below it, never holds it back.
        for tokens in [38_000, 39_000, 40_000] {
            let (mut agent, _) = unapplied_after_a_paid_fold();
            agent.arm_hybrid_backoff(37_900);
            let result = agent.compact_automatic(tokens);
            assert!(result.compacted, "{tokens}: {}", result.summary);
            assert_eq!(hybrid_stats(&agent)["skippedBackoff"], 0);
        }
    }

    #[test]
    fn a_smaller_context_than_the_failed_one_clears_the_backoff() {
        let (mut agent, _) = unapplied_after_a_paid_fold();
        let failed_at = agent.hybrid_fold_backoff.unwrap();
        let keep = agent.compaction.keep_recent_tokens;
        let result = agent.compact_automatic(failed_at - keep - 1);
        assert!(result.compacted, "{}", result.summary);
        assert_eq!(hybrid_stats(&agent)["skippedBackoff"], 0);
    }

    #[test]
    fn a_small_drop_from_a_byte_estimate_does_not_clear_the_backoff() {
        // The failure was recorded from a byte estimate; the next provider count
        // reads about 3k lower. That is the scale, not a replaced history.
        let (mut agent, calls) = unapplied_after_a_paid_fold();
        let paid = calls.load(Ordering::SeqCst);
        let failed_at = agent.hybrid_fold_backoff.unwrap();
        let keep = agent.compaction.keep_recent_tokens;
        let result = agent.compact_automatic(failed_at - keep);
        assert!(result.summary.contains("deferred"), "{}", result.summary);
        assert_eq!(agent.hybrid_fold_backoff, Some(failed_at));
        assert_eq!(calls.load(Ordering::SeqCst), paid, "a fold was paid for");
    }

    #[test]
    fn a_failed_recovery_records_no_backoff_and_a_failed_automatic_attempt_does() {
        let mut agent = agent(Arc::new(AtomicUsize::new(0)), || {});
        let failed = CompactionResult {
            summary: "Compaction not saved: disk full".into(),
            messages: agent.messages.clone(),
            compacted: false,
            details: CompactionDetails::default(),
            first_kept_entry_id: String::new(),
            tokens_before: 10,
            tokens_after: 10,
            usage: None,
        };
        agent.hybrid_last_fold_requests = 1;
        agent.hybrid_compaction_stats.attempts += 1;
        agent.note_hybrid_outcome(0, &failed, 39_000, true);
        assert_eq!(agent.hybrid_fold_backoff, None);
        agent.hybrid_compaction_stats.attempts += 1;
        agent.note_hybrid_outcome(0, &failed, 12_000, false);
        assert_eq!(agent.hybrid_fold_backoff, Some(12_000));
    }

    #[test]
    fn a_failure_that_sent_no_request_does_not_arm_the_backoff() {
        let mut agent = agent(Arc::new(AtomicUsize::new(0)), || {});
        let failed = CompactionResult {
            summary: "Compaction not saved: disk full".into(),
            messages: agent.messages.clone(),
            compacted: false,
            details: CompactionDetails::default(),
            first_kept_entry_id: String::new(),
            tokens_before: 10,
            tokens_after: 10,
            usage: None,
        };
        agent.hybrid_last_fold_requests = 0;
        agent.hybrid_compaction_stats.attempts += 1;
        agent.note_hybrid_outcome(0, &failed, 12_000, false);
        assert_eq!(agent.hybrid_fold_backoff, None);
        assert_eq!(agent.hybrid_fold_backoff_settings, None);
        assert_eq!(hybrid_stats(&agent)["notAppliedOther"], 1);
    }

    #[test]
    fn a_non_shrinking_replacement_arms_the_backoff_only_after_a_paid_fold() {
        let mut agent = agent(Arc::new(AtomicUsize::new(0)), || {});
        let grew = CompactionResult {
            summary: "replaced".into(),
            messages: agent.messages.clone(),
            compacted: true,
            details: CompactionDetails::default(),
            first_kept_entry_id: String::new(),
            tokens_before: 10,
            tokens_after: 10,
            usage: None,
        };
        // Built from an existing checkpoint: no request, nothing to defer.
        agent.hybrid_last_fold_requests = 0;
        agent.hybrid_compaction_stats.attempts += 1;
        agent.note_hybrid_outcome(0, &grew, 12_000, false);
        assert_eq!(agent.hybrid_fold_backoff, None);
        assert_eq!(agent.hybrid_fold_backoff_settings, None);
        // A paid fold whose replacement did not shrink the context does.
        agent.hybrid_last_fold_requests = 1;
        agent.hybrid_compaction_stats.attempts += 1;
        agent.note_hybrid_outcome(0, &grew, 12_000, false);
        assert_eq!(agent.hybrid_fold_backoff, Some(12_000));
        // Both attempts are counted as applied, so the outcomes still sum.
        let stats = hybrid_stats(&agent);
        assert_eq!(stats["applied"], 2, "{stats}");
        assert_eq!(stats["attempts"], 2, "{stats}");
    }

    #[test]
    fn a_changed_summarizer_or_context_vm_mode_clears_the_backoff() {
        let (mut agent, _) = unapplied_after_a_paid_fold();
        let failed_at = agent.hybrid_fold_backoff.unwrap();

        // The same summarizer, even re-assigned as a clone, keeps the backoff.
        agent.summarizer = agent.summarizer.clone();
        let result = agent.compact_automatic(failed_at);
        assert!(result.summary.contains("deferred"), "{}", result.summary);

        agent.summarizer = Some(summarizer(Arc::new(AtomicUsize::new(0)), || {}));
        let result = agent.compact_automatic(failed_at);
        assert!(!result.summary.contains("deferred"), "{}", result.summary);
        assert_eq!(agent.hybrid_fold_backoff, None);

        let (mut agent, _) = unapplied_after_a_paid_fold();
        // Setting the mode it already has keeps the backoff.
        agent.set_context_vm_mode(crate::runtime::ContextVmMode::Hybrid);
        assert!(agent.hybrid_fold_backoff.is_some());
        agent.set_context_vm_mode(crate::runtime::ContextVmMode::Shadow);
        assert_eq!(agent.hybrid_fold_backoff, None);
        assert_eq!(agent.hybrid_fold_backoff_settings, None);
    }

    #[test]
    fn a_backoff_deferral_emits_the_compact_events_like_a_dry_run_skip() {
        use crate::runtime::{
            bus::RuntimeSubscriber, AgentId, RunId, RuntimeBus, RuntimeDecision, RuntimeEvent,
            RuntimeEventEnvelope, RuntimeHandle,
        };
        use std::sync::Mutex;

        #[derive(Default)]
        struct EventCollector {
            events: Mutex<Vec<RuntimeEvent>>,
        }
        impl RuntimeSubscriber for EventCollector {
            fn on_event(&self, event: &RuntimeEventEnvelope) -> RuntimeDecision {
                self.events.lock().unwrap().push(event.payload.clone());
                RuntimeDecision::Continue
            }
        }

        let mut agent = agent(Arc::new(AtomicUsize::new(0)), || {});
        let bus = RuntimeBus::new();
        let collector = Arc::new(EventCollector::default());
        bus.subscribe(collector.clone());
        agent.set_runtime(RuntimeHandle::new(RunId::new(), AgentId::new(), bus));
        fill(&mut agent, 30, 400);
        agent.arm_hybrid_backoff(12_000);
        let result = agent.compact_automatic(12_500);
        assert!(result.summary.contains("deferred"), "{}", result.summary);
        let events = collector.events.lock().unwrap().clone();
        let pre: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                RuntimeEvent::PreCompact { estimated_tokens } => Some(*estimated_tokens),
                _ => None,
            })
            .collect();
        let post: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                RuntimeEvent::PostCompact {
                    before_tokens,
                    after_tokens,
                } => Some((*before_tokens, *after_tokens)),
                _ => None,
            })
            .collect();
        assert_eq!(pre.len(), 1, "{events:?}");
        assert_eq!(post.len(), 1, "{events:?}");
        assert!(pre[0] > 0);
        assert_eq!(post[0], (pre[0], pre[0]));
    }

    #[test]
    fn changing_the_window_or_the_compaction_settings_clears_the_backoff() {
        for change in [
            (|agent: &mut Agent| agent.context_window = 50_000) as fn(&mut Agent),
            |agent: &mut Agent| agent.compaction.keep_recent_tokens = 500,
            |agent: &mut Agent| agent.compaction.reserve_tokens = 1_000,
            |agent: &mut Agent| {
                agent.compaction.threshold =
                    Some(crate::compaction::CompactionThreshold::Tokens(9_000))
            },
        ] {
            let (mut agent, _) = unapplied_after_a_paid_fold();
            let failed_at = agent.hybrid_fold_backoff.unwrap();
            // Unchanged settings: deferred.
            let result = agent.compact_automatic(failed_at);
            assert!(result.summary.contains("deferred"), "{}", result.summary);
            change(&mut agent);
            let result = agent.compact_automatic(failed_at);
            assert!(!result.summary.contains("deferred"), "{}", result.summary);
            assert_eq!(agent.hybrid_fold_backoff, None);
        }
    }

    #[test]
    fn reserve_band_requests_do_not_wait_for_a_backoff_after_a_failed_recovery() {
        // Default compaction settings (keep 20,000, reserve 16,384), a 60k window and a
        // first prompt of 42k tokens: with the 2k overhead turn 0 is already above the
        // capacity limit (43,616) and has no earlier turn to replace, so its recovery
        // pays a fold that cannot apply. Before the backoff existed, turn 1 compacted;
        // a backoff held through the reserve band for many turns.
        let calls = Arc::new(AtomicUsize::new(0));
        let mut agent = agent(calls.clone(), || {});
        agent.context_window = 60_000;
        agent.compaction = CompactionSettings::default();
        agent.set_provider_context_overhead_tokens(Some(2_000));
        let capacity = 60_000 - agent.compaction.reserve_tokens;
        let above_capacity = Arc::new(AtomicUsize::new(0));
        let mut applied_at = None;
        for turn in 0..30 {
            let size = if turn == 0 { 42_000 } else { 500 };
            agent.prompt(&format!("turn {turn} {}", "x".repeat(size * 4)));
            let counter = above_capacity.clone();
            agent
                .run_loop(move |agent: &Agent| {
                    let tokens = crate::compaction::estimate_context_tokens(&agent.messages)
                        + agent.estimated_request_overhead_tokens();
                    if tokens > capacity {
                        counter.fetch_add(1, Ordering::SeqCst);
                    }
                    Ok::<_, String>(reply_with(tokens))
                })
                .unwrap();
            if applied_at.is_none() && hybrid_stats(&agent)["applied"].as_u64().unwrap() > 0 {
                applied_at = Some(turn);
            }
        }
        assert_eq!(applied_at, Some(1), "compaction must apply at turn 1");
        assert!(
            above_capacity.load(Ordering::SeqCst) <= 1,
            "{} requests above the capacity limit",
            above_capacity.load(Ordering::SeqCst)
        );
        assert_eq!(agent.hybrid_fold_backoff, None);
    }

    #[test]
    fn a_failed_fold_reports_the_full_request_estimate_in_both_fields() {
        // The checkpoint cannot be written, so the fold fails.
        let dir = tempfile::tempdir().unwrap();
        let mut agent = agent(Arc::new(AtomicUsize::new(0)), || {});
        agent
            .load_from_session(JsonlSession::create(dir.path(), "fixture", None).unwrap())
            .unwrap();
        agent.set_context_vm_mode(crate::runtime::ContextVmMode::Hybrid);
        fill(&mut agent, 6, 200);
        let full = agent.estimated_context_tokens();
        let messages = crate::compaction::estimate_context_tokens(&agent.messages);
        assert!(full > messages);
        let path = agent.session.as_ref().unwrap().path.clone();
        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&path, permissions).unwrap();

        let result = agent.compact(None);

        let mut permissions = std::fs::metadata(&path).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        std::fs::set_permissions(&path, permissions).unwrap();
        assert!(!result.compacted, "{}", result.summary);
        assert_eq!(hybrid_stats(&agent)["applied"], 0);
        assert_eq!((result.tokens_before, result.tokens_after), (full, full));
    }

    #[test]
    fn a_manual_compact_recovers_when_the_fold_shrinks_the_ledger_below_the_dry_run() {
        let (mut agent, calls, _, _) = a_long_parent_narrative_the_next_fold_shortens();

        let result = agent.compact(None);

        assert!(result.compacted, "{}", result.summary);
        assert_eq!(calls.load(Ordering::SeqCst), 1, "one paid fold");
        assert_eq!(hybrid_stats(&agent)["skippedNoFit"], 0);
    }

    #[test]
    fn an_automatic_attempt_at_capacity_recovers_when_the_fold_shrinks_the_ledger() {
        let (mut agent, calls, _, _) = a_long_parent_narrative_the_next_fold_shortens();

        let result = agent.compact_automatic(38_000);

        assert!(result.compacted, "{}", result.summary);
        assert_eq!(calls.load(Ordering::SeqCst), 1, "one paid fold");
        assert_eq!(hybrid_stats(&agent)["skippedNoFit"], 0);
    }

    #[test]
    fn below_capacity_a_long_parent_narrative_alone_does_not_false_skip_the_dry_run() {
        let (mut agent, calls, _, _) = a_long_parent_narrative_the_next_fold_shortens();

        // Below capacity, so the free dry run runs. The fold may replace the parent
        // narrative, so the dry run renders without it and fits.
        let result = agent.compact_automatic(20_000);

        assert!(result.compacted, "{}", result.summary);
        assert_eq!(calls.load(Ordering::SeqCst), 1, "one paid fold");
        assert_eq!(hybrid_stats(&agent)["skippedNoFit"], 0);
    }

    #[test]
    fn a_recovery_does_not_run_the_dry_run() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut agent = agent(calls.clone(), || {});
        fill(&mut agent, 4, 400);
        // The newest request alone is over the capacity limit: HEAD paid a fold here too.
        agent.prompt(&"y".repeat(39_000 * 4));

        let result = agent.compact(None);

        assert!(!result.compacted);
        assert!(
            calls.load(Ordering::SeqCst) >= 1,
            "recovery skipped the fold"
        );
        let stats = hybrid_stats(&agent);
        assert_eq!(stats["skippedNoFit"], 0, "{stats}");
        assert_eq!(stats["notAppliedAfterFold"], 1, "{stats}");
    }

    fn narrative_proposal(agent: &Agent, narrative: &str) -> String {
        let source_ref = agent
            .context_vm_events_for_vm()
            .iter()
            .rev()
            .find(|event| event.kind == crate::runtime::context_vm::ContextEventKind::Assistant)
            .unwrap()
            .source_ref
            .clone();
        serde_json::json!({"narrative": {
            "value": narrative, "source_refs": [source_ref], "provenance_kind": "agent_inference"}})
        .to_string()
    }

    fn tokens_needed(agent: &Agent, ledger: &str) -> u64 {
        let result = crate::compaction::compact_messages_with_summary(
            &agent.messages,
            agent.compaction.keep_recent_tokens,
            ledger,
            0,
        );
        let at = result.summary.find(" need ").unwrap() + 6;
        result.summary[at..]
            .split(' ')
            .next()
            .unwrap()
            .parse()
            .unwrap()
    }

    /// The first fold stores a 1,000-byte narrative; the second proposes "short". The
    /// deterministic dry run keeps the long one, so its ledger is bigger than the folded
    /// one, and the budget sits between the two needs. Returns the agent, the summarizer
    /// request counter and the two needs.
    fn a_long_parent_narrative_the_next_fold_shortens() -> (Agent, Arc<AtomicUsize>, u64, u64) {
        let calls = Arc::new(AtomicUsize::new(0));
        let reply = Arc::new(std::sync::Mutex::new(String::from("{}")));
        let counter = calls.clone();
        let fold_reply = reply.clone();
        let mut agent = Agent::new("You are a coding agent.");
        agent.set_runtime(crate::RuntimeHandle::new(
            crate::RunId::new(),
            crate::AgentId::new(),
            crate::RuntimeBus::new(),
        ));
        agent.set_context_vm_mode(crate::runtime::ContextVmMode::Hybrid);
        agent.context_window = 40_000;
        agent.compaction = CompactionSettings {
            reserve_tokens: 2_000,
            keep_recent_tokens: 1_000,
            threshold: None,
            ..CompactionSettings::default()
        };
        agent.auto_compaction = false;
        agent.summarizer = Some(Summarizer::new(move |request| {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok(SummarizeResponse {
                text: if request.label == "context state fold" {
                    fold_reply.lock().unwrap().clone()
                } else {
                    "## Goal\nContinue.".into()
                },
                usage: Some(Default::default()),
                stop_reason: Some(davinci_ai::StopReason::Stop),
                error_message: None,
                has_tool_call: false,
            })
        }));
        agent.prompt("Build the scheduler.");
        agent.record_assistant("ok, starting");
        for turn in 0..6 {
            agent.prompt(&format!("step {turn} {}", "x".repeat(1_600)));
            agent.record_assistant(&format!("done {turn}"));
        }
        *reply.lock().unwrap() = narrative_proposal(&agent, &"N".repeat(1_000));
        agent
            .fold_context(runtime::context_vm::FoldReason::Manual, None)
            .unwrap();
        for turn in 6..12 {
            agent.prompt(&format!("step {turn} {}", "x".repeat(1_600)));
            agent.record_assistant(&format!("done {turn}"));
        }
        agent.prompt(&format!("next {}", "z".repeat(4_000)));
        let state = agent
            .runtime
            .as_ref()
            .unwrap()
            .context_vm
            .load_state_from_root()
            .unwrap();
        assert!(
            state.narrative.is_some(),
            "the first fold kept no narrative"
        );
        let newer: Vec<_> = agent
            .context_vm_events_for_vm()
            .into_iter()
            .filter(|event| event.seq > state.through_seq)
            .collect();
        let long = runtime::context_vm::ContextStateReducer::deterministic_delta(&state, &newer)
            .checkpoint_patch;
        let mut short = long.clone();
        short.narrative.as_mut().unwrap().value = "short".into();
        let need_long = tokens_needed(&agent, &runtime::context_vm::render_ledger(&long));
        let need_short = tokens_needed(&agent, &runtime::context_vm::render_ledger(&short));
        assert!(need_short < need_long);
        // Budget between the two needs: the long ledger cannot fit, the folded one can.
        agent.set_provider_context_overhead_tokens(Some(0));
        let base = agent.estimated_request_overhead_tokens();
        let budget = (need_long + need_short) / 2;
        agent.set_provider_context_overhead_tokens(Some(38_000 - budget - base));
        assert_eq!(agent.hybrid_replacement_budget(true), budget);
        *reply.lock().unwrap() = narrative_proposal(&agent, "short");
        let paid_so_far = calls.load(Ordering::SeqCst);
        calls.store(0, Ordering::SeqCst);
        let _ = paid_so_far;
        (agent, calls, need_long, need_short)
    }

    /// `fill(4, 400)`, then a 12k-token prompt, "ok" and "next": the threshold
    /// budget (10k) fits a replacement that keeps only the last turns, and the
    /// capacity budget (38k) would fit one that keeps the 12k prompt.
    fn the_probe_history(calls: Arc<AtomicUsize>) -> Agent {
        let mut agent = agent(calls, || {});
        agent.auto_compaction = false;
        fill(&mut agent, 4, 400);
        agent.prompt(&"y".repeat(12_000 * 4));
        agent.record_assistant("ok");
        agent.prompt("next");
        agent
    }

    /// B035 review finding 4: a usage whose input, output or cached-read count
    /// the provider did not report holds a placeholder zero. It must not be
    /// installed as a measured context size (Hybrid mode compacts from it), and
    /// it must clear an earlier real observation rather than leave it stale.
    #[test]
    fn audit_wp03_b035_partial_usage_is_not_a_measured_context() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut agent = agent(calls, || {});
        agent.prompt("hello");
        agent.record_assistant("ok");
        let reply = agent.messages.last().cloned().unwrap();

        let reported = davinci_protocol::Usage {
            input: 90_000,
            output: 10,
            ..Default::default()
        };
        agent.note_provider_context(Some(&reported), &reply);
        assert_eq!(
            agent.last_provider_context.as_ref().map(|o| o.tokens),
            Some(90_010)
        );

        for partial in [
            davinci_protocol::Usage {
                output: 10,
                input_unreported: true,
                ..Default::default()
            },
            davinci_protocol::Usage {
                input: 90_000,
                output_unreported: true,
                ..Default::default()
            },
            davinci_protocol::Usage {
                input: 90_000,
                output: 10,
                cache_read_unreported: true,
                ..Default::default()
            },
        ] {
            agent.note_provider_context(Some(&reported), &reply);
            assert!(agent.last_provider_context.is_some());
            agent.note_provider_context(Some(&partial), &reply);
            assert!(
                agent.last_provider_context.is_none(),
                "{partial:?} was installed as a measured context"
            );
        }
        agent.note_provider_context(Some(&reported), &reply);
        agent.note_provider_context(None, &reply);
        assert!(agent.last_provider_context.is_none());

        // Reported zeros are measurements: zero input with ten output tokens
        // is a ten-token context, not an unknown one.
        let zeros = davinci_protocol::Usage {
            output: 10,
            ..Default::default()
        };
        agent.note_provider_context(Some(&zeros), &reply);
        assert_eq!(
            agent.last_provider_context.as_ref().map(|o| o.tokens),
            Some(10)
        );
    }

    #[test]
    fn a_recovery_lands_under_the_threshold_when_something_fits_there() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut agent = the_probe_history(calls.clone());
        let threshold_budget = agent.hybrid_replacement_budget(false);

        let result = agent.compact(None);

        assert!(result.compacted, "{}", result.summary);
        assert!(
            result.tokens_after <= threshold_budget,
            "{} tokens, threshold budget {threshold_budget}",
            result.tokens_after
        );
        let paid = calls.load(Ordering::SeqCst);
        assert!(paid >= 1);
        // The next turn does not trigger a second compaction.
        let mut settings = agent.compaction;
        settings.enabled = true;
        assert!(!crate::should_compact(
            agent.calibrated_context_tokens(),
            agent.context_window,
            &settings
        ));
        assert_eq!(hybrid_stats(&agent)["applied"], 1);
    }

    #[test]
    fn a_recovery_falls_back_to_capacity_only_when_nothing_fits_the_threshold() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut agent = agent(calls, || {});
        fill(&mut agent, 4, 400);
        // The newest turn alone is over the 10k threshold, under the 38k capacity.
        agent.prompt(&"y".repeat(12_000 * 4));

        let result = agent.compact(None);

        assert!(result.compacted, "{}", result.summary);
        assert!(result.tokens_after > agent.hybrid_replacement_budget(false));
    }

    #[test]
    fn an_overflow_retry_does_not_compact_a_second_time_in_the_same_turn() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut agent = the_probe_history(calls.clone());
        agent.auto_compaction = true;
        // The automatic attempt defers (backoff armed at the current size), so
        // the overflow retry is the compaction.
        agent.arm_hybrid_backoff(agent.calibrated_context_tokens());
        let requests = Arc::new(AtomicUsize::new(0));
        let counter = requests.clone();

        agent
            .run_loop(move |_: &Agent| {
                if counter.fetch_add(1, Ordering::SeqCst) == 0 {
                    Err("context_length_exceeded: input exceeds the context window".to_string())
                } else {
                    Ok(reply_with(1_000))
                }
            })
            .unwrap();

        let stats = hybrid_stats(&agent);
        assert_eq!(requests.load(Ordering::SeqCst), 2);
        assert_eq!(stats["skippedBackoff"], 1, "{stats}");
        assert_eq!(stats["applied"], 1, "{stats}");
        assert_eq!(agent.run_stats().compactions, 1, "{stats}");
        assert_eq!(stats["attempts"], 2, "{stats}");
    }

    #[test]
    fn a_host_overhead_that_counts_the_tool_catalog_is_not_charged_the_schema_twice() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut agent = agent(calls, || {});
        assert!(agent.pending_retrieval_schema_tokens() > 0);
        agent.set_provider_context_overhead_tokens(Some(500));
        assert_eq!(agent.pending_retrieval_schema_tokens(), 0);
        agent.set_provider_context_overhead_tokens(None);
        assert!(agent.pending_retrieval_schema_tokens() > 0);
        agent.set_provider_context_overhead_estimator(overhead_estimator);
        assert_eq!(agent.pending_retrieval_schema_tokens(), 0);
    }

    #[test]
    fn replacing_the_history_resets_the_backoff() {
        let (mut agent, _) = unapplied_after_a_paid_fold();
        assert!(agent.hybrid_fold_backoff.is_some());
        agent.restore_branch_state();
        assert_eq!(agent.hybrid_fold_backoff, None);
    }

    #[test]
    fn manual_compaction_ignores_the_backoff() {
        let (mut agent, _) = unapplied_after_a_paid_fold();
        assert!(agent.hybrid_fold_backoff.is_some());
        let result = agent.compact(None);
        assert!(result.compacted, "{}", result.summary);
        assert_eq!(hybrid_stats(&agent)["skippedBackoff"], 0);
    }

    #[test]
    fn a_context_overflow_refusal_ignores_the_backoff() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut agent = agent(calls.clone(), || {});
        agent.auto_compaction = false;
        fill(&mut agent, 30, 400);
        agent.prompt("Continue.");
        agent.auto_compaction = true;
        // Over the 10_000 threshold, so the automatic trigger fires; the backoff
        // armed at the current size makes that attempt defer.
        assert!(agent.estimated_context_tokens() > 10_000);
        agent.arm_hybrid_backoff(agent.calibrated_context_tokens());
        let requests = Arc::new(AtomicUsize::new(0));
        let counter = requests.clone();

        agent
            .run_loop(move |_: &Agent| {
                if counter.fetch_add(1, Ordering::SeqCst) == 0 {
                    Err("context_length_exceeded: input exceeds the context window".to_string())
                } else {
                    Ok(reply_with(1_000))
                }
            })
            .unwrap();

        assert_eq!(requests.load(Ordering::SeqCst), 2);
        let stats = hybrid_stats(&agent);
        assert_eq!(
            stats["skippedBackoff"], 1,
            "the automatic path must have deferred first: {stats}"
        );
        assert_eq!(agent.run_stats().compactions, 1, "{stats}");
        assert_eq!(stats["applied"], 1, "{stats}");
        assert!(
            calls.load(Ordering::SeqCst) >= 1,
            "recovery skipped the fold"
        );
    }

    // ---- Review findings -------------------------------------------------------------------

    fn reply_with(input: u64) -> davinci_ai::AssistantMessage {
        davinci_ai::AssistantMessage {
            extra: Default::default(),
            id: "fixture".into(),
            role: "assistant".into(),
            model: "fixture".into(),
            usage: Some(davinci_protocol::Usage {
                input,
                output: 4,
                ..Default::default()
            }),
            error_message: None,
            content: vec![davinci_ai::ContentBlock::Text {
                text: "done".into(),
            }],
            stop_reason: Some(davinci_ai::StopReason::Stop),
        }
    }

    #[test]
    fn the_fit_budget_is_the_limit_minus_the_estimated_overhead() {
        // The budget must not depend on what the provider counted: a provider that
        // counts the messages below the byte estimate would otherwise grow it, let
        // the first cut land just under the threshold, and pay a fold every turn.
        let calls = Arc::new(AtomicUsize::new(0));
        let mut agent = agent(calls, || {});
        agent.set_provider_context_overhead_tokens(Some(3_000));
        fill(&mut agent, 6, 200);
        let overhead = agent.estimated_request_overhead_tokens();
        let schema = agent.pending_retrieval_schema_tokens();
        assert_eq!(schema, 0, "a host overhead already counts the catalog");
        assert_eq!(agent.hybrid_replacement_budget(false), 10_000 - overhead);
        assert_eq!(agent.hybrid_replacement_budget(true), 38_000 - overhead);
        // Same budget after the provider reports a count far below the estimate.
        agent
            .run_loop(|_: &Agent| Ok::<_, String>(reply_with(500)))
            .unwrap();
        assert_eq!(
            agent.hybrid_replacement_budget(false),
            10_000 - agent.estimated_request_overhead_tokens()
        );
    }

    /// 40 turns of about 400 tokens against a provider that counts the messages at
    /// three quarters of the byte estimate. Returns (summarizer requests, applied).
    fn forty_turns_with_a_low_counting_provider(keep_recent: u64) -> (usize, u64) {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut agent = agent(calls.clone(), || {});
        agent.compaction.keep_recent_tokens = keep_recent;
        agent.set_provider_context_overhead_tokens(Some(3_000));
        agent.prompt("Build the scheduler. Never modify files under migrations/.");
        for turn in 0..40 {
            agent.prompt(&format!("step {turn} {}", "x".repeat(400 * 4)));
            agent
                .run_loop(|agent: &Agent| {
                    let messages = crate::compaction::estimate_context_tokens(&agent.messages);
                    Ok::<_, String>(reply_with(
                        messages * 3 / 4 + agent.estimated_request_overhead_tokens(),
                    ))
                })
                .unwrap();
        }
        let stats = hybrid_stats(&agent);
        (
            calls.load(Ordering::SeqCst),
            stats["applied"].as_u64().unwrap(),
        )
    }

    #[test]
    fn a_provider_that_counts_low_does_not_make_every_turn_a_paid_fold() {
        // Upper bounds are the summarizer requests the budget formula
        // `limit - estimated overhead` pays on this probe.
        for (keep_recent, bound) in [(6_000, 4), (7_000, 4), (7_500, 4), (8_000, 4), (9_000, 3)] {
            let (paid, applied) = forty_turns_with_a_low_counting_provider(keep_recent);
            assert!(
                paid <= bound && applied as usize <= paid,
                "keep_recent {keep_recent}: {paid} paid requests (bound {bound}), {applied} applied"
            );
        }
    }

    #[test]
    fn manual_compaction_budgets_against_the_capacity_limit() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut agent = agent(calls.clone(), || {});
        fill(&mut agent, 4, 400);
        // The newest turn alone is over the 10k threshold but far under the 38k capacity.
        agent.prompt(&"y".repeat(12_000 * 4));

        let result = agent.compact(None);

        assert!(result.compacted, "{}", result.summary);
        assert!(result.tokens_after < result.tokens_before);
    }

    #[test]
    fn a_full_window_recovers_even_when_the_threshold_budget_cannot_fit() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut agent = agent(calls.clone(), || {});
        agent.auto_compaction = false;
        for turn in 0..30 {
            agent.prompt(&format!("step {turn} {}", "x".repeat(4_000)));
            agent.record_assistant("ok");
        }
        agent.prompt("Continue.");
        agent
            .run_loop(|agent: &Agent| Ok::<_, String>(reply_with(agent.estimated_context_tokens())))
            .unwrap();
        agent.auto_compaction = true;
        agent.prompt(&"y".repeat(12_000 * 4));
        // A paid fold failed at this size, so the backoff is holding.
        agent.arm_hybrid_backoff(agent.calibrated_context_tokens());
        assert!(agent.calibrated_context_tokens() >= agent.context_window);

        let outcome = agent.run_loop(|_: &Agent| Ok::<_, String>(reply_with(1_000)));

        outcome.expect("the request must not be blocked: the window can be recovered");
        assert!(agent.run_stats().compactions >= 1);
    }

    #[test]
    fn a_ledger_of_another_branch_does_not_block_the_dry_run() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut agent = agent(calls.clone(), || {});
        agent.context_window = 1_600;
        agent.compaction = CompactionSettings {
            reserve_tokens: 500,
            keep_recent_tokens: 100,
            ..CompactionSettings::default()
        };
        agent.set_provider_context_overhead_tokens(Some(0));
        // The VM follows a long conversation...
        for turn in 0..60 {
            agent.prompt(&format!("old step {turn} {}", "o".repeat(4_000)));
            agent.record_assistant("ok");
        }
        let events = agent.context_vm_events_for_vm();
        let vm = agent.runtime.as_ref().unwrap().context_vm.clone();
        vm.append_delta(&events).unwrap();
        let stale = crate::compaction::estimate_context_tokens(&[ChatMessage::text(
            "user",
            runtime::context_vm::render_ledger(&vm.load_state_from_root().unwrap()),
        )]);
        assert!(
            stale > agent.hybrid_replacement_budget(false),
            "the old ledger must not fit: {stale}"
        );
        // ...then the user navigates to a short, different one.
        agent.messages.clear();
        for turn in 0..3 {
            agent.prompt(&format!("new step {turn} {}", "n".repeat(200)));
            agent.record_assistant("ok");
        }
        agent.prompt("Continue.");

        let result = agent.compact(None);

        assert!(result.compacted, "{}", result.summary);
    }

    #[test]
    fn a_prefix_without_an_assistant_reply_is_not_replaced() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut agent = configure(
            Agent::new_builtin(crate::prompt::PromptProfile::Stable),
            calls.clone(),
            || {},
        );
        agent.turn_context_placement_override =
            Some(crate::turn_context::TurnContextPlacement::Appended);
        agent.prompt("Build the scheduler. Never modify files under migrations/.");
        agent.append_turn_context(None);
        assert!(
            agent.messages.len() >= 2,
            "turn 1 must hold a user prompt and a context message"
        );
        assert!(agent.messages.iter().all(|m| m.role != "assistant"));
        let before = agent.messages.clone();

        let result = agent.compact_automatic(15_000);

        assert!(!result.compacted, "{}", result.summary);
        assert_eq!(calls.load(Ordering::SeqCst), 0, "a fold was paid for");
        assert_eq!(agent.messages, before);
        assert!(
            result.summary.contains("no assistant reply") && !result.summary.contains("cannot fit"),
            "{}",
            result.summary
        );
        let stats = hybrid_stats(&agent);
        assert_eq!(stats["skippedNoAssistantReply"], 1, "{stats}");
        assert_eq!(stats["skippedNoFit"], 0, "{stats}");
        assert_eq!(stats["attempts"], 1, "{stats}");
        // A free skip never starts the backoff.
        assert_eq!(agent.hybrid_fold_backoff, None);
    }

    /// `[U1 huge, A1, TR1 huge]`: the cut point lands inside the tool result, the
    /// only cut left is after A1, and the prefix to replace is the user's prompt.
    /// With the window nearly full, replacing it with the ledger is the only
    /// way out.
    fn a_huge_prompt_and_a_huge_tool_result(calls: Arc<AtomicUsize>) -> Agent {
        let mut agent = agent(calls, || {});
        agent.auto_compaction = false;
        agent.prompt(&"u".repeat(20_000 * 4));
        agent.record_assistant("calling");
        agent.messages.push(ChatMessage::tool_result(
            "c1",
            "read",
            "t".repeat(16_000 * 4),
            false,
        ));
        assert!(agent.estimated_context_tokens() > 36_000);
        assert!(agent.estimated_context_tokens() < 40_000);
        agent
    }

    #[test]
    fn a_full_window_of_one_prompt_and_one_tool_result_recovers_when_automatic() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut agent = a_huge_prompt_and_a_huge_tool_result(calls.clone());

        // At the capacity limit (38_000) the automatic attempt is a recovery.
        let result = agent.compact_automatic(39_000);

        assert!(result.compacted, "{}", result.summary);
        assert!(result.tokens_after < result.tokens_before);
        assert!(calls.load(Ordering::SeqCst) >= 1);
        assert_eq!(hybrid_stats(&agent)["skippedNoAssistantReply"], 0);
    }

    #[test]
    fn a_full_window_of_one_prompt_and_one_tool_result_recovers_when_manual() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut agent = a_huge_prompt_and_a_huge_tool_result(calls);

        let result = agent.compact(None);

        assert!(result.compacted, "{}", result.summary);
        assert_eq!(hybrid_stats(&agent)["skippedNoAssistantReply"], 0);
    }

    #[test]
    fn a_full_window_of_one_prompt_and_one_tool_result_recovers_on_overflow() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut agent = a_huge_prompt_and_a_huge_tool_result(calls.clone());
        agent.auto_compaction = true;
        agent.prompt("Continue.");
        let requests = Arc::new(AtomicUsize::new(0));
        let counter = requests.clone();

        agent
            .run_loop(move |_: &Agent| {
                if counter.fetch_add(1, Ordering::SeqCst) == 0 {
                    Err("context_length_exceeded: input exceeds the context window".to_string())
                } else {
                    Ok(reply_with(1_000))
                }
            })
            .expect("the overflow retry must recover the session");

        assert_eq!(requests.load(Ordering::SeqCst), 2);
        assert!(agent.run_stats().compactions >= 1);
        assert!(hybrid_stats(&agent)["applied"].as_u64().unwrap() >= 1);
    }

    #[test]
    fn an_automatic_attempt_below_capacity_leaves_the_lone_prompt_alone() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut agent = a_huge_prompt_and_a_huge_tool_result(calls.clone());

        let result = agent.compact_automatic(15_000);

        assert!(!result.compacted, "{}", result.summary);
        assert_eq!(calls.load(Ordering::SeqCst), 0, "a fold was paid for");
        assert_eq!(hybrid_stats(&agent)["skippedNoAssistantReply"], 1);
    }

    #[test]
    fn the_automatic_path_picks_a_later_cut_when_the_first_has_no_assistant_reply() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut agent = agent(calls, || {});
        // After an earlier compaction: [ledger, U5, A5, U6]. keep_recent is 1_000 tokens, so
        // the walk-back stops at U5 and the first cut tried is the one right after the ledger.
        agent
            .messages
            .push(crate::compaction::compaction_context_message(
                "earlier ledger",
            ));
        agent.prompt(&format!("step 5 {}", "x".repeat(1_100 * 4)));
        agent.record_assistant("ok");
        agent.prompt("next");
        let first = crate::compaction::find_cut_point(&agent.messages, 1_000).first_kept_index;
        assert_eq!(
            first, 1,
            "the test needs the first cut to be after the ledger"
        );

        let result = agent.compact_automatic(15_000);

        assert!(result.compacted, "{}", result.summary);
        assert_eq!(hybrid_stats(&agent)["skippedNoAssistantReply"], 0);
        // The ledger, then only the newest turn: the cut moved past U5 and A5.
        assert_eq!(agent.messages.len(), 2, "{:?}", agent.messages);
        assert_eq!(content_text(&agent.messages[1].content), "next");
    }

    #[test]
    fn the_backoff_records_the_size_the_trigger_measured() {
        // `unapplied_after_a_paid_fold` triggers at 15_000 on a history whose
        // plain estimate is lower: the backoff must hold the trigger's number.
        let (agent, _) = unapplied_after_a_paid_fold();
        assert_eq!(agent.hybrid_fold_backoff, Some(15_000));
    }

    #[test]
    fn the_retrieval_tool_schema_is_in_the_budget_before_the_first_fold() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut agent = agent(calls, || {});
        fill(&mut agent, 6, 200);
        assert!(!agent.context_vm_offers_retrieval());
        let before = agent.hybrid_replacement_budget(false);
        agent
            .fold_context(runtime::context_vm::FoldReason::Manual, None)
            .unwrap();
        assert!(agent.context_vm_offers_retrieval());
        let after = agent.hybrid_replacement_budget(false);
        assert!(
            before.abs_diff(after) <= 2,
            "the first fold changed the budget from {before} to {after}"
        );
    }

    #[test]
    fn only_hybrid_mode_reports_hybrid_compaction() {
        for mode in [
            crate::runtime::ContextVmMode::Active,
            crate::runtime::ContextVmMode::Shadow,
        ] {
            let mut agent = Agent::new("x");
            agent.set_context_vm_mode(mode);
            let vm = agent.feature_use()["contextVm"].clone();
            assert!(vm.get("hybridCompaction").is_none(), "{mode:?}: {vm}");
        }
    }

    #[test]
    fn counters_separate_paid_folds_from_other_failures() {
        let (agent, _) = unapplied_after_a_paid_fold();
        let stats = hybrid_stats(&agent);
        assert_eq!(stats["notAppliedAfterFold"], 1, "{stats}");
        assert_eq!(stats["notAppliedOther"], 0, "{stats}");

        // No summarizer request is sent: the failure is not a paid fold.
        let mut agent = agent_without_summarizer();
        fill(&mut agent, 30, 400);
        agent.hybrid_last_fold_requests = 0;
        let failed = CompactionResult {
            summary: "Compaction not saved: disk full".into(),
            messages: agent.messages.clone(),
            compacted: false,
            details: CompactionDetails::default(),
            first_kept_entry_id: String::new(),
            tokens_before: 10,
            tokens_after: 10,
            usage: None,
        };
        agent.hybrid_compaction_stats.attempts += 1;
        agent.note_hybrid_outcome(0, &failed, 10, false);
        let stats = hybrid_stats(&agent);
        assert_eq!(stats["notAppliedOther"], 1, "{stats}");
        assert_eq!(stats["notAppliedAfterFold"], 0, "{stats}");
        let sum: u64 = [
            "applied",
            "skippedNoFit",
            "skippedNoAssistantReply",
            "skippedBackoff",
            "notAppliedAfterFold",
            "notAppliedOther",
        ]
        .iter()
        .map(|key| stats[key].as_u64().unwrap())
        .sum();
        assert_eq!(stats["attempts"].as_u64().unwrap(), sum, "{stats}");
    }

    fn agent_without_summarizer() -> Agent {
        let mut agent = agent(Arc::new(AtomicUsize::new(0)), || {});
        agent.summarizer = None;
        agent
    }

    #[test]
    fn skip_results_report_message_estimates_in_both_fields() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut agent = agent(calls, || {});
        fill(&mut agent, 30, 400);
        agent.arm_hybrid_backoff(12_000);
        let estimate = crate::compaction::estimate_context_tokens(&agent.messages);
        let deferred = agent.compact_automatic(12_500);
        assert!(
            deferred.summary.contains("deferred"),
            "{}",
            deferred.summary
        );
        assert_eq!(
            (deferred.tokens_before, deferred.tokens_after),
            (estimate, estimate)
        );

        agent.hybrid_fold_backoff = None;
        let mut small = agent_without_summarizer();
        small.prompt("one message");
        let estimate = crate::compaction::estimate_context_tokens(&small.messages);
        let skipped = small.compact(None);
        assert_eq!(
            (skipped.tokens_before, skipped.tokens_after),
            (estimate, estimate)
        );
    }

    #[test]
    fn counters_belong_to_the_agent_and_survive_a_session_load() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut agent = agent(calls, || {});
        fill(&mut agent, 30, 400);
        assert!(agent.compact(None).compacted);
        let before = hybrid_stats(&agent);
        assert_eq!(before["applied"], 1);
        let dir = tempfile::tempdir().unwrap();
        agent
            .load_from_session(JsonlSession::create(dir.path(), "fixture", None).unwrap())
            .unwrap();
        agent.set_context_vm_mode(crate::runtime::ContextVmMode::Hybrid);
        assert_eq!(hybrid_stats(&agent), before);
    }
}

#[test]
fn audit_wp02_b030_history_manifest_entries_carry_role_provenance() {
    use runtime::context_manifest::ProvenanceKind;
    let mut agent = Agent::new("");
    agent
        .messages
        .push(ChatMessage::text("user", "build the parser"));
    agent
        .messages
        .push(ChatMessage::text("assistant", "I will start now"));
    agent
        .messages
        .push(ChatMessage::text("toolResult", "exit code 0"));
    let manifest = agent.prepare_context_manifest("request", RunId::new(), 1, 1);
    let history: Vec<_> = manifest
        .entries
        .iter()
        .filter(|entry| entry.category == "history")
        .collect();
    let kinds: Vec<_> = history.iter().map(|entry| entry.provenance_kind).collect();
    assert_eq!(
        kinds,
        [
            ProvenanceKind::UserDecision,
            ProvenanceKind::AgentInference,
            ProvenanceKind::ToolEvidence
        ]
    );
    // The inspector's serialized form keeps the distinction.
    let json = serde_json::to_value(&manifest).unwrap().to_string();
    assert!(json.contains("user_decision") && json.contains("agent_inference"));
}

#[test]
fn audit_wp02_b029_malformed_message_fails_import_and_changes_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let mut good = JsonlSession::create(dir.path(), "fixture", Some("good")).unwrap();
    good.append_entry(SessionEntry::message(
        "user",
        serde_json::json!([{"type": "text", "text": "keep me"}]),
    ))
    .unwrap();
    let good_id = good.header.id.clone();
    let mut agent = Agent::new("fixture");
    agent.load_from_session(good).unwrap();
    let messages_before = agent.messages.clone();
    assert_eq!(messages_before.len(), 1);

    let other = tempfile::tempdir().unwrap();
    let mut bad = JsonlSession::create(other.path(), "fixture", Some("bad")).unwrap();
    bad.append_entry(SessionEntry::message(
        "user",
        serde_json::json!([{"type": "text", "text": "fine"}]),
    ))
    .unwrap();
    bad.append_entry(SessionEntry::message(
        "user",
        serde_json::json!([{"type": "no_such_part"}]),
    ))
    .unwrap();
    let broken_id = bad.entries.last().unwrap().id.clone();
    assert!(entry_to_chat(bad.entries.last().unwrap()).is_none());
    let error = agent.load_from_session(bad).unwrap_err();
    assert!(error.contains(&broken_id), "{error}");
    assert!(error.contains("malformed message"), "{error}");
    // Current context and session are untouched.
    assert_eq!(agent.messages, messages_before);
    assert_eq!(agent.session.as_ref().unwrap().header.id, good_id);

    // Valid message, custom and compaction entries still load.
    let third = tempfile::tempdir().unwrap();
    let mut valid = JsonlSession::create(third.path(), "fixture", Some("valid")).unwrap();
    valid
        .append_entry(SessionEntry::message(
            "user",
            serde_json::json!([{"type": "text", "text": "hello"}]),
        ))
        .unwrap();
    let mut custom = SessionEntry::message("user", serde_json::json!(""));
    custom.entry_type = "custom_message".into();
    custom.message = None;
    custom.custom_type = Some("hint".into());
    custom.extra.insert(
        "content".into(),
        serde_json::json!([{"type": "text", "text": "extra context"}]),
    );
    valid.append_entry(custom).unwrap();
    let mut compaction = SessionEntry::message("user", serde_json::json!(""));
    compaction.entry_type = "compaction".into();
    compaction.message = None;
    compaction
        .extra
        .insert("summary".into(), serde_json::json!("earlier work"));
    valid.append_entry(compaction).unwrap();
    agent.load_from_session(valid).unwrap();
    assert!(!agent.messages.is_empty());
}

fn b029_text(role: &str, text: &str) -> SessionEntry {
    SessionEntry::message(role, serde_json::json!([{"type": "text", "text": text}]))
}

fn b029_broken() -> SessionEntry {
    SessionEntry::message("assistant", serde_json::json!([{"type": "no_such_part"}]))
}

/// A malformed message is refused wherever it sits: on an abandoned branch,
/// or in history a compaction hides. Neither is visible at initial load, but
/// both come back (navigation, a later compaction rewrite).
#[test]
fn audit_wp02_b029_malformed_messages_off_the_active_path_fail_the_load() {
    let good_dir = tempfile::tempdir().unwrap();
    let mut good = JsonlSession::create(good_dir.path(), "fixture", Some("good")).unwrap();
    good.append_entry(b029_text("user", "keep me")).unwrap();
    let mut agent = Agent::new("fixture");
    let good_id = good.header.id.clone();
    agent.load_from_session(good).unwrap();
    let messages_before = agent.messages.clone();

    // 1. Malformed message on a branch that is not the selected one.
    let dir = tempfile::tempdir().unwrap();
    let mut branched = JsonlSession::create(dir.path(), "fixture", Some("branched")).unwrap();
    branched.append_entry(b029_text("user", "root")).unwrap();
    let root = branched.entries.last().unwrap().id.clone();
    branched
        .append_entry(b029_text("assistant", "active"))
        .unwrap();
    let active = branched.entries.last().unwrap().id.clone();
    branched.set_leaf(Some(root));
    branched.append_entry(b029_broken()).unwrap();
    let abandoned = branched.entries.last().unwrap().id.clone();
    branched.set_leaf(Some(active));
    // The selected context is clean, so the old active-path check admitted it.
    assert!(
        !davinci_session::build_context_entries(&branched.entries, branched.leaf_id.as_deref())
            .iter()
            .any(|entry| entry.id == abandoned),
        "premise: the broken message is off the active path"
    );
    let error = agent.load_from_session(branched).unwrap_err();
    assert!(
        error.contains(&abandoned) && error.contains("malformed message"),
        "{error}"
    );

    // 2. Malformed message before a compaction: the context starts after it.
    let dir = tempfile::tempdir().unwrap();
    let mut compacted = JsonlSession::create(dir.path(), "fixture", Some("compacted")).unwrap();
    compacted.append_entry(b029_broken()).unwrap();
    let hidden = compacted.entries.last().unwrap().id.clone();
    compacted.append_entry(b029_text("user", "kept")).unwrap();
    let kept = compacted.entries.last().unwrap().id.clone();
    let mut compaction = SessionEntry::message("user", serde_json::json!(""));
    compaction.entry_type = "compaction".into();
    compaction.message = None;
    compaction
        .extra
        .insert("summary".into(), serde_json::json!("earlier work"));
    compaction
        .extra
        .insert("firstKeptEntryId".into(), serde_json::json!(kept));
    compacted.append_entry(compaction).unwrap();
    let context: Vec<_> =
        davinci_session::build_context_entries(&compacted.entries, compacted.leaf_id.as_deref())
            .into_iter()
            .map(|entry| entry.id.clone())
            .collect();
    assert!(
        !context.contains(&hidden),
        "premise: the compaction hides it"
    );
    let error = agent.load_from_session(compacted).unwrap_err();
    assert!(
        error.contains(&hidden) && error.contains("malformed message"),
        "{error}"
    );

    // 3. A summary entry that is not text would vanish from the context too.
    let dir = tempfile::tempdir().unwrap();
    let mut summarized = JsonlSession::create(dir.path(), "fixture", Some("summary")).unwrap();
    let mut hollow = SessionEntry::message("user", serde_json::json!(""));
    hollow.entry_type = "branch_summary".into();
    hollow.message = None;
    summarized.append_entry(hollow).unwrap();
    assert!(agent
        .load_from_session(summarized)
        .unwrap_err()
        .contains("summary"));

    // A failed load changes nothing.
    assert_eq!(agent.messages, messages_before);
    assert_eq!(agent.session.as_ref().unwrap().header.id, good_id);
}

/// Navigation to a branch that would silently drop a message is refused before
/// anything moves: not the leaf, not the persisted entries, not the messages.
#[test]
fn audit_wp02_b029_navigation_refuses_a_branch_with_a_malformed_message() {
    let dir = tempfile::tempdir().unwrap();
    let mut session = JsonlSession::create(dir.path(), "fixture", Some("nav")).unwrap();
    session.append_entry(b029_text("user", "root")).unwrap();
    let root = session.entries.last().unwrap().id.clone();
    session
        .append_entry(b029_text("assistant", "active"))
        .unwrap();
    let active = session.entries.last().unwrap().id.clone();
    let mut agent = Agent::new("fixture");
    agent.load_from_session(session).unwrap();

    // A sibling branch is added after the load (another writer, a repair
    // tool): one valid reply and one that cannot be read.
    let (valid, broken) = {
        let session = agent.session.as_mut().unwrap();
        session.set_leaf(Some(root.clone()));
        session
            .append_entry(b029_text("assistant", "sibling"))
            .unwrap();
        let valid = session.entries.last().unwrap().id.clone();
        session.set_leaf(Some(root));
        session.append_entry(b029_broken()).unwrap();
        let broken = session.entries.last().unwrap().id.clone();
        session.set_leaf(Some(active.clone()));
        (valid, broken)
    };
    let entries_before = agent.session.as_ref().unwrap().entries.clone();
    let messages_before = agent.messages.clone();
    let id_before = agent.session.as_ref().unwrap().header.id.clone();

    for summarize in [false, true] {
        let error = agent
            .navigate_tree_entry(&broken, summarize, None, false, 0)
            .unwrap_err();
        assert!(
            error.contains(&broken) && error.contains("malformed message"),
            "{error}"
        );
        let session = agent.session.as_ref().unwrap();
        assert_eq!(session.leaf_id.as_deref(), Some(active.as_str()));
        assert_eq!(session.entries, entries_before, "nothing was appended");
        assert_eq!(session.header.id, id_before);
        assert_eq!(agent.messages, messages_before);
    }

    // The readable sibling is still reachable.
    agent
        .navigate_tree_entry(&valid, false, None, false, 0)
        .unwrap();
    assert_eq!(
        agent.session.as_ref().unwrap().leaf_id.as_deref(),
        Some(valid.as_str())
    );
    assert_ne!(agent.messages, messages_before);
}
