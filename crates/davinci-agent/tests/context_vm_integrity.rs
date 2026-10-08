//! Context VM integrity: what reaches the provider and what the checkpoint
//! may claim on the user's behalf (WOR-58 to WOR-62, WOR-72).

use davinci_agent::runtime::context_manifest::ProvenanceKind;
use davinci_agent::runtime::context_vm::{
    CheckpointProposal, ContextEvent, ContextEventKind, ContextStateReducer, ProposedStateValue,
    StateSlot, StateTransition, TransitionKind,
};
use davinci_agent::runtime::context_vm::{ContextVmMode, FoldReason};
use davinci_agent::{Agent, AgentId, RunId, RuntimeBus, RuntimeHandle};
use davinci_ai::ChatMessage;

fn event(source_ref: &str, seq: u64, kind: ContextEventKind, text: &str) -> ContextEvent {
    let provenance_kind = match kind {
        ContextEventKind::User => ProvenanceKind::UserDecision,
        ContextEventKind::ToolResult => ProvenanceKind::ToolEvidence,
        _ => ProvenanceKind::AgentInference,
    };
    ContextEvent {
        source_ref: source_ref.into(),
        seq,
        kind,
        provenance_kind,
        content_hash: format!("hash-{source_ref}"),
        visible_text: text.into(),
        artifact_refs: Vec::new(),
        images: Vec::new(),
    }
}

fn user_value(value: &str, source_ref: &str) -> ProposedStateValue {
    ProposedStateValue {
        value: value.into(),
        source_refs: vec![source_ref.into()],
        provenance_kind: ProvenanceKind::UserDecision,
    }
}

fn all_text(state: &davinci_agent::runtime::context_vm::CheckpointState) -> String {
    serde_json::to_string(state).unwrap()
}

/// WOR-62: a summarizer must not attach a claim the user never made to a
/// real user message and have it stored as the user's own constraint.
#[test]
fn wor62_forged_user_constraint_citing_unrelated_text_is_rejected() {
    let events = vec![event("user:1", 1, ContextEventKind::User, "hello")];
    let parent =
        ContextStateReducer::deterministic_delta(&Default::default(), &events).checkpoint_patch;
    let proposal = CheckpointProposal {
        constraints: vec![user_value("User requires deleting all files", "user:1")],
        goals: vec![user_value("ship the release tonight", "user:1")],
        decisions: vec![user_value("drop the database", "user:1")],
        ..CheckpointProposal::default()
    };
    let state = ContextStateReducer::validate_proposal(&parent, &events, proposal);
    assert!(state.constraints.is_empty(), "{:?}", state.constraints);
    let text = all_text(&state);
    for forged in ["deleting", "release tonight", "drop the database"] {
        assert!(!text.contains(forged), "{forged} was accepted: {text}");
    }
}

#[test]
fn wor62_grounded_user_values_may_drop_function_words() {
    let events = vec![event(
        "user:1",
        1,
        ContextEventKind::User,
        "Please keep the public API stable. Never touch the migrations folder; ship by Friday.",
    )];
    let proposal = CheckpointProposal {
        goals: vec![user_value("keep public API stable", "user:1")],
        // Consecutive clauses may be quoted together.
        constraints: vec![user_value(
            "never touch migrations folder, ship by Friday",
            "user:1",
        )],
        ..CheckpointProposal::default()
    };
    let state = ContextStateReducer::validate_proposal(&Default::default(), &events, proposal);
    assert_eq!(state.goals.len(), 1);
    assert_eq!(state.goals[0].value, "keep public API stable");
    assert_eq!(state.constraints.len(), 1);
    // Part of a clause is not a quote of it.
    let proposal = CheckpointProposal {
        constraints: vec![user_value("touch the migrations folder", "user:1")],
        ..CheckpointProposal::default()
    };
    let state = ContextStateReducer::validate_proposal(&Default::default(), &events, proposal);
    assert!(state.constraints.is_empty());
    // A negation is a content word: it cannot be added to a quote either.
    let proposal = CheckpointProposal {
        constraints: vec![user_value("do not keep the public API stable", "user:1")],
        ..CheckpointProposal::default()
    };
    let state = ContextStateReducer::validate_proposal(&Default::default(), &events, proposal);
    assert!(state.constraints.is_empty());
}

