//! Regression tests for mailbox idempotency and replay (WOR-87, 90, 92, 93).

use davinci_agent::runtime::mailbox::MAX_QUEUE_CAPACITY;
use davinci_agent::runtime::{
    AgentId, AgentKind, AgentMailbox, AgentMessage, AgentRecord, AgentState, MailboxError, RunId,
    RuntimeEvent, RuntimeEventEnvelope, RuntimeRegistry,
};
use std::path::PathBuf;
use uuid::Uuid;

fn record(id: AgentId, run_id: RunId, state: AgentState) -> AgentRecord {
    AgentRecord {
        id,
        run_id,
        parent: None,
        kind: AgentKind::Teammate,
        name: "w".into(),
        provider: "mock".into(),
        model_id: "mock".into(),
        cwd: PathBuf::from("/t"),
        state,
        task_id: None,
        worktree: None,
        started_ms: 1,
        updated_ms: 1,
        failure_reason: None,
    }
}

fn queued(seq: u64, msg: &AgentMessage) -> RuntimeEventEnvelope {
    RuntimeEventEnvelope::new(
        seq,
        msg.run_id,
        None,
        Some(msg.from),
        None,
        RuntimeEvent::AgentMessageQueued {
            to: msg.to,
            message_id: msg.id,
            message: Some(msg.clone()),
        },
    )
}

fn setup(state: AgentState) -> (RuntimeRegistry, AgentMailbox, RunId, AgentId) {
    let registry = RuntimeRegistry::new();
    let run_id = RunId::new();
    let agent = AgentId::new();
    registry
        .register_agent(record(agent, run_id, AgentState::Starting))
        .unwrap();
    match state {
        AgentState::Starting => {}
        AgentState::Idle => {
            registry.transition(agent, AgentState::Running).unwrap();
            registry.transition(agent, AgentState::Idle).unwrap();
        }
        other => {
            registry.transition(agent, AgentState::Running).unwrap();
            if other != AgentState::Running {
                registry.transition(agent, other).unwrap();
            }
        }
    }
    let mailbox = AgentMailbox::new().with_registry(registry.clone());
    (registry, mailbox, run_id, agent)
}

#[test]
fn wor87_steer_with_same_id_is_idempotent() {
    let (registry, mailbox, _run, agent) = setup(AgentState::Running);
    let id = Uuid::new_v4();
    let generation = registry.get_generation(&agent);
    let first = mailbox
        .send_steer_with_id(id, agent, generation, "go left".into(), false)
        .unwrap();
    let second = mailbox
        .send_steer_with_id(id, agent, generation, "go left".into(), false)
        .unwrap();
    assert_eq!(first, second);
    assert_eq!(mailbox.pending_count(&agent), 1);
    assert_eq!(mailbox.drain(agent, 10).len(), 1);
    // Still idempotent after delivery: nothing is re-enqueued.
    let third = mailbox
        .send_steer_with_id(id, agent, generation, "go left".into(), false)
        .unwrap();
    assert_eq!(third.message_id, id);
    assert_eq!(mailbox.pending_count(&agent), 0);
}

#[test]
fn wor87_rejected_steer_id_may_be_retried() {
    let (registry, mailbox, _run, agent) = setup(AgentState::Running);
    let id = Uuid::new_v4();
    let generation = registry.get_generation(&agent);
    let stale = mailbox
        .send_steer_with_id(id, agent, generation + 5, "x".into(), false)
        .unwrap();
    assert_eq!(stale.state, "rejected");
    let retry = mailbox
        .send_steer_with_id(id, agent, generation, "x".into(), false)
        .unwrap();
    assert_eq!(retry.state, "queued");
    assert_eq!(mailbox.pending_count(&agent), 1);
}

#[test]
fn wor90_queue_full_does_not_burn_message_id() {
    let (_registry, mailbox, run_id, agent) = setup(AgentState::Running);
    let sender = AgentId::new();
    for _ in 0..MAX_QUEUE_CAPACITY {
        mailbox
            .send(AgentMessage::new(run_id, sender, agent, "fill"))
            .unwrap();
    }
    let stable = AgentMessage::new(run_id, sender, agent, "retry me");
    assert_eq!(
        mailbox.send(stable.clone()),
        Err(MailboxError::QueueFull(agent))
    );
    mailbox.drain(agent, 1);
    mailbox.send(stable.clone()).expect("retry after drain");
    assert_eq!(
        mailbox.send(stable.clone()),
        Err(MailboxError::DuplicateMessage(stable.id))
    );
}

#[test]
fn wor90_unknown_recipient_does_not_burn_message_id() {
    let (registry, mailbox, run_id, _agent) = setup(AgentState::Running);
    let late = AgentId::new();
    let msg = AgentMessage::new(run_id, AgentId::new(), late, "hi");
    assert_eq!(
        mailbox.send(msg.clone()),
        Err(MailboxError::AgentNotFound(late))
    );
    registry
        .register_agent(record(late, run_id, AgentState::Starting))
        .unwrap();
    mailbox.send(msg).expect("retry once recipient exists");
    assert_eq!(mailbox.pending_count(&late), 1);
}

#[test]
fn wor92_rehydrating_twice_does_not_duplicate() {
    let (_registry, mailbox, run_id, agent) = setup(AgentState::Running);
    let msg = AgentMessage::new(run_id, AgentId::new(), agent, "once");
    let events = vec![queued(1, &msg)];
    mailbox.rehydrate_from_events(&events);
    mailbox.rehydrate_from_events(&events);
    assert_eq!(mailbox.pending_count(&agent), 1);
}

#[test]
fn wor93_rehydrate_preserves_queue_order_for_equal_timestamps() {
    let (_registry, mailbox, run_id, agent) = setup(AgentState::Running);
    let sender = AgentId::new();
    let mut a = AgentMessage::new(run_id, sender, agent, "first");
    let mut b = AgentMessage::new(run_id, sender, agent, "second");
    a.sent_at_ms = 5_000;
    b.sent_at_ms = 5_000;
    // Force UUID order opposite to queue order.
    if a.id < b.id {
        std::mem::swap(&mut a.id, &mut b.id);
    }
    assert!(a.id > b.id);
    mailbox.rehydrate_from_events(&[queued(1, &a), queued(2, &b)]);
    let drained: Vec<String> = mailbox
        .drain(agent, 10)
        .into_iter()
        .map(|m| m.content)
        .collect();
    assert_eq!(drained, vec!["first", "second"]);
}
