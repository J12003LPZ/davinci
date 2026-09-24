//! Dependency-aware shared task registry for multi-agent coordination.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::bus::RuntimeBus;
use super::completion::CompletionEvaluation;
use super::events::{RuntimeEvent, RuntimeEventEnvelope};
use super::ids::{AgentId, EvidenceId, RunId, TaskId};
use super::source_manifest::SourceManifest;
use super::task_store::{
    TaskChange, TaskCommitSink, TaskCreateRequest, TaskJournal, TaskOperationReceipt,
    TaskOperationRequest, MAX_OPERATION_RECEIPTS,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskState {
    Pending,
    Ready,
    Running,
    Completed,
    Failed,
    Blocked,
    Cancelled,
}

impl TaskState {
    /// Public board vocabulary; stored state retains dependency readiness.
    pub fn public_status(self) -> &'static str {
        match self {
            Self::Pending | Self::Ready => "pending",
            Self::Running => "in_progress",
            Self::Completed => "completed",
            Self::Failed => "failed",
            Self::Blocked => "blocked",
            Self::Cancelled => "cancelled",
        }
    }

    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed | Self::Failed | Self::Cancelled)
    }
}

/// Returns true if transitioning from `from` to `to` is valid for a task.
pub fn is_valid_task_transition(from: TaskState, to: TaskState) -> bool {
    use TaskState::*;
    match from {
        Pending => matches!(to, Ready | Blocked | Cancelled),
        Ready => matches!(to, Running | Completed | Failed | Blocked | Cancelled),
        Running => matches!(to, Ready | Completed | Failed | Blocked | Cancelled),
        Blocked => matches!(to, Ready | Cancelled),
        Completed | Failed | Cancelled => false,
    }
}

/// An exact historical plan step, not execution consent or a mutable title.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PlanStepRef {
    pub plan_id: String,
    pub plan_revision: u64,
    pub step_id: String,
}

/// A structured decision prerequisite for a task, referring to decision ID and revision, not array index.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DecisionPrerequisiteRef {
    pub decision_id: String,
    pub expected_revision: u64,
}

