//! Workflow artifact and state storage.
//!
//! Keeps intermediate worker results outside the main model context,
//! providing Governor-backed overflow storage for large artifacts.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};
use thiserror::Error;
use uuid::Uuid;

use crate::runtime::ids::{AgentId, WorkflowId};

/// Default cap above which artifacts are offloaded to overflow storage.
pub const DEFAULT_MAX_INLINE_ARTIFACT_BYTES: usize = 32 * 1024; // 32 KB

#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum WorkflowStateError {
    #[error("artifact not found: {0}")]
    ArtifactNotFound(Uuid),
    #[error("io error: {0}")]
    IoError(String),
    #[error("serialization error: {0}")]
    SerializationError(String),
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// A single artifact produced by a workflow worker in a phase.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct WorkflowArtifact {
    pub id: Uuid,
    pub workflow_id: WorkflowId,
    pub phase_id: String,
    pub worker_id: AgentId,
    #[serde(default)]
    pub schema: Option<Value>,
    /// Inline value or overflow reference descriptor.
    pub value: Value,
    pub is_overflow: bool,
    pub byte_size: usize,
    #[serde(default)]
    pub overflow_path: Option<PathBuf>,
    pub created_ms: i64,
}

impl WorkflowArtifact {
    /// Return a concise reference representation suitable for prompt injection.
    pub fn to_reference_summary(&self) -> Value {
        serde_json::json!({
            "artifact_id": self.id.to_string(),
            "phase": self.phase_id,
            "worker": self.worker_id.to_string(),
            "size_bytes": self.byte_size,
            "is_overflow": self.is_overflow,
        })
    }
}

type PhaseArtifactMap = HashMap<(WorkflowId, String), Vec<Uuid>>;

/// Thread-safe in-memory and disk-overflow artifact store.
#[derive(Debug, Clone)]
pub struct WorkflowStateStore {
    max_inline_bytes: usize,
    overflow_dir: PathBuf,
    artifacts: Arc<RwLock<HashMap<Uuid, WorkflowArtifact>>>,
    // (workflow_id, phase_id) -> list of artifact ids
    phase_index: Arc<RwLock<PhaseArtifactMap>>,
}

impl Default for WorkflowStateStore {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkflowStateStore {
    pub fn new() -> Self {
        Self::with_options(
            DEFAULT_MAX_INLINE_ARTIFACT_BYTES,
            std::env::temp_dir()
                .join("davinci")
                .join("workflow_artifacts"),
        )
    }

