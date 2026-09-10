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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub definition_digest: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub simulated: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_id: Option<String>,
}

impl ReplayFingerprint {
    #[allow(dead_code)]
    pub fn for_task(cwd: &Path, graph_version: u32, briefing: &str, expect: ArtifactKind) -> Self {
        Self::for_saved_task(cwd, graph_version, briefing, expect, None, false)
    }

    pub fn for_saved_task(
        cwd: &Path,
        graph_version: u32,
        briefing: &str,
        expect: ArtifactKind,
        definition_digest: Option<&str>,
        simulated: bool,
    ) -> Self {
        Self {
            graph_version,
            config_hash: compute_config_hash(cwd),
            repo_state_hash: compute_repo_state_hash(cwd),
            input_hash: compute_input_hash(briefing),
            contract_hash: compute_contract_hash(expect),
            definition_digest: definition_digest.map(str::to_string),
            simulated: if simulated { Some(true) } else { None },
            model_id: None,
        }
    }
}

pub fn saved_replay_allowed(
    definition_matches: bool,
    source_matches: bool,
    policy_matches: bool,
    simulated: bool,
) -> bool {
    definition_matches && source_matches && policy_matches && !simulated
}

pub fn replay_compatible(stored: &ReplayFingerprint, current: &ReplayFingerprint) -> bool {
    let definition_matches = stored.definition_digest == current.definition_digest;
    let source_matches = stored.repo_state_hash == current.repo_state_hash;
    let policy_matches = stored.config_hash == current.config_hash
        && stored.contract_hash == current.contract_hash
        && stored.graph_version == current.graph_version;
    let simulated = stored.simulated.unwrap_or(false) && !current.simulated.unwrap_or(false);

    if !saved_replay_allowed(
        definition_matches,
        source_matches,
        policy_matches,
        simulated,
    ) {
        return false;
    }
    if let (Some(s_mod), Some(c_mod)) = (&stored.model_id, &current.model_id) {
        if s_mod != c_mod {
            return false;
        }
    }
    stored.input_hash == current.input_hash
}