impl DecisionPrerequisiteRef {
    pub fn new(decision_id: impl Into<String>, expected_revision: u64) -> Self {
        Self {
            decision_id: decision_id.into(),
            expected_revision,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BlockReason {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TaskRecord {
    pub id: TaskId,
    pub run_id: RunId,
    pub title: String,
    pub description: Option<String>,
    pub dependencies: Vec<TaskId>,
    pub state: TaskState,
    pub assigned_to: Option<AgentId>,
    pub result: Option<String>,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    #[serde(default)]
    pub revision: u64,
    #[serde(default)]
    pub owner_generation: u64,
    #[serde(default)]
    pub parent_plan_step: Option<PlanStepRef>,
    #[serde(default)]
    pub decision_prerequisites: Vec<DecisionPrerequisiteRef>,
    #[serde(default)]
    pub evidence_refs: Vec<EvidenceId>,
    #[serde(default)]
    pub blocked_reasons: Vec<BlockReason>,
    #[serde(default)]
    pub attempt: u32,
    #[serde(default)]
    pub contract_digest: Option<String>,
}

impl TaskRecord {
    fn advance_revision(&mut self, committed_at_ms: i64) -> Result<(), TaskError> {
        let revision = self
            .revision
            .checked_add(1)
            .ok_or(TaskError::RevisionOverflow(self.id))?;
        self.revision = revision;
        self.updated_at_ms = committed_at_ms;
        Ok(())
    }

    /// Validate bounded metadata before accepting a record into the registry.
    pub fn validate(&self) -> Result<(), TaskError> {
        for (field, size, limit) in [
            ("title", self.title.len(), 512),
            (
                "description",
                self.description.as_ref().map_or(0, String::len),
                16 * 1024,
            ),
            ("dependencies", self.dependencies.len(), 64),
            ("evidence_refs", self.evidence_refs.len(), 128),
            (
                "decision_prerequisites",
                self.decision_prerequisites.len(),
                64,
            ),
        ] {
            if size > limit {
                return Err(TaskError::MetadataTooLarge { field, limit });
            }
        }
        if let Some(ref digest) = self.contract_digest {
            if digest.len() > 128 {
                return Err(TaskError::MetadataTooLarge {
                    field: "contract_digest",
                    limit: 128,
                });
            }
        }
        Ok(())
    }

    pub fn attach_evidence(&mut self, evidence_id: EvidenceId) -> Result<(), TaskError> {
        if self.evidence_refs.contains(&evidence_id) {
            return Ok(());
        }
        if self.evidence_refs.len() >= 128 {
            return Err(TaskError::MetadataTooLarge {
                field: "evidence_refs",
                limit: 128,
            });
        }
        self.evidence_refs.push(evidence_id);
        Ok(())
    }

    pub fn new(run_id: RunId, title: impl Into<String>) -> Self {
        let now = now_ms();
        Self {
            id: TaskId::new(),
            run_id,
            title: title.into(),
            description: None,
            dependencies: Vec::new(),
            state: TaskState::Ready,
            assigned_to: None,
            result: None,
            created_at_ms: now,
            updated_at_ms: now,
            revision: 0,
            owner_generation: 0,
            parent_plan_step: None,
            decision_prerequisites: Vec::new(),
            evidence_refs: Vec::new(),
            blocked_reasons: Vec::new(),
            attempt: 0,
            contract_digest: None,
        }
    }

    pub fn with_description(mut self, desc: impl Into<String>) -> Self {
        self.description = Some(desc.into());
        self
    }

    pub fn with_dependencies(mut self, deps: Vec<TaskId>) -> Self {
        self.dependencies = deps;
        self
    }

    pub fn with_decision_prerequisites(mut self, prereqs: Vec<DecisionPrerequisiteRef>) -> Self {
        self.decision_prerequisites = prereqs;
        self
    }

    pub fn with_assigned(mut self, agent_id: AgentId) -> Self {
        self.assigned_to = Some(agent_id);
        self
    }

    pub fn with_contract_digest(mut self, digest: impl Into<String>) -> Self {
        self.contract_digest = Some(digest.into());
        self
    }
}

#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum TaskError {
    #[error("operation is already in progress; retry after it settles")]
    OperationInProgress,
    #[error("operation ID was already used with a different request")]
    OperationConflict,
    #[error("caller does not own task: {0}")]
    NotOwner(TaskId),
    #[error("task registry lock poisoned")]
    RegistryPoisoned,
    #[error("task belongs to another run: {0}")]
    RunMismatch(TaskId),
    #[error("task persistence failed: {0}")]
    Persistence(String),
    #[error("task revision exhausted: {0}")]
    RevisionOverflow(TaskId),
    #[error("task revision conflict: {0}")]
    RevisionConflict(TaskId),
    #[error("task {field} exceeds limit {limit}")]
    MetadataTooLarge { field: &'static str, limit: usize },
    #[error("task already exists: {0}")]
    DuplicateTask(TaskId),
    #[error("task not found: {0}")]
    TaskNotFound(TaskId),
    #[error("self-dependency is not allowed: {0}")]
    SelfDependency(TaskId),
    #[error("unknown dependency: {0}")]
    UnknownDependency(TaskId),
    #[error("duplicate dependency: {0}")]
    DuplicateDependency(TaskId),
    #[error("cycle detected involving task {0}")]
    CycleDetected(TaskId),
    #[error("invalid task transition for task {task_id} from {from:?} to {to:?}")]
    InvalidTransition {
        task_id: TaskId,
        from: TaskState,
        to: TaskState,
    },
    #[error("task completion refused: {0}")]
    CompletionRefused(String),
    #[error("cannot modify terminal task {task_id} in state {state:?}")]
    TerminalTask { task_id: TaskId, state: TaskState },
    #[error("task dependency not completed for task {0}")]
    DependencyNotCompleted(TaskId),
}

/// Shared by live creation and replay of creation receipts.
pub(super) fn initial_task_state(
    task: &TaskRecord,
    tasks: &HashMap<TaskId, TaskRecord>,
    lineage: Option<&TaskLineage>,
) -> Result<TaskState, TaskError> {
    if !task.blocked_reasons.is_empty() {
        return Ok(TaskState::Blocked);
    }
    let mut seen = HashSet::new();
    let mut any_failed = false;
    let mut all_completed = true;
    for dep_id in &task.dependencies {
        if *dep_id == task.id {
            return Err(TaskError::SelfDependency(task.id));
        }
        if !seen.insert(*dep_id) {
            return Err(TaskError::DuplicateDependency(*dep_id));
        }
        let dependency = tasks
            .get(dep_id)
            .ok_or(TaskError::UnknownDependency(*dep_id))?;
        if dependency.run_id != task.run_id
            && !lineage.is_some_and(|scope| {
                scope.contains(dependency.run_id) && scope.contains(task.run_id)
            })
        {
            return Err(TaskError::RunMismatch(*dep_id));
        }
        if path_exists(tasks, *dep_id, task.id) {
            return Err(TaskError::CycleDetected(task.id));
        }
        any_failed |= matches!(
            dependency.state,
            TaskState::Failed | TaskState::Blocked | TaskState::Cancelled
        );
        all_completed &= dependency.state == TaskState::Completed;
    }
    Ok(if any_failed {
        TaskState::Blocked
    } else if all_completed {
        TaskState::Ready
    } else {
        TaskState::Pending
    })
}

pub fn evaluate_decision_prerequisites(
    prerequisites: &[DecisionPrerequisiteRef],
    plan: &crate::LivingPlan,
    cwd: &std::path::Path,
) -> Vec<BlockReason> {
    let mut reasons = Vec::new();
    for prereq in prerequisites {
        let Some(decision) = plan.structured_decisions.get(&prereq.decision_id) else {
            reasons.push(BlockReason {
                code: "missing_decision".into(),
                message: format!("Decision {} not found in living plan", prereq.decision_id),
            });
            continue;
        };
        if decision.state != crate::decisions::DecisionState::AnsweredByUser {
            reasons.push(BlockReason {
                code: "unresolved_decision".into(),
                message: format!(
                    "Decision {} is {:?}, not answered by user",
                    prereq.decision_id, decision.state
                ),
            });
            continue;
        }
        if decision.plan_revision != prereq.expected_revision {
            reasons.push(BlockReason {
                code: "revision_mismatch".into(),
                message: format!(
                    "Decision {} revision mismatch: expected {}, plan has {}",
                    prereq.decision_id, prereq.expected_revision, decision.plan_revision
                ),
            });
            continue;
        }
        for (evidence_ref, issued_fingerprint) in &decision.evidence_fingerprints {
            match plan.decision_evidence_fingerprint(evidence_ref, cwd) {
                Ok(current) if &current == issued_fingerprint => {}
                _ => {
                    reasons.push(BlockReason {
                        code: "stale_evidence".into(),
                        message: format!(
                            "Decision {} evidence {} is stale",
                            prereq.decision_id, evidence_ref
                        ),
                    });
                }
            }
        }
    }
    reasons
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

fn path_exists(tasks: &HashMap<TaskId, TaskRecord>, from: TaskId, target: TaskId) -> bool {
    let mut visited = HashSet::new();
    let mut queue = VecDeque::new();
    queue.push_back(from);
    visited.insert(from);

    while let Some(current) = queue.pop_front() {
        if current == target {
            return true;
        }
        if let Some(task) = tasks.get(&current) {
            for dep in &task.dependencies {
                if visited.insert(*dep) {
                    queue.push_back(*dep);
                }
            }
        }
    }
    false
}

fn collect_downstream_dependents(tasks: &HashMap<TaskId, TaskRecord>, root: TaskId) -> Vec<TaskId> {
    let mut dependents = Vec::new();
    for id in tasks.keys() {
        if *id != root && path_exists(tasks, *id, root) {
            dependents.push(*id);
        }
    }
    dependents
}

/// Expected ownership supplied by a trusted host adapter, never model identity fields.
#[derive(Clone, Copy)]
pub struct TaskOwner {
    pub run_id: RunId,
    pub agent_id: AgentId,
    pub revision: u64,
    pub generation: u64,
}

impl TaskOwner {
    pub fn new(run_id: RunId, agent_id: AgentId, revision: u64, generation: u64) -> Self {
        Self {
            run_id,
            agent_id,
            revision,
            generation,
        }
    }

    fn validate(self, task: &TaskRecord, lineage: Option<&TaskLineage>) -> Result<(), TaskError> {
        if !run_matches(lineage, self.run_id, task.run_id) {
            return Err(TaskError::RunMismatch(task.id));
        }
        if task.assigned_to != Some(self.agent_id) {
            return Err(TaskError::NotOwner(task.id));
        }
        if task.revision != self.revision || task.owner_generation != self.generation {
            return Err(TaskError::RevisionConflict(task.id));
        }
        Ok(())
    }
}

/// Host-owned session scope. Task records retain their original run provenance.
#[derive(Clone)]
pub(crate) struct TaskLineage {
    pub primary: RunId,
    pub origins: HashSet<RunId>,
}

impl TaskLineage {
    pub(crate) fn contains(&self, run: RunId) -> bool {
        run == self.primary || self.origins.contains(&run)
    }
}

fn run_matches(lineage: Option<&TaskLineage>, scope: RunId, origin: RunId) -> bool {
    scope == origin
        || lineage.is_some_and(|lineage| scope == lineage.primary && lineage.contains(origin))
}

struct PendingOperation {
    operations: Arc<RwLock<HashSet<uuid::Uuid>>>,
    id: uuid::Uuid,
}

impl Drop for PendingOperation {
    fn drop(&mut self) {
        if let Ok(mut operations) = self.operations.write() {
            operations.remove(&self.id);
        }
    }
}

/// Thread-safe task projection with an optional authoritative journal.
#[derive(Clone)]
pub struct TaskRegistry {
    pending_operations: Arc<RwLock<HashSet<uuid::Uuid>>>,
    receipts: Arc<RwLock<HashMap<uuid::Uuid, TaskOperationReceipt>>>,
    tasks: Arc<RwLock<HashMap<TaskId, TaskRecord>>>,
    bus: Option<RuntimeBus>,
    seq: Arc<AtomicU64>,
    clock: Arc<dyn Fn() -> i64 + Send + Sync>,
    store: Option<Arc<dyn TaskCommitSink>>,
    lineage: Option<Arc<TaskLineage>>,
}

impl Default for TaskRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl TaskRegistry {
    pub(crate) fn is_durable(&self) -> bool {
        self.store.is_some()
    }

    /// Check a trusted caller's run against the session's persisted scope.
    pub fn matches_run(&self, scope: RunId, origin: RunId) -> bool {
        run_matches(self.lineage.as_deref(), scope, origin)
    }

    /// Create an ephemeral registry without crash persistence.
    pub fn new() -> Self {
        Self {
            pending_operations: Arc::new(RwLock::new(HashSet::new())),
            receipts: Arc::new(RwLock::new(HashMap::new())),
            tasks: Arc::new(RwLock::new(HashMap::new())),
            bus: None,
            seq: Arc::new(AtomicU64::new(0)),
            clock: Arc::new(now_ms),
            store: None,
            lineage: None,
        }
    }

    /// Create an ephemeral registry with runtime observers.
    pub fn with_bus(bus: RuntimeBus) -> Self {
        Self {
            pending_operations: Arc::new(RwLock::new(HashSet::new())),
            receipts: Arc::new(RwLock::new(HashMap::new())),
            tasks: Arc::new(RwLock::new(HashMap::new())),
            bus: Some(bus),
            seq: Arc::new(AtomicU64::new(0)),
            clock: Arc::new(now_ms),
            store: None,
            lineage: None,
        }
    }

    /// Configure the host clock before sharing the registry with workers.
    /// Clock callbacks are evaluated outside registry locks.
    pub fn with_clock(mut self, clock: impl Fn() -> i64 + Send + Sync + 'static) -> Self {
        self.clock = Arc::new(clock);
        self
    }

    /// Open the authoritative journal for an explicitly selected run lineage.
    /// Keep this registry (or a clone) alive to retain the OS writer lease.
    pub fn open_durable(
        path: impl AsRef<std::path::Path>,
        run_id: RunId,
    ) -> Result<Self, TaskError> {
        let (journal, tasks) = TaskJournal::open(path.as_ref(), run_id)?;
        Ok(Self::from_journal(journal, tasks))
    }

    /// Select a persisted run under the journal's exclusive writer lease.
    /// The host supplies a stable key binding the canonical session source and
    /// header identity. Legacy journals require explicit migration first.
    pub fn open_session_durable(
        path: impl AsRef<std::path::Path>,
        session_key: &str,
    ) -> Result<(Self, RunId), TaskError> {
        let (journal, tasks) = TaskJournal::open_session(path.as_ref(), session_key)?;
        let run_id = journal.run_id;
        Ok((Self::from_journal(journal, tasks), run_id))
    }

    /// Import a host-validated legacy projection when initializing a session journal.
    /// Existing journals are authoritative and never reimport the supplied records.
    /// The snapshot shares the journal's bounded, checksummed header and writer lease.
    pub fn open_session_durable_with_legacy(
        path: impl AsRef<std::path::Path>,
        session_key: &str,
        records: Vec<TaskRecord>,
    ) -> Result<(Self, RunId), TaskError> {
        let (journal, tasks) =
            TaskJournal::open_session_with_legacy(path.as_ref(), session_key, records)?;
        let run_id = journal.run_id;
        Ok((Self::from_journal(journal, tasks), run_id))
    }

    fn from_journal(mut journal: TaskJournal, tasks: HashMap<TaskId, TaskRecord>) -> Self {
        Self {
            receipts: Arc::new(RwLock::new(std::mem::take(
                &mut journal.restored_operations,
            ))),
            tasks: Arc::new(RwLock::new(tasks)),
            lineage: Some(Arc::new(journal.lineage.clone())),
            store: Some(Arc::new(journal)),
            ..Self::new()
        }
    }

    /// Return the durable domain receipt for an operation without reapplying
    /// its task transition.  Recovery adapters use this after a crash between
    /// the task journal commit and the outer operation response.
    pub fn operation_receipt(
        &self,
        operation_id: uuid::Uuid,
    ) -> Result<Option<TaskOperationReceipt>, TaskError> {
        self.receipts
            .read()
            .map_err(|_| TaskError::RegistryPoisoned)
            .map(|receipts| receipts.get(&operation_id).cloned())
    }

    /// Validate restored operation receipts before a session accepts new
    /// commands.  A task projection without its exact command identity cannot
    /// safely participate in idempotent recovery.
    pub fn validate_operation_receipts(&self) -> Result<(), TaskError> {
        let receipts = self
            .receipts
            .read()
            .map_err(|_| TaskError::RegistryPoisoned)?;
        for (operation_id, receipt) in receipts.iter() {
            if *operation_id != receipt.operation_id {
                return Err(TaskError::Persistence(
                    "task operation receipt identity mismatch".into(),
                ));
            }
            receipt.response.validate()?;
        }
        Ok(())
    }

    /// Attach observers before sharing the durable registry with workers.
    pub fn with_observers(mut self, bus: RuntimeBus) -> Self {
        self.bus = Some(bus);
        self
    }

    fn persist_candidate(
        &self,
        previous: &HashMap<TaskId, TaskRecord>,
        candidate: &HashMap<TaskId, TaskRecord>,
    ) -> Result<(), TaskError> {
        self.persist_operation(previous, candidate, None)
    }

    fn persist_operation(
        &self,
        previous: &HashMap<TaskId, TaskRecord>,
        candidate: &HashMap<TaskId, TaskRecord>,
        claim: Option<TaskOperationReceipt>,
    ) -> Result<(), TaskError> {
        let mut changes = Vec::new();
        for (id, record) in candidate {
            if previous.get(id) != Some(record) {
                record.validate()?;
                changes.push(TaskChange {
                    prior_revision: previous.get(id).map(|task| task.revision),
                    record: record.clone(),
                });
            }
        }
        changes.sort_by_key(|change| change.record.id);
        if let Some(store) = &self.store {
            if let Some(receipt) = claim {
                store.commit_operation(changes, receipt)?;
            } else {
                store.commit(changes)?;
            }
        }
        Ok(())
    }

    /// Create and register a task with dependency validation and initial state derivation.
    pub fn create_task(&self, task: TaskRecord) -> Result<TaskId, TaskError> {
        self.create_task_inner(task, None).map(|record| record.id)
    }

    /// Create from host-authenticated identity, replaying the original generated fields.
    pub fn create_command(
        &self,
        input: TaskCreateRequest,
        run_id: RunId,
        actor: AgentId,
        operation_id: Option<uuid::Uuid>,
    ) -> Result<TaskRecord, TaskError> {
        let mut task = TaskRecord::new(run_id, input.title.clone());
        if input.assigned_to.is_some_and(|owner| owner != actor) {
            return Err(TaskError::NotOwner(task.id));
        }
        task.description = input.description.clone();
        task.dependencies = input.dependencies.clone();
        task.assigned_to = input.assigned_to;
        task.owner_generation = u64::from(input.assigned_to.is_some());
        task.parent_plan_step = input.parent_plan_step.clone();
        task.decision_prerequisites = input.decision_prerequisites.clone();
        task.contract_digest = input.contract_digest.clone();
        let operation = operation_id.map(|id| {
            (
                id,
                TaskOperationRequest {
                    task_id: None,
                    run_id,
                    actor,
                    expected_revision: 0,
                    status: None,
                    create: Some(input),
                },
            )
        });
        self.create_task_inner(task, operation)
    }

    fn create_task_inner(
        &self,
        mut task: TaskRecord,
        operation: Option<(uuid::Uuid, TaskOperationRequest)>,
    ) -> Result<TaskRecord, TaskError> {
        task.validate()?;
        // A task that depends on a structured user decision is not claimable
        // until the host evaluates that decision against the current plan and
        // evidence. This prevents a direct status update from treating an
        // unanswered prerequisite as consent.
        if !task.decision_prerequisites.is_empty() && task.blocked_reasons.is_empty() {
            task.blocked_reasons.push(BlockReason {
                code: "decision_prerequisites_unverified".into(),
                message: "Decision prerequisites require host evaluation before execution".into(),
            });
        }
        let committed_at = (self.clock)();
        let task_id = task.id;

        // 1. Validate self-dependency
        if task.dependencies.contains(&task_id) {
            return Err(TaskError::SelfDependency(task_id));
        }

        let assigned_agent = task.assigned_to;
        let run_id = task.run_id;

        {
            let mut tasks = self
                .tasks
                .write()
                .map_err(|_| TaskError::InvalidTransition {
                    task_id,
                    from: task.state,
                    to: task.state,
                })?;

            if let Some(response) = self.replay_operation(&operation)? {
                return Ok(response);
            }
            if let Some((id, _)) = &operation {
                if self
                    .pending_operations
                    .read()
                    .map_err(|_| TaskError::RegistryPoisoned)?
                    .contains(id)
                {
                    return Err(TaskError::OperationInProgress);
                }
            }
            if tasks.contains_key(&task_id) {
                return Err(TaskError::DuplicateTask(task_id));
            }

            task.state = initial_task_state(&task, &tasks, self.lineage.as_deref())?;
            task.advance_revision(committed_at)?;
            let mut candidate = tasks.clone();
            candidate.insert(task_id, task.clone());
            let receipt = operation.map(|(operation_id, request)| TaskOperationReceipt {
                operation_id,
                request,
                response: task.clone(),
            });
            let mut receipts = self
                .receipts
                .write()
                .map_err(|_| TaskError::RegistryPoisoned)?;
            self.persist_operation(&tasks, &candidate, receipt.clone())?;
            *tasks = candidate;
            if let Some(receipt) = receipt {
                receipts.insert(receipt.operation_id, receipt);
            }
        }

        // 4. Emit observe events
        if let Some(bus) = &self.bus {
            let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
            let envelope = RuntimeEventEnvelope::new(
                seq,
                run_id,
                None,
                assigned_agent,
                None,
                RuntimeEvent::TaskCreated {
                    task_id,
                    record: Some(task.clone()),
                },
            );
            bus.emit_observe(envelope);

            if let Some(agent_id) = assigned_agent {
                let seq_assign = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
                let envelope_assign = RuntimeEventEnvelope::new(
                    seq_assign,
                    run_id,
                    None,
                    Some(agent_id),
                    None,
                    RuntimeEvent::TaskAssigned { task_id, agent_id },
                );
                bus.emit_observe(envelope_assign);
            }
        }

        Ok(task)
    }

    /// Evaluate decision prerequisites against a living plan and update task blocked reasons and state.
    pub fn evaluate_and_update_decision_prerequisites(
        &self,
        task_id: TaskId,
        plan: &crate::LivingPlan,
        cwd: &std::path::Path,
    ) -> Result<TaskRecord, TaskError> {
        let committed_at = (self.clock)();
        let mut tasks = self
            .tasks
            .write()
            .map_err(|_| TaskError::RegistryPoisoned)?;
        let mut task = tasks
            .get(&task_id)
            .cloned()
            .ok_or(TaskError::TaskNotFound(task_id))?;
        if matches!(task.state, TaskState::Completed | TaskState::Cancelled) {
            return Ok(task);
        }
        let reasons = evaluate_decision_prerequisites(&task.decision_prerequisites, plan, cwd);
        task.blocked_reasons = reasons;
        if !task.blocked_reasons.is_empty() {
            task.state = TaskState::Blocked;
        } else if task.state == TaskState::Blocked {
            task.state = initial_task_state(&task, &tasks, self.lineage.as_deref())?;
        }
        task.advance_revision(committed_at)?;
        let mut candidate = tasks.clone();
        candidate.insert(task_id, task.clone());
        self.persist_candidate(&tasks, &candidate)?;
        *tasks = candidate;
        Ok(task)
    }

    /// Atomically claim an unowned ready task using host-provided identity.
    pub(crate) fn claim_task(
        &self,
        task_id: TaskId,
        run_id: RunId,
        agent_id: AgentId,
        expected_revision: u64,
    ) -> Result<(), TaskError> {
        self.claim_inner(task_id, run_id, agent_id, expected_revision, None)
            .map(|_| ())
    }

    pub fn claim_operation(
        &self,
        task_id: TaskId,
        run_id: RunId,
        agent_id: AgentId,
        expected_revision: u64,
        operation_id: uuid::Uuid,
    ) -> Result<TaskRecord, TaskError> {
        self.claim_inner(
            task_id,
            run_id,
            agent_id,
            expected_revision,
            Some(operation_id),
        )
    }

    fn claim_inner(
        &self,
        task_id: TaskId,
        run_id: RunId,
        agent_id: AgentId,
        expected_revision: u64,
        operation_id: Option<uuid::Uuid>,
    ) -> Result<TaskRecord, TaskError> {
        let committed_at = (self.clock)();
        let response = {
            let mut stored = self
                .tasks
                .write()
                .map_err(|_| TaskError::RegistryPoisoned)?;
            let mut receipts = self
                .receipts
                .write()
                .map_err(|_| TaskError::RegistryPoisoned)?;
            let request = TaskOperationRequest {
                status: None,
                create: None,
                task_id: Some(task_id),
                run_id,
                actor: agent_id,
                expected_revision,
            };
            if let Some(operation_id) = operation_id {
                if self
                    .pending_operations
                    .read()
                    .map_err(|_| TaskError::RegistryPoisoned)?
                    .contains(&operation_id)
                {
                    return Err(TaskError::OperationInProgress);
                }
                if let Some(receipt) = receipts.get(&operation_id) {
                    return if receipt.request == request {
                        Ok(receipt.response.clone())
                    } else {
                        Err(TaskError::OperationConflict)
                    };
                }
                if receipts.len() >= MAX_OPERATION_RECEIPTS {
                    return Err(TaskError::Persistence(
                        "claim receipt capacity exceeded".into(),
                    ));
                }
            }
            let mut candidate = stored.clone();
            let task = candidate
                .get_mut(&task_id)
                .ok_or(TaskError::TaskNotFound(task_id))?;
            if !self.matches_run(run_id, task.run_id) {
                return Err(TaskError::RunMismatch(task_id));
            }
            if task.revision != expected_revision || task.assigned_to.is_some() {
                return Err(TaskError::RevisionConflict(task_id));
            }
            if task.state != TaskState::Ready {
                return Err(TaskError::InvalidTransition {
                    task_id,
                    from: task.state,
                    to: TaskState::Running,
                });
            }
            task.owner_generation = task
                .owner_generation
                .checked_add(1)
                .ok_or(TaskError::RevisionOverflow(task_id))?;
            task.advance_revision(committed_at)?;
            task.assigned_to = Some(agent_id);
            task.state = TaskState::Running;
            let response = task.clone();
            let receipt = operation_id.map(|operation_id| TaskOperationReceipt {
                operation_id,
                request,
                response: response.clone(),
            });
            self.persist_operation(&stored, &candidate, receipt.clone())?;
            *stored = candidate;
            if let Some(receipt) = receipt {
                receipts.insert(receipt.operation_id, receipt);
            }
            response
        };
        if let Some(bus) = &self.bus {
            let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
            bus.emit_observe(RuntimeEventEnvelope::new(
                seq,
                run_id,
                None,
                Some(agent_id),
                None,
                RuntimeEvent::TaskAssigned { task_id, agent_id },
            ));
        }
        Ok(response)
    }

    /// Atomically retry a task with a new attempt and advanced generation.
    pub fn retry_task(
        &self,
        task_id: TaskId,
        run_id: RunId,
        expected_revision: u64,
        new_agent_id: Option<AgentId>,
    ) -> Result<TaskRecord, TaskError> {
        let committed_at = (self.clock)();
        let response = {
            let mut stored = self
                .tasks
                .write()
                .map_err(|_| TaskError::RegistryPoisoned)?;
            let mut candidate = stored.clone();
            let task = candidate
                .get_mut(&task_id)
                .ok_or(TaskError::TaskNotFound(task_id))?;
            if !self.matches_run(run_id, task.run_id) {
                return Err(TaskError::RunMismatch(task_id));
            }
            if task.revision != expected_revision {
                return Err(TaskError::RevisionConflict(task_id));
            }
            task.attempt = task.attempt.saturating_add(1);
            task.owner_generation = task
                .owner_generation
                .checked_add(1)
                .ok_or(TaskError::RevisionOverflow(task_id))?;
            task.advance_revision(committed_at)?;
            task.assigned_to = new_agent_id;
            task.state = if new_agent_id.is_some() {
                TaskState::Running
            } else {
                TaskState::Ready
            };
            task.result = None;
            task.evidence_refs.clear();
            let response = task.clone();
            self.persist_operation(&stored, &candidate, None)?;
            *stored = candidate;
            response
        };
        if let Some(bus) = &self.bus {
            if let Some(agent_id) = new_agent_id {
                let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
                bus.emit_observe(RuntimeEventEnvelope::new(
                    seq,
                    run_id,
                    None,
                    Some(agent_id),
                    None,
                    RuntimeEvent::TaskAssigned { task_id, agent_id },
                ));
            }
        }
        Ok(response)
    }

    /// Assign a task to an agent.
    pub fn assign_task(&self, task_id: TaskId, agent_id: AgentId) -> Result<(), TaskError> {
        let committed_at = (self.clock)();
        let (run_id, previous_agent) = {
            let mut stored = self
                .tasks
                .write()
                .map_err(|_| TaskError::InvalidTransition {
                    task_id,
                    from: TaskState::Ready,
                    to: TaskState::Running,
                })?;

            let mut tasks = stored.clone();
            let task = tasks
                .get_mut(&task_id)
                .ok_or(TaskError::TaskNotFound(task_id))?;

            if task.state.is_terminal() {
                return Err(TaskError::TerminalTask {
                    task_id,
                    state: task.state,
                });
            }

            let prev = task.assigned_to;
            task.advance_revision(committed_at)?;
            task.assigned_to = Some(agent_id);
            if task.state == TaskState::Ready {
                task.state = TaskState::Running;
            }
            let run_id = task.run_id;
            self.persist_candidate(&stored, &tasks)?;
            *stored = tasks;
            (run_id, prev)
        };

        if previous_agent != Some(agent_id) {
            if let Some(bus) = &self.bus {
                let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
                let envelope = RuntimeEventEnvelope::new(
                    seq,
                    run_id,
                    None,
                    Some(agent_id),
                    None,
                    RuntimeEvent::TaskAssigned { task_id, agent_id },
                );
                bus.emit_observe(envelope);
            }
        }

        Ok(())
    }

    /// Attach an evidence reference to a task.
    pub fn attach_evidence(
        &self,
        task_id: TaskId,
        evidence_id: EvidenceId,
    ) -> Result<(), TaskError> {
        let committed_at = (self.clock)();
        let mut stored = self
            .tasks
            .write()
            .map_err(|_| TaskError::TaskNotFound(task_id))?;
        let mut tasks = stored.clone();
        let task = tasks
            .get_mut(&task_id)
            .ok_or(TaskError::TaskNotFound(task_id))?;
        task.attach_evidence(evidence_id)?;
        task.advance_revision(committed_at)?;
        self.persist_candidate(&stored, &tasks)?;
        *stored = tasks;
        Ok(())
    }

    /// Complete a task, passing through the runtime decision gate.
    pub fn complete_task(&self, task_id: TaskId, result: Option<String>) -> Result<(), TaskError> {
        self.complete_task_inner(task_id, result, None, None)
            .map(|_| ())
    }

    pub(crate) fn complete_owned_task(
        &self,
        task_id: TaskId,
        result: Option<String>,
        owner: TaskOwner,
    ) -> Result<(), TaskError> {
        self.complete_task_inner(task_id, result, Some(owner), None)
            .map(|_| ())
    }

    /// Complete a task backed by verified evaluation without specific owner validation.
    pub fn complete_verified_task(
        &self,
        task_id: TaskId,
        evaluation: &CompletionEvaluation,
        current_manifest: &SourceManifest,
        result: Option<String>,
    ) -> Result<TaskRecord, TaskError> {
        self.complete_task_with_evaluation(task_id, evaluation, current_manifest, result, None)
    }

    /// Complete a task backed by verified evaluation, re-checking revisions and source state.
    pub(crate) fn complete_task_with_evaluation(
        &self,
        task_id: TaskId,
        evaluation: &CompletionEvaluation,
        current_manifest: &SourceManifest,
        result: Option<String>,
        owner: Option<TaskOwner>,
    ) -> Result<TaskRecord, TaskError> {
        if evaluation.task_id != task_id {
            return Err(TaskError::CompletionRefused(
                "completion evaluation targets a different task".into(),
            ));
        }
        if !evaluation.allowed {
            return Err(TaskError::CompletionRefused(format!(
                "completion evaluation failed with remaining gaps: {:?}",
                evaluation.remaining_gaps
            )));
        }

        // Compare-and-swap check: source must not have changed after evaluation
        if evaluation.source_fingerprint != current_manifest.digest {
            return Err(TaskError::CompletionRefused(
                "source modified after evaluation".into(),
            ));
        }

        let committed_at = (self.clock)();
        let mut stored = self
            .tasks
            .write()
            .map_err(|_| TaskError::InvalidTransition {
                task_id,
                from: TaskState::Running,
                to: TaskState::Completed,
            })?;
        let mut tasks = stored.clone();
        let response = {
            let task = tasks
                .get_mut(&task_id)
                .ok_or(TaskError::TaskNotFound(task_id))?;

            if let Some(owner) = owner {
                owner.validate(task, self.lineage.as_deref())?;
            }
            if task.revision != evaluation.evaluated_revision
                || task.attempt != evaluation.evaluated_attempt
            {
                return Err(TaskError::CompletionRefused(
                    "task changed after completion evaluation".into(),
                ));
            }
            if task.contract_digest != evaluation.contract_digest {
                return Err(TaskError::CompletionRefused(
                    "task contract changed after completion evaluation".into(),
                ));
            }

            if task.state.is_terminal() {
                return Err(TaskError::TerminalTask {
                    task_id,
                    state: task.state,
                });
            }

            if !task.decision_prerequisites.is_empty() && !task.blocked_reasons.is_empty() {
                return Err(TaskError::CompletionRefused(
                    "decision prerequisites are unresolved".into(),
                ));
            }

            if !is_valid_task_transition(task.state, TaskState::Completed) {
                return Err(TaskError::InvalidTransition {
                    task_id,
                    from: task.state,
                    to: TaskState::Completed,
                });
            }

            task.advance_revision(committed_at)?;
            task.state = TaskState::Completed;
            task.result = result;

            for dim in &evaluation.dimensions {
                if let Some(ev_id) = dim.evidence_id {
                    let _ = task.attach_evidence(ev_id);
                }
            }
            task.clone()
        };

        let all_keys: Vec<TaskId> = tasks.keys().cloned().collect();
        for dep_key in all_keys {
            if let Some(dependent) = tasks.get(&dep_key) {
                if dependent.state == TaskState::Pending
                    && dependent.dependencies.contains(&task_id)
                {
                    let all_deps_completed = dependent.dependencies.iter().all(|dep_id| {
                        tasks
                            .get(dep_id)
                            .is_some_and(|d| d.state == TaskState::Completed)
                    });
                    if all_deps_completed {
                        if let Some(dep_mut) = tasks.get_mut(&dep_key) {
                            dep_mut.state = TaskState::Ready;
                            let _ = dep_mut.advance_revision(committed_at);
                        }
                    }
                }
            }
        }

        self.persist_candidate(&stored, &tasks)?;
        *stored = tasks;
        Ok(response)
    }

    pub(crate) fn complete_operation(
        &self,
        task_id: TaskId,
        result: Option<String>,
        owner: TaskOwner,
        operation_id: uuid::Uuid,
    ) -> Result<TaskRecord, TaskError> {
        self.status_operation(task_id, TaskState::Completed, result, owner, operation_id)
    }

    pub fn status_operation(
        &self,
        task_id: TaskId,
        state: TaskState,
        result: Option<String>,
        owner: TaskOwner,
        operation_id: uuid::Uuid,
    ) -> Result<TaskRecord, TaskError> {
        if result.as_ref().is_some_and(|s| s.len() > 65_536)
            || (state == TaskState::Cancelled && result.is_some())
        {
            return Err(TaskError::CompletionRefused("Invalid status result".into()));
        }
        let request = TaskOperationRequest {
            create: None,
            task_id: Some(task_id),
            run_id: owner.run_id,
            actor: owner.agent_id,
            expected_revision: owner.revision,
            status: Some(super::task_store::StatusRequest {
                state,
                generation: owner.generation,
                result: result.clone(),
            }),
        };
        let operation = Some((operation_id, request));
        let _pending = {
            // Reserve across hooks without holding any lock while hooks execute.
            let _tasks = self.tasks.read().map_err(|_| TaskError::RegistryPoisoned)?;
            if let Some(response) = self.replay_operation(&operation)? {
                return Ok(response);
            }
            let mut pending = self
                .pending_operations
                .write()
                .map_err(|_| TaskError::RegistryPoisoned)?;
            if pending.len() >= MAX_OPERATION_RECEIPTS || !pending.insert(operation_id) {
                return Err(TaskError::OperationInProgress);
            }
            PendingOperation {
                operations: self.pending_operations.clone(),
                id: operation_id,
            }
        };
        match state {
            TaskState::Completed => {
                self.complete_task_inner(task_id, result, Some(owner), operation)
            }
            TaskState::Failed => self.fail_task_inner(task_id, result, Some(owner), operation),
            TaskState::Cancelled => self.cancel_task_inner(task_id, Some(owner), operation),
            _ => Err(TaskError::InvalidTransition {
                task_id,
                from: state,
                to: state,
            }),
        }
    }

    fn replay_operation(
        &self,
        operation: &Option<(uuid::Uuid, TaskOperationRequest)>,
    ) -> Result<Option<TaskRecord>, TaskError> {
        let Some((id, request)) = operation else {
            return Ok(None);
        };
        let receipts = self
            .receipts
            .read()
            .map_err(|_| TaskError::RegistryPoisoned)?;
        if let Some(receipt) = receipts.get(id) {
            return if &receipt.request == request {
                Ok(Some(receipt.response.clone()))
            } else {
                Err(TaskError::OperationConflict)
            };
        }
        if receipts.len() >= MAX_OPERATION_RECEIPTS {
            return Err(TaskError::Persistence(
                "operation receipt capacity exceeded".into(),
            ));
        }
        Ok(None)
    }

    fn complete_task_inner(
        &self,
        task_id: TaskId,
        result: Option<String>,
        owner: Option<TaskOwner>,
        operation: Option<(uuid::Uuid, TaskOperationRequest)>,
    ) -> Result<TaskRecord, TaskError> {
        let (run_id, assigned_to, expected_revision) = {
            let tasks = self
                .tasks
                .read()
                .map_err(|_| TaskError::InvalidTransition {
                    task_id,
                    from: TaskState::Running,
                    to: TaskState::Completed,
                })?;

            if let Some(response) = self.replay_operation(&operation)? {
                return Ok(response);
            }
            let task = tasks
                .get(&task_id)
                .ok_or(TaskError::TaskNotFound(task_id))?;

            if let Some(owner) = owner {
                owner.validate(task, self.lineage.as_deref())?;
            }

            if task.state.is_terminal() {
                return Err(TaskError::TerminalTask {
                    task_id,
                    state: task.state,
                });
            }

            if !task.decision_prerequisites.is_empty() && !task.blocked_reasons.is_empty() {
                return Err(TaskError::CompletionRefused(
                    "decision prerequisites are unresolved".into(),
                ));
            }

            if !is_valid_task_transition(task.state, TaskState::Completed) {
                return Err(TaskError::InvalidTransition {
                    task_id,
                    from: task.state,
                    to: TaskState::Completed,
                });
            }

            (task.run_id, task.assigned_to, task.revision)
        };

        // A proposal can be refused; it must never be replayed as completion.
        if let Some(bus) = &self.bus {
            let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
            let envelope = RuntimeEventEnvelope::new(
                seq,
                run_id,
                None,
                assigned_to,
                None,
                RuntimeEvent::TaskCompletionRequested {
                    task_id,
                    expected_revision,
                },
            );
            if let Err(reason) = bus.emit_decision(envelope) {
                return Err(TaskError::CompletionRefused(reason));
            }
        }

        // Commit completion and cascade to dependents
        let now = (self.clock)();
        let response = {
            let mut stored = self
                .tasks
                .write()
                .map_err(|_| TaskError::InvalidTransition {
                    task_id,
                    from: TaskState::Running,
                    to: TaskState::Completed,
                })?;

            if let Some(response) = self.replay_operation(&operation)? {
                return Ok(response);
            }
            let mut tasks = stored.clone();

            let task = tasks
                .get_mut(&task_id)
                .ok_or(TaskError::TaskNotFound(task_id))?;

            if task.revision != expected_revision {
                return Err(TaskError::RevisionConflict(task_id));
            }
            if let Some(owner) = owner {
                owner.validate(task, self.lineage.as_deref())?;
            }
            if !task.decision_prerequisites.is_empty() && !task.blocked_reasons.is_empty() {
                return Err(TaskError::CompletionRefused(
                    "decision prerequisites are unresolved".into(),
                ));
            }
            task.advance_revision(now)?;
            task.state = TaskState::Completed;
            task.result = result;

            // Cascade: any Pending dependent whose dependencies are ALL completed transitions to Ready
            let all_keys: Vec<TaskId> = tasks.keys().cloned().collect();
            for dep_key in all_keys {
                if let Some(dependent) = tasks.get(&dep_key) {
                    if dependent.state == TaskState::Pending
                        && dependent.dependencies.contains(&task_id)
                    {
                        let all_deps_done = dependent.dependencies.iter().all(|d| {
                            tasks
                                .get(d)
                                .map(|t| t.state == TaskState::Completed)
                                .unwrap_or(false)
                        });
                        if all_deps_done {
                            if let Some(dep_mut) = tasks.get_mut(&dep_key) {
                                dep_mut.advance_revision(now)?;
                                dep_mut.state = TaskState::Ready;
                            }
                        }
                    }
                }
            }
            let response = tasks[&task_id].clone();
            let receipt = operation.map(|(operation_id, request)| TaskOperationReceipt {
                operation_id,
                request,
                response: response.clone(),
            });
            // The task write lane serializes receipt insertion with all commands.
            let mut receipts = self
                .receipts
                .write()
                .map_err(|_| TaskError::RegistryPoisoned)?;
            self.persist_operation(&stored, &tasks, receipt.clone())?;
            *stored = tasks;
            if let Some(receipt) = receipt {
                receipts.insert(receipt.operation_id, receipt);
            }
            response
        };

        if let Some(bus) = &self.bus {
            let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
            bus.emit_observe(RuntimeEventEnvelope::new(
                seq,
                run_id,
                None,
                assigned_to,
                None,
                RuntimeEvent::TaskCompleted {
                    task_id,
                    success: true,
                },
            ));
        }
        Ok(response)
    }

    /// Fail a task and cascade Blocked state to all dependents.
    pub fn fail_task(
        &self,
        task_id: TaskId,
        error_reason: Option<String>,
    ) -> Result<(), TaskError> {
        self.fail_task_inner(task_id, error_reason, None, None)
            .map(|_| ())
    }

    pub(crate) fn fail_owned_task(
        &self,
        task_id: TaskId,
        reason: Option<String>,
        owner: TaskOwner,
    ) -> Result<(), TaskError> {
        self.fail_task_inner(task_id, reason, Some(owner), None)
            .map(|_| ())
    }

    fn fail_task_inner(
        &self,
        task_id: TaskId,
        error_reason: Option<String>,
        owner: Option<TaskOwner>,
        operation: Option<(uuid::Uuid, TaskOperationRequest)>,
    ) -> Result<TaskRecord, TaskError> {
        let now = (self.clock)();
        let (run_id, assigned_to, response) = {
            let mut stored = self
                .tasks
                .write()
                .map_err(|_| TaskError::InvalidTransition {
                    task_id,
                    from: TaskState::Running,
                    to: TaskState::Failed,
                })?;

            if let Some(response) = self.replay_operation(&operation)? {
                return Ok(response);
            }
            let mut tasks = stored.clone();

            let task = tasks
                .get_mut(&task_id)
                .ok_or(TaskError::TaskNotFound(task_id))?;

            if let Some(owner) = owner {
                owner.validate(task, self.lineage.as_deref())?;
            }
            if task.state.is_terminal() {
                return Err(TaskError::TerminalTask {
                    task_id,
                    state: task.state,
                });
            }

            task.advance_revision(now)?;
            task.state = TaskState::Failed;
            task.result = error_reason;
            let run_id = task.run_id;
            let assigned_to = task.assigned_to;

            // Cascade: direct and indirect dependents move to Blocked
            let mut to_block = vec![task_id];
            while let Some(failed_id) = to_block.pop() {
                for other_task in tasks.values_mut() {
                    if other_task.dependencies.contains(&failed_id)
                        && !other_task.state.is_terminal()
                        && other_task.state != TaskState::Blocked
                    {
                        other_task.advance_revision(now)?;
                        other_task.state = TaskState::Blocked;
                        to_block.push(other_task.id);
                    }
                }
            }

            let response = tasks[&task_id].clone();
            let receipt = operation.map(|(operation_id, request)| TaskOperationReceipt {
                operation_id,
                request,
                response: response.clone(),
            });
            let mut receipts = self
                .receipts
                .write()
                .map_err(|_| TaskError::RegistryPoisoned)?;
            self.persist_operation(&stored, &tasks, receipt.clone())?;
            *stored = tasks;
            if let Some(receipt) = receipt {
                receipts.insert(receipt.operation_id, receipt);
            }
            (run_id, assigned_to, response)
        };

        if let Some(bus) = &self.bus {
            let seq = self.seq.fetch_add(1, Ordering::SeqCst) + 1;
            let envelope = RuntimeEventEnvelope::new(
                seq,
                run_id,
                None,
                assigned_to,
                None,
                RuntimeEvent::TaskCompleted {
                    task_id,
                    success: false,
                },
            );
            bus.emit_observe(envelope);
        }

        Ok(response)
    }

    /// Cancel a task and cascade Blocked state to its dependents.
    pub fn cancel_task(&self, task_id: TaskId) -> Result<(), TaskError> {
        self.cancel_task_inner(task_id, None, None).map(|_| ())
    }

    pub(crate) fn cancel_owned_task(
        &self,
        task_id: TaskId,
        owner: TaskOwner,
    ) -> Result<(), TaskError> {
        self.cancel_task_inner(task_id, Some(owner), None)
            .map(|_| ())
    }

    fn cancel_task_inner(
        &self,
        task_id: TaskId,
        owner: Option<TaskOwner>,
        operation: Option<(uuid::Uuid, TaskOperationRequest)>,
    ) -> Result<TaskRecord, TaskError> {
        let now = (self.clock)();
        let mut stored = self
            .tasks
            .write()
            .map_err(|_| TaskError::InvalidTransition {
                task_id,
                from: TaskState::Ready,
                to: TaskState::Cancelled,
            })?;

        if let Some(response) = self.replay_operation(&operation)? {
            return Ok(response);
        }
        let mut tasks = stored.clone();

        let task = tasks
            .get_mut(&task_id)
            .ok_or(TaskError::TaskNotFound(task_id))?;

        if let Some(owner) = owner {
            owner.validate(task, self.lineage.as_deref())?;
        }
        if task.state.is_terminal() {
            return Err(TaskError::TerminalTask {
                task_id,
                state: task.state,
            });
        }

        task.advance_revision(now)?;
        task.state = TaskState::Cancelled;
        if operation.is_some() {
            task.result = None;
        }

        // Cascade Blocked state to dependents
        let mut to_block = vec![task_id];
        while let Some(cancelled_id) = to_block.pop() {
            for other_task in tasks.values_mut() {
                if other_task.dependencies.contains(&cancelled_id)
                    && !other_task.state.is_terminal()
                    && other_task.state != TaskState::Blocked
                {
                    other_task.advance_revision(now)?;
                    other_task.state = TaskState::Blocked;
                    to_block.push(other_task.id);
                }
            }
        }

        let response = tasks[&task_id].clone();
        let receipt = operation.map(|(operation_id, request)| TaskOperationReceipt {
            operation_id,
            request,
            response: response.clone(),
        });
        let mut receipts = self
            .receipts
            .write()
            .map_err(|_| TaskError::RegistryPoisoned)?;
        self.persist_operation(&stored, &tasks, receipt.clone())?;
        *stored = tasks;
        if let Some(receipt) = receipt {
            receipts.insert(receipt.operation_id, receipt);
        }
        Ok(response)
    }

    /// Atomically revise the contract digest for a host-owned task.
    ///
    /// Scope expansion calls this only after the user-authorized contract has been
    /// durably recorded. The task projection is committed before the caller exposes
    /// the new live contract, so stale revisions/owners/storage failures stay fail-closed.
    #[allow(clippy::too_many_arguments)]
    pub fn revise_contract_digest(
        &self,
        task_id: TaskId,
        run_id: RunId,
        agent_id: AgentId,
        expected_revision: u64,
        expected_generation: u64,
        expected_digest: &str,
        new_digest: &str,
    ) -> Result<TaskRecord, TaskError> {
        let committed_at = (self.clock)();
        let mut stored = self
            .tasks
            .write()
            .map_err(|_| TaskError::RegistryPoisoned)?;
        let mut candidate = stored.clone();
        let task = candidate
            .get_mut(&task_id)
            .ok_or(TaskError::TaskNotFound(task_id))?;
        if !self.matches_run(run_id, task.run_id) {
            return Err(TaskError::RunMismatch(task_id));
        }
        if task.assigned_to != Some(agent_id) {
            return Err(TaskError::NotOwner(task_id));
        }
        if task.revision != expected_revision
            || task.owner_generation != expected_generation
            || task.contract_digest.as_deref() != Some(expected_digest)
        {
            return Err(TaskError::RevisionConflict(task_id));
        }
        if task.state.is_terminal() {
            return Err(TaskError::TerminalTask {
                task_id,
                state: task.state,
            });
        }
        task.contract_digest = Some(new_digest.to_string());
        task.advance_revision(committed_at)?;
        task.validate()?;
        let response = task.clone();
        self.persist_candidate(&stored, &candidate)?;
        *stored = candidate;
        Ok(response)
    }

    /// Fetch a task record by ID.
    pub fn get_task(&self, task_id: &TaskId) -> Option<TaskRecord> {
        self.tasks.read().ok()?.get(task_id).cloned()
    }

    /// List all tasks, optionally filtered by run ID.
    pub fn list_tasks(&self, run_id: Option<RunId>) -> Vec<TaskRecord> {
        let Ok(tasks) = self.tasks.read() else {
            return Vec::new();
        };
        tasks
            .values()
            .filter(|t| {
                run_id
                    .map(|r| self.matches_run(r, t.run_id))
                    .unwrap_or(true)
            })
            .cloned()
            .collect()
    }

    /// List ready tasks (unblocked and eligible to be picked up).
    pub fn get_ready_tasks(&self, run_id: Option<RunId>) -> Vec<TaskRecord> {
        let Ok(tasks) = self.tasks.read() else {
            return Vec::new();
        };
        tasks
            .values()
            .filter(|t| {
                t.state == TaskState::Ready
                    && run_id
                        .map(|r| self.matches_run(r, t.run_id))
                        .unwrap_or(true)
            })
            .cloned()
            .collect()
    }

    /// List tasks assigned to a specific agent.
    pub fn get_agent_tasks(&self, agent_id: AgentId) -> Vec<TaskRecord> {
        let Ok(tasks) = self.tasks.read() else {
            return Vec::new();
        };
        tasks
            .values()
            .filter(|t| t.assigned_to == Some(agent_id))
            .cloned()
            .collect()
    }

    /// Rewinds task state to a previous attempt/checkpoint.
    /// Appends a new attempt without rewriting terminal state history in place.
    /// Marks all downstream dependent tasks and evidence as stale/blocked.
    pub fn rewind_task_state(
        &self,
        task_id: TaskId,
        checkpoint_id: &str,
    ) -> Result<(TaskRecord, Vec<TaskId>, Vec<EvidenceId>), TaskError> {
        let committed_at = (self.clock)();
        let mut stored = self
            .tasks
            .write()
            .map_err(|_| TaskError::RegistryPoisoned)?;

        let mut next = stored.clone();
        if !next.contains_key(&task_id) {
            return Err(TaskError::TaskNotFound(task_id));
        }

        let downstream_ids = collect_downstream_dependents(&next, task_id);
        let mut stale_tasks = Vec::new();
        let mut stale_evidence = Vec::new();

        for dep_id in downstream_ids {
            if let Some(dep_task) = next.get_mut(&dep_id) {
                stale_tasks.push(dep_id);
                stale_evidence.extend(dep_task.evidence_refs.iter().copied());
                dep_task.state = TaskState::Blocked;
                dep_task
                    .blocked_reasons
                    .retain(|r| r.code != "dependency_rewound");
                dep_task.blocked_reasons.push(BlockReason {
                    code: "dependency_rewound".into(),
                    message: format!(
                        "Upstream task {} was rewound to checkpoint {}",
                        task_id, checkpoint_id
                    ),
                });
                dep_task.advance_revision(committed_at)?;
            }
        }

        let task = next.get_mut(&task_id).unwrap();
        stale_evidence.extend(task.evidence_refs.iter().copied());
        task.attempt += 1;
        task.result = None;
        task.assigned_to = None;
        task.blocked_reasons.retain(|r| r.code != "rewound");

        let task_clone = task.clone();
        let ready_state = initial_task_state(&task_clone, &next, self.lineage.as_deref())?;

        let task = next.get_mut(&task_id).unwrap();
        task.state = ready_state;
        task.advance_revision(committed_at)?;

        let updated_record = task.clone();
        *stored = next;
        Ok((updated_record, stale_tasks, stale_evidence))
    }

    /// Marks affected evidence and downstream tasks stale following a code-only rewind,
    /// even when task state was not selected for rewind.
    pub fn invalidate_for_code_rewind(
        &self,
        task_id: TaskId,
        checkpoint_id: &str,
    ) -> Result<(Vec<TaskId>, Vec<EvidenceId>), TaskError> {
        let committed_at = (self.clock)();
        let mut stored = self
            .tasks
            .write()
            .map_err(|_| TaskError::RegistryPoisoned)?;

        if !stored.contains_key(&task_id) {
            return Err(TaskError::TaskNotFound(task_id));
        }

        let mut affected_tasks = vec![task_id];
        affected_tasks.extend(collect_downstream_dependents(&stored, task_id));

        let mut stale_evidence = Vec::new();
        let mut stale_tasks = Vec::new();

        for tid in affected_tasks {
            if let Some(t) = stored.get_mut(&tid) {
                stale_evidence.extend(t.evidence_refs.iter().copied());
                if tid != task_id {
                    stale_tasks.push(tid);
                    if t.state == TaskState::Completed || t.state == TaskState::Running {
                        t.state = TaskState::Blocked;
                        t.blocked_reasons.push(BlockReason {
                            code: "code_rewound".into(),
                            message: format!("Code was rewound to checkpoint {}", checkpoint_id),
                        });
                        let _ = t.advance_revision(committed_at);
                    }
                } else {
                    t.blocked_reasons.push(BlockReason {
                        code: "code_rewound".into(),
                        message: format!("Code was rewound to checkpoint {}", checkpoint_id),
                    });
                    let _ = t.advance_revision(committed_at);
                }
            }
        }

        Ok((stale_tasks, stale_evidence))
    }

    /// Rehydrate task registry state from historical runtime event envelopes.
    ///
    /// Rules:
    /// - Tasks recorded via `TaskCreated` are inserted.
    /// - `TaskAssigned` updates assigned agent and transitions Ready tasks to Running.
    /// - `TaskCompleted` transitions task to Completed (if success=true) or Failed (if success=false).
    /// - Terminal states (`Completed`, `Failed`, `Cancelled`) stay terminal.
    /// - Tasks that were `Running` when the process terminated rehydrate as `Failed` with
    ///   reason `process_terminated`, and cascade `Blocked` to dependent tasks.
    pub fn rehydrate_from_events(&self, events: &[RuntimeEventEnvelope]) -> Result<(), TaskError> {
        if self.store.is_some() {
            return Err(TaskError::Persistence(
                "observer replay cannot overwrite an authoritative task journal".into(),
            ));
        }
        // Validate the entire input before changing any projection state.
        for envelope in events {
            if let RuntimeEvent::TaskCreated {
                record: Some(record),
                ..
            } = &envelope.payload
            {
                record.validate()?;
            }
        }
        let mut tasks = self
            .tasks
            .write()
            .map_err(|_| TaskError::InvalidTransition {
                task_id: TaskId::new(),
                from: TaskState::Pending,
                to: TaskState::Failed,
            })?;

        for envelope in events {
            match &envelope.payload {
                RuntimeEvent::TaskCreated { task_id, record } => {
                    if let Some(rec) = record {
                        tasks.insert(*task_id, rec.clone());
                    } else {
                        let mut rec = TaskRecord::new(envelope.run_id, format!("task-{}", task_id));
                        rec.id = *task_id;
                        rec.assigned_to = envelope.agent_id;
                        rec.created_at_ms = envelope.timestamp_ms;
                        rec.updated_at_ms = envelope.timestamp_ms;
                        tasks.insert(*task_id, rec);
                    }
                }
                RuntimeEvent::TaskAssigned { task_id, agent_id } => {
                    if let Some(task) = tasks.get_mut(task_id) {
                        task.assigned_to = Some(*agent_id);
                        if task.state == TaskState::Ready {
                            task.state = TaskState::Running;
                        }
                        task.updated_at_ms = envelope.timestamp_ms;
                    }
                }
                RuntimeEvent::TaskCompleted { task_id, success } => {
                    if let Some(task) = tasks.get_mut(task_id) {
                        if *success {
                            task.state = TaskState::Completed;
                        } else {
                            task.state = TaskState::Failed;
                        }
                        task.updated_at_ms = envelope.timestamp_ms;
                    }
                }
                _ => {}
            }
        }

        // Post-replay: tasks that were Running when the process died transition to Failed with "process_terminated"
        let now = now_ms();
        let mut newly_failed = Vec::new();
        for task in tasks.values_mut() {
            if task.state == TaskState::Running {
                task.state = TaskState::Failed;
                task.result = Some("process_terminated".to_string());
                task.updated_at_ms = now;
                newly_failed.push(task.id);
            }
        }

        // Cascade Blocked state to dependents of newly failed tasks
        while let Some(failed_id) = newly_failed.pop() {
            for other_task in tasks.values_mut() {
                if other_task.dependencies.contains(&failed_id)
                    && !other_task.state.is_terminal()
                    && other_task.state != TaskState::Blocked
                {
                    other_task.state = TaskState::Blocked;
                    other_task.updated_at_ms = now;
                    newly_failed.push(other_task.id);
                }
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn f03_migration_preserves_original_runs_with_authorized_scope() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tasks.jsonl");
        let first = TaskRecord::new(RunId::new(), "first historical run");
        let mut second = TaskRecord::new(RunId::new(), "second historical run");
        second.state = TaskState::Cancelled;
        second.revision = 7;
        let (registry, run) = TaskRegistry::open_session_durable_with_legacy(
            &path,
            "source/session",
            vec![first.clone(), second.clone()],
        )
        .unwrap();
        assert_eq!(registry.list_tasks(Some(run)).len(), 2);
        assert_eq!(registry.get_task(&first.id), Some(first.clone()));
        assert_eq!(registry.get_task(&second.id), Some(second.clone()));
        assert!(registry.list_tasks(Some(RunId::new())).is_empty());
        drop(registry);
        let (registry, resumed) =
            TaskRegistry::open_session_durable(&path, "source/session").unwrap();
        assert_eq!(resumed, run);
        assert_eq!(registry.get_task(&second.id), Some(second));
        registry
            .claim_task(first.id, run, AgentId::new(), first.revision)
            .unwrap();
        let claimed = registry.get_task(&first.id).unwrap();
        assert_eq!(claimed.run_id, first.run_id);
        assert_eq!(claimed.revision, first.revision + 1);
    }

    #[test]
    fn f03_migration_receipts_dependencies_and_scope_survive_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tasks.jsonl");
        let first = TaskRecord::new(RunId::new(), "historical prerequisite");
        let mut second = TaskRecord::new(RunId::new(), "historical dependent");
        second.dependencies = vec![first.id];
        second.state = TaskState::Pending;
        let (registry, run) = TaskRegistry::open_session_durable_with_legacy(
            &path,
            "session",
            vec![second.clone(), first.clone()],
        )
        .unwrap();
        assert_eq!(registry.list_tasks(Some(first.run_id)), vec![first.clone()]);
        assert!(!registry.matches_run(second.run_id, first.run_id));
        let actor = AgentId::new();
        assert!(matches!(
            registry.claim_task(first.id, second.run_id, actor, 0),
            Err(TaskError::RunMismatch(_))
        ));
        let claim_id = uuid::Uuid::new_v4();
        let claimed = registry
            .claim_operation(first.id, run, actor, 0, claim_id)
            .unwrap();
        let owner = TaskOwner {
            run_id: run,
            agent_id: actor,
            revision: claimed.revision,
            generation: claimed.owner_generation,
        };
        let complete_id = uuid::Uuid::new_v4();
        let completed = registry
            .complete_operation(first.id, Some("verified".into()), owner, complete_id)
            .unwrap();
        assert_eq!(completed.run_id, first.run_id);
        assert_eq!(
            registry.get_task(&second.id).unwrap().state,
            TaskState::Ready
        );
        let created = registry
            .create_command(
                TaskCreateRequest {
                    title: "new dependent".into(),
                    dependencies: vec![first.id],
                    ..TaskCreateRequest::default()
                },
                run,
                actor,
                Some(uuid::Uuid::new_v4()),
            )
            .unwrap();
        assert_eq!(created.state, TaskState::Ready);
        assert_eq!(created.run_id, run);
        drop(registry);
        let before = std::fs::read(&path).unwrap();
        let (registry, resumed) = TaskRegistry::open_session_durable_with_legacy(
            &path,
            "session",
            vec![TaskRecord::new(RunId::new(), "must not reimport")],
        )
        .unwrap();
        assert_eq!(resumed, run);
        assert_eq!(registry.list_tasks(Some(run)).len(), 3);
        assert_eq!(
            registry
                .claim_operation(first.id, run, actor, 0, claim_id)
                .unwrap(),
            claimed
        );
        assert_eq!(
            registry
                .complete_operation(first.id, Some("verified".into()), owner, complete_id)
                .unwrap(),
            completed
        );
        drop(registry);
        assert_eq!(std::fs::read(&path).unwrap(), before);
    }

    #[test]
    fn f03_migration_rejects_invalid_snapshots_before_writing() {
        let first = TaskRecord::new(RunId::new(), "original");
        let mut missing = first.clone();
        missing.dependencies.push(TaskId::new());
        let mut cyclic = first.clone();
        cyclic.dependencies.push(cyclic.id);
        let mut oversized = first.clone();
        oversized.description = Some("x".repeat(20_000));
        for records in [
            vec![first.clone(), first.clone()],
            vec![missing],
            vec![cyclic],
            vec![oversized],
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("tasks.jsonl");
            assert!(
                TaskRegistry::open_session_durable_with_legacy(&path, "session", records).is_err()
            );
            assert_eq!(std::fs::metadata(path).unwrap().len(), 0);
        }
    }

    #[test]
    fn f03_creation_replays_after_reopen_and_later_updates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tasks.jsonl");
        let run = RunId::new();
        let actor = AgentId::new();
        let operation = uuid::Uuid::new_v4();
        let input = TaskCreateRequest {
            title: "durable creation".into(),
            ..TaskCreateRequest::default()
        };
        let original = {
            let registry = TaskRegistry::open_durable(&path, run).unwrap();
            let original = registry
                .create_command(input.clone(), run, actor, Some(operation))
                .unwrap();
            registry.claim_task(original.id, run, actor, 1).unwrap();
            registry
                .complete_owned_task(
                    original.id,
                    Some("later".into()),
                    TaskOwner {
                        run_id: run,
                        agent_id: actor,
                        revision: 2,
                        generation: 1,
                    },
                )
                .unwrap();
            original
        };
        let before = std::fs::read(&path).unwrap();
        let registry = TaskRegistry::open_durable(&path, run).unwrap();
        assert_eq!(
            registry
                .create_command(input.clone(), run, actor, Some(operation))
                .unwrap(),
            original
        );
        assert_eq!(
            registry.get_task(&original.id).unwrap().state,
            TaskState::Completed
        );
        assert_eq!(registry.list_tasks(None).len(), 1);
        assert_eq!(
            registry.create_command(input.clone(), run, AgentId::new(), Some(operation)),
            Err(TaskError::OperationConflict)
        );
        assert_eq!(
            registry.create_command(
                TaskCreateRequest {
                    title: "different".into(),
                    ..input
                },
                run,
                actor,
                Some(operation)
            ),
            Err(TaskError::OperationConflict)
        );
        assert_eq!(
            registry.claim_operation(original.id, run, actor, 1, operation),
            Err(TaskError::OperationConflict)
        );
        drop(registry);
        assert_eq!(std::fs::read(path).unwrap(), before);
    }

    #[test]
    fn f03_creation_concurrent_retries_publish_once() {
        struct Count(Arc<AtomicU64>);
        impl RuntimeSubscriber for Count {
            fn on_event(&self, event: &RuntimeEventEnvelope) -> RuntimeDecision {
                if matches!(event.payload, RuntimeEvent::TaskCreated { .. }) {
                    self.0.fetch_add(1, Ordering::SeqCst);
                }
                RuntimeDecision::Continue
            }
        }
        let count = Arc::new(AtomicU64::new(0));
        let bus = RuntimeBus::new();
        bus.subscribe(Arc::new(Count(count.clone())));
        let registry = TaskRegistry::with_bus(bus);
        let run = RunId::new();
        let actor = AgentId::new();
        let operation = uuid::Uuid::new_v4();
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let registry = registry.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    registry
                        .create_command(
                            TaskCreateRequest {
                                title: "race".into(),
                                ..TaskCreateRequest::default()
                            },
                            run,
                            actor,
                            Some(operation),
                        )
                        .unwrap()
                })
            })
            .collect();
        let responses: Vec<_> = handles
            .into_iter()
            .map(|handle| handle.join().unwrap())
            .collect();
        assert_eq!(responses[0], responses[1]);
        assert_eq!(registry.list_tasks(None).len(), 1);
        assert_eq!(registry.receipts.read().unwrap().len(), 1);
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn f03_creation_rejects_foreign_owner_and_dependencies() {
        let registry = TaskRegistry::new();
        let run = RunId::new();
        let actor = AgentId::new();
        let foreign = registry
            .create_task(TaskRecord::new(RunId::new(), "foreign"))
            .unwrap();
        let local = registry.create_task(TaskRecord::new(run, "local")).unwrap();
        for input in [
            TaskCreateRequest {
                assigned_to: Some(AgentId::new()),
                ..TaskCreateRequest::default()
            },
            TaskCreateRequest {
                dependencies: vec![foreign],
                ..TaskCreateRequest::default()
            },
            TaskCreateRequest {
                dependencies: vec![local, local],
                ..TaskCreateRequest::default()
            },
        ] {
            assert!(registry
                .create_command(input, run, actor, Some(uuid::Uuid::new_v4()))
                .is_err());
        }
        assert_eq!(registry.list_tasks(None).len(), 2);
        assert!(registry.receipts.read().unwrap().is_empty());
    }

    #[test]
    fn f03_creation_failed_persistence_does_not_cache_success() {
        struct Refuse;
        impl TaskCommitSink for Refuse {
            fn commit(&self, _: Vec<TaskChange>) -> Result<(), TaskError> {
                Err(TaskError::Persistence("fixture disk full".into()))
            }
        }
        let mut registry = TaskRegistry::new();
        registry.store = Some(Arc::new(Refuse));
        let run = RunId::new();
        let actor = AgentId::new();
        let operation = uuid::Uuid::new_v4();
        let input = TaskCreateRequest {
            title: "retry refused commit".into(),
            ..TaskCreateRequest::default()
        };
        assert!(registry
            .create_command(input.clone(), run, actor, Some(operation))
            .is_err());
        assert!(registry.receipts.read().unwrap().is_empty());
        assert!(registry.list_tasks(None).is_empty());
        registry.store = None;
        assert!(registry
            .create_command(input, run, actor, Some(operation))
            .is_ok());
    }

    #[test]
    fn f03_status_capacity_rechecked_after_concurrent_commit() {
        struct Pause {
            entered: std::sync::mpsc::SyncSender<()>,
            release: Mutex<std::sync::mpsc::Receiver<()>>,
        }
        impl RuntimeSubscriber for Pause {
            fn on_event(&self, event: &RuntimeEventEnvelope) -> RuntimeDecision {
                if matches!(event.payload, RuntimeEvent::TaskCompletionRequested { .. }) {
                    self.entered.send(()).unwrap();
                    self.release
                        .lock()
                        .unwrap()
                        .recv_timeout(std::time::Duration::from_secs(5))
                        .unwrap();
                }
                RuntimeDecision::Continue
            }
        }
        let bus = RuntimeBus::new();
        let registry = TaskRegistry::with_bus(bus.clone());
        let run = RunId::new();
        let actor = AgentId::new();
        let id = registry
            .create_task(TaskRecord::new(run, "paused"))
            .unwrap();
        let other = registry
            .create_task(TaskRecord::new(run, "winner"))
            .unwrap();
        let claim = uuid::Uuid::new_v4();
        registry.claim_operation(id, run, actor, 1, claim).unwrap();
        registry.claim_task(other, run, actor, 1).unwrap();
        // Seed only the receipt projection: this tests admission/commit locking,
        // not journal replay or the validity of fabricated historical requests.
        {
            let mut receipts = registry.receipts.write().unwrap();
            let template = receipts[&claim].clone();
            for _ in 1..MAX_OPERATION_RECEIPTS - 1 {
                let receipt = TaskOperationReceipt {
                    operation_id: uuid::Uuid::new_v4(),
                    ..template.clone()
                };
                receipts.insert(receipt.operation_id, receipt);
            }
        }
        let owner = TaskOwner {
            run_id: run,
            agent_id: actor,
            revision: 2,
            generation: 1,
        };
        let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(1);
        let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);
        bus.subscribe(Arc::new(Pause {
            entered: entered_tx,
            release: Mutex::new(release_rx),
        }));
        let worker = registry.clone();
        let handle = std::thread::spawn(move || {
            worker.complete_operation(id, None, owner, uuid::Uuid::new_v4())
        });
        entered_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        registry
            .status_operation(other, TaskState::Failed, None, owner, uuid::Uuid::new_v4())
            .unwrap();
        release_tx.send(()).unwrap();
        assert!(matches!(
            handle.join().unwrap(),
            Err(TaskError::Persistence(_))
        ));
        assert_eq!(
            registry.receipts.read().unwrap().len(),
            MAX_OPERATION_RECEIPTS
        );
        assert_eq!(registry.get_task(&id).unwrap().state, TaskState::Running);
        assert_eq!(registry.get_task(&id).unwrap().revision, 2);
        assert!(registry.pending_operations.read().unwrap().is_empty());
    }

    #[test]
    fn f03_concurrent_completion_operation_runs_one_hook() {
        struct Pause {
            entered: std::sync::mpsc::SyncSender<()>,
            release: Mutex<std::sync::mpsc::Receiver<()>>,
        }
        impl RuntimeSubscriber for Pause {
            fn on_event(&self, event: &RuntimeEventEnvelope) -> RuntimeDecision {
                if matches!(event.payload, RuntimeEvent::TaskCompletionRequested { .. }) {
                    self.entered.send(()).unwrap();
                    self.release
                        .lock()
                        .unwrap()
                        .recv_timeout(std::time::Duration::from_secs(5))
                        .unwrap();
                }
                RuntimeDecision::Continue
            }
        }
        let bus = RuntimeBus::new();
        let registry = TaskRegistry::with_bus(bus.clone());
        let run = RunId::new();
        let actor = AgentId::new();
        let id = registry
            .create_task(TaskRecord::new(run, "concurrent"))
            .unwrap();
        registry.claim_task(id, run, actor, 1).unwrap();
        let owner = TaskOwner {
            run_id: run,
            agent_id: actor,
            revision: 2,
            generation: 1,
        };
        let operation = uuid::Uuid::new_v4();
        let (entered_tx, entered_rx) = std::sync::mpsc::sync_channel(1);
        let (release_tx, release_rx) = std::sync::mpsc::sync_channel(1);
        bus.subscribe(Arc::new(Pause {
            entered: entered_tx,
            release: Mutex::new(release_rx),
        }));
        let worker = registry.clone();
        let handle =
            std::thread::spawn(move || worker.complete_operation(id, None, owner, operation));
        entered_rx
            .recv_timeout(std::time::Duration::from_secs(5))
            .unwrap();
        assert_eq!(
            registry.complete_operation(id, None, owner, operation),
            Err(TaskError::OperationInProgress)
        );
        release_tx.send(()).unwrap();
        let original = handle.join().unwrap().unwrap();
        assert_eq!(
            registry
                .complete_operation(id, None, owner, operation)
                .unwrap(),
            original
        );
        assert!(entered_rx.try_recv().is_err());
    }

    #[test]
    fn f03_status_operations_replay_cascades() {
        for state in [
            TaskState::Completed,
            TaskState::Failed,
            TaskState::Cancelled,
        ] {
            let dir = tempfile::tempdir().unwrap();
            let path = dir.path().join("tasks.jsonl");
            let run = RunId::new();
            let actor = AgentId::new();
            let owner = TaskOwner {
                run_id: run,
                agent_id: actor,
                revision: 2,
                generation: 1,
            };
            let operation = uuid::Uuid::new_v4();
            let (id, child, response) = {
                let registry = TaskRegistry::open_durable(&path, run).unwrap();
                let id = registry
                    .create_task(TaskRecord::new(run, "parent"))
                    .unwrap();
                let child = registry
                    .create_task(TaskRecord::new(run, "child").with_dependencies(vec![id]))
                    .unwrap();
                registry.claim_task(id, run, actor, 1).unwrap();
                let response = registry
                    .status_operation(id, state, None, owner, operation)
                    .unwrap();
                (id, child, response)
            };
            let registry = TaskRegistry::open_durable(path, run).unwrap();
            assert_eq!(
                registry
                    .status_operation(id, state, None, owner, operation)
                    .unwrap(),
                response
            );
            let expected = if state == TaskState::Completed {
                TaskState::Ready
            } else {
                TaskState::Blocked
            };
            assert_eq!(registry.get_task(&child).unwrap().state, expected);
            assert!(registry
                .claim_operation(id, run, actor, 2, operation)
                .is_err());
            assert!(registry
                .status_operation(
                    id,
                    state,
                    None,
                    TaskOwner {
                        agent_id: AgentId::new(),
                        ..owner
                    },
                    operation
                )
                .is_err());
        }
    }

    #[test]
    fn f03_pending_completion_rejects_reentry_and_releases_after_denial() {
        struct Reenter {
            registry: TaskRegistry,
            owner: TaskOwner,
            operation: uuid::Uuid,
            calls: Arc<AtomicU64>,
        }
        impl RuntimeSubscriber for Reenter {
            fn on_event(&self, event: &RuntimeEventEnvelope) -> RuntimeDecision {
                if let RuntimeEvent::TaskCompletionRequested { task_id, .. } = event.payload {
                    assert_eq!(
                        self.registry
                            .complete_operation(task_id, None, self.owner, self.operation),
                        Err(TaskError::OperationInProgress)
                    );
                    if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
                        return RuntimeDecision::Deny {
                            reason: "fixture denial".into(),
                        };
                    }
                }
                RuntimeDecision::Continue
            }
        }
        let bus = RuntimeBus::new();
        let registry = TaskRegistry::with_bus(bus.clone());
        let run = RunId::new();
        let actor = AgentId::new();
        let id = registry.create_task(TaskRecord::new(run, "hooks")).unwrap();
        registry.claim_task(id, run, actor, 1).unwrap();
        let owner = TaskOwner {
            run_id: run,
            agent_id: actor,
            revision: 2,
            generation: 1,
        };
        let operation = uuid::Uuid::new_v4();
        let calls = Arc::new(AtomicU64::new(0));
        bus.subscribe(Arc::new(Reenter {
            registry: registry.clone(),
            owner,
            operation,
            calls: calls.clone(),
        }));
        assert!(registry
            .complete_operation(id, None, owner, operation)
            .is_err());
        assert!(registry.pending_operations.read().unwrap().is_empty());
        registry
            .complete_operation(id, None, owner, operation)
            .unwrap();
        registry
            .complete_operation(id, None, owner, operation)
            .unwrap();
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn f03_completion_operation_replays_without_hooks_after_reopen() {
        struct Count(Arc<AtomicU64>);
        impl RuntimeSubscriber for Count {
            fn on_event(&self, event: &RuntimeEventEnvelope) -> RuntimeDecision {
                if matches!(event.payload, RuntimeEvent::TaskCompletionRequested { .. }) {
                    self.0.fetch_add(1, Ordering::SeqCst);
                }
                RuntimeDecision::Continue
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tasks.jsonl");
        let run = RunId::new();
        let actor = AgentId::new();
        let count = Arc::new(AtomicU64::new(0));
        let bus = RuntimeBus::new();
        bus.subscribe(Arc::new(Count(count.clone())));
        let owner = TaskOwner {
            run_id: run,
            agent_id: actor,
            revision: 2,
            generation: 1,
        };
        let op = uuid::Uuid::new_v4();
        let (id, original) = {
            let registry = TaskRegistry::open_durable(&path, run)
                .unwrap()
                .with_observers(bus.clone());
            let id = registry
                .create_task(TaskRecord::new(run, "complete"))
                .unwrap();
            registry.claim_task(id, run, actor, 1).unwrap();
            let original = registry
                .complete_operation(id, Some("done".into()), owner, op)
                .unwrap();
            assert_eq!(
                registry
                    .complete_operation(id, Some("done".into()), owner, op)
                    .unwrap(),
                original
            );
            (id, original)
        };
        let registry = TaskRegistry::open_durable(path, run)
            .unwrap()
            .with_observers(bus);
        assert_eq!(
            registry
                .complete_operation(id, Some("done".into()), owner, op)
                .unwrap(),
            original
        );
        assert!(registry
            .complete_operation(id, Some("changed".into()), owner, op)
            .is_err());
        assert_eq!(count.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn f03_claim_receipt_capacity_preserves_existing_replay() {
        let registry = TaskRegistry::new();
        let run = RunId::new();
        let actor = AgentId::new();
        let id = registry
            .create_task(TaskRecord::new(run, "cached"))
            .unwrap();
        let operation = uuid::Uuid::new_v4();
        let response = registry
            .claim_operation(id, run, actor, 1, operation)
            .unwrap();
        {
            let mut receipts = registry.receipts.write().unwrap();
            let template = receipts[&operation].clone();
            for _ in 1..MAX_OPERATION_RECEIPTS {
                let receipt = TaskOperationReceipt {
                    operation_id: uuid::Uuid::new_v4(),
                    ..template.clone()
                };
                receipts.insert(receipt.operation_id, receipt);
            }
        }
        let other = registry
            .create_task(TaskRecord::new(run, "capacity"))
            .unwrap();
        let before = registry.get_task(&other);
        assert!(registry
            .claim_operation(other, run, actor, 1, uuid::Uuid::new_v4())
            .is_err());
        assert_eq!(registry.get_task(&other), before);
        assert_eq!(
            registry
                .claim_operation(id, run, actor, 1, operation)
                .unwrap(),
            response
        );
    }

    #[test]
    fn f03_claim_operation_concurrent_retry_commits_once() {
        let registry = TaskRegistry::new();
        let run = RunId::new();
        let actor = AgentId::new();
        let operation = uuid::Uuid::new_v4();
        let id = registry
            .create_task(TaskRecord::new(run, "duplicate"))
            .unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let registry = registry.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    barrier.wait();
                    registry
                        .claim_operation(id, run, actor, 1, operation)
                        .unwrap()
                })
            })
            .collect();
        let responses: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(responses[0], responses[1]);
        assert_eq!(registry.get_task(&id).unwrap().revision, 2);
        assert_eq!(registry.receipts.read().unwrap().len(), 1);
    }

    #[test]
    fn f03_claim_operation_storage_failure_does_not_cache_success() {
        struct Refuse;
        impl TaskCommitSink for Refuse {
            fn commit(&self, _: Vec<TaskChange>) -> Result<(), TaskError> {
                Err(TaskError::Persistence("fixture refused".into()))
            }
        }
        let mut registry = TaskRegistry::new();
        let run = RunId::new();
        let actor = AgentId::new();
        let operation = uuid::Uuid::new_v4();
        let id = registry
            .create_task(TaskRecord::new(run, "retry failed write"))
            .unwrap();
        let before = registry.get_task(&id);
        registry.store = Some(Arc::new(Refuse));
        assert!(registry
            .claim_operation(id, run, actor, 1, operation)
            .is_err());
        assert!(registry.receipts.read().unwrap().is_empty());
        assert_eq!(registry.get_task(&id), before);
        registry.store = None;
        assert!(registry
            .claim_operation(id, run, actor, 1, operation)
            .is_ok());
    }

    #[test]
    fn f03_claim_operation_replays_original_after_reopen() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tasks.jsonl");
        let run = RunId::new();
        let actor = AgentId::new();
        let operation = uuid::Uuid::new_v4();
        let (id, original) = {
            let registry = TaskRegistry::open_durable(&path, run).unwrap();
            let id = registry.create_task(TaskRecord::new(run, "retry")).unwrap();
            let original = registry
                .claim_operation(id, run, actor, 1, operation)
                .unwrap();
            registry.complete_task(id, Some("later".into())).unwrap();
            assert_eq!(
                registry
                    .claim_operation(id, run, actor, 1, operation)
                    .unwrap(),
                original
            );
            (id, original)
        };
        let before = std::fs::read(&path).unwrap();
        let registry = TaskRegistry::open_durable(&path, run).unwrap();
        assert_eq!(
            registry
                .claim_operation(id, run, actor, 1, operation)
                .unwrap(),
            original
        );
        assert!(registry
            .claim_operation(id, run, AgentId::new(), 1, operation)
            .is_err());
        assert!(registry
            .claim_operation(id, run, actor, 2, operation)
            .is_err());
        assert_eq!(registry.get_task(&id).unwrap().state, TaskState::Completed);
        drop(registry);
        assert_eq!(std::fs::read(path).unwrap(), before);
    }

