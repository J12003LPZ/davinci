//! Without a summarizer, compaction falls back to a local summary. That fallback must
//! compress: a bounded share of the history it replaces, never a copy of the transcript.

use davinci_agent::compact_messages_with_options;
use davinci_agent::estimate_context_tokens;
use davinci_ai::ChatMessage;

fn long_history() -> Vec<ChatMessage> {
    let output = "tool output line with exact path src/lib.rs\n".repeat(400);
    let mut messages = vec![ChatMessage::text(
        "user",
        "Original request: fix the parser",
    )];
    for n in 0..12 {
        messages.push(ChatMessage::text(
            "assistant",
            format!("step {n}\n{output}"),
        ));
        messages.push(ChatMessage::text("user", format!("continue {n}")));
    }
    messages
}

#[test]
fn local_fallback_summary_is_a_bounded_share_of_the_history() {
    let messages = long_history();
    let result = compact_messages_with_options(&messages, None, 2_000, 16_384, None, None);
    assert!(result.compacted);
    let summary_tokens = result.summary.len() as u64 / 4;
    let before = estimate_context_tokens(&messages);
    assert!(
        summary_tokens <= before / 3,
        "summary {summary_tokens} tokens vs history {before}"
    );
    assert!(result.tokens_after < result.tokens_before);
    assert!(result.summary.contains("Original request: fix the parser"));
    assert!(result.summary.contains("omitted by local compaction"));
}

#[test]
fn local_fallback_respects_the_reserve_budget_and_utf8() {
    let messages: Vec<ChatMessage> = (0..40)
        .map(|n| ChatMessage::text("user", format!("é漢字 {n} ").repeat(500)))
        .collect();
    let result = compact_messages_with_options(&messages, None, 500, 256, None, None);
    assert!(result.compacted);
    assert!(
        result.summary.len() <= 256 * 4 + 512,
        "{}",
        result.summary.len()
    );
}

#[test]
fn short_history_is_kept_whole_by_the_local_fallback() {
    let messages = vec![
        ChatMessage::text("user", "hello"),
        ChatMessage::text("assistant", "hi"),
        ChatMessage::text("user", "next"),
    ];
    let result = compact_messages_with_options(&messages, None, 1, 16_384, None, None);
    assert!(!result.summary.contains("omitted by local compaction"));
}
