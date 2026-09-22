use davinci_agent::AgentId;
use davinci_coding_agent::native_extensions::ecosystem::{
    WorkerContextQuery, DEFAULT_GRAPH_CONTEXT_TOKENS, WORKER_CONTEXT_QUERY_MAX_CHARS,
};
use davinci_coding_agent::native_extensions::graph::{
    build_worker_args, resolve_worker_model_identity, worker_cache_profile, ArtifactKind, Role,
    WorkerSessionBinding, WorkerSpec, WorkerUsage,
};
use davinci_session::{JsonlSession, SessionEntry};
use std::path::Path;

fn spec(cwd: &Path, model: Option<&str>) -> WorkerSpec {
    WorkerSpec {
        task_id: "cache-worker".into(),
        role: Role::Researcher,
        expect: ArtifactKind::Evidence,
        briefing: "Investigate only src/cache.rs; do not clone parent transcript".into(),
        system_prompt: "Stable researcher bootstrap".into(),
        cwd: cwd.to_path_buf(),
        model: model.map(str::to_string),
        thinking_level: Some("medium".into()),
        tools: vec!["read".into(), "grep".into(), "graph_submit".into()],
        authorized_tools: vec![
            "read".into(),
            "grep".into(),
            "graph_submit".into(),
            "retrieve_output".into(),
        ],
        initially_exposed_tools: vec!["read".into(), "grep".into(), "graph_submit".into()],
        extra_extensions: Vec::new(),
        timeout_ms: 0,
        run_deadline: None,
        artifact_path: cwd.join("artifact.json"),
        transcript_path: None,
        project_trusted: false,
        runtime_agent_id: Some(AgentId::new()),
        worker_session: None,
        task_contract: None,
        coordinator_client: None,
        node_abort: None,
    }
}

fn create_run_dirs(cwd: &Path, run_id: &str) {
    let root = cwd.join(".davinci").join("graph").join("runs").join(run_id);
    std::fs::create_dir_all(root.join("artifacts")).unwrap();
    std::fs::create_dir_all(root.join("logs")).unwrap();
}

#[test]
fn worker_model_resolution_never_invents_default_identity() {
    assert_eq!(
        resolve_worker_model_identity(Some("openai/gpt-5.6-sol"), Some("openai/gpt-5.6-luna"))
            .as_deref(),
        Some("openai/gpt-5.6-sol")
    );
    assert_eq!(
        resolve_worker_model_identity(None, Some("openai/gpt-5.6-luna")).as_deref(),
        Some("openai/gpt-5.6-luna")
    );
    assert_eq!(
        resolve_worker_model_identity(Some("   "), Some("openai/gpt-5.6-luna")).as_deref(),
        Some("openai/gpt-5.6-luna")
    );
    assert_eq!(resolve_worker_model_identity(None, None), None);
}

#[test]
fn stable_bootstrap_affinity_excludes_variable_goal_but_includes_model_and_authority() {
    let dir = tempfile::tempdir().unwrap();
    let a = spec(dir.path(), Some("openai/gpt-5.6-sol"));
    let mut b = a.clone();
    b.briefing = "A different variable node objective".into();

    let a_profile = worker_cache_profile(&a);
    let b_profile = worker_cache_profile(&b);
    assert_eq!(a_profile.cache_key, b_profile.cache_key);
    assert_eq!(
        a_profile.stable_bootstrap_fingerprint,
        b_profile.stable_bootstrap_fingerprint
    );
    assert_ne!(
        a_profile.variable_goal_fingerprint,
        b_profile.variable_goal_fingerprint
    );

    let mut changed_model = a.clone();
    changed_model.model = Some("openai/gpt-5.6-luna".into());
    assert_ne!(
        a_profile.cache_key,
        worker_cache_profile(&changed_model).cache_key
    );

    let mut changed_authority = a.clone();
    changed_authority
        .authorized_tools
        .push("workspace_snapshot".into());
    assert_ne!(
        a_profile.cache_key,
        worker_cache_profile(&changed_authority).cache_key
    );

    let unresolved = spec(dir.path(), None);
    assert_eq!(worker_cache_profile(&unresolved).cache_key, None);
}

