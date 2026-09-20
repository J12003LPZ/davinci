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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DecisionQuestion {
    #[serde(rename = "type")]
    pub question_type: DecisionQuestionType,
    pub instructions: String,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub criteria: BTreeMap<String, String>,
}

impl DecisionQuestion {
    pub fn noul(instructions: impl Into<String>) -> Self {
        Self {
            question_type: DecisionQuestionType::Noul,
            instructions: instructions.into(),
            criteria: BTreeMap::new(),
        }
    }

    pub fn choice(
        instructions: impl Into<String>,
        criteria: impl IntoIterator<Item = (String, String)>,
    ) -> Self {
        Self {
            question_type: DecisionQuestionType::Choice,
            instructions: instructions.into(),
            criteria: criteria.into_iter().collect(),
        }
    }

    pub fn score(instructions: impl Into<String>) -> Self {
        Self {
            question_type: DecisionQuestionType::Score,
            instructions: instructions.into(),
            criteria: BTreeMap::new(),
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

    pub fn validate_size(&self) -> Result<Vec<u8>, DecisionError> {
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
