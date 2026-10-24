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

#[test]
fn peer_send_requires_a_durable_queue_record_and_can_retry_after_restart() {
    use davinci_agent::runtime::{RuntimeBus, RuntimeDecision, RuntimeSubscriber};
    use std::sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    };
    struct Journal {
        fail: AtomicBool,
        events: Mutex<Vec<RuntimeEventEnvelope>>,
    }
    impl RuntimeSubscriber for Journal {
        fn on_event(&self, event: &RuntimeEventEnvelope) -> RuntimeDecision {
            let _ = self.record_durably(event);
            RuntimeDecision::Continue
        }
        fn record_durably(
            &self,
            event: &RuntimeEventEnvelope,
        ) -> Result<(), davinci_agent::runtime::DurableError> {
            if self.fail.load(Ordering::SeqCst) {
                return Err("injected journal failure".into());
            }
            self.events.lock().unwrap().push(event.clone());
            Ok(())
        }
        fn is_journal(&self) -> bool {
            true
        }
    }
    let (registry, _, run, agent) = setup(AgentState::Running);
    let journal = Arc::new(Journal {
        fail: AtomicBool::new(true),
        events: Mutex::new(vec![]),
    });
    let bus = RuntimeBus::new();
    bus.subscribe(journal.clone());
    let mailbox = AgentMailbox::with_registry_and_bus(registry.clone(), bus);
    let message = AgentMessage::new(run, AgentId::new(), agent, "handoff");
    assert!(matches!(
        mailbox.send(message.clone()),
        Err(MailboxError::NotRecorded(_))
    ));
    assert_eq!(mailbox.pending_count(&agent), 0);
    let recovered = AgentMailbox::new().with_registry(registry);
    recovered.rehydrate_from_events(&journal.events.lock().unwrap());
    assert_eq!(recovered.pending_count(&agent), 0);
    journal.fail.store(false, Ordering::SeqCst);
    mailbox.send(message.clone()).unwrap();
    recovered.rehydrate_from_events(&journal.events.lock().unwrap());
    assert_eq!(recovered.drain(agent, 10), vec![message]);
}

/// A journal that records every event unless told to fail.
struct SwitchJournal {
    fail: std::sync::atomic::AtomicBool,
    events: std::sync::Mutex<Vec<RuntimeEventEnvelope>>,
}

impl SwitchJournal {
    fn new(fail: bool) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            fail: std::sync::atomic::AtomicBool::new(fail),
            events: std::sync::Mutex::new(vec![]),
        })
    }
}

impl davinci_agent::runtime::RuntimeSubscriber for SwitchJournal {
    fn on_event(&self, event: &RuntimeEventEnvelope) -> davinci_agent::runtime::RuntimeDecision {
        let _ = self.record_durably(event);
        davinci_agent::runtime::RuntimeDecision::Continue
    }
    fn record_durably(
        &self,
        event: &RuntimeEventEnvelope,
    ) -> Result<(), davinci_agent::runtime::DurableError> {
        if self.fail.load(std::sync::atomic::Ordering::SeqCst) {
            return Err("injected journal failure".into());
        }
        self.events.lock().unwrap().push(event.clone());
        Ok(())
    }
    fn is_journal(&self) -> bool {
        true
    }
}

#[test]
fn only_the_first_journal_decides_whether_a_queue_record_is_kept() {
    use davinci_agent::runtime::RuntimeBus;
    use std::sync::atomic::Ordering;
    let (registry, _, run, agent) = setup(AgentState::Running);
    let authoritative = SwitchJournal::new(true);
    let second = SwitchJournal::new(false);
    let bus = RuntimeBus::new();
    bus.subscribe(authoritative.clone());
    bus.subscribe(second.clone());
    let mailbox = AgentMailbox::with_registry_and_bus(registry.clone(), bus);
    let message = AgentMessage::new(run, AgentId::new(), agent, "handoff");

    // The authoritative journal refuses: the second one never sees the record.
    assert!(matches!(
        mailbox.send(message.clone()),
        Err(MailboxError::NotRecorded(_))
    ));
    assert_eq!(mailbox.pending_count(&agent), 0);
    assert!(second.events.lock().unwrap().is_empty());
    let recovered = AgentMailbox::new().with_registry(registry.clone());
    recovered.rehydrate_from_events(&second.events.lock().unwrap());
    assert_eq!(recovered.pending_count(&agent), 0);

    // The authoritative journal keeps it: a failing second journal changes nothing.
    authoritative.fail.store(false, Ordering::SeqCst);
    second.fail.store(true, Ordering::SeqCst);
    mailbox.send(message.clone()).unwrap();
    assert_eq!(mailbox.pending_count(&agent), 1);
    let recovered = AgentMailbox::new().with_registry(registry);
    recovered.rehydrate_from_events(&authoritative.events.lock().unwrap());
    assert_eq!(recovered.drain(agent, 10), vec![message]);
    assert_eq!(recovered.pending_count(&agent), 0);
}

#[test]
fn a_refused_queue_record_does_not_wake_an_idle_recipient() {
    use davinci_agent::runtime::RuntimeBus;
    let (registry, _, run, agent) = setup(AgentState::Idle);
    let bus = RuntimeBus::new();
    bus.subscribe(SwitchJournal::new(true));
    let mailbox = AgentMailbox::with_registry_and_bus(registry.clone(), bus);
    let message = AgentMessage::new(run, AgentId::new(), agent, "wake me");
    assert!(matches!(
        mailbox.send(message),
        Err(MailboxError::NotRecorded(_))
    ));
    assert_eq!(registry.get(&agent).unwrap().state, AgentState::Idle);
    assert_eq!(mailbox.pending_count(&agent), 0);
}

#[test]
fn a_refused_steering_record_does_not_wake_an_idle_recipient() {
    use davinci_agent::runtime::RuntimeBus;
    let (registry, _, _, agent) = setup(AgentState::Idle);
    let bus = RuntimeBus::new();
    bus.subscribe(SwitchJournal::new(true));
    let mailbox = AgentMailbox::with_registry_and_bus(registry.clone(), bus);
    let id = Uuid::new_v4();
    let generation = registry.get_generation(&agent);
    assert!(matches!(
        mailbox.send_steer_with_id(id, agent, generation, "wake me".into(), false),
        Err(MailboxError::NotRecorded(_))
    ));
    assert_eq!(registry.get(&agent).unwrap().state, AgentState::Idle);
    assert_eq!(mailbox.pending_count(&agent), 0);
    assert!(mailbox.get_steering_receipt(&id).is_none());
}
