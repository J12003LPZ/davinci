//! In-memory runtime agent registry tracking lifecycle states.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use thiserror::Error;

use super::bus::RuntimeBus;
use super::events::{AgentRecord, AgentState, RuntimeEvent, RuntimeEventEnvelope};
use super::ids::{AgentId, RunId};

#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum RegistryError {
    #[error("agent already registered: {0}")]
    DuplicateAgent(AgentId),
    #[error("agent not found: {0}")]
    AgentNotFound(AgentId),
    #[error("invalid state transition for agent {agent_id} from {from:?} to {to:?}")]
    InvalidTransition {
        agent_id: AgentId,
        from: AgentState,
        to: AgentState,
    },
    #[error("cannot transition terminal agent {agent_id} from state {state:?}")]
    TerminalResurrection {
        agent_id: AgentId,
        state: AgentState,
    },
}

/// Returns true if transitioning from `from` to `to` is permitted.
pub fn is_valid_transition(from: AgentState, to: AgentState) -> bool {
    use AgentState::*;
    match from {
        Starting => matches!(to, Running | Failed | Cancelled),
        Running => matches!(to, Waiting | Idle | Stopping | Completed | Failed | Cancelled),
        Waiting => matches!(to, Running | Stopping | Failed | Cancelled),
        Idle => matches!(to, Running | Stopping | Completed | Cancelled),
        Stopping => matches!(to, Completed | Failed | Cancelled),
        Completed | Failed | Cancelled => false,
    }
}

/// In-memory thread-safe registry of active and historical agent records.
#[derive(Clone, Default)]
pub struct RuntimeRegistry {
    records: Arc<RwLock<HashMap<AgentId, AgentRecord>>>,
    bus: Option<RuntimeBus>,
    seq: Arc<AtomicU64>,
}

impl RuntimeRegistry {
    pub fn new() -> Self {
        Self {
            records: Arc::new(RwLock::new(HashMap::new())),
            bus: None,
            seq: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn with_bus(bus: RuntimeBus) -> Self {
        Self {
            records: Arc::new(RwLock::new(HashMap::new())),
            bus: Some(bus),
            seq: Arc::new(AtomicU64::new(0)),
        }
    }

    fn now_ms() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0)
    }

    /// Register a new agent. Returns an error if an agent with this ID is already registered.
    pub fn register_agent(&self, record: AgentRecord) -> Result<(), RegistryError> {
        {
            let mut map = self.records.write().map_err(|_| {
                RegistryError::InvalidTransition {
                    agent_id: record.id,
                    from: record.state,
                    to: record.state,
                }
            })?;
            if map.contains_key(&record.id) {
                return Err(RegistryError::DuplicateAgent(record.id));
            }
            map.insert(record.id, record.clone());
        }

        if let Some(bus) = &self.bus {
            let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
            let envelope = RuntimeEventEnvelope::new(
                seq,
                record.run_id,
                None,
                Some(record.id),
                record.parent,
                RuntimeEvent::AgentStarted {
                    record: record.clone(),
                },
            );
            bus.emit_observe(envelope);
        }

        Ok(())
    }

    /// Transition an agent's lifecycle state.
    pub fn transition(&self, id: AgentId, to: AgentState) -> Result<(), RegistryError> {
        let (from, run_id, parent) = {
            let mut map = self.records.write().map_err(|_| {
                RegistryError::InvalidTransition {
                    agent_id: id,
                    from: to,
                    to,
                }
            })?;
            let record = map.get_mut(&id).ok_or(RegistryError::AgentNotFound(id))?;

            let from = record.state;
            if matches!(from, AgentState::Completed | AgentState::Failed | AgentState::Cancelled) {
                return Err(RegistryError::TerminalResurrection {
                    agent_id: id,
                    state: from,
                });
            }

            if !is_valid_transition(from, to) {
                return Err(RegistryError::InvalidTransition {
                    agent_id: id,
                    from,
                    to,
                });
            }

            record.state = to;
            record.updated_ms = Self::now_ms();
            (from, record.run_id, record.parent)
        };

        if let Some(bus) = &self.bus {
            let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
            let envelope = RuntimeEventEnvelope::new(
                seq,
                run_id,
                None,
                Some(id),
                parent,
                RuntimeEvent::AgentStateChanged { from, to },
            );
            bus.emit_observe(envelope);
        }

        Ok(())
    }

    /// Get an agent record by ID.
    pub fn get(&self, id: &AgentId) -> Option<AgentRecord> {
        let map = self.records.read().ok()?;
        map.get(id).cloned()
    }

    /// List all agents belonging to a run.
    pub fn get_by_run(&self, run_id: &RunId) -> Vec<AgentRecord> {
        let Ok(map) = self.records.read() else {
            return Vec::new();
        };
        map.values()
            .filter(|r| &r.run_id == run_id)
            .cloned()
            .collect()
    }

