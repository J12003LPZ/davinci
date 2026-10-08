use davinci_agent::runtime::cache::CacheRuntime;
use davinci_agent::runtime::context_vm::{
    events_from_messages, ContextEventKind, ContextVmConfig, ContextVmRuntime,
    MAX_ROOT_EVIDENCE_REFS,
};
use davinci_agent::ContextPacket;
use davinci_ai::ChatMessage;

#[test]
fn wor63_root_evidence_refs_stay_bounded_in_tool_heavy_sessions() {
    let runtime = ContextVmRuntime::new(ContextVmConfig::default(), CacheRuntime::default());
    let mut messages = vec![ChatMessage::text("user", "run many tools")];
    for index in 0..2_000 {
        messages.push(ChatMessage::tool_result(
            format!("call-{index}"),
            "read",
            format!("output {index}"),
            false,
        ));
    }
    messages.push(ChatMessage::text("user", "continue"));
    let events = events_from_messages(&messages);
    runtime
        .compile(&events, &ContextPacket::empty(), 10_000_000)
        .unwrap();

    let root = runtime.root();
    assert_eq!(root.evidence_refs.len(), MAX_ROOT_EVIDENCE_REFS);
    // The newest tool-result refs are the ones kept, oldest first.
    let tool_refs = events
        .iter()
        .filter(|event| event.kind == ContextEventKind::ToolResult)
        .map(|event| event.source_ref.clone())
        .collect::<Vec<_>>();
    assert_eq!(
        root.evidence_refs,
        tool_refs[tool_refs.len() - MAX_ROOT_EVIDENCE_REFS..]
    );
    let bytes = serde_json::to_vec(&root.evidence_refs).unwrap().len();
    assert!(
        bytes < 4 * 1024,
        "evidence refs serialized to {bytes} bytes"
    );
}
