//! What makes the active Context VM fold, what a fold request carries, and
//! where the changing part of the image goes.
use davinci_agent::runtime::cache::CacheRuntime;
use davinci_agent::runtime::context_vm::{
    events_from_messages, ContextVmConfig, ContextVmRuntime, FoldReason,
};
use davinci_agent::runtime::ContextVmMode;
use davinci_agent::{
    Agent, AgentId, ContextPacket, RunId, RuntimeBus, RuntimeHandle, SummarizeResponse, Summarizer,
};
use davinci_ai::{AssistantMessage, ChatMessage, ContentBlock, StopReason};
use std::sync::{Arc, Mutex};

fn active_agent(window: u64, hot_event_tokens: u64) -> Agent {
    let mut agent = Agent::new("You are a coding agent.");
    let mut runtime = RuntimeHandle::new(RunId::new(), AgentId::new(), RuntimeBus::new());
    runtime.context_vm = ContextVmRuntime::new(
        ContextVmConfig {
            hot_event_tokens,
            ..ContextVmConfig::default()
        },
        CacheRuntime::default(),
    );
    agent.set_runtime(runtime);
    agent.set_context_vm_mode(ContextVmMode::Active);
    agent.context_window = window;
    agent.auto_compaction = true;
    agent
}

/// Records every summarizer prompt and proposes `proposal`.
fn summarizer(prompts: Arc<Mutex<Vec<String>>>, proposal: String) -> Summarizer {
    Summarizer::new(move |request| {
        prompts.lock().unwrap().push(request.prompt.clone());
        Ok(SummarizeResponse {
            text: proposal.clone(),
            usage: Default::default(),
            stop_reason: Some(StopReason::Stop),
            error_message: None,
            has_tool_call: false,
        })
    })
}

fn reply(_: &Agent) -> Result<AssistantMessage, String> {
    Ok(AssistantMessage {
        extra: Default::default(),
        id: "fixture".into(),
        role: "assistant".into(),
        model: "fixture".into(),
        usage: None,
        error_message: None,
        content: vec![ContentBlock::Text {
            text: "done".into(),
        }],
        stop_reason: Some(StopReason::Stop),
    })
}

/// A turn whose tool output carries no verification marker, so the
/// deterministic state does not change.
fn quiet_turn(agent: &mut Agent, turn: usize) {
    agent
        .messages
        .push(ChatMessage::text("user", format!("step {turn}")));
    agent.messages.push(ChatMessage::tool_result(
        format!("call-{turn}"),
        "read",
        format!(
            "src/module_{turn}.rs\n{}",
            "let value = compute();\n".repeat(130)
        ),
        false,
    ));
    agent
        .messages
        .push(ChatMessage::text("assistant", format!("read {turn}")));
}

#[test]
fn window_pressure_is_measured_on_the_threshold_scale_not_the_admission_ceiling() {
    let mut agent = active_agent(100_000, 20_000);
    agent.compaction.threshold = Some(davinci_agent::CompactionThreshold::Percent(30));
    let prompts = Arc::new(Mutex::new(Vec::new()));
    agent.summarizer = Some(summarizer(prompts.clone(), "{}".into()));
    for turn in 0..20 {
        quiet_turn(&mut agent, turn);
    }
    agent.messages.push(ChatMessage::text("user", "continue"));
    // The admission ceiling counts about a token per byte, so it is past
    // the 30% threshold although the request is far below it.
    let image = agent.prepared_context_image().unwrap();
    let budget = agent.provider_context_budget();
    let ceiling = image.estimated_tokens + budget.system + budget.tools;
    assert!(
        ceiling >= 30_000,
        "fixture must exceed the old trigger: {ceiling}"
    );
    // Both modes report the request on the same scale.
    let active = agent.estimated_context_tokens();
    agent.set_context_vm_mode(ContextVmMode::Off);
    let off = agent.estimated_context_tokens();
    agent.set_context_vm_mode(ContextVmMode::Active);
    assert!(
        active < 30_000 && active * 4 < off * 5 && off * 4 < active * 5,
        "{active} vs {off}"
    );

    agent.run_loop(reply).expect("turn runs");

    assert_eq!(agent.run_stats().compactions, 0);
    assert!(prompts.lock().unwrap().is_empty());
}

#[test]
fn an_over_budget_request_gets_a_fold_before_it_is_blocked() {
    let mut agent = active_agent(100_000, 20_000);
    let prompts = Arc::new(Mutex::new(Vec::new()));
    agent.summarizer = Some(summarizer(prompts.clone(), "{}".into()));
    agent
        .messages
        .push(ChatMessage::text("user", "x".repeat(200_000)));

    let error = agent.run_loop(reply).unwrap_err();

    assert!(error.contains("context compilation failed"), "{error}");
    assert_eq!(prompts.lock().unwrap().len(), 1, "no fold was attempted");
}

