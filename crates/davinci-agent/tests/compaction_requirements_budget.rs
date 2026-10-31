//! Compaction carries the user's requests verbatim. How much user text the
//! history holds must not decide whether `/compact` works: the replacement is
//! admitted against the context that is actually left, and refused explicitly
//! only when the carried requests themselves cannot fit.

use davinci_agent::{Agent, CompactionSettings, SummarizeResponse, Summarizer};
use davinci_ai::{ChatMessage, StopReason};

/// Fourteen exchanges of ~2k tokens each; the user halves alone total about
/// 110 KB, far over the 24 KiB the carried block used to be limited to.
fn agent_with_history(context_window: u64, settings: CompactionSettings) -> Agent {
    let mut agent = Agent::new("You are a coding agent.");
    agent.context_window = context_window;
    agent.compaction = settings;
    let filler = "word ".repeat(1_600);
    for index in 0..14 {
        agent.messages.push(ChatMessage::text(
            "user",
            format!("request {index}: {filler}"),
        ));
        agent.messages.push(ChatMessage::text(
            "assistant",
            format!("reply {index}: {filler}"),
        ));
    }
    agent.summarizer = Some(Summarizer::new(|_| {
        Ok(SummarizeResponse {
            text: "## Goal\nContinue the requests.".into(),
            usage: Some(Default::default()),
            stop_reason: Some(StopReason::Stop),
            error_message: None,
            has_tool_call: false,
        })
    }));
    agent
}

#[test]
fn manual_compact_works_when_earlier_user_messages_total_over_24_kib() {
    let mut agent = agent_with_history(100_000, CompactionSettings::default());
    let before = agent.estimated_context_tokens();

    let result = agent.compact(None);

    assert!(result.compacted, "{}", result.summary);
    assert!(agent.estimated_context_tokens() < before);
    let carried = davinci_agent::compaction_summary_meta(&agent.messages[0])
        .expect("generated summary metadata")
        .carried_user_requirements;
    assert!(
        carried.iter().map(String::len).sum::<usize>() > 24 * 1024,
        "the carried requests are not truncated"
    );

    // And again: carried requests accumulate through metadata, and the next
    // compact must not be the one that finally refuses.
    let filler = "more ".repeat(1_600);
    for index in 0..6 {
        agent.messages.push(ChatMessage::text(
            "user",
            format!("follow-up {index}: {filler}"),
        ));
        agent.messages.push(ChatMessage::text(
            "assistant",
            format!("done {index}: {filler}"),
        ));
    }
    let again = agent.compact(None);
    assert!(again.compacted, "{}", again.summary);
}

#[test]
fn compact_refuses_explicitly_when_the_carried_requests_cannot_fit() {
    // Nearly everything is replaced (tiny recent tail), so about 28k tokens of
    // user text must be carried into a context with roughly 24k left.
    let settings = CompactionSettings {
        keep_recent_tokens: 1_000,
        reserve_tokens: 1_000,
        ..CompactionSettings::default()
    };
    let mut agent = agent_with_history(25_000, settings);
    let before = agent.messages.clone();

    let result = agent.compact(None);

    assert!(!result.compacted);
    assert!(
        result.summary.contains("context has left"),
        "refusal must say why: {}",
        result.summary
    );
    assert_eq!(agent.messages, before, "the history is unchanged");
}
