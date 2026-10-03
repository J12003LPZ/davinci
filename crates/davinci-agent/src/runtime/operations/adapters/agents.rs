//! Durable ownership for executions which outlive the caller's turn.
//!
//! Subagents, workflow workers, and background jobs all have the same
//! lifecycle: a parent admits a child, the child may continue after the
//! parent turn ends, and a later notice or resume must not create a second
//! child when the original result is already durable.  This adapter keeps
//! that lifecycle on the existing operation journal and deliberately reuses
//! the parent's dispatcher and execution owner.

use super::tools::{ToolOperationDispatchError, ToolOperationRuntime};
use crate::runtime::ids::{AgentId, RunId, TaskId, WorkflowId};
use crate::runtime::operations::{
    AdmittedOperation, CallerType, EffectClass, EffectProfile, ExecutionOwner, IdempotencyScope,
    JournalError, OperationAdmission, OperationContext, OperationId, OperationKind, OperationState,
    PlannedToolOperation, ToolOperationPlanError, ToolOperationPlanner,
};
use crate::runtime::RuntimeHandle;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Stable kind recorded in the operation payload for every child execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChildExecutionKind {
    Subagent,
    WorkflowPhase,
    BackgroundJob,
}

impl ChildExecutionKind {
    fn caller(self) -> CallerType {
        match self {
            Self::Subagent => CallerType::Subagent,
            Self::WorkflowPhase => CallerType::Workflow,
            Self::BackgroundJob => CallerType::BackgroundJob,
        }
    }

    fn operation_kind(self) -> OperationKind {
        match self {
            Self::Subagent => OperationKind::SubagentLaunch,
            Self::WorkflowPhase => OperationKind::WorkflowPhase,
            Self::BackgroundJob => OperationKind::BackgroundJob,
        }
    }
}

/// The host-owned lineage carried into a child.  It is part of the durable
/// intent so a resumed host can tell which parent owns an unresolved child.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChildExecutionContext {
    pub logical_id: String,
    pub parent_run_id: RunId,
    pub parent_agent_id: AgentId,
    pub parent_session_id: Option<String>,
    pub child_run_id: RunId,
    pub child_agent_id: Option<AgentId>,
    pub task_id: Option<TaskId>,
    pub workflow_id: Option<WorkflowId>,
    pub job_id: Option<u32>,
    pub host: String,
}

impl ChildExecutionContext {
    pub fn new(
        logical_id: impl Into<String>,
        parent: &RuntimeHandle,
        child_agent_id: Option<AgentId>,
    ) -> Self {
        Self {
            logical_id: logical_id.into(),
            parent_run_id: parent.run_id,
            parent_agent_id: parent.agent_id,
            parent_session_id: parent.session_id.clone(),
            child_run_id: parent.run_id,
            child_agent_id,
            task_id: None,
            workflow_id: None,
            job_id: None,
            host: "runtime".to_owned(),
        }
    }

    fn validate_against(&self, context: &OperationContext) -> Result<(), AgentOperationError> {
        if self.logical_id.trim().is_empty() {
            return Err(AgentOperationError::InvalidContext(
                "child logical identity is empty".into(),
            ));
        }
        if self.parent_run_id != context.runtime_run_id || self.parent_agent_id != context.agent_id
        {
            return Err(AgentOperationError::ParentMismatch);
        }
        if self.parent_session_id.as_deref() != Some(context.session_id.as_str())
            && self.parent_session_id.is_some()
        {
            return Err(AgentOperationError::ParentMismatch);
        }
        if self.host.trim().is_empty() {
            return Err(AgentOperationError::InvalidContext(
                "child host is empty".into(),
            ));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentLaunchDisposition {
    New,
    ExistingInFlight,
    ExistingResult,
}

/// A durable child admission.  Only a `New` handle may execute a child; an
/// existing result is replayable and an in-flight child is still owned by the
/// original parent generation.
#[derive(Clone)]
pub struct AgentOperationHandle {
    adapter: AgentOperationAdapter,
    pub admitted: AdmittedOperation,
    plan: PlannedToolOperation,
    pub child: ChildExecutionContext,
    pub kind: ChildExecutionKind,
    disposition: AgentLaunchDisposition,
}

impl std::fmt::Debug for AgentOperationHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentOperationHandle")
            .field("operation_id", &self.admitted.spec.operation_id())
            .field("attempt_id", &self.admitted.attempt.attempt_id())
            .field("child", &self.child)
            .field("kind", &self.kind)
            .field("disposition", &self.disposition)
            .finish()
    }
}

impl AgentOperationHandle {
    pub fn operation_id(&self) -> OperationId {
        self.admitted.spec.operation_id()
    }

