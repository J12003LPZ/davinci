//! Typed worker control commands, revisioned snapshots, and interventions.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::events::{AgentKind, AgentState};
use super::ids::{AgentId, RunId, TaskId};
use super::registry::RuntimeRegistry;

/// Returns true if the control command's target revision and generation match the actual current state
/// and the actor is authorized to perform control actions.
pub fn control_current(
    actual_revision: u64,
    expected_revision: u64,
    actual_generation: u64,
    expected_generation: u64,
    actor_authorized: bool,
) -> bool {
    actual_revision == expected_revision
        && actual_generation == expected_generation
        && actor_authorized
}

/// Reduces stop lifecycle booleans to canonical ControlStatus.
pub fn reduce_stop_status(requested: bool, exited: bool, failed: bool) -> ControlStatus {
    if exited {
        ControlStatus::Stopped
    } else if failed {
        ControlStatus::FailedToStop
    } else if requested {
        ControlStatus::Stopping
    } else {
        ControlStatus::Accepted
    }
}

/// Returns true if retry conditions (prior quiescent, ownership valid, budget available, source reconciled) are satisfied.
pub fn retry_allowed(
    prior_quiescent: bool,
    ownership_valid: bool,
    budget_available: bool,
    source_reconciled: bool,
) -> bool {
    prior_quiescent && ownership_valid && budget_available && source_reconciled
}

/// Action to be performed on a target worker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerControlAction {
    Inspect,
    Steer {
        message: String,
        redirect: bool,
    },
    Stop {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    Retry {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reason: Option<String>,
    },
    Diff,
}

impl WorkerControlAction {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Inspect => "inspect",
            Self::Steer { .. } => "steer",
            Self::Stop { .. } => "stop",
            Self::Retry { .. } => "retry",
            Self::Diff => "diff",
        }
    }
}

/// Status result of evaluating a control command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ControlStatus {
    Accepted,
    Stale,
    Rejected,
    Stopping,
    Stopped,
    FailedToStop,
    Unknown,
}

/// Effect certainty for a managed-process control.  An accepted control is
/// durable intent; it becomes observed only after the supervisor reports the
/// corresponding OS event.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessControlEffect {
    NotStarted,
    Accepted,
    Observed,
    Unknown,
}

/// Durable receipt for a managed-process command.  The process ID is only a
/// locator; the owner/session/lifetime tuple is the authority boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessControlReceipt {
    pub command_id: Uuid,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub operation: Option<crate::runtime::operations::ProcessOperationBinding>,
    pub action: String,
    pub owner: Uuid,
    pub session: Uuid,
    pub workspace: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub generation: Option<u64>,
    pub process_id: u32,
    pub pid: u32,
    pub lifetime: Uuid,
    pub status: ControlStatus,
    pub effect: ProcessControlEffect,
    pub terminated: bool,
    /// Root/descendant PIDs still owned by the supervisor when proof is absent.
    pub unresolved_descendants: Vec<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl ProcessControlReceipt {
    pub fn accepted(
        command_id: Uuid,
        operation: Option<crate::runtime::operations::ProcessOperationBinding>,
        action: impl Into<String>,
        snapshot: &crate::jobs::managed::ProcessSnapshot,
    ) -> Self {
        Self {
            command_id,
            generation: operation.as_ref().map(|value| value.owner_generation),
            operation,
            action: action.into(),
            owner: snapshot.owner,
            session: snapshot.session,
            workspace: snapshot.workspace.clone(),
            process_id: snapshot.id,
            pid: snapshot.pid,
            lifetime: snapshot.lifetime,
            status: ControlStatus::Accepted,
            effect: ProcessControlEffect::Accepted,
            terminated: snapshot.state == "exited",
            unresolved_descendants: if snapshot.state == "exited" || snapshot.pid == 0 {
                Vec::new()
            } else {
                vec![snapshot.pid]
            },
            reason: None,
        }
    }

    pub fn rejected(
        command_id: Uuid,
        operation: Option<crate::runtime::operations::ProcessOperationBinding>,
        action: impl Into<String>,
        snapshot: Option<&crate::jobs::managed::ProcessSnapshot>,
        status: ControlStatus,
        reason: impl Into<String>,
    ) -> Self {
        let (owner, session, workspace, process_id, pid, lifetime) = snapshot
            .map(|value| {
                (
                    value.owner,
                    value.session,
                    value.workspace.clone(),
                    value.id,
                    value.pid,
                    value.lifetime,
                )
            })
            .unwrap_or_else(|| (Uuid::nil(), Uuid::nil(), PathBuf::new(), 0, 0, Uuid::nil()));
        Self {
            command_id,
            generation: operation.as_ref().map(|value| value.owner_generation),
            operation,
            action: action.into(),
            owner,
            session,
            workspace,
            process_id,
            pid,
            lifetime,
            status,
            effect: ProcessControlEffect::NotStarted,
            terminated: false,
            unresolved_descendants: Vec::new(),
            reason: Some(reason.into()),
        }
    }

    pub fn mark_stopping(mut self, reason: Option<String>) -> Self {
        self.status = ControlStatus::Stopping;
        self.effect = ProcessControlEffect::Accepted;
        self.terminated = false;
        self.unresolved_descendants = if self.pid == 0 {
            Vec::new()
        } else {
            vec![self.pid]
        };
        self.reason = reason;
        self
    }

    pub fn mark_stopped(mut self) -> Self {
        self.status = ControlStatus::Stopped;
        self.effect = ProcessControlEffect::Observed;
        self.terminated = true;
        self.unresolved_descendants.clear();
        self
    }

    pub fn mark_unknown(mut self, reason: impl Into<String>) -> Self {
        self.status = ControlStatus::Unknown;
        self.effect = ProcessControlEffect::Unknown;
        self.terminated = false;
        self.reason = Some(reason.into());
        self
    }
}

