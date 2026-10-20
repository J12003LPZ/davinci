//! A steer is "accepted" only when the worker's mailbox holds the message.
//! `WorkerController::execute_command` alone checks freshness and delivers
//! nothing, which is how RPC `agent_control steer` used to answer "accepted"
//! for a message nobody received.

use davinci_agent::runtime::{
    AgentId, AgentKind, AgentMailbox, AgentRecord, AgentState, ControlStatus, RunId,
    RuntimeRegistry, WorkerControlAction, WorkerControlCommand, WorkerController,
};
use std::path::PathBuf;
use uuid::Uuid;

struct Fx {
    registry: RuntimeRegistry,
    mailbox: AgentMailbox,
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
            kind: AgentKind::Background,
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
    registry.transition(agent, AgentState::Running).unwrap();
    if state != AgentState::Running {
        registry.transition(agent, state).unwrap();
    }
    let mailbox = AgentMailbox::new().with_registry(registry.clone());
    Fx {
        registry,
        mailbox,
        run_id,
        agent,
    }
}

fn steer(fx: &Fx, generation: u64) -> WorkerControlCommand {
    WorkerControlCommand {
        id: Uuid::new_v4(),
        root_run_id: fx.run_id,
        agent_id: fx.agent,
        generation,
        task_id: None,
        expected_revision: fx.registry.get_revision(&fx.agent),
        action: WorkerControlAction::Steer {
            message: "look at the parser".into(),
            redirect: false,
        },
    }
}

#[test]
fn a_steer_that_is_accepted_is_in_the_workers_mailbox() {
    let fx = fx(AgentState::Running);
    let controller = WorkerController::new(fx.registry.clone());
    let generation = fx.registry.get_generation(&fx.agent);
    let outcome = controller.execute_steer(&fx.mailbox, steer(&fx, generation), true);
    assert_eq!(outcome.control.status, ControlStatus::Accepted);
    assert!(outcome.is_held(), "{outcome:?}");
    assert_eq!(outcome.state(), "queued");
    assert_eq!(fx.mailbox.pending_count(&fx.agent), 1);
}

#[test]
fn the_controller_alone_still_delivers_nothing() {
    // The reason execute_steer exists: this receipt is a freshness check.
    let fx = fx(AgentState::Running);
    let controller = WorkerController::new(fx.registry.clone());
    let generation = fx.registry.get_generation(&fx.agent);
    let receipt = controller.execute_command(steer(&fx, generation), true);
    assert_eq!(receipt.status, ControlStatus::Accepted);
    assert_eq!(fx.mailbox.pending_count(&fx.agent), 0);
}

#[test]
fn a_stale_generation_is_refused_and_nothing_is_queued() {
    let fx = fx(AgentState::Running);
    let controller = WorkerController::new(fx.registry.clone());
    let outcome = controller.execute_steer(&fx.mailbox, steer(&fx, 0), true);
    assert_eq!(outcome.state(), "stale");
    assert!(!outcome.is_held());
    assert_eq!(fx.mailbox.pending_count(&fx.agent), 0);
}

#[test]
fn a_finished_worker_is_not_resurrected_by_a_late_steer() {
    let fx = fx(AgentState::Completed);
    let controller = WorkerController::new(fx.registry.clone());
    let generation = fx.registry.get_generation(&fx.agent);
    let outcome = controller.execute_steer(&fx.mailbox, steer(&fx, generation), true);
    assert!(!outcome.is_held(), "{outcome:?}");
    assert_eq!(outcome.reason().as_deref(), Some("worker_already_terminal"));
    assert_eq!(fx.mailbox.pending_count(&fx.agent), 0);
    assert_eq!(
        fx.registry.get(&fx.agent).unwrap().state,
        AgentState::Completed
    );
}

#[test]
fn the_same_command_id_is_delivered_once() {
    let fx = fx(AgentState::Running);
    let controller = WorkerController::new(fx.registry.clone());
    let generation = fx.registry.get_generation(&fx.agent);
    let command = steer(&fx, generation);
    controller.execute_steer(&fx.mailbox, command.clone(), true);
    let again = controller.execute_steer(&fx.mailbox, command, true);
    assert_eq!(fx.mailbox.pending_count(&fx.agent), 1, "{again:?}");
}
