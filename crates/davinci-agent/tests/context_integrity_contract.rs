//! Phase 3 contract: what the provider is sent, who said it, and what is lost.

use davinci_agent::runtime::context_vm::ContextVmMode;
use davinci_agent::{Agent, AgentId, RunId, RuntimeBus, RuntimeHandle};
use davinci_ai::{AssistantMessage, ChatMessage, ContentBlock, StopReason};

const SECRET: &str = "sk-live-0123456789abcdef";

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

fn agent_in(mode: ContextVmMode) -> Agent {
    let mut agent = Agent::new("system rules");
    agent.set_runtime(RuntimeHandle::new(
        RunId::new(),
        AgentId::new(),
        RuntimeBus::new(),
    ));
    agent.set_context_vm_mode(mode);
    agent.messages.push(ChatMessage::text(
        "user",
        format!("deploy with the key {SECRET} and keep the export format stable"),
    ));
    agent
}

#[test]
fn context_trace_reports_selected_origin_and_omissions() {
    // Active mode: the manifest is built for the real dispatch.
    let mut agent = agent_in(ContextVmMode::Active);
    assert!(agent.last_prepared_manifest.is_none());
    agent.run_loop(reply).expect("turn runs");
    let manifest = agent
        .last_prepared_manifest
        .clone()
        .expect("the dispatch left its manifest for /context inspect");
    assert_eq!(manifest.request_id, "req-1");
    let system = manifest
        .entries
        .iter()
        .find(|entry| entry.id == "system_prompt")
        .expect("the system prompt is a manifest entry");
    assert!(system.mandatory && system.selected);
    assert_eq!(system.source_ref, "agent::system_prompt");
    assert_eq!(
        system.provenance_kind,
        davinci_agent::runtime::ProvenanceKind::MandatoryPolicy
    );
    for entry in &manifest.entries {
        assert!(!entry.content_hash.is_empty(), "{} has no hash", entry.id);
        if entry.mandatory {
            assert!(entry.selected, "{} is mandatory yet omitted", entry.id);
        }
    }

    // An omission is reported with its reason. A derived-state failure is the
    // one way a dispatch leaves something out; seed one and read it back.
    agent
        .runtime
        .as_ref()
        .expect("the turn built a runtime")
        .context_vm
        .record_failure("compile", "injected derived-cache failure");
    let after = agent.context_manifest_now("req-omit");
    let omitted: Vec<_> = after
        .entries
        .iter()
        .filter(|entry| !entry.selected)
        .collect();
    assert!(!omitted.is_empty(), "the failure left no omitted entry");
    for entry in omitted {
        assert!(!entry.mandatory, "{} is mandatory yet omitted", entry.id);
        assert!(
            entry
                .selection_reason
                .as_deref()
                .is_some_and(|reason| reason.contains("injected derived-cache failure")),
            "{} was omitted without its reason: {:?}",
            entry.id,
            entry.selection_reason
        );
    }
    // Hashes and sizes, never the text: a secret in the history is not copied
    // into the trace.
    let rendered = serde_json::to_string(&manifest).unwrap();
    assert!(
        !rendered.contains(SECRET),
        "the trace holds raw source text"
    );
}

#[test]
fn other_modes_build_the_manifest_only_on_demand_and_leave_requests_alone() {
    for mode in [ContextVmMode::Off, ContextVmMode::Shadow] {
        let mut agent = agent_in(mode);
        agent.run_loop(reply).expect("turn runs");
        assert!(
            agent.last_prepared_manifest.is_none(),
            "{mode:?} paid for a manifest on every turn"
        );
        let before = agent.messages_for_provider();
        let manifest = agent.context_manifest_now("inspect");
        assert_eq!(manifest.request_id, "inspect");
        assert!(
            manifest
                .entries
                .iter()
                .any(|entry| entry.category == "history"),
            "{mode:?}: the inspector sees no history"
        );
        assert!(!serde_json::to_string(&manifest).unwrap().contains(SECRET));
        assert_eq!(
            agent.messages_for_provider(),
            before,
            "{mode:?}: building the manifest changed the provider request"
        );
    }
}
