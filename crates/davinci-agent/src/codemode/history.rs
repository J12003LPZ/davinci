//! Versioned history items for one Script run.
use super::{CodeModeMode, CodeModeOutcome};
use serde::{Deserialize, Serialize};

pub const SCRIPT_WIRE_KIND: &str = "function";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ScriptHistoryItem {
    pub version: u32,
    pub wire_kind: String,
    pub wire_name: String,
    pub call_id: String,
    pub operation_ref: String,
    pub output: String,
    pub complete: bool,
}

impl ScriptHistoryItem {
    /// The persisted projection of a finished run. It depends only on the
    /// outcome, so a later mode switch or model change cannot rewrite it.
    pub fn from_outcome(outcome: &CodeModeOutcome) -> Self {
        Self {
            version: 1,
            wire_kind: SCRIPT_WIRE_KIND.into(),
            wire_name: "codemode".into(),
            call_id: outcome.operation_ref.clone(),
            operation_ref: outcome.operation_ref.clone(),
            output: outcome.output_text.clone(),
            complete: outcome.output_complete,
        }
    }
}

/// Project the effect-safe presentation of a finished Script. `mode` is
/// accepted for callers that need the run mode, but it must not appear in the
/// projection: including it would rewrite cached history on a switch.
pub fn project_script_history(outcome: &CodeModeOutcome, mode: &CodeModeMode) -> ScriptHistoryItem {
    let _ = mode;
    ScriptHistoryItem::from_outcome(outcome)
}
