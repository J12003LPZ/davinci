use davinci_agent::runtime::context_vm::ContextVmMode;
use davinci_agent::{Agent, AgentId, ContextFile, RunId, RuntimeBus, RuntimeHandle};
use davinci_ai::ChatMessage;
use std::path::PathBuf;

#[test]
fn shadow_mode_compiles_without_changing_the_legacy_provider_projection() {
    let mut agent = Agent::new("system authority");
    agent.set_runtime(RuntimeHandle::new(
        RunId::new(),
        AgentId::new(),
        RuntimeBus::new(),
    ));
    agent.messages = vec![
        ChatMessage::text("user", "visible request"),
        ChatMessage::text("assistant", "visible answer"),
    ];
    agent.context_files = vec![ContextFile {
        path: PathBuf::from("AGENTS.md"),
        name: "AGENTS.md".into(),
        body: "mandatory repository authority".into(),
    }];
    agent.set_ephemeral_context(vec![ChatMessage::text(
        "custom",
        "optional ephemeral evidence",
    )]);
    let legacy = agent.legacy_messages_for_provider_for_test();

    agent.set_context_vm_mode(ContextVmMode::Shadow);
    assert_eq!(agent.messages_for_provider(), legacy);
    assert!(
        agent
            .runtime
            .as_ref()
            .unwrap()
            .context_vm
            .metrics()
            .images_compiled
            > 0
    );
    let metrics = agent.runtime.as_ref().unwrap().context_vm.metrics();
    assert_eq!(metrics.shadow_missing_user_refs, 0);
    assert_eq!(metrics.shadow_missing_tool_refs, 0);
    assert_eq!(agent.context_vm_cache_affinity(), None);
}
