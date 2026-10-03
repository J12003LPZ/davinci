//! Evidence types and canonical evidence verification for task completion.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::ids::{AgentId, EvidenceId, TaskId};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExitOutcome {
    Exited(i32),
    Signalled,
    TimedOut,
    Cancelled,
    NotStarted,
}

impl ExitOutcome {
    pub fn is_success(&self) -> bool {
        matches!(self, ExitOutcome::Exited(0))
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactRef {
    pub id: String,
    pub sha256: String,
    pub media_type: String,
    pub size: u64,
    pub relative_store_path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub redaction: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    Implementation,
    TargetedTests,
    Build,
    InstalledApp,
    LiveUiCheck,
    InteractionTesting,
    Tool,
    Custom(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceStatus {
    Current,
    Stale,
    Failed,
    Unproven,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DimensionState {
    NotRequired,
    NotPerformed,
    Running,
    PassedCurrent,
    Failed,
    Stale,
    Unproven,
}

impl DimensionState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::NotRequired => "not_required",
            Self::NotPerformed => "not_performed",
            Self::Running => "running",
            Self::PassedCurrent => "passed_current",
            Self::Failed => "failed",
            Self::Stale => "stale",
            Self::Unproven => "unproven",
        }
    }

    pub fn is_passed_current(&self) -> bool {
        matches!(self, Self::PassedCurrent)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AssertionCounts {
    pub total: u32,
    pub passed: u32,
    pub failed: u32,
    pub skipped: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvidenceRecord {
    pub id: EvidenceId,
    pub task_id: TaskId,
    pub attempt: u32,
    pub actor_id: AgentId,
    pub operation_id: Option<String>,
    pub requirement_id: Option<String>,
    pub kind: EvidenceKind,
    pub argv: Vec<String>,
    pub cwd: String,
    pub started_at_ms: i64,
    pub finished_at_ms: i64,
    pub exit: ExitOutcome,
    pub source_before: String,
    pub source_after: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input_manifest: Option<String>,
    #[serde(default)]
    pub runtime_versions: HashMap<String, String>,
    #[serde(default)]
    pub artifact_refs: Vec<ArtifactRef>,
    #[serde(default)]
    pub assertions: AssertionCounts,
    pub status: EvidenceStatus,
}

impl EvidenceRecord {
    pub fn is_current(&self, current_source: &str, complete_coverage: bool) -> bool {
        self.exit.is_success()
            && evidence_current(
                &self.source_before,
                &self.source_after,
                current_source,
                complete_coverage,
            )
    }

    /// Link this evidence record to a progress watchdog observation to mark progress.
    pub fn link_to_observation(
        &self,
        observation: &mut super::progress_watchdog::ProgressObservation,
    ) {
        observation.has_new_evidence = true;
        observation
            .hypothesis_evidence_refs
            .push(self.id.to_string());
    }
}

pub fn evidence_current(before: &str, after: &str, current: &str, complete_coverage: bool) -> bool {
    complete_coverage && before == after && after == current
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn f06_source_must_match() {
        assert!(evidence_current("a", "a", "a", true));
        assert!(!evidence_current("a", "b", "b", true));
        assert!(!evidence_current("a", "a", "b", true));
        assert!(!evidence_current("a", "a", "a", false));
    }
}