    pub fn attempt_id(&self) -> crate::runtime::operations::AttemptId {
        self.admitted.attempt.attempt_id()
    }

    pub fn disposition(&self) -> AgentLaunchDisposition {
        self.disposition
    }

    pub fn should_execute(&self) -> bool {
        self.disposition == AgentLaunchDisposition::New
    }

    pub fn replay_result(&self) -> Result<crate::tools::ToolResult, AgentOperationError> {
        if self.disposition != AgentLaunchDisposition::ExistingResult {
            return Err(AgentOperationError::NotReplayable);
        }
        self.adapter
            .runtime
            .dispatcher()
            .replay_result(&self.admitted)
            .map_err(Into::into)
    }

    /// Dispatches the host action under the parent's durable authority and
    /// persists its result before returning to the caller.
    pub fn execute(
        &self,
        cancelled: bool,
        revalidate: impl FnOnce() -> Result<(), String>,
        action: impl FnOnce() -> crate::tools::ToolResult,
    ) -> Result<crate::tools::ToolResult, AgentOperationError> {
        if !self.should_execute() {
            return match self.disposition {
                AgentLaunchDisposition::ExistingResult => self.replay_result(),
                AgentLaunchDisposition::ExistingInFlight => {
                    Err(AgentOperationError::ChildAlreadyActive(self.operation_id()))
                }
                AgentLaunchDisposition::New => unreachable!(),
            };
        }
        let result = self.adapter.runtime.dispatcher().dispatch(
            &self.admitted,
            &self.plan,
            Some(0),
            cancelled,
            revalidate,
            action,
        )?;
        self.adapter
            .runtime
            .dispatcher()
            .complete_for_session(&self.admitted, &result, false)?;
        Ok(result)
    }

    /// Claims and latches a child launch without completing its journal
    /// record. Background jobs use this boundary to durably establish the
    /// process effect first, then attach the eventual job notice as the
    /// operation result.
    pub fn begin<T>(
        &self,
        cancelled: bool,
        revalidate: impl FnOnce() -> Result<(), String>,
        action: impl FnOnce() -> T,
    ) -> Result<T, AgentOperationError> {
        if !self.should_execute() {
            return match self.disposition {
                AgentLaunchDisposition::ExistingInFlight => {
                    Err(AgentOperationError::ChildAlreadyActive(self.operation_id()))
                }
                AgentLaunchDisposition::ExistingResult => Err(AgentOperationError::NotReplayable),
                AgentLaunchDisposition::New => unreachable!(),
            };
        }
        let permit = self.adapter.runtime.dispatcher().begin_dispatch(
            &self.admitted,
            &self.plan,
            Some(0),
            cancelled,
            revalidate,
        )?;
        permit
            .dispatch(self.adapter.runtime.dispatcher().journal(), action)
            .map_err(AgentOperationError::Journal)
    }

    /// Cancels a queued child when its host could not create the underlying
    /// process. A later retry can then admit the same logical child safely.
    pub fn cancel_before_start(&self, reason: &str) -> Result<(), AgentOperationError> {
        if !self.should_execute() {
            return Err(AgentOperationError::ChildAlreadyActive(self.operation_id()));
        }
        self.adapter
            .runtime
            .dispatcher()
            .cancel_before_start(&self.admitted, reason)
            .map_err(Into::into)
    }