#[test]
fn worker_launch_keeps_system_bootstrap_and_variable_briefing_separate() {
    let dir = tempfile::tempdir().unwrap();
    let spec = spec(dir.path(), Some("openai/gpt-5.6-sol"));
    let system = dir.path().join("system.md");
    let briefing = dir.path().join("briefing.md");
    std::fs::write(&system, "SYSTEM-BOOTSTRAP-ONLY").unwrap();
    std::fs::write(&briefing, "VARIABLE-GOAL-ONLY").unwrap();

    let args = build_worker_args(&spec, &briefing, &system);
    let joined = args.join("\n");
    assert!(joined.contains(system.to_string_lossy().as_ref()));
    assert!(joined.contains(&format!("@{}", briefing.to_string_lossy())));
    assert!(!joined.contains("SYSTEM-BOOTSTRAP-ONLY"));
    assert!(!joined.contains("VARIABLE-GOAL-ONLY"));
    assert!(args.iter().any(|arg| arg == "--no-skills"));
    assert!(args.iter().any(|arg| arg == "--no-prompt-templates"));
}

#[test]
fn worker_context_query_is_bounded_and_does_not_clone_unrelated_parent_history() {
    let parent_secret = "PARENT-TRANSCRIPT-MUST-NOT-BE-CLONED";
    let query = WorkerContextQuery {
        role: Some(Role::Researcher),
        node_objective: "x".repeat(8_000),
        graph_goal: "inspect the cache boundary".into(),
        target_hints: vec!["src/cache.rs".into()],
        failure_hint: None,
    }
    .render();

    assert!(query.chars().count() <= WORKER_CONTEXT_QUERY_MAX_CHARS);
    assert_eq!(DEFAULT_GRAPH_CONTEXT_TOKENS, 2_500);
    assert!(!query.contains(parent_secret));
    assert!(query.contains("src/cache.rs"));
}

#[test]
fn streaming_and_final_worker_usage_are_counted_exactly_once() {
    let first = WorkerUsage {
        input: 100,
        output: 20,
        cache_read: 40,
        cache_write: 10,
        cost_usd: 0.1,
        turns: 1,
    };
    let final_usage = WorkerUsage {
        input: 175,
        output: 35,
        cache_read: 90,
        cache_write: 25,
        cost_usd: 0.25,
        turns: 2,
    };
    let mut total = WorkerUsage::default();
    total.add(&WorkerUsage::delta(&first, &WorkerUsage::default()));
    total.add(&WorkerUsage::delta(&final_usage, &first));
    assert_eq!(total, final_usage);
}

#[test]
fn valid_retry_reuses_private_worker_conversation_and_uncertain_mutation_blocks() {
    let dir = tempfile::tempdir().unwrap();
    let run_id = "cache-worker-run";
    create_run_dirs(dir.path(), run_id);
    let mut spec = spec(dir.path(), Some("openai/gpt-5.6-sol"));

    let first = WorkerSessionBinding::create(&spec, run_id, 1, 1, None, None).unwrap();
    let mut session = JsonlSession::open(&first.session_path).unwrap();
    session
        .append_entry(SessionEntry::message(
            "assistant",
            serde_json::json!([{"type":"text","text":"retained worker context"}]),
        ))
        .unwrap();

    spec.briefing.push_str("\nretry delta");
    let second = WorkerSessionBinding::create(&spec, run_id, 2, 2, None, Some(&first)).unwrap();
    assert_eq!(second.session_id, first.session_id);
    assert_eq!(second.session_path, first.session_path);
    assert_eq!(second.agent, first.agent);
    assert_ne!(second.input_hash, first.input_hash);

    let ledger_path = second.session_path.with_extension("tool-ledger.json");
    let mut ledger =
        davinci_agent::ToolCallLedger::load_bound(&ledger_path, &second.session_id).unwrap();
    ledger.record_start(
        "write-1",
        "write",
        &serde_json::json!({"path":"changed.txt"}),
    );
    let error = second.validate_retry_safety().unwrap_err();
    assert!(error.contains("uncertain side effect"), "{error}");
}
