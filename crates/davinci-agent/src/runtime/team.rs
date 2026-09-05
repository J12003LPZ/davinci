//! Multi-agent team coordination, task claiming, and teammate lifecycle management.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, RwLock};

use thiserror::Error;

use super::cancellation::CancellationToken;
use super::events::{AgentKind, AgentRecord, AgentState};
use super::ids::{AgentId, RunId, TaskId};
use super::tasks::TaskState;
use super::RuntimeHandle;

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[derive(Debug, Clone)]
pub struct TeamConfig {
    pub max_teammates: usize,
    pub lead_agent_id: AgentId,
    pub run_id: RunId,
}

impl TeamConfig {
    pub const DEFAULT_MAX_TEAMMATES: usize = 4;
    pub const HARD_CAP_TEAMMATES: usize = 8;

    pub fn new(run_id: RunId, lead_agent_id: AgentId) -> Self {
        Self {
            max_teammates: Self::DEFAULT_MAX_TEAMMATES,
            lead_agent_id,
            run_id,
        }
    }

    pub fn with_max_teammates(mut self, max: usize) -> Self {
        self.max_teammates = max.clamp(1, Self::HARD_CAP_TEAMMATES);
        self
    }
}

pub struct TeammateHandle {
    pub agent_id: AgentId,
    pub name: String,
    pub token: CancellationToken,
}

#[derive(Debug, Error, PartialEq, Eq, Clone)]
pub enum TeamError {
    #[error("team mode disabled: set DAVINCI_EXPERIMENTAL_AGENT_TEAMS=1")]
    TeamModeDisabled,
    #[error("maximum teammates exceeded (current: {current}, max: {max})")]
    MaxTeammatesExceeded { current: usize, max: usize },
    #[error("duplicate teammate ID: {0}")]
    DuplicateTeammate(AgentId),
    #[error("teammate not found: {0}")]
    TeammateNotFound(AgentId),
    #[error("task already claimed or not ready: {0}")]
    TaskAlreadyClaimed(TaskId),
    #[error("task error: {0}")]
    TaskError(String),
    #[error("mailbox error: {0}")]
    MailboxError(String),
    #[error("registry error: {0}")]
    RegistryError(String),
}

/// Coordinates an agent team with shared tasks, messaging, and isolated cancellation.
#[derive(Clone)]
pub struct TeamManager {
    config: TeamConfig,
    runtime: RuntimeHandle,
    teammates: Arc<RwLock<HashMap<AgentId, TeammateHandle>>>,
}

