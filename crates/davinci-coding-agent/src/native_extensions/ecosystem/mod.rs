//! Bounded ecosystem integration contracts for Graph, Token Governor, Vector Memory, and Learning.

pub mod cache_affinity;
pub mod context;
pub mod resource;
pub mod risk;
pub mod telemetry;
pub mod verification;

#[allow(unused_imports)]
pub use cache_affinity::*;
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
    use std::sync::Arc;
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
            task_id: "test-task".into(),
            role: Role::Researcher,
            tools: tools.clone(),
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
            confidence: None,
            source_session_id: Some("session-123".into()),
            source_turn: Some(5),
            verification: None,
            use_count: 0,
            last_used_at: None,
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
            "pub fn key() -> &'static str { \"sk-secret12345\" }\n",
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
            project_trusted: false,
            on_update: Arc::new(|_, _| {}),
            memory: Some(memory),
            learning: Some(learning),
            governor: None,
        };

        let options = RunOptions {
            goal: "high risk security gate proof".into(),
            cwd: dir.path().to_path_buf(),
            forced: Some(Complexity::Trivial),
            dry_run: false,
            abort: Arc::new(AtomicBool::new(false)),
            resume_artifacts: HashMap::new(),
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

    /// Task 4 Step 7: ecosystem_loop_full_circle
    /// Graph Run #1 -> verification -> learning persistence -> Graph Run #2 receives persisted learning -> Run #2 verification -> outcome ledger update.
    #[test]
    fn ecosystem_loop_full_circle() {
        let dir = tempdir().unwrap();

        // 1. Initial candidate skill persisted and approved (representing Run #1 outcome)
        let skill_dir = dir
            .path()
            .join(".pi")
            .join("skills")
            .join("full-circle-skill");
        std::fs::create_dir_all(&skill_dir).unwrap();
        let content = "---\nname: full-circle-skill\ndescription: Full circle offline ecosystem integration skill\nroles: [writer]\n---\n# Full Circle\nFollow strict integration verification.\n";
        let skill_file = skill_dir.join("SKILL.md");
        std::fs::write(&skill_file, content).unwrap();

        let mut learning =
            crate::native_extensions::LearningController::new(dir.path(), None, None);
        learning.set_project_trusted(true);
        let record = SkillLedgerRecord {
            skill_id: "skill-fc-01".into(),
            name: "full-circle-skill".into(),
            scope: LearningScope::Project,
            origin: SkillOrigin::LearnedReview,
            status: ArtifactStatus::Active,
            path: skill_file,
            content_hash: "hash-fc-01".into(),
            version: 1,
            success_count: 0,
            failure_count: 0,
            neutral_count: 0,
            last_used_at_ms: None,
            created_at_ms: 1000,
            updated_at_ms: 1000,
            pinned: false,
        };
        learning.project_store.upsert_skill(record).unwrap();

        // 2. Graph Run #2 starts with this learning controller
        let runner: Arc<WorkerRunner> = Arc::new(|spec, _abort, _on_progress| {
            let artifact = match spec.expect {
                ArtifactKind::Classification => Artifact::Classification(Classification {
                    task_class: TaskClass::Trivial,
                    complexity: Complexity::Trivial,
                    rationale: "full circle run 2".into(),
                    research_tasks: vec![],
                    milestones: None,
                }),
                ArtifactKind::PatchReport => Artifact::PatchReport(Box::new(PatchReport {
                    changed_files: vec!["src/lib.rs".into()],
                    summary: "applied full circle skill".into(),
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

        let memory = VectorMemory::new(dir.path().to_path_buf());
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
            project_trusted: true,
            on_update: Arc::new(|_, _| {}),
            memory: Some(memory),
            learning: Some(learning),
            governor: None,
        };

        let options = RunOptions {
            goal: "full circle offline integration test with full-circle-skill".into(),
            cwd: dir.path().to_path_buf(),
            forced: Some(Complexity::Trivial),
            dry_run: false,
            abort: Arc::new(AtomicBool::new(false)),
            resume_artifacts: HashMap::new(),
        };

        let run = run_graph(options, deps);

        // 3. Run #2 succeeds cleanly
        assert_eq!(run.phase, Phase::Done);
        assert!(run.verification.as_ref().map(|v| v.passed).unwrap_or(false));

        // 4. Learning controller state in project store updated with success
        let reloaded_learning =
            crate::native_extensions::LearningController::new(dir.path(), None, None);
        let updated_skill = reloaded_learning
            .project_store
            .skill("full-circle-skill")
            .expect("full-circle-skill must exist in store");
        assert_eq!(updated_skill.success_count, 1);
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
                    id: format!("mem-invariant-{:03}", i),
                    repo_id: repo.clone(),
                    kind: MemoryKind::Discovery,
                    text: format!(
                        "Invariant memory entry {} with lengthy verbose text that consumes plenty of prompt space across multiple sentences to test truncation.",
                        i
                    ),
                    source: "user".into(),
                    content_hash: format!("hash-{}", i),
                    importance: 0.85,
                    created_at: 2000 + i as u64,
                    embedding: None,
                    confidence: None,
                    source_session_id: None,
                    source_turn: None,
                    verification: None,
                    use_count: 0,
                    last_used_at: None,
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
            project_trusted: false,
            on_update: Arc::new(|_, _| {}),
            memory: Some(memory),
            learning: Some(learning),
            governor: None,
        };

        let options = RunOptions {
            goal: "invariant zero extra coordinator calls".into(),
            cwd: dir.path().to_path_buf(),
            forced: Some(Complexity::Trivial),
            dry_run: false,
            abort: Arc::new(AtomicBool::new(false)),
            resume_artifacts: HashMap::new(),
        };

        let run = run_graph(options, deps);
        assert_eq!(run.phase, Phase::Done);
        // Exactly 2 worker invocations for Trivial path (classifier + writer)
        // 0 coordinator preparation/model calls.
        assert_eq!(runner_invocations.load(Ordering::SeqCst), 2);
        assert_eq!(run.counters.workers_spawned, 2);
    }
}