    /// Snapshot all agent records, ordered deterministically by started_ms, then agent id.
    pub fn snapshot(&self) -> Vec<AgentRecord> {
        let Ok(map) = self.records.read() else {
            return Vec::new();
        };
        let mut list: Vec<AgentRecord> = map.values().cloned().collect();
        list.sort_by(|a, b| {
            a.started_ms
                .cmp(&b.started_ms)
                .then_with(|| a.id.cmp(&b.id))
        });
        list
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::bus::{RuntimeDecision, RuntimeSubscriber};
    use crate::runtime::events::AgentKind;
    use std::path::PathBuf;
    use std::sync::Mutex;

    fn sample_record(id: AgentId, state: AgentState) -> AgentRecord {
        AgentRecord {
            id,
            run_id: RunId::new(),
            parent: None,
            kind: AgentKind::Main,
            name: "test-agent".to_string(),
            provider: "mock".to_string(),
            model_id: "mock-model".to_string(),
            cwd: PathBuf::from("/test"),
            state,
            task_id: None,
            worktree: None,
            started_ms: 1000,
            updated_ms: 1000,
        }
    }

    #[test]
    fn test_valid_transitions_starting() {
        assert!(is_valid_transition(AgentState::Starting, AgentState::Running));
        assert!(is_valid_transition(AgentState::Starting, AgentState::Failed));
        assert!(is_valid_transition(AgentState::Starting, AgentState::Cancelled));
        assert!(!is_valid_transition(AgentState::Starting, AgentState::Completed));
        assert!(!is_valid_transition(AgentState::Starting, AgentState::Idle));
    }

    #[test]
    fn test_valid_transitions_running() {
        assert!(is_valid_transition(AgentState::Running, AgentState::Waiting));
        assert!(is_valid_transition(AgentState::Running, AgentState::Idle));
        assert!(is_valid_transition(AgentState::Running, AgentState::Stopping));
        assert!(is_valid_transition(AgentState::Running, AgentState::Completed));
        assert!(is_valid_transition(AgentState::Running, AgentState::Failed));
        assert!(is_valid_transition(AgentState::Running, AgentState::Cancelled));
        assert!(!is_valid_transition(AgentState::Running, AgentState::Starting));
    }

    #[test]
    fn test_reject_duplicate_agent() {
        let registry = RuntimeRegistry::new();
        let id = AgentId::new();
        let r1 = sample_record(id, AgentState::Starting);
        let r2 = sample_record(id, AgentState::Starting);

        assert!(registry.register_agent(r1).is_ok());
        let err = registry.register_agent(r2).unwrap_err();
        assert_eq!(err, RegistryError::DuplicateAgent(id));
    }

    #[test]
    fn test_reject_terminal_resurrection() {
        let registry = RuntimeRegistry::new();
        let id = AgentId::new();
        let record = sample_record(id, AgentState::Starting);
        registry.register_agent(record).unwrap();

        registry.transition(id, AgentState::Running).unwrap();
        registry.transition(id, AgentState::Completed).unwrap();

        let err = registry.transition(id, AgentState::Running).unwrap_err();
        assert_eq!(
            err,
            RegistryError::TerminalResurrection {
                agent_id: id,
                state: AgentState::Completed
            }
        );
    }

    #[test]
    fn test_reject_invalid_transition() {
        let registry = RuntimeRegistry::new();
        let id = AgentId::new();
        let record = sample_record(id, AgentState::Starting);
        registry.register_agent(record).unwrap();

        let err = registry.transition(id, AgentState::Completed).unwrap_err();
        assert_eq!(
            err,
            RegistryError::InvalidTransition {
                agent_id: id,
                from: AgentState::Starting,
                to: AgentState::Completed
            }
        );
    }

    struct EventCollector {
        events: Arc<Mutex<Vec<RuntimeEvent>>>,
    }

    impl RuntimeSubscriber for EventCollector {
        fn on_event(&self, event: &RuntimeEventEnvelope) -> RuntimeDecision {
            self.events.lock().unwrap().push(event.payload.clone());
            RuntimeDecision::Continue
        }
    }

    #[test]
    fn test_emits_agent_started_and_state_changed() {
        let bus = RuntimeBus::default();
        let events = Arc::new(Mutex::new(Vec::new()));
        bus.subscribe(Arc::new(EventCollector {
            events: Arc::clone(&events),
        }));

        let registry = RuntimeRegistry::with_bus(bus);
        let id = AgentId::new();
        let record = sample_record(id, AgentState::Starting);
        registry.register_agent(record).unwrap();
        registry.transition(id, AgentState::Running).unwrap();
        registry.transition(id, AgentState::Completed).unwrap();

        let captured = events.lock().unwrap();
        assert_eq!(captured.len(), 3);
        match &captured[0] {
            RuntimeEvent::AgentStarted { record } => assert_eq!(record.id, id),
            other => panic!("expected AgentStarted, got {other:?}"),
        }
        assert_eq!(
            captured[1],
            RuntimeEvent::AgentStateChanged {
                from: AgentState::Starting,
                to: AgentState::Running
            }
        );
        assert_eq!(
            captured[2],
            RuntimeEvent::AgentStateChanged {
                from: AgentState::Running,
                to: AgentState::Completed
            }
        );
    }
}