/// Each value below uses only the user's words but says something the user
/// did not: reordered, cut out of a negated clause, a question, or reported
/// speech.
#[test]
fn wor62_quotes_cannot_invert_or_restitch_the_users_meaning() {
    let cases = [
        ("Use tabs, not spaces.", "use spaces, not tabs"),
        ("Use tabs, not spaces.", "spaces"),
        (
            "Don't delete the tests; clean the build dir.",
            "delete the tests",
        ),
        (
            "Don't delete the tests; clean the build dir.",
            "don't clean the build dir",
        ),
        ("Remove the env section from the docs.", "remove .env"),
        (
            "Never push to main without review. Push the branch.",
            "push to main without review",
        ),
        (
            "Do not, under any circumstances, drop the table.",
            "drop the table",
        ),
        (
            "Deleting the tests is not acceptable.",
            "deleting the tests",
        ),
        (
            "Should we drop the production table?",
            "drop the production table",
        ),
        (
            "Should we drop the production table?",
            "we drop the production table",
        ),
        ("Rewrite it in Rust? No way.", "rewrite it in rust"),
        (
            "I was going to force push to main but changed my mind.",
            "force push to main",
        ),
        (
            "My coworker said to disable the auth check; I disagree.",
            "disable the auth check",
        ),
    ];
    for (said, forged) in cases {
        let events = vec![event("user:1", 1, ContextEventKind::User, said)];
        let proposal = CheckpointProposal {
            constraints: vec![user_value(forged, "user:1")],
            ..CheckpointProposal::default()
        };
        let state = ContextStateReducer::validate_proposal(&Default::default(), &events, proposal);
        assert!(
            state.constraints.is_empty(),
            "{forged:?} accepted from {said:?}"
        );
    }
    // The faithful quotes from the same messages still pass.
    let faithful = [
        ("Use tabs, not spaces.", "use tabs, not spaces"),
        (
            "Don't delete the tests; clean the build dir.",
            "clean the build dir",
        ),
        (
            "Don't delete the tests; clean the build dir.",
            "don't delete the tests",
        ),
        (
            "Never push to main without review. Push the branch.",
            "never push to main without review",
        ),
    ];
    for (said, quote) in faithful {
        let events = vec![event("user:1", 1, ContextEventKind::User, said)];
        let proposal = CheckpointProposal {
            constraints: vec![user_value(quote, "user:1")],
            ..CheckpointProposal::default()
        };
        let state = ContextStateReducer::validate_proposal(&Default::default(), &events, proposal);
        assert_eq!(
            state.constraints.len(),
            1,
            "{quote:?} rejected from {said:?}"
        );
    }
}

/// Words from two different messages cannot be stitched into one claim.
#[test]
fn wor62_a_quote_comes_from_one_cited_source() {
    let events = vec![
        event("user:1", 1, ContextEventKind::User, "delete"),
        event("user:2", 2, ContextEventKind::User, "all the files"),
    ];
    let proposal = CheckpointProposal {
        constraints: vec![ProposedStateValue {
            value: "delete all files".into(),
            source_refs: vec!["user:1".into(), "user:2".into()],
            provenance_kind: ProvenanceKind::UserDecision,
        }],
        ..CheckpointProposal::default()
    };
    let state = ContextStateReducer::validate_proposal(&Default::default(), &events, proposal);
    assert!(state.constraints.is_empty());
}

/// A forged reason cannot retire a real user goal either.
#[test]
fn wor62_transition_evidence_must_be_grounded_in_the_newer_user_text() {
    let events = vec![
        event("user:1", 1, ContextEventKind::User, "use the old API"),
        event("user:2", 2, ContextEventKind::User, "hello again"),
    ];
    let parent = ContextStateReducer::deterministic_delta(&Default::default(), &events[..1])
        .checkpoint_patch;
    let forged = CheckpointProposal {
        transitions: vec![StateTransition {
            slot: StateSlot::Goal,
            kind: TransitionKind::Reject,
            previous: "use the old API".into(),
            evidence: user_value("user abandoned the API work", "user:2"),
            replacement: None,
        }],
        ..CheckpointProposal::default()
    };
    let state = ContextStateReducer::validate_proposal(&parent, &events, forged);
    assert!(state
        .goals
        .iter()
        .any(|goal| goal.value == "use the old API"));
    assert!(state.retired.is_empty());
}

