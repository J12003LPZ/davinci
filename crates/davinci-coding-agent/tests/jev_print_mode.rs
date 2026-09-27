use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use davinci_agent::decision::provider::{DecisionError, DecisionProvider};
use davinci_agent::decision::request::{DecisionQuestionType, DecisionRequest};
use davinci_agent::decision::response::{parse_and_validate_response, DecisionResponse};
use davinci_agent::decision::risk::DecisionRisk;
use davinci_agent::decision::DecisionRuntime;
use davinci_agent::effort::EffortPolicy;
use davinci_agent::Agent;
use davinci_coding_agent::decision_state::DecisionMetadata;
use davinci_coding_agent::turn_decision::{
    prepare_turn_decision, DecisionAdmission, DecisionSnapshot,
};
use davinci_protocol::ThinkingLevel;
use serde_json::{json, Map, Value};

struct FixtureProvider {
    calls: AtomicUsize,
}

impl DecisionProvider for FixtureProvider {
    fn name(&self) -> &'static str {
        "fixture"
    }

    fn model(&self) -> &'static str {
        "fixture-model"
    }

    fn evaluate(
        &self,
        request: &DecisionRequest,
        _budget: Duration,
    ) -> Result<DecisionResponse, DecisionError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let completion = request.state.get("completionEvidence").is_some();
        assert!(request
            .questions
            .keys()
            .all(|id| id.starts_with("requirement_") == completion));
        assert_eq!(request.state.get("requirements").is_some(), completion);
        let mut answers = Map::new();
        for (id, question) in &request.questions {
            let answer = match question.question_type {
                DecisionQuestionType::Noul => json!({"type": "noul", "noul": 0.9}),
                DecisionQuestionType::Choice => {
                    let choices: Vec<_> = question.choice_ids().map(str::to_owned).collect();
                    let probability = 1.0 / choices.len() as f32;
                    let probabilities = choices
                        .iter()
                        .map(|choice| (choice.clone(), json!(probability)))
                        .collect::<Map<String, Value>>();
                    json!({
                        "type": "choice",
                        "choice": choices[0],
                        "probabilities": probabilities,
                        "confidence": 0.9
                    })
                }
                DecisionQuestionType::Score => {
                    let levels = question.score_levels();
                    let probability = 1.0 / levels as f32;
                    let probabilities = (0..levels)
                        .map(|level| (level.to_string(), json!(probability)))
                        .collect::<Map<String, Value>>();
                    let score = (id == "regression_risk").then_some(levels - 1).unwrap_or(0);
                    json!({
                        "type": "score",
                        "score": score,
                        "probabilities": probabilities,
                        "confidence": 0.9
                    })
                }
            };
            answers.insert(id.clone(), answer);
        }
        let raw = json!({"answers": answers}).to_string();
        parse_and_validate_response(raw.as_bytes(), request)
    }
}

fn snapshot(id: &str, evidence_revision: u64) -> DecisionSnapshot {
    DecisionSnapshot {
        request_id: id.to_owned(),
        task: "Fix the parser and verify the public API".to_owned(),
        decision_class: DecisionRisk::Planning,
        metadata: DecisionMetadata::default(),
        evidence_revision,
        mutation_revision: 0,
    }
}

#[test]
fn missing_runtime_is_a_disabled_no_wait_path() {
    let mut agent = Agent::new("test system");
    let started = Instant::now();
    assert_eq!(
        prepare_turn_decision(&mut agent, snapshot("disabled", 1)),
        DecisionAdmission::Disabled
    );
    assert!(started.elapsed() < Duration::from_millis(50));
}

fn enabled_agent() -> (Agent, Arc<FixtureProvider>, Arc<DecisionRuntime>) {
    let provider = Arc::new(FixtureProvider {
        calls: AtomicUsize::new(0),
    });
    let runtime = Arc::new(DecisionRuntime::new(provider.clone()));
    runtime.enable();
    let mut agent = Agent::new("test system");
    agent.set_decision_runtime(runtime.clone());
    agent.set_decision_effort_advice_enabled(true);
    agent.thinking_level = ThinkingLevel::Medium;
    agent.effort_policy = EffortPolicy::Adaptive;
    (agent, provider, runtime)
}