    #[test]
    fn f03_owned_status_checks_identity_revision_and_generation() {
        for state in [
            TaskState::Completed,
            TaskState::Failed,
            TaskState::Cancelled,
        ] {
            let registry = TaskRegistry::new();
            let run = RunId::new();
            let actor = AgentId::new();
            let id = registry.create_task(TaskRecord::new(run, "owned")).unwrap();
            registry.claim_task(id, run, actor, 1).unwrap();
            let valid = TaskOwner {
                run_id: run,
                agent_id: actor,
                revision: 2,
                generation: 1,
            };
            let apply = |owner| match state {
                TaskState::Completed => registry.complete_owned_task(id, None, owner),
                TaskState::Failed => registry.fail_owned_task(id, None, owner),
                _ => registry.cancel_owned_task(id, owner),
            };
            let before = registry.get_task(&id);
            for invalid in [
                TaskOwner {
                    run_id: RunId::new(),
                    ..valid
                },
                TaskOwner {
                    agent_id: AgentId::new(),
                    ..valid
                },
                TaskOwner {
                    revision: 1,
                    ..valid
                },
                TaskOwner {
                    generation: 0,
                    ..valid
                },
            ] {
                assert!(apply(invalid).is_err());
                assert_eq!(registry.get_task(&id), before);
            }
            apply(valid).unwrap();
            assert_eq!(registry.get_task(&id).unwrap().state, state);
            assert!(apply(valid).is_err());
        }
    }