/// Folded events leave the event list; a value citing one is grounded in the
/// parent values that already cite it.
#[test]
fn wor62_values_citing_folded_sources_are_grounded_in_parent_state() {
    let folded = vec![event(
        "user:1",
        1,
        ContextEventKind::User,
        "keep the public API stable",
    )];
    let parent =
        ContextStateReducer::deterministic_delta(&Default::default(), &folded).checkpoint_patch;
    let current = vec![event("user:2", 2, ContextEventKind::User, "continue")];
    let proposal = CheckpointProposal {
        constraints: vec![
            user_value("keep public API stable", "user:1"),
            user_value("rewrite everything in Go", "user:1"),
        ],
        ..CheckpointProposal::default()
    };
    let state = ContextStateReducer::validate_proposal(&parent, &current, proposal);
    let values = state
        .constraints
        .iter()
        .map(|value| value.value.as_str())
        .collect::<Vec<_>>();
    assert_eq!(values, ["keep public API stable"]);
}

fn append_user(session: &mut davinci_session::JsonlSession, text: &str) {
    let seq = session.entries.len() as u64 + 1;
    session
        .append_entry(davinci_session::SessionEntry {
            id: format!("event-{seq}"),
            entry_type: "message".into(),
            parent_id: session.leaf_id.clone(),
            seq,
            timestamp: 0,
            message: Some(serde_json::to_value(ChatMessage::text("user", text)).unwrap()),
            custom_type: None,
            extra: Default::default(),
        })
        .unwrap();
}

fn runtime() -> RuntimeHandle {
    RuntimeHandle::new(RunId::new(), AgentId::new(), RuntimeBus::new())
}

fn active_agent_on(path: &std::path::Path) -> Agent {
    let mut agent = Agent::new("system");
    agent.set_context_vm_mode(ContextVmMode::Active);
    agent.session = Some(davinci_session::JsonlSession::open(path).unwrap());
    agent.set_runtime(runtime());
    agent
}

/// WOR-61: the checkpoint entry is the durable record of a fold. When it
/// cannot be written, the live VM must be exactly where it was, not a fold
/// ahead of what a reload would see.
#[test]
fn wor61_failed_checkpoint_persistence_rolls_the_fold_back() {
    let directory = tempfile::tempdir().unwrap();
    let mut session =
        davinci_session::JsonlSession::create(directory.path(), "fixture", None).unwrap();
    append_user(&mut session, "keep the public API stable");
    append_user(&mut session, "now add the retry flag");
    let path = session.path.clone();
    drop(session);

    let mut agent = active_agent_on(&path);
    agent.prepared_context_image().unwrap();
    let vm = agent.runtime.as_ref().unwrap().context_vm.clone();
    let root = vm.root();
    let state = vm.load_state_from_root().unwrap();
    let affinity = vm.cache_affinity();
    assert!(!agent.context_vm_offers_retrieval());

    // Another writer holds the session, so the checkpoint append fails.
    let mut lock = path.as_os_str().to_owned();
    lock.push(".lock");
    let held =
        davinci_sys::lock::ExclusiveFileLock::try_acquire(std::path::Path::new(&lock)).unwrap();
    let error = agent
        .fold_context(FoldReason::WindowPressure, None)
        .unwrap_err();
    assert!(
        error.contains("context checkpoint persistence failed"),
        "{error}"
    );
    drop(held);

    assert_eq!(vm.root(), root);
    assert_eq!(vm.load_state_from_root().unwrap(), state);
    assert_eq!(vm.cache_affinity(), affinity);
    assert_eq!(vm.metrics().folds, 0);
    assert_eq!(vm.last_fold_reason(), None);
    assert!(!agent.context_vm_offers_retrieval());
    assert_eq!(vm.failure_count(), 1, "the rollback is recorded");
    assert!(agent
        .session
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .all(|entry| entry.entry_type != "context_checkpoint"));

    // A process that reloads the session sees the same state.
    let reloaded = active_agent_on(&path);
    reloaded.prepared_context_image().unwrap();
    let reloaded_vm = &reloaded.runtime.as_ref().unwrap().context_vm;
    assert_eq!(reloaded_vm.load_state_from_root().unwrap(), state);
    assert_eq!(reloaded_vm.cache_affinity(), affinity);
    assert_eq!(
        agent.prepared_context_image().unwrap().prefix_digest,
        reloaded.prepared_context_image().unwrap().prefix_digest
    );
}

fn sessionless_active_agent() -> Agent {
    let mut agent = Agent::new("system");
    agent.set_context_vm_mode(ContextVmMode::Active);
    agent.set_runtime(runtime());
    agent
}

