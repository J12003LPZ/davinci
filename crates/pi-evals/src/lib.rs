//! Fixture-driven eval harness matching `@earendil-works/pi-evals`.

use pi_agent::{run_agent, AgentConfig, AgentEvent, AgentMessage, ToolRegistry};
use pi_ai::stream::FixtureResponse;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalCase {
    pub name: String,
    pub prompt: String,
    pub fixture: FixtureResponse,
    #[serde(default)]
    pub expect_contains: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvalResult {
    pub name: String,
    pub passed: bool,
    pub events: Vec<AgentEvent>,
    pub artifacts: Vec<String>,
}

pub fn run_eval(case: &EvalCase) -> EvalResult {
    let config = AgentConfig {
        cwd: PathBuf::from("."),
        system_prompt: "eval".into(),
        model_provider: "faux".into(),
        model_id: "fixture".into(),
        api_key: None,
        allow_network: false,
        auto_retry: false,
        max_retries: 0,
        auto_compact: false,
        context_window: 128_000,
        max_turns: 8,
        fixture: Some(case.fixture.clone()),
        permission: Box::new(pi_agent::AllowAllPermissionPolicy),
        transport: None,
        session_id: None,
    };
    let events = run_agent(
        &config,
        &[AgentMessage {
            role: "user".into(),
            content: case.prompt.clone(),
            images: vec![],
        }],
        &ToolRegistry::builtins(),
        &mut pi_agent::SteerQueue::default(),
        &mut pi_agent::FollowUpQueue::default(),
    )
    .unwrap_or_else(|e| {
        vec![AgentEvent::Error {
            message: e.to_string(),
        }]
    });
    let blob = serde_json::to_string(&events).unwrap_or_default();
    let passed = case
        .expect_contains
        .iter()
        .all(|needle| blob.contains(needle) || case.prompt.contains(needle));
    EvalResult {
        name: case.name.clone(),
        passed,
        artifacts: vec![
            format!("events:{}", events.len()),
            format!("blob:{}", blob.len()),
        ],
        events,
    }
}

pub fn run_suite(cases: &[EvalCase]) -> Vec<EvalResult> {
    cases.iter().map(run_eval).collect()
}

pub fn summarize(results: &[EvalResult]) -> serde_json::Value {
    let passed = results.iter().filter(|r| r.passed).count();
    serde_json::json!({
        "total": results.len(),
        "passed": passed,
        "failed": results.len() - passed
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use pi_ai::events::{AssistantMessage, AssistantMessageEvent, StopReason};

    #[test]
    fn fixture_eval_passes() {
        let case = EvalCase {
            name: "smoke".into(),
            prompt: "hi".into(),
            fixture: FixtureResponse {
                events: vec![AssistantMessageEvent::Done {
                    reason: StopReason::Stop,
                    message: AssistantMessage {
                        id: "1".into(),
                        role: "assistant".into(),
                        content: vec![pi_ai::ContentBlock::Text {
                            text: "hello".into(),
                        }],
                        model: None,
                        stop_reason: Some(StopReason::Stop),
                        usage: None,
                        error_message: None,
                        timestamp: 1,
                    },
                }],
                sse: None,
            },
            expect_contains: vec!["hello".into()],
        };
        let result = run_eval(&case);
        assert!(result.passed);
        let suite = run_suite(&[case]);
        let summary = summarize(&suite);
        assert_eq!(summary["passed"], 1);
    }
}
