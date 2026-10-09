//! Regression tests for worker control semantics (WOR-95, 96, 97, 98, 100).

use davinci_agent::runtime::{
    AgentId, AgentKind, AgentRecord, AgentState, ControlStatus, RunId, RuntimeRegistry,
    WorkerControlAction, WorkerControlCommand, WorkerController,
};
use std::io::Write;
use std::path::PathBuf;
use uuid::Uuid;

struct Fx {
    registry: RuntimeRegistry,
    run_id: RunId,
    agent: AgentId,
}

fn fx(state: AgentState) -> Fx {
    let registry = RuntimeRegistry::new();
    let run_id = RunId::new();
    let agent = AgentId::new();
    registry
        .register_agent(AgentRecord {
            id: agent,
            run_id,
            parent: None,
            kind: AgentKind::Teammate,
            name: "w".into(),
            provider: "mock".into(),
            model_id: "mock".into(),
            cwd: PathBuf::from("/t"),
            state: AgentState::Starting,
            task_id: None,
            worktree: None,
            started_ms: 1,
            updated_ms: 1,
            failure_reason: None,
        })
        .unwrap();
    if state != AgentState::Starting {
        registry.transition(agent, AgentState::Running).unwrap();
        if state != AgentState::Running {
            registry.transition(agent, state).unwrap();
        }
    }
    Fx {
        registry,
        run_id,
        agent,
    }
}

fn command(fx: &Fx, action: WorkerControlAction) -> WorkerControlCommand {
    WorkerControlCommand {
        id: Uuid::new_v4(),
        root_run_id: fx.run_id,
        agent_id: fx.agent,
        generation: fx.registry.get_generation(&fx.agent),
        task_id: None,
        expected_revision: fx.registry.get_revision(&fx.agent),
        action,
    }
}

#[test]
fn wor95_unauthorized_command_id_is_not_consumed() {
    let fx = fx(AgentState::Running);
    let controller = WorkerController::new(fx.registry.clone());
    let cmd = command(&fx, WorkerControlAction::Inspect);
    let denied = controller.execute_command(cmd.clone(), false);
    assert_eq!(denied.status, ControlStatus::Rejected);
    assert_eq!(denied.reason.as_deref(), Some("unauthorized"));
    let retry = controller.execute_command(cmd, true);
    assert_eq!(retry.status, ControlStatus::Accepted, "{retry:?}");
}

#[test]
fn wor95_stale_command_id_is_not_consumed() {
    let fx = fx(AgentState::Running);
    let controller = WorkerController::new(fx.registry.clone());
    let mut cmd = command(&fx, WorkerControlAction::Inspect);
    cmd.expected_revision += 7;
    assert_eq!(
        controller.execute_command(cmd.clone(), true).status,
        ControlStatus::Stale
    );
    cmd.expected_revision = fx.registry.get_revision(&fx.agent);
    assert_eq!(
        controller.execute_command(cmd, true).status,
        ControlStatus::Accepted
    );
}

#[test]
fn wor96_duplicate_returns_original_receipt() {
    let fx = fx(AgentState::Running);
    let controller = WorkerController::new(fx.registry.clone());
    let cmd = command(
        &fx,
        WorkerControlAction::Stop {
            reason: Some("because".into()),
        },
    );
    let first = controller.execute_command(cmd.clone(), true);
    assert_eq!(first.status, ControlStatus::Stopping);
    let second = controller.execute_command(cmd, true);
    assert_eq!(second, first);
}

#[test]
fn wor96_different_digest_is_still_a_collision() {
    let fx = fx(AgentState::Running);
    let controller = WorkerController::new(fx.registry.clone());
    let cmd = command(&fx, WorkerControlAction::Inspect);
    controller.execute_command(cmd.clone(), true);
    let mut other = cmd;
    other.action = WorkerControlAction::Diff;
    let receipt = controller.execute_command(other, true);
    assert_eq!(receipt.status, ControlStatus::Rejected);
    assert_eq!(receipt.reason.as_deref(), Some("command_id_collision"));
}