/// WOR-60: with no session, both bound ids are None and used to compare
/// equal, so an unrelated run on the same Agent inherited the old VM.
#[test]
fn wor60_unrelated_sessionless_run_starts_a_fresh_vm() {
    let mut agent = sessionless_active_agent();
    agent.messages = vec![ChatMessage::text("user", "private marker alpha-7731")];
    assert!(agent.compact(None).compacted);
    let first = agent.runtime.as_ref().unwrap().context_vm.clone();
    let episode = first.root().episodes[0].id.clone();
    first.record_failure("compile", "run A failure");

    agent.messages = vec![ChatMessage::text("user", "an unrelated task")];
    agent.set_runtime(runtime());
    let second = agent.runtime.as_ref().unwrap().context_vm.clone();
    assert!(!second.shares_state_with(&first));
    assert_eq!(second.failure_count(), 0);
    assert!(second.root().episodes.is_empty());
    assert!(!agent.context_vm_offers_retrieval());
    let image = agent.prepared_context_image().unwrap();
    let text = serde_json::to_string(&image.messages).unwrap();
    assert!(!text.contains("alpha-7731"), "{text}");
    assert!(second
        .retrieve(
            &davinci_agent::runtime::context_vm::RetrieveContextRequest {
                page: Some(episode),
                ..Default::default()
            }
        )
        .is_err());
    assert_ne!(second.cache_affinity(), first.cache_affinity());
}

/// The same conversation continuing on a new prompt handle keeps its VM.
#[test]
fn wor60_sessionless_continuation_keeps_its_vm() {
    let mut agent = sessionless_active_agent();
    agent.messages = vec![ChatMessage::text("user", "first step")];
    assert!(agent.compact(None).compacted);
    let first = agent.runtime.as_ref().unwrap().context_vm.clone();
    agent
        .messages
        .push(ChatMessage::text("user", "second step"));
    agent.set_runtime(runtime());
    let second = &agent.runtime.as_ref().unwrap().context_vm;
    assert!(second.shares_state_with(&first));
    assert_eq!(second.metrics().folds, 1);
}

/// A fold on a history the VM no longer matches must not hand the old
/// history's checkpoint to the summarizer as the parent state.
#[test]
fn wor60_fold_never_shows_another_historys_state_to_the_summarizer() {
    let mut agent = sessionless_active_agent();
    agent.messages = vec![ChatMessage::text("user", "private marker alpha-7731")];
    assert!(agent.compact(None).compacted);
    agent.messages = vec![ChatMessage::text("user", "an unrelated task")];
    let prompts = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = prompts.clone();
    agent.summarizer = Some(davinci_agent::Summarizer::new(move |request| {
        seen.lock().unwrap().push(request.prompt.clone());
        Err("no proposal".into())
    }));
    agent.fold_context(FoldReason::Manual, None).unwrap();
    let prompts = prompts.lock().unwrap();
    assert_eq!(prompts.len(), 1);
    assert!(!prompts[0].contains("alpha-7731"), "{}", prompts[0]);
    let state = agent
        .runtime
        .as_ref()
        .unwrap()
        .context_vm
        .load_state_from_root()
        .unwrap();
    assert!(!serde_json::to_string(&state)
        .unwrap()
        .contains("alpha-7731"));
}

fn tool_exchange(output: &str) -> Vec<ChatMessage> {
    vec![
        ChatMessage::text("user", "read the log"),
        ChatMessage {
            role: "assistant".into(),
            content: vec![davinci_ai::MessageContent::ToolCall {
                id: "live-call".into(),
                name: "read".into(),
                arguments: serde_json::json!({"path": "build.log"}),
            }],
            ..Default::default()
        },
        ChatMessage::tool_result("live-call", "read", output, false),
    ]
}

fn assert_exchange_sent_once(agent: &Agent, marker: &str) {
    let image = agent.context_vm_image().unwrap();
    let rendered = serde_json::to_string(&image.messages).unwrap();
    assert_eq!(rendered.matches(marker).count(), 1, "{rendered}");
    let wire = davinci_ai::openai_responses_input(&image.messages);
    let rendered = serde_json::to_string(&wire).unwrap();
    assert_eq!(rendered.matches(marker).count(), 1, "{rendered}");
    assert_eq!(rendered.matches("build.log").count(), 1, "{rendered}");
    assert!(wire
        .iter()
        .any(|item| item["type"] == "function_call_output"));
    assert!(image
        .entries
        .iter()
        .all(|entry| entry.category != "hot_tool_result" && entry.category != "hot_assistant"));
    let live = image
        .entries
        .iter()
        .filter(|entry| entry.category == "live_tool_exchange")
        .count();
    assert_eq!(live, 2);
}

/// WOR-59: the live exchange was compiled as hot evidence and then appended
/// again as native messages, so its output went out (and was budgeted) twice.
#[test]
fn wor59_live_tool_exchange_is_sent_exactly_once() {
    let mut agent = sessionless_active_agent();
    agent.messages = tool_exchange("live output marker-5521");
    assert_exchange_sent_once(&agent, "marker-5521");
}

