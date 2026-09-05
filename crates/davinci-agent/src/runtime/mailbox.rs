//! Agent mailbox and inter-agent message passing subsystem.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use thiserror::Error;
use uuid::Uuid;

use super::bus::RuntimeBus;
use super::events::{AgentState, RuntimeEvent, RuntimeEventEnvelope};
use super::ids::{AgentId, RunId};
use super::registry::RuntimeRegistry;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentMessage {
    pub id: Uuid,
    pub run_id: RunId,
    pub from: AgentId,
    pub to: AgentId,
    pub content: String,
    pub sent_at_ms: i64,
    pub reply_to: Option<Uuid>,
}

impl AgentMessage {
    pub fn new(run_id: RunId, from: AgentId, to: AgentId, content: impl Into<String>) -> Self {
        Self {
            id: Uuid::now_v7(),
            run_id,
            from,
            to,
            content: content.into(),
            sent_at_ms: now_ms(),
            reply_to: None,
        }
    }

    pub fn with_reply_to(mut self, reply_to: Uuid) -> Self {
        self.reply_to = Some(reply_to);
        self
    }
}

#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum MailboxError {
    #[error("agent not found: {0}")]
    AgentNotFound(AgentId),
    #[error("cannot send message to terminal agent {agent_id} in state {state:?}")]
    TerminalAgent {
        agent_id: AgentId,
        state: AgentState,
    },
    #[error("duplicate message id: {0}")]
    DuplicateMessage(Uuid),
    #[error("failed to wake agent {0}: {1}")]
    WakeError(AgentId, String),
}

/// In-memory thread-safe mailbox supporting message routing, deduplication, and wakeups.
#[derive(Clone, Default)]
pub struct AgentMailbox {
    queues: Arc<RwLock<HashMap<AgentId, VecDeque<AgentMessage>>>>,
    seen_message_ids: Arc<RwLock<HashSet<Uuid>>>,
    registry: Option<RuntimeRegistry>,
    bus: Option<RuntimeBus>,
    seq: Arc<AtomicU64>,
}

