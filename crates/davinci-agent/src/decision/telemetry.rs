use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use super::calibration::distribution_margin;
use super::provider::DecisionError;
use super::response::{DecisionAnswer, DecisionResponse};

const MAX_ANSWER_TELEMETRY: usize = 256;
const MAX_METADATA_TEXT_CHARS: usize = 128;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionAnswerTelemetry {
    pub question_id: String,
    pub answer_type: String,
    pub value: Option<f32>,
    pub choice: Option<String>,
    pub confidence: Option<f32>,
    pub probability_margin: Option<f32>,
    pub behavior_affecting: bool,
    pub shadow_only: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct DecisionTelemetrySnapshot {
    pub requests: u64,
    pub successes: u64,
    pub timeouts: u64,
    pub soft_deadline_misses: u64,
    pub credential_invalid: u64,
    pub schema_mismatch: u64,
    pub http_401: u64,
    pub http_422: u64,
    pub http_429: u64,
    pub http_529: u64,
    pub schema_failures: u64,
    pub rate_limited: u64,
    pub overloaded: u64,
    pub network_failures: u64,
    pub fallbacks: u64,
    pub additions: u64,
    pub disagreements: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub latency_ms_total: u64,
    pub answers: Vec<DecisionAnswerTelemetry>,
}

#[derive(Debug, Default)]
pub struct DecisionTelemetry {
    snapshot: Mutex<DecisionTelemetrySnapshot>,
}

impl DecisionTelemetry {
    pub fn snapshot(&self) -> DecisionTelemetrySnapshot {
        self.snapshot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    pub fn record_request(&self) {
        self.with_snapshot(|snapshot| snapshot.requests += 1);
    }

    pub fn record_success(
        &self,
        latency_ms: u64,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
    ) {
        self.with_snapshot(|snapshot| {
            snapshot.successes += 1;
            snapshot.latency_ms_total = snapshot.latency_ms_total.saturating_add(latency_ms);
            snapshot.input_tokens = snapshot
                .input_tokens
                .saturating_add(input_tokens.unwrap_or(0));
            snapshot.output_tokens = snapshot
                .output_tokens
                .saturating_add(output_tokens.unwrap_or(0));
        });
    }

    pub fn record_answers(
        &self,
        response: &DecisionResponse,
        behavior_affecting: bool,
        shadow_only: bool,
    ) {
        let answers = response
            .answers
            .iter()
            .take(MAX_ANSWER_TELEMETRY)
            .map(|(question_id, answer)| {
                answer_metadata(question_id, answer, behavior_affecting, shadow_only)
            })
            .collect::<Vec<_>>();
        self.with_snapshot(|snapshot| {
            snapshot.answers.extend(answers);
            let excess = snapshot.answers.len().saturating_sub(MAX_ANSWER_TELEMETRY);
            if excess > 0 {
                snapshot.answers.drain(..excess);
            }
        });
    }

    pub fn record_failure(&self, error: &DecisionError, latency_ms: u64) {
        self.with_snapshot(|snapshot| {
            snapshot.latency_ms_total = snapshot.latency_ms_total.saturating_add(latency_ms);
            match error {
                DecisionError::CredentialInvalid => snapshot.credential_invalid += 1,
                DecisionError::HttpStatus(401) => {
                    snapshot.credential_invalid += 1;
                    snapshot.http_401 += 1;
                }
                DecisionError::SchemaMismatch(_) => {
                    snapshot.schema_mismatch += 1;
                    snapshot.schema_failures += 1;
                }
                DecisionError::HttpStatus(422) => {
                    snapshot.schema_mismatch += 1;
                    snapshot.http_422 += 1;
                    snapshot.schema_failures += 1;
                }
                DecisionError::RateLimited => snapshot.rate_limited += 1,
                DecisionError::HttpStatus(429) => {
                    snapshot.rate_limited += 1;
                    snapshot.http_429 += 1;
                }
                DecisionError::Overloaded => snapshot.overloaded += 1,
                DecisionError::HttpStatus(529) => {
                    snapshot.overloaded += 1;
                    snapshot.http_529 += 1;
                }
                DecisionError::Unavailable(_) => snapshot.network_failures += 1,
                DecisionError::Busy
                | DecisionError::StaleResponse
                | DecisionError::InvalidRequest(_)
                | DecisionError::Disabled => {}
                DecisionError::HttpStatus(_) => snapshot.network_failures += 1,
            }
        });
    }

    pub fn record_timeout(&self) {
        self.with_snapshot(|snapshot| snapshot.timeouts += 1);
    }
    pub fn record_soft_deadline_miss(&self) {
        self.with_snapshot(|snapshot| snapshot.soft_deadline_misses += 1);
    }

    pub fn record_fallback(&self) {
        self.with_snapshot(|snapshot| snapshot.fallbacks += 1);
    }

    pub fn record_addition(&self) {
        self.with_snapshot(|snapshot| snapshot.additions += 1);
    }

    pub fn record_disagreement(&self) {
        self.with_snapshot(|snapshot| snapshot.disagreements += 1);
    }

    fn with_snapshot(&self, update: impl FnOnce(&mut DecisionTelemetrySnapshot)) {
        let mut snapshot = self
            .snapshot
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        update(&mut snapshot);
    }
}

pub fn answer_metadata(
    question_id: &str,
    answer: &DecisionAnswer,
    behavior_affecting: bool,
    shadow_only: bool,
) -> DecisionAnswerTelemetry {
    let mut metadata = DecisionAnswerTelemetry {
        question_id: bounded_text(question_id),
        answer_type: String::new(),
        value: None,
        choice: None,
        confidence: None,
        probability_margin: None,
        behavior_affecting,
        shadow_only,
    };
    match answer {
        DecisionAnswer::Noul { value } => {
            metadata.answer_type = "noul".to_owned();
            metadata.value = Some(*value);
        }
        DecisionAnswer::Choice {
            choice,
            probabilities,
            confidence,
        } => {
            metadata.answer_type = "choice".to_owned();
            metadata.choice = Some(bounded_text(choice));
            metadata.confidence = Some(*confidence);
            metadata.probability_margin = distribution_margin(probabilities);
        }
        DecisionAnswer::Score {
            score,
            levels,
            probabilities,
            confidence,
        } => {
            metadata.answer_type = "score".to_owned();
            // Normalized to 0..1 so scales with different level counts compare.
            metadata.value = Some(score / levels.saturating_sub(1).max(1) as f32);
            metadata.confidence = Some(*confidence);
            metadata.probability_margin = distribution_margin(probabilities);
        }
    }
    metadata
}

fn bounded_text(value: &str) -> String {
    value.chars().take(MAX_METADATA_TEXT_CHARS).collect()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;

    #[test]
    fn answer_telemetry_contains_only_bounded_metadata() {
        let mut answers = BTreeMap::new();
        answers.insert(
            "browser_relevant".to_owned(),
            DecisionAnswer::Choice {
                choice: "browser".to_owned(),
                probabilities: BTreeMap::from([
                    ("browser".to_owned(), 0.9),
                    ("no_browser".to_owned(), 0.1),
                ]),
                confidence: 0.92,
            },
        );
        answers.insert(
            "change_impact_relevant".to_owned(),
            DecisionAnswer::Score {
                score: 1.6,
                levels: 3,
                probabilities: BTreeMap::from([
                    ("0".to_owned(), 0.0),
                    ("1".to_owned(), 0.4),
                    ("2".to_owned(), 0.6),
                ]),
                confidence: 0.8,
            },
        );
        answers.insert(
            "test_impact_relevant".to_owned(),
            DecisionAnswer::Noul { value: 0.9 },
        );
        let telemetry = DecisionTelemetry::default();
        telemetry.record_answers(
            &DecisionResponse {
                answers,
                model: None,
                input_tokens: None,
                output_tokens: None,
            },
            false,
            true,
        );

        let snapshot = telemetry.snapshot();
        assert_eq!(snapshot.answers.len(), 3);
        assert!(snapshot.answers.iter().all(|answer| answer.shadow_only));
        assert!(snapshot
            .answers
            .iter()
            .all(|answer| !answer.behavior_affecting));
        let browser = snapshot
            .answers
            .iter()
            .find(|answer| answer.question_id == "browser_relevant")
            .expect("choice metadata");
        assert_eq!(browser.answer_type, "choice");
        assert_eq!(browser.choice.as_deref(), Some("browser"));
        assert_eq!(browser.confidence, Some(0.92));
        assert!((browser.probability_margin.unwrap_or_default() - 0.8).abs() < 0.001);
        let score = snapshot
            .answers
            .iter()
            .find(|answer| answer.question_id == "change_impact_relevant")
            .expect("score metadata");
        // Level position 1.6 on a 0..=2 scale is recorded as 0.8.
        assert!((score.value.unwrap_or_default() - 0.8).abs() < 0.001);
    }

    #[test]
    fn answer_telemetry_is_bounded() {
        let mut answers = BTreeMap::new();
        answers.insert(
            "x".repeat(MAX_METADATA_TEXT_CHARS + 10),
            DecisionAnswer::Noul { value: 0.5 },
        );
        let telemetry = DecisionTelemetry::default();
        let response = DecisionResponse {
            answers,
            model: None,
            input_tokens: None,
            output_tokens: None,
        };
        for _ in 0..(MAX_ANSWER_TELEMETRY + 1) {
            telemetry.record_answers(&response, false, true);
        }
        let snapshot = telemetry.snapshot();
        assert_eq!(snapshot.answers.len(), MAX_ANSWER_TELEMETRY);
        assert_eq!(
            snapshot.answers[0].question_id.chars().count(),
            MAX_METADATA_TEXT_CHARS
        );
    }

    #[test]
    fn failure_telemetry_retains_status_classes_and_schema_failures() {
        let telemetry = DecisionTelemetry::default();
        for status in [401, 422, 429, 529] {
            telemetry.record_failure(&DecisionError::HttpStatus(status), 1);
        }
        telemetry.record_failure(&DecisionError::SchemaMismatch("invalid answer".into()), 1);
        telemetry.record_timeout();

        let snapshot = telemetry.snapshot();
        assert_eq!(snapshot.http_401, 1);
        assert_eq!(snapshot.http_422, 1);
        assert_eq!(snapshot.http_429, 1);
        assert_eq!(snapshot.http_529, 1);
        assert_eq!(snapshot.schema_failures, 2);
        assert_eq!(snapshot.timeouts, 1);
    }
}