#[test]
fn wor59_live_tool_exchange_is_sent_once_from_a_session() {
    let directory = tempfile::tempdir().unwrap();
    let mut session =
        davinci_session::JsonlSession::create(directory.path(), "fixture", None).unwrap();
    let messages = tool_exchange("live output marker-5522");
    for (index, message) in messages.iter().enumerate() {
        let seq = index as u64 + 1;
        session
            .append_entry(davinci_session::SessionEntry {
                id: format!("event-{seq}"),
                entry_type: "message".into(),
                parent_id: session.leaf_id.clone(),
                seq,
                timestamp: 0,
                message: Some(serde_json::to_value(message).unwrap()),
                custom_type: None,
                extra: Default::default(),
            })
            .unwrap();
    }
    let mut agent = Agent::new("system");
    agent.set_context_vm_mode(ContextVmMode::Active);
    agent.session = Some(session);
    agent.messages = messages;
    agent.set_runtime(runtime());
    assert_exchange_sent_once(&agent, "marker-5522");
}

/// An exchange that fits once but not twice must compile.
#[test]
fn wor59_large_live_output_is_budgeted_once() {
    let mut agent = sessionless_active_agent();
    let output = format!("marker-5523 {}", "x".repeat(60_000));
    agent.messages = tool_exchange(&output);
    let one_copy = davinci_agent::runtime::context_vm::events_from_messages(&agent.messages)
        .iter()
        .map(|event| event.visible_text.len() as u64)
        .sum::<u64>();
    let image = agent.context_vm_image().unwrap();
    assert!(
        // The estimate is a ceiling of about one token per byte; a second
        // copy would put it near twice the history.
        image.estimated_tokens < one_copy + one_copy / 2,
        "estimated {} tokens for {one_copy} bytes of history",
        image.estimated_tokens
    );
}

/// WOR-72: optional broker context was admitted before recent turns, so one
/// large optional item could push every hot event but the newest out.
#[test]
fn wor72_recent_turns_are_admitted_before_optional_broker_context() {
    use davinci_agent::runtime::context_vm::{
        events_from_messages, ContextVmConfig, ContextVmRuntime,
    };
    use davinci_agent::{ContextItem, ContextPacket};
    let vm = ContextVmRuntime::new(ContextVmConfig::default(), Default::default());
    let messages = (0..10)
        .map(|n| {
            let role = if n % 2 == 0 { "user" } else { "assistant" };
            ChatMessage::text(role, format!("turn-{n} {}", "detail ".repeat(40)))
        })
        .collect::<Vec<_>>();
    let events = events_from_messages(&messages);
    let broker = ContextPacket {
        items: vec![ContextItem {
            source: "file::docs/big.md".into(),
            content: "background ".repeat(2_000),
            estimated_tokens: 5_500,
            priority: 100,
            stable_for_cache: true,
            provenance: serde_json::json!({"provenance_kind": "repository_fact"}),
        }],
        estimated_tokens: 5_500,
        cache_key: "fixture".into(),
    };
    let everything = vm.compile(&events, &broker, 1_000_000).unwrap();
    let hot_tokens = everything
        .entries
        .iter()
        .filter(|entry| entry.category.starts_with("hot_"))
        .map(|entry| entry.estimated_tokens)
        .sum::<u64>();
    // The optional file alone fits, but only by evicting half the turns.
    let budget = everything.estimated_tokens - hot_tokens / 2;
    let image = vm.compile(&events, &broker, budget).unwrap();
    let hot = image
        .entries
        .iter()
        .filter(|entry| entry.category.starts_with("hot_"))
        .count();
    assert_eq!(hot, events.len(), "recent turns were evicted");
    assert!(image
        .entries
        .iter()
        .all(|entry| entry.category != "broker_context"));
    assert!(image.estimated_tokens <= budget);

    // With room for both, order on the wire is unchanged: broker, then hot.
    let categories = everything
        .entries
        .iter()
        .map(|entry| entry.category.as_str())
        .collect::<Vec<_>>();
    let broker_at = categories.iter().position(|c| *c == "broker_context");
    let first_hot = categories.iter().position(|c| c.starts_with("hot_"));
    assert!(broker_at < first_hot, "{categories:?}");
}

fn user_with_image(text: Option<&str>, data: &str) -> ChatMessage {
    let mut content = Vec::new();
    if let Some(text) = text {
        content.push(davinci_ai::MessageContent::Text { text: text.into() });
    }
    content.push(davinci_ai::MessageContent::Image {
        data: data.into(),
        mime_type: "image/png".into(),
    });
    ChatMessage {
        role: "user".into(),
        content,
        ..Default::default()
    }
}

