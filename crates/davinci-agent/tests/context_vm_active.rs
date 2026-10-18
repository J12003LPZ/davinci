use davinci_agent::runtime::context_vm::ContextVmMode;
use davinci_agent::{Agent, AgentId, RunId, RuntimeBus, RuntimeHandle};
use davinci_ai::{ChatMessage, MessageContent};

#[test]
fn prepared_context_is_reused_and_every_input_change_invalidates_it() {
    let mut agent = Agent::new("system");
    agent.set_runtime(RuntimeHandle::new(
        RunId::new(),
        AgentId::new(),
        RuntimeBus::new(),
    ));
    agent.set_context_vm_mode(ContextVmMode::Active);
    agent.messages.push(ChatMessage::text("user", "hello"));
    let first = agent.prepared_context_image().unwrap();
    let again = agent.prepared_context_image().unwrap();
    assert!(std::sync::Arc::ptr_eq(&first, &again));
    agent.messages[0] = ChatMessage::text("user", "corrected");
    let changed = agent.prepared_context_image().unwrap();
    assert!(!std::sync::Arc::ptr_eq(&first, &changed));
    agent.set_ephemeral_context(vec![ChatMessage::text("custom", "new overlay")]);
    let overlay = agent.prepared_context_image().unwrap();
    assert!(!std::sync::Arc::ptr_eq(&changed, &overlay));
    agent.set_permission_mode(davinci_agent::PermissionMode::ReadOnly);
    let permission_changed = agent.prepared_context_image().unwrap();
    assert!(!std::sync::Arc::ptr_eq(&overlay, &permission_changed));
    let mut overlay_v1 = davinci_agent::runtime::ContextOverlay::new(1);
    overlay_v1.exclude("item", false, true).unwrap();
    agent.set_context_overlay(Some(overlay_v1.clone()));
    let with_overlay = agent.prepared_context_image().unwrap();
    assert!(!std::sync::Arc::ptr_eq(&permission_changed, &with_overlay));
    // Installing an equal overlay changes nothing.
    agent.set_context_overlay(Some(overlay_v1.clone()));
    assert!(std::sync::Arc::ptr_eq(
        &with_overlay,
        &agent.prepared_context_image().unwrap()
    ));
    // A new revision, and new content at the same revision, both recompile.
    overlay_v1.add_correction("item", "fixed");
    agent.set_context_overlay(Some(overlay_v1.clone()));
    let revised = agent.prepared_context_image().unwrap();
    assert!(!std::sync::Arc::ptr_eq(&with_overlay, &revised));
    let mut same_revision = overlay_v1.clone();
    same_revision.pinned_ids.insert("other".into());
    agent.set_context_overlay(Some(same_revision));
    assert!(!std::sync::Arc::ptr_eq(
        &revised,
        &agent.prepared_context_image().unwrap()
    ));
    assert_eq!(
        agent
            .runtime
            .as_ref()
            .unwrap()
            .context_vm
            .metrics()
            .images_compiled,
        7
    );
}

struct FixedItems(Vec<davinci_agent::ContextItem>);
impl davinci_agent::ContextSource for FixedItems {
    fn collect(&self, _request: &davinci_agent::ContextRequest) -> Vec<davinci_agent::ContextItem> {
        self.0.clone()
    }
}

fn broker_item(id: &str, text: &str, bytes: usize, priority: i32) -> davinci_agent::ContextItem {
    davinci_agent::ContextItem {
        source: format!("memory-{id}"),
        content: format!("{text}{}", "x".repeat(bytes)),
        estimated_tokens: bytes as u64,
        priority,
        stable_for_cache: false,
        authority: Default::default(),
        provenance: serde_json::json!({ "id": id }),
    }
}

fn agent_with_items(items: Vec<davinci_agent::ContextItem>) -> Agent {
    let mut agent = Agent::new("system");
    let runtime = RuntimeHandle::new(RunId::new(), AgentId::new(), RuntimeBus::new());
    runtime
        .context_broker
        .register_context_source(std::sync::Arc::new(FixedItems(items)));
    agent.set_runtime(runtime);
    agent.set_context_vm_mode(ContextVmMode::Active);
    agent.context_window = 200_000;
    agent.messages.push(ChatMessage::text("user", "hello"));
    agent
}

