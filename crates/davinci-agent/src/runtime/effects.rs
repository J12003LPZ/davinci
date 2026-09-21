//! Task-owned effect identity and attribution.

use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;

use super::ids::{AgentId, TaskId};

/// Effect reports cross the graph worker process boundary as bounded JSONL.
pub const MAX_EFFECT_REPORT_BYTES: usize = 128 * 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileEffectKind {
    Created,
    Modified,
    Deleted,
    ModeChanged,
    Unattributed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnedFileEffect {
    pub operation_id: String,
    pub actor: AgentId,
    pub owner_generation: u64,
    pub path: String,
    pub kind: FileEffectKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_blob: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_blob: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_mode: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_mode: Option<u32>,
    pub task_id: TaskId,
}

impl OwnedFileEffect {
    pub fn new(
        operation_id: impl Into<String>,
        actor: AgentId,
        owner_generation: u64,
        path: impl Into<String>,
        kind: FileEffectKind,
        task_id: TaskId,
    ) -> Self {
        Self {
            operation_id: operation_id.into(),
            actor,
            owner_generation,
            path: path.into(),
            kind,
            before_blob: None,
            after_blob: None,
            before_mode: None,
            after_mode: None,
            task_id,
        }
    }
}

/// A task-owned effect plus the exact pre/post bytes needed to restore it.
/// Hashes remain on `OwnedFileEffect`; bytes are optional for mode-only or
/// externally persisted effects and are validated whenever present.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnedFileEffectReport {
    pub effect: OwnedFileEffect,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before_bytes: Option<Vec<u8>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after_bytes: Option<Vec<u8>>,
}

fn validate_report_bytes(
    label: &str,
    expected_hash: Option<&str>,
    bytes: Option<&[u8]>,
) -> Result<(), String> {
    let Some(bytes) = bytes else {
        return Ok(());
    };
    if bytes.len() > super::checkpoints::MAX_BLOB_BYTES {
        return Err(format!(
            "{label} exceeds the {} byte blob limit",
            super::checkpoints::MAX_BLOB_BYTES
        ));
    }
    let Some(expected_hash) = expected_hash else {
        return Err(format!(
            "{label} supplied without a corresponding blob hash"
        ));
    };
    let actual = super::checkpoints::compute_sha256(bytes);
    if actual != expected_hash {
        return Err(format!(
            "{label} hash mismatch: expected {expected_hash}, got {actual}"
        ));
    }
    Ok(())
}

/// Append one validated effect report and durably flush it to disk.
pub fn append_effect_report(
    path: &Path,
    effect: &OwnedFileEffect,
    before_bytes: Option<&[u8]>,
    after_bytes: Option<&[u8]>,
) -> Result<(), String> {
    validate_report_bytes("before_bytes", effect.before_blob.as_deref(), before_bytes)?;
    validate_report_bytes("after_bytes", effect.after_blob.as_deref(), after_bytes)?;
    let report = OwnedFileEffectReport {
        effect: effect.clone(),
        before_bytes: before_bytes.map(ToOwned::to_owned),
        after_bytes: after_bytes.map(ToOwned::to_owned),
    };
    let mut line =
        serde_json::to_vec(&report).map_err(|e| format!("serialize effect report: {e}"))?;
    line.push(b'\n');
    let current_len = fs::metadata(path)
        .map(|meta| meta.len() as usize)
        .unwrap_or(0);
    if current_len.saturating_add(line.len()) > MAX_EFFECT_REPORT_BYTES {
        return Err(format!(
            "effect report exceeds the {} byte limit",
            MAX_EFFECT_REPORT_BYTES
        ));
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|e| format!("create effect report directory: {e}"))?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("open effect report: {e}"))?;
    file.write_all(&line)
        .and_then(|_| file.sync_all())
        .map_err(|e| format!("persist effect report: {e}"))?;
    #[cfg(unix)]
    fs::File::open(parent)
        .and_then(|directory| directory.sync_all())
        .map_err(|e| format!("persist effect report directory: {e}"))?;
    Ok(())
}

