use davinci_agent::runtime::context_vm::ContextVmMode;
use davinci_agent::{Agent, AgentId, RunId, RuntimeBus, RuntimeHandle};
use davinci_ai::{ChatMessage, MessageContent};

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