fn image_text(agent: &Agent) -> String {
    let image = agent.prepared_context_image().unwrap();
    serde_json::to_string(&image.messages).unwrap()
}

#[test]
fn overlay_exclusion_removes_broker_item_from_next_compiled_image() {
    let mut agent = agent_with_items(vec![
        broker_item("keep", "KEEP-MARKER", 10, 10),
        broker_item("drop", "DROP-MARKER", 10, 10),
    ]);
    let before = image_text(&agent);
    assert!(before.contains("KEEP-MARKER") && before.contains("DROP-MARKER"));
    let mut overlay = davinci_agent::runtime::ContextOverlay::new(1);
    overlay.exclude("drop", false, true).unwrap();
    agent.set_context_overlay(Some(overlay));
    let after = image_text(&agent);
    assert!(after.contains("KEEP-MARKER"));
    assert!(!after.contains("DROP-MARKER"));
    agent.set_context_overlay(None);
    assert!(image_text(&agent).contains("DROP-MARKER"));
}

#[test]
fn overlay_pin_survives_budget_pressure_and_correction_replaces_text() {
    let budget = agent_with_items(vec![]).provider_context_budget();
    let bytes = (budget.working_set_budget() as usize) * 55 / 100;
    let mut agent = agent_with_items(vec![
        broker_item("loud", "LOUD-MARKER", bytes, 100),
        broker_item("quiet", "QUIET-MARKER", bytes, 1),
    ]);
    let plain = image_text(&agent);
    assert!(plain.contains("LOUD-MARKER") && !plain.contains("QUIET-MARKER"));
    let mut overlay = davinci_agent::runtime::ContextOverlay::new(1);
    overlay.pin("quiet", false, true).unwrap();
    agent.set_context_overlay(Some(overlay.clone()));
    let pinned = image_text(&agent);
    assert!(pinned.contains("QUIET-MARKER") && !pinned.contains("LOUD-MARKER"));

    overlay.add_correction("quiet", "CORRECTED-TEXT");
    agent.set_context_overlay(Some(overlay));
    let corrected = image_text(&agent);
    assert!(corrected.contains("CORRECTED-TEXT"));
    assert!(corrected.contains("user correction of memory-quiet"));
    assert!(!corrected.contains("QUIET-MARKER"));
}

#[test]
fn installed_overlay_revision_drives_the_manifest() {
    let mut agent = agent_with_items(vec![broker_item("a", "A", 10, 1)]);
    let mut overlay = davinci_agent::runtime::ContextOverlay::new(7);
    overlay.exclude("a", false, true).unwrap();
    let revision = overlay.revision;
    agent.set_context_overlay(Some(overlay));
    let manifest = agent.prepare_context_manifest("r", RunId::new(), 1, 1);
    assert_eq!(manifest.overlay_revision, revision);
    assert!(manifest
        .entries
        .iter()
        .all(|entry| !entry.source_ref.contains("memory-a")));
}