fn wire_for(mode: ContextVmMode, messages: Vec<ChatMessage>) -> String {
    let mut agent = Agent::new("system");
    agent.set_context_vm_mode(mode);
    agent.set_runtime(runtime());
    agent.messages = messages;
    // The provider list every adapter encodes; the Responses test helper
    // has no image encoding, in any mode.
    serde_json::to_string(&agent.messages_for_provider()).unwrap()
}

/// WOR-58: the active VM projected only text, so an image-only message
/// vanished and a mixed message lost its image.
#[test]
fn wor58_user_images_reach_the_provider_in_active_mode() {
    let image_only = vec![user_with_image(None, "UE5HLWltYWdlLW9ubHk=")];
    let mixed = vec![user_with_image(
        Some("what is wrong in this screenshot?"),
        "UE5HLW1peGVkLWltYWdl",
    )];
    for (messages, data) in [
        (image_only, "UE5HLWltYWdlLW9ubHk="),
        (mixed, "UE5HLW1peGVkLWltYWdl"),
    ] {
        let off = wire_for(ContextVmMode::Off, messages.clone());
        let active = wire_for(ContextVmMode::Active, messages);
        assert_eq!(off.matches(data).count(), 1, "{off}");
        assert_eq!(active.matches(data).count(), 1, "{active}");
    }
    let active = wire_for(
        ContextVmMode::Active,
        vec![user_with_image(
            Some("what is wrong in this screenshot?"),
            "UE5HLW1peGVkLWltYWdl",
        )],
    );
    assert!(active.contains("what is wrong in this screenshot?"));
}

#[test]
fn wor58_a_different_image_changes_the_event_identity() {
    use davinci_agent::runtime::context_vm::events_from_messages;
    let a = events_from_messages(&[user_with_image(Some("compare"), "QUFBQQ==")]);
    let b = events_from_messages(&[user_with_image(Some("compare"), "QkJCQg==")]);
    let text = events_from_messages(&[ChatMessage::text("user", "compare")]);
    assert_ne!(a[0].content_hash, b[0].content_hash);
    assert_ne!(a[0].source_ref, b[0].source_ref);
    assert_ne!(a[0].content_hash, text[0].content_hash);
    // Text-only identities are unchanged, so persisted roots stay valid.
    assert_eq!(
        text[0].content_hash,
        davinci_agent::runtime::cache::digest(b"compare")
    );
}

#[test]
fn wor58_blocked_images_stay_blocked_in_active_mode() {
    let mut agent = Agent::new("system");
    agent.set_context_vm_mode(ContextVmMode::Active);
    agent.set_runtime(runtime());
    agent.block_images = true;
    agent.messages = vec![user_with_image(Some("look"), "UE5HLWJsb2NrZWQ=")];
    let wire = serde_json::to_string(&agent.messages_for_provider()).unwrap();
    assert!(!wire.contains("UE5HLWJsb2NrZWQ="), "{wire}");
    assert!(wire.contains("look"));
}

#[test]
fn wor58_session_images_survive_replay_into_the_image() {
    let directory = tempfile::tempdir().unwrap();
    let mut session =
        davinci_session::JsonlSession::create(directory.path(), "fixture", None).unwrap();
    let message = user_with_image(None, "UE5HLXNlc3Npb24=");
    session
        .append_entry(davinci_session::SessionEntry {
            id: "event-1".into(),
            entry_type: "message".into(),
            parent_id: None,
            seq: 1,
            timestamp: 0,
            message: Some(serde_json::to_value(&message).unwrap()),
            custom_type: None,
            extra: Default::default(),
        })
        .unwrap();
    let path = session.path.clone();
    drop(session);
    let mut agent = active_agent_on(&path);
    agent.messages = vec![message];
    let wire = serde_json::to_string(&agent.messages_for_provider()).unwrap();
    assert_eq!(wire.matches("UE5HLXNlc3Npb24=").count(), 1, "{wire}");
}

