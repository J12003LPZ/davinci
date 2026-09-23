use davinci_agent::runtime::operations::{
    ControlOperationAdapter, ControlOperationError, ControlReceiptValue, ExecutionOwner,
    ExecutionOwnerId, JournalId, JournalIdentity, OperationJournal, OperationState,
    RootNamespaceId, ToolOperationRuntime, WorkspaceId, WorkspaceIdentity,
};
use davinci_agent::runtime::{
    AgentId, AgentKind, AgentMailbox, AgentRecord, AgentState, RunId, RuntimeRegistry,
    TaskCreateRequest, TaskRegistry, WorkerControlAction, WorkerControlCommand, WorkerController,
};
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;
use uuid::Uuid;

struct Fixture {
    _temp: TempDir,
    run_id: RunId,
    actor: AgentId,
    tasks: TaskRegistry,
    registry: RuntimeRegistry,
    mailbox: AgentMailbox,
    adapter: ControlOperationAdapter,
}

impl Fixture {
    fn new() -> Self {
        let temp = tempfile::tempdir().unwrap();
        let workspace = std::fs::canonicalize(temp.path()).unwrap();
        let run_id = RunId::new();
        let actor = AgentId::new();
        let root = RootNamespaceId::new();
        let identity = JournalIdentity::new(
            JournalId::new(),
            WorkspaceIdentity {
                id: WorkspaceId::new(),
                binding_version: 1,
            },
        )
        .unwrap();
        let context = davinci_agent::runtime::operations::OperationContext {
            journal_id: identity.journal_id,
            root_namespace_id: root,
            session_id: "operation-control-test".into(),
            runtime_run_id: run_id,
            parent_operation_id: None,
            agent_id: actor,
            worker_id: None,
            task_id: None,
            graph: None,
            workspace: identity.workspace.clone(),
            caller: davinci_agent::runtime::operations::CallerType::TaskTransport,
            wire_tool_call_id: None,
        };
        let journal =
            Arc::new(OperationJournal::open(&workspace.join("journal"), identity, root).unwrap());
        let operations = ToolOperationRuntime::new(
            journal,
            context,
            ExecutionOwner::new(ExecutionOwnerId::new(), 1).unwrap(),
            &workspace,
        )
        .unwrap();
        let registry = RuntimeRegistry::new();
        let mailbox = AgentMailbox::new().with_registry(registry.clone());
        Self {
            _temp: temp,
            run_id,
            actor,
            tasks: TaskRegistry::new(),
            registry,
            mailbox,
            adapter: ControlOperationAdapter::new(operations),
        }
    }

    fn create_input(&self, title: &str) -> TaskCreateRequest {
        TaskCreateRequest {
            title: title.into(),
            ..TaskCreateRequest::default()
        }
    }

    fn running_worker(&self) -> AgentId {
        let agent = AgentId::new();
        self.registry
            .register_agent(AgentRecord {
                id: agent,
                run_id: self.run_id,
                parent: None,
                kind: AgentKind::GraphWorker,
                name: "control-worker".into(),
                provider: "test".into(),
                model_id: "test".into(),
                cwd: PathBuf::from("."),
                state: AgentState::Starting,
                task_id: None,
                worktree: None,
                started_ms: 0,
                updated_ms: 0,
                failure_reason: None,
            })
            .unwrap();
        self.registry
            .transition(agent, AgentState::Running)
            .unwrap();
        agent
    }
}

#[test]
fn duplicate_task_create_replays_one_domain_commit() {
    let fixture = Fixture::new();
    let command_id = Uuid::new_v4();
    let first = fixture
        .adapter
        .execute_task_create(
            &fixture.tasks,
            fixture.create_input("one"),
            fixture.run_id,
            fixture.actor,
            command_id,
        )
        .unwrap();
    let second = fixture
        .adapter
        .execute_task_create(
            &fixture.tasks,
            fixture.create_input("one"),
            fixture.run_id,
            fixture.actor,
            command_id,
        )
        .unwrap();
    assert!(!first.replayed);
    assert!(second.replayed);
    assert_eq!(first.value, second.value);
    assert_eq!(fixture.tasks.list_tasks(Some(fixture.run_id)).len(), 1);
}

#[test]
fn changed_command_with_same_id_is_rejected_before_domain_dispatch() {
    let fixture = Fixture::new();
    let command_id = Uuid::new_v4();
    fixture
        .adapter
        .execute_task_create(
            &fixture.tasks,
            fixture.create_input("original"),
            fixture.run_id,
            fixture.actor,
            command_id,
        )
        .unwrap();
    let error = fixture
        .adapter
        .execute_task_create(
            &fixture.tasks,
            fixture.create_input("changed"),
            fixture.run_id,
            fixture.actor,
            command_id,
        )
        .unwrap_err();
    assert!(matches!(error, ControlOperationError::Collision));
    assert_eq!(fixture.tasks.list_tasks(Some(fixture.run_id)).len(), 1);
}