#[test]
fn maintenance_folds_need_evicted_events_and_a_summarizer() {
    for with_summarizer in [true, false] {
        let mut agent = active_agent(1_000_000, 2_000);
        let prompts = Arc::new(Mutex::new(Vec::new()));
        if with_summarizer {
            agent.summarizer = Some(summarizer(prompts.clone(), "{}".into()));
        }
        let (mut first_fold, mut first_eviction) = (None, None);
        for turn in 0..30 {
            agent
                .messages
                .push(ChatMessage::text("user", format!("implement step {turn}")));
            agent.messages.push(ChatMessage::tool_result(
                format!("call-{turn}"),
                "bash",
                format!("{}test result: ok", "compiling\n".repeat(80)),
                false,
            ));
            agent.run_loop(reply).expect("turn runs");
            let start = agent
                .runtime
                .as_ref()
                .unwrap()
                .context_vm
                .root()
                .hot_event_refs
                .first()
                .cloned();
            let oldest = agent.context_vm_events_for_test()[0].source_ref.clone();
            if first_eviction.is_none() && start.is_some_and(|start| start != oldest) {
                first_eviction = Some(turn);
            }
            if first_fold.is_none() && agent.run_stats().compactions > 0 {
                first_fold = Some(turn);
            }
        }
        let folds = agent.run_stats().compactions;
        if with_summarizer {
            // While every event is still in the window there is nothing for
            // a fold to add.
            let first_eviction = first_eviction.expect("the window advanced");
            assert!(first_eviction > 0);
            assert!(first_fold.is_some_and(|turn| turn >= first_eviction));
            // At most one fold per window advance, never one per turn.
            assert!((1..=10).contains(&folds), "{folds}");
            assert_eq!(prompts.lock().unwrap().len() as u64, folds);
        } else {
            assert_eq!(folds, 0, "a summarizer-less fold only rotates the cache");
        }
    }
}

#[test]
fn a_fold_request_carries_only_events_no_earlier_fold_saw() {
    let mut agent = active_agent(1_000_000, 20_000);
    agent
        .messages
        .push(ChatMessage::text("user", "Never touch migrations/."));
    let alpha = agent.context_vm_events_for_test()[0].source_ref.clone();
    let prompts = Arc::new(Mutex::new(Vec::new()));
    agent.summarizer = Some(summarizer(prompts.clone(), "{}".into()));
    agent.fold_context(FoldReason::Manual, None).unwrap();

    agent
        .messages
        .push(ChatMessage::text("user", "Add the retry queue."));
    let beta = agent.context_vm_events_for_test()[1].source_ref.clone();
    // The later proposal may still cite the folded event: validation sees
    // the whole history even though the summarizer did not.
    let proposal = serde_json::json!({"constraints":[{"value":"Never touch migrations/.",
        "source_refs":[alpha],"provenance_kind":"user_decision"}]})
    .to_string();
    agent.summarizer = Some(summarizer(prompts.clone(), proposal));
    agent.fold_context(FoldReason::Manual, None).unwrap();

    let prompts = prompts.lock().unwrap();
    assert!(prompts[0].contains(&format!("\"source_ref\":\"{alpha}\"")));
    assert!(!prompts[1].contains(&format!("\"source_ref\":\"{alpha}\"")));
    assert!(prompts[1].contains(&format!("\"source_ref\":\"{beta}\"")));
    let state = agent
        .runtime
        .as_ref()
        .unwrap()
        .context_vm
        .load_state_from_root()
        .unwrap();
    assert_eq!(state.constraints[0].value, "Never touch migrations/.");
}

#[test]
fn the_state_update_shows_only_what_the_model_cannot_see_and_comes_last() {
    let vm = ContextVmRuntime::new(
        ContextVmConfig {
            hot_event_tokens: 300,
            ..ContextVmConfig::default()
        },
        CacheRuntime::default(),
    );
    let compile = |messages: &[ChatMessage]| {
        let events = events_from_messages(messages);
        vm.append_delta(&events).unwrap();
        vm.compile(&events, &ContextPacket::empty(), 1_000_000)
            .unwrap()
    };
    let mut messages = vec![ChatMessage::text("user", "Never modify migrations/.")];
    compile(&messages);
    messages.push(ChatMessage::text("user", "Use tokio for the runtime."));
    let image = compile(&messages);
    // The first goal is in the checkpoint, the second in the recent events:
    // the delta adds nothing the model cannot already see.
    assert!(!vm.root().deltas.is_empty());
    assert!(image.entries.iter().all(|entry| entry.category != "delta"));

    for filler in 0..4 {
        messages.push(ChatMessage::text(
            "user",
            format!("note {filler} {}", "z".repeat(400)),
        ));
    }
    let image = compile(&messages);
    let position = |category: &str| {
        image
            .entries
            .iter()
            .rposition(|entry| entry.category == category)
    };
    let delta = position("delta").expect("evicted state is shown");
    let content = &image.entries[delta].content;
    assert!(content.contains("Use tokio for the runtime."), "{content}");
    assert!(!content.contains("Never modify migrations/."), "{content}");
    assert!(
        delta > position("hot_user").unwrap(),
        "delta must follow the hot events"
    );
    assert_eq!(image.messages.len(), image.entries.len());
}

#[test]
fn a_session_with_a_fold_reopens_and_restores_its_root() {
    let sessions = tempfile::tempdir().unwrap();
    let mut agent = active_agent(1_000_000, 20_000);
    agent.session =
        Some(davinci_session::JsonlSession::create(sessions.path(), "fold", None).unwrap());
    agent.prompt("Never touch migrations/.");
    let root = agent.fold_context(FoldReason::Manual, None).unwrap();
    let path = agent.session.as_ref().unwrap().path.clone();
    // Earlier builds wrote an entry type the session codec rejects here.
    let reopened = davinci_session::JsonlSession::open(&path).unwrap();
    let (persisted, _) = davinci_agent::runtime::context_vm::latest_persisted_root(
        &reopened.entries,
        reopened.leaf_id.as_deref(),
    )
    .expect("the fold record survives reopening");
    assert_eq!(persisted.checkpoint, root.checkpoint);
}
