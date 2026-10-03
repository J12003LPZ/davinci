use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitIntelligenceConfig {
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    #[serde(default = "default_max_commits")]
    pub max_commits: usize,
    #[serde(default = "default_max_diff_bytes")]
    pub max_diff_bytes: usize,
    #[serde(default = "default_timeout_ms")]
    pub timeout_ms: u64,
}

fn default_enabled() -> bool {
    true
}

fn default_max_commits() -> usize {
    50
}

fn default_max_diff_bytes() -> usize {
    1_000_000
}

fn default_timeout_ms() -> u64 {
    10_000
}

impl Default for GitIntelligenceConfig {
    fn default() -> Self {
        Self {
            enabled: default_enabled(),
            max_commits: default_max_commits(),
            max_diff_bytes: default_max_diff_bytes(),
            timeout_ms: default_timeout_ms(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IntroductionStatus {
    Introduced,
    Unknown,
    Partial,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolCommitFact {
    pub commit: String,
    pub author: String,
    pub author_email: String,
    pub date: String,
    pub summary: String,
    pub change_kind: String,
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lines_changed: Option<usize>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileMovement {
    pub from_path: String,
    pub to_path: String,
    pub commit: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolHistoryFacts {
    pub commits_inspected: usize,
    pub revisions_with_symbol: usize,
    pub total_line_modifications: usize,
    pub is_shallow: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SymbolHistoryInference {
    pub notes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolHistoryResult {
    pub symbol: String,
    pub file: String,
    pub introduction_status: IntroductionStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub introduced_commit: Option<SymbolCommitFact>,
    pub modifications: Vec<SymbolCommitFact>,
    pub file_movements: Vec<FileMovement>,
    pub facts: SymbolHistoryFacts,
    pub inference: SymbolHistoryInference,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitFacts {
    pub files_changed: usize,
    pub insertions: usize,
    pub deletions: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitInference {
    pub pr_number: Option<u64>,
    pub intent_summary: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelatedCommit {
    pub commit: String,
    pub author: String,
    pub author_email: String,
    pub date: String,
    pub message: String,
    pub facts: CommitFacts,
    pub inference: CommitInference,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RelatedCommitsResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub query: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol: Option<String>,
    pub total_commits: usize,
    pub commits: Vec<RelatedCommit>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangedSymbolRange {
    pub start_line: usize,
    pub end_line: usize,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChangedSymbol {
    pub name: String,
    pub qualified_name: String,
    pub kind: String,
    pub file: String,
    pub change_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_range: Option<ChangedSymbolRange>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub head_range: Option<ChangedSymbolRange>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChangedSymbolsResult {
    pub base: String,
    pub head: String,
    pub total_changed_symbols: usize,
    pub symbols: Vec<ChangedSymbol>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileDiffSummary {
    pub path: String,
    pub status: String,
    pub insertions: usize,
    pub deletions: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patch_snippet: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BranchDiffResult {
    pub base: String,
    pub head: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merge_base: Option<String>,
    pub commits_ahead: usize,
    pub commits_behind: usize,
    pub files_changed: usize,
    pub insertions: usize,
    pub deletions: usize,
    pub files: Vec<FileDiffSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlameLine {
    pub line_number: usize,
    pub commit: String,
    pub author: String,
    pub date: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlameAuthorStat {
    pub author: String,
    pub line_count: usize,
    pub percentage: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SymbolBlameResult {
    pub symbol: String,
    pub file: String,
    pub revision: String,
    pub start_line: usize,
    pub end_line: usize,
    pub total_lines: usize,
    pub lines: Vec<BlameLine>,
    pub authors: Vec<BlameAuthorStat>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub most_recent_commit: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitContextFacts {
    pub files_changed: Vec<String>,
    pub insertions: usize,
    pub deletions: usize,
    pub modified_symbols: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitContextInference {
    pub pr_reference: Option<String>,
    pub intent_summary: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommitContextResult {
    pub commit: String,
    pub author: String,
    pub author_email: String,
    pub date: String,
    pub message: String,
    pub parents: Vec<String>,
    pub facts: CommitContextFacts,
    pub inference: CommitContextInference,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictRange {
    pub start_line: usize,
    pub end_line: usize,
    pub ours_content: String,
    pub theirs_content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_content: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictFileReport {
    pub path: String,
    pub has_base_stage: bool,
    pub has_ours_stage: bool,
    pub has_theirs_stage: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub base_object: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ours_object: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub theirs_object: Option<String>,
    pub conflict_markers_count: usize,
    pub conflicting_ranges: Vec<ConflictRange>,
    pub overlapping_symbols: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConflictExplainResult {
    pub total_conflicted_files: usize,
    pub files: Vec<ConflictFileReport>,
    pub zero_mutation_guaranteed: bool,
}