pub fn incompatibility_reason(
    stored: &ReplayFingerprint,
    current: &ReplayFingerprint,
) -> Option<String> {
    if stored.simulated.unwrap_or(false) && !current.simulated.unwrap_or(false) {
        return Some("cannot replay simulated artifact from dry-run in real execution".into());
    }
    if let (Some(s_mod), Some(c_mod)) = (&stored.model_id, &current.model_id) {
        if s_mod != c_mod {
            return Some(format!("model changed: stored={s_mod}, current={c_mod}"));
        }
    }
    if stored.definition_digest != current.definition_digest {
        return Some(format!(
            "saved definition digest changed: stored={:?}, current={:?}",
            stored.definition_digest, current.definition_digest
        ));
    }
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
    let davinci_path = cwd.join(CONFIG_DIR).join("graph.json");
    let config_path = if davinci_path.exists() {
        davinci_path
    } else {
        cwd.join(super::store::LEGACY_CONFIG_DIR).join("graph.json")
    };
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
        let mut dirty_manifest = Vec::new();
        for line in status.lines() {
            if line.len() > 3 {
                let file_path_str = line[3..].trim();
                let file_path = cwd.join(file_path_str);
                if file_path.is_file() {
                    if let Ok(bytes) = std::fs::read(&file_path) {
                        dirty_manifest.push(format!(
                            "{}:{}:{}",
                            file_path_str,
                            bytes.len(),
                            sha256_hex(&bytes)
                        ));
                    }
                }
            }
        }
        dirty_manifest.sort();
        sha256_hex(format!("{head}\0{status}\0{}", dirty_manifest.join("\n")).as_bytes())
    } else {
        fn collect_entries(root: &Path, dir: &Path, entries: &mut Vec<String>) {
            if let Ok(read_dir) = std::fs::read_dir(dir) {
                for entry in read_dir.flatten() {
                    let path = entry.path();
                    let file_name = entry.file_name().to_string_lossy().into_owned();
                    if file_name == ".git"
                        || file_name == "target"
                        || file_name == ".pi"
                        || file_name == ".davinci"
                    {
                        continue;
                    }
                    if path.is_file() {
                        if let Ok(bytes) = std::fs::read(&path) {
                            let rel = path
                                .strip_prefix(root)
                                .unwrap_or(&path)
                                .to_string_lossy()
                                .replace('\\', "/");
                            entries.push(format!("{}:{}:{}", rel, bytes.len(), sha256_hex(&bytes)));
                        }
                    } else if path.is_dir() {
                        collect_entries(root, &path, entries);
                    }
                }
            }
        }
        let mut entries = Vec::new();
        collect_entries(cwd, cwd, &mut entries);
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
            definition_digest: None,
            simulated: None,
            model_id: None,
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
            runtime: None,
            permissions: None,
            task_contract: None,
        };

        // Candidate with incompatible repo_state_hash
        let mut resume_artifacts = HashMap::new();
        let incompatible_fp = ReplayFingerprint {
            graph_version: 1,
            config_hash: compute_config_hash(dir.path()),
            repo_state_hash: "wrong_repo_state".into(),
            input_hash: "any".into(),
            contract_hash: compute_contract_hash(ArtifactKind::Classification),
            definition_digest: None,
            simulated: None,
            model_id: None,
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
            resume_run: None,
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
            execution_origin: None,
            definition_digest: None,
            saved_definition: None,
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
            ecosystem_stats: Default::default(),
            updated_at: 0,
            definition: None,
            lifecycle: None,
            revision: 0,
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

    #[test]
    fn test_saved_replay_allowed() {
        assert!(saved_replay_allowed(true, true, true, false));
        assert!(!saved_replay_allowed(false, true, true, false));
        assert!(!saved_replay_allowed(true, false, true, false));
        assert!(!saved_replay_allowed(true, true, false, false));
        assert!(!saved_replay_allowed(true, true, true, true));
        assert!(!saved_replay_allowed(false, false, false, true));
    }

    #[test]
    fn test_replay_refused_on_edited_definition() {
        let mut fp1 = sample_fingerprint();
        fp1.definition_digest = Some("digest_original".into());
        let mut fp2 = sample_fingerprint();
        fp2.definition_digest = Some("digest_modified".into());

        assert!(!replay_compatible(&fp1, &fp2));
        let reason = incompatibility_reason(&fp1, &fp2).unwrap();
        assert!(reason.contains("saved definition digest changed"));

        // When definition digest matches, it is compatible
        fp2.definition_digest = Some("digest_original".into());
        assert!(replay_compatible(&fp1, &fp2));
    }

    #[test]
    fn test_replay_refused_on_dirty_bytes_change() {
        let dir = tempdir().unwrap();
        let file_path = dir.path().join("tracked_file.txt");

        // Write first content (5 bytes)
        std::fs::write(&file_path, b"apple").unwrap();
        let hash_1 = compute_repo_state_hash(dir.path());

        // Overwrite with same byte length, different content (5 bytes)
        std::fs::write(&file_path, b"lemon").unwrap();
        let hash_2 = compute_repo_state_hash(dir.path());

        // Hash must be different because file content bytes changed
        assert_ne!(hash_1, hash_2);

        let mut fp1 = sample_fingerprint();
        fp1.repo_state_hash = hash_1;
        let mut fp2 = sample_fingerprint();
        fp2.repo_state_hash = hash_2;

        assert!(!replay_compatible(&fp1, &fp2));
        let reason = incompatibility_reason(&fp1, &fp2).unwrap();
        assert!(reason.contains("repo state changed"));
    }

    #[test]
    fn test_replay_refused_on_policy_change() {
        let fp_base = sample_fingerprint();

        // Config hash change
        let mut fp_config_changed = fp_base.clone();
        fp_config_changed.config_hash = "config_updated".into();
        assert!(!replay_compatible(&fp_base, &fp_config_changed));
        assert!(incompatibility_reason(&fp_base, &fp_config_changed)
            .unwrap()
            .contains("config hash changed"));

        // Contract hash change
        let mut fp_contract_changed = fp_base.clone();
        fp_contract_changed.contract_hash = "contract_updated".into();
        assert!(!replay_compatible(&fp_base, &fp_contract_changed));
        assert!(incompatibility_reason(&fp_base, &fp_contract_changed)
            .unwrap()
            .contains("contract hash changed"));

        // Graph version change
        let mut fp_version_changed = fp_base.clone();
        fp_version_changed.graph_version += 1;
        assert!(!replay_compatible(&fp_base, &fp_version_changed));
        assert!(incompatibility_reason(&fp_base, &fp_version_changed)
            .unwrap()
            .contains("graph version changed"));
    }

    #[test]
    fn test_replay_refused_on_simulated_artifact() {
        let mut stored = sample_fingerprint();
        stored.simulated = Some(true);

        let current_real = sample_fingerprint();
        assert!(!replay_compatible(&stored, &current_real));
        let reason = incompatibility_reason(&stored, &current_real).unwrap();
        assert!(reason.contains("cannot replay simulated artifact from dry-run in real execution"));

        // Both simulated: dry-run on dry-run allowed
        let mut current_sim = sample_fingerprint();
        current_sim.simulated = Some(true);
        assert!(replay_compatible(&stored, &current_sim));
    }

    #[test]
    fn test_legacy_state_v1_loads_cleanly() {
        let legacy_json = r#"{
            "graphVersion": 1,
            "configHash": "cfg123",
            "repoStateHash": "repo456",
            "inputHash": "in789",
            "contractHash": "contract000"
        }"#;

        let parsed: ReplayFingerprint = serde_json::from_str(legacy_json).unwrap();
        assert_eq!(parsed.graph_version, 1);
        assert_eq!(parsed.config_hash, "cfg123");
        assert_eq!(parsed.repo_state_hash, "repo456");
        assert_eq!(parsed.input_hash, "in789");
        assert_eq!(parsed.contract_hash, "contract000");
        assert_eq!(parsed.definition_digest, None);
        assert_eq!(parsed.simulated, None);

        // Serialized output omits None fields
        let serialized = serde_json::to_string(&parsed).unwrap();
        assert!(!serialized.contains("definitionDigest"));
        assert!(!serialized.contains("simulated"));
    }
}
