//! Fixture-driven eval harness matching `@earendil-works/pi-evals`.

pub mod artifacts;
pub mod harness_table;
pub mod reporter;
pub mod summary;

pub use artifacts::{
    persist_eval_artifact_references, record_eval_session_artifact, record_eval_source_artifact,
    ArtifactReference, EvalArtifact, EvalAttachment, PI_SESSION_SNAPSHOT_ARTIFACT,
};
pub use harness_table::{
    derive_eval_group_key, eval_harness_table, parse_eval_harness_iteration_artifact,
    EvalHarnessIterationArtifact, EvalHarnessTableRow, EVAL_HARNESS_ITERATION_ARTIFACT,
};
pub use reporter::{
    append_harness_run_report, collect_harness_observations, format_test_run_end, is_harness_run,
    HarnessRun, HarnessTestCase, HarnessTestModule, HarnessTimings, HarnessUsage,
    EVAL_COMPARISONS_INTERRUPTED,
};
pub use summary::{
    format_harness_comparison_report, strip_vt_control_characters, summarize_harness_comparisons,
    HarnessComparisonReport, HarnessObservation,
};

use pi_agent::Agent;
use pi_ai::{
    content_text, AssistantMessage, ChatMessage, ContentBlock, MessageContent, StopReason,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalCase {
    pub name: String,
    pub prompts: Vec<String>,
    #[serde(default)]
    pub expected_contains: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalResult {
    pub name: String,
    pub passed: bool,
    pub output: String,
    pub artifacts: serde_json::Value,
}

#[derive(Debug, Clone)]
pub struct ModelSelection {
    pub provider: String,
    pub id: String,
}

pub fn resolve_model_selection(
    explicit: Option<ModelSelection>,
    provider_env: Option<&str>,
    model_env: Option<&str>,
) -> Result<ModelSelection, String> {
    let provider = explicit
        .as_ref()
        .map(|m| m.provider.clone())
        .or_else(|| provider_env.map(str::to_string))
        .filter(|s| !s.trim().is_empty());
    let id = explicit
        .as_ref()
        .map(|m| m.id.clone())
        .or_else(|| model_env.map(str::to_string))
        .filter(|s| !s.trim().is_empty());
    match (provider, id) {
        (Some(provider), Some(id)) => Ok(ModelSelection { provider, id }),
        _ => Err(
            "Select a harness model explicitly or set both PI_PROVIDER and PI_MODEL as defaults."
                .into(),
        ),
    }
}

pub fn run_fixture_eval(agent: &mut Agent, case: &EvalCase, canned: &[(&str, &str)]) -> EvalResult {
    let mut output = String::new();
    for prompt in &case.prompts {
        agent.prompt(prompt);
        let reply = canned
            .iter()
            .find(|(needle, _)| prompt.contains(needle))
            .map(|(_, reply)| (*reply).to_string())
            .unwrap_or_else(|| format!("ok: {prompt}"));
        agent.record_assistant(&reply);
        output.push_str(&reply);
        output.push('\n');
    }
    let passed = case
        .expected_contains
        .iter()
        .all(|needle| output.contains(needle));
    EvalResult {
        name: case.name.clone(),
        passed,
        output,
        artifacts: serde_json::json!({
            "messages": agent.messages.len(),
            "session": agent.session.as_ref().map(|s| s.header.id.clone()),
        }),
    }
}

pub fn transcript_from_messages(messages: &[ChatMessage]) -> Vec<serde_json::Value> {
    let mut events = Vec::new();
    for message in messages {
        if message.role == "user" {
            events.push(serde_json::json!({
                "type": "message",
                "role": "user",
                "content": content_text(&message.content),
            }));
        } else if message.role == "assistant" {
            let text = content_text(&message.content);
            if !text.is_empty() {
                events.push(serde_json::json!({
                    "type": "message",
                    "role": "assistant",
                    "content": text,
                }));
            }
            for part in &message.content {
                if let MessageContent::ToolCall {
                    id,
                    name,
                    arguments,
                } = part
                {
                    events.push(serde_json::json!({
                        "type": "tool_call",
                        "id": id,
                        "name": name,
                        "arguments": arguments,
                    }));
                }
            }
        } else if message.role == "toolResult" {
            let text = content_text(&message.content);
            let mut event = serde_json::json!({
                "type": "tool_result",
                "toolCallId": message.tool_call_id,
                "name": message.tool_name,
                "content": text,
            });
            if message.is_error.unwrap_or(false) {
                event["error"] = serde_json::json!({ "message": text });
            }
            events.push(event);
        }
    }
    events
}

#[derive(Debug, Clone)]
pub enum HarnessStep {
    Prompt(String),
    Reload,
}

#[derive(Debug, Clone)]
pub struct HarnessResult {
    pub output: String,
    pub events: Vec<serde_json::Value>,
}

pub fn eval_model_not_found(provider: &str, id: &str) -> String {
    format!("Eval model not found: {provider}/{id}")
}

/// TS `runPiCodingAgent` / `promptAgent` against an already-created session.
pub fn run_harness<F>(
    agent: &mut Agent,
    steps: &[HarnessStep],
    mut complete: F,
) -> Result<HarnessResult, String>
where
    F: FnMut(&Agent) -> Result<AssistantMessage, String>,
{
    if steps.is_empty()
        || !steps
            .iter()
            .any(|step| matches!(step, HarnessStep::Prompt(_)))
    {
        return Err("Pi eval input must include at least one prompt step.".into());
    }
    let mut response = None;
    for step in steps {
        match step {
            HarnessStep::Reload => {}
            HarnessStep::Prompt(text) => {
                let previous = agent.messages.len();
                agent.prompt(text);
                agent.run_loop(&mut complete)?;
                let assistant = agent
                    .messages
                    .iter()
                    .skip(previous)
                    .rev()
                    .find(|message| message.role == "assistant")
                    .ok_or_else(|| {
                        "Agent run completed without an assistant message.".to_string()
                    })?;
                let _ = assistant;
                let output = agent
                    .last_assistant_text()
                    .ok_or_else(|| "Agent run produced no assistant text.".to_string())?;
                response = Some(output);
            }
        }
    }
    let output = response
        .ok_or_else(|| "Pi eval input must include at least one prompt step.".to_string())?;
    Ok(HarnessResult {
        output,
        events: transcript_from_messages(&agent.messages),
    })
}

pub fn fixture_complete(text: &str) -> AssistantMessage {
    AssistantMessage {
        id: pi_agent::new_message_id(),
        role: "assistant".into(),
        content: vec![ContentBlock::Text {
            text: text.to_string(),
        }],
        model: "eval".into(),
        usage: None,
        stop_reason: Some(StopReason::Stop),
        error_message: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixture_eval_and_model_selection_errors() {
        let error = resolve_model_selection(None, None, None).unwrap_err();
        assert_eq!(
            error,
            "Select a harness model explicitly or set both PI_PROVIDER and PI_MODEL as defaults."
        );
        let mut agent = Agent::new("test");
        let result = run_fixture_eval(
            &mut agent,
            &EvalCase {
                name: "smoke".into(),
                prompts: vec!["hello".into()],
                expected_contains: vec!["ok: hello".into()],
            },
            &[],
        );
        assert!(result.passed);
        assert_eq!(transcript_from_messages(&agent.messages).len(), 2);

        let missing =
            run_harness(&mut Agent::new("x"), &[], |_| Ok(fixture_complete("x"))).unwrap_err();
        assert_eq!(
            missing,
            "Pi eval input must include at least one prompt step."
        );
        assert_eq!(
            eval_model_not_found("google", "missing"),
            "Eval model not found: google/missing"
        );
        let mut agent = Agent::new("eval");
        let harness = run_harness(&mut agent, &[HarnessStep::Prompt("hi".into())], |_| {
            Ok(fixture_complete("done"))
        })
        .unwrap();
        assert_eq!(harness.output, "done");
        assert!(harness
            .events
            .iter()
            .any(|event| event["type"] == "message" && event["role"] == "assistant"));
    }
}