/// The per-round delta and the compile must agree on the event list, or
/// every tool round reads as a diverged history and rebuilds the VM.
#[test]
fn wor59_tool_rounds_do_not_rebuild_the_vm() {
    let mut agent = sessionless_active_agent();
    agent.messages = vec![ChatMessage::text("user", "fix the build")];
    agent.prepared_context_image().unwrap();
    let vm = agent.runtime.as_ref().unwrap().context_vm.clone();
    let baseline = vm.metrics().rebuilds;
    for round in 0..3 {
        agent.messages.push(ChatMessage {
            role: "assistant".into(),
            content: vec![davinci_ai::MessageContent::ToolCall {
                id: format!("call-{round}"),
                name: "read".into(),
                arguments: serde_json::json!({"path": format!("f{round}")}),
            }],
            ..Default::default()
        });
        agent.messages.push(ChatMessage::tool_result(
            format!("call-{round}"),
            "read",
            format!("contents {round}"),
            false,
        ));
        agent.invalidate_context_image();
        vm.append_delta(&agent.context_vm_events_for_test())
            .unwrap();
        agent.prepared_context_image().unwrap();
    }
    assert_eq!(
        vm.metrics().rebuilds,
        baseline,
        "a tool round rebuilt the VM"
    );
}

/// A fold made while a tool exchange is live must survive the compile that
/// follows it, and match what the session persisted.
#[test]
fn wor59_fold_with_a_live_tool_exchange_is_kept() {
    let directory = tempfile::tempdir().unwrap();
    let mut session =
        davinci_session::JsonlSession::create(directory.path(), "fixture", None).unwrap();
    let messages = tool_exchange("live output marker-5524");
    for (index, message) in messages.iter().enumerate() {
        let seq = index as u64 + 1;
        session
            .append_entry(davinci_session::SessionEntry {
                id: format!("event-{seq}"),
                entry_type: "message".into(),
                parent_id: session.leaf_id.clone(),
                seq,
                timestamp: 0,
                message: Some(serde_json::to_value(message).unwrap()),
                custom_type: None,
                extra: Default::default(),
            })
            .unwrap();
    }
    let mut agent = Agent::new("system");
    agent.set_context_vm_mode(ContextVmMode::Active);
    agent.session = Some(session);
    agent.messages = messages;
    agent.set_runtime(runtime());
    let folded = agent.fold_context(FoldReason::Manual, None).unwrap();
    agent.prepared_context_image().unwrap();
    let vm = &agent.runtime.as_ref().unwrap().context_vm;
    assert_eq!(vm.root().epoch, folded.epoch);
    assert_eq!(vm.root().checkpoint, folded.checkpoint);
    assert_eq!(vm.root().episodes, folded.episodes);
    let (persisted, _) = davinci_agent::runtime::context_vm::latest_persisted_root(
        &agent.session.as_ref().unwrap().entries,
        agent.session.as_ref().unwrap().leaf_id.as_deref(),
    )
    .unwrap();
    assert_eq!(persisted.checkpoint, folded.checkpoint);
}

/// The living plan (priority 500) outranks recent turns; a repository file
/// (priority 100) does not.
#[test]
fn wor72_the_living_plan_outranks_recent_turns() {
    use davinci_agent::runtime::context_vm::{
        events_from_messages, ContextVmConfig, ContextVmRuntime,
    };
    use davinci_agent::{ContextItem, ContextPacket};
    let vm = ContextVmRuntime::new(ContextVmConfig::default(), Default::default());
    let messages = (0..10)
        .map(|n| {
            let role = if n % 2 == 0 { "user" } else { "assistant" };
            ChatMessage::text(role, format!("turn-{n} {}", "detail ".repeat(40)))
        })
        .collect::<Vec<_>>();
    let events = events_from_messages(&messages);
    let item = |source: &str, priority: i32| ContextItem {
        source: source.into(),
        content: format!("{source} {}", "step ".repeat(300)),
        estimated_tokens: 400,
        priority,
        stable_for_cache: true,
        provenance: serde_json::json!({"provenance_kind": "user_decision"}),
    };
    let broker = ContextPacket {
        items: vec![
            item("file::README.md", 100),
            item("agent::living_plan", 500),
        ],
        estimated_tokens: 800,
        cache_key: "fixture".into(),
    };
    let everything = vm.compile(&events, &broker, 1_000_000).unwrap();
    let tokens = |category: &str| -> u64 {
        everything
            .entries
            .iter()
            .filter(|entry| entry.category == category)
            .map(|entry| entry.estimated_tokens)
            .sum()
    };
    // Room for every turn and the plan, but not the README too.
    let budget = everything.estimated_tokens - tokens("broker_context") / 4;
    let image = vm.compile(&events, &broker, budget).unwrap();
    let sources = image
        .entries
        .iter()
        .filter(|entry| entry.category == "broker_context")
        .map(|entry| entry.source_ref.as_str())
        .collect::<Vec<_>>();
    assert_eq!(sources, ["agent::living_plan"]);
    // When turns must give way, they give way to the plan.
    let tight = everything.estimated_tokens - tokens("broker_context") / 2 - tokens("hot_user") / 2;
    let image = vm.compile(&events, &broker, tight).unwrap();
    assert!(image
        .entries
        .iter()
        .any(|entry| entry.source_ref == "agent::living_plan"));
}