#[test]
fn manual_fold_invokes_structured_summarizer_and_applies_a_user_correction() {
    let mut agent = Agent::new("system");
    agent.set_runtime(RuntimeHandle::new(
        RunId::new(),
        AgentId::new(),
        RuntimeBus::new(),
    ));
    agent.set_context_vm_mode(ContextVmMode::Active);
    agent.messages = vec![
        ChatMessage::text("user", "use old API"),
        ChatMessage::text("user", "use new API"),
    ];
    let events = davinci_agent::runtime::context_vm::events_from_messages(&agent.messages);
    let new_ref = events[1].source_ref.clone();
    agent.summarizer = Some(davinci_agent::Summarizer::new(move |request| {
        assert!(request.prompt.contains("custom fold instruction"));
        assert!(request.system.contains("JSON"));
        Ok(davinci_agent::SummarizeResponse {
            text: serde_json::json!({"transitions":[{"slot":"goal","kind":"supersede",
                "previous":"use old API","evidence":{"value":"use new API",
                "source_refs":[new_ref],"provenance_kind":"user_decision"},
                "replacement":{"value":"use new API","source_refs":[new_ref],"provenance_kind":"user_decision"}}]}).to_string(),
            usage: Default::default(), stop_reason: None, error_message: None, has_tool_call: false,
        })
    }));
    agent
        .fold_context(
            davinci_agent::runtime::context_vm::FoldReason::Manual,
            Some("custom fold instruction"),
        )
        .unwrap();
    let state = agent
        .runtime
        .as_ref()
        .unwrap()
        .context_vm
        .load_state_from_root()
        .unwrap();
    assert_eq!(state.goals.len(), 1);
    assert_eq!(state.goals[0].value, "use new API");
    assert_eq!(state.retired.len(), 1);
}

#[test]
fn materialized_updates_do_not_repeat_full_states_in_a_delta_chain() {
    use davinci_agent::runtime::context_vm::*;
    let vm = ContextVmRuntime::new(ContextVmConfig::default(), Default::default());
    let mut messages = Vec::new();
    for n in 0..12 {
        messages.push(ChatMessage::text("user", format!("goal {n}")));
        vm.append_delta(&events_from_messages(&messages)).unwrap();
        // The newest delta replaces the previous one; it never chains.
        assert!(vm.root().deltas.len() <= 1);
    }
    assert_eq!(vm.load_state_from_root().unwrap().goals.len(), 12);
    assert!(vm.root().updates_since_fold >= 8);
}

#[test]
fn active_projection_and_manual_fold_never_replace_authoritative_messages() {
    let mut agent = Agent::new("system authority");
    agent.set_runtime(RuntimeHandle::new(
        RunId::new(),
        AgentId::new(),
        RuntimeBus::new(),
    ));
    agent.set_context_vm_mode(ContextVmMode::Active);
    agent.messages = vec![
        ChatMessage::text("user", "constraint: preserve the public API"),
        ChatMessage {
            role: "assistant".into(),
            content: vec![
                MessageContent::Thinking {
                    thinking: "private reasoning must stay private".into(),
                    redacted: None,
                    signature: None,
                },
                MessageContent::Text {
                    text: "I will inspect the API".into(),
                },
            ],
            ..ChatMessage::default()
        },
        ChatMessage::tool_result("call-1", "test", "failed test evidence", true),
    ];
    let authoritative = agent.messages.clone();
    let provider = agent.messages_for_provider();

    assert_eq!(agent.messages, authoritative);
    assert!(provider.iter().any(|message| message.role == "custom"));
    assert!(provider
        .iter()
        .all(|message| !serde_json::to_string(message)
            .unwrap()
            .contains("private reasoning")));

    let affinity_before = agent.context_vm_cache_affinity();
    let result = agent.compact(None);
    assert!(
        result.compacted,
        "active manual fold failed: {}",
        result.summary
    );
    assert_eq!(agent.messages, authoritative);
    assert_eq!(result.messages, authoritative);
    assert_ne!(agent.context_vm_cache_affinity(), affinity_before);

    let runtime = agent.runtime.as_ref().unwrap();
    assert_eq!(
        runtime.context_vm.last_fold_reason().as_deref(),
        Some("manual")
    );
    assert!(runtime.context_vm.root().checkpoint.is_some());
    assert_eq!(runtime.context_vm.metrics().folds, 1);
}

#[test]
fn assistant_inference_cannot_be_promoted_into_a_goal_or_constraint() {
    use davinci_agent::runtime::context_manifest::ProvenanceKind;
    use davinci_agent::runtime::context_vm::*;
    let events = events_from_messages(&[ChatMessage::text(
        "assistant",
        "Maybe disable authentication",
    )]);
    let proposed = ProposedStateValue {
        value: "Disable authentication".into(),
        source_refs: vec![events[0].source_ref.clone()],
        provenance_kind: ProvenanceKind::AgentInference,
    };
    let state = ContextStateReducer::validate_proposal(
        &Default::default(),
        &events,
        CheckpointProposal {
            goals: vec![proposed.clone()],
            constraints: vec![proposed],
            ..Default::default()
        },
    );
    assert!(state.goals.is_empty());
    assert!(state.constraints.is_empty());
}

