use super::*;

pub fn fixture(cwd: &Path) -> (WorkerSpec, WorkerSessionBinding) {
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

pub fn launch_args(spec: &WorkerSpec, scratch: &Path) -> crate::args::Args {
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