fn wait_for_success(runtime: &DecisionRuntime, count: u64) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while runtime.telemetry().snapshot().successes < count && Instant::now() < deadline {
        std::thread::yield_now();
    }
    assert!(runtime.telemetry().snapshot().successes >= count);
}

fn poll_until_ready(agent: &mut Agent, runtime: &DecisionRuntime) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while runtime.telemetry().snapshot().ready_advice == 0 && Instant::now() < deadline {
        agent.poll_decision_advice(true);
        std::thread::yield_now();
    }
    assert_eq!(runtime.telemetry().snapshot().ready_advice, 1);
}

#[test]
fn ready_same_turn_advice_never_changes_request_one() {
    let (mut agent, _, runtime) = enabled_agent();
    agent.prompt("Fix the parser");
    assert_eq!(
        prepare_turn_decision(&mut agent, snapshot("first", 1)),
        DecisionAdmission::Enqueued
    );
    wait_for_success(&runtime, 1);
    agent.poll_decision_advice(false);
    assert_eq!(agent.request_thinking_level(), ThinkingLevel::Low);
    poll_until_ready(&mut agent, &runtime);
    assert_eq!(agent.request_thinking_level(), ThinkingLevel::High);
}

#[test]
fn previous_user_turn_advice_cannot_change_the_next_submissions_first_request() {
    let (mut agent, _, runtime) = enabled_agent();
    agent.prompt("First task");
    assert_eq!(
        prepare_turn_decision(&mut agent, snapshot("first", 1)),
        DecisionAdmission::Enqueued
    );
    wait_for_success(&runtime, 1);
    // Wait only in the fixture so the previous answer is in the mailbox.
    std::thread::sleep(Duration::from_millis(5));
    agent.prompt("Second unrelated task");
    assert_eq!(
        prepare_turn_decision(&mut agent, snapshot("second", 2)),
        DecisionAdmission::Enqueued
    );
    agent.poll_decision_advice(false);
    assert_eq!(agent.request_thinking_level(), ThinkingLevel::Low);
    assert!(agent.completion_advice().is_none());
    assert_eq!(runtime.telemetry().snapshot().ready_advice, 0);
}

#[test]
fn advice_for_a_different_current_key_is_rejected() {
    for field in ["turn", "evidence", "mutation", "generation"] {
        let (mut agent, _, runtime) = enabled_agent();
        agent.prompt("Fix the parser");
        assert_eq!(
            prepare_turn_decision(&mut agent, snapshot("first", 1)),
            DecisionAdmission::Enqueued
        );
        wait_for_success(&runtime, 1);
        let mut current = davinci_agent::decision::DecisionAdviceKey {
            request_id: "first".into(),
            generation: runtime.generation(),
            evidence_revision: 1,
            mutation_revision: 0,
        };
        match field {
            "turn" => current.request_id = "second".into(),
            "evidence" => current.evidence_revision += 1,
            "mutation" => current.mutation_revision += 1,
            _ => current.generation += 1,
        }
        agent.set_decision_advice_key(Some(current));
        let deadline = Instant::now() + Duration::from_secs(1);
        while runtime.telemetry().snapshot().stale_advice == 0 && Instant::now() < deadline {
            agent.poll_decision_advice(true);
            std::thread::yield_now();
        }
        assert_eq!(runtime.telemetry().snapshot().stale_advice, 1, "{field}");
        assert_eq!(
            agent.request_thinking_level(),
            ThinkingLevel::Low,
            "{field}"
        );
    }
}

#[test]
fn disabling_runtime_invalidates_already_consumed_effort() {
    let (mut agent, _, runtime) = enabled_agent();
    agent.prompt("Fix parser");
    prepare_turn_decision(&mut agent, snapshot("first", 1));
    poll_until_ready(&mut agent, &runtime);
    assert_eq!(agent.request_thinking_level(), ThinkingLevel::High);
    runtime.disable();
    assert_eq!(agent.request_thinking_level(), ThinkingLevel::Low);
}