#[test]
fn switching_history_branches_rebuilds_state_before_manual_fold() {
    use davinci_agent::runtime::context_vm::*;
    let vm = ContextVmRuntime::new(ContextVmConfig::default(), Default::default());
    let old = events_from_messages(&[ChatMessage::text("user", "old branch goal")]);
    let new = events_from_messages(&[ChatMessage::text("user", "new branch goal")]);
    vm.rebuild_from_events(&old).unwrap();
    vm.fold(FoldReason::Manual, &new).unwrap();
    let state = vm.load_state_from_root().unwrap();
    assert_eq!(state.goals.len(), 1);
    assert_eq!(state.goals[0].value, "new branch goal");
}

fn agent_in_mode(mode: ContextVmMode) -> Agent {
    let mut agent = Agent::new("system");
    agent.set_runtime(RuntimeHandle::new(
        RunId::new(),
        AgentId::new(),
        RuntimeBus::new(),
    ));
    agent.set_context_vm_mode(mode);
    agent
}

fn provider_tool_names(agent: &Agent) -> Vec<String> {
    agent
        .provider_tool_specs()
        .into_iter()
        .map(|tool| tool.name)
        .collect()
}

/// Enough history that the hot window (20k tokens) pages older events out.
fn long_history() -> Vec<ChatMessage> {
    (0..4)
        .map(|n| ChatMessage::text("user", format!("goal {n} {}", "x".repeat(40_000))))
        .collect()
}

#[test]
fn retrieve_context_is_offered_only_by_an_active_vm_that_folded_or_paged() {
    for mode in [ContextVmMode::Off, ContextVmMode::Shadow] {
        let mut agent = agent_in_mode(mode);
        agent.expose_active_tools();
        agent.messages = long_history();
        let _ = agent.messages_for_provider();
        assert!(!provider_tool_names(&agent).contains(&"retrieve_context".to_string()));
    }

    let mut agent = agent_in_mode(ContextVmMode::Active);
    agent.messages = vec![ChatMessage::text("user", "short task")];
    agent.prepared_context_image().unwrap();
    assert!(!agent.context_vm_offers_retrieval(), "nothing paged yet");
    assert!(!provider_tool_names(&agent).contains(&"retrieve_context".to_string()));

    agent.compact(None);
    assert!(agent.context_vm_offers_retrieval());
    assert!(provider_tool_names(&agent).contains(&"retrieve_context".to_string()));

    let mut paged = agent_in_mode(ContextVmMode::Active);
    paged.messages = long_history();
    paged.prepared_context_image().unwrap();
    assert!(
        paged.context_vm_offers_retrieval(),
        "hot window paged events out"
    );
}

#[test]
fn folded_episode_placeholder_names_the_recovery_tool_and_page() {
    let mut agent = agent_in_mode(ContextVmMode::Active);
    agent.messages = vec![ChatMessage::text("user", "remember the build flags")];
    assert!(agent.compact(None).compacted);
    let root = agent.runtime.as_ref().unwrap().context_vm.root();
    let episode = root
        .episodes
        .first()
        .expect("fold records an episode")
        .clone();
    let image = agent.prepared_context_image().unwrap();
    let placeholder = image
        .entries
        .iter()
        .find(|entry| entry.category == "episode")
        .expect("episode placeholder in the image");
    assert!(placeholder.content.contains("retrieve_context"));
    assert!(placeholder
        .content
        .contains(&format!("page={}", episode.id)));
    let recovered = agent
        .runtime
        .as_ref()
        .unwrap()
        .context_vm
        .retrieve(
            &davinci_agent::runtime::context_vm::RetrieveContextRequest {
                page: Some(episode.id.clone()),
                ..Default::default()
            },
        )
        .unwrap();
    assert!(recovered.content.contains("Context fold"));
}

