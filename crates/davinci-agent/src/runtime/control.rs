//! Typed worker control commands, revisioned snapshots, and interventions.

use std::collections::{HashMap, HashSet};
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
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

/// Reason on the record a command writes before it acts (WOR-99).
const IN_FLIGHT_REASON: &str = "control_in_flight";

impl WorkerControlReceipt {
    /// True for the record written before the command acted: the command
    /// is still running, or a crash left its outcome unknown.
    pub fn is_in_flight(&self) -> bool {
        self.status == ControlStatus::Unknown && self.reason.as_deref() == Some(IN_FLIGHT_REASON)
    }
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ControlReceiptFrame {
    command_id: Uuid,
    command_digest: String,
    receipt: WorkerControlReceipt,
}

fn command_digest(command: &WorkerControlCommand) -> Result<String, String> {
    let encoded = serde_json::to_vec(command).map_err(|error| error.to_string())?;
    Ok(format!(
        "{:x}",
        davinci_sys::hex::Lower(&Sha256::digest(encoded))
    ))
}

fn load_control_receipts(
    path: &Path,
) -> Result<HashMap<Uuid, (String, WorkerControlReceipt)>, String> {
    if !path.exists() {
        return Ok(HashMap::new());
    }
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    let mut receipts = HashMap::new();
    let mut offset = 0usize;
    while offset < bytes.len() {
        let newline = bytes[offset..].iter().position(|byte| *byte == b'\n');
        let end = newline.map_or(bytes.len(), |at| offset + at);
        let line = std::str::from_utf8(&bytes[offset..end]).map_err(|error| error.to_string());
        let frame = line.and_then(|line| {
            if line.trim().is_empty() {
                return Ok(None);
            }
            serde_json::from_str::<ControlReceiptFrame>(line)
                .map(Some)
                .map_err(|error| error.to_string())
        });
        match frame {
            Ok(Some(frame)) => {
                if frame.command_id != frame.receipt.command_id {
                    return Err("control receipt ledger command identity mismatch".into());
                }
                receipts.insert(frame.command_id, (frame.command_digest, frame.receipt));
            }
            Ok(None) => {}
            // Appends write the frame and then its newline, so a crash can
            // only leave a partial frame at the very end with no newline.
            // Cut it off so later appends do not fuse onto it; every earlier
            // frame is intact.  Anything else is real corruption.
            Err(_) if newline.is_none() => {
                OpenOptions::new()
                    .write(true)
                    .open(path)
                    .and_then(|file| file.set_len(offset as u64))
                    .map_err(|error| error.to_string())?;
                break;
            }
            Err(error) => return Err(error),
        }
        if newline.is_none() {
            // A complete final frame that lost only its newline: restore it
            // so the next append starts on its own line.
            let mut file = OpenOptions::new()
                .append(true)
                .open(path)
                .map_err(|error| error.to_string())?;
            file.write_all(b"\n").map_err(|error| error.to_string())?;
        }
        offset = end + 1;
    }
    Ok(receipts)
}

/// In-flight claim on a command ID.  Dropped without `commit` it releases the
/// ID, so only commands that actually executed consume it.
struct CommandClaim<'a> {
    seen: &'a RwLock<HashSet<Uuid>>,
    id: Uuid,
    acquired: bool,
    committed: bool,
}

impl<'a> CommandClaim<'a> {
    fn new(seen: &'a RwLock<HashSet<Uuid>>, id: Uuid) -> Self {
        // A poisoned lock cannot prove a duplicate; the receipt ledger check
        // above remains the durable guard.
        let acquired = seen.write().map(|mut set| set.insert(id)).unwrap_or(true);
        Self {
            seen,
            id,
            acquired,
            committed: false,
        }
    }

    fn acquired(&self) -> bool {
        self.acquired
    }

    fn commit(&mut self) {
        self.committed = true;
    }
}

impl Drop for CommandClaim<'_> {
    fn drop(&mut self) {
        if self.acquired && !self.committed {
            if let Ok(mut set) = self.seen.write() {
                set.remove(&self.id);
            }
        }
    }
}

/// Controller responsible for querying snapshots and dispatching control commands.
#[derive(Clone)]
pub struct WorkerController {
    registry: RuntimeRegistry,
    leases: Arc<RwLock<HashMap<AgentId, ProcessLease>>>,
    seen_commands: Arc<RwLock<HashSet<Uuid>>>,
    command_receipts: Arc<RwLock<HashMap<Uuid, (String, WorkerControlReceipt)>>>,
    receipt_path: Option<Arc<PathBuf>>,
}

