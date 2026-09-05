//! Shared runtime subsystem for Davinci.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

pub mod bus;
pub mod cache;
pub mod cancellation;
pub mod context;
pub mod events;
pub mod ids;
pub mod mailbox;
pub mod registry;
pub mod tasks;
pub mod team;
pub mod tools_agent;
pub mod tools_task;
pub mod workflow;
pub mod worktree;

pub use bus::{RuntimeBus, RuntimeDecision, RuntimeSubscriber};
pub use cache::{hash_system_prompt, hash_tool_names, CacheIdentity, CacheMissReason};
pub use cancellation::CancellationToken;
pub use context::{
    wrap_untrusted_data, ContextBroker, ContextItem, ContextPacket, ContextRequest, ContextSource,
};
pub use events::{AgentKind, AgentRecord, AgentState, RuntimeEvent, RuntimeEventEnvelope};
pub use ids::{AgentId, RunId, TaskId, WorkflowId};
pub use mailbox::{AgentMailbox, AgentMessage, MailboxError};
pub use registry::{is_valid_transition, RegistryError, RuntimeRegistry};
pub use tasks::{is_valid_task_transition, TaskError, TaskRecord, TaskRegistry, TaskState};
pub use team::{TeamConfig, TeamError, TeamManager, TeammateHandle};
pub use tools_agent::{agent_message_tool, agent_status_tool, agent_stop_tool, agent_tool_specs};
pub use tools_task::{task_create_tool, task_list_tool, task_tool_specs, task_update_tool};
pub use workflow::{
    find_saved_workflow, is_mutating_tool, save_workflow_to_project, validate_workflow,
    validate_workflow_with_permissions, workflow_run_tool, workflow_status_tool,
    workflow_tool_specs, PhaseExecutionState, PhaseStatus, WorkflowArtifact,
    WorkflowExecutionError, WorkflowExecutionState, WorkflowExecutor, WorkflowJoin,
    WorkflowPhaseSpec, WorkflowSpec, WorkflowStateError, WorkflowStateStore, WorkflowStatus,
    WorkflowValidationError, WorkflowWorkerSpec,
};
pub use worktree::{WorktreeError, WorktreeLease, WorktreeManager};

/// Handle held by an executing Agent or worker to participate in the shared runtime.
#[derive(Clone)]
pub struct RuntimeHandle {
    pub run_id: RunId,
    pub agent_id: AgentId,
    pub parent_agent_id: Option<AgentId>,
    pub session_id: Option<String>,
    sequence: Arc<AtomicU64>,
    pub bus: RuntimeBus,
    pub cancellation_token: CancellationToken,
    pub registry: RuntimeRegistry,
    pub context_broker: ContextBroker,
    pub task_registry: TaskRegistry,
    pub mailbox: AgentMailbox,
    pub worktree_manager: Option<WorktreeManager>,
    pub workflow_executor: Option<Arc<WorkflowExecutor>>,
    pub project_trusted: bool,
}

impl std::fmt::Debug for RuntimeHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RuntimeHandle")
            .field("run_id", &self.run_id)
            .field("agent_id", &self.agent_id)
            .field("parent_agent_id", &self.parent_agent_id)
            .field("session_id", &self.session_id)
            .field("cancelled", &self.is_cancelled())
            .field("project_trusted", &self.project_trusted)
            .finish()
    }
}

impl RuntimeHandle {
    pub fn new(run_id: RunId, agent_id: AgentId, bus: RuntimeBus) -> Self {
        let registry = RuntimeRegistry::with_bus(bus.clone());
        let task_registry = TaskRegistry::with_bus(bus.clone());
        let mailbox = AgentMailbox::with_registry_and_bus(registry.clone(), bus.clone());
        Self {
            run_id,
            agent_id,
            parent_agent_id: None,
            session_id: None,
            sequence: Arc::new(AtomicU64::new(0)),
            bus,
            cancellation_token: CancellationToken::new(),
            registry,
            context_broker: ContextBroker::new(),
            task_registry,
            mailbox,
            worktree_manager: None,
            workflow_executor: None,
            project_trusted: false,
        }
    }

    pub fn with_workflow_executor(mut self, executor: Arc<WorkflowExecutor>) -> Self {
        self.workflow_executor = Some(executor);
        self
    }

    pub fn with_project_trusted(mut self, trusted: bool) -> Self {
        self.project_trusted = trusted;
        self
    }

    pub fn with_worktree_manager(mut self, mgr: WorktreeManager) -> Self {
        self.worktree_manager = Some(mgr);
        self
    }

    pub fn with_task_registry(mut self, task_registry: TaskRegistry) -> Self {
        self.task_registry = task_registry;
        self
    }

    pub fn with_mailbox(mut self, mailbox: AgentMailbox) -> Self {
        self.mailbox = mailbox;
        self
    }

    pub fn with_context_broker(mut self, broker: ContextBroker) -> Self {
        self.context_broker = broker;
        self
    }

    pub fn with_cancellation_token(mut self, token: CancellationToken) -> Self {
        self.cancellation_token = token;
        self
    }

    pub fn with_registry(mut self, registry: RuntimeRegistry) -> Self {
        self.registry = registry;
        self
    }

    pub fn with_session(mut self, session_id: impl Into<String>) -> Self {
        self.session_id = Some(session_id.into());
        self
    }

    pub fn with_parent(mut self, parent_id: AgentId) -> Self {
        self.parent_agent_id = Some(parent_id);
        self
    }

    pub fn register_context_source(&self, source: Arc<dyn ContextSource>) {
        self.context_broker.register_context_source(source);
    }

    pub fn build_context(&self, request: &ContextRequest) -> ContextPacket {
        self.context_broker.build_context(request)
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
        self.cancellation_token.is_cancelled()
    }

    pub fn cancel(&self) {
        self.cancellation_token.cancel();
    }

    pub fn send_message(
        &self,
        to: AgentId,
        content: impl Into<String>,
    ) -> Result<uuid::Uuid, MailboxError> {
        let msg = AgentMessage::new(self.run_id, self.agent_id, to, content);
        let id = msg.id;
        self.mailbox.send(msg)?;
        Ok(id)
    }

    pub fn drain_messages(&self, limit: usize) -> Vec<AgentMessage> {
        self.mailbox.drain(self.agent_id, limit)
    }

    /// Rehydrate runtime registry, tasks, and mailboxes from historical event log envelopes.
    pub fn rehydrate_from_log(&self, events: &[RuntimeEventEnvelope]) -> Result<(), String> {
        self.registry
            .rehydrate_from_events(events)
            .map_err(|e| format!("registry rehydration failed: {e}"))?;
        self.task_registry
            .rehydrate_from_events(events)
            .map_err(|e| format!("task registry rehydration failed: {e}"))?;
        self.mailbox.rehydrate_from_events(events);
        Ok(())
    }
}
