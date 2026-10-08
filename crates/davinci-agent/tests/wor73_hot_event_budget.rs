//! WOR-73: the hot-event token estimate must cover the exact provider message,
//! wrapper and escaping included, and no wrapper may be applied twice.
use davinci_agent::provider_budget::message_token_ceiling;
use davinci_agent::runtime::cache::CacheRuntime;
use davinci_agent::runtime::context_vm::{events_from_messages, ContextVmConfig, ContextVmRuntime};
use davinci_agent::ContextPacket;
use davinci_ai::{content_text, ChatMessage};

#[test]
fn wor73_hot_entry_estimate_covers_the_wire_message_for_every_event_kind() {
    let escapes =
        "quote \" backslash \\ newline \n tab \t nul \u{0} emoji \u{1F600} <tag> ".repeat(40);
    let runtime = ContextVmRuntime::new(ContextVmConfig::default(), CacheRuntime::default());
    let events = events_from_messages(&[
        ChatMessage::text("user", format!("user {escapes}")),
        ChatMessage::text("assistant", format!("assistant {escapes}")),
        ChatMessage::tool_result("call-1", "read", &format!("tool {escapes}"), false),
        ChatMessage::text("custom", format!("custom {escapes}")),
        ChatMessage::text("user", "latest"),
    ]);
    let image = runtime
        .compile(&events, &ContextPacket::empty(), 1_000_000)
        .unwrap();

    let hot: Vec<_> = image
        .entries
        .iter()
        .filter(|entry| entry.category.starts_with("hot_"))
        .collect();
    assert!(hot.len() >= 4, "expected every kind in the hot window");
    let wire = &image.messages[image.messages.len() - hot.len()..];
    let mut kinds = Vec::new();
    for (entry, message) in hot.iter().zip(wire) {
        let ceiling = message_token_ceiling(message);
        assert!(
            entry.estimated_tokens >= ceiling,
            "{}: estimate {} below wire ceiling {ceiling}",
            entry.category,
            entry.estimated_tokens
        );
        let text = content_text(&message.content);
        assert!(
            text.matches("<context_data").count() <= 1,
            "{}: wrapper applied twice",
            entry.category
        );
        kinds.push(entry.category.as_str());
    }
    assert!(kinds.contains(&"hot_tool_result"), "{kinds:?}");
}