#[test]
fn active_fallbacks_are_recorded_in_manifest_and_status_and_noticed_once() {
    let mut agent = Agent::new("s".repeat(2048));
    agent.set_runtime(RuntimeHandle::new(
        RunId::new(),
        AgentId::new(),
        RuntimeBus::new(),
    ));
    agent.set_context_vm_mode(ContextVmMode::Active);
    agent.context_window = 4096;
    agent.thinking_level = davinci_protocol::ThinkingLevel::High;
    agent.set_provider_context_overhead_tokens(Some(1024));
    agent.messages.push(ChatMessage::text("user", "hello"));

    let legacy = agent.legacy_messages_for_provider_for_test();
    assert_eq!(agent.messages_for_provider(), legacy, "legacy fallback");
    let vm = agent.runtime.as_ref().unwrap().context_vm.clone();
    assert_eq!(vm.failure_count(), 1);
    assert_eq!(vm.recent_failures()[0].stage, "compile");
    let notices = agent.take_context_vm_notices();
    assert_eq!(notices.len(), 1, "{notices:?}");
    assert!(notices[0].contains("compile failed"));

    let manifest = agent.prepare_context_manifest("request", RunId::new(), 1, 1);
    assert!(manifest
        .entries
        .iter()
        .any(|entry| entry.id == "context_vm_budget" && entry.mandatory));
    assert!(manifest.entries.iter().any(|entry| {
        entry.id.starts_with("context_vm_failure_")
            && entry
                .selection_reason
                .as_deref()
                .is_some_and(|reason| reason.starts_with("compile: "))
    }));

    // A different failure is counted, but the session was already told once.
    vm.record_failure("append_delta", "page store unavailable");
    assert_eq!(vm.failure_count(), 2);
    assert!(agent.take_context_vm_notices().is_empty());
}

#[test]
fn compact_works_in_active_mode_before_the_first_prompt() {
    let mut agent = Agent::new("system");
    agent.set_context_vm_mode(ContextVmMode::Active);
    agent.messages = vec![ChatMessage::text("user", "resume the earlier work")];
    assert!(agent.runtime.is_none());
    let result = agent.compact(None);
    assert!(result.compacted, "{}", result.summary);
    let vm = agent.runtime.as_ref().unwrap().context_vm.clone();
    assert_eq!(vm.metrics().folds, 1);

    // The next prompt's fresh handle keeps the folded state.
    agent.set_runtime(RuntimeHandle::new(
        RunId::new(),
        AgentId::new(),
        RuntimeBus::new(),
    ));
    let carried = &agent.runtime.as_ref().unwrap().context_vm;
    assert!(carried.shares_state_with(&vm));
    assert_eq!(carried.metrics().folds, 1);
}

#[test]
fn context_vm_state_persists_across_prompt_handles_within_one_session() {
    let directory = tempfile::tempdir().unwrap();
    let mut agent = agent_in_mode(ContextVmMode::Active);
    agent.session =
        Some(davinci_session::JsonlSession::create(directory.path(), "fixture", None).unwrap());
    agent.set_runtime(RuntimeHandle::new(
        RunId::new(),
        AgentId::new(),
        RuntimeBus::new(),
    ));
    let first = agent.runtime.as_ref().unwrap().context_vm.clone();
    first.record_failure("compile", "first prompt failure");
    agent.set_runtime(RuntimeHandle::new(
        RunId::new(),
        AgentId::new(),
        RuntimeBus::new(),
    ));
    let second = agent.runtime.as_ref().unwrap().context_vm.clone();
    assert!(second.shares_state_with(&first));
    assert_eq!(second.failure_count(), 1);

    agent.session =
        Some(davinci_session::JsonlSession::create(directory.path(), "other", None).unwrap());
    agent.set_runtime(RuntimeHandle::new(
        RunId::new(),
        AgentId::new(),
        RuntimeBus::new(),
    ));
    let switched = &agent.runtime.as_ref().unwrap().context_vm;
    assert!(
        !switched.shares_state_with(&first),
        "a new session starts a new VM"
    );
    assert_eq!(switched.failure_count(), 0);
}

