//! Dependency-aware shared task registry for multi-agent coordination.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use super::bus::RuntimeBus;
use super::events::{RuntimeEvent, RuntimeEventEnvelope};
use super::ids::{AgentId, RunId, TaskId};

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
}

impl TaskRecord {
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

    pub fn with_assigned(mut self, agent_id: AgentId) -> Self {
        self.assigned_to = Some(agent_id);
        self
    }
}

#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum TaskError {
    #[error("task already exists: {0}")]
    DuplicateTask(TaskId),
    #[error("task not found: {0}")]
    TaskNotFound(TaskId),
    #[error("self-dependency is not allowed: {0}")]
    SelfDependency(TaskId),
    #[error("unknown dependency: {0}")]
    UnknownDependency(TaskId),
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

/// In-memory thread-safe registry of shared tasks across coordinating agents.
#[derive(Clone, Default)]
pub struct TaskRegistry {
    tasks: Arc<RwLock<HashMap<TaskId, TaskRecord>>>,
    bus: Option<RuntimeBus>,
    seq: Arc<AtomicU64>,
}

impl TaskRegistry {
    pub fn new() -> Self {
        Self {
            tasks: Arc::new(RwLock::new(HashMap::new())),
            bus: None,
            seq: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn with_bus(bus: RuntimeBus) -> Self {
        Self {
            tasks: Arc::new(RwLock::new(HashMap::new())),
            bus: Some(bus),
            seq: Arc::new(AtomicU64::new(0)),
        }
    }

    /// Create and register a task with dependency validation and initial state derivation.
    pub fn create_task(&self, mut task: TaskRecord) -> Result<TaskId, TaskError> {
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

            if tasks.contains_key(&task_id) {
                return Err(TaskError::DuplicateTask(task_id));
            }

            // 2. Validate all dependencies exist and check for cycles
            for dep_id in &task.dependencies {
                if !tasks.contains_key(dep_id) {
                    return Err(TaskError::UnknownDependency(*dep_id));
                }
                if path_exists(&tasks, *dep_id, task_id) {
                    return Err(TaskError::CycleDetected(task_id));
                }
            }

            // 3. Derive initial state based on dependencies
            let any_failed = task.dependencies.iter().any(|dep_id| {
                if let Some(dep) = tasks.get(dep_id) {
                    matches!(
                        dep.state,
                        TaskState::Failed | TaskState::Blocked | TaskState::Cancelled
                    )
                } else {
                    false
                }
            });

            let all_completed = !task.dependencies.is_empty()
                && task.dependencies.iter().all(|dep_id| {
                    if let Some(dep) = tasks.get(dep_id) {
                        dep.state == TaskState::Completed
                    } else {
                        false
                    }
                });

            let derived_state = if any_failed {
                TaskState::Blocked
            } else if task.dependencies.is_empty() || all_completed {
                TaskState::Ready
            } else {
                TaskState::Pending
            };

            task.state = derived_state;
            task.updated_at_ms = now_ms();
            tasks.insert(task_id, task);
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
                RuntimeEvent::TaskCreated { task_id },
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

        Ok(task_id)
    }

    /// Assign a task to an agent.
    pub fn assign_task(&self, task_id: TaskId, agent_id: AgentId) -> Result<(), TaskError> {
        let (run_id, previous_agent) = {
            let mut tasks = self
                .tasks
                .write()
                .map_err(|_| TaskError::InvalidTransition {
                    task_id,
                    from: TaskState::Ready,
                    to: TaskState::Running,
                })?;

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
            task.assigned_to = Some(agent_id);
            if task.state == TaskState::Ready {
                task.state = TaskState::Running;
            }
            task.updated_at_ms = now_ms();
            (task.run_id, prev)
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

    /// Complete a task, passing through the runtime decision gate.
    pub fn complete_task(&self, task_id: TaskId, result: Option<String>) -> Result<(), TaskError> {
        let (run_id, assigned_to) = {
            let tasks = self
                .tasks
                .read()
                .map_err(|_| TaskError::InvalidTransition {
                    task_id,
                    from: TaskState::Running,
                    to: TaskState::Completed,
                })?;

            let task = tasks
                .get(&task_id)
                .ok_or(TaskError::TaskNotFound(task_id))?;

            if task.state.is_terminal() {
                return Err(TaskError::TerminalTask {
                    task_id,
                    state: task.state,
                });
            }

            if !is_valid_task_transition(task.state, TaskState::Completed) {
                return Err(TaskError::InvalidTransition {
                    task_id,
                    from: task.state,
                    to: TaskState::Completed,
                });
            }

            (task.run_id, task.assigned_to)
        };

        // Decision gate: emit TaskCompleted decision event before committing state
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
                    success: true,
                },
            );
            if let Err(reason) = bus.emit_decision(envelope) {
                return Err(TaskError::CompletionRefused(reason));
            }
        }

        // Commit completion and cascade to dependents
        {
            let mut tasks = self
                .tasks
                .write()
                .map_err(|_| TaskError::InvalidTransition {
                    task_id,
                    from: TaskState::Running,
                    to: TaskState::Completed,
                })?;

            let task = tasks
                .get_mut(&task_id)
                .ok_or(TaskError::TaskNotFound(task_id))?;

            task.state = TaskState::Completed;
            task.result = result;
            task.updated_at_ms = now_ms();

            // Cascade: any Pending dependent whose dependencies are ALL completed transitions to Ready
            let now = now_ms();
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
                                dep_mut.state = TaskState::Ready;
                                dep_mut.updated_at_ms = now;
                            }
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Fail a task and cascade Blocked state to all dependents.
    pub fn fail_task(
        &self,
        task_id: TaskId,
        error_reason: Option<String>,
    ) -> Result<(), TaskError> {
        let (run_id, assigned_to) = {
            let mut tasks = self
                .tasks
                .write()
                .map_err(|_| TaskError::InvalidTransition {
                    task_id,
                    from: TaskState::Running,
                    to: TaskState::Failed,
                })?;

            let task = tasks
                .get_mut(&task_id)
                .ok_or(TaskError::TaskNotFound(task_id))?;

            if task.state.is_terminal() {
                return Err(TaskError::TerminalTask {
                    task_id,
                    state: task.state,
                });
            }

            task.state = TaskState::Failed;
            task.result = error_reason;
            let now = now_ms();
            task.updated_at_ms = now;
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
                        other_task.state = TaskState::Blocked;
                        other_task.updated_at_ms = now;
                        to_block.push(other_task.id);
                    }
                }
            }

            (run_id, assigned_to)
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

        Ok(())
    }

