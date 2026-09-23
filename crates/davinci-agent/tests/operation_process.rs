use davinci_agent::jobs::supervisor::{
    self, ProcessConfig, ProcessEvent, ProcessLaunchState, Supervisor, SupervisorCommand,
};
use davinci_agent::runtime::operations::{
    AttemptId, CallerType, ExecutionOwner, ExecutionOwnerId, JournalId, JournalIdentity,
    OperationAdmission, OperationContext, OperationJournal, OperationState,
    ProcessOperationBinding, RootNamespaceId, ToolOperationDispatcher, ToolOperationPlanner,
    WorkspaceId, WorkspaceIdentity,
};
use davinci_agent::runtime::{AgentId, RunId};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::Arc;
use tempfile::TempDir;

struct Fixture {
    _temp: TempDir,
    workspace: PathBuf,
    journal: Arc<OperationJournal>,
    dispatcher: ToolOperationDispatcher,
    context: OperationContext,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let temp_root = std::fs::canonicalize(temp.path()).unwrap();
        let workspace = temp_root.join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let root = RootNamespaceId::new();
        let identity = JournalIdentity::new(
            JournalId::new(),
            WorkspaceIdentity {
                id: WorkspaceId::new(),
                binding_version: 1,
            },
        )
        .unwrap();
        let owner = ExecutionOwner::new(ExecutionOwnerId::new(), 1).unwrap();
        let journal = Arc::new(
            OperationJournal::open(&temp_root.join("journal"), identity.clone(), root).unwrap(),
        );
        let context = OperationContext {
            journal_id: identity.journal_id,
            root_namespace_id: root,
            session_id: "operation-process-tests".into(),
            runtime_run_id: RunId::new(),
            parent_operation_id: None,
            agent_id: AgentId::new(),
            worker_id: None,
            task_id: None,
            graph: None,
            workspace: identity.workspace,
            caller: CallerType::ProviderToolCall,
            wire_tool_call_id: None,
        };
        Self {
            _temp: temp,
            workspace,
            dispatcher: ToolOperationDispatcher::new(journal.clone(), owner),
            journal,
            context,
        }
    }

    fn admit(
        &self,
        call_id: &str,
        tool: &str,
    ) -> davinci_agent::runtime::operations::AdmittedOperation {
        let plan = ToolOperationPlanner::provider_call(
            self.context.clone(),
            call_id,
            tool,
            &serde_json::json!({"command":"fixture"}),
            None,
            None,
        )
        .unwrap();
        match self.dispatcher.admit(plan, 1).unwrap() {
            OperationAdmission::New(admitted) => admitted,
            other => panic!("expected a new operation, got {other:?}"),
        }
    }
}

fn supervisor_host(test_name: &str) -> SupervisorCommand {
    SupervisorCommand {
        executable: std::env::current_exe().unwrap(),
        argv: vec!["--exact".into(), test_name.into(), "--nocapture".into()],
    }
}

fn config(executable: PathBuf, argv: Vec<String>, cwd: &Path) -> ProcessConfig {
    ProcessConfig::new(executable, argv, cwd.into(), BTreeMap::new())
}

#[test]
fn operation_process_supervisor_fixture() {
    if std::env::var_os("DAVINCI_INTERNAL_PROCESS_SUPERVISOR").is_some() {
        supervisor::run();
    }
}

#[test]
fn operation_process_child_fixture() {
    let Some(path) = std::env::var_os("DAVINCI_PROCESS_CHILD_MARKER") else {
        return;
    };
    std::fs::write(path, b"launched").unwrap();
    print!("partial stdout");
    eprint!("partial stderr");
}

#[test]
fn operation_process_missing_ack_host_fixture() {
    if std::env::var_os("DAVINCI_INTERNAL_PROCESS_SUPERVISOR").is_none() {
        return;
    }
    let mut stdout = std::io::stdout().lock();
    stdout.write_all(b"DAVINCI_PROCESS_V2\n").unwrap();
    write_frame(
        &mut stdout,
        &serde_json::json!({"kind":"hello", "pid":std::process::id()}),
    );
    drop(stdout);

    let mut stdin = std::io::stdin().lock();
    let request: serde_json::Value = read_frame(&mut stdin);
    let config: ProcessConfig = serde_json::from_value(request["config"].clone()).unwrap();
    let mut child = Command::new(config.executable)
        .args(config.argv)
        .current_dir(config.cwd)
        .env_clear()
        .envs(config.environment)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    child.wait().unwrap();
    // The child effect occurred, but this helper deliberately withholds the
    // Started acknowledgement. The host must classify that as unknown.
}

fn write_frame(output: &mut impl Write, value: &serde_json::Value) {
    let bytes = serde_json::to_vec(value).unwrap();
    output
        .write_all(&(bytes.len() as u32).to_be_bytes())
        .unwrap();
    output.write_all(&bytes).unwrap();
    output.flush().unwrap();
}

