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
        Running => matches!(
            to,
            Waiting | Idle | Stopping | Completed | Failed | Cancelled
        ),
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
    generations: Arc<RwLock<HashMap<AgentId, u64>>>,
    revisions: Arc<RwLock<HashMap<AgentId, u64>>>,
    last_activity_ms: Arc<RwLock<HashMap<AgentId, i64>>>,
    tool_counts: Arc<RwLock<HashMap<AgentId, u64>>>,
}

impl RuntimeRegistry {
    pub fn new() -> Self {
        Self {
            records: Arc::new(RwLock::new(HashMap::new())),
            bus: None,
            seq: Arc::new(AtomicU64::new(0)),
            generations: Arc::new(RwLock::new(HashMap::new())),
            revisions: Arc::new(RwLock::new(HashMap::new())),
            last_activity_ms: Arc::new(RwLock::new(HashMap::new())),
            tool_counts: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    pub fn with_bus(bus: RuntimeBus) -> Self {
        Self {
            records: Arc::new(RwLock::new(HashMap::new())),
            bus: Some(bus),
            seq: Arc::new(AtomicU64::new(0)),
            generations: Arc::new(RwLock::new(HashMap::new())),
            revisions: Arc::new(RwLock::new(HashMap::new())),
            last_activity_ms: Arc::new(RwLock::new(HashMap::new())),
            tool_counts: Arc::new(RwLock::new(HashMap::new())),
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
            let mut map = self
                .records
                .write()
                .map_err(|_| RegistryError::InvalidTransition {
                    agent_id: record.id,
                    from: record.state,
                    to: record.state,
                })?;
            if map.contains_key(&record.id) {
                return Err(RegistryError::DuplicateAgent(record.id));
            }
            map.insert(record.id, record.clone());
        }

        if let Ok(mut gens) = self.generations.write() {
            gens.entry(record.id).or_insert(1);
        }
        if let Ok(mut revs) = self.revisions.write() {
            revs.entry(record.id).or_insert(1);
        }
        if let Ok(mut act) = self.last_activity_ms.write() {
            act.entry(record.id).or_insert(record.started_ms);
        }
        if let Ok(mut tools) = self.tool_counts.write() {
            tools.entry(record.id).or_insert(0);
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
            let mut map = self
                .records
                .write()
                .map_err(|_| RegistryError::InvalidTransition {
                    agent_id: id,
                    from: to,
                    to,
                })?;
            let record = map.get_mut(&id).ok_or(RegistryError::AgentNotFound(id))?;

            let from = record.state;
            if matches!(
                from,
                AgentState::Completed | AgentState::Failed | AgentState::Cancelled
            ) {
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

        self.record_activity(&id);
        self.advance_revision(&id);

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

    pub fn get_generation(&self, id: &AgentId) -> u64 {
        self.generations
            .read()
            .ok()
            .and_then(|g| g.get(id).copied())
            .unwrap_or(1)
    }

    pub fn advance_generation(&self, id: &AgentId) -> u64 {
        if let Ok(mut g) = self.generations.write() {
            let val = g.entry(*id).or_insert(1);
            *val += 1;
            *val
        } else {
            1
        }
    }

    pub fn get_revision(&self, id: &AgentId) -> u64 {
        self.revisions
            .read()
            .ok()
            .and_then(|r| r.get(id).copied())
            .unwrap_or(1)
    }

    pub fn advance_revision(&self, id: &AgentId) -> u64 {
        if let Ok(mut r) = self.revisions.write() {
            let val = r.entry(*id).or_insert(1);
            *val += 1;
            *val
        } else {
            1
        }
    }

    pub fn record_activity(&self, id: &AgentId) {
        if let Ok(mut a) = self.last_activity_ms.write() {
            a.insert(*id, Self::now_ms());
        }
    }

    pub fn get_last_activity(&self, id: &AgentId) -> i64 {
        self.last_activity_ms
            .read()
            .ok()
            .and_then(|a| a.get(id).copied())
            .unwrap_or_else(Self::now_ms)
    }

    pub fn increment_tool_count(&self, id: &AgentId) -> u64 {
        self.record_activity(id);
        if let Ok(mut t) = self.tool_counts.write() {
            let val = t.entry(*id).or_insert(0);
            *val += 1;
            *val
        } else {
            0
        }
    }

    pub fn get_tool_count(&self, id: &AgentId) -> u64 {
        self.tool_counts
            .read()
            .ok()
            .and_then(|t| t.get(id).copied())
            .unwrap_or(0)
    }

    #[allow(clippy::too_many_arguments)]
    pub fn emit_control_ack(
        &self,
        command_id: uuid::Uuid,
        task_id: Option<super::ids::TaskId>,
        agent_id: AgentId,
        generation: u64,
        action: String,
        status: String,
        reason: Option<String>,
    ) {
        if let Some(bus) = &self.bus {
            let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
            let run_id = self.get(&agent_id).map(|r| r.run_id).unwrap_or_default();
            let envelope = RuntimeEventEnvelope::new(
                seq,
                run_id,
                None,
                Some(agent_id),
                None,
                RuntimeEvent::WorkerControlAcknowledged {
                    command_id,
                    task_id,
                    agent_id,
                    generation,
                    action,
                    status,
                    reason,
                },
            );
            bus.emit_observe(envelope);
        }
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

    /// Rehydrate the registry from a slice of historical runtime event envelopes.
    ///
    /// Rules:
    /// - Agents recorded via `AgentStarted` are inserted.
    /// - `AgentStateChanged` updates the state.
    /// - Terminal states (`Completed`, `Failed`, `Cancelled`) stay terminal.
    /// - Non-terminal states (`Starting`, `Running`, `Waiting`, `Idle`, `Stopping`)
    ///   at the end of the log represent agents that were active when the process died;
    ///   they transition to `Failed` with reason `process_terminated` unless reconnectable.
    pub fn rehydrate_from_events(
        &self,
        events: &[RuntimeEventEnvelope],
    ) -> Result<(), RegistryError> {
        let mut map = self
            .records
            .write()
            .map_err(|_| RegistryError::InvalidTransition {
                agent_id: AgentId::new(),
                from: AgentState::Starting,
                to: AgentState::Failed,
            })?;

        for envelope in events {
            match &envelope.payload {
                RuntimeEvent::AgentStarted { record } => {
                    map.insert(record.id, record.clone());
                }
                RuntimeEvent::AgentStateChanged { to, .. } => {
                    if let Some(agent_id) = envelope.agent_id {
                        if let Some(rec) = map.get_mut(&agent_id) {
                            rec.state = *to;
                            rec.updated_ms = envelope.timestamp_ms;
                        }
                    }
                }
                _ => {}
            }
        }

        let now = Self::now_ms();
        for rec in map.values_mut() {
            if !matches!(
                rec.state,
                AgentState::Completed | AgentState::Failed | AgentState::Cancelled
            ) && !rec.is_reconnectable()
            {
                rec.state = AgentState::Failed;
                rec.failure_reason = Some("process_terminated".to_string());
                rec.updated_ms = now;
            }
        }

        Ok(())
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
            failure_reason: None,
        }
    }

    #[test]
    fn test_valid_transitions_starting() {
        assert!(is_valid_transition(
            AgentState::Starting,
            AgentState::Running
        ));
        assert!(is_valid_transition(
            AgentState::Starting,
            AgentState::Failed
        ));
        assert!(is_valid_transition(
            AgentState::Starting,
            AgentState::Cancelled
        ));
        assert!(!is_valid_transition(
            AgentState::Starting,
            AgentState::Completed
        ));
        assert!(!is_valid_transition(AgentState::Starting, AgentState::Idle));
    }

    #[test]
    fn test_valid_transitions_running() {
        assert!(is_valid_transition(
            AgentState::Running,
            AgentState::Waiting
        ));
        assert!(is_valid_transition(AgentState::Running, AgentState::Idle));
        assert!(is_valid_transition(
            AgentState::Running,
            AgentState::Stopping
        ));
        assert!(is_valid_transition(
            AgentState::Running,
            AgentState::Completed
        ));
        assert!(is_valid_transition(AgentState::Running, AgentState::Failed));
        assert!(is_valid_transition(
            AgentState::Running,
            AgentState::Cancelled
        ));
        assert!(!is_valid_transition(
            AgentState::Running,
            AgentState::Starting
        ));
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

    #[test]
    fn test_rehydrate_from_events_terminal_and_crashed() {
        let registry = RuntimeRegistry::new();
        let run_id = RunId::new();

        let term_id = AgentId::new();
        let running_id = AgentId::new();

        let term_rec = sample_record(term_id, AgentState::Starting);
        let running_rec = sample_record(running_id, AgentState::Starting);

        let events = vec![
            RuntimeEventEnvelope::new(
                1,
                run_id,
                None,
                Some(term_id),
                None,
                RuntimeEvent::AgentStarted { record: term_rec },
            ),
            RuntimeEventEnvelope::new(
                2,
                run_id,
                None,
                Some(term_id),
                None,
                RuntimeEvent::AgentStateChanged {
                    from: AgentState::Starting,
                    to: AgentState::Completed,
                },
            ),
            RuntimeEventEnvelope::new(
                3,
                run_id,
                None,
                Some(running_id),
                None,
                RuntimeEvent::AgentStarted {
                    record: running_rec,
                },
            ),
            RuntimeEventEnvelope::new(
                4,
                run_id,
                None,
                Some(running_id),
                None,
                RuntimeEvent::AgentStateChanged {
                    from: AgentState::Starting,
                    to: AgentState::Running,
                },
            ),
        ];

        registry.rehydrate_from_events(&events).unwrap();

        // Terminal agent must remain Completed
        let term_loaded = registry.get(&term_id).expect("term agent present");
        assert_eq!(term_loaded.state, AgentState::Completed);
        assert_eq!(term_loaded.failure_reason, None);

        // Running agent must be safely marked Failed with "process_terminated"
        let running_loaded = registry.get(&running_id).expect("running agent present");
        assert_eq!(running_loaded.state, AgentState::Failed);
        assert_eq!(
            running_loaded.failure_reason.as_deref(),
            Some("process_terminated")
        );
    }
}