    /// Cancel a task and cascade Blocked state to its dependents.
    pub fn cancel_task(&self, task_id: TaskId) -> Result<(), TaskError> {
        let mut tasks = self
            .tasks
            .write()
            .map_err(|_| TaskError::InvalidTransition {
                task_id,
                from: TaskState::Ready,
                to: TaskState::Cancelled,
            })?;

        let task = tasks
            .get_mut(&task_id)
            .ok_or(TaskError::TaskNotFound(task_id))?;

        if task.state.is_terminal() {
            return Err(TaskError::TerminalTask {
                task_id,
                state: task.state,
            });
        }

        task.state = TaskState::Cancelled;
        let now = now_ms();
        task.updated_at_ms = now;

        // Cascade Blocked state to dependents
        let mut to_block = vec![task_id];
        while let Some(cancelled_id) = to_block.pop() {
            for other_task in tasks.values_mut() {
                if other_task.dependencies.contains(&cancelled_id)
                    && !other_task.state.is_terminal()
                    && other_task.state != TaskState::Blocked
                {
                    other_task.state = TaskState::Blocked;
                    other_task.updated_at_ms = now;
                    to_block.push(other_task.id);
                }
            }
        }

        Ok(())
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
            .filter(|t| run_id.map(|r| t.run_id == r).unwrap_or(true))
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
                t.state == TaskState::Ready && run_id.map(|r| t.run_id == r).unwrap_or(true)
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::bus::{RuntimeDecision, RuntimeSubscriber};
    use std::sync::Mutex;

    struct DenyCompletionSubscriber {
        denied_task_id: TaskId,
    }

    impl RuntimeSubscriber for DenyCompletionSubscriber {
        fn on_event(&self, event: &RuntimeEventEnvelope) -> RuntimeDecision {
            if let RuntimeEvent::TaskCompleted {
                task_id,
                success: true,
            } = event.payload
            {
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
            .any(|e| matches!(e, RuntimeEvent::TaskCreated { task_id: t } if *t == task_id)));
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
}
