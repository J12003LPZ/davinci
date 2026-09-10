//! Immutable graph history, attempt lineage, and checkpoint retention.

use crate::native_extensions::graph::replay::{replay_compatible, ReplayFingerprint};
use crate::native_extensions::graph::store::{atomic_write, is_safe_run_id, run_dir};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphHistoryEntry {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    pub run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_checkpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attempt_refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub evidence_refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub root_budget_id: Option<String>,
    pub state_revision: u64,
    #[serde(default)]
    pub timestamp: u64,
}

#[allow(dead_code)]
pub fn exact_replay(
    _old_status: &str,
    _new_status: &str,
    old_content: &str,
    new_content: &str,
    other_bindings_match: bool,
) -> bool {
    old_content == new_content && other_bindings_match
}

#[allow(dead_code)]
pub fn history_dir(cwd: &Path, run_id: &str) -> PathBuf {
    run_dir(cwd, run_id).join("history")
}

#[allow(dead_code)]
pub fn write_history_entry(cwd: &Path, entry: &GraphHistoryEntry) -> std::io::Result<()> {
    if !is_safe_run_id(&entry.run_id) || !is_safe_run_id(&entry.id) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "unsafe run_id or entry_id",
        ));
    }
    let path = history_dir(cwd, &entry.run_id).join(format!("{}.json", entry.id));
    let content = serde_json::to_vec_pretty(entry)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    atomic_write(&path, &content)
}

#[allow(dead_code)]
pub fn load_history_entry(cwd: &Path, run_id: &str, entry_id: &str) -> Option<GraphHistoryEntry> {
    if !is_safe_run_id(run_id) || !is_safe_run_id(entry_id) {
        return None;
    }
    let path = history_dir(cwd, run_id).join(format!("{entry_id}.json"));
    let raw = fs::read_to_string(path).ok()?;
    serde_json::from_str(&raw).ok()
}

#[allow(dead_code)]
pub fn list_history_entries(cwd: &Path, run_id: &str) -> Vec<GraphHistoryEntry> {
    if !is_safe_run_id(run_id) {
        return Vec::new();
    }
    let dir = history_dir(cwd, run_id);
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut list = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("json") {
            if let Ok(raw) = fs::read_to_string(&path) {
                if let Ok(h) = serde_json::from_str::<GraphHistoryEntry>(&raw) {
                    list.push(h);
                }
            }
        }
    }
    list.sort_by(|a, b| {
        a.state_revision
            .cmp(&b.state_revision)
            .then_with(|| a.timestamp.cmp(&b.timestamp))
    });
    list
}

#[allow(dead_code)]
pub fn can_replay_task(
    stored_fingerprint: Option<&ReplayFingerprint>,
    current_fingerprint: &ReplayFingerprint,
) -> bool {
    match stored_fingerprint {
        Some(stored) => replay_compatible(stored, current_fingerprint),
        None => false,
    }
}

#[allow(dead_code)]
pub fn record_task_checkpoint(
    cwd: &Path,
    run_id: &str,
    task_id: &str,
    checkpoint_id: &str,
) -> std::io::Result<()> {
    if !is_safe_run_id(run_id) || !is_safe_run_id(task_id) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "unsafe run_id or task_id",
        ));
    }
    let dir = run_dir(cwd, run_id).join("checkpoints");
    let _ = fs::create_dir_all(&dir);
    let path = dir.join(format!("{task_id}.txt"));
    atomic_write(&path, checkpoint_id.as_bytes())
}

