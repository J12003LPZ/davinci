//! Workflow artifact and state storage.
//!
//! Keeps intermediate worker results outside the main model context,
//! providing Governor-backed overflow storage for large artifacts.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};
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
    /// SHA-256 of the overflow file contents, checked on every full read.
    #[serde(default)]
    pub overflow_sha256: Option<String>,
    pub created_ms: i64,
}

fn read_lock<T>(lock: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    lock.read().unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn write_lock<T>(lock: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    lock.write()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", davinci_sys::hex::Lower(&Sha256::digest(bytes)))
}

/// Write `contents` so that `path` is either absent or complete: stage in a
/// sibling temp file, flush it to disk, then rename into place.
fn write_atomic(path: &Path, contents: &[u8]) -> std::io::Result<()> {
    let temp = path.with_extension(format!("{}.tmp", Uuid::new_v4()));
    let result = (|| {
        let mut file = std::fs::File::create(&temp)?;
        file.write_all(contents)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&temp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result
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

/// Per-workflow append-only record of every artifact's metadata (and inline
/// value), so a new store over the same directory can resume the workflow.
const MANIFEST_FILE: &str = "artifacts.jsonl";

/// Thread-safe artifact store. Large values overflow to their own file, and
/// every artifact is recorded in its workflow's manifest before it is
/// visible, so artifacts survive a restart (WOR-104).
#[derive(Debug, Clone)]
pub struct WorkflowStateStore {
    max_inline_bytes: usize,
    overflow_dir: PathBuf,
    artifacts: Arc<RwLock<HashMap<Uuid, WorkflowArtifact>>>,
    // (workflow_id, phase_id) -> list of artifact ids
    phase_index: Arc<RwLock<PhaseArtifactMap>>,
    /// Workflows whose manifest has been read into memory.
    loaded: Arc<RwLock<HashSet<WorkflowId>>>,
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
            loaded: Arc::new(RwLock::new(HashSet::new())),
        }
    }

    fn manifest_path(&self, workflow_id: WorkflowId) -> PathBuf {
        self.overflow_dir
            .join(workflow_id.to_string())
            .join(MANIFEST_FILE)
    }

    /// Read a workflow's manifest into memory once. A line that does not
    /// parse is skipped: only a crash mid-append leaves one, and only last.
    fn ensure_loaded(&self, workflow_id: WorkflowId) {
        if read_lock(&self.loaded).contains(&workflow_id) {
            return;
        }
        let mut loaded = write_lock(&self.loaded);
        if !loaded.insert(workflow_id) {
            return;
        }
        let Ok(file) = std::fs::File::open(self.manifest_path(workflow_id)) else {
            return;
        };
        // Same lock order as `put_artifact` and the readers: index, then artifacts.
        let mut index = write_lock(&self.phase_index);
        let mut arts = write_lock(&self.artifacts);
        for line in BufReader::new(file).lines().map_while(Result::ok) {
            let Ok(artifact) = serde_json::from_str::<WorkflowArtifact>(&line) else {
                continue;
            };
            if artifact.workflow_id != workflow_id || arts.contains_key(&artifact.id) {
                continue;
            }
            index
                .entry((workflow_id, artifact.phase_id.clone()))
                .or_default()
                .push(artifact.id);
            arts.insert(artifact.id, artifact);
        }
    }

    /// Load every workflow manifest under the store directory, for a lookup
    /// by artifact ID alone.
    fn load_all(&self) {
        let Ok(entries) = std::fs::read_dir(&self.overflow_dir) else {
            return;
        };
        for entry in entries.flatten() {
            if let Some(id) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<WorkflowId>().ok())
            {
                self.ensure_loaded(id);
            }
        }
    }

    fn append_manifest(&self, artifact: &WorkflowArtifact) -> Result<(), WorkflowStateError> {
        let io = |e: std::io::Error| WorkflowStateError::IoError(e.to_string());
        let path = self.manifest_path(artifact.workflow_id);
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(io)?;
        }
        let mut line = serde_json::to_vec(artifact)
            .map_err(|e| WorkflowStateError::SerializationError(e.to_string()))?;
        line.push(b'\n');
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .append(true)
            .open(&path)
            .map_err(io)?;
        // After a torn tail, start on a fresh line so this record stays whole.
        if file.metadata().map_err(io)?.len() > 0 {
            let mut last = [0u8; 1];
            file.seek(SeekFrom::End(-1)).map_err(io)?;
            file.read_exact(&mut last).map_err(io)?;
            if last[0] != b'\n' {
                line.insert(0, b'\n');
            }
        }
        file.write_all(&line).map_err(io)?;
        file.sync_data().map_err(io)
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
        // Earlier artifacts of this workflow keep their place ahead of it.
        self.ensure_loaded(workflow_id);
        let serialized = serde_json::to_string(&value)
            .map_err(|e| WorkflowStateError::SerializationError(e.to_string()))?;
        let byte_size = serialized.len();
        let id = Uuid::now_v7();
        let now = now_ms();

        let (stored_value, is_overflow, overflow_path, overflow_sha256) =
            if byte_size > self.max_inline_bytes {
                // Write overflow artifact to disk
                let wf_dir = self.overflow_dir.join(workflow_id.to_string());
                if !wf_dir.exists() {
                    std::fs::create_dir_all(&wf_dir)
                        .map_err(|e| WorkflowStateError::IoError(e.to_string()))?;
                }
                let file_path = wf_dir.join(format!("{id}.json"));
                write_atomic(&file_path, serialized.as_bytes())
                    .map_err(|e| WorkflowStateError::IoError(e.to_string()))?;

                let reference = serde_json::json!({
                    "$overflow_ref": id.to_string(),
                    "phase": phase_id,
                    "byte_size": byte_size,
                    "file": file_path.to_string_lossy(),
                });
                (
                    reference,
                    true,
                    Some(file_path),
                    Some(sha256_hex(serialized.as_bytes())),
                )
            } else {
                (value, false, None, None)
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
            overflow_sha256,
            created_ms: now,
        };

        if let Err(error) = self.append_manifest(&artifact) {
            if let Some(path) = &artifact.overflow_path {
                let _ = std::fs::remove_file(path);
            }
            return Err(error);
        }

        // Both maps change under both locks, in the same order readers use,
        // and a poisoned lock is recovered rather than panicking between the
        // two inserts and leaving an artifact the phase listing cannot see.
        {
            let mut index = write_lock(&self.phase_index);
            let mut arts = write_lock(&self.artifacts);
            arts.insert(id, artifact.clone());
            index
                .entry((workflow_id, phase_id.to_string()))
                .or_default()
                .push(id);
        }

        Ok(artifact)
    }

    /// Get an artifact by ID.
    pub fn get_artifact(&self, id: &Uuid) -> Option<WorkflowArtifact> {
        if let Some(artifact) = read_lock(&self.artifacts).get(id) {
            return Some(artifact.clone());
        }
        self.load_all();
        read_lock(&self.artifacts).get(id).cloned()
    }

    /// Retrieve the full artifact content even if stored in overflow.
    pub fn get_full_value(&self, id: &Uuid) -> Result<Value, WorkflowStateError> {
        let artifact = self
            .get_artifact(id)
            .ok_or(WorkflowStateError::ArtifactNotFound(*id))?;

        if artifact.is_overflow {
            if let Some(path) = &artifact.overflow_path {
                let bytes =
                    std::fs::read(path).map_err(|e| WorkflowStateError::IoError(e.to_string()))?;
                if let Some(expected) = &artifact.overflow_sha256 {
                    if &sha256_hex(&bytes) != expected {
                        return Err(WorkflowStateError::IoError(format!(
                            "overflow artifact {id} failed checksum verification"
                        )));
                    }
                }
                let val: Value = serde_json::from_slice(&bytes)
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
        self.ensure_loaded(workflow_id);
        let index = read_lock(&self.phase_index);
        let arts = read_lock(&self.artifacts);
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

    /// WOR-104: a new store over the same directory finds the artifacts of an
    /// earlier process, inline and overflow, in their original order.
    #[test]
    fn wor104_artifacts_survive_a_restart() {
        let tmp = tempdir().unwrap();
        let wf_id = WorkflowId::new();
        let worker = AgentId::new();
        let (small, large) = {
            let store = WorkflowStateStore::with_options(64, tmp.path().to_path_buf());
            let small = store
                .put_artifact(wf_id, "plan", worker, serde_json::json!({"step": 1}), None)
                .unwrap();
            let large = store
                .put_artifact(
                    wf_id,
                    "plan",
                    worker,
                    serde_json::json!({"blob": "y".repeat(500)}),
                    None,
                )
                .unwrap();
            assert!(large.is_overflow);
            (small, large)
        };

        let restarted = WorkflowStateStore::with_options(64, tmp.path().to_path_buf());
        let ids: Vec<_> = restarted
            .list_phase_artifacts(wf_id, "plan")
            .into_iter()
            .map(|artifact| artifact.id)
            .collect();
        assert_eq!(ids, [small.id, large.id]);
        assert_eq!(
            restarted.get_full_value(&large.id).unwrap()["blob"],
            "y".repeat(500)
        );

        // A lookup by ID alone finds it too, and new artifacts go after.
        let by_id = WorkflowStateStore::with_options(64, tmp.path().to_path_buf());
        assert_eq!(by_id.get_artifact(&small.id).unwrap(), small);
        let next = by_id
            .put_artifact(wf_id, "plan", worker, serde_json::json!({"step": 2}), None)
            .unwrap();
        let ids: Vec<_> = by_id
            .list_phase_artifacts(wf_id, "plan")
            .into_iter()
            .map(|artifact| artifact.id)
            .collect();
        assert_eq!(ids, [small.id, large.id, next.id]);
    }

    /// A crash mid-append leaves a torn last line; the rest still loads.
    #[test]
    fn wor104_torn_manifest_tail_is_skipped() {
        let tmp = tempdir().unwrap();
        let wf_id = WorkflowId::new();
        let store = WorkflowStateStore::with_options(1024, tmp.path().to_path_buf());
        let kept = store
            .put_artifact(wf_id, "p", AgentId::new(), serde_json::json!(1), None)
            .unwrap();
        let manifest = store.manifest_path(wf_id);
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&manifest)
            .unwrap();
        file.write_all(b"{\"id\":\"trunc").unwrap();
        drop(file);
        let restarted = WorkflowStateStore::with_options(1024, tmp.path().to_path_buf());
        assert_eq!(restarted.list_phase_artifacts(wf_id, "p"), [kept.clone()]);

        // The next append starts a fresh line instead of joining the torn one.
        let next = restarted
            .put_artifact(wf_id, "p", AgentId::new(), serde_json::json!(2), None)
            .unwrap();
        let again = WorkflowStateStore::with_options(1024, tmp.path().to_path_buf());
        assert_eq!(again.list_phase_artifacts(wf_id, "p"), [kept, next]);
    }

    #[test]
    fn wor102_poisoned_phase_index_does_not_orphan_artifact() {
        let tmp = tempdir().unwrap();
        let store = WorkflowStateStore::with_options(1024, tmp.path().to_path_buf());
        let index = Arc::clone(&store.phase_index);
        let _ = std::thread::spawn(move || {
            let _guard = index.write().unwrap();
            panic!("poison the phase index");
        })
        .join();
        assert!(store.phase_index.is_poisoned());

        let wf_id = WorkflowId::new();
        let artifact = store
            .put_artifact(
                wf_id,
                "p",
                AgentId::new(),
                serde_json::json!({"a": 1}),
                None,
            )
            .expect("poisoned lock must not abort the insert");
        let listed = store.list_phase_artifacts(wf_id, "p");
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, artifact.id);
        assert!(store.get_artifact(&artifact.id).is_some());
    }

    #[test]
    fn wor103_overflow_write_leaves_only_the_final_file() {
        let tmp = tempdir().unwrap();
        let store = WorkflowStateStore::with_options(16, tmp.path().to_path_buf());
        let wf_id = WorkflowId::new();
        let val = serde_json::json!({"data": "y".repeat(256)});
        let artifact = store
            .put_artifact(wf_id, "p", AgentId::new(), val.clone(), None)
            .unwrap();
        let dir = artifact.overflow_path.as_ref().unwrap().parent().unwrap();
        let names: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .filter(|name| name != MANIFEST_FILE)
            .collect();
        assert_eq!(names, vec![format!("{}.json", artifact.id)]);
        assert_eq!(store.get_full_value(&artifact.id).unwrap(), val);
    }

    #[test]
    fn wor103_tampered_overflow_file_is_rejected_on_read() {
        let tmp = tempdir().unwrap();
        let store = WorkflowStateStore::with_options(16, tmp.path().to_path_buf());
        let val = serde_json::json!({"data": "y".repeat(256)});
        let artifact = store
            .put_artifact(WorkflowId::new(), "p", AgentId::new(), val, None)
            .unwrap();
        // Still valid JSON, but not what was written.
        std::fs::write(
            artifact.overflow_path.as_ref().unwrap(),
            br#"{"data":"other"}"#,
        )
        .unwrap();
        assert!(matches!(
            store.get_full_value(&artifact.id),
            Err(WorkflowStateError::IoError(_))
        ));
    }
}