impl AgentMailbox {
    pub fn new() -> Self {
        Self {
            queues: Arc::new(RwLock::new(HashMap::new())),
            seen_message_ids: Arc::new(RwLock::new(HashSet::new())),
            registry: None,
            bus: None,
            seq: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn with_registry_and_bus(registry: RuntimeRegistry, bus: RuntimeBus) -> Self {
        Self {
            queues: Arc::new(RwLock::new(HashMap::new())),
            seen_message_ids: Arc::new(RwLock::new(HashSet::new())),
            registry: Some(registry),
            bus: Some(bus),
            seq: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn with_registry(mut self, registry: RuntimeRegistry) -> Self {
        self.registry = Some(registry);
        self
    }

    pub fn with_bus(mut self, bus: RuntimeBus) -> Self {
        self.bus = Some(bus);
        self
    }

    /// Send a message to an agent. Rejects messages to terminal agents or duplicates.
    /// Wakes up idle agents to `Running`.
    pub fn send(&self, message: AgentMessage) -> Result<(), MailboxError> {
        // 1. At-most-once deduplication check
        {
            let mut seen = self
                .seen_message_ids
                .write()
                .map_err(|_| MailboxError::DuplicateMessage(message.id))?;
            if !seen.insert(message.id) {
                return Err(MailboxError::DuplicateMessage(message.id));
            }
        }

        // 2. Validate recipient state
        let should_wake = if let Some(registry) = &self.registry {
            let record = registry
                .get(&message.to)
                .ok_or(MailboxError::AgentNotFound(message.to))?;

            if matches!(
                record.state,
                AgentState::Completed | AgentState::Failed | AgentState::Cancelled
            ) {
                return Err(MailboxError::TerminalAgent {
                    agent_id: message.to,
                    state: record.state,
                });
            }

            record.state == AgentState::Idle
        } else {
            false
        };

        // 3. Persist undelivered message to mailbox queue before acknowledging
        {
            let mut queues = self
                .queues
                .write()
                .map_err(|_| MailboxError::WakeError(message.to, "mailbox lock poisoned".into()))?;
            queues
                .entry(message.to)
                .or_default()
                .push_back(message.clone());
        }

        // 4. Wake up recipient if idle
        if should_wake {
            if let Some(registry) = &self.registry {
                if let Err(e) = registry.transition(message.to, AgentState::Running) {
                    return Err(MailboxError::WakeError(message.to, e.to_string()));
                }
            }
        }

        // 5. Emit AgentMessageQueued event
        if let Some(bus) = &self.bus {
            let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
            let envelope = RuntimeEventEnvelope::new(
                seq,
                message.run_id,
                None,
                Some(message.from),
                None,
                RuntimeEvent::AgentMessageQueued {
                    to: message.to,
                    message_id: message.id,
                    message: Some(message.clone()),
                },
            );
            bus.emit_observe(envelope);
        }

        Ok(())
    }

    /// Drain up to `limit` messages for the given agent ID.
    pub fn drain(&self, agent_id: AgentId, limit: usize) -> Vec<AgentMessage> {
        if limit == 0 {
            return Vec::new();
        }

        let drained: Vec<AgentMessage> = {
            let Ok(mut queues) = self.queues.write() else {
                return Vec::new();
            };
            if let Some(queue) = queues.get_mut(&agent_id) {
                let count = limit.min(queue.len());
                let mut items = Vec::with_capacity(count);
                for _ in 0..count {
                    if let Some(msg) = queue.pop_front() {
                        items.push(msg);
                    }
                }
                items
            } else {
                Vec::new()
            }
        };

        // Emit delivery events for all drained messages
        if let Some(bus) = &self.bus {
            for msg in &drained {
                let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
                let envelope = RuntimeEventEnvelope::new(
                    seq,
                    msg.run_id,
                    None,
                    Some(agent_id),
                    None,
                    RuntimeEvent::AgentMessageDelivered {
                        to: agent_id,
                        message_id: msg.id,
                    },
                );
                bus.emit_observe(envelope);
            }
        }

        drained
    }

    /// Count pending messages for an agent.
    pub fn pending_count(&self, agent_id: &AgentId) -> usize {
        let Ok(queues) = self.queues.read() else {
            return 0;
        };
        queues.get(agent_id).map(|q| q.len()).unwrap_or(0)
    }

    /// Peek at pending messages without removing them.
    pub fn peek(&self, agent_id: &AgentId) -> Vec<AgentMessage> {
        let Ok(queues) = self.queues.read() else {
            return Vec::new();
        };
        queues
            .get(agent_id)
            .map(|q| q.iter().cloned().collect())
            .unwrap_or_default()
    }

    /// Rehydrate mailbox state from historical runtime event envelopes.
    ///
    /// Undelivered queued messages remain in the recipient's mailbox.
    pub fn rehydrate_from_events(&self, events: &[RuntimeEventEnvelope]) {
        let mut unread_messages: HashMap<Uuid, AgentMessage> = HashMap::new();
        let mut seen = match self.seen_message_ids.write() {
            Ok(s) => s,
            Err(e) => e.into_inner(),
        };

        for envelope in events {
            match &envelope.payload {
                RuntimeEvent::AgentMessageQueued {
                    message_id,
                    message,
                    ..
                } => {
                    seen.insert(*message_id);
                    if let Some(msg) = message {
                        unread_messages.insert(*message_id, msg.clone());
                    }
                }
                RuntimeEvent::AgentMessageDelivered { message_id, .. } => {
                    unread_messages.remove(message_id);
                }
                _ => {}
            }
        }

        let mut queues = match self.queues.write() {
            Ok(q) => q,
            Err(e) => e.into_inner(),
        };

        let mut remaining: Vec<AgentMessage> = unread_messages.into_values().collect();
        remaining.sort_by(|a, b| {
            a.sent_at_ms
                .cmp(&b.sent_at_ms)
                .then_with(|| a.id.cmp(&b.id))
        });

        for msg in remaining {
            queues.entry(msg.to).or_default().push_back(msg);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::bus::{RuntimeDecision, RuntimeSubscriber};
    use crate::runtime::events::AgentKind;
    use crate::runtime::events::AgentRecord;
    use std::path::PathBuf;
    use std::sync::Mutex;

    fn make_test_record(agent_id: AgentId, run_id: RunId, state: AgentState) -> AgentRecord {
        let now = now_ms();
        AgentRecord {
            id: agent_id,
            run_id,
            parent: None,
            kind: AgentKind::Teammate,
            name: "test-agent".into(),
            provider: "mock".into(),
            model_id: "mock".into(),
            cwd: PathBuf::from("/test"),
            state,
            task_id: None,
            worktree: None,
            started_ms: now,
            updated_ms: now,
            failure_reason: None,
        }
    }

    struct EventCapture {
        events: Arc<Mutex<Vec<RuntimeEvent>>>,
    }

    impl RuntimeSubscriber for EventCapture {
        fn on_event(&self, event: &RuntimeEventEnvelope) -> RuntimeDecision {
            self.events.lock().unwrap().push(event.payload.clone());
            RuntimeDecision::Continue
        }
    }

    #[test]
    fn test_send_and_drain_messages() {
        let bus = RuntimeBus::new();
        let events = Arc::new(Mutex::new(Vec::new()));
        bus.subscribe(Arc::new(EventCapture {
            events: Arc::clone(&events),
        }));

        let registry = RuntimeRegistry::with_bus(bus.clone());
        let mailbox = AgentMailbox::with_registry_and_bus(registry.clone(), bus);

        let run_id = RunId::new();
        let sender_id = AgentId::new();
        let recipient_id = AgentId::new();

        registry
            .register_agent(make_test_record(sender_id, run_id, AgentState::Running))
            .unwrap();
        registry
            .register_agent(make_test_record(recipient_id, run_id, AgentState::Running))
            .unwrap();

        let msg1 = AgentMessage::new(run_id, sender_id, recipient_id, "Hello 1");
        let id1 = msg1.id;
        mailbox.send(msg1).unwrap();

        let msg2 = AgentMessage::new(run_id, sender_id, recipient_id, "Hello 2");
        let id2 = msg2.id;
        mailbox.send(msg2).unwrap();

        assert_eq!(mailbox.pending_count(&recipient_id), 2);

        // Drain 1 message
        let drained1 = mailbox.drain(recipient_id, 1);
        assert_eq!(drained1.len(), 1);
        assert_eq!(drained1[0].id, id1);
        assert_eq!(mailbox.pending_count(&recipient_id), 1);

        // Drain remaining
        let drained2 = mailbox.drain(recipient_id, 10);
        assert_eq!(drained2.len(), 1);
        assert_eq!(drained2[0].id, id2);
        assert_eq!(mailbox.pending_count(&recipient_id), 0);

        let captured = events.lock().unwrap();
        assert!(captured.iter().any(|e| matches!(e, RuntimeEvent::AgentMessageQueued { to, message_id, .. } if *to == recipient_id && *message_id == id1)));
        assert!(captured.iter().any(|e| matches!(e, RuntimeEvent::AgentMessageDelivered { to, message_id } if *to == recipient_id && *message_id == id1)));
    }

    #[test]
    fn test_reject_message_to_terminal_agent() {
        let registry = RuntimeRegistry::new();
        let mailbox = AgentMailbox::new().with_registry(registry.clone());

        let run_id = RunId::new();
        let sender_id = AgentId::new();
        let recipient_id = AgentId::new();

        registry
            .register_agent(make_test_record(sender_id, run_id, AgentState::Running))
            .unwrap();
        registry
            .register_agent(make_test_record(
                recipient_id,
                run_id,
                AgentState::Completed,
            ))
            .unwrap();

        let msg = AgentMessage::new(run_id, sender_id, recipient_id, "Should fail");
        let err = mailbox.send(msg).unwrap_err();
        assert_eq!(
            err,
            MailboxError::TerminalAgent {
                agent_id: recipient_id,
                state: AgentState::Completed,
            }
        );
    }

    #[test]
    fn test_wake_idle_agent_on_send() {
        let registry = RuntimeRegistry::new();
        let mailbox = AgentMailbox::new().with_registry(registry.clone());

        let run_id = RunId::new();
        let sender_id = AgentId::new();
        let recipient_id = AgentId::new();

        registry
            .register_agent(make_test_record(sender_id, run_id, AgentState::Running))
            .unwrap();
        registry
            .register_agent(make_test_record(recipient_id, run_id, AgentState::Idle))
            .unwrap();

        let msg = AgentMessage::new(run_id, sender_id, recipient_id, "Wake up!");
        mailbox.send(msg).unwrap();

        // Recipient must now be Running
        let updated = registry.get(&recipient_id).unwrap();
        assert_eq!(updated.state, AgentState::Running);
    }

    #[test]
    fn test_at_most_once_deduplication() {
        let registry = RuntimeRegistry::new();
        let mailbox = AgentMailbox::new().with_registry(registry.clone());

        let run_id = RunId::new();
        let sender_id = AgentId::new();
        let recipient_id = AgentId::new();

        registry
            .register_agent(make_test_record(sender_id, run_id, AgentState::Running))
            .unwrap();
        registry
            .register_agent(make_test_record(recipient_id, run_id, AgentState::Running))
            .unwrap();

        let msg = AgentMessage::new(run_id, sender_id, recipient_id, "Unique message");
        let dup_id = msg.id;

        mailbox.send(msg.clone()).unwrap();
        let err = mailbox.send(msg).unwrap_err();
        assert_eq!(err, MailboxError::DuplicateMessage(dup_id));
    }

    #[test]
    fn test_send_to_unknown_agent_rejected() {
        let registry = RuntimeRegistry::new();
        let mailbox = AgentMailbox::new().with_registry(registry);

        let run_id = RunId::new();
        let sender_id = AgentId::new();
        let missing_id = AgentId::new();

        let msg = AgentMessage::new(run_id, sender_id, missing_id, "Unknown agent");
        let err = mailbox.send(msg).unwrap_err();
        assert_eq!(err, MailboxError::AgentNotFound(missing_id));
    }

    #[test]
    fn test_rehydrate_mailbox_undelivered_remain_queued_and_delivered_removed() {
        let mailbox = AgentMailbox::new();
        let run_id = RunId::new();
        let sender_id = AgentId::new();
        let recipient_id = AgentId::new();

        let msg1 = AgentMessage::new(run_id, sender_id, recipient_id, "Delivered message");
        let msg2 = AgentMessage::new(
            run_id,
            sender_id,
            recipient_id,
            "Pending undelivered message",
        );

        let events = vec![
            RuntimeEventEnvelope::new(
                1,
                run_id,
                None,
                Some(sender_id),
                None,
                RuntimeEvent::AgentMessageQueued {
                    to: recipient_id,
                    message_id: msg1.id,
                    message: Some(msg1.clone()),
                },
            ),
            RuntimeEventEnvelope::new(
                2,
                run_id,
                None,
                Some(recipient_id),
                None,
                RuntimeEvent::AgentMessageDelivered {
                    to: recipient_id,
                    message_id: msg1.id,
                },
            ),
            RuntimeEventEnvelope::new(
                3,
                run_id,
                None,
                Some(sender_id),
                None,
                RuntimeEvent::AgentMessageQueued {
                    to: recipient_id,
                    message_id: msg2.id,
                    message: Some(msg2.clone()),
                },
            ),
        ];

        mailbox.rehydrate_from_events(&events);

        // msg1 was delivered, so it must not be queued
        // msg2 was undelivered, so it remains in recipient's queue
        assert_eq!(mailbox.pending_count(&recipient_id), 1);
        let drained = mailbox.drain(recipient_id, 10);
        assert_eq!(drained.len(), 1);
        assert_eq!(drained[0].id, msg2.id);
        assert_eq!(drained[0].content, "Pending undelivered message");
    }
}
