//! The configured auto-compaction threshold (`compaction.threshold`, e.g.
//! `"50%"`) is what the agent loop checks before every model request. Every
//! host — the main session, `--print` graph workers, in-process subagents and
//! workflow workers — runs this same loop, so this is the contract they share.

use davinci_agent::{
    Agent, CompactionSettings, CompactionThreshold, SummarizeResponse, Summarizer,
};
use davinci_ai::{AssistantMessage, ChatMessage, ContentBlock, StopReason};

const WINDOW: u64 = 100_000;

/// About 60% of a 100k window: 28 exchanges of ~2k tokens each, plus the
/// system prompt and tool schemas.
fn agent_with_history(threshold: Option<CompactionThreshold>) -> Agent {
    let mut agent = Agent::new("You are a coding agent.");
    agent.context_window = WINDOW;
    agent.auto_compaction = true;
    agent.compaction = CompactionSettings {
        threshold,
        ..CompactionSettings::default()
    };
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
    agent.prompt("continue");
    // Every real host attaches a model summarizer; without one compaction
    // falls back to a transcript dump, which keeps plain text nearly verbatim.
    agent.summarizer = Some(Summarizer::new(|_| {
        Ok(SummarizeResponse {
            text: "## Goal\nContinue the requests.".into(),
            usage: Default::default(),
            stop_reason: Some(StopReason::Stop),
            error_message: None,
            has_tool_call: false,
        })
    }));
    agent
}

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

#[test]
fn a_percent_threshold_compacts_once_the_context_passes_it() {
    let mut agent = agent_with_history(Some(CompactionThreshold::Percent(50)));
    let before = agent.estimated_context_tokens();
    assert!(
        before > WINDOW / 2 && before < WINDOW - davinci_agent::DEFAULT_RESERVE_TOKENS,
        "fixture must sit between 50% and the default trigger, was {before}"
    );

    agent.run_loop(reply).expect("turn runs");

    assert_eq!(agent.run_stats().compactions, 1);
    let after = agent.estimated_context_tokens();
    assert!(
        after < WINDOW / 2,
        "compaction must bring the context back under the threshold: {before} -> {after}"
    );
}

#[test]
fn without_a_threshold_the_same_context_is_left_alone() {
    let mut agent = agent_with_history(None);
    agent.run_loop(reply).expect("turn runs");
    assert_eq!(agent.run_stats().compactions, 0);
}

#[test]
fn a_token_threshold_is_honoured_the_same_way() {
    let mut agent = agent_with_history(Some(CompactionThreshold::Tokens(50_000)));
    agent.run_loop(reply).expect("turn runs");
    assert_eq!(agent.run_stats().compactions, 1);
}

#[test]
fn disabling_auto_compaction_wins_over_the_threshold() {
    let mut agent = agent_with_history(Some(CompactionThreshold::Percent(50)));
    agent.auto_compaction = false;
    agent.run_loop(reply).expect("turn runs");
    assert_eq!(agent.run_stats().compactions, 0);
}
