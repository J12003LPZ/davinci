use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::provider::DecisionError;
use super::risk::DecisionClass;

pub const MAX_REQUEST_BYTES: usize = 12 * 1024;
pub const MAX_TASK_CHARS: usize = 4096;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DecisionQuestionType {
    Noul,
    Choice,
    Score,
}

/// Score levels the System One API accepts (2..=10, ordered low to high).
pub const MIN_SCORE_LEVELS: usize = 2;
pub const MAX_SCORE_LEVELS: usize = 10;
/// Choice options the System One API accepts.
pub const MAX_CHOICE_OPTIONS: usize = 255;

/// Wire shape of `criteria`: Noul uses `{"true", "false"}` descriptions,
/// Choice maps option ids to descriptions, and Score is an ordered array of
/// level descriptions. The API rejects a Score question without levels.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DecisionCriteria {
    Levels(Vec<String>),
    Named(BTreeMap<String, String>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionQuestion {
    #[serde(rename = "type")]
    pub question_type: DecisionQuestionType,
    pub instructions: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub criteria: Option<DecisionCriteria>,
}

impl DecisionQuestion {
    pub fn noul(instructions: impl Into<String>) -> Self {
        Self {
            question_type: DecisionQuestionType::Noul,
            instructions: instructions.into(),
            criteria: None,
        }
    }

    /// A Noul whose yes and no outcomes are spelled out for the model.
    pub fn noul_with(
        instructions: impl Into<String>,
        when_true: impl Into<String>,
        when_false: impl Into<String>,
    ) -> Self {
        Self {
            question_type: DecisionQuestionType::Noul,
            instructions: instructions.into(),
            criteria: Some(DecisionCriteria::Named(BTreeMap::from([
                ("true".to_owned(), when_true.into()),
                ("false".to_owned(), when_false.into()),
            ]))),
        }
    }

    pub fn choice(
        instructions: impl Into<String>,
        criteria: impl IntoIterator<Item = (String, String)>,
    ) -> Self {
        Self {
            question_type: DecisionQuestionType::Choice,
            instructions: instructions.into(),
            criteria: Some(DecisionCriteria::Named(criteria.into_iter().collect())),
        }
    }

    /// `levels` are ordered from the low end of the scale to the high end.
    pub fn score(
        instructions: impl Into<String>,
        levels: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            question_type: DecisionQuestionType::Score,
            instructions: instructions.into(),
            criteria: Some(DecisionCriteria::Levels(
                levels.into_iter().map(Into::into).collect(),
            )),
        }
    }

    /// Option ids a Choice answer may select; empty for other question types.
    pub fn choice_ids(&self) -> impl Iterator<Item = &str> {
        let named = match (&self.question_type, &self.criteria) {
            (DecisionQuestionType::Choice, Some(DecisionCriteria::Named(map))) => Some(map),
            _ => None,
        };
        named
            .into_iter()
            .flat_map(|map| map.keys().map(String::as_str))
    }

    /// Number of Score levels; zero for other question types.
    pub fn score_levels(&self) -> usize {
        match (&self.question_type, &self.criteria) {
            (DecisionQuestionType::Score, Some(DecisionCriteria::Levels(levels))) => levels.len(),
            _ => 0,
        }
    }

    /// Mirrors the API's own shape rules, so a malformed question fails here
    /// instead of costing a round trip that returns 422 for every answer.
    fn validate_shape(&self, question_id: &str) -> Result<(), DecisionError> {
        let invalid = |reason: &str| {
            Err(DecisionError::InvalidRequest(format!(
                "question {question_id}: {reason}"
            )))
        };
        if self.instructions.trim().is_empty() {
            return invalid("instructions are empty");
        }
        match (self.question_type, &self.criteria) {
            (DecisionQuestionType::Noul, None) => Ok(()),
            (DecisionQuestionType::Noul, Some(DecisionCriteria::Named(map)))
                if map.len() == 2 && map.contains_key("true") && map.contains_key("false") =>
            {
                Ok(())
            }
            (DecisionQuestionType::Noul, _) => {
                invalid("noul criteria must describe both true and false")
            }
            // The API caps options at 255. The minimum of two is local
            // policy: a one-option Choice carries no decision.
            (DecisionQuestionType::Choice, Some(DecisionCriteria::Named(map)))
                if (2..=MAX_CHOICE_OPTIONS).contains(&map.len()) =>
            {
                Ok(())
            }
            (DecisionQuestionType::Choice, _) => invalid("choice needs 2 to 255 named options"),
            (DecisionQuestionType::Score, Some(DecisionCriteria::Levels(levels)))
                if (MIN_SCORE_LEVELS..=MAX_SCORE_LEVELS).contains(&levels.len()) =>
            {
                Ok(())
            }
            (DecisionQuestionType::Score, _) => invalid("score needs 2 to 10 ordered levels"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionRequest {
    pub request_id: String,
    pub decision_class: DecisionClass,
    pub state: Value,
    pub model: String,
    pub questions: BTreeMap<String, DecisionQuestion>,
}

impl DecisionRequest {
    pub fn new(
        request_id: impl Into<String>,
        decision_class: DecisionClass,
        state: Value,
        model: impl Into<String>,
        questions: impl IntoIterator<Item = (String, DecisionQuestion)>,
    ) -> Self {
        Self {
            request_id: request_id.into(),
            decision_class,
            state,
            model: model.into(),
            questions: questions.into_iter().collect(),
        }
    }

    /// Checks every question against the API's shape rules.
    pub fn validate_questions(&self) -> Result<(), DecisionError> {
        self.questions
            .iter()
            .try_for_each(|(question_id, question)| question.validate_shape(question_id))
    }

    /// Checks every question's API shape and the serialized size bound.
    pub fn validate_size(&self) -> Result<Vec<u8>, DecisionError> {
        self.validate_questions()?;
        let encoded = serde_json::to_vec(self)
            .map_err(|error| DecisionError::InvalidRequest(error.to_string()))?;
        if encoded.len() > MAX_REQUEST_BYTES {
            return Err(DecisionError::InvalidRequest(format!(
                "decision request exceeds {} bytes",
                MAX_REQUEST_BYTES
            )));
        }
        Ok(encoded)
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkspaceDirtyState {
    Clean,
    Dirty,
    #[default]
    Unknown,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn request(question: DecisionQuestion) -> DecisionRequest {
        DecisionRequest::new(
            "shape",
            DecisionClass::Ranking,
            json!({"task": "x"}),
            "jev-latest",
            [("q".to_owned(), question)],
        )
    }

    #[test]
    fn score_serializes_criteria_as_an_ordered_level_array() {
        let wire =
            serde_json::to_value(DecisionQuestion::score("How risky?", ["low", "high"])).unwrap();
        assert_eq!(wire["type"], "score");
        assert_eq!(wire["criteria"], json!(["low", "high"]));
    }

    #[test]
    fn noul_with_serializes_true_and_false_descriptions() {
        let wire =
            serde_json::to_value(DecisionQuestion::noul_with("Is it?", "yes", "no")).unwrap();
        assert_eq!(wire["criteria"], json!({"true": "yes", "false": "no"}));
        let bare = serde_json::to_value(DecisionQuestion::noul("Is it?")).unwrap();
        assert!(bare.get("criteria").is_none());
    }

    #[test]
    fn shapes_the_api_would_reject_fail_before_the_network() {
        let levelless_score = DecisionQuestion {
            question_type: DecisionQuestionType::Score,
            instructions: "s".into(),
            criteria: None,
        };
        let noul_with_only_true = DecisionQuestion {
            question_type: DecisionQuestionType::Noul,
            instructions: "n".into(),
            criteria: Some(DecisionCriteria::Named(BTreeMap::from([(
                "true".into(),
                "yes".into(),
            )]))),
        };
        let noul_with_extra_key = DecisionQuestion {
            question_type: DecisionQuestionType::Noul,
            instructions: "n".into(),
            criteria: Some(DecisionCriteria::Named(BTreeMap::from([(
                "maybe".into(),
                "?".into(),
            )]))),
        };
        for question in [
            DecisionQuestion::score("s", ["only"]),
            DecisionQuestion::score("s", (0..11).map(|n| n.to_string())),
            levelless_score,
            DecisionQuestion::choice("c", [("a".into(), "A".into())]),
            DecisionQuestion::noul("   "),
            noul_with_only_true,
            noul_with_extra_key,
        ] {
            assert!(
                matches!(
                    request(question.clone()).validate_size(),
                    Err(DecisionError::InvalidRequest(_))
                ),
                "accepted {question:?}"
            );
        }
        assert!(request(DecisionQuestion::score("s", ["a", "b"]))
            .validate_size()
            .is_ok());
    }
}