#[test]
fn shadow_mismatch_is_noticed_once_and_matching_views_stay_quiet() {
    let mut agent = agent_in_mode(ContextVmMode::Shadow);
    agent.messages = vec![ChatMessage::text("user", "small")];
    let _ = agent.messages_for_provider();
    assert!(agent.take_context_vm_notices().is_empty());

    agent.messages = long_history();
    let _ = agent.messages_for_provider();
    let notices = agent.take_context_vm_notices();
    assert_eq!(notices.len(), 1, "{notices:?}");
    assert!(notices[0].contains("Context VM shadow"));
    agent.messages.push(ChatMessage::text("user", "more"));
    let _ = agent.messages_for_provider();
    assert!(
        agent.take_context_vm_notices().is_empty(),
        "persisting mismatch"
    );
}

#[test]
fn automatic_fold_is_noticed_and_manual_fold_is_not() {
    let mut agent = agent_in_mode(ContextVmMode::Active);
    agent.messages = vec![ChatMessage::text("user", "task")];
    agent.compact(None);
    assert!(agent.take_context_vm_notices().is_empty());
    agent
        .fold_context(
            davinci_agent::runtime::context_vm::FoldReason::WindowPressure,
            None,
        )
        .unwrap();
    let notices = agent.take_context_vm_notices();
    assert_eq!(notices.len(), 1);
    assert!(notices[0].contains("folded"));
    assert!(notices[0].contains("retrieve_context"));
}

/// A long active-VM session must still fold on its own. Admission caps the
/// image in bytes, so a fold decision fed the four-bytes-a-token estimate
/// never saw more than a quarter of the window and never folded; older turns
/// were paged out with no summary instead.
#[test]
fn a_full_active_window_folds_from_the_run_loop() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    let mut agent = Agent::new("You are a coding agent.");
    agent.context_window = 100_000;
    agent.auto_compaction = true;
    agent.set_runtime(RuntimeHandle::new(
        RunId::new(),
        AgentId::new(),
        RuntimeBus::new(),
    ));
    agent.set_context_vm_mode(ContextVmMode::Active);
    let filler = "word ".repeat(1_600);
    for index in 0..14 {
        agent.messages.push(ChatMessage::text(
            "user",
            format!("request {index}: {filler}"),
        ));
        agent.messages.push(ChatMessage::text(
            "assistant",
            format!("reply {index}: {filler}"),
        ));
    }
    agent.prompt("continue");
    let folds = Arc::new(AtomicUsize::new(0));
    let seen = folds.clone();
    agent.summarizer = Some(davinci_agent::Summarizer::new(move |_| {
        seen.fetch_add(1, Ordering::SeqCst);
        Ok(davinci_agent::SummarizeResponse {
            text: serde_json::json!({"transitions": []}).to_string(),
            usage: Default::default(),
            stop_reason: None,
            error_message: None,
            has_tool_call: false,
        })
    }));

    agent
        .run_loop(|_: &Agent| {
            Ok::<_, String>(davinci_ai::AssistantMessage {
                extra: Default::default(),
                id: "fixture".into(),
                role: "assistant".into(),
                model: "fixture".into(),
                usage: None,
                error_message: None,
                content: vec![davinci_ai::ContentBlock::Text {
                    text: "done".into(),
                }],
                stop_reason: Some(davinci_ai::StopReason::Stop),
            })
        })
        .expect("turn runs");

    assert!(
        folds.load(Ordering::SeqCst) > 0,
        "a window the admission budget fills must fold"
    );
    // `/context` still reads in tokens, well under the admission figure.
    let usage = agent.context_usage();
    assert_eq!(usage.used(), agent.estimated_context_tokens());
}
