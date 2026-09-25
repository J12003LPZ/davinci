use super::*;

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
    assert!(competing
        .load_from_session(JsonlSession::open(&path).unwrap())
        .is_err());
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
    agent.prompt("aaaaaaaaaa bbbbbbbbbb");
    agent.record_assistant("cccccccccc");
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

    agent.prompt("message 1 for conversation history");
    agent.record_assistant("assistant response 1");
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

#[test]
fn parallel_workers_in_one_message_overlap_and_answer_in_order() {
    use std::time::{Duration, Instant};
    let mut agent = Agent::new(default_system_prompt());
    agent.set_permission_mode(PermissionMode::Ask);
    agent.approver = Some(ToolApprover(Arc::new(|_| ToolApprovalDecision::AllowOnce)));
    agent.subagent_runner = Some(SubagentRunner::new(|req| {
        std::thread::sleep(Duration::from_millis(80));
        Ok(format!("answer:{}", req.prompt))
    }));
    agent.prompt("go");
    let start = Instant::now();
    let events = agent
        .run_loop(scripted_batches(vec![vec![
            ("agent", serde_json::json!({"prompt": "one"})),
            ("agent", serde_json::json!({"prompt": "two"})),
            ("agent", serde_json::json!({"prompt": "three"})),
        ]]))
        .unwrap();
    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_millis(200),
        "three 80 ms workers took {elapsed:?}: they ran one after another"
    );
    let results: Vec<String> = agent
        .messages
        .iter()
        .filter(|message| message.role == "toolResult")
        .map(|message| content_text(&message.content))
        .collect();
    assert_eq!(results, ["answer:one", "answer:two", "answer:three"]);
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
            usage: davinci_protocol::Usage {
                input: 1,
                output: 2,
                total_tokens: 3,
                ..davinci_protocol::Usage::default()
            },
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
    agent.prompt("run");
    let events = agent
        .run_loop(|current| {
            if current
                .messages
                .iter()
                .any(|message| message.role == "toolResult")
            {
                return Ok(AssistantMessage {
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
    assert_eq!(entries.len(), 2);
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
            a.run_prepared_call(
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
    assert_eq!(
        state.latest_evidence.as_ref().map(|e| e.coverage),
        Some(VerificationCoverage::Unrelated)
    );
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
    agent.cwd = dir.to_path_buf();
    agent.set_permission_mode(PermissionMode::Ask);
    agent.approver = Some(ToolApprover(Arc::new(|_| ToolApprovalDecision::AllowOnce)));
    agent
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
            serde_json::json!({"command": "cargo check --help"}),
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

    assert_eq!(
        model_calls, 4,
        "write, verify, write, done: no reminder round trip"
    );
    assert_eq!(harness_runs(&agent), 1);
    assert!(reminders(&agent).is_empty(), "{:?}", reminders(&agent));
    assert_eq!(agent.completion_evidence(), CompletionEvidence::Verified);
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

    assert_eq!(harness_runs(&agent), 1);
    let reminders = reminders(&agent);
    assert_eq!(reminders.len(), 1, "{reminders:?}");
    assert!(
        reminders[0].contains("the harness re-ran"),
        "{}",
        reminders[0]
    );
    assert_eq!(model_calls, 5, "the model answers the reminder once");
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
