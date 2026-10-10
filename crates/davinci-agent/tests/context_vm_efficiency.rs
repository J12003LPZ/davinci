//! Offline, structural comparison of Context VM modes over one scripted
//! multi-turn coding session. Every request goes through the real run loop and
//! `messages_for_provider()`, and is serialized with the OpenAI Responses
//! input projection.
//!
//! What it measures is local: the longest common byte prefix between
//! consecutive request inputs (an upper bound on what a provider prefix cache
//! could reuse, not a cache hit), fold and summarizer calls, summarizer prompt
//! bytes, and whether each user requirement is still in the request after it
//! was stated. System prompt and tool schemas are identical across modes and
//! excluded. No provider is called.
use davinci_agent::runtime::ContextVmMode;
use davinci_agent::{
    Agent, AgentId, RunId, RuntimeBus, RuntimeHandle, SummarizeResponse, Summarizer,
};
use davinci_ai::{AssistantMessage, ChatMessage, ContentBlock, MessageContent, StopReason};
use std::sync::{Arc, Mutex};

const WINDOW: u64 = 128_000;
const TURNS: usize = 60;

const REQUIREMENTS: [(usize, &str); 3] = [
    (0, "Never modify files under migrations/."),
    (
        5,
        "All public functions must keep their current signatures.",
    ),
    (12, "Correction: use tokio, not async-std, for the runtime."),
];

fn user_text(turn: usize) -> String {
    match turn {
        0 => format!(
            "Build the job scheduler crate. {} Start with the queue module.",
            REQUIREMENTS[0].1
        ),
        // A requirement buried in the middle of a long paste.
        5 => format!(
            "Here is the design note from the team:\n{}\n{}\n{}",
            "The scheduler keeps jobs in a priority queue. ".repeat(60),
            REQUIREMENTS[1].1,
            "Workers poll the queue and report progress. ".repeat(60)
        ),
        12 => REQUIREMENTS[2].1.to_string(),
        _ => format!("Continue with step {turn}: implement module_{turn} and run its tests."),
    }
}

fn tool_output(turn: usize) -> String {
    let verdict = if turn % 3 == 0 {
        "test result: FAILED. 1 passed; 1 failed"
    } else {
        "test result: ok. 2 passed; 0 failed"
    };
    format!(
        "src/module_{turn}.rs\n{}\n{verdict}",
        format!("pub fn handler_{turn}(job: &Job) -> Result<(), Error> {{ todo!() }}\n").repeat(48)
    )
}

#[derive(Default)]
struct Calls {
    folds: usize,
    fold_prompt_bytes: usize,
}

fn reply(text: &str) -> AssistantMessage {
    AssistantMessage {
        extra: Default::default(),
        id: "fixture".into(),
        role: "assistant".into(),
        model: "fixture".into(),
        usage: None,
        error_message: None,
        content: vec![ContentBlock::Text { text: text.into() }],
        stop_reason: Some(StopReason::Stop),
    }
}

fn common_prefix(a: &[u8], b: &[u8]) -> usize {
    a.iter().zip(b).take_while(|(x, y)| x == y).count()
}

fn run(mode: ContextVmMode, summarizer: bool) -> serde_json::Value {
    let mut agent = Agent::new("You are a coding agent.");
    agent.set_runtime(RuntimeHandle::new(
        RunId::new(),
        AgentId::new(),
        RuntimeBus::new(),
    ));
    agent.set_context_vm_mode(mode);
    agent.context_window = WINDOW;
    agent.auto_compaction = true;
    let calls = Arc::new(Mutex::new(Calls::default()));
    if summarizer {
        let calls = calls.clone();
        agent.summarizer = Some(Summarizer::new(move |request| {
            let mut calls = calls.lock().unwrap();
            calls.folds += 1;
            calls.fold_prompt_bytes += request.prompt.len() + request.system.len();
            // A valid proposal that adds nothing: retention then rests on the
            // deterministic state, as it does when the model omits a fact.
            let text = if request.label == "context state fold" {
                "{}".to_string()
            } else {
                "## Goal\nContinue the scheduler.".to_string()
            };
            Ok(SummarizeResponse {
                text,
                usage: Default::default(),
                stop_reason: Some(StopReason::Stop),
                error_message: None,
                has_tool_call: false,
            })
        }));
    }
    let mut previous: Option<Vec<u8>> = None;
    let (mut total_bytes, mut reused_bytes) = (0usize, 0usize);
    let mut requirement_checks = 0usize;
    let mut requirement_hits = 0usize;
    let mut lost = Vec::new();
    for turn in 0..TURNS {
        agent
            .messages
            .push(ChatMessage::text("user", user_text(turn)));
        agent.messages.push(ChatMessage {
            role: "assistant".into(),
            content: vec![MessageContent::ToolCall {
                id: format!("call-{turn}"),
                name: "bash".into(),
                arguments: serde_json::json!({"command": format!("cargo test module_{turn}")}),
            }],
            ..ChatMessage::default()
        });
        agent.messages.push(ChatMessage::tool_result(
            format!("call-{turn}"),
            "bash",
            tool_output(turn),
            false,
        ));
        let wire = Arc::new(Mutex::new(Vec::new()));
        let captured = wire.clone();
        agent
            .run_loop(move |agent: &Agent| {
                let input = davinci_ai::openai_responses_input(&agent.messages_for_provider());
                *captured.lock().unwrap() = serde_json::to_vec(&input).unwrap();
                Ok::<_, String>(reply(&format!("Step {turn} recorded.")))
            })
            .expect("turn runs");
        let wire = wire.lock().unwrap().clone();
        total_bytes += wire.len();
        if let Some(previous) = &previous {
            let lcp = common_prefix(previous, &wire);
            reused_bytes += lcp;
            if std::env::var("TRACE_LCP").is_ok() {
                eprintln!(
                    "{mode:?} {summarizer} turn {turn} bytes {} new {}",
                    wire.len(),
                    wire.len() - lcp
                );
            }
        }
        let text = String::from_utf8_lossy(&wire);
        for (stated, requirement) in REQUIREMENTS {
            if turn >= stated {
                requirement_checks += 1;
                if text.contains(requirement) {
                    requirement_hits += 1;
                } else {
                    lost.push(serde_json::json!({"turn": turn, "stated": stated}));
                }
            }
        }
        previous = Some(wire);
    }
    let calls = calls.lock().unwrap();
    let stats = agent.run_stats();
    serde_json::json!({
        "mode": format!("{mode:?}"),
        "summarizer": summarizer,
        "requests": stats.model_turns,
        "compactions_or_folds": stats.compactions,
        "summarizer_calls": calls.folds,
        "summarizer_prompt_bytes": calls.fold_prompt_bytes,
        "request_input_bytes": total_bytes,
        "prefix_reusable_bytes": reused_bytes,
        "non_prefix_bytes": total_bytes - reused_bytes,
        "prefix_reuse_ratio": reused_bytes as f64 / total_bytes as f64,
        "requirement_retention": format!("{requirement_hits}/{requirement_checks}"),
        "first_losses": lost.into_iter().take(6).collect::<Vec<_>>(),
    })
}

#[test]
#[ignore = "offline structural measurement; run with --ignored --nocapture"]
fn context_vm_mode_efficiency_report() {
    for (mode, summarizer) in [
        (ContextVmMode::Off, true),
        (ContextVmMode::Active, true),
        (ContextVmMode::Active, false),
    ] {
        println!("CONTEXT_VM_EFFICIENCY {}", run(mode, summarizer));
    }
}
