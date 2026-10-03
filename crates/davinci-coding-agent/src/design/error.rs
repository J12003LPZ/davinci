use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "code",
    content = "message",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum DesignError {
    Denied(String),
    Conflict(String),
    NotFound(String),
    InvalidInput(String),
    UnsupportedSchema(String),
    BudgetExceeded(String),
    MissingCapability(String),
    StaleSource(String),
    Cancelled,
    CorruptArtifact(String),
    IncompleteEvidence(String),
    IoFailure(String),
}
impl fmt::Display for DesignError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for DesignError {}
impl From<std::io::Error> for DesignError {
    fn from(error: std::io::Error) -> Self {
        Self::IoFailure(error.to_string())
    }
}
impl From<serde_json::Error> for DesignError {
    fn from(error: serde_json::Error) -> Self {
        Self::InvalidInput(error.to_string())
    }
}
pub type DesignResult<T> = Result<T, DesignError>;
