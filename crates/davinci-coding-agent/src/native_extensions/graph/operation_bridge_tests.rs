use super::operation_bridge::*;
use super::store;
use super::types::{ArtifactKind, Role, WorkerResult, WorkerSpec};
use super::worker_sessions::WorkerSessionBinding;
use davinci_agent::runtime::operations::{
    ExecutionOwner, ExecutionOwnerId, JournalId, JournalIdentity, OperationJournal, OperationState,
    RootNamespaceId, ToolOperationRuntime, WorkspaceId, WorkspaceIdentity,
};
use davinci_agent::{AgentId, RunId, RuntimeBus, RuntimeHandle};
use std::path::Path;
use std::sync::Arc;

struct Fixture {
    runtime: RuntimeHandle,
    workspace: tempfile::TempDir,
    graph_run_id: String,
    task_id: String,
    binding: WorkerSessionBinding,
}

fn runtime(workspace: &Path, run_id: RunId, agent_id: AgentId) -> RuntimeHandle {
    let identity = JournalIdentity::new(
        JournalId::new(),
        WorkspaceIdentity {
            id: WorkspaceId::new(),
            binding_version: 1,
        },
    )
    .unwrap();
    let root = RootNamespaceId::new();
    let context = davinci_agent::runtime::operations::OperationContext {
        journal_id: identity.journal_id,
        root_namespace_id: root,
        session_id: format!("graph-test-{agent_id}"),
        runtime_run_id: run_id,
        parent_operation_id: None,
        agent_id,
        worker_id: None,
        task_id: None,
        graph: None,
        workspace: identity.workspace.clone(),
        caller: davinci_agent::runtime::operations::CallerType::GraphWorker,
        wire_tool_call_id: None,
    };
    let journal =
        Arc::new(OperationJournal::open(&workspace.join("journal"), identity, root).unwrap());
    let operations = ToolOperationRuntime::new(
        journal,
        context,
        ExecutionOwner::new(ExecutionOwnerId::new(), 1).unwrap(),
        workspace,
    )
    .unwrap();
    let mut runtime =
        RuntimeHandle::new(run_id, agent_id, RuntimeBus::new()).with_operation_runtime(operations);
    runtime.session_id = Some(format!("graph-test-{agent_id}"));
    runtime
}

fn fixture() -> Fixture {
    let workspace = tempfile::tempdir().unwrap();
    let runtime = runtime(workspace.path(), RunId::new(), AgentId::new());
    let graph_run_id = store::new_run_id();
    let task_id = "worker-1".to_owned();
    store::create_run_dir(workspace.path(), &graph_run_id).unwrap();
    let child = AgentId::new();
    let spec = WorkerSpec {
        task_id: task_id.clone(),
        role: Role::Researcher,
        expect: ArtifactKind::Evidence,
        briefing: "bridge fixture".into(),
        system_prompt: "bridge fixture".into(),
        cwd: workspace.path().to_path_buf(),
        model: Some("fixture/model".into()),
        thinking_level: None,
        tools: vec!["read".into()],
        authorized_tools: vec!["read".into()],
        initially_exposed_tools: vec!["read".into()],
        extra_extensions: Vec::new(),
        timeout_ms: 0,
        run_deadline: None,
        artifact_path: store::artifact_path(workspace.path(), &graph_run_id, &task_id),
        transcript_path: None,
        project_trusted: false,
        runtime_agent_id: Some(child),
        worker_session: None,
        task_contract: None,
        coordinator_client: None,
        node_abort: None,
    };
    let binding =
        WorkerSessionBinding::create(&spec, &graph_run_id, 1, 1, Some(&runtime), None).unwrap();
    Fixture {
        runtime,
        workspace,
        graph_run_id,
        task_id,
        binding,
    }
}

#[test]
fn duplicate_launch_delivery_reuses_one_operation_and_binding() {
    let fixture = fixture();
    let first = launch_worker(
        &fixture.runtime,
        fixture.workspace.path(),
        &fixture.graph_run_id,
        &fixture.task_id,
        1,
        &fixture.binding,
        None,
    )
    .unwrap();
    let second = launch_worker(
        &fixture.runtime,
        fixture.workspace.path(),
        &fixture.graph_run_id,
        &fixture.task_id,
        1,
        &fixture.binding,
        None,
    )
    .unwrap();
    assert_eq!(first.binding, second.binding);
    assert_eq!(
        first.operation.spec.operation_id(),
        second.operation.spec.operation_id()
    );
    assert_eq!(
        first.operation.attempt.state(),
        OperationState::EffectPossible
    );
    assert_eq!(
        fixture
            .runtime
            .operations
            .as_ref()
            .unwrap()
            .dispatcher()
            .journal()
            .snapshot()
            .unwrap()
            .operations
            .len(),
        1
    );
}

