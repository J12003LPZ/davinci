//! Replay compatibility fingerprints and verification for graph runs.
//!
//! Replaying a cached or previously-executed node requires that the execution
//! environment and inputs match:
//! - graph version (topology/runtime version)
//! - configuration hash (`.pi/graph.json`)
//! - repository state hash (git HEAD + uncommitted status, or dir contents)
//! - canonical task input hash (worker briefing / dependencies / goal)
//! - contract schema hash for the expected artifact kind

use super::store::CONFIG_DIR;
use super::types::ArtifactKind;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::Path;
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ReplayFingerprint {
    pub graph_version: u32,
    pub config_hash: String,
    pub repo_state_hash: String,
    pub input_hash: String,
    pub contract_hash: String,
}

impl ReplayFingerprint {
    pub fn for_task(cwd: &Path, graph_version: u32, briefing: &str, expect: ArtifactKind) -> Self {
        Self {
            graph_version,
            config_hash: compute_config_hash(cwd),
            repo_state_hash: compute_repo_state_hash(cwd),
            input_hash: compute_input_hash(briefing),
            contract_hash: compute_contract_hash(expect),
        }
    }
}

pub fn replay_compatible(stored: &ReplayFingerprint, current: &ReplayFingerprint) -> bool {
    stored == current
}

pub fn incompatibility_reason(
    stored: &ReplayFingerprint,
    current: &ReplayFingerprint,
) -> Option<String> {
    if stored.graph_version != current.graph_version {
        return Some(format!(
            "graph version changed: stored={}, current={}",
            stored.graph_version, current.graph_version
        ));
    }
    if stored.config_hash != current.config_hash {
        return Some(format!(
            "config hash changed: stored={}, current={}",
            stored.config_hash, current.config_hash
        ));
    }
    if stored.repo_state_hash != current.repo_state_hash {
        return Some(format!(
            "repo state changed: stored={}, current={}",
            stored.repo_state_hash, current.repo_state_hash
        ));
    }
    if stored.contract_hash != current.contract_hash {
        return Some(format!(
            "contract hash changed: stored={}, current={}",
            stored.contract_hash, current.contract_hash
        ));
    }
    if stored.input_hash != current.input_hash {
        return Some(format!(
            "input hash changed: stored={}, current={}",
            stored.input_hash, current.input_hash
        ));
    }
    None
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let result = hasher.finalize();
    let mut s = String::with_capacity(result.len() * 2);
    for b in result {
        use std::fmt::Write;
        let _ = write!(s, "{:02x}", b);
    }
    s
}

pub fn compute_input_hash(canonical_input: &str) -> String {
    sha256_hex(canonical_input.as_bytes())
}

pub fn compute_contract_hash(expect: ArtifactKind) -> String {
    let schema = super::validate::artifact_schema(expect);
    let canonical = serde_json::to_string(&schema).unwrap_or_default();
    sha256_hex(canonical.as_bytes())
}

pub fn compute_config_hash(cwd: &Path) -> String {
    let config_path = cwd.join(CONFIG_DIR).join("graph.json");
    if config_path.exists() {
        if let Ok(bytes) = std::fs::read(&config_path) {
            return sha256_hex(&bytes);
        }
    }
    sha256_hex(b"{}")
}

