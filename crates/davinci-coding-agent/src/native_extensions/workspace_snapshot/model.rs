use serde::{Deserialize, Serialize};

fn default_max_files() -> usize {
    64
}

fn default_max_file_bytes() -> usize {
    16 * 1024 * 1024
}

fn default_max_total_bytes() -> usize {
    64 * 1024 * 1024
}

fn default_max_checkpoints() -> usize {
    32
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceSnapshotConfig {
    pub enabled: bool,
    #[serde(default = "default_max_files")]
    pub max_files: usize,
    #[serde(default = "default_max_file_bytes")]
    pub max_file_bytes: usize,
    #[serde(default = "default_max_total_bytes")]
    pub max_total_bytes: usize,
    #[serde(default = "default_max_checkpoints")]
    pub max_checkpoints: usize,
}

impl Default for WorkspaceSnapshotConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_files: default_max_files(),
            max_file_bytes: default_max_file_bytes(),
            max_total_bytes: default_max_total_bytes(),
            max_checkpoints: default_max_checkpoints(),
        }
    }
}

impl WorkspaceSnapshotConfig {
    pub fn bounded(mut self) -> Self {
        self.max_files = self.max_files.clamp(1, 128);
        self.max_file_bytes = self.max_file_bytes.clamp(1, 16 * 1024 * 1024);
        self.max_total_bytes = self
            .max_total_bytes
            .clamp(self.max_file_bytes, 64 * 1024 * 1024);
        self.max_checkpoints = self.max_checkpoints.clamp(1, 128);
        self
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceCheckpointArgs {
    pub paths: Vec<String>,
    pub path: Option<String>,
    pub transaction_id: Option<String>,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceDiffArgs {
    pub checkpoint_id: String,
    pub paths: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceRestoreArgs {
    pub checkpoint_id: String,
    pub transaction_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum SnapshotEntryKind {
    Missing,
    File,
    Symlink { target: String },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapshotEntry {
    pub path: String,
    pub kind: SnapshotEntryKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<u32>,
    pub size: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceCheckpoint {
    pub schema_version: u32,
    pub id: String,
    pub workspace: String,
    pub workspace_identity: String,
    pub entries: Vec<SnapshotEntry>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transaction_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotDiffEntry {
    pub path: String,
    pub status: String,
    pub checkpoint_hash: Option<String>,
    pub current_hash: Option<String>,
    pub safe_to_restore: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSnapshotTelemetry {
    pub requests: u64,
    pub checkpoints: u64,
    pub diffs: u64,
    pub restores: u64,
    pub conflicts: u64,
    pub failures: u64,
    pub last_latency_ms: f64,
}