#[test]
fn crash_after_worker_creation_replays_the_durable_launch() {
    let fixture = fixture();
    let launch = launch_worker(
        &fixture.runtime,
        fixture.workspace.path(),
        &fixture.graph_run_id,
        &fixture.task_id,
        1,
        &fixture.binding,
        Some("contract-v1"),
    )
    .unwrap();
    assert!(launch.binding.path(fixture.workspace.path()).is_file());
    let replay = launch_worker(
        &fixture.runtime,
        fixture.workspace.path(),
        &fixture.graph_run_id,
        &fixture.task_id,
        1,
        &fixture.binding,
        Some("contract-v1"),
    )
    .unwrap();
    assert_eq!(
        replay.binding.launch_operation_id,
        launch.binding.launch_operation_id
    );
    assert_eq!(
        replay.operation.attempt.state(),
        OperationState::EffectPossible
    );
}

#[test]
fn child_result_is_durable_and_replayable_before_graph_projection() {
    let fixture = fixture();
    let launch = launch_worker(
        &fixture.runtime,
        fixture.workspace.path(),
        &fixture.graph_run_id,
        &fixture.task_id,
        1,
        &fixture.binding,
        None,
    )
    .unwrap();
    let result = WorkerResult {
        ok: false,
        exit_code: 17,
        final_text: "worker failed after launch".into(),
        failure_reason: Some("fixture failure".into()),
        ..WorkerResult::default()
    };
    let receipt = complete_worker(
        &fixture.runtime,
        fixture.workspace.path(),
        &launch.binding,
        &result,
    )
    .unwrap();
    assert_eq!(receipt.state, OperationState::Failed);
    assert!(receipt.payload_digest.is_some());
    assert!(receipt.published_to_outbox);
    assert!(launch
        .binding
        .result_path(fixture.workspace.path())
        .is_file());
    let replay =
        replay_projection(&fixture.runtime, fixture.workspace.path(), &launch.binding).unwrap();
    assert_eq!(replay, receipt);
    assert_eq!(
        fixture
            .runtime
            .operations
            .as_ref()
            .unwrap()
            .dispatcher()
            .journal()
            .snapshot()
            .unwrap()
            .outbox
            .len(),
        1
    );
}

#[test]
fn mismatched_parent_binding_is_rejected_before_admission() {
    let fixture = fixture();
    let mut foreign = fixture.binding.clone();
    foreign.parent = AgentId::new();
    let error = launch_worker(
        &fixture.runtime,
        fixture.workspace.path(),
        &fixture.graph_run_id,
        &fixture.task_id,
        1,
        &foreign,
        None,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        GraphOperationBridgeError::ParentAuthorityChanged
    ));
    assert_eq!(
        fixture
            .runtime
            .operations
            .as_ref()
            .unwrap()
            .dispatcher()
            .journal()
            .snapshot()
            .unwrap()
            .operations
            .len(),
        0
    );
}

#[test]
fn independent_child_coordinator_is_rejected() {
    let fixture = fixture();
    let child_id = AgentId::new();
    let independent_workspace = tempfile::tempdir().unwrap();
    let independent = runtime(
        independent_workspace.path(),
        fixture.runtime.run_id,
        child_id,
    );
    let error = validate_child_runtime(&fixture.runtime, &independent).unwrap_err();
    assert!(matches!(
        error,
        GraphOperationBridgeError::IndependentCoordinator
    ));
    let shared = RuntimeHandle::new(fixture.runtime.run_id, child_id, RuntimeBus::new())
        .with_operation_runtime(
            fixture
                .runtime
                .operations
                .as_ref()
                .unwrap()
                .clone()
                .for_worker(child_id),
        );
    assert!(validate_child_runtime(&fixture.runtime, &shared).is_ok());
}
