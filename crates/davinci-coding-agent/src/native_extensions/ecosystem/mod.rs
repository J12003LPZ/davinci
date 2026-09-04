//! Bounded ecosystem integration contracts for Graph, Token Governor, Vector Memory, and Learning.

pub mod cache_affinity;
pub mod context;
pub mod resource;
pub mod verification;

#[allow(unused_imports)]
pub use cache_affinity::*;
#[allow(unused_imports)]
pub use context::*;
#[allow(unused_imports)]
pub use resource::*;
#[allow(unused_imports)]
pub use verification::*;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::native_extensions::graph::config::GraphConfig;
    use crate::native_extensions::graph::roles::{ensure_governor_recovery_tool, role_tools};
    use crate::native_extensions::graph::types::{
        Artifact, ArtifactKind, Classification, Complexity, EvidenceArtifact, PatchReport, Phase,
        ResearchKind, ReviewDecision, Role, TaskClass, Verdict, WorkerResult, WorkerSpec,
    };
    use crate::native_extensions::graph::worker::{build_worker_args, WorkerRunner};
    use crate::native_extensions::token_governor::{TokenGovernor, TokenGovernorConfig};
    use crate::native_extensions::vector_memory::{
        resolve_repo_id, MemoryKind, MemoryRecord, VectorMemory, VectorMemoryConfig,
    };
    use davinci_agent::ToolResult;
    use serde_json::json;
    use std::collections::HashMap;
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

    /// Step 1: Governor recovery loop fixture
    /// Prove graph role -> oversized output -> digest -> retrievable original.
    #[test]
    fn test_governor_recovery_loop_fixture() {
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
        use std::fmt::Write as _;
        let mut large_output = String::new();
        for i in 0..50 {
            let _ = writeln!(
                large_output,
                "line {i}: payload with extensive telemetry data for testing governor compression"
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

    /// Step 2: Cache-affinity loop fixture
    /// Compatible retry => same key. Changed model/toolset/contract => different key. `session_id` remains `None`.
    #[test]
    fn test_cache_affinity_loop_fixture() {
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

    /// Step 3: Memory/skill packet cap fixture
    /// Assert packet estimated_tokens <= 2_500, memory hits <= 4, skills <= 2.
    #[test]
    fn test_memory_skill_packet_caps_fixture() {
        let dir = tempdir().unwrap();

        // 1. Populate 10 memory records on disk
        let mem_dir = dir.path().join(".pi").join("vector-memory");
        std::fs::create_dir_all(&mem_dir).unwrap();
        let repo = resolve_repo_id(dir.path());
        let records = (0..10)
            .map(|i| {
                let rec = MemoryRecord {
                    id: format!("mem-test-{:03}", i),
                    repo_id: repo.clone(),
                    kind: MemoryKind::Discovery,
                    text: format!("Authentication service config details for database {}", i),
                    source: "user".into(),
                    content_hash: format!("hash-{}", i),
                    importance: 0.8,
                    created_at: 1000 + i as u64,
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
                minimum_score: 0.2,
                ..VectorMemoryConfig::default()
            },
        );

        // 2. Populate 5 skills in .pi/skills
        let skills_dir = dir.path().join(".pi").join("skills");
        std::fs::create_dir_all(&skills_dir).unwrap();
        for i in 0..5 {
            let skill_dir = skills_dir.join(format!("auth-skill-{i}"));
            std::fs::create_dir_all(&skill_dir).unwrap();
            let content = format!(
                "---\nname: auth-skill-{i}\ndescription: Authentication service configuration details\nroles: [researcher, writer]\n---\n# Auth Skill {i}\nExtensive instructions for authentication.\n{}",
                "step detail line for authentication procedures\n".repeat(30)
            );
            std::fs::write(skill_dir.join("SKILL.md"), content).unwrap();
        }

        let learning = crate::native_extensions::LearningController::new(dir.path(), None, None);

        let request = ContextPacketRequest::new("Authentication service config details")
            .with_role(Role::Researcher)
            .with_token_cap(DEFAULT_GRAPH_CONTEXT_TOKENS);

        let packet = build_context_packet(&memory, &learning, request);

        assert!(!packet.is_empty());
        assert!(
            packet.estimated_tokens <= DEFAULT_GRAPH_CONTEXT_TOKENS,
            "Estimated tokens must not exceed aggregate cap 2500"
        );
        assert!(
            packet.memory_refs.len() <= DEFAULT_GRAPH_MEMORY_HITS,
            "Memory hits must not exceed 4"
        );
        assert!(
            packet.skill_refs.len() <= DEFAULT_GRAPH_SKILL_COUNT,
            "Skill count must not exceed 2"
        );

        let text = &packet.text;
        assert!(text.starts_with("<context source=\"davinci\" untrusted=\"true\">"));
        assert!(text.ends_with("</context>"));
    }

    /// Step 4: Zero extra model calls assertion
    /// Verify context construction, cache key derivation, and snapshotting incur zero model calls.
    #[test]
    fn test_zero_extra_model_calls_fixture() {
        use crate::native_extensions::graph::controller::{run_graph, ControllerDeps, RunOptions};

        let dir = tempdir().unwrap();
        let memory = VectorMemory::new(dir.path().to_path_buf());
        let learning = crate::native_extensions::LearningController::new(dir.path(), None, None);

        // Building context packet executes purely locally with 0 model calls
        let req = ContextPacketRequest::new("offline prompt").with_role(Role::Writer);
        let packet = build_context_packet(&memory, &learning, req);
        assert!(packet.is_empty() || !packet.fingerprint.is_empty());

        // Cache affinity derivation is a deterministic pure hash calculation (0 model calls)
        let key = derive_worker_cache_key(
            "/path",
            1,
            Role::Classifier,
            None,
            &[],
            "system prompt",
            ArtifactKind::Classification,
        );
        assert!(!key.is_empty());

        // Resource snapshot is a pure memory counter aggregation (0 model calls)
        let snapshot = ResourceSnapshot::collect(&[], None);
        assert_eq!(snapshot.cost_usd, 0.0);

        // Assert in a live graph run that worker invocation count equals topology nodes
        // and zero coordinator/preparation model calls occur.
        let runner_invocations = Arc::new(AtomicUsize::new(0));
        let runner_invocations_clone = runner_invocations.clone();
        let runner: Arc<WorkerRunner> = Arc::new(move |spec, _abort, _on_progress| {
            runner_invocations_clone.fetch_add(1, Ordering::SeqCst);
            let artifact = match spec.expect {
                ArtifactKind::Classification => Artifact::Classification(Classification {
                    task_class: TaskClass::Trivial,
                    complexity: Complexity::Trivial,
                    rationale: "trivial test".into(),
                    research_tasks: vec![],
                    milestones: None,
                }),
                ArtifactKind::PatchReport => Artifact::PatchReport(Box::new(PatchReport {
                    changed_files: vec![],
                    summary: "fixed".into(),
                    deviations: vec![],
                    plan_invalidated: false,
                    invalidation_reason: None,
                })),
                ArtifactKind::Review => Artifact::Review(Box::new(ReviewDecision {
                    verdict: Verdict::Approve,
                    issues: vec![],
                    notes: "ok".into(),
                    reviewed_chunk_ids: vec![],
                })),
                _ => Artifact::Evidence(Box::new(EvidenceArtifact {
                    kind: ResearchKind::CodeSearch,
                    findings: vec![],
                    risks: vec![],
                    gaps: vec![],
                    test_baseline: None,
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
            memory: Some(memory),
            learning: Some(learning),
            governor: None,
        };

        let options = RunOptions {
            goal: "zero model call proof".into(),
            cwd: dir.path().to_path_buf(),
            forced: Some(Complexity::Trivial),
            dry_run: false,
            abort: Arc::new(AtomicBool::new(false)),
            resume_artifacts: HashMap::new(),
        };

        let run = run_graph(options, deps);
        assert_eq!(run.phase, Phase::Done);
        assert_eq!(run.counters.workers_spawned, 2);
        assert_eq!(runner_invocations.load(Ordering::SeqCst), 2);
        // Resource snapshot is automatically populated
        assert!(run.resource_snapshot.is_some());
    }
}
