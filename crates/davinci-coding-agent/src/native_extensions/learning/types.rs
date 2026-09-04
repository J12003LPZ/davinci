use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LearningScope {
    Project,
    Global,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactStatus {
    Candidate,
    PendingApproval,
    Active,
    Archived,
    Rejected,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillOrigin {
    User,
    Imported,
    LearnedForeground,
    LearnedReview,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillOutcome {
    VerifiedSuccess,
    VerifiedFailure,
    Neutral,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillUse {
    pub name: String,
    pub turn: u64,
    pub outcome: SkillOutcome,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct SkillVersionRef {
    pub name: String,
    pub version: u64,
    pub content_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct VerificationEvidence {
    #[serde(default)]
    pub graph_run_id: Option<String>,
    #[serde(default)]
    pub commands_ran: u32,
    #[serde(default)]
    pub passed: bool,
    #[serde(default)]
    pub user_accepted: bool,
    #[serde(default)]
    pub user_corrected: bool,
    #[serde(default)]
    pub permission_denied: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LearningArtifact {
    Memory {
        memory_kind: String,
        text: String,
        importance: f32,
    },
    SkillCreate {
        name: String,
        description: String,
        body: String,
    },
    SkillPatch {
        name: String,
        old_text: String,
        new_text: String,
        expected_hash: String,
    },
    SkillSupportFile {
        name: String,
        relative_path: String,
        content: String,
        #[serde(default)]
        expected_hash: Option<String>,
    },
    FailureLesson {
        text: String,
        importance: f32,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LearningCandidate {
    pub id: String,
    pub scope: LearningScope,
    pub status: ArtifactStatus,
    pub artifact: LearningArtifact,
    pub confidence: f32,
    pub source_session_id: String,
    pub source_repo_id: String,
    pub source_turn: u64,
    pub created_at_ms: u64,
    pub evidence: VerificationEvidence,
    pub rationale: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SkillLedgerRecord {
    pub skill_id: String,
    pub name: String,
    pub scope: LearningScope,
    pub origin: SkillOrigin,
    pub status: ArtifactStatus,
    pub path: PathBuf,
    pub content_hash: String,
    pub version: u32,
    pub success_count: u64,
    pub failure_count: u64,
    pub neutral_count: u64,
    #[serde(default)]
    pub last_used_at_ms: Option<u64>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    #[serde(default)]
    pub pinned: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SkillContextCandidate {
    pub name: String,
    pub version: u64,
    pub content_hash: String,
    pub body: String,
    pub score: f32,
    pub estimated_tokens: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct LearningStats {
    pub reviews_started: u64,
    pub reviews_completed: u64,
    pub reviews_cancelled: u64,
    pub reviews_failed: u64,
    pub candidates_created: u64,
    pub candidates_approved: u64,
    pub candidates_rejected: u64,
    pub skills_created: u64,
    pub skills_patched: u64,
    pub skills_retrieved: u64,
    pub verified_skill_successes: u64,
    pub verified_skill_failures: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ToolEvidence {
    pub name: String,
    pub is_error: bool,
    pub args_summary: String,
    pub result_summary: String,
    pub permission_denied: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LearningEvidence {
    pub session_id: String,
    pub repo_id: String,
    pub turn: u64,
    pub messages: Vec<crate::native_extensions::vector_memory::MemoryMessage>,
    pub tools: Vec<ToolEvidence>,
    pub run_stats: davinci_agent::RunStats,
    pub verification: VerificationEvidence,
}

impl LearningEvidence {
    #[allow(dead_code)]
    pub fn serialized_len(&self) -> usize {
        serde_json::to_string(self).map(|s| s.len()).unwrap_or(0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct LearningStoreState {
    pub last_updated_ms: u64,
    pub candidate_count: usize,
    pub skill_count: usize,
    pub version: u32,
}

pub fn now_ms() -> u64 {
    if let Ok(val) = std::env::var("PI_LEARNING_CLOCK_MS") {
        if let Ok(parsed) = val.parse::<u64>() {
            return parsed;
        }
    }
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
