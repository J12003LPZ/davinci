use super::*;

pub(crate) fn fixture(cwd: &Path) -> (WorkerSpec, WorkerSessionBinding) {
    let run_id = store::new_run_id();
    store::create_run_dir(cwd, &run_id).unwrap();
    let mut spec = WorkerSpec {
        task_id: "private-worker".into(),
        role: super::super::types::Role::Researcher,
        expect: super::super::types::ArtifactKind::Evidence,
        briefing: "retain this conversation".into(),
        system_prompt: "research fixture".into(),
        cwd: cwd.into(),
        model: None,
        thinking_level: None,
        tools: vec!["read".into(), "graph_submit".into()],
        authorized_tools: vec!["read".into(), "graph_submit".into()],
        initially_exposed_tools: vec!["read".into(), "graph_submit".into()],
        extra_extensions: vec![],
        timeout_ms: 0,
        run_deadline: None,
        artifact_path: store::artifact_path(cwd, &run_id, "private-worker"),
        transcript_path: None,
        project_trusted: false,
        runtime_agent_id: Some(AgentId::new()),
        worker_session: None,
        task_contract: None,
        coordinator_client: None,
        node_abort: None,
    };
    let binding = WorkerSessionBinding::create(&spec, &run_id, 1, 1, None, None).unwrap();
    spec.worker_session = Some(binding.clone());
    (spec, binding)
}

pub(crate) fn launch_args(spec: &WorkerSpec, scratch: &Path) -> crate::args::Args {
    let brief = scratch.join("briefing.md");
    let system = scratch.join("system.md");
    std::fs::write(&brief, &spec.briefing).unwrap();
    std::fs::write(
        &system,
        format!(
            "{}\n\n{}",
            spec.system_prompt,
            super::super::validate::artifact_contract(spec.expect)
        ),
    )
    .unwrap();
    crate::args::parse_args(&super::super::worker::build_worker_args(
        spec, &brief, &system,
    ))
}

#[test]
fn worker_binding_preserves_identity_and_refuses_changed_launch_inputs() {
    let dir = tempfile::tempdir().unwrap();
    let (spec, binding) = fixture(dir.path());
    binding.validate_spec(&spec).unwrap();
    let session = JsonlSession::open(&binding.session_path).unwrap();
    assert_eq!(
        Path::new(&session.header.cwd),
        dir.path().canonicalize().unwrap()
    );
    assert_eq!(session.header.id, binding.session_id);
    let args = launch_args(&spec, dir.path());
    assert!(args.diagnostics.is_empty(), "{:?}", args.diagnostics);
    binding.validate_args(&args).unwrap();
    let mut changed = spec.clone();
    changed.briefing.push_str(" different task");
    assert!(binding.validate_spec(&changed).is_err());
    let mut changed = args.clone();
    changed.model = Some("different-model".into());
    assert!(binding.validate_args(&changed).is_err());
    let mut changed = args.clone();
    changed.permission_mode = Some(davinci_agent::PermissionMode::Ask);
    assert!(binding.validate_args(&changed).is_err());
    std::fs::write(&args.file_args[0], "changed after publication").unwrap();
    assert!(binding.validate_args(&args).is_err());
    assert!(WorkerSessionBinding::create(&spec, &binding.graph_run, 1, 1, None, None).is_err());
}

#[test]
fn worker_binding_rejects_missing_history_and_foreign_manifest() {
    let dir = tempfile::tempdir().unwrap();
    let (spec, binding) = fixture(dir.path());
    let mut wrong = binding.clone();
    wrong.attempt += 1;
    assert!(wrong.validate(dir.path()).is_err());
    let other = tempfile::tempdir().unwrap();
    assert!(binding.validate(other.path()).is_err());
    std::fs::remove_file(&binding.session_path).unwrap();
    assert!(binding.validate(dir.path()).is_err());
    let result = super::super::worker::run_worker(
        &spec,
        &std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
        &mut |_, _| {},
    );
    assert!(result.recovery_required);
    assert!(
        !binding.session_path.exists(),
        "missing history must never be silently recreated"
    );
}

#[test]
fn worker_storage_rejects_an_existing_nonprivate_directory() {
    let dir = tempfile::tempdir().unwrap();
    let nonprivate = dir.path().join("unprotected");
    std::fs::create_dir(&nonprivate).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&nonprivate, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    assert!(private_directory(&nonprivate, true).is_err());
    let private = dir.path().join("private");
    private_directory(&private, true).unwrap();
    private_directory(&private, false).unwrap();
}

#[cfg(unix)]
#[test]
fn worker_storage_rejects_redirected_private_directories() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target");
    private_directory(&target, true).unwrap();
    let link = dir.path().join("link");
    std::os::unix::fs::symlink(&target, &link).unwrap();
    assert!(private_directory(&link, true).is_err());
}

#[test]
fn worker_retry_reuses_private_conversation_with_new_binding() {
    let dir = tempfile::tempdir().unwrap();
    let (mut spec, first) = fixture(dir.path());
    let mut session = JsonlSession::open(&first.session_path).unwrap();
    session
        .append_entry(davinci_session::SessionEntry::message(
            "assistant",
            serde_json::json!([{"type":"text","text":"retained from attempt one"}]),
        ))
        .unwrap();
    spec.briefing.push_str("\nretry with prior context");
    let second = WorkerSessionBinding::create(
        &spec,
        &first.graph_run,
        first.graph_revision + 1,
        2,
        None,
        Some(&first),
    )
    .unwrap();
    assert_eq!(second.agent, first.agent);
    assert_eq!(second.session_id, first.session_id);
    assert_eq!(second.session_path, first.session_path);
    assert_ne!(second.input_hash, first.input_hash);
    first.validate(dir.path()).unwrap();
    second.validate(dir.path()).unwrap();
    let reopened = JsonlSession::open(&second.session_path).unwrap();
    assert_eq!(reopened.entries.len(), 1);
}

#[test]
fn retry_safety_refuses_uncertain_or_unpersisted_mutations() {
    let dir = tempfile::tempdir().unwrap();
    let (_spec, binding) = fixture(dir.path());
    let ledger_path = binding.session_path.with_extension("tool-ledger.json");
    let mut ledger = davinci_agent::tool_ledger::ToolCallLedger::load_bound(
        &ledger_path,
        &binding.session_id,
    )
    .unwrap();
    ledger.record_start("write-1", "write", &serde_json::json!({"path":"changed.txt"}));
    let error = binding.validate_retry_safety().unwrap_err();
    assert!(error.contains("uncertain side effect"), "{error}");

    ledger.record_completion("write-1", "ok", false);
    ledger.persist().unwrap();
    let error = binding.validate_retry_safety().unwrap_err();
    assert!(error.contains("without a persisted tool result"), "{error}");

    let mut session = JsonlSession::open(&binding.session_path).unwrap();
    let mut result = davinci_session::SessionEntry::message(
        "toolResult",
        serde_json::json!([{"type":"text","text":"ok"}]),
    );
    result.message = Some(serde_json::json!({
        "role":"toolResult",
        "content":[{"type":"text","text":"ok"}],
        "toolCallId":"write-1"
    }));
    session.append_entry(result).unwrap();
    binding.validate_retry_safety().unwrap();
}