#[test]
fn task_receipt_reconciles_when_outer_response_was_missing() {
    let fixture = Fixture::new();
    let command_id = Uuid::new_v4();
    let original = fixture
        .tasks
        .create_command(
            fixture.create_input("committed-first"),
            fixture.run_id,
            fixture.actor,
            Some(command_id),
        )
        .unwrap();
    let recovered = fixture
        .adapter
        .execute_task_create(
            &fixture.tasks,
            fixture.create_input("committed-first"),
            fixture.run_id,
            fixture.actor,
            command_id,
        )
        .unwrap();
    assert!(recovered.replayed);
    assert_eq!(recovered.value, ControlReceiptValue::Task(original));
    assert_eq!(fixture.tasks.list_tasks(Some(fixture.run_id)).len(), 1);
}

#[test]
fn worker_control_and_steering_share_command_identity() {
    let fixture = Fixture::new();
    let agent = fixture.running_worker();
    let controller = WorkerController::new(fixture.registry.clone());
    let command = WorkerControlCommand {
        id: Uuid::new_v4(),
        root_run_id: fixture.run_id,
        agent_id: agent,
        generation: fixture.registry.get_generation(&agent),
        task_id: None,
        expected_revision: fixture.registry.get_revision(&agent),
        action: WorkerControlAction::Steer {
            message: "continue".into(),
            redirect: false,
        },
    };
    let first = fixture
        .adapter
        .execute_worker_control(&controller, Some(&fixture.mailbox), command.clone(), true)
        .unwrap();
    let second = fixture
        .adapter
        .execute_worker_control(&controller, Some(&fixture.mailbox), command.clone(), true)
        .unwrap();
    assert!(!first.replayed);
    assert!(second.replayed);
    match first.value {
        ControlReceiptValue::Steering(receipt) => {
            assert_eq!(receipt.message_id, command.id);
            assert_eq!(
                fixture.mailbox.get_steering_receipt(&command.id),
                Some(receipt)
            );
        }
        other => panic!("expected steering receipt, got {other:?}"),
    }
    assert_eq!(fixture.mailbox.pending_count(&agent), 1);
}

#[test]
fn stale_worker_control_is_a_durable_rejection_receipt() {
    let fixture = Fixture::new();
    let agent = fixture.running_worker();
    let controller = WorkerController::new(fixture.registry.clone());
    fixture
        .registry
        .transition(agent, AgentState::Waiting)
        .unwrap();
    let command = WorkerControlCommand {
        id: Uuid::new_v4(),
        root_run_id: fixture.run_id,
        agent_id: agent,
        generation: 1,
        task_id: None,
        expected_revision: 1,
        action: WorkerControlAction::Inspect,
    };
    let receipt = fixture
        .adapter
        .execute_worker_control(&controller, None, command, true)
        .unwrap();
    match receipt.value {
        ControlReceiptValue::Worker(value) => {
            assert_eq!(value.status, davinci_agent::runtime::ControlStatus::Stale);
        }
        other => panic!("expected worker receipt, got {other:?}"),
    }
    assert_eq!(receipt.state, OperationState::Succeeded);
}

#[test]
fn worker_receipt_store_replays_after_controller_restart() {
    let fixture = Fixture::new();
    let agent = fixture.running_worker();
    let path = fixture._temp.path().join("control-receipts.jsonl");
    let controller = WorkerController::new(fixture.registry.clone())
        .with_receipt_store(&path)
        .unwrap();
    let command = WorkerControlCommand {
        id: Uuid::new_v4(),
        root_run_id: fixture.run_id,
        agent_id: agent,
        generation: fixture.registry.get_generation(&agent),
        task_id: None,
        expected_revision: fixture.registry.get_revision(&agent),
        action: WorkerControlAction::Inspect,
    };
    let first = controller.execute_command(command.clone(), true);
    let restarted = WorkerController::new(fixture.registry.clone())
        .with_receipt_store(&path)
        .unwrap();
    let second = restarted.execute_command(command, true);
    assert_eq!(first.status, davinci_agent::runtime::ControlStatus::Accepted);
    assert_eq!(second.status, davinci_agent::runtime::ControlStatus::Rejected);
    assert_eq!(second.reason.as_deref(), Some("duplicate_command"));
    assert_eq!(restarted.receipt_for(&first.command_id), Some(first));
}
