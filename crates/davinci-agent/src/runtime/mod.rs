//! Shared runtime subsystem for Davinci.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;

pub mod bus;
pub mod events;
pub mod ids;

pub use bus::{RuntimeBus, RuntimeDecision, RuntimeSubscriber};
pub use events::{AgentKind, AgentRecord, AgentState, RuntimeEvent, RuntimeEventEnvelope};
pub use ids::{AgentId, RunId, TaskId, WorkflowId};

/// Handle held by an executing Agent or worker to participate in the shared runtime.
#[derive(Clone)]
pub struct RuntimeHandle {
    pub run_id: RunId,
    pub agent_id: AgentId,
    pub parent_agent_id: Option<AgentId>,
    pub session_id: Option<String>,
    sequence: Arc<AtomicU64>,
    pub bus: RuntimeBus,
    pub cancellation_token: Arc<AtomicBool>,
}

impl std::fmt::Debug for RuntimeHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeHandle")
            .field("run_id", &self.run_id)
            .field("agent_id", &self.agent_id)
            .field("parent_agent_id", &self.parent_agent_id)
            .field("session_id", &self.session_id)
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

impl RuntimeHandle {
    pub fn new(run_id: RunId, agent_id: AgentId, bus: RuntimeBus) -> Self {
        Self {
            run_id,
            agent_id,
            parent_agent_id: None,
            session_id: None,
            sequence: Arc::new(AtomicU64::new(0)),
            bus,
            cancellation_token: Arc::new(AtomicBool::new(false)),
        }
    }

    pub fn with_session(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    pub fn with_parent(mut self, parent_id: AgentId) -> Self {
        self.parent_agent_id = Some(parent_id);
        self
    }

    pub fn next_sequence(&self) -> u64 {
        self.sequence.fetch_add(1, Ordering::SeqCst) + 1
    }

    pub fn emit_observe(&self, payload: RuntimeEvent) {
        let envelope = RuntimeEventEnvelope::new(
            self.next_sequence(),
            self.run_id,
            self.session_id.clone(),
            Some(self.agent_id),
            self.parent_agent_id,
            payload,
        );
        self.bus.emit_observe(envelope);
    }

    pub fn emit_decision(&self, payload: RuntimeEvent) -> Result<(), String> {
        let envelope = RuntimeEventEnvelope::new(
            self.next_sequence(),
            self.run_id,
            self.session_id.clone(),
            Some(self.agent_id),
            self.parent_agent_id,
            payload,
        );
        self.bus.emit_decision(envelope)
    }

    pub fn is_cancelled(&self) -> bool {
        self.cancellation_token.load(Ordering::SeqCst)
    }

    pub fn cancel(&self) {
        self.cancellation_token.store(true, Ordering::SeqCst);
    }
}
