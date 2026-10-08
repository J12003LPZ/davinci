use davinci_agent::runtime::cache::{CacheConfig, CacheRuntime};
use davinci_agent::runtime::context_vm::{events_from_messages, ContextVmConfig, ContextVmRuntime};
use davinci_agent::ContextPacket;
use davinci_ai::ChatMessage;

#[test]
fn wor38_routine_append_delta_keeps_the_cached_checkpoint_prefix() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = ContextVmRuntime::new(
        ContextVmConfig::default(),
        CacheRuntime::new(CacheConfig::default(), Some(directory.path().to_path_buf())),
    );
    let mut messages = vec![ChatMessage::text("user", "keep the public API stable")];
    let first = runtime
        .compile(
            &events_from_messages(&messages),
            &ContextPacket::empty(),
            20_000,
        )
        .unwrap();
    let checkpoint = first.root.checkpoint.clone().unwrap();
    let opening = first.messages[0].clone();
    let mut digests = vec![first.prefix_digest.clone()];

    for turn in 0..4 {
        messages.push(ChatMessage::text("user", format!("now do step {turn}")));
        messages.push(ChatMessage::tool_result(
            format!("call-{turn}"),
            "bash",
            format!("test step {turn} passed"),
            false,
        ));
        let events = events_from_messages(&messages);
        let root = runtime.append_delta(&events).unwrap();
        assert_eq!(root.checkpoint.as_ref(), Some(&checkpoint), "turn {turn}");
        let image = runtime
            .compile(&events, &ContextPacket::empty(), 20_000)
            .unwrap();
        assert_eq!(image.messages[0], opening, "turn {turn} rewrote the prefix");
        assert!(
            image
                .entries
                .iter()
                .any(|entry| entry.content.contains(&format!("now do step {turn}"))),
            "turn {turn} lost the new state"
        );
        digests.push(image.prefix_digest.clone());
    }
    assert!(
        digests.windows(2).all(|pair| pair[0] == pair[1]),
        "stable prefix digest churned on routine turns: {digests:?}"
    );
    assert_eq!(runtime.metrics().prefix_churn, 0);
}
