//! Hybrid Context VM: the provider gets the off-mode transcript, append-only;
//! the VM keeps the task ledger and supplies it when the transcript compacts.
use davinci_agent::runtime::ContextVmMode;
use davinci_agent::{
    Agent, AgentId, CompactionSettings, RunId, RuntimeBus, RuntimeHandle, SummarizeResponse,
    Summarizer,
};
use davinci_ai::{content_text, AssistantMessage, ChatMessage, ContentBlock, StopReason};
use davinci_session::JsonlSession;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

const NEVER: &str = "Never modify files under migrations/.";
const CORRECTION: &str = "Correction: use tokio, not async-std, for the runtime.";

fn agent(mode: ContextVmMode) -> Agent {
    let mut agent = Agent::new("You are a coding agent.");
    agent.set_runtime(RuntimeHandle::new(
        RunId::new(),
        AgentId::new(),
        RuntimeBus::new(),
    ));
    agent.set_context_vm_mode(mode);
    agent.auto_compaction = true;
    agent
}

/// A summarizer that proposes nothing: what survives compaction is what the
/// deterministic ledger kept on its own.
fn lazy_summarizer(calls: Arc<AtomicUsize>) -> Summarizer {
    Summarizer::new(move |request| {
        calls.fetch_add(1, Ordering::SeqCst);
        Ok(SummarizeResponse {
            text: if request.label == "context state fold" {
                "{}".into()
            } else {
                "## Goal\nContinue.".into()
            },
            usage: Default::default(),
            stop_reason: Some(StopReason::Stop),
            error_message: None,
            has_tool_call: false,
        })
    })
}

/// A reply that reports `input` provider tokens for its request.
fn reply(input: u64) -> AssistantMessage {
    AssistantMessage {
        extra: Default::default(),
        id: "fixture".into(),
        role: "assistant".into(),
        model: "fixture".into(),
        usage: Some(davinci_protocol::Usage {
            input,
            output: 4,
            ..Default::default()
        }),
        error_message: None,
        content: vec![ContentBlock::Text {
            text: "done".into(),
        }],
        stop_reason: Some(StopReason::Stop),
    }
}

/// The request's own size, at four bytes a token, as the provider count.
fn measured(agent: &Agent) -> u64 {
    serde_json::to_vec(&davinci_ai::openai_responses_input(
        &agent.messages_for_provider(),
    ))
    .unwrap()
    .len() as u64
        / 4
}