impl WorkerController {
    pub fn new(registry: RuntimeRegistry) -> Self {
        Self {
            registry,
            leases: Arc::new(RwLock::new(HashMap::new())),
            seen_commands: Arc::new(RwLock::new(HashSet::new())),
            command_receipts: Arc::new(RwLock::new(HashMap::new())),
            receipt_path: None,
        }
    }

    /// Attach a checksummed append-only receipt ledger.  The last frame for a
    /// command ID is authoritative, so reopening the controller preserves
    /// exact duplicate detection and request-digest collision checks.
    pub fn with_receipt_store(mut self, path: impl AsRef<Path>) -> Result<Self, String> {
        let path = path.as_ref().to_path_buf();
        let receipts = load_control_receipts(&path)?;
        self.command_receipts = Arc::new(RwLock::new(receipts));
        self.receipt_path = Some(Arc::new(path));
        Ok(self)
    }

    pub fn receipt_for(&self, command_id: &Uuid) -> Option<WorkerControlReceipt> {
        self.command_receipts
            .read()
            .ok()?
            .get(command_id)
            .map(|(_, receipt)| receipt.clone())
    }

    /// Make `receipt` the command's durable receipt, then its in-memory one.
    /// Memory never runs ahead of the ledger: a receipt that failed to reach
    /// disk is not remembered, so memory and a restarted controller agree
    /// (WOR-99).
    fn remember_receipt(
        &self,
        command: &WorkerControlCommand,
        digest: &str,
        receipt: &WorkerControlReceipt,
    ) -> bool {
        if !self.append_receipt(command, digest, receipt) {
            return false;
        }
        match self.command_receipts.write() {
            Ok(mut receipts) => {
                receipts.insert(command.id, (digest.to_string(), receipt.clone()));
                true
            }
            Err(_) => false,
        }
    }

