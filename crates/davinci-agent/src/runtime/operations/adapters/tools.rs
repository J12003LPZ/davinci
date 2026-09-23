use crate::runtime::operations::{
    AdmittedOperation, AuthorizationReceipt, EffectClass, EffectPermit, EffectStatus,
    ExecutionOwner, FailureReasonCode, FailureSubsystem, JournalError, OperationAdmission,
    OperationAttempt, OperationContext, OperationEvent, OperationFailure, OperationId,
    OperationJournal, OperationState, PayloadDigest, PlannedToolOperation, ResultRef, Timestamp,
    ToolOperationPlanError, ToolOperationPlanner,
};
use crate::runtime::{AgentId, RunId, RuntimeCapability};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, thiserror::Error)]
pub enum ToolOperationDispatchError {
    #[error(transparent)]
    Journal(#[from] JournalError),
    #[error(transparent)]
    Plan(#[from] ToolOperationPlanError),
    #[error("operation was denied before dispatch: {0}")]
    Denied(String),
    #[error("operation is already in flight and cannot be dispatched again")]
    AlreadyInFlight,
    #[error("operation has no replayable successful result")]
    NoReplayableResult,
    #[error("operation runtime context does not match the journal")]
    ContextMismatch,
    #[error("workspace could not be validated: {0}")]
    Workspace(String),
    #[error("operation result could not be encoded: {0}")]
    ResultEncoding(String),
}

/// Journal-backed admission and the single-use effect boundary for tool calls.
#[derive(Clone)]
pub struct ToolOperationDispatcher {
    journal: Arc<OperationJournal>,
    owner: ExecutionOwner,
}

impl ToolOperationDispatcher {
    pub fn new(journal: Arc<OperationJournal>, owner: ExecutionOwner) -> Self {
        Self { journal, owner }
    }

    pub fn journal(&self) -> &Arc<OperationJournal> {
        &self.journal
    }

    pub fn owner(&self) -> ExecutionOwner {
        self.owner
    }

    pub fn shares_dispatch_authority(&self, other: &Self) -> bool {
        self.owner == other.owner && Arc::ptr_eq(&self.journal, &other.journal)
    }

    /// Persist immutable operation intent without granting execution authority.
    /// Policy/permission evaluation happens after this boundary.
    pub fn persist_intent(
        &self,
        plan: PlannedToolOperation,
    ) -> Result<OperationAdmission, ToolOperationDispatchError> {
        let spec = plan.into_spec();
        let attempt = OperationAttempt::new(spec.operation_id(), 1, self.owner)
            .map_err(|error| JournalError::Serialization(error.to_string()))?;
        self.journal.admit(&spec, &attempt).map_err(Into::into)
    }

    /// Attach the current authorization receipt and queue a previously
    /// persisted operation. This is the first point at which the operation can
    /// become eligible for dispatch.
    pub fn authorize_and_queue(
        &self,
        admitted: &AdmittedOperation,
        permission_revision: u64,
    ) -> Result<AdmittedOperation, ToolOperationDispatchError> {
        let mut attempt = self.journal.load_attempt(admitted.attempt.attempt_id())?;
        if attempt.state() != OperationState::Persisted {
            return Err(ToolOperationDispatchError::AlreadyInFlight);
        }
        attempt = self.journal.transition(
            self.owner,
            attempt.attempt_id(),
            attempt.revision(),
            OperationEvent::Authorize(AuthorizationReceipt::new(
                admitted
                    .spec
                    .intent_digest()
                    .map_err(|error| JournalError::Serialization(error.to_string()))?,
                permission_revision.to_string(),
                now(),
            )),
            None,
            vec![],
        )?;
        attempt = self.journal.transition(
            self.owner,
            attempt.attempt_id(),
            attempt.revision(),
            OperationEvent::Queue,
            None,
            vec![],
        )?;
        Ok(AdmittedOperation {
            spec: admitted.spec.clone(),
            attempt,
            result: admitted.result.clone(),
        })
    }

    /// Backward-compatible convenience for callers that already completed
    /// permission evaluation before entering the dispatcher.
    pub fn admit(
        &self,
        plan: PlannedToolOperation,
        permission_revision: u64,
    ) -> Result<OperationAdmission, ToolOperationDispatchError> {
        match self.persist_intent(plan)? {
            OperationAdmission::New(admitted) => Ok(OperationAdmission::New(
                self.authorize_and_queue(&admitted, permission_revision)?,
            )),
            existing => Ok(existing),
        }
    }

    /// Revalidates immutable intent and current authority before claiming the
    /// dispatch and durably latching the possible effect.
    pub fn begin_dispatch(
        &self,
        admitted: &AdmittedOperation,
        current_plan: &PlannedToolOperation,
        permission_revision: Option<u64>,
        cancelled: bool,
        revalidate: impl FnOnce() -> Result<(), String>,
    ) -> Result<EffectPermit, ToolOperationDispatchError> {
        let deny = |reason: String| {
            self.cancel_before_start(admitted, &reason)?;
            Err(ToolOperationDispatchError::Denied(reason))
        };
        if admitted.attempt.state() != OperationState::Queued {
            return Err(ToolOperationDispatchError::AlreadyInFlight);
        }
        let expected_digest = admitted
            .spec
            .intent_digest()
            .map_err(|error| JournalError::Serialization(error.to_string()))?;
        let current_digest = current_plan
            .spec()
            .intent_digest()
            .map_err(|error| JournalError::Serialization(error.to_string()))?;
        if expected_digest != current_digest
            || admitted.spec.context().workspace != current_plan.spec().context().workspace
            || admitted.spec.context().root_namespace_id
                != current_plan.spec().context().root_namespace_id
        {
            return deny("approved tool payload or workspace changed after admission".into());
        }
        let expected_policy_revision = admitted
            .attempt
            .authorization()
            .map(|receipt| receipt.policy_revision.as_str());
        if permission_revision
            .map(|revision| revision.to_string())
            .as_deref()
            != expected_policy_revision
        {
            return deny("permission policy changed after operation admission".into());
        }
        if cancelled {
            return deny("operation was cancelled before dispatch".into());
        }
        if let Err(reason) = revalidate() {
            return deny(reason);
        }
        if admitted.attempt.owner() != self.owner {
            return deny("execution owner changed after operation admission".into());
        }

        let claim = self.journal.claim_dispatch(
            self.owner,
            admitted.attempt.attempt_id(),
            admitted.attempt.revision(),
        )?;
        self.journal
            .latch_effect_start(claim, now())
            .map_err(Into::into)
    }

    pub fn dispatch<T>(
        &self,
        admitted: &AdmittedOperation,
        current_plan: &PlannedToolOperation,
        permission_revision: Option<u64>,
        cancelled: bool,
        revalidate: impl FnOnce() -> Result<(), String>,
        action: impl FnOnce() -> T,
    ) -> Result<T, ToolOperationDispatchError> {
        let permit = self.begin_dispatch(
            admitted,
            current_plan,
            permission_revision,
            cancelled,
            revalidate,
        )?;
        permit
            .dispatch(&self.journal, action)
            .map_err(ToolOperationDispatchError::Journal)
    }

    pub fn cancel_before_start(
        &self,
        admitted: &AdmittedOperation,
        _reason: &str,
    ) -> Result<(), ToolOperationDispatchError> {
        if admitted.attempt.state() != OperationState::Queued {
            return Err(ToolOperationDispatchError::AlreadyInFlight);
        }
        self.journal.transition(
            self.owner,
            admitted.attempt.attempt_id(),
            admitted.attempt.revision(),
            OperationEvent::CancelBeforeStart { finished_at: now() },
            None,
            vec![],
        )?;

        Ok(())
    }

    pub fn complete(
        &self,
        admitted: &AdmittedOperation,
        result: &crate::tools::ToolResult,
    ) -> Result<(), ToolOperationDispatchError> {
        self.complete_for_session(admitted, result, true)
    }

    pub fn complete_for_session(
        &self,
        admitted: &AdmittedOperation,
        result: &crate::tools::ToolResult,
        publish_to_session: bool,
    ) -> Result<(), ToolOperationDispatchError> {
        let attempt = self.journal.load_attempt(admitted.attempt.attempt_id())?;
        let payload = serde_json::to_value(result)
            .map_err(|error| ToolOperationDispatchError::ResultEncoding(error.to_string()))?;
        let payload_digest = PayloadDigest::of_json(&payload)
            .map_err(|error| ToolOperationDispatchError::ResultEncoding(error.to_string()))?;
        let finished_at = now();
        let event = if result.is_error {
            OperationEvent::CompleteFailure {
                failure: OperationFailure::new(
                    FailureReasonCode::Other,
                    FailureSubsystem::Execution,
                ),
                effect_status: EffectStatus::Unknown,
                finished_at,
            }
        } else {
            let effect_status = if admitted.spec.effects().classification == EffectClass::ReadOnly {
                EffectStatus::KnownNoEffect
            } else {
                EffectStatus::Possible
            };
            OperationEvent::CompleteSuccess {
                result: ResultRef::new(payload_digest),
                effect_status,
                finished_at,
            }
        };
        let mut outbox = Vec::new();
        if publish_to_session && admitted.spec.context().wire_tool_call_id.is_some() {
            let ready = super::super::OperationResultReady::new(
                &admitted.spec,
                attempt.attempt_id(),
                payload_digest,
            )
            .map_err(ToolOperationDispatchError::ResultEncoding)?;
            outbox.push(
                ready
                    .into_draft()
                    .map_err(ToolOperationDispatchError::Journal)?,
            );
        }
        self.journal.transition(
            self.owner,
            attempt.attempt_id(),
            attempt.revision(),
            event,
            Some(payload),
            outbox,
        )?;
        Ok(())
    }

    pub fn replay_result(
        &self,
        admitted: &AdmittedOperation,
    ) -> Result<crate::tools::ToolResult, ToolOperationDispatchError> {
        if admitted.attempt.state() != OperationState::Succeeded {
            return Err(ToolOperationDispatchError::NoReplayableResult);
        }
        let payload = admitted
            .result
            .as_ref()
            .map(|result| &result.payload)
            .ok_or(ToolOperationDispatchError::NoReplayableResult)?;
        let mut result: crate::tools::ToolResult = serde_json::from_value(payload.clone())
            .map_err(|error| ToolOperationDispatchError::ResultEncoding(error.to_string()))?;
        let digest = PayloadDigest::of_json(payload)
            .map_err(|error| ToolOperationDispatchError::ResultEncoding(error.to_string()))?;
        let ready = super::super::OperationResultReady::new(
            &admitted.spec,
            admitted.attempt.attempt_id(),
            digest,
        )
        .map_err(ToolOperationDispatchError::ResultEncoding)?;
        let details = result.details.get_or_insert_with(|| serde_json::json!({}));
        if !details.is_object() {
            *details = serde_json::json!({});
        }
        details["replayed_from_operation_journal"] = serde_json::Value::Bool(true);
        details["_operation_publication"] = serde_json::to_value(ready)
            .map_err(|error| ToolOperationDispatchError::ResultEncoding(error.to_string()))?;
        Ok(result)
    }
}

#[derive(Clone)]
pub struct ToolOperationRuntime {
    dispatcher: ToolOperationDispatcher,
    context: OperationContext,
    workspace_root: PathBuf,
}

impl ToolOperationRuntime {
    pub fn new(
        journal: Arc<OperationJournal>,
        context: OperationContext,
        owner: ExecutionOwner,
        workspace_root: &Path,
    ) -> Result<Self, ToolOperationDispatchError> {
        context
            .validate()
            .map_err(|_| ToolOperationDispatchError::ContextMismatch)?;
        let identity = journal.identity();
        if context.journal_id != identity.journal_id
            || context.root_namespace_id != journal.root_namespace_id()
            || context.workspace != identity.workspace
            || !workspace_root.is_absolute()
        {
            return Err(ToolOperationDispatchError::ContextMismatch);
        }
        let workspace_root = std::fs::canonicalize(workspace_root)
            .map_err(|error| ToolOperationDispatchError::Workspace(error.to_string()))?;
        if !workspace_root.is_dir() {
            return Err(ToolOperationDispatchError::ContextMismatch);
        }
        Ok(Self {
            dispatcher: ToolOperationDispatcher::new(journal, owner),
            context,
            workspace_root,
        })
    }

    pub fn dispatcher(&self) -> &ToolOperationDispatcher {
        &self.dispatcher
    }

    pub fn operation_context(&self) -> &OperationContext {
        &self.context
    }

    pub fn shares_dispatch_authority(&self, other: &Self) -> bool {
        self.dispatcher.shares_dispatch_authority(&other.dispatcher)
            && self.workspace_root == other.workspace_root
    }

    pub fn workspace_root(&self) -> &Path {
        &self.workspace_root
    }

    pub fn validate_workspace(&self, cwd: &Path) -> Result<(), String> {
        let current = std::fs::canonicalize(cwd)
            .map_err(|error| format!("workspace could not be resolved: {error}"))?;
        if current != self.workspace_root {
            return Err("tool dispatch workspace changed after operation admission".into());
        }
        Ok(())
    }

    pub fn context_for(
        &self,
        run_id: RunId,
        agent_id: AgentId,
        session_id: Option<&str>,
    ) -> OperationContext {
        let mut context = self.context.clone();
        context.runtime_run_id = run_id;
        context.agent_id = agent_id;
        if let Some(session_id) = session_id {
            context.session_id = session_id.to_owned();
        }
        context
    }

    pub fn for_worker(&self, agent_id: AgentId) -> Self {
        let mut child = self.clone();
        child.context.agent_id = agent_id;
        child.context.worker_id = Some(agent_id.to_string());
        child
    }

    pub fn with_parent_operation(&self, parent: OperationId) -> Self {
        let mut child = self.clone();
        child.context.parent_operation_id = Some(parent);
        child
    }

    pub fn plan_provider_call(
        &self,
        run_id: RunId,
        agent_id: AgentId,
        session_id: Option<&str>,
        call_id: &str,
        tool: &str,
        args: &Value,
        capability: Option<&RuntimeCapability>,
        contract_digest: Option<&str>,
    ) -> Result<PlannedToolOperation, ToolOperationPlanError> {
        ToolOperationPlanner::provider_call(
            self.context_for(run_id, agent_id, session_id),
            call_id,
            tool,
            args,
            capability,
            contract_digest,
        )
    }

    pub fn plan_batch_child(
        &self,
        run_id: RunId,
        agent_id: AgentId,
        session_id: Option<&str>,
        parent: OperationId,
        child_index: usize,
        tool: &str,
        args: &Value,
        capability: Option<&RuntimeCapability>,
        contract_digest: Option<&str>,
    ) -> Result<PlannedToolOperation, ToolOperationPlanError> {
        ToolOperationPlanner::batch_child(
            self.context_for(run_id, agent_id, session_id),
            parent,
            child_index,
            tool,
            args,
            capability,
            contract_digest,
        )
    }
}

fn now() -> Timestamp {
    let milliseconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or_default();
    Timestamp::from_unix_millis(milliseconds)
}