impl TeamManager {
    pub fn is_enabled() -> bool {
        std::env::var("DAVINCI_EXPERIMENTAL_AGENT_TEAMS")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false)
    }

    pub fn new(runtime: RuntimeHandle, config: TeamConfig) -> Self {
        Self {
            config,
            runtime,
            teammates: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    /// Register a new teammate under the team lead with an isolated child cancellation token.
    pub fn register_teammate(
        &self,
        name: impl Into<String>,
    ) -> Result<(AgentId, CancellationToken), TeamError> {
        let name = name.into();
        let aid = AgentId::new();

        let child_token = {
            let mut map = self
                .teammates
                .write()
                .map_err(|_| TeamError::RegistryError("Teammates lock poisoned".into()))?;

            if map.len() >= self.config.max_teammates {
                return Err(TeamError::MaxTeammatesExceeded {
                    current: map.len(),
                    max: self.config.max_teammates,
                });
            }

            let token = self.runtime.cancellation_token.child_token();
            map.insert(
                aid,
                TeammateHandle {
                    agent_id: aid,
                    name: name.clone(),
                    token: token.clone(),
                },
            );
            token
        };

        let now = now_ms();
        let record = AgentRecord {
            id: aid,
            run_id: self.config.run_id,
            parent: Some(self.config.lead_agent_id),
            kind: AgentKind::Teammate,
            name,
            provider: "default".into(),
            model_id: "default".into(),
            cwd: PathBuf::from("."),
            state: AgentState::Running,
            task_id: None,
            worktree: None,
            started_ms: now,
            updated_ms: now,
        };

        self.runtime
            .registry
            .register_agent(record)
            .map_err(|e| TeamError::RegistryError(e.to_string()))?;

        Ok((aid, child_token))
    }

    /// Claim a task for a teammate. Only tasks in Ready state with no current assignee can be claimed.
    pub fn claim_task(&self, teammate_id: AgentId, task_id: TaskId) -> Result<(), TeamError> {
        // Verify teammate exists
        {
            let map = self
                .teammates
                .read()
                .map_err(|_| TeamError::RegistryError("Teammates lock poisoned".into()))?;
            if !map.contains_key(&teammate_id) {
                return Err(TeamError::TeammateNotFound(teammate_id));
            }
        }

        // Verify task state
        let task = self
            .runtime
            .task_registry
            .get_task(&task_id)
            .ok_or_else(|| TeamError::TaskError(format!("Task '{task_id}' not found")))?;

        if task.state != TaskState::Ready || task.assigned_to.is_some() {
            return Err(TeamError::TaskAlreadyClaimed(task_id));
        }

        self.runtime
            .task_registry
            .assign_task(task_id, teammate_id)
            .map_err(|e| TeamError::TaskError(e.to_string()))?;

        Ok(())
    }

    /// Complete a task assigned to a teammate and notify the team lead's mailbox.
    pub fn complete_task(
        &self,
        teammate_id: AgentId,
        task_id: TaskId,
        result: Option<String>,
    ) -> Result<(), TeamError> {
        // Verify teammate exists
        {
            let map = self
                .teammates
                .read()
                .map_err(|_| TeamError::RegistryError("Teammates lock poisoned".into()))?;
            if !map.contains_key(&teammate_id) {
                return Err(TeamError::TeammateNotFound(teammate_id));
            }
        }

        self.runtime
            .task_registry
            .complete_task(task_id, result.clone())
            .map_err(|e| TeamError::TaskError(e.to_string()))?;

        // Send completion notice to team lead
        let notice_content = format!(
            "Task {} completed by teammate {}: {}",
            task_id,
            teammate_id,
            result.as_deref().unwrap_or("success")
        );

        self.runtime
            .send_message(self.config.lead_agent_id, notice_content)
            .map_err(|e| TeamError::MailboxError(e.to_string()))?;

        Ok(())
    }

    /// Interrupt an individual teammate without cancelling other teammates or the parent run.
    pub fn interrupt_teammate(&self, teammate_id: AgentId) -> Result<(), TeamError> {
        let map = self
            .teammates
            .read()
            .map_err(|_| TeamError::RegistryError("Teammates lock poisoned".into()))?;

        let handle = map
            .get(&teammate_id)
            .ok_or(TeamError::TeammateNotFound(teammate_id))?;

        handle.token.cancel();

        let _ = self
            .runtime
            .registry
            .transition(teammate_id, AgentState::Cancelled);

        Ok(())
    }

    /// Cancel all teammates and the parent run.
    pub fn cancel_all(&self) {
        self.runtime.cancel();
    }

    pub fn get_teammate_count(&self) -> usize {
        self.teammates.read().map(|m| m.len()).unwrap_or(0)
    }

    pub fn get_teammates(&self) -> Vec<AgentId> {
        self.teammates
            .read()
            .map(|m| m.keys().cloned().collect())
            .unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::bus::RuntimeBus;
    use crate::runtime::tasks::TaskRecord;

    fn setup_team_test(max_teammates: usize) -> (TeamManager, AgentId, RunId) {
        let run_id = RunId::new();
        let lead_id = AgentId::new();
        let bus = RuntimeBus::new();
        let runtime = RuntimeHandle::new(run_id, lead_id, bus);

        // Register lead agent
        let now = now_ms();
        runtime
            .registry
            .register_agent(AgentRecord {
                id: lead_id,
                run_id,
                parent: None,
                kind: AgentKind::Main,
                name: "lead".into(),
                provider: "mock".into(),
                model_id: "mock".into(),
                cwd: PathBuf::from("/"),
                state: AgentState::Running,
                task_id: None,
                worktree: None,
                started_ms: now,
                updated_ms: now,
            })
            .unwrap();

        let config = TeamConfig::new(run_id, lead_id).with_max_teammates(max_teammates);
        let team = TeamManager::new(runtime, config);
        (team, lead_id, run_id)
    }

    #[test]
    fn test_three_teammates_claim_independent_tasks_and_lead_notified() {
        let (team, lead_id, run_id) = setup_team_test(4);

        // Register 3 teammates
        let (t1, _tok1) = team.register_teammate("teammate-1").unwrap();
        let (t2, _tok2) = team.register_teammate("teammate-2").unwrap();
        let (t3, _tok3) = team.register_teammate("teammate-3").unwrap();

        assert_eq!(team.get_teammate_count(), 3);

        // Create 3 tasks
        let id1 = team
            .runtime
            .task_registry
            .create_task(TaskRecord::new(run_id, "Task 1"))
            .unwrap();
        let id2 = team
            .runtime
            .task_registry
            .create_task(TaskRecord::new(run_id, "Task 2"))
            .unwrap();
        let id3 = team
            .runtime
            .task_registry
            .create_task(TaskRecord::new(run_id, "Task 3"))
            .unwrap();

        // Teammates claim their respective tasks
        team.claim_task(t1, id1).unwrap();
        team.claim_task(t2, id2).unwrap();
        team.claim_task(t3, id3).unwrap();

        // Complete each task
        team.complete_task(t1, id1, Some("res 1".into())).unwrap();
        team.complete_task(t2, id2, Some("res 2".into())).unwrap();
        team.complete_task(t3, id3, Some("res 3".into())).unwrap();

        // Check lead's mailbox has all 3 completion notices
        let notices = team.runtime.mailbox.drain(lead_id, 10);
        assert_eq!(notices.len(), 3);
        assert!(notices.iter().any(|m| m.content.contains(&id1.to_string())));
        assert!(notices.iter().any(|m| m.content.contains(&id2.to_string())));
        assert!(notices.iter().any(|m| m.content.contains(&id3.to_string())));
    }

    #[test]
    fn test_duplicate_task_claim_is_rejected_deterministically() {
        let (team, _lead_id, run_id) = setup_team_test(4);

        let (t1, _) = team.register_teammate("worker-1").unwrap();
        let (t2, _) = team.register_teammate("worker-2").unwrap();

        let task_id = team
            .runtime
            .task_registry
            .create_task(TaskRecord::new(run_id, "Shared Task"))
            .unwrap();

        // Worker 1 claims task
        team.claim_task(t1, task_id).unwrap();

        // Worker 2 attempts to claim same task: rejected
        let err = team.claim_task(t2, task_id).unwrap_err();
        assert_eq!(err, TeamError::TaskAlreadyClaimed(task_id));
    }

    #[test]
    fn test_interrupt_one_teammate_does_not_cancel_siblings_and_cancel_all() {
        let (team, _lead_id, _run_id) = setup_team_test(4);

        let (t1, tok1) = team.register_teammate("worker-1").unwrap();
        let (_t2, tok2) = team.register_teammate("worker-2").unwrap();
        let (_t3, tok3) = team.register_teammate("worker-3").unwrap();

        assert!(!tok1.is_cancelled());
        assert!(!tok2.is_cancelled());
        assert!(!tok3.is_cancelled());

        // Interrupt worker 1
        team.interrupt_teammate(t1).unwrap();

        assert!(tok1.is_cancelled(), "Worker 1 token must be cancelled");
        assert!(!tok2.is_cancelled(), "Worker 2 must not be cancelled");
        assert!(!tok3.is_cancelled(), "Worker 3 must not be cancelled");

        // Cancel all via team
        team.cancel_all();

        assert!(tok2.is_cancelled(), "Worker 2 must now be cancelled");
        assert!(tok3.is_cancelled(), "Worker 3 must now be cancelled");
    }

    #[test]
    fn test_max_teammates_enforced() {
        let (team, _lead_id, _run_id) = setup_team_test(2);

        team.register_teammate("w1").unwrap();
        team.register_teammate("w2").unwrap();

        let err = team.register_teammate("w3").unwrap_err();
        assert_eq!(err, TeamError::MaxTeammatesExceeded { current: 2, max: 2 });
    }
}