    #[test]
    fn f03_owned_completion_rechecks_after_hook() {
        struct Reassign {
            registry: TaskRegistry,
            replacement: AgentId,
        }
        impl RuntimeSubscriber for Reassign {
            fn on_event(&self, event: &RuntimeEventEnvelope) -> RuntimeDecision {
                if let RuntimeEvent::TaskCompletionRequested { task_id, .. } = event.payload {
                    self.registry
                        .assign_task(task_id, self.replacement)
                        .unwrap();
                }
                RuntimeDecision::Continue
            }
        }
        let bus = RuntimeBus::new();
        let registry = TaskRegistry::with_bus(bus.clone());
        let run = RunId::new();
        let actor = AgentId::new();
        let id = registry.create_task(TaskRecord::new(run, "race")).unwrap();
        registry.claim_task(id, run, actor, 1).unwrap();
        let replacement = AgentId::new();
        bus.subscribe(Arc::new(Reassign {
            registry: registry.clone(),
            replacement,
        }));
        assert_eq!(
            registry.complete_owned_task(
                id,
                Some("stale".into()),
                TaskOwner {
                    run_id: run,
                    agent_id: actor,
                    revision: 2,
                    generation: 1,
                }
            ),
            Err(TaskError::RevisionConflict(id))
        );
        let task = registry.get_task(&id).unwrap();
        assert_eq!(task.state, TaskState::Running);
        assert_eq!(task.assigned_to, Some(replacement));
        assert_eq!(task.result, None);
    }

