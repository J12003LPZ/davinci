//! B3: an outcome the harness could not establish must stay "unknown" through
//! compaction. The summarizer is shown the unknown result as an error, with
//! its wording intact, and no checkpoint field records it as completed work.

use davinci_agent::runtime::cache::CacheRuntime;
use davinci_agent::runtime::context_vm::{ContextVmConfig, ContextVmRuntime, FoldReason};
use davinci_agent::runtime::ContextVmMode;
use davinci_agent::{
    Agent, AgentId, RunId, RuntimeBus, RuntimeHandle, SummarizeResponse, Summarizer,
};
use davinci_ai::{ChatMessage, MessageContent, StopReason};
use serde_json::json;
use std::sync::{Arc, Mutex};

const UNKNOWN: &str = "Outcome unknown: the session ended after this call started, so it may or may not have taken effect. It was not re-run.";

fn unknown_result(id: &str) -> ChatMessage {
    ChatMessage {
        role: "toolResult".into(),
        content: vec![MessageContent::Text {
            text: UNKNOWN.into(),
        }],
        tool_call_id: Some(id.into()),
        tool_name: Some("bash".into()),
        is_error: Some(true),
        extra: [(
            "details".to_string(),
            json!({"toolRecovery": {"outcome": "unknown", "source": "operation_journal"}}),
        )]
        .into_iter()
        .collect(),
    }
}

#[test]
fn unknown_effect_is_not_summarized_as_success() {
    let prompts = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen = prompts.clone();
    let mut agent = Agent::new("You are a coding agent.");
    agent.compaction.keep_recent_tokens = 1;
    agent.summarizer = Some(Summarizer::new(move |request| {
        seen.lock().unwrap().push(request.prompt.clone());
        Ok(SummarizeResponse {
            text: "The deploy command outcome is unknown.".into(),
            usage: Default::default(),
            stop_reason: Some(StopReason::Stop),
            error_message: None,
            has_tool_call: false,
        })
    }));
    agent.messages.push(ChatMessage::text(
        "user",
        format!("Run the deploy script. {}", "x ".repeat(300)),
    ));
    agent.messages.push(ChatMessage {
        role: "assistant".into(),
        content: vec![MessageContent::ToolCall {
            id: "deploy_call".into(),
            name: "bash".into(),
            arguments: json!({"command": "./deploy.sh"}),
        }],
        ..ChatMessage::default()
    });
    agent.messages.push(unknown_result("deploy_call"));
    for turn in 0..4 {
        agent.messages.push(ChatMessage::text(
            "user",
            format!("next {turn} {}", "y ".repeat(300)),
        ));
        agent.messages.push(ChatMessage::text(
            "assistant",
            format!("ack {turn} {}", "z ".repeat(300)),
        ));
    }

    assert!(agent.compact(None).compacted);
    let prompts = prompts.lock().unwrap();
    let request = prompts.first().expect("the summarizer was asked");
    assert!(
        request.contains("Outcome unknown"),
        "the unknown wording must reach the summarizer: {request}"
    );
    assert!(
        !request.to_lowercase().contains("deploy succeeded"),
        "{request}"
    );
}

/// The active Context VM folds from its own events; the fold request must
/// carry the unknown result's wording too, and the checkpoint a fold produces
/// from it must not list the call as completed work.
#[test]
fn unknown_effect_reaches_the_active_context_vm_fold_unchanged() {
    let prompts = Arc::new(Mutex::new(Vec::<String>::new()));
    let seen = prompts.clone();
    let mut agent = Agent::new("You are a coding agent.");
    let mut runtime = RuntimeHandle::new(RunId::new(), AgentId::new(), RuntimeBus::new());
    // A small hot window evicts the exchange into the fold request.
    runtime.context_vm = ContextVmRuntime::new(
        ContextVmConfig {
            hot_event_tokens: 200,
            ..ContextVmConfig::default()
        },
        CacheRuntime::default(),
    );
    agent.set_runtime(runtime);
    agent.set_context_vm_mode(ContextVmMode::Active);
    agent.summarizer = Some(Summarizer::new(move |request| {
        seen.lock().unwrap().push(request.prompt.clone());
        Ok(SummarizeResponse {
            text: "{}".into(),
            usage: Default::default(),
            stop_reason: Some(StopReason::Stop),
            error_message: None,
            has_tool_call: false,
        })
    }));
    agent
        .messages
        .push(ChatMessage::text("user", "Run the deploy script."));
    agent.messages.push(ChatMessage {
        role: "assistant".into(),
        content: vec![MessageContent::ToolCall {
            id: "deploy_call".into(),
            name: "bash".into(),
            arguments: json!({"command": "./deploy.sh"}),
        }],
        ..ChatMessage::default()
    });
    agent.messages.push(unknown_result("deploy_call"));
    for turn in 0..6 {
        agent.messages.push(ChatMessage::text(
            "user",
            format!("next {turn} {}", "y ".repeat(200)),
        ));
        agent.messages.push(ChatMessage::text(
            "assistant",
            format!("ack {turn} {}", "z ".repeat(200)),
        ));
    }
    agent
        .fold_context(FoldReason::Manual, None)
        .expect("the fold runs");

    let prompts = prompts.lock().unwrap();
    let request = prompts.first().expect("the summarizer was asked");
    assert!(request.contains("Outcome unknown"), "{request}");
    let state = agent
        .runtime
        .as_ref()
        .unwrap()
        .context_vm
        .load_state_from_root()
        .expect("the checkpoint the fold wrote can be read back");
    // The property below is about a checkpoint that exists. An empty one (a
    // fold that wrote nothing, or a load that fell back to the default) would
    // satisfy it for the wrong reason.
    assert!(
        state.through_seq > 0,
        "the fold must have produced a checkpoint covering the evicted events: {state:?}"
    );
    assert!(
        !format!("{:?}", state.completed).contains("deploy"),
        "an unknown outcome is not completed work: {:?}",
        state.completed
    );
}
