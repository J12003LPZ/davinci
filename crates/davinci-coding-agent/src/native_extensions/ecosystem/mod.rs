//! Bounded ecosystem integration contracts for Graph, Token Governor, Vector Memory, and Learning.

pub mod cache_affinity;
pub mod capability;
pub mod context;
pub mod resource;
pub mod risk;
pub mod telemetry;
pub mod verification;

#[allow(unused_imports)]
pub use cache_affinity::*;
#[allow(unused_imports)]
pub use capability::*;
#[allow(unused_imports)]
pub use context::*;
#[allow(unused_imports)]
pub use resource::*;
#[allow(unused_imports)]
pub use risk::*;
#[allow(unused_imports)]
pub use telemetry::*;
#[allow(unused_imports)]
pub use verification::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_extensions::graph::config::GraphConfig;
    use crate::native_extensions::graph::controller::{run_graph, ControllerDeps, RunOptions};
    use crate::native_extensions::graph::roles::{ensure_governor_recovery_tool, role_tools};
    use crate::native_extensions::graph::types::{
        Artifact, ArtifactKind, Classification, Complexity, GraphBudgets, PatchReport, Phase,
        ReviewDecision, Role, TaskClass, Verdict, VerifyCommandSpec, WorkerResult, WorkerSpec,
    };
    use crate::native_extensions::graph::worker::{build_worker_args, WorkerRunner};
    use crate::native_extensions::learning::types::{
        ArtifactStatus, LearningScope, SkillLedgerRecord, SkillOrigin, SkillOutcome,
        SkillVersionRef,
    };
    use crate::native_extensions::token_governor::{TokenGovernor, TokenGovernorConfig};
    use crate::native_extensions::vector_memory::{
        resolve_repo_id, MemoryKind, MemoryRecord, VectorMemory, VectorMemoryConfig,
    };
    use davinci_agent::ToolResult;
    use serde_json::json;
    use std::collections::HashMap;
    use std::fmt::Write as _;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{Arc, Mutex};
    use tempfile::tempdir;

    #[test]
    fn default_graph_context_caps_match_design() {
        assert_eq!(DEFAULT_GRAPH_CONTEXT_TOKENS, 2_500);
        assert_eq!(DEFAULT_GRAPH_MEMORY_TOKENS, 1_200);
        assert_eq!(DEFAULT_GRAPH_MEMORY_HITS, 4);
        assert_eq!(DEFAULT_GRAPH_SKILL_TOKENS, 1_000);
        assert_eq!(DEFAULT_GRAPH_SKILL_COUNT, 2);
    }

    /// Task 4 Step 2: ecosystem_loop_governor_recovery
    /// Graph worker produces oversized allowed output -> governor digest -> retrieve_output available -> original content recovered byte-for-byte.
    #[test]
    fn ecosystem_loop_governor_recovery() {
        // 1. Graph role tools ensure Governor recovery capability
        let mut tools = role_tools(Role::Researcher);
        ensure_governor_recovery_tool(&mut tools);
        assert!(
            tools.contains(&"retrieve_output".to_string()),
            "retrieve_output must be present in worker tools"
        );

        // 2. Token Governor with low threshold to trigger compression
        let dir = tempdir().unwrap();
        let config = TokenGovernorConfig {
            enabled: true,
            compress_threshold_bytes: 100,
            store_dir: Some(dir.path().to_path_buf()),
            ..TokenGovernorConfig::default()
        };
        let mut governor = TokenGovernor::new("test-session", config);

        // Oversized output from compressible tool
        let mut large_output = String::new();
        for i in 0..50 {
            let _ = writeln!(
                large_output,
                "line {i}: payload with extensive telemetry data for testing governor recovery loop"
            );
        }
        assert!(large_output.len() > 100);

        let initial_result = ToolResult {
            content: large_output.clone(),
            is_error: false,
            details: None,
        };

        // 3. Output is compressed into digest with reference
        let compressed_result =
            governor.after_tool("bash", &json!({"command": "find ."}), initial_result);
        assert!(compressed_result.content.contains("retrieve_output"));
        let details = compressed_result.details.expect("details must be present");
        let gov_details = details.get("tokenGovernor").expect("tokenGovernor block");
        assert_eq!(
            gov_details.get("compressed").and_then(|v| v.as_bool()),
            Some(true)
        );
        let reference = gov_details
            .get("reference")
            .and_then(|v| v.as_str())
            .expect("reference");
        assert!(reference.starts_with("governor://"));
        let output_id = gov_details
            .get("outputId")
            .and_then(|v| v.as_str())
            .expect("outputId");

        // 4. Retrieve original via lossless recovery tool
        let retrieved = governor
            .retrieve(&json!({"id": output_id}))
            .expect("retrieve_output should succeed");
        assert!(retrieved.content.contains("1: line 0: payload"));
        assert!(retrieved.content.contains("50: line 49: payload"));
        let reconstructed = retrieved
            .content
            .lines()
            .map(|l| {
                if let Some(pos) = l.find(": ") {
                    &l[pos + 2..]
                } else {
                    l
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        assert_eq!(
            reconstructed, large_output,
            "reconstructed content must match original byte-for-byte"
        );

        // 5. Governor stats track the retrieval
        let stats = governor.stats();
        assert_eq!(stats.compressed_outputs, 1);
        assert_eq!(stats.retrievals, 1);
        assert!(stats.bytes_withheld > 0);
    }

    /// Task 4 Step 3: ecosystem_loop_cache_affinity
    /// Compatible retry gets the same cache key. Changed model, toolset, graph version, or system contract gets a different key. Worker still has session_id == None.
    #[test]
    fn ecosystem_loop_cache_affinity() {
        let cwd = "/test/repo";
        let tools = vec![
            "read".to_string(),
            "grep".to_string(),
            "retrieve_output".to_string(),
        ];
        let prompt = "System prompt";

        // Compatible retry => same key
        let key1 = derive_worker_cache_key(
            cwd,
            1,
            Role::Researcher,
            Some("claude-3-7-sonnet"),
            &tools,
            prompt,
            ArtifactKind::Evidence,
        );
        let key2 = derive_worker_cache_key(
            cwd,
            1,
            Role::Researcher,
            Some("claude-3-7-sonnet"),
            &tools,
            prompt,
            ArtifactKind::Evidence,
        );
        assert_eq!(
            key1, key2,
            "Compatible retry must yield identical cache key"
        );

        // Changed model => different key
        let key_model = derive_worker_cache_key(
            cwd,
            1,
            Role::Researcher,
            Some("claude-3-5-sonnet"),
            &tools,
            prompt,
            ArtifactKind::Evidence,
        );
        assert_ne!(key1, key_model, "Changed model must change cache key");

        // Changed tools => different key
        let mut diff_tools = tools.clone();
        diff_tools.push("patch".to_string());
        let key_tools = derive_worker_cache_key(
            cwd,
            1,
            Role::Researcher,
            Some("claude-3-7-sonnet"),
            &diff_tools,
            prompt,
            ArtifactKind::Evidence,
        );
        assert_ne!(key1, key_tools, "Changed tools must change cache key");

        // Changed graph version => different key
        let key_version = derive_worker_cache_key(
            cwd,
            2,
            Role::Researcher,
            Some("claude-3-7-sonnet"),
            &tools,
            prompt,
            ArtifactKind::Evidence,
        );
        assert_ne!(
            key1, key_version,
            "Changed graph version must change cache key"
        );

        // Changed contract => different key
        let key_contract = derive_worker_cache_key(
            cwd,
            1,
            Role::Researcher,
            Some("claude-3-7-sonnet"),
            &tools,
            prompt,
            ArtifactKind::PatchReport,
        );
        assert_ne!(key1, key_contract, "Changed contract must change cache key");

        // Ephemeral isolation verified: worker args omit session and carry --no-session
        let spec = WorkerSpec {
            service_tier: davinci_ai::CodexServiceTier::Standard,
            task_id: "test-task".into(),
            role: Role::Researcher,
            tools: tools.clone(),
            authorized_tools: tools.clone(),
            initially_exposed_tools: tools.clone(),
            model: Some("claude-3-7-sonnet".into()),
            thinking_level: None,
            project_trusted: false,
            cwd: PathBuf::from(cwd),
            briefing: "briefing".into(),
            system_prompt: prompt.into(),
            artifact_path: "artifact.json".into(),
            transcript_path: None,
            timeout_ms: 0,
            run_deadline: None,
            expect: ArtifactKind::Evidence,
            extra_extensions: vec![],
            runtime_agent_id: None,
            worker_session: None,
            task_contract: None,
            coordinator_client: None,
            node_abort: None,
        };
        let args = build_worker_args(&spec, Path::new("briefing.md"), Path::new("system.md"));
        assert!(
            args.contains(&"--no-session".to_string()),
            "worker args must contain --no-session"
        );
        assert!(
            args.contains(&"--no-extensions".to_string()),
            "worker args must contain --no-extensions"
        );
        assert!(
            args.contains(&"--no-skills".to_string()),
            "worker args must contain --no-skills"
        );

        // Verify StreamOptions decoupling: session_id is None, cache_key is populated
        let stream_opts = davinci_ai::StreamOptions {
            session_id: None,
            cache_key: Some(key1.clone()),
            ..davinci_ai::StreamOptions::default()
        };
        assert_eq!(stream_opts.session_id, None);
        assert_eq!(stream_opts.cache_key.as_deref(), Some(key1.as_str()));
        assert_eq!(
            davinci_ai::cache::effective_prompt_cache_key(&stream_opts),
            Some(key1.as_str())
        );
    }

    /// Task 4 Step 4: ecosystem_loop_memory_to_graph
    /// Settled turn -> vector index -> later graph packet retrieves bounded relevant memory.
    #[test]
    fn ecosystem_loop_memory_to_graph() {
        let dir = tempdir().unwrap();

        // 1. Settled turn indexed in vector memory
        let mem_dir = dir.path().join(".pi").join("vector-memory");
        std::fs::create_dir_all(&mem_dir).unwrap();
        let repo = resolve_repo_id(dir.path());
        let record = MemoryRecord {
            id: "mem-settled-001".into(),
            repo_id: repo.clone(),
            kind: MemoryKind::Decision,
            text: "Architectural decision: Always enforce token governor budgets during worker graph execution".into(),
            source: "assistant".into(),
            content_hash: "hash-settled-1".into(),
            importance: 0.9,
            created_at: 1000,
            embedding: None,

            embedding_identity: None,
            confidence: None,
            source_session_id: Some("session-123".into()),
            source_turn: Some(5),
            verification: None,
            source_paths: Vec::new(),
            source_state_hash: None,
            verified_at_revision: None,
            use_count: 0,
            last_used_at: None,
            agent_profile_name: None,
            memory_scope: None,
        };
        std::fs::write(
            mem_dir.join("records.jsonl"),
            serde_json::to_string(&record).unwrap() + "\n",
        )
        .unwrap();

        let memory = VectorMemory::with_config(
            dir.path().to_path_buf(),
            VectorMemoryConfig {
                minimum_score: 0.2,
                ..VectorMemoryConfig::default()
            },
        );
        let learning = crate::native_extensions::LearningController::new(dir.path(), None, None);

        // 2. Later graph turn requests context packet
        let request = ContextPacketRequest::new("token governor worker budget")
            .with_role(Role::Researcher)
            .with_token_cap(DEFAULT_GRAPH_CONTEXT_TOKENS);

        let packet = build_context_packet(&memory, &learning, request);

        // 3. Packet retrieves bounded relevant memory
        assert!(!packet.is_empty());
        assert!(!packet.fingerprint.is_empty());
        assert_eq!(packet.memory_refs.len(), 1);
        assert_eq!(packet.memory_refs[0], "mem-settled-001");
        assert!(packet.estimated_tokens <= DEFAULT_GRAPH_CONTEXT_TOKENS);
        assert!(packet.memory_tokens <= DEFAULT_GRAPH_MEMORY_TOKENS);
        assert!(packet
            .text
            .contains("Always enforce token governor budgets"));
    }

    /// Task 4 Step 5: ecosystem_loop_learning_to_graph
    /// Verified learning artifact -> selected exact skill version -> graph metadata -> successful verification -> that version's success ledger increments.
    #[test]
    fn ecosystem_loop_learning_to_graph() {
        let dir = tempdir().unwrap();

        // 1. Create skill file on disk in .pi/skills/cache-optimization/SKILL.md
        let skills_dir = dir
            .path()
            .join(".pi")
            .join("skills")
            .join("cache-optimization");
        std::fs::create_dir_all(&skills_dir).unwrap();
        let content = "---\nname: cache-optimization\ndescription: Cache optimization guidelines for compiler workers\nroles: [researcher, writer]\n---\n# Cache Optimization\nEnsure prompt prefix stability for key reuse.\n";
        let skill_path = skills_dir.join("SKILL.md");
        std::fs::write(&skill_path, content).unwrap();

        // 2. Learning controller with verified skill record in project store
        let mut learning =
            crate::native_extensions::LearningController::new(dir.path(), None, None);
        let record = SkillLedgerRecord {
            skill_id: "skill-cache-opt-01".into(),
            name: "cache-optimization".into(),
            scope: LearningScope::Project,
            origin: SkillOrigin::LearnedReview,
            status: ArtifactStatus::Active,
            path: skill_path,
            content_hash: "hash-cache-opt-1".into(),
            version: 1,
            success_count: 0,
            failure_count: 0,
            neutral_count: 0,
            last_used_at_ms: None,
            created_at_ms: 1000,
            updated_at_ms: 1000,
            applicability: Default::default(),
            pinned: false,
        };
        learning.project_store.upsert_skill(record).unwrap();

        // 3. Select exact skill version in graph context packet
        let memory = VectorMemory::new(dir.path().to_path_buf());
        let request = ContextPacketRequest::new("cache optimization compiler workers")
            .with_role(Role::Writer)
            .with_token_cap(DEFAULT_GRAPH_CONTEXT_TOKENS);
        let packet = build_context_packet(&memory, &learning, request);

        assert!(!packet.skill_refs.is_empty());
        let skill_ref = &packet.skill_refs[0];
        assert_eq!(skill_ref.name, "cache-optimization");
        assert_eq!(skill_ref.version, 1);

        // 4. Successful verification in graph run -> outcome ledger update
        let version_ref = SkillVersionRef {
            name: skill_ref.name.clone(),
            version: skill_ref.version,
            content_hash: skill_ref.content_hash.clone(),
        };
        learning
            .record_skill_version_outcome(&version_ref, SkillOutcome::VerifiedSuccess)
            .unwrap();

        // 5. Assert that version's success ledger increments
        let updated = learning.project_store.skill("cache-optimization").unwrap();
        assert_eq!(updated.success_count, 1);
        assert_eq!(learning.stats.verified_skill_successes, 1);
    }

    /// Task 4 Step 6: ecosystem_loop_security_gate
    /// High-risk graph mutation -> required security failure -> approval impossible.
    #[test]
    fn ecosystem_loop_security_gate() {
        let dir = tempdir().unwrap();
        let memory = VectorMemory::new(dir.path().to_path_buf());
        let learning = crate::native_extensions::LearningController::new(dir.path(), None, None);

        // Write a sensitive file with an exposed API key credential
        let auth_file = dir.path().join("src/auth.rs");
        std::fs::create_dir_all(auth_file.parent().unwrap()).unwrap();
        std::fs::write(
            &auth_file,
            "pub fn key() -> &'static str { \"sk-proj-4f9Qa2Lk8Zt3Vb7Nc1Xd6Rm0Hs\" }\n",
        )
        .unwrap();

        // Mock runner where writer mutates the sensitive auth file
        let runner: Arc<WorkerRunner> = Arc::new(|spec, _abort, _on_progress| {
            let artifact = match spec.expect {
                ArtifactKind::Classification => Artifact::Classification(Classification {
                    task_class: TaskClass::Trivial,
                    complexity: Complexity::Trivial,
                    rationale: "security test".into(),
                    research_tasks: vec![],
                    milestones: None,
                }),
                ArtifactKind::PatchReport => Artifact::PatchReport(Box::new(PatchReport {
                    changed_files: vec!["src/auth.rs".into()],
                    summary: "modified auth credentials".into(),
                    deviations: vec![],
                    plan_invalidated: false,
                    invalidation_reason: None,
                })),
                _ => Artifact::Review(Box::new(ReviewDecision {
                    verdict: Verdict::Approve,
                    issues: vec![],
                    notes: "ok".into(),
                    reviewed_chunk_ids: vec![],
                })),
            };
            WorkerResult {
                ok: true,
                artifact: Some(artifact),
                ..WorkerResult::default()
            }
        });

        // Unit tests pass, but security scanner detects credential blocker
        let deps = ControllerDeps {
            runner,
            verify_exec: Arc::new(|_, _, _, _| (0, "unit tests passed".into(), 10)),
            config: GraphConfig {
                security_verification: SecurityPolicyMode::Always,
                verify_commands: vec![VerifyCommandSpec {
                    command: "echo test".into(),
                    name: "test".into(),
                    from_plan: false,
                }],
                budgets: GraphBudgets {
                    max_revision_cycles: 0,
                    ..Default::default()
                },
                ..Default::default()
            },
            session_model: None,
            session_thinking: None,
            session_service_tier: davinci_ai::CodexServiceTier::Standard,
            project_trusted: false,
            on_update: Arc::new(|_, _| {}),
            memory: Some(Arc::new(Mutex::new(memory))),
            learning: Some(Arc::new(Mutex::new(learning))),
            governor: None,
            language_intelligence: None,
            processes: None,
            browser: None,
            runtime: None,
            permissions: None,
            task_contract: None,
        };

        let options = RunOptions {
            goal: "high risk security gate proof".into(),
            cwd: dir.path().to_path_buf(),
            forced: Some(Complexity::Trivial),
            dry_run: false,
            abort: Arc::new(AtomicBool::new(false)),
            resume_artifacts: HashMap::new(),
            resume_run: None,
        };

        let run = run_graph(options, deps);

        // High-risk mutation + failed security verification -> approval impossible, phase is Blocked
        assert_eq!(run.phase, Phase::Blocked);
        assert!(run.blocked_reason.is_some());
        assert!(
            run.ecosystem_stats.security_gate_triggered,
            "security gate must be triggered"
        );
        assert_eq!(
            run.ecosystem_stats.security_result,
            Some("failed".to_string())
        );
    }

    /// Worker fixture for the closed-loop tests: a trivial run whose writer
    /// changes `src/lib.rs`.
    fn full_circle_runner(cwd: PathBuf) -> Arc<WorkerRunner> {
        Arc::new(move |spec, _abort, _on_progress| {
            let artifact = match spec.expect {
                ArtifactKind::Classification => Artifact::Classification(Classification {
                    task_class: TaskClass::Trivial,
                    complexity: Complexity::Trivial,
                    rationale: "full circle".into(),
                    research_tasks: vec![],
                    milestones: None,
                }),
                ArtifactKind::PatchReport => {
                    let path = cwd.join("src").join("lib.rs");
                    let previous = std::fs::read_to_string(&path).unwrap_or_default();
                    std::fs::write(&path, format!("{previous}// integration\n")).unwrap();
                    Artifact::PatchReport(Box::new(PatchReport {
                        changed_files: vec!["src/lib.rs".into()],
                        summary: "applied full circle integration".into(),
                        deviations: vec![],
                        plan_invalidated: false,
                        invalidation_reason: None,
                    }))
                }
                _ => Artifact::Review(Box::new(ReviewDecision {
                    verdict: Verdict::Approve,
                    issues: vec![],
                    notes: "ok".into(),
                    reviewed_chunk_ids: vec![],
                })),
            };
            WorkerResult {
                ok: true,
                artifact: Some(artifact),
                ..WorkerResult::default()
            }
        })
    }

    fn full_circle_deps(
        cwd: &Path,
        memory: &Arc<Mutex<VectorMemory>>,
        learning: &Arc<Mutex<crate::native_extensions::LearningController>>,
    ) -> ControllerDeps {
        ControllerDeps {
            runner: full_circle_runner(cwd.to_path_buf()),
            verify_exec: Arc::new(|_, _, _, _| {
                (
                    0,
                    "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s".into(),
                    5,
                )
            }),
            config: GraphConfig {
                verify_commands: vec![VerifyCommandSpec {
                    command: "cargo test".into(),
                    name: "test".into(),
                    from_plan: false,
                }],
                ..Default::default()
            },
            session_model: None,
            session_thinking: None,
            session_service_tier: davinci_ai::CodexServiceTier::Standard,
            project_trusted: true,
            on_update: Arc::new(|_, _| {}),
            memory: Some(Arc::clone(memory)),
            learning: Some(Arc::clone(learning)),
            governor: None,
            language_intelligence: None,
            processes: None,
            browser: None,
            runtime: None,
            permissions: None,
            task_contract: None,
        }
    }

    fn full_circle_options(cwd: &Path, goal: &str) -> RunOptions {
        RunOptions {
            goal: goal.into(),
            cwd: cwd.to_path_buf(),
            forced: Some(Complexity::Trivial),
            dry_run: false,
            abort: Arc::new(AtomicBool::new(false)),
            resume_artifacts: HashMap::new(),
            resume_run: None,
        }
    }

    fn full_circle_workspace() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let dir = tempdir().unwrap();
        let cwd = dir.path().join("repo");
        let agent_dir = dir.path().join("agent");
        std::fs::create_dir_all(cwd.join("src")).unwrap();
        std::fs::create_dir_all(&agent_dir).unwrap();
        std::fs::write(cwd.join("src").join("lib.rs"), "pub fn lib() {}\n").unwrap();
        (dir, cwd, agent_dir)
    }

    /// Task 4 Step 7: ecosystem_loop_full_circle
    /// Graph Run #1 -> verification -> the graph itself persists a memory and
    /// hands the run to the learning reviewer, which creates a project skill ->
    /// Graph Run #2 receives both -> Run #2 outcome recorded on that exact version.
    #[test]
    fn ecosystem_loop_full_circle() {
        use crate::native_extensions::learning::{parse_review_fixture, ReviewRunner};
        let (_dir, cwd, agent_dir) = full_circle_workspace();

        // The reviewer answers from a canned fixture on the background thread
        // the live session uses; nothing is seeded into the stores.
        let learning_config = crate::native_extensions::LearningConfig {
            background_review: true,
            auto_apply_project: true,
            ..Default::default()
        };
        let mut learning = crate::native_extensions::LearningController::new(
            &cwd,
            Some(&agent_dir),
            Some(learning_config),
        );
        learning.set_live_reviewer(&cwd, None);
        let reviewed_run = Arc::new(Mutex::new(None::<String>));
        let seen = Arc::clone(&reviewed_run);
        learning.review_runner = Some(ReviewRunner(Arc::new(
            move |evidence, config, _run, _spec| {
                *seen.lock().unwrap() = evidence.verification.graph_run_id.clone();
                let fixture = json!({
                    "candidates": [{
                        "scope": "project",
                        "confidence": 0.95,
                        "rationale": "verified graph integration workflow",
                        "artifact": {
                            "kind": "skill_create",
                            "name": "full-circle-skill",
                            "description": "Full circle integration workflow for lib changes",
                            "body": "---\nname: full-circle-skill\ndescription: Full circle integration workflow for lib changes\n---\n\nEdit src/lib.rs, then run cargo test.\n"
                        }
                    }]
                })
                .to_string();
                parse_review_fixture(&fixture, evidence, config.max_candidates_per_review).unwrap()
            },
        )));
        let learning = Arc::new(Mutex::new(learning));
        let memory = VectorMemory::new(cwd.clone());
        memory.mark_dense_offline();
        let memory = Arc::new(Mutex::new(memory));

        // Run #1: nothing learned yet.
        let run1 = run_graph(
            full_circle_options(&cwd, "add full circle integration to lib"),
            full_circle_deps(&cwd, &memory, &learning),
        );
        assert_eq!(run1.phase, Phase::Done, "{:?}", run1.blocked_reason);
        assert!(run1.tasks.iter().all(|task| task.skill_refs.is_empty()));

        // The verified run left a high-confidence memory of what it did.
        let memory_id = {
            let memory = memory.lock().unwrap();
            let record = memory
                .records()
                .iter()
                .find(|record| {
                    record.kind == MemoryKind::TaskResult && record.text.contains(&run1.run_id)
                })
                .expect("verified run memory")
                .clone();
            assert!(record.text.contains("src/lib.rs"));
            assert!(record.text.contains("cargo test"));
            record.id
        };

        // ...and was submitted to the reviewer, which created a project skill.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while learning.lock().unwrap().live_review_running() {
            assert!(
                std::time::Instant::now() < deadline,
                "review never finished"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        learning.lock().unwrap().apply_completed_reviews();
        assert_eq!(
            reviewed_run.lock().unwrap().as_deref(),
            Some(run1.run_id.as_str())
        );
        let skill_v1 = learning
            .lock()
            .unwrap()
            .project_store
            .skill("full-circle-skill")
            .expect("reviewer created the skill from the graph run")
            .clone();
        assert_eq!(skill_v1.status, ArtifactStatus::Active);

        // Run #2: a related goal receives the learned skill and memory.
        let run2 = run_graph(
            full_circle_options(&cwd, "full circle integration for lib"),
            full_circle_deps(&cwd, &memory, &learning),
        );
        assert_eq!(run2.phase, Phase::Done, "{:?}", run2.blocked_reason);
        assert!(run2.verification.as_ref().is_some_and(|v| v.passed));
        let skill_ref = run2
            .tasks
            .iter()
            .flat_map(|task| task.skill_refs.iter())
            .find(|skill| skill.name == "full-circle-skill")
            .expect("run 2 received the learned skill");
        assert_eq!(skill_ref.version, u64::from(skill_v1.version));
        assert_eq!(skill_ref.content_hash, skill_v1.content_hash);
        assert!(run2
            .tasks
            .iter()
            .any(|task| task.memory_refs.contains(&memory_id)));

        // The outcome of run #2 is attributed to that exact version.
        let reloaded =
            crate::native_extensions::LearningController::new(&cwd, Some(&agent_dir), None);
        let record = reloaded
            .project_store
            .skill_version("full-circle-skill", u64::from(skill_v1.version))
            .expect("learned version persisted");
        assert_eq!(
            record.success_count + record.neutral_count + record.failure_count,
            1
        );
        assert_eq!(record.failure_count, 0);
    }

    /// The host and the graph share one learning controller: a skill the host
    /// learns after the graph was wired reaches graph workers, and graph
    /// status shows the host's learning counters.
    #[test]
    fn graph_uses_the_host_learning_controller() {
        let (_dir, cwd, agent_dir) = full_circle_workspace();
        let host = crate::native_extensions::NativeExtensionHost::new_with_agent_dir(
            "s",
            &cwd,
            Some(&agent_dir),
        );
        let shared = host.graph.learning.clone().expect("graph learning wired");
        assert!(Arc::ptr_eq(&shared, &host.learning));
        {
            let mut learning = host.learning();
            let skill_dir = learning.project_skills_dir.join("host-skill");
            std::fs::create_dir_all(&skill_dir).unwrap();
            let path = skill_dir.join("SKILL.md");
            std::fs::write(
                &path,
                "---\nname: host-skill\ndescription: Host learned lib integration workflow\n---\n\nRun cargo test after lib edits.\n",
            )
            .unwrap();
            learning
                .project_store
                .upsert_skill(SkillLedgerRecord {
                    skill_id: "host-skill-1".into(),
                    name: "host-skill".into(),
                    scope: LearningScope::Project,
                    origin: SkillOrigin::LearnedReview,
                    status: ArtifactStatus::Active,
                    path,
                    content_hash: "hash-host-1".into(),
                    version: 1,
                    success_count: 0,
                    failure_count: 0,
                    neutral_count: 0,
                    last_used_at_ms: None,
                    created_at_ms: 1,
                    updated_at_ms: 1,
                    applicability: Default::default(),
                    pinned: false,
                })
                .unwrap();
            learning.stats.reviews_dispatched = 7;
            learning.stats.reviews_skipped = 2;
            learning.stats.candidates_approved = 3;
        }

        let memory = VectorMemory::new(cwd.clone());
        memory.mark_dense_offline();
        let memory = Arc::new(Mutex::new(memory));
        let run = run_graph(
            full_circle_options(&cwd, "host learned lib integration"),
            full_circle_deps(&cwd, &memory, &shared),
        );
        assert_eq!(run.phase, Phase::Done, "{:?}", run.blocked_reason);
        assert!(run
            .tasks
            .iter()
            .flat_map(|task| task.skill_refs.iter())
            .any(|skill| skill.name == "host-skill" && skill.content_hash == "hash-host-1"));

        let host_stats = host.learning().stats.clone();
        assert!(host_stats.reviews_dispatched >= 7);
        assert_eq!(
            run.ecosystem_stats.learning_reviews_dispatched,
            host_stats.reviews_dispatched
        );
        assert_eq!(
            run.ecosystem_stats.learning_reviews_skipped,
            host_stats.reviews_skipped
        );
        assert_eq!(
            run.ecosystem_stats.learned_artifacts_applied,
            host_stats.candidates_approved
        );
    }

    /// Task 4 Step 8: ecosystem_invariants_token_and_calls
    /// Assert packet <=2,500 estimated tokens, <=4 memory hits, <=2 full skills, and no extra coordinator completer calls.
    #[test]
    fn ecosystem_invariants_token_and_calls() {
        let dir = tempdir().unwrap();

        // 1. Populate 25 memory records
        let mem_dir = dir.path().join(".pi").join("vector-memory");
        std::fs::create_dir_all(&mem_dir).unwrap();
        let repo = resolve_repo_id(dir.path());
        let records = (0..25)
            .map(|i| {
                let rec = MemoryRecord {
                    id: format!("mem-invariant-{i:03}"),
                    repo_id: repo.clone(),
                    kind: MemoryKind::Discovery,
                    text: format!(
                        "Invariant memory entry {i} with lengthy verbose text that consumes plenty of prompt space across multiple sentences to test truncation."
                    ),
                    source: "user".into(),
                    content_hash: format!("hash-{i}"),
                    importance: 0.85,
                    created_at: 2000 + i as u64,
                    embedding: None,

                    embedding_identity: None,
                    confidence: None,
                    source_session_id: None,
                    source_turn: None,
                    verification: None,
                    source_paths: Vec::new(),
                    source_state_hash: None,
                    verified_at_revision: None,
                    use_count: 0,
                    last_used_at: None,
                    agent_profile_name: None,
                    memory_scope: None,
                };
                serde_json::to_string(&rec).unwrap()
            })
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(mem_dir.join("records.jsonl"), records).unwrap();

        let memory = VectorMemory::with_config(
            dir.path().to_path_buf(),
            VectorMemoryConfig {
                minimum_score: 0.1,
                ..VectorMemoryConfig::default()
            },
        );

        // 2. Populate 10 skills with substantial instruction bodies
        let skills_dir = dir.path().join(".pi").join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        for i in 0..10 {
            let s_dir = skills_dir.join(format!("heavy-skill-{i}"));
            std::fs::create_dir_all(&s_dir).unwrap();
            let content = format!(
                "---\nname: heavy-skill-{i}\ndescription: Heavy skill {i} with verbose instructions for invariant testing\nroles: [researcher, writer]\n---\n# Heavy Skill {i}\nExtensive invariant instructions.\n{}",
                "step detail line for heavy skill instructions to test token caps\n".repeat(40)
            );
            std::fs::write(s_dir.join("SKILL.md"), content).unwrap();
        }

        let learning = crate::native_extensions::LearningController::new(dir.path(), None, None);

        // 3. Build context packet under default caps
        let request = ContextPacketRequest::new("Invariant testing with verbose instructions")
            .with_role(Role::Researcher)
            .with_token_cap(DEFAULT_GRAPH_CONTEXT_TOKENS);

        let packet = build_context_packet(&memory, &learning, request);

        // 4. Assert token and hit invariants
        assert!(!packet.is_empty());
        assert!(
            packet.estimated_tokens <= 2_500,
            "Context packet must stay <= 2,500 estimated tokens, got {}",
            packet.estimated_tokens
        );
        assert!(
            packet.memory_refs.len() <= 4,
            "Memory hits must stay <= 4, got {}",
            packet.memory_refs.len()
        );
        assert!(
            packet.skill_refs.len() <= 2,
            "Skill refs must stay <= 2, got {}",
            packet.skill_refs.len()
        );
        assert!(
            packet
                .text
                .starts_with("<context source=\"davinci\" untrusted=\"true\">"),
            "Context XML wrapper header required"
        );
        assert!(
            packet.text.ends_with("</context>"),
            "Context XML wrapper footer required"
        );

        // 5. In a graph run, assert zero extra coordinator completer calls occur
        let runner_invocations = Arc::new(AtomicUsize::new(0));
        let runner_clone = runner_invocations.clone();
        let runner: Arc<WorkerRunner> = Arc::new(move |spec, _abort, _on_progress| {
            runner_clone.fetch_add(1, Ordering::SeqCst);
            let artifact = match spec.expect {
                ArtifactKind::Classification => Artifact::Classification(Classification {
                    task_class: TaskClass::Trivial,
                    complexity: Complexity::Trivial,
                    rationale: "invariant run".into(),
                    research_tasks: vec![],
                    milestones: None,
                }),
                _ => Artifact::PatchReport(Box::new(PatchReport {
                    changed_files: vec![],
                    summary: "done".into(),
                    deviations: vec![],
                    plan_invalidated: false,
                    invalidation_reason: None,
                })),
            };
            WorkerResult {
                ok: true,
                artifact: Some(artifact),
                ..WorkerResult::default()
            }
        });

        let deps = ControllerDeps {
            runner,
            verify_exec: Arc::new(|_, _, _, _| (0, String::new(), 0)),
            config: GraphConfig {
                verify_commands: vec![VerifyCommandSpec {
                    command: "echo test".into(),
                    name: "test".into(),
                    from_plan: false,
                }],
                ..Default::default()
            },
            session_model: None,
            session_thinking: None,
            session_service_tier: davinci_ai::CodexServiceTier::Standard,
            project_trusted: false,
            on_update: Arc::new(|_, _| {}),
            memory: Some(Arc::new(Mutex::new(memory))),
            learning: Some(Arc::new(Mutex::new(learning))),
            governor: None,
            language_intelligence: None,
            processes: None,
            browser: None,
            runtime: None,
            permissions: None,
            task_contract: None,
        };

        let options = RunOptions {
            goal: "invariant zero extra coordinator calls".into(),
            cwd: dir.path().to_path_buf(),
            forced: Some(Complexity::Trivial),
            dry_run: false,
            abort: Arc::new(AtomicBool::new(false)),
            resume_artifacts: HashMap::new(),
            resume_run: None,
        };

        let run = run_graph(options, deps);
        assert_eq!(run.phase, Phase::Done);
        // Exactly 2 worker invocations for Trivial path (classifier + writer)
        // 0 coordinator preparation/model calls.
        assert_eq!(runner_invocations.load(Ordering::SeqCst), 2);
        assert_eq!(run.counters.workers_spawned, 2);
    }

    /// Task 0.1: Architectural regression gate for runtime migration.
    /// Asserts all 6 core ecosystem invariants:
    /// 1. zero added coordinator model calls;
    /// 2. Graph context remains <= 2500 estimated tokens;
    /// 3. Governor recovery stays byte-for-byte;
    /// 4. security failure blocks Graph approval;
    /// 5. learning outcome attribution still uses exact (name, version, content_hash);
    /// 6. cache-affinity retry key stays stable.
    #[test]
    fn runtime_migration_preserves_ecosystem_baseline() {
        // 1 & 2: Invariant testing: zero added coordinator calls & context <= 2,500
        let dir = tempdir().unwrap();
        let mem_dir = dir.path().join(".pi").join("vector-memory");
        std::fs::create_dir_all(&mem_dir).unwrap();
        let repo = resolve_repo_id(dir.path());
        let records = (0..5)
            .map(|i| {
                let rec = MemoryRecord {
                    id: format!("mem-baseline-{i}"),
                    repo_id: repo.clone(),
                    kind: MemoryKind::Decision,
                    text: format!("Baseline decision {i} text for context test"),
                    source: "assistant".into(),
                    content_hash: format!("hash-baseline-{i}"),
                    importance: 0.9,
                    created_at: 1000 + i as u64,
                    embedding: None,

                    embedding_identity: None,
                    confidence: None,
                    source_session_id: None,
                    source_turn: None,
                    verification: None,
                    source_paths: Vec::new(),
                    source_state_hash: None,
                    verified_at_revision: None,
                    use_count: 0,
                    last_used_at: None,
                    agent_profile_name: None,
                    memory_scope: None,
                };
                serde_json::to_string(&rec).unwrap()
            })
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(mem_dir.join("records.jsonl"), records).unwrap();

        let memory = VectorMemory::with_config(
            dir.path().to_path_buf(),
            VectorMemoryConfig {
                minimum_score: 0.1,
                ..VectorMemoryConfig::default()
            },
        );

        let skills_dir = dir.path().join(".pi").join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        for i in 0..5 {
            let s_dir = skills_dir.join(format!("baseline-skill-{i}"));
            std::fs::create_dir_all(&s_dir).unwrap();
            let content = format!(
                "---\nname: baseline-skill-{i}\ndescription: Baseline skill {i}\nroles: [researcher, writer]\n---\n# Baseline Skill {i}\nInstructions.\n"
            );
            std::fs::write(s_dir.join("SKILL.md"), content).unwrap();
        }

        let mut learning =
            crate::native_extensions::LearningController::new(dir.path(), None, None);
        let request = ContextPacketRequest::new("Baseline ecosystem test")
            .with_role(Role::Researcher)
            .with_token_cap(DEFAULT_GRAPH_CONTEXT_TOKENS);
        let packet = build_context_packet(&memory, &learning, request);
        assert!(
            packet.estimated_tokens <= 2_500,
            "Graph context packet must remain <= 2,500 estimated tokens, got {}",
            packet.estimated_tokens
        );

        // Zero added coordinator calls in Graph run
        let runner_invocations = Arc::new(AtomicUsize::new(0));
        let runner_clone = runner_invocations.clone();
        let runner: Arc<WorkerRunner> = Arc::new(move |spec, _abort, _on_progress| {
            runner_clone.fetch_add(1, Ordering::SeqCst);
            let artifact = match spec.expect {
                ArtifactKind::Classification => Artifact::Classification(Classification {
                    task_class: TaskClass::Trivial,
                    complexity: Complexity::Trivial,
                    rationale: "baseline run".into(),
                    research_tasks: vec![],
                    milestones: None,
                }),
                _ => Artifact::PatchReport(Box::new(PatchReport {
                    changed_files: vec![],
                    summary: "done".into(),
                    deviations: vec![],
                    plan_invalidated: false,
                    invalidation_reason: None,
                })),
            };
            WorkerResult {
                ok: true,
                artifact: Some(artifact),
                ..WorkerResult::default()
            }
        });

        let deps = ControllerDeps {
            runner,
            verify_exec: Arc::new(|_, _, _, _| (0, String::new(), 0)),
            config: GraphConfig {
                verify_commands: vec![VerifyCommandSpec {
                    command: "echo test".into(),
                    name: "test".into(),
                    from_plan: false,
                }],
                ..Default::default()
            },
            session_model: None,
            session_thinking: None,
            session_service_tier: davinci_ai::CodexServiceTier::Standard,
            project_trusted: false,
            on_update: Arc::new(|_, _| {}),
            memory: Some(Arc::new(Mutex::new(memory))),
            learning: Some(Arc::new(Mutex::new(
                crate::native_extensions::LearningController::new(dir.path(), None, None),
            ))),
            governor: None,
            language_intelligence: None,
            processes: None,
            browser: None,
            runtime: None,
            permissions: None,
            task_contract: None,
        };

        let options = RunOptions {
            goal: "baseline invariant check".into(),
            cwd: dir.path().to_path_buf(),
            forced: Some(Complexity::Trivial),
            dry_run: false,
            abort: Arc::new(AtomicBool::new(false)),
            resume_artifacts: HashMap::new(),
            resume_run: None,
        };
        let run = run_graph(options, deps);
        assert_eq!(run.phase, Phase::Done);
        assert_eq!(
            runner_invocations.load(Ordering::SeqCst),
            2,
            "Zero extra coordinator calls"
        );

        // 3: Governor recovery stays byte-for-byte
        let gov_dir = tempdir().unwrap();
        let gov_config = TokenGovernorConfig {
            enabled: true,
            compress_threshold_bytes: 50,
            store_dir: Some(gov_dir.path().to_path_buf()),
            ..TokenGovernorConfig::default()
        };
        let mut governor = TokenGovernor::new("baseline-session", gov_config);
        let original_output =
            "line 1: important output\nline 2: verbose data\nline 3: end of output\n".repeat(10);
        let initial_result = ToolResult {
            content: original_output.clone(),
            is_error: false,
            details: None,
        };
        let compressed =
            governor.after_tool("bash", &json!({"command": "cat test"}), initial_result);
        let output_id = compressed
            .details
            .as_ref()
            .and_then(|d| d.get("tokenGovernor"))
            .and_then(|tg| tg.get("outputId"))
            .and_then(|id| id.as_str())
            .expect("outputId present");
        let retrieved = governor
            .retrieve(&json!({"id": output_id}))
            .expect("retrieval succeeds");
        let reconstructed = retrieved
            .content
            .lines()
            .map(|l| {
                if let Some(pos) = l.find(": ") {
                    &l[pos + 2..]
                } else {
                    l
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        assert_eq!(
            reconstructed, original_output,
            "Governor recovery must be byte-for-byte"
        );

        // 4: Security failure blocks Graph approval
        let sec_dir = tempdir().unwrap();
        let auth_file = sec_dir.path().join("src/auth.rs");
        std::fs::create_dir_all(auth_file.parent().unwrap()).unwrap();
        std::fs::write(
            &auth_file,
            "pub fn key() -> &'static str { \"sk-proj-4f9Qa2Lk8Zt3Vb7Nc1Xd6Rm0Hs\" }\n",
        )
        .unwrap();
        let sec_runner: Arc<WorkerRunner> = Arc::new(|spec, _abort, _on_progress| {
            let artifact = match spec.expect {
                ArtifactKind::Classification => Artifact::Classification(Classification {
                    task_class: TaskClass::Trivial,
                    complexity: Complexity::Trivial,
                    rationale: "security test".into(),
                    research_tasks: vec![],
                    milestones: None,
                }),
                ArtifactKind::PatchReport => Artifact::PatchReport(Box::new(PatchReport {
                    changed_files: vec!["src/auth.rs".into()],
                    summary: "modified auth".into(),
                    deviations: vec![],
                    plan_invalidated: false,
                    invalidation_reason: None,
                })),
                _ => Artifact::Review(Box::new(ReviewDecision {
                    verdict: Verdict::Approve,
                    issues: vec![],
                    notes: "ok".into(),
                    reviewed_chunk_ids: vec![],
                })),
            };
            WorkerResult {
                ok: true,
                artifact: Some(artifact),
                ..WorkerResult::default()
            }
        });
        let sec_deps = ControllerDeps {
            runner: sec_runner,
            verify_exec: Arc::new(|_, _, _, _| (0, "ok".into(), 1)),
            config: GraphConfig {
                security_verification: SecurityPolicyMode::Always,
                verify_commands: vec![VerifyCommandSpec {
                    command: "echo test".into(),
                    name: "test".into(),
                    from_plan: false,
                }],
                ..Default::default()
            },
            session_model: None,
            session_thinking: None,
            session_service_tier: davinci_ai::CodexServiceTier::Standard,
            project_trusted: false,
            on_update: Arc::new(|_, _| {}),
            memory: None,
            learning: None,
            governor: None,
            language_intelligence: None,
            processes: None,
            browser: None,
            runtime: None,
            permissions: None,
            task_contract: None,
        };
        let sec_run = run_graph(
            RunOptions {
                goal: "sec check".into(),
                cwd: sec_dir.path().to_path_buf(),
                forced: Some(Complexity::Trivial),
                dry_run: false,
                abort: Arc::new(AtomicBool::new(false)),
                resume_artifacts: HashMap::new(),
                resume_run: None,
            },
            sec_deps,
        );
        assert_eq!(
            sec_run.phase,
            Phase::Blocked,
            "Security failure must block Graph approval"
        );

        // 5: Learning outcome attribution uses exact (name, version, content_hash)
        let sref = SkillVersionRef {
            name: "test-skill".into(),
            version: 2,
            content_hash: "exact-hash-123".into(),
        };
        let l_record = SkillLedgerRecord {
            skill_id: "test-skill-id".into(),
            name: sref.name.clone(),
            scope: LearningScope::Project,
            origin: SkillOrigin::LearnedReview,
            status: ArtifactStatus::Active,
            path: dir.path().join("SKILL.md"),
            content_hash: sref.content_hash.clone(),
            version: sref.version as u32,
            success_count: 0,
            failure_count: 0,
            neutral_count: 0,
            last_used_at_ms: None,
            created_at_ms: 1000,
            updated_at_ms: 1000,
            applicability: Default::default(),
            pinned: false,
        };
        learning.project_store.upsert_skill(l_record).unwrap();
        learning
            .record_skill_version_outcome(&sref, SkillOutcome::VerifiedSuccess)
            .unwrap();
        let updated_skill = learning.project_store.skill("test-skill").unwrap();
        assert_eq!(updated_skill.success_count, 1);
        assert_eq!(updated_skill.version, 2);
        assert_eq!(updated_skill.content_hash, "exact-hash-123");

        // 6: Cache-affinity retry key stays stable
        let key_a = derive_worker_cache_key(
            "/test",
            1,
            Role::Researcher,
            Some("m1"),
            &["read".into()],
            "p",
            ArtifactKind::Evidence,
        );
        let key_b = derive_worker_cache_key(
            "/test",
            1,
            Role::Researcher,
            Some("m1"),
            &["read".into()],
            "p",
            ArtifactKind::Evidence,
        );
        assert_eq!(
            key_a, key_b,
            "Cache affinity key must stay stable across retries"
        );
    }

    #[test]
    fn test_graph_worker_suppresses_memory_injection_when_flag_set() {
        let dir = tempdir().unwrap();
        let mem_dir = dir.path().join(".pi").join("vector-memory");
        std::fs::create_dir_all(&mem_dir).unwrap();
        let repo = resolve_repo_id(dir.path());
        let records = (0..5)
            .map(|i| {
                let rec = MemoryRecord {
                    id: format!("mem-suppress-{i}"),
                    repo_id: repo.clone(),
                    kind: MemoryKind::Decision,
                    text: format!("Memory record {i} for suppression test"),
                    source: "assistant".into(),
                    content_hash: format!("hash-suppress-{i}"),
                    importance: 0.9,
                    created_at: 1000 + i as u64,
                    embedding: None,

                    embedding_identity: None,
                    confidence: None,
                    source_session_id: None,
                    source_turn: None,
                    verification: None,
                    source_paths: Vec::new(),
                    source_state_hash: None,
                    verified_at_revision: None,
                    use_count: 0,
                    last_used_at: None,
                    agent_profile_name: None,
                    memory_scope: None,
                };
                serde_json::to_string(&rec).unwrap()
            })
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(mem_dir.join("records.jsonl"), records).unwrap();

        let memory = VectorMemory::with_config(
            dir.path().to_path_buf(),
            VectorMemoryConfig {
                minimum_score: 0.1,
                ..VectorMemoryConfig::default()
            },
        );

        let source =
            crate::native_extensions::vector_memory::MemoryContextSource::from_memory(memory);
        let broker = davinci_agent::runtime::context::ContextBroker::new();
        broker.register_context_source(Arc::new(source));

        let request = davinci_agent::runtime::context::ContextRequest {
            run_id: davinci_agent::RunId::new(),
            agent_id: davinci_agent::AgentId::new(),
            goal: "Memory record suppression test".to_string(),
            provider: "mock".to_string(),
            model_id: "mock".to_string(),
            tools: vec!["read".to_string()],
            max_tokens: 2500,
            kind: davinci_agent::runtime::events::AgentKind::GraphWorker,
            agent_profile_name: None,
            memory_scope: None,
        };

        // When flag is NOT set, items should be collected (capped at 4 hits, 1200 tokens)
        std::env::remove_var("PI_GRAPH_SUPPRESS_MEMORY_INJECT");
        let packet_normal = broker.build_context(&request);
        assert!(!packet_normal.items.is_empty());
        assert!(packet_normal.items.len() <= 4);
        assert!(packet_normal.estimated_tokens <= 1200);

        // When flag IS set to 1, no duplicate memory should appear (returns empty)
        std::env::set_var("PI_GRAPH_SUPPRESS_MEMORY_INJECT", "1");
        let packet_suppressed = broker.build_context(&request);
        std::env::remove_var("PI_GRAPH_SUPPRESS_MEMORY_INJECT");

        assert!(
            packet_suppressed.items.is_empty(),
            "Memory injection must be empty when PI_GRAPH_SUPPRESS_MEMORY_INJECT=1"
        );
        assert_eq!(packet_suppressed.estimated_tokens, 0);
    }

    #[test]
    fn test_broker_feeds_memory_and_skills_within_graph_bounds() {
        let dir = tempdir().unwrap();
        let mem_dir = dir.path().join(".pi").join("vector-memory");
        std::fs::create_dir_all(&mem_dir).unwrap();
        let repo = resolve_repo_id(dir.path());
        let records = (0..5)
            .map(|i| {
                let rec = MemoryRecord {
                    id: format!("mem-joint-{i}"),
                    repo_id: repo.clone(),
                    kind: MemoryKind::Decision,
                    text: format!("Joint test memory {i} context hit"),
                    source: "assistant".into(),
                    content_hash: format!("hash-joint-{i}"),
                    importance: 0.9,
                    created_at: 1000 + i as u64,
                    embedding: None,

                    embedding_identity: None,
                    confidence: None,
                    source_session_id: None,
                    source_turn: None,
                    verification: None,
                    source_paths: Vec::new(),
                    source_state_hash: None,
                    verified_at_revision: None,
                    use_count: 0,
                    last_used_at: None,
                    agent_profile_name: None,
                    memory_scope: None,
                };
                serde_json::to_string(&rec).unwrap()
            })
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(mem_dir.join("records.jsonl"), records).unwrap();

        let memory = VectorMemory::with_config(
            dir.path().to_path_buf(),
            VectorMemoryConfig {
                minimum_score: 0.1,
                ..VectorMemoryConfig::default()
            },
        );

        let skills_dir = dir.path().join(".pi").join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        for i in 0..3 {
            let s_dir = skills_dir.join(format!("joint-skill-{i}"));
            std::fs::create_dir_all(&s_dir).unwrap();
            let content = format!(
                "---\nname: joint-skill-{i}\ndescription: Joint skill {i}\nroles: [writer]\n---\n# Joint Skill {i}\nInstructions.\n"
            );
            std::fs::write(s_dir.join("SKILL.md"), content).unwrap();
        }
        let learning = crate::native_extensions::LearningController::new(dir.path(), None, None);

        let broker = davinci_agent::runtime::context::ContextBroker::new();
        broker.register_context_source(Arc::new(
            crate::native_extensions::vector_memory::MemoryContextSource::from_memory(memory),
        ));
        broker.register_context_source(Arc::new(
            crate::native_extensions::learning::SkillContextSource::from_controller(learning),
        ));

        let request = davinci_agent::runtime::context::ContextRequest {
            run_id: davinci_agent::RunId::new(),
            agent_id: davinci_agent::AgentId::new(),
            goal: "Joint test context hit".to_string(),
            provider: "mock".to_string(),
            model_id: "mock".to_string(),
            tools: vec!["read".to_string()],
            max_tokens: 2500,
            kind: davinci_agent::runtime::events::AgentKind::GraphWorker,
            agent_profile_name: None,
            memory_scope: None,
        };

        std::env::remove_var("PI_GRAPH_SUPPRESS_MEMORY_INJECT");
        let packet = broker.build_context(&request);
        assert!(!packet.items.is_empty());
        assert!(packet.estimated_tokens <= 2500);

        let mem_count = packet
            .items
            .iter()
            .filter(|i| i.source == "vector_memory")
            .count();
        let skill_count = packet
            .items
            .iter()
            .filter(|i| i.source == "learning_skills")
            .count();

        assert!(mem_count <= 4, "Memory hits must be <= 4");
        assert!(skill_count <= 2, "Skill candidates must be <= 2");
    }

    #[test]
    fn injected_irrelevant_skill_is_not_credited_as_helpful() {
        let dir = tempdir().unwrap();
        let skills_root = dir.path().join(".pi").join("skills");
        let learning_root = dir.path().join(".pi").join("learning");
        std::fs::create_dir_all(&skills_root).unwrap();
        std::fs::create_dir_all(&learning_root).unwrap();

        let mut lines = Vec::new();
        for (name, applicability) in [
            (
                "aaa-irrelevant",
                json!({
                    "languages": ["python"],
                    "taskTypes": [],
                    "pathGlobs": [],
                    "requiredSignals": [],
                    "verificationCategories": []
                }),
            ),
            (
                "zzz-relevant",
                json!({
                    "languages": ["rust"],
                    "taskTypes": ["debugging"],
                    "pathGlobs": [],
                    "requiredSignals": [],
                    "verificationCategories": []
                }),
            ),
        ] {
            let skill_dir = skills_root.join(name);
            std::fs::create_dir_all(&skill_dir).unwrap();
            let skill_file = skill_dir.join("SKILL.md");
            std::fs::write(
                &skill_file,
                format!(
                    "---\nname: {name}\ndescription: shared workflow rust debugging\nroles: [writer]\n---\n# Workflow\nRun verification.\n"
                ),
            )
            .unwrap();
            lines.push(
                json!({
                    "skillId": format!("skill-{name}"),
                    "name": name,
                    "scope": "project",
                    "origin": "learned_review",
                    "status": "active",
                    "path": skill_file,
                    "contentHash": format!("hash-{name}"),
                    "version": 1,
                    "successCount": 0,
                    "failureCount": 0,
                    "neutralCount": 0,
                    "lastUsedAtMs": null,
                    "createdAtMs": 1,
                    "updatedAtMs": 1,
                    "pinned": false,
                    "applicability": applicability
                })
                .to_string(),
            );
        }
        std::fs::write(learning_root.join("skills.jsonl"), lines.join("\n") + "\n").unwrap();

        let learning = crate::native_extensions::LearningController::new(dir.path(), None, None);
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/auth.rs"), "before").unwrap();
        let mutation_root = dir.path().to_path_buf();
        let runner: Arc<WorkerRunner> = Arc::new(move |spec, _abort, _on_progress| {
            if matches!(spec.expect, ArtifactKind::PatchReport) {
                std::fs::write(mutation_root.join("src/auth.rs"), "after").unwrap();
            }
            let artifact = match spec.expect {
                ArtifactKind::Classification => Artifact::Classification(Classification {
                    task_class: TaskClass::Trivial,
                    complexity: Complexity::Trivial,
                    rationale: "credit test".into(),
                    research_tasks: vec![],
                    milestones: None,
                }),
                ArtifactKind::PatchReport => Artifact::PatchReport(Box::new(PatchReport {
                    changed_files: vec!["src/auth.rs".into()],
                    summary: "debugged auth".into(),
                    deviations: vec![],
                    plan_invalidated: false,
                    invalidation_reason: None,
                })),
                _ => Artifact::Review(Box::new(ReviewDecision {
                    verdict: Verdict::Approve,
                    issues: vec![],
                    notes: "ok".into(),
                    reviewed_chunk_ids: vec![],
                })),
            };
            WorkerResult {
                ok: true,
                artifact: Some(artifact),
                ..WorkerResult::default()
            }
        });
        let deps = ControllerDeps {
            runner,
            verify_exec: Arc::new(|_, _, _, _| (0, "all tests pass".into(), 5)),
            config: GraphConfig {
                verify_commands: vec![VerifyCommandSpec {
                    command: "echo test".into(),
                    name: "test".into(),
                    from_plan: false,
                }],
                ..Default::default()
            },
            session_model: None,
            session_thinking: None,
            session_service_tier: davinci_ai::CodexServiceTier::Standard,
            project_trusted: true,
            on_update: Arc::new(|_, _| {}),
            memory: Some(Arc::new(Mutex::new(VectorMemory::new(
                dir.path().to_path_buf(),
            )))),
            learning: Some(Arc::new(Mutex::new(learning))),
            governor: None,
            language_intelligence: None,
            processes: None,
            browser: None,
            runtime: None,
            permissions: None,
            task_contract: None,
        };
        let run = run_graph(
            RunOptions {
                goal: "shared workflow rust debugging".into(),
                cwd: dir.path().to_path_buf(),
                forced: Some(Complexity::Trivial),
                dry_run: false,
                abort: Arc::new(AtomicBool::new(false)),
                resume_artifacts: HashMap::new(),
                resume_run: None,
            },
            deps,
        );
        assert_eq!(run.phase, Phase::Done);

        let reloaded = crate::native_extensions::LearningController::new(dir.path(), None, None);
        let relevant = reloaded.project_store.skill("zzz-relevant").unwrap();
        let irrelevant = reloaded.project_store.skill("aaa-irrelevant").unwrap();
        assert_eq!(relevant.success_count, 1);
        assert_eq!(irrelevant.success_count, 0);
        assert_eq!(irrelevant.neutral_count, 1);
    }
}