    fn append_receipt(
        &self,
        command: &WorkerControlCommand,
        digest: &str,
        receipt: &WorkerControlReceipt,
    ) -> bool {
        let Some(path) = &self.receipt_path else {
            return true;
        };
        let frame = ControlReceiptFrame {
            command_id: command.id,
            command_digest: digest.to_string(),
            receipt: receipt.clone(),
        };
        let Ok(encoded) = serde_json::to_vec(&frame) else {
            return false;
        };
        super::append_log::append_record(path.as_ref(), &encoded).is_ok()
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
        let (record, generation, revision) = self.registry.versioned(agent_id)?;
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

        let digest = command_digest(&cmd).unwrap_or_default();
        if let Ok(receipts) = self.command_receipts.read() {
            if let Some((prior_digest, prior_receipt)) = receipts.get(&cmd.id) {
                if prior_digest == &digest {
                    // Replay of an executed command: hand back the original
                    // receipt so a caller that lost the response can tell
                    // what actually happened.
                    return prior_receipt.clone();
                }
                return WorkerControlReceipt {
                    command_id: cmd.id,
                    task_id: cmd.task_id,
                    agent_id: cmd.agent_id,
                    generation: cmd.generation,
                    action: action_name,
                    status: ControlStatus::Rejected,
                    reason: Some("command_id_collision".to_string()),
                };
            }
        }

        // 1. Claim the command ID against concurrent duplicates.  The claim is
        // released on every rejection below, so a command that was refused
        // before it did anything can be retried under the same ID.
        let mut claim = CommandClaim::new(&self.seen_commands, cmd.id);
        if !claim.acquired() {
            // The other call may have written its in-flight record since the
            // check above; answer with it when it has.
            if let Some((prior_digest, prior)) = self
                .command_receipts
                .read()
                .ok()
                .and_then(|receipts| receipts.get(&cmd.id).cloned())
            {
                if prior_digest == digest {
                    return prior;
                }
            }
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

        // 3. Look up target worker. State, generation and revision come from
        // one consistent read, so the checks below judge a single version.
        let (record, actual_generation, actual_revision) =
            match self.registry.versioned(&cmd.agent_id) {
                Some(versioned) => versioned,
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

        // 7. Write ahead. The command is recorded as in flight before it acts,
        // so a crash or a failed final write can never let a retry of the
        // same ID act twice: after a restart the ledger still answers with
        // this record. If even this record cannot be written, nothing has
        // happened yet and the dropped claim frees the ID for a retry.
        let in_flight = WorkerControlReceipt {
            command_id: cmd.id,
            task_id: cmd.task_id,
            agent_id: cmd.agent_id,
            generation: cmd.generation,
            action: action_name.clone(),
            status: ControlStatus::Unknown,
            reason: Some(IN_FLIGHT_REASON.to_string()),
        };
        if !self.remember_receipt(&cmd, &digest, &in_flight) {
            return WorkerControlReceipt {
                status: ControlStatus::Rejected,
                reason: Some("control_receipt_persistence_failed".to_string()),
                ..in_flight
            };
        }
        // A receipt now exists for this ID: it stays consumed, and a repeat
        // gets that receipt back (the in-flight one while this still runs).
        claim.commit();

        // 8. Apply action. A transition the state machine refuses is this
        // command's outcome, recorded like any other (WOR-97, WOR-98).
        let transition_rejected = |error: &dyn std::fmt::Display| {
            (
                ControlStatus::Rejected,
                Some(format!("transition_rejected: {error}")),
            )
        };
        let (status, reason) = match &cmd.action {
            WorkerControlAction::Inspect => (ControlStatus::Accepted, None),
            WorkerControlAction::Steer { .. } => (ControlStatus::Accepted, None),
            WorkerControlAction::Stop { reason } => {
                // A repeated stop under a new ID is still allowed to re-signal
                // a worker that is already stopping.
                if let Err(error) = self.registry.transition(cmd.agent_id, AgentState::Stopping) {
                    let already_stopping = self
                        .registry
                        .get(&cmd.agent_id)
                        .is_some_and(|record| record.state == AgentState::Stopping);
                    if !already_stopping {
                        return self.finish(
                            &cmd,
                            &digest,
                            action_name,
                            transition_rejected(&error),
                        );
                    }
                }
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
                // Verify the transition first: advancing the generation of a
                // worker that does not restart would invalidate its in-flight
                // messages and control handles for nothing.
                match self.registry.transition(cmd.agent_id, AgentState::Starting) {
                    Ok(()) => {
                        self.registry.advance_generation_and_revision(&cmd.agent_id);
                        (ControlStatus::Accepted, reason.clone())
                    }
                    Err(error) => transition_rejected(&error),
                }
            }
            WorkerControlAction::Diff => (ControlStatus::Accepted, None),
        };
        self.finish(&cmd, &digest, action_name, (status, reason))
    }

    /// Record a command's outcome over its in-flight record and acknowledge it.
    fn finish(
        &self,
        cmd: &WorkerControlCommand,
        digest: &str,
        action_name: String,
        (status, reason): (ControlStatus, Option<String>),
    ) -> WorkerControlReceipt {
        let mut receipt = WorkerControlReceipt {
            command_id: cmd.id,
            task_id: cmd.task_id,
            agent_id: cmd.agent_id,
            generation: cmd.generation,
            action: action_name,
            status,
            reason,
        };
        if !self.remember_receipt(cmd, digest, &receipt) {
            // The action ran but its outcome is not durable; the in-flight
            // record stands, and the caller is told the outcome is unknown.
            receipt.status = ControlStatus::Unknown;
            receipt.reason = Some("control_receipt_persistence_failed".to_string());
        }

        self.registry.emit_control_ack(
            cmd.id,
            cmd.task_id,
            cmd.agent_id,
            cmd.generation,
            receipt.action.clone(),
            receipt.status.as_str().to_string(),
            receipt.reason.clone(),
        );

        receipt
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
        assert_eq!(second, first);
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

    fn stop_command(
        run_id: RunId,
        agent_id: AgentId,
        registry: &RuntimeRegistry,
    ) -> WorkerControlCommand {
        WorkerControlCommand {
            id: Uuid::new_v4(),
            root_run_id: run_id,
            agent_id,
            generation: registry.get_generation(&agent_id),
            task_id: None,
            expected_revision: registry.get_revision(&agent_id),
            action: WorkerControlAction::Stop { reason: None },
        }
    }

    /// WOR-99: when the receipt ledger cannot be written, the command must
    /// not act, and the same ID must stay usable once the ledger recovers.
    #[test]
    fn wor99_unwritable_ledger_rejects_the_command_before_it_acts() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = dir.path().join("control.jsonl");
        let registry = RuntimeRegistry::new();
        let run_id = RunId::new();
        let agent_id = AgentId::new();
        registry
            .register_agent(sample_record(agent_id, run_id, AgentState::Running))
            .unwrap();
        let controller = WorkerController::new(registry.clone())
            .with_receipt_store(&ledger)
            .unwrap();
        // A directory where the ledger file belongs makes every append fail.
        std::fs::create_dir(&ledger).unwrap();

        let cmd = stop_command(run_id, agent_id, &registry);
        let receipt = controller.execute_command(cmd.clone(), true);
        assert_eq!(receipt.status, ControlStatus::Rejected);
        assert_eq!(
            receipt.reason.as_deref(),
            Some("control_receipt_persistence_failed")
        );
        assert_eq!(registry.get(&agent_id).unwrap().state, AgentState::Running);
        assert!(controller.receipt_for(&cmd.id).is_none());

        std::fs::remove_dir(&ledger).unwrap();
        let receipt = controller.execute_command(cmd, true);
        assert_eq!(receipt.status, ControlStatus::Stopping);
        assert_eq!(registry.get(&agent_id).unwrap().state, AgentState::Stopping);
    }

    /// WOR-99: a crash after the command acted but before its outcome was
    /// written must not let a restarted controller run it again.
    #[test]
    fn wor99_restart_after_an_unrecorded_outcome_never_repeats_the_command() {
        let dir = tempfile::tempdir().unwrap();
        let ledger = dir.path().join("control.jsonl");
        let registry = RuntimeRegistry::new();
        let run_id = RunId::new();
        let agent_id = AgentId::new();
        registry
            .register_agent(sample_record(agent_id, run_id, AgentState::Running))
            .unwrap();
        let controller = WorkerController::new(registry.clone())
            .with_receipt_store(&ledger)
            .unwrap();
        let cmd = stop_command(run_id, agent_id, &registry);
        assert_eq!(
            controller.execute_command(cmd.clone(), true).status,
            ControlStatus::Stopping
        );

        // The ledger holds the in-flight record, then the outcome. Drop the
        // outcome, as a crash between acting and recording it would.
        let text = std::fs::read_to_string(&ledger).unwrap();
        let frames: Vec<&str> = text.lines().collect();
        assert_eq!(frames.len(), 2, "{text}");
        std::fs::write(&ledger, format!("{}\n", frames[0])).unwrap();

        let restarted = WorkerController::new(registry.clone())
            .with_receipt_store(&ledger)
            .unwrap();
        // The retry is answered from the ledger instead of acting again: the
        // outcome is reported as unknown, for the caller to reconcile.
        let retry = restarted.execute_command(cmd.clone(), true);
        assert_eq!(retry.status, ControlStatus::Unknown);
        assert_eq!(retry.reason.as_deref(), Some("control_in_flight"));
        assert_eq!(restarted.receipt_for(&cmd.id), Some(retry));
        assert_eq!(std::fs::read_to_string(&ledger).unwrap().lines().count(), 1);
    }

    /// WOR-106: a snapshot's state and revision come from one version, even
    /// while the worker keeps changing state.
    #[test]
    fn wor106_snapshot_state_and_revision_are_one_version() {
        let registry = RuntimeRegistry::new();
        let run_id = RunId::new();
        let agent_id = AgentId::new();
        // Registered Running at revision 1; each transition adds one, so
        // Running always pairs with an odd revision and Waiting with an even.
        registry
            .register_agent(sample_record(agent_id, run_id, AgentState::Running))
            .unwrap();
        let controller = WorkerController::new(registry.clone());
        let writer = {
            let registry = registry.clone();
            std::thread::spawn(move || {
                for _ in 0..20_000 {
                    registry.transition(agent_id, AgentState::Waiting).unwrap();
                    registry.transition(agent_id, AgentState::Running).unwrap();
                }
            })
        };
        while !writer.is_finished() {
            let snapshot = controller.build_snapshot(&agent_id).unwrap();
            let odd = snapshot.revision % 2 == 1;
            assert_eq!(
                odd,
                snapshot.state == AgentState::Running,
                "{:?} at revision {}",
                snapshot.state,
                snapshot.revision
            );
        }
        writer.join().unwrap();
    }
}