#[allow(dead_code)]
pub fn get_task_checkpoint(cwd: &Path, run_id: &str, task_id: &str) -> Option<String> {
    if !is_safe_run_id(run_id) || !is_safe_run_id(task_id) {
        return None;
    }
    let path = run_dir(cwd, run_id)
        .join("checkpoints")
        .join(format!("{task_id}.txt"));
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

#[allow(dead_code)]
pub fn find_before_writer_checkpoint(
    cwd: &Path,
    run_id: &str,
    writer_task_id: &str,
) -> Option<String> {
    get_task_checkpoint(cwd, run_id, writer_task_id).or_else(|| {
        let entries = list_history_entries(cwd, run_id);
        for entry in entries.iter().rev() {
            if let Some(cp) = &entry.task_checkpoint {
                return Some(cp.clone());
            }
        }
        None
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_extensions::graph::replay::{
        compute_repo_state_hash, incompatibility_reason,
    };
    use crate::native_extensions::graph::store::{create_run_dir, load_run, save_run};
    use crate::native_extensions::graph::types::{
        GraphBudgets, GraphCounters, GraphLifecycle, GraphRun, Phase,
    };
    use tempfile::tempdir;

    fn sample_run(cwd: &Path, run_id: &str, goal: &str) -> GraphRun {
        GraphRun {
            version: 1,
            run_id: run_id.to_string(),
            goal: goal.to_string(),
            cwd: cwd.to_string_lossy().into_owned(),
            phase: Phase::Classify,
            forced: None,
            dry_run: false,
            execution_origin: None,
            definition_digest: None,
            saved_definition: None,
            definition: None,
            classification: None,
            milestones: None,
            current_milestone: None,
            tasks: Vec::new(),
            verification: None,
            verification_bundle: None,
            review_coverage: None,
            budgets: GraphBudgets::default(),
            counters: GraphCounters {
                workers_spawned: 0,
                revision_cycles: 0,
                replans: 0,
                cost_usd: 0.0,
                started_at: 0,
            },
            blocked_reason: None,
            resource_snapshot: None,
            ecosystem_stats: Default::default(),
            updated_at: 0,
            lifecycle: None,
            revision: 0,
        }
    }

    fn sample_fingerprint() -> ReplayFingerprint {
        ReplayFingerprint {
            graph_version: 1,
            config_hash: "config_abc".into(),
            repo_state_hash: "repo_123".into(),
            input_hash: "input_xyz".into(),
            contract_hash: "contract_789".into(),
            definition_digest: None,
            simulated: None,
            model_id: None,
        }
    }

    #[test]
    fn f14_replay_content_not_status() {
        assert!(!exact_replay("dirty", "dirty", "bytes-a", "bytes-b", true));
        assert!(exact_replay("dirty", "dirty", "bytes-a", "bytes-a", true));
        assert!(!exact_replay("clean", "clean", "bytes-a", "bytes-a", false));
    }

    #[test]
    fn f14_same_head_status_different_bytes() {
        // Same status ("dirty" -> "dirty"), but different content bytes
        assert!(!exact_replay(
            "dirty",
            "dirty",
            "fn run() { 1 }",
            "fn run() { 2 }",
            true
        ));
        assert!(exact_replay(
            "dirty",
            "dirty",
            "fn run() { 1 }",
            "fn run() { 1 }",
            true
        ));
    }

    #[test]
    fn f14_same_size_nested_non_git_edit() {
        let dir = tempdir().unwrap();
        let nested_dir = dir.path().join("nested").join("sub");
        fs::create_dir_all(&nested_dir).unwrap();
        let target_file = nested_dir.join("config.txt");

        // Write 8 bytes: "12345678"
        fs::write(&target_file, b"12345678").unwrap();
        let hash_a = compute_repo_state_hash(dir.path());

        // Overwrite with exact same length (8 bytes), different content: "abcdefgh"
        fs::write(&target_file, b"abcdefgh").unwrap();
        let hash_b = compute_repo_state_hash(dir.path());

        assert_ne!(
            hash_a, hash_b,
            "Nested edit of identical size must yield a different repo state hash"
        );
    }

    #[test]
    fn f14_missing_prior_fingerprint() {
        let current_fp = sample_fingerprint();
        assert!(
            !can_replay_task(None, &current_fp),
            "Missing prior fingerprint must force reexecution"
        );
        assert!(can_replay_task(Some(&current_fp), &current_fp));
    }

    #[test]
    fn f14_corrupt_history() {
        let dir = tempdir().unwrap();
        let run_id = "run-history-corrupt";
        create_run_dir(dir.path(), run_id).unwrap();

        // Write a valid history entry
        let valid_entry = GraphHistoryEntry {
            id: "entry-001".into(),
            parent_id: None,
            run_id: run_id.into(),
            definition_digest: Some("digest1".into()),
            task_checkpoint: Some("chk1".into()),
            attempt_refs: vec!["att1".into()],
            evidence_refs: vec!["ev1".into()],
            root_budget_id: None,
            state_revision: 1,
            timestamp: 100,
        };
        write_history_entry(dir.path(), &valid_entry).unwrap();

        // Write a corrupt history entry
        let corrupt_path = history_dir(dir.path(), run_id).join("entry-002.json");
        fs::write(&corrupt_path, "{ broken invalid json ...").unwrap();

        // Reading corrupt entry returns None safely
        assert!(load_history_entry(dir.path(), run_id, "entry-002").is_none());

        // list_history_entries skips corrupt entry and returns valid ones
        let entries = list_history_entries(dir.path(), run_id);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, "entry-001");
    }

    #[test]
    fn f14_parent_retention() {
        let dir = tempdir().unwrap();
        let parent_id = "run-parent-100";
        create_run_dir(dir.path(), parent_id).unwrap();
        let mut parent_run = sample_run(dir.path(), parent_id, "parent");
        parent_run.phase = Phase::Done;
        parent_run.lifecycle = Some(GraphLifecycle::Stopped);
        parent_run.updated_at = 10;
        save_run(&mut parent_run).unwrap();

        // Child run references parent in history
        let child_id = "run-child-200";
        create_run_dir(dir.path(), child_id).unwrap();
        let mut child_run = sample_run(dir.path(), child_id, "child");
        child_run.phase = Phase::Classify;
        child_run.updated_at = 20;
        save_run(&mut child_run).unwrap();

        let history_entry = GraphHistoryEntry {
            id: "fork-001".into(),
            parent_id: Some(parent_id.into()),
            run_id: child_id.into(),
            definition_digest: None,
            task_checkpoint: None,
            attempt_refs: Vec::new(),
            evidence_refs: Vec::new(),
            root_budget_id: None,
            state_revision: 1,
            timestamp: 20,
        };
        write_history_entry(dir.path(), &history_entry).unwrap();

        // Create 25 finished unreferenced runs to trigger pruning
        for i in 1..=25 {
            let id = format!("run-prune-{:03}", i);
            create_run_dir(dir.path(), &id).unwrap();
            let mut r = sample_run(dir.path(), &id, &format!("prune {i}"));
            r.phase = Phase::Done;
            r.lifecycle = Some(GraphLifecycle::Stopped);
            r.updated_at = 100 + i as u64;
            save_run(&mut r).unwrap();
        }

        // Parent must be retained because child history entry references it as parent_id
        create_run_dir(dir.path(), "run-trigger-gc").unwrap();
        assert!(
            run_dir(dir.path(), parent_id).exists(),
            "Parent referenced in history must be retained"
        );
    }

    #[test]
    fn f14_crash_after_history_before_projection() {
        let dir = tempdir().unwrap();
        let run_id = "run-crash-test";
        create_run_dir(dir.path(), run_id).unwrap();

        let mut run = sample_run(dir.path(), run_id, "initial revision");
        run.revision = 1;
        save_run(&mut run).unwrap();

        // Step 1: Persist history entry for revision 2 BEFORE switching active projection
        let entry_rev2 = GraphHistoryEntry {
            id: "rev-2-entry".into(),
            parent_id: None,
            run_id: run_id.into(),
            definition_digest: Some("def-digest-rev2".into()),
            task_checkpoint: Some("chk-writer".into()),
            attempt_refs: vec![],
            evidence_refs: vec![],
            root_budget_id: None,
            state_revision: 2,
            timestamp: 200,
        };
        write_history_entry(dir.path(), &entry_rev2).unwrap();

        // SIMULATED CRASH: process terminates before `state.json` can be overwritten with revision 2!
        // On restart, state.json is still at revision 1
        let loaded = load_run(dir.path(), run_id).expect("loaded");
        assert_eq!(
            loaded.revision, 1,
            "Active projection must still be prior revision"
        );

        // But the history ledger recorded revision 2 and remains durable
        let history = list_history_entries(dir.path(), run_id);
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].state_revision, 2);
    }

    #[test]
    fn f14_model_policy_changed() {
        let mut fp1 = sample_fingerprint();
        fp1.model_id = Some("gpt-4".into());
        let mut fp2 = sample_fingerprint();
        fp2.model_id = Some("claude-3-5".into());

        assert!(!replay_compatible(&fp1, &fp2));
        let reason = incompatibility_reason(&fp1, &fp2).unwrap();
        assert!(reason.contains("model changed"));

        // Policy change (config_hash)
        let mut fp3 = fp1.clone();
        fp3.config_hash = "config_different".into();
        assert!(!replay_compatible(&fp1, &fp3));
        let policy_reason = incompatibility_reason(&fp1, &fp3).unwrap();
        assert!(policy_reason.contains("config hash changed"));
    }

    #[test]
    fn f14_simulated_artifact_not_replayed_as_real() {
        let mut fp_stored = sample_fingerprint();
        fp_stored.simulated = Some(true);

        let fp_current_real = sample_fingerprint();
        assert!(!replay_compatible(&fp_stored, &fp_current_real));
        let reason = incompatibility_reason(&fp_stored, &fp_current_real).unwrap();
        assert!(reason.contains("cannot replay simulated artifact"));
    }
}