#[test]
fn wor97_retry_that_cannot_transition_keeps_generation_and_revision() {
    let fx = fx(AgentState::Running);
    let controller = WorkerController::new(fx.registry.clone());
    let generation = fx.registry.get_generation(&fx.agent);
    let revision = fx.registry.get_revision(&fx.agent);
    let cmd = command(&fx, WorkerControlAction::Retry { reason: None });
    let receipt = controller.execute_command(cmd, true);
    assert_eq!(receipt.status, ControlStatus::Rejected, "{receipt:?}");
    assert_eq!(fx.registry.get_generation(&fx.agent), generation);
    assert_eq!(fx.registry.get_revision(&fx.agent), revision);
    assert_eq!(
        fx.registry.get(&fx.agent).unwrap().state,
        AgentState::Running
    );
}

#[test]
fn wor98_stop_that_cannot_transition_reports_rejection() {
    // Starting cannot move to Stopping.
    let fx = fx(AgentState::Starting);
    let controller = WorkerController::new(fx.registry.clone());
    let cmd = command(&fx, WorkerControlAction::Stop { reason: None });
    let receipt = controller.execute_command(cmd, true);
    assert_eq!(receipt.status, ControlStatus::Rejected, "{receipt:?}");
    assert!(receipt
        .reason
        .as_deref()
        .is_some_and(|r| r.starts_with("transition_rejected")));
    assert_eq!(
        fx.registry.get(&fx.agent).unwrap().state,
        AgentState::Starting
    );
}

#[test]
fn wor98_stop_on_running_worker_still_reports_stopping() {
    let fx = fx(AgentState::Running);
    let controller = WorkerController::new(fx.registry.clone());
    let cmd = command(&fx, WorkerControlAction::Stop { reason: None });
    let receipt = controller.execute_command(cmd, true);
    assert_eq!(receipt.status, ControlStatus::Stopping);
    assert_eq!(
        fx.registry.get(&fx.agent).unwrap().state,
        AgentState::Stopping
    );
}

#[test]
fn wor100_torn_final_ledger_line_recovers_prior_receipts() {
    let fx = fx(AgentState::Running);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("receipts.jsonl");
    let controller = WorkerController::new(fx.registry.clone())
        .with_receipt_store(&path)
        .unwrap();
    let first = controller.execute_command(command(&fx, WorkerControlAction::Inspect), true);
    assert_eq!(first.status, ControlStatus::Accepted);
    // Crash mid-append: a partial JSON tail with no newline.
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .unwrap();
    file.write_all(b"{\"command_id\":\"0190").unwrap();
    drop(file);

    let reopened = WorkerController::new(fx.registry.clone())
        .with_receipt_store(&path)
        .expect("torn tail must not block startup");
    assert_eq!(reopened.receipt_for(&first.command_id), Some(first.clone()));

    // New appends after recovery must stay parseable.
    let second = reopened.execute_command(command(&fx, WorkerControlAction::Diff), true);
    let again = WorkerController::new(fx.registry.clone())
        .with_receipt_store(&path)
        .expect("ledger stays readable after recovery");
    assert_eq!(again.receipt_for(&second.command_id), Some(second));
    assert_eq!(again.receipt_for(&first.command_id), Some(first));
}

#[test]
fn wor100_corrupt_middle_line_is_still_an_error() {
    let fx = fx(AgentState::Running);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("receipts.jsonl");
    let controller = WorkerController::new(fx.registry.clone())
        .with_receipt_store(&path)
        .unwrap();
    controller.execute_command(command(&fx, WorkerControlAction::Inspect), true);
    let good = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, format!("not json\n{good}")).unwrap();
    assert!(WorkerController::new(fx.registry.clone())
        .with_receipt_store(&path)
        .is_err());
}