fn provider_text(agent: &Agent) -> String {
    agent
        .messages_for_provider()
        .iter()
        .map(|message| content_text(&message.content))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Drive 60 turns in a 12k window: enough for several compactions. Each
/// turn checks that both requirements are in the request once stated.
fn long_session(agent: &mut Agent) {
    agent.context_window = 12_000;
    agent.compaction = CompactionSettings {
        reserve_tokens: 2_000,
        keep_recent_tokens: 1_000,
        ..CompactionSettings::default()
    };
    for turn in 0..60 {
        let text = match turn {
            0 => format!("Build the scheduler. {NEVER}"),
            7 => CORRECTION.to_string(),
            _ => format!("Continue with step {turn}. {}", "notes ".repeat(300)),
        };
        agent.prompt(&text);
        agent
            .run_loop(|agent: &Agent| Ok::<_, String>(reply(measured(agent))))
            .expect("turn runs");
        let text = provider_text(agent);
        assert!(text.contains(NEVER), "turn {turn} lost the constraint");
        if turn >= 7 {
            assert!(text.contains(CORRECTION), "turn {turn} lost the correction");
        }
    }
}

#[test]
fn hybrid_sends_exactly_the_off_mode_transcript_until_it_compacts() {
    let mut off = agent(ContextVmMode::Off);
    let mut hybrid = agent(ContextVmMode::Hybrid);
    for agent in [&mut off, &mut hybrid] {
        agent.prompt(NEVER);
        agent.record_assistant("I will leave migrations alone.");
        agent.messages.push(ChatMessage::tool_result(
            "call-1",
            "bash",
            "test result: ok",
            false,
        ));
        agent.prompt("Continue.");
    }
    assert_eq!(off.messages_for_provider(), hybrid.messages_for_provider());
    // No compiled image, so no Context VM cache partition either.
    assert_eq!(hybrid.context_vm_cache_affinity(), None);
}

#[test]
fn requirements_survive_repeated_compaction_without_a_session() {
    let mut agent = agent(ContextVmMode::Hybrid);
    let calls = Arc::new(AtomicUsize::new(0));
    agent.summarizer = Some(lazy_summarizer(calls.clone()));
    long_session(&mut agent);
    let compactions = agent.run_stats().compactions;
    assert!(compactions >= 2, "{compactions}");
    // At most one fold request per compaction, never one per turn. In this
    // small window the fold prompt may not fit; the deterministic ledger
    // then carries the requirements alone.
    assert!(calls.load(Ordering::SeqCst) as u64 <= compactions);
    let summary = content_text(&agent.messages[0].content);
    assert!(summary.contains("Task ledger"), "{summary}");
    // The ledger rides in a user message but is never taken for one.
    let state = agent
        .runtime
        .as_ref()
        .unwrap()
        .context_vm
        .load_state_from_root()
        .unwrap();
    assert!(state
        .goals
        .iter()
        .all(|goal| !goal.value.contains("Task ledger")));
}

#[test]
fn requirements_survive_repeated_compaction_and_reload_from_the_session() {
    let sessions = tempfile::tempdir().unwrap();
    let mut agent = agent(ContextVmMode::Hybrid);
    agent.session = Some(JsonlSession::create(sessions.path(), "hybrid", None).unwrap());
    agent.summarizer = Some(lazy_summarizer(Arc::default()));
    long_session(&mut agent);
    assert!(agent.run_stats().compactions >= 2);

    let path = agent.session.as_ref().unwrap().path.clone();
    let mut reloaded = self::agent(ContextVmMode::Hybrid);
    reloaded
        .load_from_session(JsonlSession::open(&path).unwrap())
        .unwrap();
    reloaded.set_context_vm_mode(ContextVmMode::Hybrid);
    let text = provider_text(&reloaded);
    assert!(text.contains(NEVER) && text.contains(CORRECTION));
    // What the provider receives is the same; reloaded messages carry extra
    // session metadata that the wire projection drops.
    assert_eq!(
        davinci_ai::openai_responses_input(&reloaded.messages_for_provider()),
        davinci_ai::openai_responses_input(&agent.messages_for_provider())
    );
}

#[test]
fn the_provider_count_triggers_compaction_where_the_estimate_would_not() {
    let mut agent = agent(ContextVmMode::Hybrid);
    agent.context_window = 100_000;
    agent.prompt("Build the scheduler.");
    // The provider counted far more than four bytes a token suggests.
    agent
        .run_loop(|_: &Agent| Ok::<_, String>(reply(95_000)))
        .unwrap();
    assert!(agent.estimated_context_tokens() < 10_000);
    assert!(agent.calibrated_context_tokens() > 95_000);
    assert_eq!(agent.run_stats().compactions, 0);

    agent.prompt("Continue.");
    agent
        .run_loop(|_: &Agent| Ok::<_, String>(reply(1_000)))
        .unwrap();
    assert_eq!(agent.run_stats().compactions, 1);
    // The count belonged to the history compaction replaced.
    assert!(agent.calibrated_context_tokens() < 95_000);
}

#[test]
fn a_context_overflow_refusal_compacts_once_and_retries() {
    for always_refuse in [false, true] {
        let mut agent = agent(ContextVmMode::Hybrid);
        for turn in 0..6 {
            agent.prompt(&format!("step {turn}"));
            agent.record_assistant("ok");
        }
        agent.prompt("Continue.");
        let requests = Arc::new(Mutex::new(0));
        let counter = requests.clone();
        let result = agent.run_loop(move |_: &Agent| {
            let mut requests = counter.lock().unwrap();
            *requests += 1;
            if always_refuse || *requests == 1 {
                Err("context_length_exceeded: input exceeds the context window".to_string())
            } else {
                Ok(reply(100))
            }
        });
        assert_eq!(
            *requests.lock().unwrap(),
            2,
            "refuse always: {always_refuse}"
        );
        assert_eq!(agent.run_stats().compactions, 1);
        assert_eq!(result.is_err(), always_refuse);
    }
}
