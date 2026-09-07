//! User false-positive feedback expires when the cited control source changes.

use super::snapshot::Snapshot;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ControlFeedback {
    pub fingerprint: String,
    pub control_path: String,
    pub control_hash: String,
    #[serde(default = "super::validation::default_side")]
    pub snapshot_side: String,
}

impl ControlFeedback {
    pub fn still_valid(&self, snapshot: &Snapshot) -> bool {
        snapshot
            .file(&self.control_path, &self.snapshot_side)
            .map(|file| file.hash == self.control_hash)
            .unwrap_or(false)
    }
}

/// Stale feedback cannot suppress a live finding after the control bytes change.
pub fn suppressions_still_valid<'a>(
    records: &'a [ControlFeedback],
    fingerprint: &str,
    snapshot: &Snapshot,
) -> Vec<&'a ControlFeedback> {
    records
        .iter()
        .filter(|record| record.fingerprint == fingerprint && record.still_valid(snapshot))
        .collect()
}

/// Drop suppressions whose cited control bytes no longer match the snapshot.
pub fn expire_stale_suppressions(
    candidates: &mut [serde_json::Value],
    records: &[ControlFeedback],
    snapshot: &Snapshot,
) {
    if records.is_empty() {
        return;
    }
    for candidate in candidates {
        if candidate["assessment"]["disposition"] != "suppressed" {
            continue;
        }
        let Some(fingerprint) = candidate["fingerprint"]["value"].as_str() else {
            continue;
        };
        if records
            .iter()
            .any(|record| record.fingerprint == fingerprint)
            && suppressions_still_valid(records, fingerprint, snapshot).is_empty()
        {
            candidate["assessment"]["disposition"] = serde_json::json!("deferred");
            candidate["assessment"]["reason"] =
                serde_json::json!("prior feedback expired after control change");
            candidate["assessment"]["proofGaps"] =
                serde_json::json!(["control changed since feedback"]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::{command::ScanCommand, config::ScanConfig, snapshot::Snapshot};
    use super::*;

    #[test]
    fn security_feedback_expires_after_control_change() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("access.rs");
        std::fs::write(&path, "fn allow() { false }\n").unwrap();
        let snapshot = Snapshot::capture(
            dir.path(),
            &ScanCommand::parse("").unwrap(),
            &ScanConfig::default(),
        )
        .unwrap();
        let hash = snapshot.file("access.rs", "worktree").unwrap().hash.clone();
        let feedback = ControlFeedback {
            fingerprint: "root-1".into(),
            control_path: "access.rs".into(),
            control_hash: hash,
            snapshot_side: "worktree".into(),
        };
        assert_eq!(
            suppressions_still_valid(&[feedback.clone()], "root-1", &snapshot).len(),
            1
        );
        std::fs::write(
            &path,
            "fn allow(owner: u64, actor: u64) { owner == actor }\n",
        )
        .unwrap();
        let changed = Snapshot::capture(
            dir.path(),
            &ScanCommand::parse("").unwrap(),
            &ScanConfig::default(),
        )
        .unwrap();
        assert!(suppressions_still_valid(&[feedback.clone()], "root-1", &changed).is_empty());
        let mut suppressed = serde_json::json!({"fingerprint":{"value":"root-1"},"assessment":{"disposition":"suppressed"}});
        expire_stale_suppressions(std::slice::from_mut(&mut suppressed), &[feedback], &changed);
        assert_eq!(suppressed["assessment"]["disposition"], "deferred");
    }
}
