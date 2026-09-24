use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;

use super::provider::DecisionError;
use super::request::DecisionQuestionType;
use super::request::DecisionRequest;

pub const MAX_RESPONSE_BYTES: usize = 32 * 1024;
const SCORE_EPSILON: f64 = 1e-6;

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
    /// `score` is the probability-weighted level position in
    /// `0..=levels - 1`, not a probability. Divide by `levels - 1` for a
    /// 0..1 value.
    Score {
        score: f32,
        levels: usize,
        probabilities: BTreeMap<String, f32>,
        confidence: f32,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct DecisionResponse {
    pub answers: BTreeMap<String, DecisionAnswer>,
    /// Concrete model that answered, such as `jev-1.13.0` for `jev-latest`.
    pub model: Option<String>,
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
    // The documented response is exactly `model`, `answers` and `usage`.
    reject_unknown_keys(object.keys(), ["model", "answers", "usage"])?;
    let model = match object.get("model") {
        None => None,
        Some(Value::String(model)) if !model.is_empty() && model.len() <= 128 => {
            Some(model.clone())
        }
        Some(_) => return Err(schema("response.model must be a short string")),
    };

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
                if !question.choice_ids().any(|id| id == choice) {
                    return Err(schema(format!(
                        "answer {question_id} chose an unknown option"
                    )));
                }
                let probabilities = distribution(answer.get("probabilities"), "probabilities")?;
                if !probabilities.contains_key(&choice)
                    || probabilities
                        .keys()
                        .any(|key| !question.choice_ids().any(|id| id == key))
                {
                    return Err(schema(format!(
                        "answer {question_id} probabilities do not match its options"
                    )));
                }
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
                    ["type", "score", "legend", "probabilities", "confidence"],
                )?;
                let levels = question.score_levels();
                let top = levels.saturating_sub(1) as f64;
                // A probability-weighted mean can land a float ulp past an
                // end level; tolerate that and clamp, reject anything more.
                let score = answer
                    .get("score")
                    .and_then(Value::as_f64)
                    .filter(|score| {
                        score.is_finite() && (-SCORE_EPSILON..=top + SCORE_EPSILON).contains(score)
                    })
                    .ok_or_else(|| {
                        schema(format!("answer {question_id} score is outside its levels"))
                    })?
                    .clamp(0.0, top) as f32;
                if answer
                    .get("legend")
                    .is_some_and(|legend| !legend.is_object())
                {
                    return Err(schema(format!("answer {question_id} legend is invalid")));
                }
                let probabilities = distribution(answer.get("probabilities"), "probabilities")?;
                if probabilities.len() != levels
                    || (0..levels).any(|level| !probabilities.contains_key(&level.to_string()))
                {
                    return Err(schema(format!(
                        "answer {question_id} probabilities do not match its levels"
                    )));
                }
                let confidence = probability(answer.get("confidence"), "confidence")?;
                DecisionAnswer::Score {
                    score,
                    levels,
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
        model,
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
    // Providers may round each entry to two decimals (the documented examples
    // do), so the drift allowance grows with the number of entries.
    let tolerance = (0.005 * result.len() as f32).max(0.02);
    if !(1.0 - tolerance..=1.0 + tolerance).contains(&total) {
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
    reject_unknown_keys(object.keys(), ["input_tokens", "output_tokens"])?;
    let input = object
        .get("input_tokens")
        .map(parse_token_count)
        .transpose()?;
    let output = object
        .get("output_tokens")
        .map(parse_token_count)
        .transpose()?;
    Ok((input, output))
}

fn parse_token_count(value: &Value) -> Result<u64, DecisionError> {
    value
        .as_u64()
        .ok_or_else(|| schema("usage token counts must be non-negative integers"))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::decision::request::DecisionQuestion;

    fn request(id: &str, question: DecisionQuestion) -> DecisionRequest {
        DecisionRequest::new(
            "wire",
            crate::decision::risk::DecisionClass::Ranking,
            json!({"task": "x"}),
            "jev-latest",
            [(id.to_owned(), question)],
        )
    }

    // The three response bodies below are copied verbatim from
    // https://docs.typesafe.ai/api.md (fetched 2026-09-24). Before this fix
    // the parser rejected all three: `model` and snake_case `usage` keys were
    // unknown, `legend` was unknown, and a Score above 1.0 was out of range.

    #[test]
    fn documented_noul_response_is_accepted() {
        let raw = br#"{
  "model": "jev-1.13.0",
  "answers": {
    "is_urgent": {
      "type": "noul",
      "noul": 0.95
    }
  },
  "usage": { "input_tokens": 307, "output_tokens": 20 }
}"#;
        let parsed =
            parse_and_validate_response(raw, &request("is_urgent", DecisionQuestion::noul("u")))
                .expect("documented noul response");
        assert_eq!(parsed.model.as_deref(), Some("jev-1.13.0"));
        assert_eq!(parsed.input_tokens, Some(307));
        assert_eq!(parsed.output_tokens, Some(20));
        assert!(matches!(
            parsed.answers["is_urgent"],
            DecisionAnswer::Noul { value } if (value - 0.95).abs() < 1e-6
        ));
    }

    #[test]
    fn documented_choice_response_is_accepted() {
        let raw = br#"{
  "model": "jev-1.13.0",
  "answers": {
    "department": {
      "type": "choice",
      "choice": "billing",
      "probabilities": { "billing": 0.88, "technical": 0.12, "sales": 0.0 },
      "confidence": 0.81
    }
  },
  "usage": { "input_tokens": 318, "output_tokens": 34 }
}"#;
        let question = DecisionQuestion::choice(
            "Which department?",
            ["billing", "technical", "sales"].map(|id| (id.to_owned(), id.to_owned())),
        );
        let parsed = parse_and_validate_response(raw, &request("department", question))
            .expect("documented choice response");
        assert!(matches!(
            &parsed.answers["department"],
            DecisionAnswer::Choice { choice, .. } if choice == "billing"
        ));
    }

    #[test]
    fn documented_score_response_is_accepted_as_a_level_position() {
        let raw = br#"{
  "model": "jev-1.13.0",
  "answers": {
    "frustration": {
      "type": "score",
      "score": 1.05,
      "legend": { "0": "Calm", "1": "Frustrated", "2": "Very angry" },
      "probabilities": { "0": 0.0, "1": 0.95, "2": 0.05 },
      "confidence": 0.92
    }
  },
  "usage": { "input_tokens": 304, "output_tokens": 18 }
}"#;
        let question =
            DecisionQuestion::score("How frustrated?", ["Calm", "Frustrated", "Very angry"]);
        let parsed = parse_and_validate_response(raw, &request("frustration", question))
            .expect("documented score response");
        assert!(matches!(
            parsed.answers["frustration"],
            DecisionAnswer::Score { score, levels: 3, .. } if (score - 1.05).abs() < 1e-6
        ));
    }

    #[test]
    fn score_outside_its_levels_or_with_foreign_level_keys_is_rejected() {
        let question = || DecisionQuestion::score("s", ["low", "mid", "high"]);
        for raw in [
            // Above the top level (2).
            json!({"answers":{"s":{"type":"score","score":2.5,
                "probabilities":{"0":0.0,"1":0.0,"2":1.0},"confidence":0.9}}}),
            // Negative.
            json!({"answers":{"s":{"type":"score","score":-0.1,
                "probabilities":{"0":1.0,"1":0.0,"2":0.0},"confidence":0.9}}}),
            // Level keys that do not match the three requested levels.
            json!({"answers":{"s":{"type":"score","score":1.0,
                "probabilities":{"0":0.5,"1":0.5},"confidence":0.9}}}),
            // Legend must be an object when present.
            json!({"answers":{"s":{"type":"score","score":1.0,"legend":"x",
                "probabilities":{"0":0.0,"1":1.0,"2":0.0},"confidence":0.9}}}),
        ] {
            let raw = serde_json::to_vec(&raw).unwrap();
            assert!(
                matches!(
                    parse_and_validate_response(&raw, &request("s", question())),
                    Err(DecisionError::SchemaMismatch(_))
                ),
                "accepted {}",
                String::from_utf8_lossy(&raw)
            );
        }
    }

    #[test]
    fn undocumented_top_level_fields_and_camel_case_usage_stay_rejected() {
        for raw in [
            json!({"answers":{"u":{"type":"noul","noul":0.5}},"debug":"x"}),
            json!({"answers":{"u":{"type":"noul","noul":0.5}},"model":7}),
            json!({"answers":{"u":{"type":"noul","noul":0.5}},"usage":{"inputTokens":1}}),
        ] {
            let raw = serde_json::to_vec(&raw).unwrap();
            assert!(matches!(
                parse_and_validate_response(&raw, &request("u", DecisionQuestion::noul("u"))),
                Err(DecisionError::SchemaMismatch(_))
            ));
        }
    }

    #[test]
    fn float_noise_at_the_top_level_is_clamped_not_rejected() {
        let raw = br#"{"answers":{"s":{"type":"score","score":2.0000000000000004,"probabilities":{"0":0.0,"1":0.0,"2":1.0},"confidence":0.99}}}"#;
        let parsed = parse_and_validate_response(
            raw,
            &request("s", DecisionQuestion::score("s", ["a", "b", "c"])),
        )
        .expect("ulp above the top level");
        assert!(matches!(
            parsed.answers["s"],
            DecisionAnswer::Score { score, .. } if score == 2.0
        ));
    }

    #[test]
    fn two_decimal_rounding_across_ten_levels_is_accepted() {
        // Nine levels at 0.10 plus one at 0.06 sums to 0.96.
        let mut probabilities = serde_json::Map::new();
        for level in 0..10 {
            probabilities.insert(
                level.to_string(),
                json!(if level == 9 { 0.06 } else { 0.10 }),
            );
        }
        let raw = serde_json::to_vec(&json!({"answers":{"s":{"type":"score","score":4.4,
            "probabilities":probabilities,"confidence":0.1}}}))
        .unwrap();
        let question = DecisionQuestion::score("s", (0..10).map(|n| n.to_string()));
        assert!(parse_and_validate_response(&raw, &request("s", question)).is_ok());
    }

    #[test]
    fn choice_probabilities_must_name_declared_options_and_the_choice() {
        let question = || {
            DecisionQuestion::choice(
                "c",
                [
                    ("a".to_owned(), "A".to_owned()),
                    ("b".to_owned(), "B".to_owned()),
                ],
            )
        };
        for raw in [
            &br#"{"answers":{"c":{"type":"choice","choice":"a","probabilities":{"zzz":1.0},"confidence":0.9}}}"#[..],
            &br#"{"answers":{"c":{"type":"choice","choice":"a","probabilities":{"b":1.0},"confidence":0.9}}}"#[..],
        ] {
            assert!(matches!(
                parse_and_validate_response(raw, &request("c", question())),
                Err(DecisionError::SchemaMismatch(_))
            ));
        }
    }

    #[test]
    fn choice_outside_declared_options_is_rejected() {
        let question = DecisionQuestion::choice(
            "c",
            [
                ("a".to_owned(), "A".to_owned()),
                ("b".to_owned(), "B".to_owned()),
            ],
        );
        let raw = br#"{"answers":{"c":{"type":"choice","choice":"z","probabilities":{"z":1.0},"confidence":1.0}}}"#;
        assert!(matches!(
            parse_and_validate_response(raw, &request("c", question)),
            Err(DecisionError::SchemaMismatch(_))
        ));
    }
}