/// Read and validate a bounded JSONL effect report.
pub fn read_effect_report(path: &Path) -> Result<Vec<OwnedFileEffectReport>, String> {
    let raw = fs::read(path).map_err(|e| format!("read effect report: {e}"))?;
    if raw.len() > MAX_EFFECT_REPORT_BYTES {
        return Err(format!(
            "effect report exceeds the {} byte limit",
            MAX_EFFECT_REPORT_BYTES
        ));
    }
    let mut reports = Vec::new();
    for (line_no, line) in raw.split(|byte| *byte == b'\n').enumerate() {
        if line.iter().all(u8::is_ascii_whitespace) {
            continue;
        }
        let report: OwnedFileEffectReport = serde_json::from_slice(line)
            .map_err(|e| format!("invalid effect report line {}: {e}", line_no + 1))?;
        validate_report_bytes(
            "before_bytes",
            report.effect.before_blob.as_deref(),
            report.before_bytes.as_deref(),
        )?;
        validate_report_bytes(
            "after_bytes",
            report.effect.after_blob.as_deref(),
            report.after_bytes.as_deref(),
        )?;
        reports.push(report);
    }
    Ok(reports)
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalEffectReceipt {
    pub receipt_id: String,
    pub task_id: TaskId,
    pub operation_id: String,
    pub kind: String,
    #[serde(default)]
    pub details: serde_json::Value,
    pub timestamp_ms: u64,
    pub reversible: bool,
}

impl ExternalEffectReceipt {
    pub fn redacted_details(&self) -> serde_json::Value {
        redact_external_details(&self.details)
    }
}

pub fn redact_external_details(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(map) => {
            let mut out = serde_json::Map::new();
            for (k, v) in map {
                let lower = k.to_lowercase();
                if lower.contains("token")
                    || lower.contains("secret")
                    || lower.contains("key")
                    || lower.contains("password")
                    || lower.contains("auth")
                    || lower.contains("credential")
                {
                    out.insert(k.clone(), serde_json::Value::String("[REDACTED]".into()));
                } else {
                    out.insert(k.clone(), redact_external_details(v));
                }
            }
            serde_json::Value::Object(out)
        }
        serde_json::Value::Array(arr) => {
            serde_json::Value::Array(arr.iter().map(redact_external_details).collect())
        }
        other => other.clone(),
    }
}

pub fn effect_rewind_action(kind: &str, owned: bool) -> &'static str {
    if kind == "local_file" && owned {
        "restore"
    } else {
        "disclose_only"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_owned_file_effect_construction() {
        let task_id = TaskId::new();
        let actor = AgentId::new();
        let effect = OwnedFileEffect::new(
            "op_1",
            actor,
            1,
            "src/main.rs",
            FileEffectKind::Modified,
            task_id,
        );
        assert_eq!(effect.path, "src/main.rs");
        assert_eq!(effect.kind, FileEffectKind::Modified);
        assert_eq!(effect.owner_generation, 1);
    }

    #[test]
    fn test_secret_redaction_in_external_effect() {
        let details = serde_json::json!({
            "service": "aws",
            "api_key": "AKIASECRET123",
            "auth_token": "bearer xyz",
            "region": "us-west-2"
        });
        let redacted = redact_external_details(&details);
        assert_eq!(redacted["api_key"], "[REDACTED]");
        assert_eq!(redacted["auth_token"], "[REDACTED]");
        assert_eq!(redacted["service"], "aws");
        assert_eq!(redacted["region"], "us-west-2");
    }

    #[test]
    fn effect_report_round_trips_and_validates_hashes() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("effects.jsonl");
        let task_id = TaskId::new();
        let before = b"before";
        let after = b"after";
        let mut effect = OwnedFileEffect::new(
            "op-report",
            AgentId::new(),
            1,
            "src/lib.rs",
            FileEffectKind::Modified,
            task_id,
        );
        effect.before_blob = Some(crate::runtime::checkpoints::compute_sha256(before));
        effect.after_blob = Some(crate::runtime::checkpoints::compute_sha256(after));
        append_effect_report(&path, &effect, Some(before), Some(after)).unwrap();

        let reports = read_effect_report(&path).unwrap();
        assert_eq!(reports.len(), 1);
        assert_eq!(reports[0].effect, effect);
        assert_eq!(reports[0].before_bytes.as_deref(), Some(before.as_slice()));
        assert_eq!(reports[0].after_bytes.as_deref(), Some(after.as_slice()));
    }

    #[test]
    fn effect_report_rejects_hash_mismatch() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("effects.jsonl");
        let task_id = TaskId::new();
        let mut effect = OwnedFileEffect::new(
            "op-report",
            AgentId::new(),
            1,
            "src/lib.rs",
            FileEffectKind::Modified,
            task_id,
        );
        effect.before_blob = Some(crate::runtime::checkpoints::compute_sha256(b"before"));
        assert!(append_effect_report(&path, &effect, Some(b"tampered"), None).is_err());
        assert!(!path.exists());
    }
}