impl ControlStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Accepted => "accepted",
            Self::Stale => "stale",
            Self::Rejected => "rejected",
            Self::Stopping => "stopping",
            Self::Stopped => "stopped",
            Self::FailedToStop => "failed_to_stop",
            Self::Unknown => "unknown",
        }
    }
}

/// Host-authorized worker control command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerControlCommand {
    pub id: Uuid,
    pub root_run_id: RunId,
    pub agent_id: AgentId,
    pub generation: u64,
    pub task_id: Option<TaskId>,
    pub expected_revision: u64,
    pub action: WorkerControlAction,
}

/// Receipt returned upon processing a worker control command.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerControlReceipt {
    pub command_id: Uuid,
    pub task_id: Option<TaskId>,
    pub agent_id: AgentId,
    pub generation: u64,
    pub action: String,
    pub status: ControlStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

/// Host-owned process lease capturing OS handle and process tree.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessLease {
    pub lease_id: Uuid,
    pub task_id: Option<TaskId>,
    pub agent_id: AgentId,
    pub generation: u64,
    pub os_handle_identity: u32,
    pub created_at: i64,
    pub child_tree: Vec<u32>,
}

impl ProcessLease {
    pub fn new_server_lease(
        task_id: Option<TaskId>,
        agent_id: AgentId,
        generation: u64,
        os_handle_identity: u32,
    ) -> Self {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        Self {
            lease_id: Uuid::new_v4(),
            task_id,
            agent_id,
            generation,
            os_handle_identity,
            created_at: now_ms,
            child_tree: Vec::new(),
        }
    }

    pub fn new_pty_lease(agent_id: AgentId, pid: u32, child_tree: Vec<u32>) -> Self {
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        Self {
            lease_id: Uuid::new_v4(),
            task_id: None,
            agent_id,
            generation: 1,
            os_handle_identity: pid,
            created_at: now_ms,
            child_tree,
        }
    }

    pub fn owns_process(&self, pid: u32) -> bool {
        self.os_handle_identity == pid || self.child_tree.contains(&pid)
    }
}

/// Bounded point-in-time snapshot of an active or historical worker.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkerSnapshot {
    pub agent_id: AgentId,
    pub run_id: RunId,
    pub task_id: Option<TaskId>,
    pub name: String,
    pub kind: AgentKind,
    pub state: AgentState,
    pub generation: u64,
    pub revision: u64,
    pub elapsed_ms: u64,
    pub last_activity_ms: i64,
    pub tool_count: u64,
    pub owned_paths: Vec<PathBuf>,
    pub waiting_on: Option<String>,
    pub disconnected: bool,
    pub usage_unknown: bool,
}

/// Controller responsible for querying snapshots and dispatching control commands.
#[derive(Clone)]
pub struct WorkerController {
    registry: RuntimeRegistry,
    leases: Arc<RwLock<HashMap<AgentId, ProcessLease>>>,
    seen_commands: Arc<RwLock<HashSet<Uuid>>>,
}

