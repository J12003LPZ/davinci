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

#[test]
fn ready_shadow_advice_applies_only_when_constructing_a_later_request() {
    let provider = Arc::new(FixtureProvider {
        calls: AtomicUsize::new(0),
    });
    let runtime = Arc::new(DecisionRuntime::new(provider.clone()));
    runtime.enable();

    let mut agent = Agent::new("test system");
    agent.set_decision_runtime(runtime);
    agent.set_decision_effort_advice_enabled(true);
    agent.thinking_level = ThinkingLevel::Medium;
    agent.effort_policy = EffortPolicy::Adaptive;

    let started = Instant::now();
    assert_eq!(
        prepare_turn_decision(&mut agent, snapshot("first", 1)),
        DecisionAdmission::Enqueued
    );
    assert_eq!(agent.request_thinking_level(), ThinkingLevel::Low);
    assert!(started.elapsed() < Duration::from_millis(100));

    let deadline = Instant::now() + Duration::from_secs(1);
    while provider.calls.load(Ordering::SeqCst) == 0 && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(1));
    }
    assert_eq!(provider.calls.load(Ordering::SeqCst), 1);

    assert_eq!(
        prepare_turn_decision(&mut agent, snapshot("second", 2)),
        DecisionAdmission::Enqueued
    );
    assert_eq!(agent.request_thinking_level(), ThinkingLevel::High);
}
