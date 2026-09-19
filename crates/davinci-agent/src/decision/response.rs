use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::provider::DecisionError;
use super::request::DecisionQuestionType;
use super::request::DecisionRequest;

pub const MAX_RESPONSE_BYTES: usize = 32 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub enum DecisionAnswer {
    Noul {
        value: f32,
    },
    Choice {
        choice: String,
        probabilities: BTreeMap<String, f32>,
        confidence: f32,
    },
    Score {
        score: f32,
        probabilities: BTreeMap<String, f32>,
        confidence: f32,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct DecisionResponse {
    pub answers: BTreeMap<String, DecisionAnswer>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
}

pub fn parse_and_validate_response(
    raw: &[u8],
    request: &DecisionRequest,
) -> Result<DecisionResponse, DecisionError> {
    if raw.len() > MAX_RESPONSE_BYTES {
        return Err(schema(
            "decision response exceeds the bounded response size",
        ));
    }

    let value: Value = serde_json::from_slice(raw).map_err(|error| schema(error.to_string()))?;
    let object = value
        .as_object()
        .ok_or_else(|| schema("response must be a JSON object"))?;
    reject_unknown_keys(object.keys(), ["answers", "usage", "schemaVersion"])?;

    let answers = object
        .get("answers")
        .and_then(Value::as_object)
        .ok_or_else(|| schema("response.answers must be an object"))?;
    if answers.len() != request.questions.len()
        || request
            .questions
            .keys()
            .any(|question_id| !answers.contains_key(question_id))
    {
        return Err(schema(
            "response answers do not exactly match the request questions",
        ));
    }

    let mut parsed = BTreeMap::new();
    for (question_id, question) in &request.questions {
        let answer = answers
            .get(question_id)
            .and_then(Value::as_object)
            .ok_or_else(|| schema(format!("answer {question_id} must be an object")))?;
        let answer_type = answer
            .get("type")
            .and_then(Value::as_str)
            .ok_or_else(|| schema(format!("answer {question_id} is missing type")))?;

        let parsed_answer = match question.question_type {
            DecisionQuestionType::Noul => {
                if answer_type != "noul" {
                    return Err(schema(format!("answer {question_id} has the wrong type")));
                }
                reject_unknown_keys(answer.keys(), ["type", "noul"])?;
                let value = probability(answer.get("noul"), "noul")?;
                DecisionAnswer::Noul { value }
            }
            DecisionQuestionType::Choice => {
                if answer_type != "choice" {
                    return Err(schema(format!("answer {question_id} has the wrong type")));
                }
                reject_unknown_keys(
                    answer.keys(),
                    ["type", "choice", "probabilities", "confidence"],
                )?;
                let choice = answer
                    .get("choice")
                    .and_then(Value::as_str)
                    .ok_or_else(|| schema(format!("answer {question_id} is missing choice")))?
                    .to_owned();
                if !question.criteria.is_empty() && !question.criteria.contains_key(&choice) {
                    return Err(schema(format!(
                        "answer {question_id} chose an unknown option"
                    )));
                }
                let probabilities = distribution(answer.get("probabilities"), "probabilities")?;
                let confidence = probability(answer.get("confidence"), "confidence")?;
                DecisionAnswer::Choice {
                    choice,
                    probabilities,
                    confidence,
                }
            }
            DecisionQuestionType::Score => {
                if answer_type != "score" {
                    return Err(schema(format!("answer {question_id} has the wrong type")));
                }
                reject_unknown_keys(
                    answer.keys(),
                    ["type", "score", "probabilities", "confidence"],
                )?;
                let score = probability(answer.get("score"), "score")?;
                let probabilities = distribution(answer.get("probabilities"), "probabilities")?;
                let confidence = probability(answer.get("confidence"), "confidence")?;
                DecisionAnswer::Score {
                    score,
                    probabilities,
                    confidence,
                }
            }
        };
        parsed.insert(question_id.clone(), parsed_answer);
    }

    let (input_tokens, output_tokens) = usage(object.get("usage"))?;
    Ok(DecisionResponse {
        answers: parsed,
        input_tokens,
        output_tokens,
    })
}

fn schema(message: impl Into<String>) -> DecisionError {
    DecisionError::SchemaMismatch(message.into())
}

fn reject_unknown_keys<'a, I>(
    keys: I,
    allowed: impl IntoIterator<Item = &'a str>,
) -> Result<(), DecisionError>
where
    I: IntoIterator<Item = &'a String>,
{
    let allowed: BTreeSet<&str> = allowed.into_iter().collect();
    if keys.into_iter().any(|key| !allowed.contains(key.as_str())) {
        return Err(schema("response contains an unknown field"));
    }
    Ok(())
}

fn probability(value: Option<&Value>, field: &str) -> Result<f32, DecisionError> {
    let number = value
        .and_then(Value::as_f64)
        .ok_or_else(|| schema(format!("missing or invalid {field}")))?;
    if !number.is_finite() || !(0.0..=1.0).contains(&number) {
        return Err(schema(format!("{field} is outside [0, 1]")));
    }
    Ok(number as f32)
}

fn distribution(
    value: Option<&Value>,
    field: &str,
) -> Result<BTreeMap<String, f32>, DecisionError> {
    let object = value
        .and_then(Value::as_object)
        .ok_or_else(|| schema(format!("missing or invalid {field}")))?;
    if object.is_empty() {
        return Err(schema(format!("{field} cannot be empty")));
    }
    let mut result = BTreeMap::new();
    let mut total = 0.0_f32;
    for (key, value) in object {
        let parsed = probability(Some(value), field)?;
        total += parsed;
        result.insert(key.clone(), parsed);
    }
    if !(0.98..=1.02).contains(&total) {
        return Err(schema(format!("{field} does not sum to one")));
    }
    Ok(result)
}

fn usage(value: Option<&Value>) -> Result<(Option<u64>, Option<u64>), DecisionError> {
    let Some(value) = value else {
        return Ok((None, None));
    };
    let object = value
        .as_object()
        .ok_or_else(|| schema("usage must be an object"))?;
    reject_unknown_keys(object.keys(), ["inputTokens", "outputTokens"])?;
    let input = object
        .get("inputTokens")
        .map(parse_token_count)
        .transpose()?;
    let output = object
        .get("outputTokens")
        .map(parse_token_count)
        .transpose()?;
    Ok((input, output))
}

fn parse_token_count(value: &Value) -> Result<u64, DecisionError> {
    value
        .as_u64()
        .ok_or_else(|| schema("usage token counts must be non-negative integers"))
}