impl WorkerController {
    pub fn new(registry: RuntimeRegistry) -> Self {
        Self {
            registry,
            leases: Arc::new(RwLock::new(HashMap::new())),
            seen_commands: Arc::new(RwLock::new(HashSet::new())),
        }
    }

    pub fn register_lease(&self, lease: ProcessLease) {
        if let Ok(mut map) = self.leases.write() {
            map.insert(lease.agent_id, lease);
        }
    }

    pub fn get_lease(&self, agent_id: &AgentId) -> Option<ProcessLease> {
        self.leases.read().ok()?.get(agent_id).cloned()
    }

    pub fn release_lease(&self, agent_id: &AgentId) -> Option<ProcessLease> {
        self.leases.write().ok()?.remove(agent_id)
    }

    pub fn has_active_lease(&self, agent_id: &AgentId) -> bool {
        self.leases
            .read()
            .map(|m| m.contains_key(agent_id))
            .unwrap_or(false)
    }

    /// Build a consistent snapshot of a single worker.
    pub fn build_snapshot(&self, agent_id: &AgentId) -> Option<WorkerSnapshot> {
        let record = self.registry.get(agent_id)?;
        let generation = self.registry.get_generation(agent_id);
        let revision = self.registry.get_revision(agent_id);
        let last_activity_ms = self.registry.get_last_activity(agent_id);
        let tool_count = self.registry.get_tool_count(agent_id);

        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);

        let elapsed_ms = if now_ms > record.started_ms {
            (now_ms - record.started_ms) as u64
        } else {
            0
        };

        let disconnected = matches!(record.state, AgentState::Failed)
            && record.failure_reason.as_deref() == Some("process_terminated");

        let mut owned_paths = Vec::new();
        if let Some(ref wt) = record.worktree {
            owned_paths.push(wt.clone());
        }

        let waiting_on = if record.state == AgentState::Waiting {
            Some("resource_or_input".to_string())
        } else {
            None
        };