    #[test]
    fn f03_claim_counter_overflow_preserves_projection() {
        for (revision, generation) in [(u64::MAX, 0), (1, u64::MAX)] {
            let registry = TaskRegistry::new();
            let run = RunId::new();
            let mut task = TaskRecord::new(run, "exhausted");
            task.revision = revision;
            task.owner_generation = generation;
            registry
                .tasks
                .write()
                .unwrap()
                .insert(task.id, task.clone());
            assert_eq!(
                registry.claim_task(task.id, run, AgentId::new(), revision),
                Err(TaskError::RevisionOverflow(task.id))
            );
            assert_eq!(registry.get_task(&task.id), Some(task));
        }
    }

    #[test]
    fn f03_claim_rejections_preserve_projection() {
        let registry = super::TaskRegistry::new();
        let run = super::RunId::new();
        let actor = super::AgentId::new();
        let dependency = registry
            .create_task(super::TaskRecord::new(run, "dependency"))
            .unwrap();
        let pending = registry
            .create_task(super::TaskRecord::new(run, "pending").with_dependencies(vec![dependency]))
            .unwrap();
        let before = registry.get_task(&pending).unwrap();
        assert!(registry
            .claim_task(pending, run, actor, before.revision)
            .is_err());
        assert_eq!(registry.get_task(&pending).unwrap(), before);
        let before = registry.get_task(&dependency).unwrap();
        assert!(registry
            .claim_task(dependency, super::RunId::new(), actor, before.revision)
            .is_err());
        assert!(registry
            .claim_task(dependency, run, actor, before.revision + 1)
            .is_err());
        assert_eq!(registry.get_task(&dependency).unwrap(), before);
        registry.cancel_task(dependency).unwrap();
        let before = registry.get_task(&dependency).unwrap();
        assert!(registry
            .claim_task(dependency, run, actor, before.revision)
            .is_err());
        assert_eq!(registry.get_task(&dependency).unwrap(), before);
    }

