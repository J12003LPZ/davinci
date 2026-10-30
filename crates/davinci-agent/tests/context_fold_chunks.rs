//! A fold sends the unfolded history to the summarizer oldest first, in
//! chunks that each fit its window, and fails closed when it cannot.

use davinci_agent::runtime::context_vm::{ContextVmMode, FoldReason};
use davinci_agent::{
    Agent, AgentId, RunId, RuntimeBus, RuntimeHandle, SummarizeResponse, Summarizer,
};
use davinci_ai::{ChatMessage, StopReason};
use std::sync::{Arc, Mutex};

fn agent(window: u64, messages: usize) -> Agent {
    let mut agent = Agent::new("You are a coding agent.");
    agent.set_runtime(RuntimeHandle::new(
        RunId::new(),
        AgentId::new(),
        RuntimeBus::new(),
    ));
    agent.set_context_vm_mode(ContextVmMode::Hybrid);
    agent.context_window = window;
    for n in 0..messages {
        agent.messages.push(ChatMessage::text(
            "user",
            format!("CONSTRAINT-{n} {}", "pad ".repeat(60)),
        ));
    }
    agent
}

fn recording(prompts: Arc<Mutex<Vec<String>>>) -> Summarizer {
    Summarizer::new(move |request| {
        prompts.lock().unwrap().push(request.prompt.clone());
        Ok(SummarizeResponse {
            text: "{}".into(),
            usage: Some(Default::default()),
            stop_reason: Some(StopReason::Stop),
            error_message: None,
            has_tool_call: false,
        })
    })
}

#[test]
fn every_event_reaches_the_summarizer_oldest_first_and_each_request_fits() {
    let window = 16_000;
    let mut agent = agent(window, 80);
    let prompts = Arc::new(Mutex::new(Vec::new()));
    agent.summarizer = Some(recording(prompts.clone()));

    let root = agent
        .fold_context(FoldReason::Manual, None)
        .expect("the fold completes");

    let prompts = prompts.lock().unwrap();
    assert!(prompts.len() > 1, "80 events do not fit one request");
    for prompt in prompts.iter() {
        assert!(prompt.len() as u64 <= window, "{} bytes", prompt.len());
    }
    let joined = prompts.join("\n");
    for n in 0..80 {
        assert!(
            joined.contains(&format!("CONSTRAINT-{n} ")),
            "event {n} lost"
        );
    }
    // Oldest first: the first request holds the first event, not the last.
    assert!(prompts[0].contains("CONSTRAINT-0 "));
    assert!(!prompts[0].contains("CONSTRAINT-79 "));
    assert!(prompts.last().unwrap().contains("CONSTRAINT-79 "));
    assert_eq!(
        root.folded_through_seq,
        agent.context_vm_events_for_test().last().unwrap().seq
    );
}

#[test]
fn a_fold_that_needs_too_many_requests_fails_closed_and_keeps_the_chunks_done() {
    let mut agent = agent(16_000, 600);
    let before = agent.messages.clone();
    let prompts = Arc::new(Mutex::new(Vec::new()));
    agent.summarizer = Some(recording(prompts.clone()));

    let error = agent
        .fold_context(FoldReason::Manual, None)
        .expect_err("600 events cannot be folded in the request cap");

    assert!(error.starts_with("CONTEXT_ADMISSION_FAILED"), "{error}");
    assert_eq!(prompts.lock().unwrap().len(), 12, "the cap is 12 requests");
    assert_eq!(agent.messages, before, "history is untouched");
    let root = agent.runtime.as_ref().unwrap().context_vm.root();
    assert!(
        root.folded_through_seq > 0,
        "the chunks already folded stay committed"
    );
}

#[test]
fn a_state_that_alone_fills_the_window_sends_nothing_and_keeps_the_deterministic_state() {
    let mut agent = agent(900, 3);
    let prompts = Arc::new(Mutex::new(Vec::new()));
    agent.summarizer = Some(recording(prompts.clone()));

    let root = agent
        .fold_context(FoldReason::Manual, None)
        .expect("the deterministic state folds without a request");

    assert!(prompts.lock().unwrap().is_empty(), "nothing was sent");
    assert!(root.folded_through_seq > 0);
}
