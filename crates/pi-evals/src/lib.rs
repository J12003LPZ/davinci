//! Fixture-driven eval harness matching `@earendil-works/pi-evals`.

use pi_agent::Agent;
use pi_ai::{content_text, ChatMessage};
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
    messages
        .iter()
        .map(|message| {
            serde_json::json!({
                "type": "message",
                "role": message.role,
                "content": content_text(&message.content),
            })
        })
        .collect()
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
    }
}