    #[test]
    fn f03_claim_race() {
        let registry = super::TaskRegistry::new();
        let run = super::RunId::new();
        let id = registry
            .create_task(super::TaskRecord::new(run, "race"))
            .unwrap();
        let revision = registry.get_task(&id).unwrap().revision;
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let registry = registry.clone();
                let barrier = barrier.clone();
                std::thread::spawn(move || {
                    let actor = super::AgentId::new();
                    barrier.wait();
                    (actor, registry.claim_task(id, run, actor, revision))
                })
            })
            .collect();
        let outcomes: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(
            outcomes.iter().filter(|(_, result)| result.is_ok()).count(),
            1
        );
        let task = registry.get_task(&id).unwrap();
        assert_eq!(task.revision, revision + 1);
        assert_eq!(task.owner_generation, 1);
        assert_eq!(
            task.assigned_to,
            outcomes
                .iter()
                .find(|(_, result)| result.is_ok())
                .map(|(actor, _)| *actor)
        );
    }

    use super::*;
    use crate::runtime::bus::{RuntimeDecision, RuntimeSubscriber};
    use std::sync::Mutex;

    #[test]
    fn task_rewind_is_atomic() {
        let registry = TaskRegistry::new();
        let run = RunId::new();
        let root = registry.create_task(TaskRecord::new(run, "root")).unwrap();
        let dependent = registry
            .create_task(TaskRecord::new(run, "dependent").with_dependencies(vec![root]))
            .unwrap();
        let missing = TaskId::new();
        registry
            .tasks
            .write()
            .unwrap()
            .get_mut(&root)
            .unwrap()
            .dependencies
            .push(missing);

        let before = registry.tasks.read().unwrap().clone();
        assert_eq!(
            registry.rewind_task_state(root, "checkpoint"),
            Err(TaskError::UnknownDependency(missing))
        );
        let after = registry.tasks.read().unwrap().clone();
        assert_eq!(after, before);
        assert_eq!(after[&dependent].state, TaskState::Pending);

        {
            let mut tasks = registry.tasks.write().unwrap();
            tasks
                .get_mut(&root)
                .unwrap()
                .dependencies
                .retain(|dependency| *dependency != missing);
            tasks.get_mut(&dependent).unwrap().revision = u64::MAX;
        }
        let before = registry.tasks.read().unwrap().clone();
        assert_eq!(
            registry.rewind_task_state(root, "checkpoint"),
            Err(TaskError::RevisionOverflow(dependent))
        );
        let after = registry.tasks.read().unwrap().clone();
        assert_eq!(after, before);
    }

    #[test]
    fn f03_no_publish_without_commit() {
        struct DiskFull;
        impl TaskCommitSink for DiskFull {
            fn commit(&self, _: Vec<TaskChange>) -> Result<(), TaskError> {
                Err(TaskError::Persistence("fixture: disk full".into()))
            }
        }
        let bus = RuntimeBus::new();
        let events = Arc::new(Mutex::new(Vec::new()));
        bus.subscribe(Arc::new(EventCapture {
            events: events.clone(),
        }));
        let mut registry = TaskRegistry::with_bus(bus);
        let run = RunId::new();
        let parent = registry
            .create_task(TaskRecord::new(run, "parent"))
            .unwrap();
        registry
            .create_task(TaskRecord::new(run, "child").with_dependencies(vec![parent]))
            .unwrap();
        let before = registry.tasks.read().unwrap().clone();
        registry.store = Some(Arc::new(DiskFull));
        events.lock().unwrap().clear();
        assert!(registry
            .create_task(TaskRecord::new(run, "not created"))
            .is_err());
        assert!(registry.assign_task(parent, AgentId::new()).is_err());
        assert!(registry
            .claim_task(parent, run, AgentId::new(), before[&parent].revision)
            .is_err());
        assert!(registry
            .complete_task(parent, Some("not published".into()))
            .is_err());
        assert!(registry
            .fail_task(parent, Some("not published".into()))
            .is_err());
        assert!(registry.cancel_task(parent).is_err());
        assert_eq!(*registry.tasks.read().unwrap(), before);
        assert!(events
            .lock()
            .unwrap()
            .iter()
            .all(|event| matches!(event, RuntimeEvent::TaskCompletionRequested { .. })));
    }

    #[test]
    fn f03_public_status() {
        for (state, public) in [
            (TaskState::Pending, "pending"),
            (TaskState::Ready, "pending"),
            (TaskState::Running, "in_progress"),
            (TaskState::Completed, "completed"),
            (TaskState::Failed, "failed"),
            (TaskState::Blocked, "blocked"),
            (TaskState::Cancelled, "cancelled"),
        ] {
            assert_eq!(state.public_status(), public);
        }
        // Presentation must not change the persisted readiness vocabulary.
        assert_eq!(
            serde_json::to_string(&TaskState::Ready).unwrap(),
            "\"ready\""
        );
    }

    #[test]
    fn f03_old_record_defaults_without_inventing_plan_or_evidence() {
        let record: TaskRecord = serde_json::from_value(serde_json::json!({
            "id": "00000000-0000-0000-0000-000000000001",
            "run_id": "00000000-0000-0000-0000-000000000002",
            "title": "legacy", "description": null, "dependencies": [],
            "state": "ready", "assigned_to": null, "result": null,
            "created_at_ms": 12, "updated_at_ms": 34
        }))
        .unwrap();
        assert_eq!(record.revision, 0);
        assert_eq!(record.owner_generation, 0);
        assert_eq!(record.attempt, 0);
        assert!(record.parent_plan_step.is_none());
        assert!(record.evidence_refs.is_empty());
        assert!(record.blocked_reasons.is_empty());
        assert_eq!(record.created_at_ms, 12);
        assert_eq!(record.updated_at_ms, 34);
    }

    #[test]
    fn f03_oversized_create_never_publishes_task() {
        let registry = TaskRegistry::new();
        let record = TaskRecord::new(RunId::new(), "é".repeat(257));
        let id = record.id;
        assert!(registry.create_task(record).is_err());
        assert!(registry.get_task(&id).is_none());
        let record = TaskRecord::new(RunId::new(), "description")
            .with_description("x".repeat(16 * 1024 + 1));
        let id = record.id;
        assert!(registry.create_task(record).is_err());
        assert!(registry.get_task(&id).is_none());
    }

    #[test]
    fn f03_collection_limits_apply_before_registry_insert() {
        let registry = TaskRegistry::new();
        let mut evidence = TaskRecord::new(RunId::new(), "evidence");
        evidence.evidence_refs = (0..129).map(|_| EvidenceId::new()).collect();
        let id = evidence.id;
        assert_eq!(
            registry.create_task(evidence),
            Err(TaskError::MetadataTooLarge {
                field: "evidence_refs",
                limit: 128,
            })
        );
        assert!(registry.get_task(&id).is_none());
        let deps = TaskRecord::new(RunId::new(), "dependencies")
            .with_dependencies((0..65).map(|_| TaskId::new()).collect());
        let id = deps.id;
        assert_eq!(
            registry.create_task(deps),
            Err(TaskError::MetadataTooLarge {
                field: "dependencies",
                limit: 64,
            })
        );
        assert!(registry.get_task(&id).is_none());
    }

    #[test]
    fn f03_invalid_replay_leaves_registry_unchanged() {
        let registry = TaskRegistry::new();
        let run = RunId::new();
        let events: Vec<_> = ["valid".into(), "x".repeat(513)]
            .into_iter()
            .enumerate()
            .map(|(i, title)| {
                let record = TaskRecord::new(run, title);
                RuntimeEventEnvelope::new(
                    i as u64 + 1,
                    run,
                    None,
                    None,
                    None,
                    RuntimeEvent::TaskCreated {
                        task_id: record.id,
                        record: Some(record),
                    },
                )
            })
            .collect();
        assert!(registry.rehydrate_from_events(&events).is_err());
        assert!(registry.list_tasks(None).is_empty());
    }

    #[test]
    fn f03_committed_updates_keep_created_time_and_advance_revision() {
        let time = Arc::new(std::sync::atomic::AtomicI64::new(42));
        let clock = time.clone();
        let registry = TaskRegistry::new().with_clock(move || clock.load(Ordering::SeqCst));
        let mut record = TaskRecord::new(RunId::new(), "revision");
        record.created_at_ms = 12;
        let id = registry.create_task(record).unwrap();
        let created = registry.get_task(&id).unwrap();
        assert_eq!(
            (
                created.revision,
                created.created_at_ms,
                created.updated_at_ms
            ),
            (1, 12, 42)
        );
        time.store(43, Ordering::SeqCst);
        registry.assign_task(id, AgentId::new()).unwrap();
        time.store(44, Ordering::SeqCst);
        registry.complete_task(id, None).unwrap();
        let completed = registry.get_task(&id).unwrap();
        assert_eq!(
            (
                completed.revision,
                completed.created_at_ms,
                completed.updated_at_ms
            ),
            (3, 12, 44)
        );
    }

    #[test]
    fn f03_revision_overflow_never_changes_task() {
        let registry = TaskRegistry::new();
        let mut record = TaskRecord::new(RunId::new(), "overflow");
        record.revision = u64::MAX;
        let id = record.id;
        registry
            .rehydrate_from_events(&[RuntimeEventEnvelope::new(
                1,
                record.run_id,
                None,
                None,
                None,
                RuntimeEvent::TaskCreated {
                    task_id: id,
                    record: Some(record.clone()),
                },
            )])
            .unwrap();
        assert!(registry.assign_task(id, AgentId::new()).is_err());
        assert_eq!(registry.get_task(&id), Some(record.clone()));
        assert!(registry.complete_task(id, None).is_err());
        assert_eq!(registry.get_task(&id), Some(record.clone()));
        assert!(registry.fail_task(id, None).is_err());
        assert_eq!(registry.get_task(&id), Some(record.clone()));
        assert!(registry.cancel_task(id).is_err());
        assert_eq!(registry.get_task(&id), Some(record));
    }

    #[test]
    fn f03_cascade_overflow_rolls_back_entire_mutation() {
        for action in ["complete", "fail", "cancel"] {
            let registry = TaskRegistry::new();
            let run = RunId::new();
            let parent = TaskRecord::new(run, "parent");
            let mut child = TaskRecord::new(run, "child").with_dependencies(vec![parent.id]);
            child.state = TaskState::Pending;
            child.revision = u64::MAX;
            let events: Vec<_> = [parent.clone(), child.clone()]
                .into_iter()
                .enumerate()
                .map(|(i, record)| {
                    RuntimeEventEnvelope::new(
                        i as u64 + 1,
                        run,
                        None,
                        None,
                        None,
                        RuntimeEvent::TaskCreated {
                            task_id: record.id,
                            record: Some(record),
                        },
                    )
                })
                .collect();
            registry.rehydrate_from_events(&events).unwrap();
            let result = match action {
                "complete" => registry.complete_task(parent.id, None),
                "fail" => registry.fail_task(parent.id, None),
                _ => registry.cancel_task(parent.id),
            };
            assert_eq!(
                result,
                Err(TaskError::RevisionOverflow(child.id)),
                "{action}"
            );
            assert_eq!(registry.get_task(&parent.id), Some(parent));
            assert_eq!(registry.get_task(&child.id), Some(child));
        }
    }

    #[test]
    fn f03_clock_is_called_outside_registry_lock() {
        let registry = TaskRegistry::new();
        let tasks = registry.tasks.clone();
        let registry = registry.with_clock(move || {
            assert!(
                tasks.try_write().is_ok(),
                "clock invoked under registry lock"
            );
            42
        });
        let id = registry
            .create_task(TaskRecord::new(RunId::new(), "clock"))
            .unwrap();
        registry.assign_task(id, AgentId::new()).unwrap();
        registry.complete_task(id, None).unwrap();
    }

    #[test]
    fn f03_completion_hook_cannot_overwrite_intervening_cancellation() {
        struct CancelDuringCompletion(TaskRegistry);
        impl RuntimeSubscriber for CancelDuringCompletion {
            fn on_event(&self, event: &RuntimeEventEnvelope) -> RuntimeDecision {
                if let RuntimeEvent::TaskCompletionRequested { task_id, .. } = event.payload {
                    self.0.cancel_task(task_id).unwrap();
                }
                RuntimeDecision::Continue
            }
        }
        let bus = RuntimeBus::new();
        let registry = TaskRegistry::with_bus(bus.clone());
        bus.subscribe(Arc::new(CancelDuringCompletion(registry.clone())));
        let id = registry
            .create_task(TaskRecord::new(RunId::new(), "cancel in hook"))
            .unwrap();
        assert_eq!(
            registry.complete_task(id, None),
            Err(TaskError::RevisionConflict(id))
        );
        assert_eq!(registry.get_task(&id).unwrap().state, TaskState::Cancelled);
    }

    #[test]
    fn f03_metadata_boundaries_and_references_survive_registry() {
        let registry = TaskRegistry::new();
        let mut record =
            TaskRecord::new(RunId::new(), "é".repeat(256)).with_description("x".repeat(16 * 1024));
        record.parent_plan_step = Some(PlanStepRef {
            plan_id: "session-plan".into(),
            plan_revision: 7,
            step_id: "removed-step".into(),
        });
        record.evidence_refs = (0..128).map(|_| EvidenceId::new()).collect();
        let original = record.clone();
        let id = registry.create_task(record).unwrap();
        let stored = registry.get_task(&id).unwrap();
        assert_eq!(stored.parent_plan_step, original.parent_plan_step);
        assert_eq!(stored.evidence_refs, original.evidence_refs);
        assert_eq!(stored.created_at_ms, original.created_at_ms);
        assert_eq!(stored.title.len(), 512);
        assert_eq!(stored.description.unwrap().len(), 16 * 1024);
    }

    struct DenyCompletionSubscriber {
        denied_task_id: TaskId,
    }

    impl RuntimeSubscriber for DenyCompletionSubscriber {
        fn on_event(&self, event: &RuntimeEventEnvelope) -> RuntimeDecision {
            if let RuntimeEvent::TaskCompletionRequested { task_id, .. } = event.payload {
                if task_id == self.denied_task_id {
                    return RuntimeDecision::Deny {
                        reason: "Verification checks failed: output missing required format".into(),
                    };
                }
            }
            RuntimeDecision::Continue
        }
    }

    struct EventCapture {
        events: Arc<Mutex<Vec<RuntimeEvent>>>,
    }

    #[test]
    fn f03_rejected_completion_never_emits_completed_fact() {
        let bus = RuntimeBus::new();
        let registry = TaskRegistry::with_bus(bus.clone());
        let captured = Arc::new(Mutex::new(Vec::new()));
        // The log may be subscribed before the decision-making hook.
        bus.subscribe(Arc::new(EventCapture {
            events: captured.clone(),
        }));
        let id = registry
            .create_task(TaskRecord::new(RunId::new(), "denied"))
            .unwrap();
        bus.subscribe(Arc::new(DenyCompletionSubscriber { denied_task_id: id }));
        assert!(registry
            .complete_task(id, Some("uncommitted result".into()))
            .is_err());
        assert!(!captured
            .lock()
            .unwrap()
            .iter()
            .any(|event| matches!(event, RuntimeEvent::TaskCompleted { .. })));
        assert_eq!(registry.get_task(&id).unwrap().state, TaskState::Ready);
        let run_id = registry.get_task(&id).unwrap().run_id;
        let events: Vec<_> = captured
            .lock()
            .unwrap()
            .iter()
            .enumerate()
            .map(|(seq, event)| {
                RuntimeEventEnvelope::new(seq as u64 + 1, run_id, None, None, None, event.clone())
            })
            .collect();
        let restored = TaskRegistry::new();
        restored.rehydrate_from_events(&events).unwrap();
        assert_eq!(restored.get_task(&id).unwrap().state, TaskState::Ready);
        assert_eq!(restored.get_task(&id).unwrap().result, None);
    }

    #[test]
    fn f03_completed_observer_reads_published_result_and_cascade() {
        struct InspectCompletion {
            registry: TaskRegistry,
            dependent: TaskId,
            seen: Arc<std::sync::atomic::AtomicBool>,
        }
        impl RuntimeSubscriber for InspectCompletion {
            fn on_event(&self, event: &RuntimeEventEnvelope) -> RuntimeDecision {
                if let RuntimeEvent::TaskCompleted {
                    task_id,
                    success: true,
                } = event.payload
                {
                    let task = self.registry.get_task(&task_id).unwrap();
                    let dependent = self.registry.get_task(&self.dependent).unwrap();
                    self.seen.store(
                        task.state == TaskState::Completed
                            && task.result.as_deref() == Some("finished")
                            && dependent.state == TaskState::Ready,
                        Ordering::SeqCst,
                    );
                }
                RuntimeDecision::Continue
            }
        }
        let bus = RuntimeBus::new();
        let registry = TaskRegistry::with_bus(bus.clone());
        let run = RunId::new();
        let id = registry
            .create_task(TaskRecord::new(run, "parent"))
            .unwrap();
        let dependent = registry
            .create_task(TaskRecord::new(run, "child").with_dependencies(vec![id]))
            .unwrap();
        let seen = Arc::new(std::sync::atomic::AtomicBool::new(false));
        bus.subscribe(Arc::new(InspectCompletion {
            registry: registry.clone(),
            dependent,
            seen: seen.clone(),
        }));
        registry.complete_task(id, Some("finished".into())).unwrap();
        assert!(seen.load(Ordering::SeqCst));
    }

    impl RuntimeSubscriber for EventCapture {
        fn on_event(&self, event: &RuntimeEventEnvelope) -> RuntimeDecision {
            self.events.lock().unwrap().push(event.payload.clone());
            RuntimeDecision::Continue
        }
    }

    #[test]
    fn test_task_creation_and_ready_derivation() {
        let registry = TaskRegistry::new();
        let run_id = RunId::new();

        // Task A has no dependencies: immediately Ready
        let task_a = TaskRecord::new(run_id, "Task A");
        let id_a = registry.create_task(task_a).unwrap();
        assert_eq!(registry.get_task(&id_a).unwrap().state, TaskState::Ready);

        // Task B depends on Task A: initial state is Pending
        let task_b = TaskRecord::new(run_id, "Task B").with_dependencies(vec![id_a]);
        let id_b = registry.create_task(task_b).unwrap();
        assert_eq!(registry.get_task(&id_b).unwrap().state, TaskState::Pending);

        let ready = registry.get_ready_tasks(Some(run_id));
        assert_eq!(ready.len(), 1);
        assert_eq!(ready[0].id, id_a);
    }

    #[test]
    fn test_self_dependency_rejected() {
        let registry = TaskRegistry::new();
        let run_id = RunId::new();
        let mut task = TaskRecord::new(run_id, "Self dependent");
        task.dependencies = vec![task.id];

        let err = registry.create_task(task).unwrap_err();
        assert!(matches!(err, TaskError::SelfDependency(_)));
    }

    #[test]
    fn test_unknown_dependency_rejected() {
        let registry = TaskRegistry::new();
        let run_id = RunId::new();
        let missing_id = TaskId::new();
        let task =
            TaskRecord::new(run_id, "Task with missing dep").with_dependencies(vec![missing_id]);

        let err = registry.create_task(task).unwrap_err();
        assert_eq!(err, TaskError::UnknownDependency(missing_id));
    }

    #[test]
    fn test_cycle_detection_rejected() {
        let registry = TaskRegistry::new();
        let run_id = RunId::new();

        let id_1 = registry.create_task(TaskRecord::new(run_id, "T1")).unwrap();
        let id_2 = registry
            .create_task(TaskRecord::new(run_id, "T2").with_dependencies(vec![id_1]))
            .unwrap();
        let id_3 = registry
            .create_task(TaskRecord::new(run_id, "T3").with_dependencies(vec![id_2]))
            .unwrap();

        assert!(path_exists(&registry.tasks.read().unwrap(), id_3, id_1));
        assert!(!path_exists(&registry.tasks.read().unwrap(), id_1, id_3));
    }

    #[test]
    fn test_dependency_completion_cascades_to_ready() {
        let registry = TaskRegistry::new();
        let run_id = RunId::new();

        let id_a = registry
            .create_task(TaskRecord::new(run_id, "Task A"))
            .unwrap();
        let id_b = registry
            .create_task(TaskRecord::new(run_id, "Task B"))
            .unwrap();
        let id_c = registry
            .create_task(TaskRecord::new(run_id, "Task C").with_dependencies(vec![id_a, id_b]))
            .unwrap();

        assert_eq!(registry.get_task(&id_c).unwrap().state, TaskState::Pending);

        // Complete A: C remains Pending because B is not completed yet
        registry
            .complete_task(id_a, Some("result A".into()))
            .unwrap();
        assert_eq!(registry.get_task(&id_c).unwrap().state, TaskState::Pending);

        // Complete B: C cascades to Ready!
        registry
            .complete_task(id_b, Some("result B".into()))
            .unwrap();
        assert_eq!(registry.get_task(&id_c).unwrap().state, TaskState::Ready);
    }

    #[test]
    fn test_dependency_failure_cascades_to_blocked() {
        let registry = TaskRegistry::new();
        let run_id = RunId::new();

        let id_a = registry
            .create_task(TaskRecord::new(run_id, "Task A"))
            .unwrap();
        let id_b = registry
            .create_task(TaskRecord::new(run_id, "Task B").with_dependencies(vec![id_a]))
            .unwrap();
        let id_c = registry
            .create_task(TaskRecord::new(run_id, "Task C").with_dependencies(vec![id_b]))
            .unwrap();

        // Fail A: both B and C should cascade to Blocked
        registry
            .fail_task(id_a, Some("Build failed with exit code 1".into()))
            .unwrap();

        assert_eq!(registry.get_task(&id_a).unwrap().state, TaskState::Failed);
        assert_eq!(registry.get_task(&id_b).unwrap().state, TaskState::Blocked);
        assert_eq!(registry.get_task(&id_c).unwrap().state, TaskState::Blocked);
    }

    #[test]
    fn test_task_assignment_emits_event() {
        let bus = RuntimeBus::new();
        let events = Arc::new(Mutex::new(Vec::new()));
        bus.subscribe(Arc::new(EventCapture {
            events: Arc::clone(&events),
        }));

        let registry = TaskRegistry::with_bus(bus);
        let run_id = RunId::new();
        let agent_id = AgentId::new();

        let task_id = registry
            .create_task(TaskRecord::new(run_id, "Task Assigned Test"))
            .unwrap();
        registry.assign_task(task_id, agent_id).unwrap();

        let captured = events.lock().unwrap();
        assert!(captured
            .iter()
            .any(|e| matches!(e, RuntimeEvent::TaskCreated { task_id: t, .. } if *t == task_id)));
        assert!(captured.iter().any(|e| matches!(e, RuntimeEvent::TaskAssigned { task_id: t, agent_id: a } if *t == task_id && *a == agent_id)));

        let task = registry.get_task(&task_id).unwrap();
        assert_eq!(task.assigned_to, Some(agent_id));
        assert_eq!(task.state, TaskState::Running);
    }

    #[test]
    fn test_completion_decision_gate_accept_and_deny() {
        let bus = RuntimeBus::new();
        let denied_id = TaskId::new();
        bus.subscribe(Arc::new(DenyCompletionSubscriber {
            denied_task_id: denied_id,
        }));

        let registry = TaskRegistry::with_bus(bus);
        let run_id = RunId::new();

        // 1. Task that gets denied by verification policy
        let mut denied_task = TaskRecord::new(run_id, "Must pass lint check");
        denied_task.id = denied_id;
        registry.create_task(denied_task).unwrap();

        let err = registry
            .complete_task(denied_id, Some("done".into()))
            .unwrap_err();
        assert!(
            matches!(err, TaskError::CompletionRefused(ref reason) if reason.contains("Verification checks failed"))
        );
        // Task state must NOT be Completed
        assert_eq!(
            registry.get_task(&denied_id).unwrap().state,
            TaskState::Ready
        );

        // 2. Task that gets accepted
        let allowed_id = registry
            .create_task(TaskRecord::new(run_id, "Allowed task"))
            .unwrap();
        registry
            .complete_task(allowed_id, Some("success".into()))
            .unwrap();
        assert_eq!(
            registry.get_task(&allowed_id).unwrap().state,
            TaskState::Completed
        );
    }

    #[test]
    fn test_terminal_task_immutable() {
        let registry = TaskRegistry::new();
        let run_id = RunId::new();
        let agent_id = AgentId::new();

        let task_id = registry
            .create_task(TaskRecord::new(run_id, "Task Terminal"))
            .unwrap();
        registry
            .complete_task(task_id, Some("all done".into()))
            .unwrap();

        // Attempting to assign or re-complete a terminal task returns TerminalTask error
        let err_assign = registry.assign_task(task_id, agent_id).unwrap_err();
        assert!(matches!(err_assign, TaskError::TerminalTask { .. }));

        let err_complete = registry.complete_task(task_id, None).unwrap_err();
        assert!(matches!(err_complete, TaskError::TerminalTask { .. }));
    }

    #[test]
    fn test_rehydrate_tasks_running_marked_failed_and_cascades_blocked() {
        let registry = TaskRegistry::new();
        let run_id = RunId::new();
        let agent_id = AgentId::new();

        let t1_id = TaskId::new();
        let t2_id = TaskId::new();
        let t3_id = TaskId::new();

        let mut t1 = TaskRecord::new(run_id, "T1 (Running at crash)");
        t1.id = t1_id;
        t1.state = TaskState::Running;
        t1.assigned_to = Some(agent_id);

        let mut t2 = TaskRecord::new(run_id, "T2 (Depends on T1)");
        t2.id = t2_id;
        t2.dependencies = vec![t1_id];
        t2.state = TaskState::Pending;

        let mut t3 = TaskRecord::new(run_id, "T3 (Completed)");
        t3.id = t3_id;
        t3.state = TaskState::Completed;

        let events = vec![
            RuntimeEventEnvelope::new(
                1,
                run_id,
                None,
                Some(agent_id),
                None,
                RuntimeEvent::TaskCreated {
                    task_id: t1_id,
                    record: Some(t1),
                },
            ),
            RuntimeEventEnvelope::new(
                2,
                run_id,
                None,
                None,
                None,
                RuntimeEvent::TaskCreated {
                    task_id: t2_id,
                    record: Some(t2),
                },
            ),
            RuntimeEventEnvelope::new(
                3,
                run_id,
                None,
                None,
                None,
                RuntimeEvent::TaskCreated {
                    task_id: t3_id,
                    record: Some(t3),
                },
            ),
            RuntimeEventEnvelope::new(
                4,
                run_id,
                None,
                None,
                None,
                RuntimeEvent::TaskCompleted {
                    task_id: t3_id,
                    success: true,
                },
            ),
        ];

        registry.rehydrate_from_events(&events).unwrap();

        // T1 was running at process crash: rehydrates as Failed with "process_terminated"
        let loaded_t1 = registry.get_task(&t1_id).unwrap();
        assert_eq!(loaded_t1.state, TaskState::Failed);
        assert_eq!(loaded_t1.result.as_deref(), Some("process_terminated"));

        // T2 depended on T1: cascaded to Blocked
        let loaded_t2 = registry.get_task(&t2_id).unwrap();
        assert_eq!(loaded_t2.state, TaskState::Blocked);

        // T3 was Completed: stays Completed
        let loaded_t3 = registry.get_task(&t3_id).unwrap();
        assert_eq!(loaded_t3.state, TaskState::Completed);
    }

    #[test]
    fn test_task_attach_evidence() {
        let registry = TaskRegistry::new();
        let run_id = RunId::new();
        let task_id = registry
            .create_task(TaskRecord::new(run_id, "Test task"))
            .unwrap();
        let ev1 = EvidenceId::new();
        let ev2 = EvidenceId::new();

        registry.attach_evidence(task_id, ev1).unwrap();
        registry.attach_evidence(task_id, ev2).unwrap();
        // Attaching same evidence again is idempotent
        registry.attach_evidence(task_id, ev1).unwrap();

        let updated = registry.get_task(&task_id).unwrap();
        assert_eq!(updated.evidence_refs, vec![ev1, ev2]);
    }

    #[test]
    fn test_complete_task_with_evaluation_and_stale_source() {
        let registry = TaskRegistry::new();
        let run_id = RunId::new();
        let task_id = registry
            .create_task(TaskRecord::new(run_id, "Verified task"))
            .unwrap();

        let dir = tempfile::tempdir().unwrap();
        let m1 = crate::runtime::source_manifest::SourceManifestBuilder::new(dir.path())
            .build()
            .unwrap();

        let task = registry.get_task(&task_id).unwrap();
        let receipt = crate::runtime::evidence_store::ExecutionReceipt {
            receipt_id: crate::runtime::ids::EvidenceId::new(),
            operation_id: "impl_1".into(),
            task_id: Some(task_id),
            tool_name: "implementation".into(),
            started: true,
            exit_code: Some(0),
            ..Default::default()
        };
        let eval =
            crate::runtime::completion::evaluate_task_completion(&task, None, &[receipt], &m1, 0);
        assert!(eval.allowed);

        // Source changed after evaluation
        let mut m2 = m1.clone();
        m2.digest = "new_digest_after_edit".into();

        let err = registry
            .complete_task_with_evaluation(task_id, &eval, &m2, None, None)
            .unwrap_err();
        assert!(matches!(err, TaskError::CompletionRefused(_)));

        // With matching source manifest digest, completion succeeds
        let completed = registry
            .complete_task_with_evaluation(task_id, &eval, &m1, None, None)
            .unwrap();
        assert_eq!(completed.state, TaskState::Completed);
    }

    #[test]
    fn f03_completion_rechecks_evaluation_revision() {
        let registry = TaskRegistry::new();
        let run_id = RunId::new();
        let task_id = registry
            .create_task(TaskRecord::new(run_id, "Revision-raced task"))
            .unwrap();

        let dir = tempfile::tempdir().unwrap();
        let manifest = crate::runtime::source_manifest::SourceManifestBuilder::new(dir.path())
            .build()
            .unwrap();
        let task = registry.get_task(&task_id).unwrap();
        let receipt = crate::runtime::evidence_store::ExecutionReceipt {
            receipt_id: crate::runtime::ids::EvidenceId::new(),
            operation_id: "impl_revision_race".into(),
            task_id: Some(task_id),
            tool_name: "implementation".into(),
            started: true,
            exit_code: Some(0),
            ..Default::default()
        };
        let eval = crate::runtime::completion::evaluate_task_completion(
            &task,
            None,
            &[receipt],
            &manifest,
            0,
        );
        assert!(eval.allowed);

        registry
            .attach_evidence(task_id, crate::runtime::ids::EvidenceId::new())
            .unwrap();

        let err = registry
            .complete_task_with_evaluation(task_id, &eval, &manifest, None, None)
            .unwrap_err();
        assert!(
            matches!(err, TaskError::CompletionRefused(message) if message.contains("task changed"))
        );
        assert_ne!(
            registry.get_task(&task_id).unwrap().state,
            TaskState::Completed
        );
    }

    #[test]
    fn f05_completion_rejects_mismatched_contract_evaluation() {
        let registry = TaskRegistry::new();
        let run_id = RunId::new();
        let base = TaskRecord::new(run_id, "Contract-bound task");
        let contract = crate::runtime::contracts::TaskContract::new(
            "completion-contract",
            1,
            base.id,
            1,
            vec!["src/**".into()],
            vec![],
            false,
            vec![],
            vec![],
            vec![],
        )
        .unwrap();
        let task_id = registry
            .create_task(base.with_contract_digest(contract.digest.clone()))
            .unwrap();
        let dir = tempfile::tempdir().unwrap();
        let manifest = crate::runtime::source_manifest::SourceManifestBuilder::new(dir.path())
            .build()
            .unwrap();
        let task = registry.get_task(&task_id).unwrap();
        let receipt = crate::runtime::evidence_store::ExecutionReceipt {
            receipt_id: crate::runtime::ids::EvidenceId::new(),
            operation_id: "impl_contract_mismatch".into(),
            task_id: Some(task_id),
            tool_name: "implementation".into(),
            started: true,
            exit_code: Some(0),
            ..Default::default()
        };
        let evaluation = crate::runtime::completion::evaluate_task_completion(
            &task,
            None,
            &[receipt],
            &manifest,
            0,
        );
        assert!(evaluation.allowed);

        let err = registry
            .complete_task_with_evaluation(task_id, &evaluation, &manifest, None, None)
            .unwrap_err();
        assert!(matches!(
            err,
            TaskError::CompletionRefused(message)
                if message.contains("contract changed")
        ));
    }

    #[test]
    fn f02_deferred_decision_keeps_task_blocked() {
        let registry = TaskRegistry::new();
        let run_id = RunId::new();
        let mut plan = crate::LivingPlan {
            revision: 1,
            ..Default::default()
        };
        let q = crate::decisions::DecisionQuestion {
            id: "dec_1".into(),
            kind: crate::decisions::DecisionKind::Scope,
            title: "Database choice".into(),
            question: "Choose database".into(),
            materiality: "Material".into(),
            evidence_refs: Vec::new(),
            evidence_fingerprints: std::collections::BTreeMap::new(),
            options: Vec::new(),
            allow_custom: true,
            custom_only: true,
            plan_revision: 1,
            state: crate::decisions::DecisionState::Deferred,
            answer: None,
        };
        plan.structured_decisions.insert("dec_1".into(), q);

        let dir = tempfile::tempdir().unwrap();
        let prereq = DecisionPrerequisiteRef::new("dec_1", 1);
        let task = TaskRecord::new(run_id, "Deferred prereq task")
            .with_decision_prerequisites(vec![prereq]);
        let task_id = registry.create_task(task).unwrap();

        let created = registry.get_task(&task_id).unwrap();
        assert_eq!(created.state, TaskState::Blocked);
        assert_eq!(
            created.blocked_reasons[0].code,
            "decision_prerequisites_unverified"
        );

        let updated = registry
            .evaluate_and_update_decision_prerequisites(task_id, &plan, dir.path())
            .unwrap();
        assert_eq!(updated.state, TaskState::Blocked);
        assert_eq!(updated.blocked_reasons.len(), 1);
        assert_eq!(updated.blocked_reasons[0].code, "unresolved_decision");

        let err = registry.complete_task(task_id, None).unwrap_err();
        assert!(matches!(
            err,
            TaskError::CompletionRefused(message)
                if message.contains("decision prerequisites are unresolved")
        ));
    }

    #[test]
    fn f02_stale_evidence_keeps_decision_and_task_blocked() {
        let registry = TaskRegistry::new();
        let run_id = RunId::new();
        let dir = tempfile::tempdir().unwrap();
        let file_path = dir.path().join("spec.md");
        std::fs::write(&file_path, "initial spec content").unwrap();

        let mut plan = crate::LivingPlan {
            revision: 0,
            goal: "Build feature".into(),
            steps: vec![crate::living_plan::PlanStep {
                id: "step_1".into(),
                change: "implement spec".into(),
                files: vec!["spec.md".into()],
                why: "foundation".into(),
                depends_on: vec![],
                verify: vec!["check".into()],
            }],
            ..Default::default()
        };
        plan.update(
            &serde_json::json!({
                "expected_revision": 0,
                "goal": "Build feature",
                "evidence": [{
                    "path": "spec.md",
                    "finding": "spec content",
                }],
                "steps": [plan.steps[0].clone()],
            }),
            dir.path(),
        )
        .unwrap();

        let fingerprint = plan.evidence[0].fingerprint.clone();
        let mut fingerprints = std::collections::BTreeMap::new();
        fingerprints.insert("spec.md".into(), fingerprint);

        let q = crate::decisions::DecisionQuestion {
            id: "dec_1".into(),
            kind: crate::decisions::DecisionKind::Architecture,
            title: "Architecture".into(),
            question: "Architecture choice".into(),
            materiality: "Material".into(),
            evidence_refs: vec!["spec.md".into()],
            evidence_fingerprints: fingerprints,
            options: Vec::new(),
            allow_custom: true,
            custom_only: true,
            plan_revision: plan.revision,
            state: crate::decisions::DecisionState::AnsweredByUser,
            answer: Some(crate::decisions::DecisionAnswer {
                source: crate::decisions::DecisionAnswerSource::UserInteraction,
                choice_id: None,
                custom_text: Some("Use architecture A".into()),
                answered_at_ms: 1000,
                host_event_id: "evt_1".into(),
            }),
        };
        plan.structured_decisions.insert("dec_1".into(), q);

        let prereq = DecisionPrerequisiteRef::new("dec_1", plan.revision);
        let task = TaskRecord::new(run_id, "Task with evidence prereq")
            .with_decision_prerequisites(vec![prereq]);
        let task_id = registry.create_task(task).unwrap();

        // 1. Evidence matches: task evaluates to Ready
        let ready_task = registry
            .evaluate_and_update_decision_prerequisites(task_id, &plan, dir.path())
            .unwrap();
        assert_eq!(ready_task.state, TaskState::Ready);
        assert!(ready_task.blocked_reasons.is_empty());

        // 2. Modify evidence on disk: evidence becomes stale
        std::fs::write(
            &file_path,
            "modified spec content which invalidates fingerprint",
        )
        .unwrap();
        let blocked_task = registry
            .evaluate_and_update_decision_prerequisites(task_id, &plan, dir.path())
            .unwrap();
        assert_eq!(blocked_task.state, TaskState::Blocked);
        assert_eq!(blocked_task.blocked_reasons.len(), 1);
        assert_eq!(blocked_task.blocked_reasons[0].code, "stale_evidence");

        // The user's answer is NOT modified or erased!
        let dec = plan.structured_decisions.get("dec_1").unwrap();
        assert_eq!(dec.state, crate::decisions::DecisionState::AnsweredByUser);
        assert!(dec.answer.is_some());
    }
}
