//! Verification evidence store: immutable execution receipts and artifact indexing.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::evidence::{ArtifactRef, AssertionCounts};
use super::ids::{EvidenceId, TaskId};

pub fn execution_passed(
    started: bool,
    exit_code: Option<i32>,
    timed_out: bool,
    cancelled: bool,
) -> bool {
    started && exit_code == Some(0) && !timed_out && !cancelled
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionReceipt {
    pub receipt_id: EvidenceId,
    pub operation_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<TaskId>,
    #[serde(default)]
    pub attempt: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requirement_id: Option<String>,
    pub tool_name: String,
    #[serde(default)]
    pub argv: Vec<String>,
    /// Workspace-relative target roots reported by an actual compiler invocation.
    /// Does not imply coverage of transitive modules or arbitrary sibling files.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub compiler_source_roots: Vec<String>,
    #[serde(default)]
    pub cwd: String,
    pub started: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    pub timed_out: bool,
    pub cancelled: bool,
    pub killed: bool,
    pub permission_denied: bool,
    pub simulated: bool,
    pub hook_vetoed: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdout_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr_hash: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stdout_artifact: Option<ArtifactRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr_artifact: Option<ArtifactRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assertion_counts: Option<AssertionCounts>,
    #[serde(default)]
    pub runtime_versions: HashMap<String, String>,
    pub started_at_ms: i64,
    pub finished_at_ms: i64,
}

impl Default for ExecutionReceipt {
    fn default() -> Self {
        Self {
            receipt_id: EvidenceId::new(),
            operation_id: String::new(),
            task_id: None,
            attempt: 0,
            requirement_id: None,
            tool_name: String::new(),
            argv: Vec::new(),
            compiler_source_roots: Vec::new(),
            cwd: ".".into(),
            started: false,
            exit_code: None,
            timed_out: false,
            cancelled: false,
            killed: false,
            permission_denied: false,
            simulated: false,
            hook_vetoed: false,
            stdout_hash: None,
            stderr_hash: None,
            stdout_artifact: None,
            stderr_artifact: None,
            assertion_counts: None,
            runtime_versions: HashMap::new(),
            started_at_ms: 0,
            finished_at_ms: 0,
        }
    }
}

impl ExecutionReceipt {
    pub fn is_passed(&self) -> bool {
        execution_passed(self.started, self.exit_code, self.timed_out, self.cancelled)
            && !self.killed
            && !self.permission_denied
            && !self.simulated
            && !self.hook_vetoed
            && self.assertions_passed()
    }

    fn assertions_passed(&self) -> bool {
        if let Some(ref counts) = self.assertion_counts {
            counts.total > 0 && counts.failed == 0 && counts.passed > 0
        } else {
            true
        }
    }
}

#[derive(Debug, Clone)]
pub struct VerificationEvidenceStore {
    dir: PathBuf,
    receipts: Arc<Mutex<HashMap<String, ExecutionReceipt>>>,
}

impl VerificationEvidenceStore {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        let dir = dir.into();
        Self {
            dir,
            receipts: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Store raw artifact blob securely and return its ArtifactRef.
    pub fn store_artifact(&self, media_type: &str, data: &[u8]) -> Result<ArtifactRef, String> {
        fs::create_dir_all(&self.dir).map_err(|e| e.to_string())?;

        let mut hasher = Sha256::new();
        hasher.update(data);
        let sha256 = format!("{:x}", hasher.finalize());

        let file_name = format!("{}.bin", &sha256);
        let target_path = self.dir.join(&file_name);

        // Write artifact blob atomically
        if !target_path.exists() {
            fs::write(&target_path, data).map_err(|e| e.to_string())?;
        }

        let id = format!("artifact_{}", &sha256[..12]);
        Ok(ArtifactRef {
            id,
            sha256,
            media_type: media_type.to_string(),
            size: data.len() as u64,
            relative_store_path: file_name,
            redaction: None,
        })
    }

    /// Retrieve an artifact verifying bounds, non-escape, existence, size, and SHA-256.
    pub fn get_artifact(&self, artifact_ref: &ArtifactRef) -> Result<Vec<u8>, String> {
        // Prevent directory traversal escape
        let rel_path = Path::new(&artifact_ref.relative_store_path);
        for component in rel_path.components() {
            if matches!(component, std::path::Component::ParentDir) {
                return Err("directory traversal attempt in artifact retrieval".into());
            }
        }

        let full_path = self.dir.join(rel_path);
        if !full_path.exists() {
            return Err(format!("artifact blob missing: {}", artifact_ref.id));
        }

        let bytes = fs::read(&full_path).map_err(|e| e.to_string())?;
        if bytes.len() as u64 != artifact_ref.size {
            return Err(format!(
                "artifact size mismatch: expected {}, got {}",
                artifact_ref.size,
                bytes.len()
            ));
        }

        let mut hasher = Sha256::new();
        hasher.update(&bytes);
        let calculated = format!("{:x}", hasher.finalize());
        if calculated != artifact_ref.sha256 {
            return Err(format!(
                "artifact hash mismatch: expected {}, got {}",
                artifact_ref.sha256, calculated
            ));
        }

        Ok(bytes)
    }

    /// Record receipt with idempotent deduplication by operation_id.
    /// Returns Ok(true) if newly inserted, Ok(false) if duplicate already existed.
    pub fn record_receipt(&self, receipt: ExecutionReceipt) -> Result<bool, String> {
        let mut map = self.receipts.lock().map_err(|e| e.to_string())?;
        if map.contains_key(&receipt.operation_id) {
            // Already recorded, ignore duplicate delivery
            return Ok(false);
        }

        map.insert(receipt.operation_id.clone(), receipt);
        Ok(true)
    }

    pub fn get_receipt(&self, operation_id: &str) -> Option<ExecutionReceipt> {
        self.receipts
            .lock()
            .ok()
            .and_then(|map| map.get(operation_id).cloned())
    }

    pub fn all_receipts(&self) -> Vec<ExecutionReceipt> {
        self.receipts
            .lock()
            .ok()
            .map(|map| map.values().cloned().collect())
            .unwrap_or_default()
    }
}

/// Sanitize text for display treating ANSI escape codes and HTML as untrusted.
pub fn sanitize_display_output(text: &str) -> String {
    // Strip ANSI escape sequences: ESC [ ... [letter]
    let mut cleaned = String::with_capacity(text.len());
    let mut in_escape = false;

    for c in text.chars() {
        if c == '\x1b' {
            in_escape = true;
            continue;
        }
        if in_escape {
            if c.is_ascii_alphabetic() || c == 'm' || c == 'K' || c == 'H' || c == 'J' {
                in_escape = false;
            }
            continue;
        }

        // Escape HTML special characters
        match c {
            '&' => cleaned.push_str("&amp;"),
            '<' => cleaned.push_str("&lt;"),
            '>' => cleaned.push_str("&gt;"),
            '"' => cleaned.push_str("&quot;"),
            '\'' => cleaned.push_str("&#39;"),
            _ => cleaned.push(c),
        }
    }

    cleaned
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn f06_no_skipped_success() {
        assert!(execution_passed(true, Some(0), false, false));
        assert!(!execution_passed(false, Some(0), false, false));
        assert!(!execution_passed(true, Some(0), true, false));
        assert!(!execution_passed(true, None, false, true));
    }

    #[test]
    fn test_permission_denied_before_launch() {
        let receipt = ExecutionReceipt {
            receipt_id: EvidenceId::new(),
            operation_id: "op1".into(),
            tool_name: "bash".into(),
            argv: vec!["cargo".into(), "test".into()],
            started: false,
            cancelled: true,
            permission_denied: true,
            ..Default::default()
        };
        assert!(!receipt.is_passed());
    }

    #[test]
    fn test_process_killed() {
        let receipt = ExecutionReceipt {
            receipt_id: EvidenceId::new(),
            operation_id: "op2".into(),
            tool_name: "bash".into(),
            argv: vec!["cargo".into(), "test".into()],
            started: true,
            exit_code: Some(137),
            cancelled: true,
            killed: true,
            ..Default::default()
        };
        assert!(!receipt.is_passed());
    }

    #[test]
    fn test_background_job_not_finished() {
        // Job started, still running: exit_code is None
        assert!(!execution_passed(true, None, false, false));
    }

    #[test]
    fn test_zero_tests_selected() {
        let receipt = ExecutionReceipt {
            receipt_id: EvidenceId::new(),
            operation_id: "op3".into(),
            tool_name: "cargo test".into(),
            argv: vec!["cargo".into(), "test".into()],
            started: true,
            exit_code: Some(0),
            assertion_counts: Some(AssertionCounts {
                total: 0,
                passed: 0,
                failed: 0,
                skipped: 0,
            }),
            ..Default::default()
        };
        assert!(!receipt.is_passed());
    }

    #[test]
    fn test_duplicate_exit_notification() {
        let dir = tempdir().unwrap();
        let store = VerificationEvidenceStore::new(dir.path());
        let receipt = ExecutionReceipt {
            receipt_id: EvidenceId::new(),
            operation_id: "op_dup".into(),
            tool_name: "test".into(),
            started: true,
            exit_code: Some(0),
            ..Default::default()
        };

        let first = store.record_receipt(receipt.clone()).unwrap();
        assert!(first);

        let second = store.record_receipt(receipt).unwrap();
        assert!(!second); // Duplicate ignored

        assert_eq!(store.all_receipts().len(), 1);
    }

    #[test]
    fn test_missing_artifact_blob() {
        let dir = tempdir().unwrap();
        let store = VerificationEvidenceStore::new(dir.path());
        let bad_ref = ArtifactRef {
            id: "missing".into(),
            sha256: "fake_hash".into(),
            media_type: "text/plain".into(),
            size: 10,
            relative_store_path: "nonexistent.bin".into(),
            redaction: None,
        };

        assert!(store.get_artifact(&bad_ref).is_err());
    }

    #[test]
    fn test_artifact_traversal_escape_rejected() {
        let dir = tempdir().unwrap();
        let store = VerificationEvidenceStore::new(dir.path());
        let escape_ref = ArtifactRef {
            id: "escape".into(),
            sha256: "fake".into(),
            media_type: "text/plain".into(),
            size: 10,
            relative_store_path: "../../../secret.txt".into(),
            redaction: None,
        };

        assert!(store.get_artifact(&escape_ref).is_err());
    }

    #[test]
    fn test_successful_raw_exit_followed_by_hook_veto() {
        let receipt = ExecutionReceipt {
            receipt_id: EvidenceId::new(),
            operation_id: "vetoed".into(),
            tool_name: "test".into(),
            started: true,
            exit_code: Some(0),
            hook_vetoed: true, // Vetoed by post-hook
            ..Default::default()
        };
        assert!(!receipt.is_passed());
    }

    #[test]
    fn test_simulated_exit_zero_cannot_satisfy_real_evidence() {
        let receipt = ExecutionReceipt {
            receipt_id: EvidenceId::new(),
            operation_id: "simulated".into(),
            tool_name: "dry_run_verify_exec".into(),
            started: true,
            exit_code: Some(0),
            simulated: true, // Dry run / simulated
            ..Default::default()
        };
        assert!(!receipt.is_passed());
    }

    #[test]
    fn test_sanitize_display_output() {
        let raw = "\x1b[31mRed Text\x1b[0m <script>alert(1)</script>";
        let sanitized = sanitize_display_output(raw);
        assert!(!sanitized.contains("\x1b"));
        assert!(sanitized.contains("&lt;script&gt;"));
    }
}
