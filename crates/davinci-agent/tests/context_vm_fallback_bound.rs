use davinci_agent::runtime::context_vm::{events_from_messages, ContextStateReducer};
use davinci_ai::ChatMessage;

#[test]
fn wor40_deterministic_fallback_stays_bounded_over_a_long_run() {
    let mut messages = Vec::new();
    for turn in 0..300 {
        messages.push(ChatMessage::text(
            "user",
            format!("request {turn}: {}", "detail ".repeat(60)),
        ));
        messages.push(ChatMessage::tool_result(
            format!("call-{turn}"),
            "bash",
            format!("test run {turn} passed"),
            false,
        ));
    }
    let events = events_from_messages(&messages);

    // Fold in small batches, as automatic folds do across a long session.
    let mut state = Default::default();
    for batch in events.chunks(10) {
        state = ContextStateReducer::deterministic_delta(&state, batch).checkpoint_patch;
    }

    assert!(
        state.goals.len() <= 16,
        "{} goals retained",
        state.goals.len()
    );
    assert!(
        state.verification.len() <= 8,
        "{} verification entries retained",
        state.verification.len()
    );
    assert!(state.goals.first().unwrap().value.starts_with("request 0:"));
    assert!(state
        .goals
        .last()
        .unwrap()
        .value
        .starts_with("request 299:"));
    assert_eq!(
        state.verification.last().unwrap().value,
        "test run 299 passed"
    );
    let bytes = serde_json::to_vec(&state).unwrap().len();
    assert!(
        bytes < 64 * 1024,
        "fallback checkpoint grew to {bytes} bytes"
    );
}