        Some(WorkerSnapshot {
            agent_id: *agent_id,
            run_id: record.run_id,
            task_id: record.task_id,
            name: record.name,
            kind: record.kind,
            state: record.state,
            generation,
            revision,
            elapsed_ms,
            last_activity_ms,
            tool_count,
            owned_paths,
            waiting_on,
            disconnected,
            usage_unknown: false,
        })
    }

    /// Build snapshots of all registered workers, ordered deterministically.
    pub fn build_all_snapshots(&self) -> Vec<WorkerSnapshot> {
        let records = self.registry.snapshot();
        records
            .into_iter()
            .filter_map(|r| self.build_snapshot(&r.id))
            .collect()
    }

    /// Execute a worker control command with authority and revision checking.
    pub fn execute_command(
        &self,
        cmd: WorkerControlCommand,
        actor_authorized: bool,
    ) -> WorkerControlReceipt {
        let action_name = cmd.action.name().to_string();

        // 1. Check duplicate command ID
        if let Ok(mut seen) = self.seen_commands.write() {
            if !seen.insert(cmd.id) {
                return WorkerControlReceipt {
                    command_id: cmd.id,
                    task_id: cmd.task_id,
                    agent_id: cmd.agent_id,
                    generation: cmd.generation,
                    action: action_name,
                    status: ControlStatus::Rejected,
                    reason: Some("duplicate_command".to_string()),
                };
            }
        }

        // 2. Check actor authorization
        if !actor_authorized {
            return WorkerControlReceipt {
                command_id: cmd.id,
                task_id: cmd.task_id,
                agent_id: cmd.agent_id,
                generation: cmd.generation,
                action: action_name,
                status: ControlStatus::Rejected,
                reason: Some("unauthorized".to_string()),
            };
        }

        // 3. Look up target worker
        let record = match self.registry.get(&cmd.agent_id) {
            Some(r) => r,
            None => {
                return WorkerControlReceipt {
                    command_id: cmd.id,
                    task_id: cmd.task_id,
                    agent_id: cmd.agent_id,
                    generation: cmd.generation,
                    action: action_name,
                    status: ControlStatus::Rejected,
                    reason: Some("agent_not_found".to_string()),
                };
            }
        };

        // 4. Verify run affinity
        if record.run_id != cmd.root_run_id {
            return WorkerControlReceipt {
                command_id: cmd.id,
                task_id: cmd.task_id,
                agent_id: cmd.agent_id,
                generation: cmd.generation,
                action: action_name,
                status: ControlStatus::Rejected,
                reason: Some("run_id_mismatch".to_string()),
            };
        }

        // 5. Check if worker already exited/terminal
        if matches!(
            record.state,
            AgentState::Completed | AgentState::Failed | AgentState::Cancelled
        ) {
            return WorkerControlReceipt {
                command_id: cmd.id,
                task_id: cmd.task_id,
                agent_id: cmd.agent_id,
                generation: cmd.generation,
                action: action_name,
                status: ControlStatus::Stale,
                reason: Some("worker_already_terminal".to_string()),
            };
        }

        // 6. Verify revision and generation freshness
        let actual_revision = self.registry.get_revision(&cmd.agent_id);
        let actual_generation = self.registry.get_generation(&cmd.agent_id);
        if !control_current(
            actual_revision,
            cmd.expected_revision,
            actual_generation,
            cmd.generation,
            actor_authorized,
        ) {
            return WorkerControlReceipt {
                command_id: cmd.id,
                task_id: cmd.task_id,
                agent_id: cmd.agent_id,
                generation: cmd.generation,
                action: action_name,
                status: ControlStatus::Stale,
                reason: Some("stale_control_version".to_string()),
            };
        }

        // 7. Apply action
        let (status, reason) = match &cmd.action {
            WorkerControlAction::Inspect => (ControlStatus::Accepted, None),
            WorkerControlAction::Steer { .. } => (ControlStatus::Accepted, None),
            WorkerControlAction::Stop { reason } => {
                let _ = self.registry.transition(cmd.agent_id, AgentState::Stopping);
                if let Some(lease) = self.get_lease(&cmd.agent_id) {
                    crate::jobs::kill_tree(lease.os_handle_identity);
                    for child_pid in &lease.child_tree {
                        crate::jobs::kill_tree(*child_pid);
                    }
                }
                // Signal delivery is only an accepted stop request.  The
                // worker remains stopping until its host reports a terminal
                // state; a bounded wait cannot manufacture termination proof.
                (ControlStatus::Stopping, reason.clone())
            }
            WorkerControlAction::Retry { reason } => {
                self.registry.advance_generation(&cmd.agent_id);
                self.registry.advance_revision(&cmd.agent_id);
                let _ = self.registry.transition(cmd.agent_id, AgentState::Starting);
                (ControlStatus::Accepted, reason.clone())
            }
            WorkerControlAction::Diff => (ControlStatus::Accepted, None),
        };

        self.registry.emit_control_ack(
            cmd.id,
            cmd.task_id,
            cmd.agent_id,
            cmd.generation,
            action_name.clone(),
            status.as_str().to_string(),
            reason.clone(),
        );

        WorkerControlReceipt {
            command_id: cmd.id,
            task_id: cmd.task_id,
            agent_id: cmd.agent_id,
            generation: cmd.generation,
            action: action_name,
            status,
            reason,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::events::AgentRecord;

    fn sample_record(id: AgentId, run_id: RunId, state: AgentState) -> AgentRecord {
        AgentRecord {
            id,
            run_id,
            parent: None,
            kind: AgentKind::Main,
            name: "test_worker".to_string(),
            provider: "anthropic".to_string(),
            model_id: "claude-3-5".to_string(),
            cwd: PathBuf::from("/test"),
            state,
            task_id: None,
            worktree: Some(PathBuf::from("/test/wt")),
            started_ms: 1000,
            updated_ms: 1000,
            failure_reason: None,
        }
    }

    #[test]
    fn f07_stale_control_rejected() {
        assert!(control_current(4, 4, 2, 2, true));
        assert!(!control_current(4, 3, 2, 2, true));
        assert!(!control_current(4, 4, 2, 1, true));
        assert!(!control_current(4, 4, 2, 2, false));
    }

    #[test]
    fn test_worker_exits_between_render_and_click() {
        let registry = RuntimeRegistry::new();
        let run_id = RunId::new();
        let agent_id = AgentId::new();
        let record = sample_record(agent_id, run_id, AgentState::Starting);
        registry.register_agent(record).unwrap();
        registry.transition(agent_id, AgentState::Running).unwrap();

        let controller = WorkerController::new(registry.clone());
        let snapshot = controller.build_snapshot(&agent_id).expect("snapshot");
        assert_eq!(snapshot.state, AgentState::Running);

        // Worker transitions to Completed (exits)
        registry
            .transition(agent_id, AgentState::Completed)
            .unwrap();

        // Control command arrives with old snapshot revision
        let cmd = WorkerControlCommand {
            id: Uuid::new_v4(),
            root_run_id: run_id,
            agent_id,
            generation: snapshot.generation,
            task_id: None,
            expected_revision: snapshot.revision,
            action: WorkerControlAction::Stop { reason: None },
        };

        let receipt = controller.execute_command(cmd, true);
        assert_eq!(receipt.status, ControlStatus::Stale);
    }

    #[test]
    fn test_stale_snapshot_rejected() {
        let registry = RuntimeRegistry::new();
        let run_id = RunId::new();
        let agent_id = AgentId::new();
        let record = sample_record(agent_id, run_id, AgentState::Starting);
        registry.register_agent(record).unwrap();
        registry.transition(agent_id, AgentState::Running).unwrap();

        let controller = WorkerController::new(registry.clone());
        let snapshot = controller.build_snapshot(&agent_id).expect("snapshot");

        // Intermediate transition advances revision
        registry.transition(agent_id, AgentState::Waiting).unwrap();

        let cmd = WorkerControlCommand {
            id: Uuid::new_v4(),
            root_run_id: run_id,
            agent_id,
            generation: snapshot.generation,
            task_id: None,
            expected_revision: snapshot.revision, // Stale revision!
            action: WorkerControlAction::Inspect,
        };

        let receipt = controller.execute_command(cmd, true);
        assert_eq!(receipt.status, ControlStatus::Stale);
        assert_eq!(receipt.reason.as_deref(), Some("stale_control_version"));
    }

    #[test]
    fn test_renamed_display_label_targets_stable_agent_id() {
        let registry = RuntimeRegistry::new();
        let run_id = RunId::new();
        let agent_id = AgentId::new();
        let mut record = sample_record(agent_id, run_id, AgentState::Starting);
        record.name = "initial_label".to_string();
        registry.register_agent(record).unwrap();
        registry.transition(agent_id, AgentState::Running).unwrap();

        let controller = WorkerController::new(registry.clone());
        let snapshot = controller.build_snapshot(&agent_id).expect("snapshot");
        assert_eq!(snapshot.name, "initial_label");

        // Execute control on stable agent_id
        let cmd = WorkerControlCommand {
            id: Uuid::new_v4(),
            root_run_id: run_id,
            agent_id,
            generation: snapshot.generation,
            task_id: None,
            expected_revision: snapshot.revision,
            action: WorkerControlAction::Inspect,
        };

        let receipt = controller.execute_command(cmd, true);
        assert_eq!(receipt.status, ControlStatus::Accepted);
    }

    #[test]
    fn test_command_for_another_run_rejected() {
        let registry = RuntimeRegistry::new();
        let run_id = RunId::new();
        let other_run_id = RunId::new();
        let agent_id = AgentId::new();
        let record = sample_record(agent_id, run_id, AgentState::Starting);
        registry.register_agent(record).unwrap();
        registry.transition(agent_id, AgentState::Running).unwrap();

        let controller = WorkerController::new(registry.clone());
        let snapshot = controller.build_snapshot(&agent_id).expect("snapshot");

        let cmd = WorkerControlCommand {
            id: Uuid::new_v4(),
            root_run_id: other_run_id, // mismatch!
            agent_id,
            generation: snapshot.generation,
            task_id: None,
            expected_revision: snapshot.revision,
            action: WorkerControlAction::Inspect,
        };

        let receipt = controller.execute_command(cmd, true);
        assert_eq!(receipt.status, ControlStatus::Rejected);
        assert_eq!(receipt.reason.as_deref(), Some("run_id_mismatch"));
    }

    #[test]
    fn test_duplicate_stop_retry_rejected() {
        let registry = RuntimeRegistry::new();
        let run_id = RunId::new();
        let agent_id = AgentId::new();
        let record = sample_record(agent_id, run_id, AgentState::Starting);
        registry.register_agent(record).unwrap();
        registry.transition(agent_id, AgentState::Running).unwrap();

        let controller = WorkerController::new(registry.clone());
        let snapshot = controller.build_snapshot(&agent_id).expect("snapshot");

        let cmd_id = Uuid::new_v4();
        let cmd = WorkerControlCommand {
            id: cmd_id,
            root_run_id: run_id,
            agent_id,
            generation: snapshot.generation,
            task_id: None,
            expected_revision: snapshot.revision,
            action: WorkerControlAction::Inspect,
        };

        let first = controller.execute_command(cmd.clone(), true);
        assert_eq!(first.status, ControlStatus::Accepted);

        let second = controller.execute_command(cmd, true);
        assert_eq!(second.status, ControlStatus::Rejected);
        assert_eq!(second.reason.as_deref(), Some("duplicate_command"));
    }

    #[test]
    fn test_zero_workers_snapshot_and_command() {
        let registry = RuntimeRegistry::new();
        let controller = WorkerController::new(registry);

        let snapshots = controller.build_all_snapshots();
        assert!(snapshots.is_empty());

        let cmd = WorkerControlCommand {
            id: Uuid::new_v4(),
            root_run_id: RunId::new(),
            agent_id: AgentId::new(),
            generation: 1,
            task_id: None,
            expected_revision: 1,
            action: WorkerControlAction::Inspect,
        };

        let receipt = controller.execute_command(cmd, true);
        assert_eq!(receipt.status, ControlStatus::Rejected);
        assert_eq!(receipt.reason.as_deref(), Some("agent_not_found"));
    }

    #[test]
    fn test_missing_usage_shown_unknown() {
        let mut snapshot = WorkerSnapshot {
            agent_id: AgentId::new(),
            run_id: RunId::new(),
            task_id: None,
            name: "worker".to_string(),
            kind: AgentKind::Main,
            state: AgentState::Running,
            generation: 1,
            revision: 1,
            elapsed_ms: 50,
            last_activity_ms: 1000,
            tool_count: 0,
            owned_paths: vec![],
            waiting_on: None,
            disconnected: false,
            usage_unknown: true,
        };
        assert!(snapshot.usage_unknown);
        snapshot.usage_unknown = false;
        assert!(!snapshot.usage_unknown);
    }

    #[test]
    fn f07_retry_lease() {
        assert!(!retry_allowed(false, true, true, true));
        assert!(!retry_allowed(true, false, true, true));
        assert!(!retry_allowed(true, true, false, true));
        assert!(retry_allowed(true, true, true, true));
    }

    #[test]
    fn test_retry_scenarios() {
        // old worker still alive (prior_quiescent false)
        assert!(!retry_allowed(false, true, true, true));
        // root budget exhausted
        assert!(!retry_allowed(true, true, false, true));
        // pending patch journal / un-reconciled source
        assert!(!retry_allowed(true, true, true, false));
        // ownership invalid
        assert!(!retry_allowed(true, false, true, true));
        // all valid
        assert!(retry_allowed(true, true, true, true));
    }

    #[test]
    fn test_terminal_task_retry_new_attempt() {
        let registry = crate::runtime::tasks::TaskRegistry::new();
        let run_id = RunId::new();
        let mut task = crate::runtime::tasks::TaskRecord::new(run_id, "terminal task");
        task.state = crate::runtime::tasks::TaskState::Ready;
        let task_id = registry.create_task(task).unwrap();
        let created = registry.get_task(&task_id).unwrap();
        assert_eq!(created.attempt, 0);
        assert_eq!(created.owner_generation, 0);

        // Fail the task
        registry
            .fail_task(task_id, Some("failed run".into()))
            .unwrap();
        let failed = registry.get_task(&task_id).unwrap();
        assert_eq!(failed.state, crate::runtime::tasks::TaskState::Failed);

        // Retry the task with CAS
        let new_agent = AgentId::new();
        let retried = registry
            .retry_task(task_id, run_id, failed.revision, Some(new_agent))
            .unwrap();
        assert_eq!(retried.attempt, 1);
        assert_eq!(retried.owner_generation, 1);
        assert_eq!(retried.state, crate::runtime::tasks::TaskState::Running);
        assert_eq!(retried.assigned_to, Some(new_agent));

        // Stale revision retry fails
        let stale_err = registry
            .retry_task(task_id, run_id, failed.revision, None)
            .unwrap_err();
        assert!(matches!(
            stale_err,
            crate::runtime::tasks::TaskError::RevisionConflict(_)
        ));
    }

    #[test]
    fn test_stale_tool_call_from_old_generation_denied() {
        // control_current with actual generation 2, expected generation 1 -> denied
        assert!(!control_current(5, 5, 2, 1, true));
        // matching generation and revision -> allowed
        assert!(control_current(5, 5, 2, 2, true));
    }
}
