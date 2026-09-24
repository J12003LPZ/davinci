//! Shared runtime subsystem for Davinci.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

pub mod budget;
pub mod bus;
pub mod cache;
pub mod cancellation;
pub mod capabilities;
pub mod capacity;
pub mod checkpoints;
pub mod completion;
pub mod context;
pub mod context_manifest;
pub mod context_overlay;
pub mod context_vm;
pub mod contract_executor;
pub mod contracts;
pub mod control;
pub mod conversation;
pub mod effects;
pub mod events;
pub mod evidence;
pub mod evidence_store;
pub mod ids;
pub mod mailbox;
pub mod operations;
pub mod progress_watchdog;
pub mod registry;
pub mod rewind;
pub mod session;
pub mod source_manifest;
mod task_migration;
pub mod transactions;
pub use task_migration::LegacyTaskRecovery;
pub mod task_store;
pub mod task_transport;
pub mod tasks;
pub mod tools_agent;
pub mod tools_task;
pub mod workflow;
pub mod worktree;

pub use budget::*;
pub use bus::{RuntimeBus, RuntimeDecision, RuntimeSubscriber};
pub use cache::{
    hash_system_prompt, hash_system_prompt_with_manifest, hash_tool_names, CacheIdentity,
    CacheMissReason,
};
pub use cancellation::CancellationToken;
pub use capabilities::{
    builtin_capabilities, compute_schema_hash, conservative_replay_policy,
    default_declared_effects, default_execution_policies, output_policy_for_tool, CapabilitySource,
    ConcurrencyPolicy, DeclaredEffect, OutputPolicy, PreparedAction, ReplayPolicy,
    RuntimeCapability, RuntimeCapabilityRegistry, ToolExposureState,
};
pub use checkpoints::{
    may_mutate_with_checkpoint, BlobStore, CheckpointError, CheckpointRef, FileCapture,
    MAX_BLOB_BYTES, MAX_TASK_BLOBS_BYTES,
};
pub use context::{
    wrap_untrusted_data, ContextBroker, ContextItem, ContextPacket, ContextRequest, ContextSource,
};
pub use context_manifest::{
    overlay_application, ContextManifestEntry, PreparedContextManifest, ProvenanceKind,
};
pub use context_overlay::{overlay_change_allowed, ContextOverlay};
pub use context_vm::{
    CheckpointState, ContextImage, ContextImageEntry, ContextPageKind, ContextPageRef, ContextRoot,
    ContextVmConfig, ContextVmMode, ContextVmRuntime, ContextVmState, Episode, ProvenanceRef,
    StateDelta, StateValue,
};
pub use contract_executor::{
    effect_profile_allows, ContractExecutor, ExecutionError, ExecutorCapabilities,
};
pub use contracts::{
    check_dependency_addition, compile_plan_contract, contract_gate, extract_tool_targets,
    normalize_relative_path, path_scope_allows, path_scope_allows_case, resolve_contract_path,
    ContractError, ScopeViolation, TaskContract,
};
pub use control::{
    control_current, reduce_stop_status, retry_allowed, ControlStatus, ProcessLease,
    WorkerControlAction, WorkerControlCommand, WorkerControlReceipt, WorkerController,
    WorkerSnapshot,
};
pub use conversation::{
    ConversationError, ConversationIdentity, ConversationRuntime, ConversationSnapshot,
    ConversationState, TurnOutcome,
};
pub use effects::{effect_rewind_action, ExternalEffectReceipt, FileEffectKind, OwnedFileEffect};
pub use events::{AgentKind, AgentRecord, AgentState, RuntimeEvent, RuntimeEventEnvelope};
pub use evidence::{
    evidence_current, ArtifactRef, AssertionCounts, DimensionState, EvidenceKind, EvidenceRecord,
    EvidenceStatus, ExitOutcome,
};
pub use ids::{AgentId, EvidenceId, RunId, TaskId, WorkflowId};
pub use mailbox::{steering_state, AgentMailbox, AgentMessage, MailboxError, SteeringReceipt};
pub use operations::{
    AgentLaunchDisposition, AgentOperationAdapter, AgentOperationError, AgentOperationHandle,
    AllowedRecoveryAction, CausalFailureReport, ChildExecutionContext, ChildExecutionKind,
    DurableResultStatus, EffectCertainty, FailureReasonCode, FailureSubsystem, OperationAttempt,
    OperationSpec, OperationState, PublicationStatus, UnresolvedChild, VerificationStatus,
};
pub use progress_watchdog::*;
pub use registry::{is_valid_transition, RegistryError, RuntimeRegistry};
pub use rewind::{
    build_rewind_preview, can_apply_rewind, check_rename_conflict, plan_file_rewind,
    restore_domains, restore_kind, FileRewindPlan, RewindPreview, RewindSelection,
};
pub use source_manifest::{
    compute_manifest_digest, FileKind, ManifestEntry, SourceManifest, SourceManifestBuilder,
};
pub use task_store::{
    StatusRequest, TaskCreateRequest, TaskOperationReceipt, TaskOperationRequest,
};
pub use tasks::{
    is_valid_task_transition, TaskError, TaskOwner, TaskRecord, TaskRegistry, TaskState,
};
pub use tools_agent::{agent_message_tool, agent_status_tool, agent_stop_tool, agent_tool_specs};
pub use tools_task::{
    task_create_tool, task_get_tool, task_list_tool, task_tool_specs, task_update_tool,
};
pub use workflow::{
    find_saved_workflow, is_mutating_tool, is_mutating_tool_with_registry,
    save_workflow_to_project, validate_workflow, validate_workflow_with_capabilities,
    validate_workflow_with_permissions, workflow_run_tool, workflow_status_tool,
    workflow_tool_specs, PhaseExecutionState, PhaseStatus, WorkflowArtifact,
    WorkflowExecutionError, WorkflowExecutionState, WorkflowExecutor, WorkflowJoin,
    WorkflowPhaseSpec, WorkflowSpec, WorkflowStateError, WorkflowStateStore, WorkflowStatus,
    WorkflowValidationError, WorkflowWorkerSpec,
};
pub use worktree::{has_uncommitted_changes, WorktreeError, WorktreeLease, WorktreeManager};

