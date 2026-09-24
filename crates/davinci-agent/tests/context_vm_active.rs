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
    assert_eq!(
        agent
            .runtime
            .as_ref()
            .unwrap()
            .context_vm
            .metrics()
            .images_compiled,
        4
    );
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
                "previous":"use old API","evidence":{"value":"user corrected API",
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
        assert!(vm.root().deltas.is_empty());
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
