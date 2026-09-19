use super::super::context_manifest::ProvenanceKind;
use davinci_ai::ChatMessage;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextVmMode {
    Off,
    Shadow,
    Active,
}

impl ContextVmMode {
    pub fn from_env_value(value: Option<&str>) -> Self {
        match value {
            Some("shadow") => Self::Shadow,
            Some("active") => Self::Active,
            _ => Self::Off,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextVmConfig {
    pub mode: ContextVmMode,
    pub hot_event_tokens: u64,
    pub max_delta_pages: usize,
    pub max_delta_tokens: u64,
    pub window_pressure_percent: u8,
}

impl Default for ContextVmConfig {
    fn default() -> Self {
        Self {
            mode: ContextVmMode::Off,
            hot_event_tokens: 20_000,
            max_delta_pages: 8,
            max_delta_tokens: 8_000,
            window_pressure_percent: 80,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ContextPageKind {
    Checkpoint,
    Delta,
    Episode,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextPageRef {
    pub id: String,
    pub kind: ContextPageKind,
    pub content_hash: String,
    pub estimated_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProvenanceRef {
    pub kind: ProvenanceKind,
    pub source_refs: Vec<String>,
    pub content_hash: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateValue<T> {
    pub value: T,
    pub provenance: Vec<ProvenanceRef>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckpointState {
    pub through_seq: u64,
    pub goals: Vec<StateValue<String>>,
    pub constraints: Vec<StateValue<String>>,
    pub completed: Vec<StateValue<String>>,
    pub in_progress: Vec<StateValue<String>>,
    pub blockers: Vec<StateValue<String>>,
    pub decisions: Vec<StateValue<String>>,
    pub modified_files: Vec<StateValue<String>>,
    pub verification: Vec<StateValue<String>>,
    pub narrative: Option<StateValue<String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateDelta {
    pub from_seq: u64,
    pub through_seq: u64,
    pub checkpoint_patch: CheckpointState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Episode {
    pub title: String,
    pub outcome: String,
    pub source_refs: Vec<String>,
    pub artifact_refs: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextRoot {
    pub epoch: u64,
    pub checkpoint: Option<ContextPageRef>,
    pub deltas: Vec<ContextPageRef>,
    pub episodes: Vec<ContextPageRef>,
    pub hot_event_refs: Vec<String>,
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextImageEntry {
    pub id: String,
    pub category: String,
    pub provenance_kind: ProvenanceKind,
    pub source_ref: String,
    pub content_hash: String,
    pub content: String,
    pub estimated_tokens: u64,
    pub mandatory: bool,
    pub stable_for_cache: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ContextImage {
    pub root: ContextRoot,
    pub entries: Vec<ContextImageEntry>,
    pub messages: Vec<ChatMessage>,
    pub estimated_tokens: u64,
    pub prefix_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ArtifactRef {
    pub uri: String,
    pub content_hash: String,
    pub source_ref: String,
}