/// Handle held by an executing Agent or worker to participate in the shared runtime.
#[derive(Clone)]
pub struct RuntimeHandle {
    pub cache: cache::CacheRuntime,
    pub context_vm: context_vm::ContextVmRuntime,
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
    pub operations: Option<operations::ToolOperationRuntime>,
    pub capability_registry: RuntimeCapabilityRegistry,
    pub project_trusted: bool,
    pub budget_ledger: Option<Arc<ResourceLedger>>,
    pub progress_watchdog: Arc<Mutex<ProgressWatchdog>>,
    pub blob_store: BlobStore,
    pub effect_ledger: Arc<std::sync::RwLock<Vec<OwnedFileEffect>>>,
    pub conversation: Arc<Mutex<ConversationRuntime>>,
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
        let cache = cache::CacheRuntime::default();
        let context_vm_config = context_vm::ContextVmConfig {
            mode: context_vm::ContextVmMode::from_env_value(
                std::env::var("DAVINCI_CONTEXT_VM").ok().as_deref(),
            ),
            ..Default::default()
        };
        Self {
            context_vm: context_vm::ContextVmRuntime::new(context_vm_config, cache.clone()),
            cache,
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
            operations: None,
            capability_registry: RuntimeCapabilityRegistry::with_builtins(),
            project_trusted: false,
            budget_ledger: None,
            progress_watchdog: Arc::new(Mutex::new(ProgressWatchdog::new())),
            blob_store: BlobStore::new(),
            effect_ledger: Arc::new(std::sync::RwLock::new(Vec::new())),
            conversation: Arc::new(Mutex::new(ConversationRuntime::new(run_id, agent_id, None))),
        }
    }

    pub fn with_budget_ledger(mut self, ledger: Arc<ResourceLedger>) -> Self {
        self.budget_ledger = Some(ledger);
        self
    }
    pub fn with_cache(mut self, cache: cache::CacheRuntime) -> Self {
        let config = self.context_vm.config().clone();
        self.cache = cache.clone();
        self.context_vm = context_vm::ContextVmRuntime::new(config, cache);
        self
    }

    pub fn with_capability_registry(mut self, registry: RuntimeCapabilityRegistry) -> Self {
        self.capability_registry = registry;
        self
    }

    pub fn with_workflow_executor(mut self, executor: Arc<WorkflowExecutor>) -> Self {
        self.workflow_executor = Some(executor);
        self
    }

    pub fn with_operation_runtime(mut self, operations: operations::ToolOperationRuntime) -> Self {
        self.operations = Some(operations);
        self
    }

    /// Return the journal-backed adapter shared by subagents, workflow
    /// workers, and background jobs. It never creates a second coordinator.
    pub fn child_operation_adapter(&self) -> Option<operations::AgentOperationAdapter> {
        operations::AgentOperationAdapter::from_runtime(self)
    }

    pub fn unresolved_child_operations(
        &self,
    ) -> Result<Vec<operations::UnresolvedChild>, operations::AgentOperationError> {
        self.child_operation_adapter()
            .ok_or_else(|| {
                operations::AgentOperationError::InvalidContext(
                    "operation journal is not configured".into(),
                )
            })?
            .unresolved_children()
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

    /// Continue a host-validated session with fresh turn observers and cancellation token.
    /// Retain live identities, coordination state, writer lease and event counters.
    pub fn with_session_state_from(mut self, previous: &Self) -> Self {
        self.cache = previous.cache.clone();
        self.context_vm = previous.context_vm.clone();
        self.run_id = previous.run_id;
        self.agent_id = previous.agent_id;
        self.parent_agent_id = previous.parent_agent_id;
        self.session_id = previous.session_id.clone();
        self.sequence = previous.sequence.clone();
        self.conversation = previous.conversation.clone();
        previous.bus.replace_turn_subscribers_from(&self.bus);
        self.bus = previous.bus.clone();
        self.registry = previous.registry.clone();
        self.mailbox = previous.mailbox.clone();
        self.task_registry = previous.task_registry.clone();
        self.operations = previous.operations.clone();
        self.budget_ledger = previous.budget_ledger.clone();
        self.progress_watchdog = previous.progress_watchdog.clone();
        self.worktree_manager = self
            .worktree_manager
            .map(|manager| manager.with_bus(self.bus.clone()));
        self
    }

    pub fn with_mailbox(mut self, mailbox: AgentMailbox) -> Self {
        self.mailbox = mailbox;
        self
    }

    /// Preserve a host-bound worker's coordinator without replacing parent observers.
    pub fn with_worker_state_from(mut self, worker: &Self) -> Self {
        self.cache = worker.cache.clone();
        self.context_vm = worker.context_vm.clone();
        self.run_id = worker.run_id;
        self.agent_id = worker.agent_id;
        self.parent_agent_id = worker.parent_agent_id;
        self.session_id = worker.session_id.clone();
        self.sequence = worker.sequence.clone();
        self.conversation = worker.conversation.clone();
        self.cancellation_token = worker.cancellation_token.clone();
        self.registry = worker.registry.clone();
        self.task_registry = worker.task_registry.clone();
        self.operations = worker.operations.clone();
        self.mailbox = worker.mailbox.clone();
        self.budget_ledger = worker.budget_ledger.clone();
        self.progress_watchdog = worker.progress_watchdog.clone();
        self
    }

    pub(crate) fn for_worker(
        &self,
        child: AgentId,
        cancellation: Option<CancellationToken>,
    ) -> Result<Self, String> {
        let record = self
            .registry
            .get(&child)
            .ok_or("worker is not registered")?;
        if record.run_id != self.run_id
            || record.parent != Some(self.agent_id)
            || record.state != AgentState::Running
        {
            return Err("worker does not belong to the active parent runtime".into());
        }
        let mut worker =
            Self::new(self.run_id, child, RuntimeBus::new()).with_worker_state_from(self);
        worker.agent_id = child;
        worker.parent_agent_id = Some(self.agent_id);
        worker.operations = worker
            .operations
            .map(|operations| operations.for_worker(child));
        worker.sequence = Arc::new(AtomicU64::new(0));
        worker.ensure_conversation_identity_current();
        worker.cancellation_token =
            cancellation.unwrap_or_else(|| self.cancellation_token.child_token());
        Ok(worker)
    }

    pub fn with_context_broker(mut self, broker: ContextBroker) -> Self {
        self.context_broker = broker;
        self
    }

    /// Detach a cloned runtime's projection when it is rebound to another worker.
    pub(crate) fn ensure_conversation_identity_current(&mut self) {
        let identity = ConversationIdentity {
            run_id: self.run_id,
            agent_id: self.agent_id,
            session_id: self.session_id.clone(),
        };
        let matches = self
            .conversation
            .lock()
            .map(|conversation| conversation.snapshot().identity == identity)
            .unwrap_or(false);
        if !matches {
            self.conversation = Arc::new(Mutex::new(ConversationRuntime::new(
                self.run_id,
                self.agent_id,
                self.session_id.clone(),
            )));
        }
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
        let session_id = session_id.into();
        self.session_id = Some(session_id.clone());
        if let Ok(mut conversation) = self.conversation.lock() {
            conversation.set_session_id(Some(session_id));
        }
        self
    }

    pub fn restore_conversation(&mut self, events: &[RuntimeEventEnvelope]) -> Result<(), String> {
        let identity = ConversationIdentity {
            run_id: self.run_id,
            agent_id: self.agent_id,
            session_id: self.session_id.clone(),
        };
        let restored = ConversationRuntime::replay(identity, events)
            .map_err(|e| format!("conversation runtime could not be replayed: {e}"))?;
        self.conversation = Arc::new(Mutex::new(restored));
        self.sequence.store(
            events.iter().map(|event| event.sequence).max().unwrap_or(0),
            Ordering::SeqCst,
        );
        Ok(())
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
        if let Ok(mut conversation) = self.conversation.lock() {
            if let Err(error) = conversation.apply(&envelope) {
                eprintln!("[davinci-runtime] rejected observe event: {error}");
                return;
            }
        }
        self.bus.emit_observe(envelope);
    }

    /// Emit a structured, redacted failure report as an observation.  The
    /// operation journal remains authoritative for transitions; this event is
    /// a derived host/UI projection and never carries execution authority.
    pub fn emit_causal_failure(&self, report: &CausalFailureReport) {
        let message = serde_json::to_string(report)
            .unwrap_or_else(|_| "{\"schema_version\":1,\"serialization_error\":true}".into());
        self.emit_observe(RuntimeEvent::RuntimeWarning {
            code: "causal_failure".into(),
            message,
        });
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
        if let Ok(mut conversation) = self.conversation.lock() {
            conversation
                .apply(&envelope)
                .map_err(|error| error.to_string())?;
        }
        self.bus.emit_decision(envelope)
    }

    pub fn emit_turn_end(&self, success: bool) {
        self.emit_observe(RuntimeEvent::TurnEnded { success });
    }

    pub fn mark_turn_failed(&self) {
        if let Ok(mut conversation) = self.conversation.lock() {
            let _ = conversation.mark_turn_failed();
        }
    }

    pub fn emit_session_started(&self, resumed: bool) {
        self.emit_observe(RuntimeEvent::SessionStarted { resumed });
    }

    pub fn emit_session_ended(&self, reason: impl Into<String>) {
        self.emit_observe(RuntimeEvent::SessionEnded {
            reason: reason.into(),
        });
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

#[cfg(test)]
mod conversation_identity_tests {
    use super::*;

    fn runtime_with_operations() -> (RuntimeHandle, tempfile::TempDir) {
        use operations::{
            ExecutionOwner, ExecutionOwnerId, JournalId, JournalIdentity, OperationContext,
            OperationJournal, RootNamespaceId, ToolOperationRuntime, WorkspaceId,
            WorkspaceIdentity,
        };

        let workspace = tempfile::tempdir().unwrap();
        let root_namespace_id = RootNamespaceId::new();
        let identity = JournalIdentity::new(
            JournalId::new(),
            WorkspaceIdentity {
                id: WorkspaceId::new(),
                binding_version: 1,
            },
        )
        .unwrap();
        let run_id = RunId::new();
        let agent_id = AgentId::new();
        let context = OperationContext {
            journal_id: identity.journal_id,
            root_namespace_id,
            session_id: "runtime-context-test".into(),
            runtime_run_id: run_id,
            parent_operation_id: None,
            agent_id,
            worker_id: None,
            task_id: None,
            graph: None,
            workspace: identity.workspace.clone(),
            caller: operations::CallerType::ProviderToolCall,
            wire_tool_call_id: None,
        };
        let journal = Arc::new(
            OperationJournal::open(
                &workspace.path().join("journal"),
                identity,
                root_namespace_id,
            )
            .unwrap(),
        );
        let operations = ToolOperationRuntime::new(
            journal,
            context,
            ExecutionOwner::new(ExecutionOwnerId::new(), 1).unwrap(),
            workspace.path(),
        )
        .unwrap();
        let mut runtime = RuntimeHandle::new(run_id, agent_id, RuntimeBus::new())
            .with_operation_runtime(operations);
        runtime.session_id = Some("runtime-context-test".into());
        (runtime, workspace)
    }

    #[test]
    fn worker_refresh_preserves_conversation_and_sequence() {
        let worker = RuntimeHandle::new(RunId::new(), AgentId::new(), RuntimeBus::new());
        worker.emit_session_started(false);
        let refreshed = RuntimeHandle::new(RunId::new(), AgentId::new(), RuntimeBus::new())
            .with_worker_state_from(&worker);
        assert!(Arc::ptr_eq(&refreshed.conversation, &worker.conversation));
        refreshed.emit_session_started(true);
        assert_eq!(
            refreshed
                .conversation
                .lock()
                .unwrap()
                .snapshot()
                .last_sequence,
            2
        );
    }
    #[test]
    fn cloned_worker_identity_detaches_without_resetting_parent() {
        let parent = RuntimeHandle::new(RunId::new(), AgentId::new(), RuntimeBus::new());
        parent.emit_session_started(false);
        let mut worker = parent.clone();
        worker.agent_id = AgentId::new();
        worker.ensure_conversation_identity_current();
        assert!(!Arc::ptr_eq(&worker.conversation, &parent.conversation));
        worker.emit_session_started(false);
        assert_eq!(
            worker
                .conversation
                .lock()
                .unwrap()
                .snapshot()
                .identity
                .agent_id,
            worker.agent_id
        );
        assert_eq!(
            parent.conversation.lock().unwrap().snapshot().last_sequence,
            1
        );
        let retained = worker.conversation.clone();
        worker.ensure_conversation_identity_current();
        assert!(Arc::ptr_eq(&worker.conversation, &retained));
    }

    #[test]
    fn operation_context_survives_continuation_and_is_scoped_to_child_workers() {
        let (parent, workspace) = runtime_with_operations();
        let continued = RuntimeHandle::new(RunId::new(), AgentId::new(), RuntimeBus::new())
            .with_session_state_from(&parent);
        let continued_context = continued.operations.as_ref().unwrap().context_for(
            continued.run_id,
            continued.agent_id,
            continued.session_id.as_deref(),
        );
        assert_eq!(continued_context.runtime_run_id, parent.run_id);
        assert_eq!(continued_context.agent_id, parent.agent_id);
        assert_eq!(continued_context.session_id, "runtime-context-test");

        let child_id = AgentId::new();
        parent
            .registry
            .register_agent(AgentRecord {
                id: child_id,
                run_id: parent.run_id,
                parent: Some(parent.agent_id),
                kind: AgentKind::Subagent,
                name: "operation-context-child".into(),
                provider: String::new(),
                model_id: String::new(),
                cwd: workspace.path().to_path_buf(),
                state: AgentState::Starting,
                task_id: None,
                worktree: None,
                started_ms: 1,
                updated_ms: 1,
                failure_reason: None,
            })
            .unwrap();
        parent
            .registry
            .transition(child_id, AgentState::Running)
            .unwrap();
        let child = parent.for_worker(child_id, None).unwrap();
        let child_context = child.operations.as_ref().unwrap().context_for(
            child.run_id,
            child.agent_id,
            child.session_id.as_deref(),
        );
        assert_eq!(child_context.agent_id, child_id);
        let child_worker_id = child_id.to_string();
        assert_eq!(
            child_context.worker_id.as_deref(),
            Some(child_worker_id.as_str())
        );

        let parent_context = parent.operations.as_ref().unwrap().context_for(
            parent.run_id,
            parent.agent_id,
            parent.session_id.as_deref(),
        );
        assert_eq!(parent_context.agent_id, parent.agent_id);
        assert!(parent_context.worker_id.is_none());
    }
}