pub fn compute_repo_state_hash(cwd: &Path) -> String {
    if cwd.join(".git").exists() {
        let head = Command::new("git")
            .args(["rev-parse", "HEAD"])
            .current_dir(cwd)
            .output()
            .ok()
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_string())
            .unwrap_or_default();
        let status = Command::new("git")
            .args(["status", "--porcelain"])
            .current_dir(cwd)
            .output()
            .ok()
            .map(|output| String::from_utf8_lossy(&output.stdout).to_string())
            .unwrap_or_default();
        sha256_hex(format!("{head}\0{status}").as_bytes())
    } else {
        let mut entries = Vec::new();
        if let Ok(dir) = std::fs::read_dir(cwd) {
            for entry in dir.flatten() {
                if let Ok(meta) = entry.metadata() {
                    entries.push(format!(
                        "{}:{}:{}",
                        entry.file_name().to_string_lossy(),
                        meta.len(),
                        meta.is_dir()
                    ));
                }
            }
        }
        entries.sort();
        sha256_hex(entries.join("\n").as_bytes())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn sample_fingerprint() -> ReplayFingerprint {
        ReplayFingerprint {
            graph_version: 1,
            config_hash: "config_abc".into(),
            repo_state_hash: "repo_123".into(),
            input_hash: "input_xyz".into(),
            contract_hash: "contract_789".into(),
        }
    }

    #[test]
    fn graph_replay_same_inputs_are_compatible() {
        let fp1 = sample_fingerprint();
        let fp2 = sample_fingerprint();
        assert!(replay_compatible(&fp1, &fp2));
        assert!(incompatibility_reason(&fp1, &fp2).is_none());
    }

    #[test]
    fn graph_replay_version_mismatch_is_incompatible() {
        let fp1 = sample_fingerprint();
        let mut fp2 = sample_fingerprint();
        fp2.graph_version = 2;
        assert!(!replay_compatible(&fp1, &fp2));
        let reason = incompatibility_reason(&fp1, &fp2).unwrap();
        assert!(reason.contains("graph version changed"));
    }

    #[test]
    fn graph_replay_config_hash_mismatch_is_incompatible() {
        let fp1 = sample_fingerprint();
        let mut fp2 = sample_fingerprint();
        fp2.config_hash = "config_different".into();
        assert!(!replay_compatible(&fp1, &fp2));
        let reason = incompatibility_reason(&fp1, &fp2).unwrap();
        assert!(reason.contains("config hash changed"));
    }

    #[test]
    fn graph_replay_repo_state_hash_mismatch_is_incompatible() {
        let fp1 = sample_fingerprint();
        let mut fp2 = sample_fingerprint();
        fp2.repo_state_hash = "repo_different".into();
        assert!(!replay_compatible(&fp1, &fp2));
        let reason = incompatibility_reason(&fp1, &fp2).unwrap();
        assert!(reason.contains("repo state changed"));
    }

    #[test]
    fn graph_replay_input_hash_mismatch_is_incompatible() {
        let fp1 = sample_fingerprint();
        let mut fp2 = sample_fingerprint();
        fp2.input_hash = "input_different".into();
        assert!(!replay_compatible(&fp1, &fp2));
        let reason = incompatibility_reason(&fp1, &fp2).unwrap();
        assert!(reason.contains("input hash changed"));
    }

    #[test]
    fn graph_replay_contract_hash_mismatch_is_incompatible() {
        let fp1 = sample_fingerprint();
        let mut fp2 = sample_fingerprint();
        fp2.contract_hash = "contract_different".into();
        assert!(!replay_compatible(&fp1, &fp2));
        let reason = incompatibility_reason(&fp1, &fp2).unwrap();
        assert!(reason.contains("contract hash changed"));
    }

    #[test]
    fn graph_replay_deterministic_hashing_from_canonical_serialized_inputs() {
        let input_a = "Briefing for node A";
        let input_b = "Briefing for node B";
        assert_eq!(compute_input_hash(input_a), compute_input_hash(input_a));
        assert_ne!(compute_input_hash(input_a), compute_input_hash(input_b));

        let c1 = compute_contract_hash(ArtifactKind::Classification);
        let c2 = compute_contract_hash(ArtifactKind::Plan);
        assert_eq!(c1, compute_contract_hash(ArtifactKind::Classification));
        assert_ne!(c1, c2);

        let dir = tempdir().unwrap();
        let empty_cfg = compute_config_hash(dir.path());
        assert_eq!(empty_cfg, sha256_hex(b"{}"));

        let cfg_dir = dir.path().join(".pi");
        std::fs::create_dir_all(&cfg_dir).unwrap();
        std::fs::write(cfg_dir.join("graph.json"), b"{\"maxWorkers\": 4}").unwrap();
        let loaded_cfg = compute_config_hash(dir.path());
        assert_ne!(loaded_cfg, empty_cfg);
        assert_eq!(loaded_cfg, sha256_hex(b"{\"maxWorkers\": 4}"));
    }

    #[test]
    fn graph_replay_resume_refuses_incompatible_fingerprint_and_reexecutes() {
        use crate::native_extensions::graph::controller::{run_graph, ControllerDeps, RunOptions};
        use crate::native_extensions::graph::types::{
            Artifact, Classification, Complexity, Phase, TaskClass, WorkerResult, WorkerUsage,
        };
        use std::collections::HashMap;
        use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
        use std::sync::Arc;

        let dir = tempdir().unwrap();
        let classify_spawns = Arc::new(AtomicUsize::new(0));
        let spawns_clone = Arc::clone(&classify_spawns);

        let runner: Arc<crate::native_extensions::graph::worker::WorkerRunner> =
            Arc::new(move |spec, _abort, _on_progress| {
                if spec.role == crate::native_extensions::graph::types::Role::Classifier {
                    spawns_clone.fetch_add(1, Ordering::SeqCst);
                    WorkerResult {
                        ok: true,
                        artifact: Some(Artifact::Classification(Classification {
                            task_class: TaskClass::Feature,
                            complexity: Complexity::Trivial,
                            research_tasks: vec![],
                            milestones: None,
                            rationale: "test".into(),
                        })),
                        ..WorkerResult::default()
                    }
                } else {
                    WorkerResult {
                        ok: true,
                        artifact: Some(Artifact::PatchReport(Box::new(
                            crate::native_extensions::graph::types::PatchReport {
                                summary: "done".into(),
                                changed_files: vec![],
                                deviations: vec![],
                                invalidation_reason: None,
                                plan_invalidated: false,
                            },
                        ))),
                        ..WorkerResult::default()
                    }
                }
            });

        let verify_exec: Arc<crate::native_extensions::graph::verify::VerifyExec> =
            Arc::new(|_, _, _, _| (0, String::new(), 0));
        let deps = ControllerDeps {
            runner,
            verify_exec,
            config: crate::native_extensions::graph::config::GraphConfig {
                verify_commands: vec![crate::native_extensions::graph::types::VerifyCommandSpec {
                    command: "echo test".into(),
                    name: "test".into(),
                    from_plan: false,
                }],
                ..Default::default()
            },
            session_model: None,
            session_thinking: None,
            project_trusted: false,
            on_update: Arc::new(|_, _| {}),
            memory: None,
            learning: None,
            governor: None,
        };

        // Candidate with incompatible repo_state_hash
        let mut resume_artifacts = HashMap::new();
        let incompatible_fp = ReplayFingerprint {
            graph_version: 1,
            config_hash: compute_config_hash(dir.path()),
            repo_state_hash: "wrong_repo_state".into(),
            input_hash: "any".into(),
            contract_hash: compute_contract_hash(ArtifactKind::Classification),
        };
        resume_artifacts.insert(
            "classify".to_string(),
            (
                Artifact::Classification(Classification {
                    task_class: TaskClass::Feature,
                    complexity: Complexity::Trivial,
                    research_tasks: vec![],
                    milestones: None,
                    rationale: "test".into(),
                }),
                WorkerUsage::default(),
                Some(incompatible_fp),
            ),
        );

        let options = RunOptions {
            goal: "test incompatible resume".into(),
            cwd: dir.path().to_path_buf(),
            forced: Some(Complexity::Trivial),
            dry_run: false,
            abort: Arc::new(AtomicBool::new(false)),
            resume_artifacts,
        };

        let run = run_graph(options, deps);
        assert_eq!(run.phase, Phase::Done);
        // classify must have re-executed because reuse was refused!
        assert_eq!(classify_spawns.load(Ordering::SeqCst), 1);
        let task = run.tasks.iter().find(|t| t.id == "classify").unwrap();
        assert!(task.fingerprint.is_some());
        assert_ne!(
            task.fingerprint.as_ref().unwrap().repo_state_hash,
            "wrong_repo_state"
        );
    }

    #[test]
    fn graph_replay_conservative_revision_loop_refuses_superseded_plan_and_patch_nodes() {
        use crate::native_extensions::graph::types::{
            GraphCounters, GraphRun, GraphTaskState, Phase, Role, TaskStatus,
        };

        let old_run = GraphRun {
            version: 1,
            run_id: "run-revised".into(),
            goal: "superseded test".into(),
            cwd: ".".into(),
            phase: Phase::Implement,
            forced: None,
            dry_run: false,
            classification: None,
            milestones: None,
            current_milestone: None,
            tasks: vec![
                GraphTaskState {
                    id: "classify".into(),
                    role: Role::Classifier,
                    expect: ArtifactKind::Classification,
                    depends_on: vec![],
                    focus: None,
                    status: TaskStatus::Succeeded,
                    attempts: 1,
                    artifact_file: Some("artifacts/classify.json".into()),
                    error: None,
                    usage: Default::default(),
                    started_at: Some(1),
                    ended_at: Some(2),
                    last_activity: None,
                    fingerprint: Some(sample_fingerprint()),
                    mutation: None,
                    context_fingerprint: None,
                    context_tokens: 0,
                    memory_refs: Vec::new(),
                    skill_refs: Vec::new(),
                },
                GraphTaskState {
                    id: "plan-1".into(),
                    role: Role::Planner,
                    expect: ArtifactKind::Plan,
                    depends_on: vec!["classify".into()],
                    focus: None,
                    status: TaskStatus::Succeeded,
                    attempts: 1,
                    artifact_file: Some("artifacts/plan-1.json".into()),
                    error: None,
                    usage: Default::default(),
                    started_at: Some(2),
                    ended_at: Some(3),
                    last_activity: None,
                    fingerprint: Some(sample_fingerprint()),
                    mutation: None,
                    context_fingerprint: None,
                    context_tokens: 0,
                    memory_refs: Vec::new(),
                    skill_refs: Vec::new(),
                },
                GraphTaskState {
                    id: "implement-1".into(),
                    role: Role::Writer,
                    expect: ArtifactKind::PatchReport,
                    depends_on: vec!["plan-1".into()],
                    focus: None,
                    status: TaskStatus::Succeeded,
                    attempts: 1,
                    artifact_file: Some("artifacts/implement-1.json".into()),
                    error: None,
                    usage: Default::default(),
                    started_at: Some(3),
                    ended_at: Some(4),
                    last_activity: None,
                    fingerprint: Some(sample_fingerprint()),
                    mutation: None,
                    context_fingerprint: None,
                    context_tokens: 0,
                    memory_refs: Vec::new(),
                    skill_refs: Vec::new(),
                },
            ],
            verification: None,
            verification_bundle: None,
            review_coverage: None,
            budgets: Default::default(),
            counters: GraphCounters {
                workers_spawned: 3,
                revision_cycles: 1, // Revised!
                replans: 0,
                cost_usd: 0.0,
                started_at: 0,
            },
            blocked_reason: None,
            resource_snapshot: None,
            updated_at: 0,
            definition: None,
        };

        // Evaluate resume candidates under conservative revision-loop rule
        let superseded = old_run.counters.revision_cycles > 0 || old_run.counters.replans > 0;
        let mut reusable = Vec::new();
        for task in &old_run.tasks {
            if task.status != TaskStatus::Succeeded || task.artifact_file.is_none() {
                continue;
            }
            let is_investigation = task.id == "classify" || task.id.starts_with("research-");
            if superseded && !is_investigation {
                continue;
            }
            reusable.push(task.id.clone());
        }

        // Only classify is reused; plan-1 and implement-1 are NOT reused!
        assert_eq!(reusable, vec!["classify"]);
    }
}