    pub fn complete(&self, result: &crate::tools::ToolResult) -> Result<(), AgentOperationError> {
        self.adapter
            .runtime
            .dispatcher()
            .complete_for_session(&self.admitted, result, false)
            .map_err(Into::into)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum AgentOperationError {
    #[error(transparent)]
    Dispatch(#[from] ToolOperationDispatchError),
    #[error(transparent)]
    Plan(#[from] ToolOperationPlanError),
    #[error(transparent)]
    Journal(#[from] JournalError),
    #[error("child execution context does not belong to the active parent")]
    ParentMismatch,
    #[error("invalid child execution context: {0}")]
    InvalidContext(String),
    #[error("child operation {0} is still owned by its original parent")]
    ChildAlreadyActive(OperationId),
    #[error("child operation has no replayable durable result")]
    NotReplayable,
}

#[derive(Clone)]
pub struct AgentOperationAdapter {
    runtime: ToolOperationRuntime,
}

impl std::fmt::Debug for AgentOperationAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("AgentOperationAdapter")
    }
}

impl AgentOperationAdapter {
    pub fn new(runtime: ToolOperationRuntime) -> Self {
        Self { runtime }
    }

    pub fn from_runtime(runtime: &RuntimeHandle) -> Option<Self> {
        runtime.operations.clone().map(Self::new)
    }

    pub fn runtime(&self) -> &ToolOperationRuntime {
        &self.runtime
    }

    pub fn start(
        &self,
        kind: ChildExecutionKind,
        child: ChildExecutionContext,
        payload: Value,
    ) -> Result<AgentOperationHandle, AgentOperationError> {
        child.validate_against(self.runtime.operation_context())?;
        let payload = json!({
            "schema_version": 1,
            "kind": kind,
            "child": child,
            "payload": payload,
        });
        let effects = EffectProfile {
            // The launch record itself has no resource claim.  The child
            // retains the parent's tool scoping and performs its own effect
            // admission, so unrelated read-only workers remain concurrent.
            classification: EffectClass::ReadOnly,
            supports_idempotency_key: true,
            supports_postcondition_probe: false,
            supports_compensation: false,
            requires_live_owner: true,
        };
        let plan = ToolOperationPlanner::managed_execution(
            self.runtime.operation_context().clone(),
            kind.caller(),
            IdempotencyScope::WorkerLaunch,
            &child.logical_id,
            kind.operation_kind(),
            effects,
            payload,
        )?;
        let admission = self.runtime.dispatcher().admit(plan.clone(), 0)?;
        let (admitted, disposition) = match admission {
            OperationAdmission::New(admitted) => (admitted, AgentLaunchDisposition::New),
            OperationAdmission::ExistingInFlight(admitted) => {
                (admitted, AgentLaunchDisposition::ExistingInFlight)
            }
            OperationAdmission::ExistingResult(admitted) => {
                (admitted, AgentLaunchDisposition::ExistingResult)
            }
            OperationAdmission::Collision => {
                return Err(AgentOperationError::Dispatch(
                    ToolOperationDispatchError::Journal(
                        crate::runtime::operations::JournalError::IdempotencyCollision,
                    ),
                ));
            }
        };
        Ok(AgentOperationHandle {
            adapter: self.clone(),
            admitted,
            plan,
            child,
            kind,
            disposition,
        })
    }

    pub fn unresolved_children(&self) -> Result<Vec<UnresolvedChild>, AgentOperationError> {
        let snapshot = self.runtime.dispatcher().journal().snapshot()?;
        let mut unresolved = Vec::new();
        for spec in snapshot.operations {
            if !matches!(
                spec.kind(),
                OperationKind::SubagentLaunch
                    | OperationKind::WorkflowPhase
                    | OperationKind::BackgroundJob
            ) {
                continue;
            }
            let Some(attempt) = snapshot
                .attempts
                .iter()
                .filter(|attempt| attempt.operation_id() == spec.operation_id())
                .max_by_key(|attempt| attempt.attempt_number())
            else {
                continue;
            };
            if attempt.state().is_terminal() {
                continue;
            }
            let child = spec.payload().get("child").cloned().ok_or_else(|| {
                AgentOperationError::InvalidContext("missing child payload".into())
            })?;
            let child: ChildExecutionContext = serde_json::from_value(child)
                .map_err(|error| AgentOperationError::InvalidContext(error.to_string()))?;
            unresolved.push(UnresolvedChild {
                operation_id: spec.operation_id(),
                attempt_id: attempt.attempt_id(),
                kind: spec.kind(),
                child,
                state: attempt.state(),
                owner: attempt.owner(),
            });
        }
        Ok(unresolved)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnresolvedChild {
    pub operation_id: OperationId,
    pub attempt_id: crate::runtime::operations::AttemptId,
    pub kind: OperationKind,
    pub child: ChildExecutionContext,
    pub state: OperationState,
    pub owner: ExecutionOwner,
}