fn read_frame(input: &mut impl Read) -> serde_json::Value {
    let mut header = [0; 4];
    input.read_exact(&mut header).unwrap();
    let mut bytes = vec![0; u32::from_be_bytes(header) as usize];
    input.read_exact(&mut bytes).unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

#[test]
fn shell_entrypoints_are_latched_before_adapter_execution() {
    for (index, tool) in ["bash", "powershell", "exec_command"]
        .into_iter()
        .enumerate()
    {
        let fixture = Fixture::new();
        let admitted = fixture.admit(&format!("call-{index}"), tool);
        let attempt_id: AttemptId = admitted.attempt.attempt_id();
        let plan = ToolOperationPlanner::provider_call(
            fixture.context.clone(),
            &format!("call-{index}"),
            tool,
            &serde_json::json!({"command":"fixture"}),
            None,
            None,
        )
        .unwrap();
        fixture
            .dispatcher
            .dispatch(
                &admitted,
                &plan,
                Some(1),
                false,
                || Ok(()),
                || {
                    assert_eq!(
                        fixture.journal.load_attempt(attempt_id).unwrap().state(),
                        OperationState::EffectPossible,
                        "{tool} must not contact a process helper before the effect latch"
                    );
                },
            )
            .unwrap();
    }
}

#[test]
fn launch_ack_and_exit_are_bound_to_the_operation_lifetime() {
    let fixture = Fixture::new();
    let admitted = fixture.admit("bound-launch", "exec_command");
    let binding = ProcessOperationBinding::from_admitted(&admitted);
    let marker = fixture.workspace.join("launched.marker");
    let mut environment = BTreeMap::new();
    environment.insert(
        "DAVINCI_PROCESS_CHILD_MARKER".into(),
        marker.to_string_lossy().into_owned(),
    );
    environment.insert(
        "API_TOKEN".into(),
        "credential-value-must-not-be-evidence".into(),
    );
    let config = config(
        std::env::current_exe().unwrap(),
        vec![
            "--exact".into(),
            "operation_process_child_fixture".into(),
            "--nocapture".into(),
        ],
        &fixture.workspace,
    )
    .with_operation_binding(binding.clone())
    .with_environment(environment);
    let event_config = config.clone();
    let output = Arc::new(std::sync::Mutex::new(Vec::new()));
    let output_clone = output.clone();
    let process = Supervisor::spawn(
        &supervisor_host("operation_process_supervisor_fixture"),
        config,
        Arc::new(move |event| {
            if let ProcessEvent::Output(bytes) = event {
                output_clone.lock().unwrap().extend(bytes);
            }
        }),
    )
    .unwrap();
    let exit = process.wait(std::time::Duration::from_secs(5)).unwrap();
    assert_eq!(exit.code, Some(0));
    assert!(exit.output_complete);
    assert_eq!(process.identity(), &exit.identity);
    assert_eq!(exit.identity.operation.as_ref(), Some(&binding));
    assert!(marker.exists());
    let observed = String::from_utf8_lossy(&output.lock().unwrap()).to_string();
    assert!(observed.contains("partial stdout"));
    assert!(observed.contains("partial stderr"));
    let evidence = event_config.execution_evidence(
        process.identity().clone(),
        ProcessLaunchState::Exited,
        exit.code,
        Some(exit.output_complete),
    );
    let serialized = serde_json::to_string(&evidence).unwrap();
    assert!(!serialized.contains("credential-value-must-not-be-evidence"));
    assert!(serialized.contains("API_TOKEN"));
}

#[test]
fn launch_failure_and_missing_ack_have_distinct_certainty() {
    let fixture = Fixture::new();
    let admitted = fixture.admit("launch-failure", "exec_command");
    let failed_config = config(
        fixture.workspace.join("not-an-executable"),
        Vec::new(),
        &fixture.workspace,
    )
    .with_operation_binding(ProcessOperationBinding::from_admitted(&admitted));
    let failure = Supervisor::spawn(
        &supervisor_host("operation_process_supervisor_fixture"),
        failed_config,
        Arc::new(|_| {}),
    )
    .unwrap_err();
    assert_eq!(
        failure.launch_state(),
        ProcessLaunchState::FailedBeforeChild
    );

    let marker = fixture.workspace.join("missing-ack.marker");
    let mut config = config(
        std::env::current_exe().unwrap(),
        vec![
            "--exact".into(),
            "operation_process_child_fixture".into(),
            "--nocapture".into(),
        ],
        &fixture.workspace,
    );
    config.environment.insert(
        "DAVINCI_PROCESS_CHILD_MARKER".into(),
        marker.to_string_lossy().into_owned(),
    );
    let missing_ack = Supervisor::spawn(
        &supervisor_host("operation_process_missing_ack_host_fixture"),
        config.with_operation_binding(ProcessOperationBinding::from_admitted(&admitted)),
        Arc::new(|_| {}),
    )
    .unwrap_err();
    assert_eq!(missing_ack.launch_state(), ProcessLaunchState::Unknown);
    assert!(
        marker.exists(),
        "the child ran before the acknowledgement was lost"
    );
}
