use davinci_agent::runtime::cache::CacheRuntime;
use davinci_agent::runtime::context_vm::{
    events_from_messages, ContextVmConfig, ContextVmRuntime, MAX_ROOT_EVIDENCE_REFS,
};
use davinci_agent::ContextPacket;
use davinci_ai::ChatMessage;

#[test]
fn wor63_root_evidence_refs_stay_bounded_in_tool_heavy_sessions() {
    let runtime = ContextVmRuntime::new(ContextVmConfig::default(), CacheRuntime::default());
    let mut messages = vec![ChatMessage::text("user", "run many tools")];
    for index in 0..2_000 {
        messages.push(ChatMessage::tool_result(
            &format!("call-{index}"),
            "read",
            &format!("output {index}"),
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
    // Newest refs are the ones kept, in order.
    let newest = events
        .iter()
        .rev()
        .filter(|event| event.source_ref.contains("call-"))
        .map(|event| event.source_ref.clone())
        .next();
    if let Some(newest) = newest {
        assert_eq!(root.evidence_refs.last(), Some(&newest));
    }
    let bytes = serde_json::to_vec(&root.evidence_refs).unwrap().len();
    assert!(
        bytes < 4 * 1024,
        "evidence refs serialized to {bytes} bytes"
    );
}