#[test]
fn all_advice_subflags_off_makes_no_paid_shadow_request() {
    let (mut agent, provider, _) = enabled_agent();
    agent.set_decision_effort_advice_enabled(false);
    agent.prompt("Fix parser");
    assert_eq!(
        prepare_turn_decision(&mut agent, snapshot("first", 1)),
        DecisionAdmission::Disabled
    );
    assert_eq!(provider.calls.load(Ordering::SeqCst), 0);
}

fn coding_reply(
    content: davinci_ai::ContentBlock,
    stop_reason: davinci_ai::StopReason,
) -> davinci_ai::AssistantMessage {
    serde_json::from_value(json!({
        "id": davinci_agent::new_message_id(), "role": "assistant", "model": "fixture",
        "content": [content], "stopReason": stop_reason,
    }))
    .unwrap()
}

#[test]
fn shared_print_loop_consumes_same_turn_advice_between_tool_rounds() {
    use davinci_ai::{ContentBlock, StopReason};
    let (mut agent, _, runtime) = enabled_agent();
    let root = tempfile::tempdir().unwrap();
    agent.cwd = root.path().to_owned();
    agent.prompt("Inspect the parser");
    prepare_turn_decision(&mut agent, snapshot("print", 1));
    let mut observed = Vec::new();
    let events = agent
        .run_loop(|current| {
            observed.push(current.request_thinking_level());
            if observed.len() == 1 {
                wait_for_success(&runtime, 1);
                Ok(coding_reply(
                    ContentBlock::ToolCall {
                        id: "inspect".into(),
                        name: "ls".into(),
                        arguments: json!({"path":"."}),
                    },
                    StopReason::ToolUse,
                ))
            } else {
                Ok(coding_reply(
                    ContentBlock::Text {
                        text: "Inspected.".into(),
                    },
                    StopReason::Stop,
                ))
            }
        })
        .unwrap();
    assert!(!events.is_empty());
    assert_eq!(observed, [ThinkingLevel::Low, ThinkingLevel::High]);
}

#[test]
fn completion_shadow_consumes_fresh_evidence_without_an_extra_coding_request() {
    use davinci_ai::{ContentBlock, StopReason};
    let (mut agent, _, runtime) = enabled_agent();
    let root = tempfile::tempdir().unwrap();
    agent.cwd = root.path().to_owned();
    agent.set_decision_effort_advice_enabled(false);
    agent.set_decision_completion_advice_enabled(true);
    agent.prompt("Must preserve the public API");
    let mut submission = snapshot("completion", 1);
    submission.task = "Must preserve the public API".into();
    prepare_turn_decision(&mut agent, submission);
    let mut turns = 0;
    agent
        .run_loop(|_| {
            turns += 1;
            if turns == 1 {
                Ok(coding_reply(
                    ContentBlock::ToolCall {
                        id: "inspect".into(),
                        name: "ls".into(),
                        arguments: json!({"path":"."}),
                    },
                    StopReason::ToolUse,
                ))
            } else {
                wait_for_success(&runtime, 1);
                // Only fixture synchronization; the production consumer never waits.
                std::thread::sleep(Duration::from_millis(5));
                Ok(coding_reply(
                    ContentBlock::Text {
                        text: "Done.".into(),
                    },
                    StopReason::Stop,
                ))
            }
        })
        .unwrap();
    assert_eq!(turns, 2);
    let completion = agent
        .completion_advice()
        .expect("same-turn completion observation");
    assert_eq!(completion.key.request_id, "completion");
    assert_eq!(completion.requirements.len(), 1);
    assert!(completion.requirements[0].evidence_references.is_empty());
    assert_eq!(
        completion.deterministic_evidence,
        agent.completion_evidence()
    );
    assert_eq!(runtime.telemetry().snapshot().completion_advice_observed, 1);
}