/// The hot window is sized in tokens; an image costs as much there as it
/// does in the compiled image, not one token.
#[test]
fn wor58_images_count_toward_the_hot_window() {
    use davinci_agent::runtime::context_vm::{
        events_from_messages, ContextVmConfig, ContextVmRuntime,
    };
    let vm = ContextVmRuntime::new(ContextVmConfig::default(), Default::default());
    let messages = (0..12)
        .map(|n| user_with_image(None, &format!("SU1BR0Ut{n:02}")))
        .collect::<Vec<_>>();
    let events = events_from_messages(&messages);
    vm.compile(&events, &davinci_agent::ContextPacket::empty(), 1_000_000)
        .unwrap();
    let hot = vm.root().hot_event_refs.len();
    assert!(
        hot < events.len(),
        "{hot} image events fit a 20k-token window"
    );
    assert!(hot >= 1);
}

/// An image-only last turn does not make the broker's goal an empty string.
#[test]
fn wor58_image_only_turn_keeps_a_textual_goal() {
    let mut agent = sessionless_active_agent();
    agent.messages = vec![
        ChatMessage::text("user", "match this mockup"),
        user_with_image(None, "UE5HLWdvYWw="),
    ];
    let state_before = agent.prepared_context_image().unwrap();
    let goals = agent
        .runtime
        .as_ref()
        .unwrap()
        .context_vm
        .load_state_from_root()
        .unwrap()
        .goals;
    assert!(goals.iter().all(|goal| !goal.value.trim().is_empty()));
    assert!(serde_json::to_string(&state_before.messages)
        .unwrap()
        .contains("UE5HLWdvYWw="));
}

/// WOR-81: a retrieval query filtered lines case-sensitively, so "error"
/// missed "Error:" and "ERROR".
#[test]
fn wor81_retrieval_query_matches_case_insensitively() {
    use davinci_agent::runtime::context_vm::{
        events_from_messages, ContextVmConfig, ContextVmRuntime, RetrieveContextRequest,
    };
    let vm = ContextVmRuntime::new(ContextVmConfig::default(), Default::default());
    let events = events_from_messages(&[ChatMessage::text(
        "user",
        "Error: build failed\nall good\nERROR in linker\nan error here",
    )]);
    vm.compile(&events, &davinci_agent::ContextPacket::empty(), 1_000_000)
        .unwrap();
    let result = vm
        .retrieve(&RetrieveContextRequest {
            source_ref: Some(events[0].source_ref.clone()),
            query: Some("error".into()),
            ..Default::default()
        })
        .unwrap();
    assert_eq!(
        result.content,
        "Error: build failed\nERROR in linker\nan error here"
    );
}

/// The same property through the real run loop: tool rounds driven by
/// `run_loop` (which records a delta each round) never rebuild the VM.
#[test]
fn wor59_run_loop_tool_rounds_do_not_rebuild_the_vm() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("a.txt"), "alpha").unwrap();
    let mut agent = sessionless_active_agent();
    agent.cwd = root.path().to_path_buf();
    agent.set_permission_mode(davinci_agent::PermissionMode::AlwaysApprove);
    agent.prompt_user_with("read a.txt three times", &[]);
    let mut requests = 0;
    agent
        .run_loop(|_| {
            requests += 1;
            let message = if requests <= 3 {
                serde_json::json!({
                    "id": format!("r{requests}"), "role": "assistant", "model": "fixture",
                    "stopReason": "toolUse",
                    "content": [{"type": "toolCall", "id": format!("call-{requests}"),
                        "name": "read", "arguments": {"path": "a.txt"}}]
                })
            } else {
                serde_json::json!({
                    "id": "done", "role": "assistant", "model": "fixture",
                    "stopReason": "stop", "content": [{"type": "text", "text": "done"}]
                })
            };
            Ok(serde_json::from_value::<davinci_ai::AssistantMessage>(message).unwrap())
        })
        .unwrap();
    assert_eq!(requests, 4);
    let vm = &agent.runtime.as_ref().unwrap().context_vm;
    // One rebuild creates the first checkpoint; none after that.
    assert_eq!(vm.metrics().rebuilds, 1, "{:?}", vm.metrics());
}