    pub fn with_options(max_inline_bytes: usize, overflow_dir: PathBuf) -> Self {
        Self {
            max_inline_bytes,
            overflow_dir,
            artifacts: Arc::new(RwLock::new(HashMap::new())),
            phase_index: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Store an artifact produced by a worker.
    pub fn put_artifact(
        &self,
        workflow_id: WorkflowId,
        phase_id: &str,
        worker_id: AgentId,
        value: Value,
        schema: Option<Value>,
    ) -> Result<WorkflowArtifact, WorkflowStateError> {
        let serialized = serde_json::to_string(&value)
            .map_err(|e| WorkflowStateError::SerializationError(e.to_string()))?;
        let byte_size = serialized.len();
        let id = Uuid::now_v7();
        let now = now_ms();

        let (stored_value, is_overflow, overflow_path) = if byte_size > self.max_inline_bytes {
            // Write overflow artifact to disk
            let wf_dir = self.overflow_dir.join(workflow_id.to_string());
            if !wf_dir.exists() {
                std::fs::create_dir_all(&wf_dir)
                    .map_err(|e| WorkflowStateError::IoError(e.to_string()))?;
            }
            let file_path = wf_dir.join(format!("{id}.json"));
            std::fs::write(&file_path, &serialized)
                .map_err(|e| WorkflowStateError::IoError(e.to_string()))?;

            let reference = serde_json::json!({
                "$overflow_ref": id.to_string(),
                "phase": phase_id,
                "byte_size": byte_size,
                "file": file_path.to_string_lossy(),
            });
            (reference, true, Some(file_path))
        } else {
            (value, false, None)
        };

        let artifact = WorkflowArtifact {
            id,
            workflow_id,
            phase_id: phase_id.to_string(),
            worker_id,
            schema,
            value: stored_value,
            is_overflow,
            byte_size,
            overflow_path,
            created_ms: now,
        };

        {
            let mut arts = self.artifacts.write().unwrap();
            arts.insert(id, artifact.clone());
        }

        {
            let mut index = self.phase_index.write().unwrap();
            index
                .entry((workflow_id, phase_id.to_string()))
                .or_default()
                .push(id);
        }

        Ok(artifact)
    }

    /// Get an artifact by ID.
    pub fn get_artifact(&self, id: &Uuid) -> Option<WorkflowArtifact> {
        let arts = self.artifacts.read().unwrap();
        arts.get(id).cloned()
    }

    /// Retrieve the full artifact content even if stored in overflow.
    pub fn get_full_value(&self, id: &Uuid) -> Result<Value, WorkflowStateError> {
        let artifact = self
            .get_artifact(id)
            .ok_or(WorkflowStateError::ArtifactNotFound(*id))?;

        if artifact.is_overflow {
            if let Some(path) = &artifact.overflow_path {
                let content = std::fs::read_to_string(path)
                    .map_err(|e| WorkflowStateError::IoError(e.to_string()))?;
                let val: Value = serde_json::from_str(&content)
                    .map_err(|e| WorkflowStateError::SerializationError(e.to_string()))?;
                Ok(val)
            } else {
                Err(WorkflowStateError::ArtifactNotFound(*id))
            }
        } else {
            Ok(artifact.value)
        }
    }

    /// List all artifacts for a given workflow and phase.
    pub fn list_phase_artifacts(
        &self,
        workflow_id: WorkflowId,
        phase_id: &str,
    ) -> Vec<WorkflowArtifact> {
        let index = self.phase_index.read().unwrap();
        let arts = self.artifacts.read().unwrap();
        if let Some(ids) = index.get(&(workflow_id, phase_id.to_string())) {
            ids.iter().filter_map(|id| arts.get(id).cloned()).collect()
        } else {
            Vec::new()
        }
    }

    /// Format compact artifact references for injection into worker prompts.
    pub fn format_prompt_references(&self, workflow_id: WorkflowId, phase_ids: &[&str]) -> String {
        let mut summaries = Vec::new();
        for phase in phase_ids {
            let artifacts = self.list_phase_artifacts(workflow_id, phase);
            for art in artifacts {
                summaries.push(art.to_reference_summary());
            }
        }
        if summaries.is_empty() {
            String::new()
        } else {
            serde_json::to_string_pretty(&summaries).unwrap_or_default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_inline_artifact_storage_and_retrieval() {
        let tmp = tempdir().unwrap();
        let store = WorkflowStateStore::with_options(1024, tmp.path().to_path_buf());
        let wf_id = WorkflowId::new();
        let worker_id = AgentId::new();

        let val = serde_json::json!({"summary": "investigation findings", "count": 42});
        let artifact = store
            .put_artifact(wf_id, "investigate", worker_id, val.clone(), None)
            .unwrap();

        assert!(!artifact.is_overflow);
        assert_eq!(artifact.phase_id, "investigate");
        assert_eq!(artifact.worker_id, worker_id);

        let retrieved = store.get_artifact(&artifact.id).unwrap();
        assert_eq!(retrieved.value, val);

        let phase_arts = store.list_phase_artifacts(wf_id, "investigate");
        assert_eq!(phase_arts.len(), 1);
        assert_eq!(phase_arts[0].id, artifact.id);
    }

    #[test]
    fn test_large_1mb_artifact_overflows_and_does_not_bloat_prompt() {
        let tmp = tempdir().unwrap();
        // 16 KB inline limit
        let store = WorkflowStateStore::with_options(16 * 1024, tmp.path().to_path_buf());
        let wf_id = WorkflowId::new();
        let worker_id = AgentId::new();

        // Create ~1 MB string payload
        let large_string = "x".repeat(1024 * 1024);
        let val = serde_json::json!({
            "large_data": large_string,
            "status": "ok"
        });

        let artifact = store
            .put_artifact(wf_id, "data-gen", worker_id, val.clone(), None)
            .unwrap();

        // Must be flagged as overflow
        assert!(artifact.is_overflow);
        assert!(artifact.byte_size >= 1024 * 1024);
        assert!(artifact.overflow_path.is_some());
        assert!(artifact.overflow_path.as_ref().unwrap().exists());

        // Value in artifact is a compact overflow reference, not 1MB!
        let ref_str = serde_json::to_string(&artifact.value).unwrap();
        assert!(ref_str.len() < 500);

        // Prompt formatting produces a compact manifest, NOT the 1 MB data!
        let prompt_refs = store.format_prompt_references(wf_id, &["data-gen"]);
        assert!(prompt_refs.len() < 500);
        assert!(prompt_refs.contains("artifact_id"));
        assert!(prompt_refs.contains("size_bytes"));

        // Full value is still retrievable via get_full_value
        let full_val = store.get_full_value(&artifact.id).unwrap();
        assert_eq!(full_val["status"], "ok");
        assert_eq!(full_val["large_data"].as_str().unwrap().len(), 1024 * 1024);
    }
}
