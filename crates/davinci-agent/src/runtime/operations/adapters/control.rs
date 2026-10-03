//! Durable adapters for task and worker-control commands.
//!
//! Task and worker projections are useful read models, but the operation
//! journal owns admission, effect latching, and recovery.  These adapters
//! keep the domain command ID in the journal payload and reconcile an already
//! committed domain receipt before considering another effect.

use crate::runtime::{
    AgentId, AgentMailbox, MailboxError, RunId, SteeringReceipt, TaskCreateRequest, TaskError,
    TaskId, TaskOwner, TaskRecord, TaskRegistry, TaskState, WorkerControlAction,
    WorkerControlCommand, WorkerControlReceipt, WorkerController,
};
use crate::tools::ToolResult;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use uuid::Uuid;

use super::super::super::operations::{
    AdmittedOperation, AuthorizationReceipt, CallerType, EffectClass, EffectProfile,
    IdempotencyScope, JournalError, OperationAdmission, OperationAttempt, OperationEvent,
    OperationId, OperationKind, OperationSpec, OperationState, ScopedIdempotencyKey, Timestamp,
    ToolOperationDispatchError, ToolOperationRuntime,
};

pub const CONTROL_OPERATION_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum ControlOperationError {
    #[error("operation journal error: {0}")]
    Journal(#[from] JournalError),
    #[error("operation model error: {0}")]
    Model(String),
    #[error("operation result could not be encoded: {0}")]
    Encoding(String),
    #[error("task operation failed: {0}")]
    Task(#[from] TaskError),
    #[error("mailbox operation failed: {0}")]
    Mailbox(#[from] MailboxError),
    #[error("control operation failed: {0}")]
    Domain(String),
    #[error("operation requires recovery evidence before another effect can run")]
    RecoveryRequired,
    #[error("operation idempotency key collided with a different command")]
    Collision,
    #[error("operation result was not a control receipt")]
    InvalidReceipt,
}

#[allow(clippy::large_enum_variant)]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ControlReceiptValue {
    Task(TaskRecord),
    Worker(WorkerControlReceipt),
    Steering(SteeringReceipt),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ControlOperationReceipt {
    pub schema_version: u32,
    pub operation_id: OperationId,
    pub attempt_id: crate::runtime::operations::AttemptId,
    pub command_id: Uuid,
    pub state: OperationState,
    pub value: ControlReceiptValue,
    pub replayed: bool,
}

/// Journal-backed task and worker-control command adapter.
#[derive(Clone)]
pub struct ControlOperationAdapter {
    operations: ToolOperationRuntime,
}

impl ControlOperationAdapter {
    pub fn new(operations: ToolOperationRuntime) -> Self {
        Self { operations }
    }

    pub fn operations(&self) -> &ToolOperationRuntime {
        &self.operations
    }

    pub fn execute_task_create(
        &self,
        registry: &TaskRegistry,
        input: TaskCreateRequest,
        run_id: RunId,
        actor: AgentId,
        command_id: Uuid,
    ) -> Result<ControlOperationReceipt, ControlOperationError> {
        let request = json!({
            "kind": "task_create",
            "commandId": command_id,
            "runId": run_id,
            "actor": actor,
            "input": input,
        });
        let admitted = self.admit(
            run_id,
            actor,
            None,
            CallerType::TaskTransport,
            OperationKind::TaskControl,
            command_id,
            request,
            EffectClass::IdempotentMutation,
        )?;
        self.execute_task_domain(admitted, registry, command_id, || {
            registry
                .create_command(input, run_id, actor, Some(command_id))
                .map_err(ControlOperationError::from)
        })
    }

    pub fn execute_task_claim(
        &self,
        registry: &TaskRegistry,
        task_id: TaskId,
        run_id: RunId,
        actor: AgentId,
        expected_revision: u64,
        command_id: Uuid,
    ) -> Result<ControlOperationReceipt, ControlOperationError> {
        let request = json!({
            "kind": "task_claim",
            "commandId": command_id,
            "taskId": task_id,
            "runId": run_id,
            "actor": actor,
            "expectedRevision": expected_revision,
        });
        let admitted = self.admit(
            run_id,
            actor,
            Some(task_id),
            CallerType::TaskTransport,
            OperationKind::TaskControl,
            command_id,
            request,
            EffectClass::IdempotentMutation,
        )?;
        self.execute_task_domain(admitted, registry, command_id, || {
            registry
                .claim_operation(task_id, run_id, actor, expected_revision, command_id)
                .map_err(ControlOperationError::from)
        })
    }

    pub fn execute_task_status(
        &self,
        registry: &TaskRegistry,
        task_id: TaskId,
        state: TaskState,
        result: Option<String>,
        owner: TaskOwner,
        command_id: Uuid,
    ) -> Result<ControlOperationReceipt, ControlOperationError> {
        let request = json!({
            "kind": "task_status",
            "commandId": command_id,
            "taskId": task_id,
            "state": state,
            "result": result,
            "owner": {
                "runId": owner.run_id,
                "agentId": owner.agent_id,
                "revision": owner.revision,
                "generation": owner.generation,
            },
        });
        let admitted = self.admit(
            owner.run_id,
            owner.agent_id,
            Some(task_id),
            CallerType::TaskTransport,
            OperationKind::TaskControl,
            command_id,
            request,
            EffectClass::IdempotentMutation,
        )?;
        self.execute_task_domain(admitted, registry, command_id, || {
            registry
                .status_operation(task_id, state, result, owner, command_id)
                .map_err(ControlOperationError::from)
        })
    }

    /// Execute a host control command and, for steering, journal the mailbox
    /// acceptance under the same command identity.
    pub fn execute_worker_control(
        &self,
        controller: &WorkerController,
        mailbox: Option<&AgentMailbox>,
        command: WorkerControlCommand,
        actor_authorized: bool,
    ) -> Result<ControlOperationReceipt, ControlOperationError> {
        let request = serde_json::to_value(&command)
            .map_err(|error| ControlOperationError::Encoding(error.to_string()))?;
        let admitted = self.admit(
            command.root_run_id,
            command.agent_id,
            command.task_id,
            CallerType::HostControl,
            OperationKind::GraphControl,
            command.id,
            json!({ "kind": "worker_control", "command": request }),
            match command.action {
                WorkerControlAction::Stop { .. } | WorkerControlAction::Retry { .. } => {
                    EffectClass::ProcessMutation
                }
                WorkerControlAction::Steer { .. } => EffectClass::ExternalMutation,
                WorkerControlAction::Inspect | WorkerControlAction::Diff => EffectClass::ReadOnly,
            },
        )?;

        if let Some(receipt) = self.replay_existing(&admitted, true)? {
            return Ok(receipt);
        }
        if admitted.attempt.state() != OperationState::Succeeded {
            if let Some(worker) = controller.receipt_for(&command.id) {
                let steering = match &command.action {
                    WorkerControlAction::Steer { .. } => {
                        mailbox.and_then(|mailbox| mailbox.get_steering_receipt(&command.id))
                    }
                    _ => None,
                };
                if matches!(&command.action, WorkerControlAction::Steer { .. })
                    && steering.is_none()
                {
                    return Err(ControlOperationError::RecoveryRequired);
                }
                let value = steering
                    .map(ControlReceiptValue::Steering)
                    .unwrap_or_else(|| ControlReceiptValue::Worker(worker));
                let mut receipt = ControlOperationReceipt {
                    schema_version: CONTROL_OPERATION_SCHEMA_VERSION,
                    operation_id: admitted.spec.operation_id(),
                    attempt_id: admitted.attempt.attempt_id(),
                    command_id: command.id,
                    state: admitted.attempt.state(),
                    value,
                    replayed: true,
                };
                if admitted.attempt.state() != OperationState::EffectPossible {
                    self.dispatch_domain(&admitted, || Ok::<_, ControlOperationError>(()))?;
                }
                self.complete_success(&admitted, &mut receipt)?;
                return Ok(receipt);
            }
        }
        if admitted.attempt.state() == OperationState::EffectPossible {
            return Err(ControlOperationError::RecoveryRequired);
        }
        let worker = self.dispatch_domain(&admitted, || {
            Ok::<_, ControlOperationError>(
                controller.execute_command(command.clone(), actor_authorized),
            )
        })?;
        let mut receipt = self.receipt_for_worker(&admitted, command.id, worker, false);
        let steering = match &command.action {
            WorkerControlAction::Steer { message, redirect } => mailbox
                .map(|mailbox| {
                    mailbox.send_steer_with_id(
                        command.id,
                        command.agent_id,
                        command.generation,
                        message.clone(),
                        *redirect,
                    )
                })
                .transpose()?,
            _ => None,
        };
        let worker_value = receipt.value.clone();
        receipt.value = if let Some(steering) = steering {
            ControlReceiptValue::Steering(steering)
        } else {
            worker_value
        };
        self.complete_success(&admitted, &mut receipt)?;
        Ok(receipt)
    }

    fn execute_task_domain(
        &self,
        admitted: AdmittedOperation,
        registry: &TaskRegistry,
        command_id: Uuid,
        action: impl FnOnce() -> Result<TaskRecord, ControlOperationError>,
    ) -> Result<ControlOperationReceipt, ControlOperationError> {
        if let Some(receipt) = registry.operation_receipt(command_id)? {
            let mut result = self.receipt_for_task(&admitted, command_id, receipt.response, true);
            if admitted.attempt.state() != OperationState::Succeeded {
                self.dispatch_domain(&admitted, || Ok::<_, ControlOperationError>(()))?;
                self.complete_success(&admitted, &mut result)?;
            }
            return Ok(result);
        }
        if let Some(receipt) = self.replay_existing(&admitted, true)? {
            return Ok(receipt);
        }
        if admitted.attempt.state() == OperationState::EffectPossible {
            return Err(ControlOperationError::RecoveryRequired);
        }
        let task = self.dispatch_domain(&admitted, action)?;
        let mut receipt = self.receipt_for_task(&admitted, command_id, task, false);
        self.complete_success(&admitted, &mut receipt)?;
        Ok(receipt)
    }

    fn dispatch_domain<T>(
        &self,
        admitted: &AdmittedOperation,
        action: impl FnOnce() -> Result<T, ControlOperationError>,
    ) -> Result<T, ControlOperationError> {
        let mut admitted = admitted.clone();
        self.ensure_queued(&mut admitted)?;
        if admitted.attempt.state() == OperationState::EffectPossible {
            return Err(ControlOperationError::RecoveryRequired);
        }
        let dispatcher = self.operations.dispatcher();
        let claim = dispatcher.journal().claim_dispatch(
            dispatcher.owner(),
            admitted.attempt.attempt_id(),
            admitted.attempt.revision(),
        )?;
        let permit = dispatcher.journal().latch_effect_start(claim, now())?;
        permit
            .dispatch(dispatcher.journal(), action)
            .map_err(ControlOperationError::Journal)?
    }

    fn complete_success(
        &self,
        admitted: &AdmittedOperation,
        receipt: &mut ControlOperationReceipt,
    ) -> Result<(), ControlOperationError> {
        receipt.state = OperationState::Succeeded;
        let result = ToolResult {
            content: "control operation accepted".to_owned(),
            is_error: false,
            details: Some(json!({
                "controlReceipt": receipt,
            })),
        };
        self.operations
            .dispatcher()
            .complete_for_session(admitted, &result, false)
            .map_err(map_dispatch_error)
    }

    fn replay_existing(
        &self,
        admitted: &AdmittedOperation,
        replayed: bool,
    ) -> Result<Option<ControlOperationReceipt>, ControlOperationError> {
        if admitted.attempt.state() != OperationState::Succeeded {
            return Ok(None);
        }
        let result = self
            .operations
            .dispatcher()
            .replay_result(admitted)
            .map_err(map_dispatch_error)?;
        let Some(details) = result.details else {
            return Err(ControlOperationError::InvalidReceipt);
        };
        let Some(value) = details.get("controlReceipt") else {
            return Err(ControlOperationError::InvalidReceipt);
        };
        let mut receipt: ControlOperationReceipt = serde_json::from_value(value.clone())
            .map_err(|error| ControlOperationError::Encoding(error.to_string()))?;
        receipt.replayed = replayed;
        receipt.state = admitted.attempt.state();
        Ok(Some(receipt))
    }

    fn receipt_for_task(
        &self,
        admitted: &AdmittedOperation,
        command_id: Uuid,
        task: TaskRecord,
        replayed: bool,
    ) -> ControlOperationReceipt {
        ControlOperationReceipt {
            schema_version: CONTROL_OPERATION_SCHEMA_VERSION,
            operation_id: admitted.spec.operation_id(),
            attempt_id: admitted.attempt.attempt_id(),
            command_id,
            state: admitted.attempt.state(),
            value: ControlReceiptValue::Task(task),
            replayed,
        }
    }

    fn receipt_for_worker(
        &self,
        admitted: &AdmittedOperation,
        command_id: Uuid,
        worker: WorkerControlReceipt,
        replayed: bool,
    ) -> ControlOperationReceipt {
        ControlOperationReceipt {
            schema_version: CONTROL_OPERATION_SCHEMA_VERSION,
            operation_id: admitted.spec.operation_id(),
            attempt_id: admitted.attempt.attempt_id(),
            command_id,
            state: admitted.attempt.state(),
            value: ControlReceiptValue::Worker(worker),
            replayed,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn admit(
        &self,
        run_id: RunId,
        actor: AgentId,
        task_id: Option<TaskId>,
        caller: CallerType,
        kind: OperationKind,
        command_id: Uuid,
        payload: Value,
        classification: EffectClass,
    ) -> Result<AdmittedOperation, ControlOperationError> {
        let mut context = self.operations.context_for(run_id, actor, None);
        context.caller = caller;
        context.task_id = task_id;
        context.wire_tool_call_id = Some(format!("runtime-control:{command_id}"));
        let scope = if caller == CallerType::HostControl {
            IdempotencyScope::HostControl
        } else {
            IdempotencyScope::CallerDefined
        };
        let key = ScopedIdempotencyKey::new(scope, format!("davinci-control:{command_id}"))
            .map_err(|error| ControlOperationError::Model(error.to_string()))?;
        let spec = OperationSpec::new(
            context,
            key,
            kind,
            EffectProfile {
                classification,
                supports_idempotency_key: true,
                supports_postcondition_probe: true,
                supports_compensation: false,
                requires_live_owner: classification != EffectClass::ReadOnly,
            },
            payload,
            Vec::new(),
        )
        .map_err(|error| ControlOperationError::Model(error.to_string()))?;
        let initial =
            OperationAttempt::new(spec.operation_id(), 1, self.operations.dispatcher().owner())
                .map_err(|error| ControlOperationError::Model(error.to_string()))?;
        match self
            .operations
            .dispatcher()
            .journal()
            .admit(&spec, &initial)?
        {
            OperationAdmission::New(mut admitted)
            | OperationAdmission::ExistingInFlight(mut admitted) => {
                self.ensure_queued(&mut admitted)?;
                Ok(admitted)
            }
            OperationAdmission::ExistingResult(admitted) => Ok(admitted),
            OperationAdmission::Collision => Err(ControlOperationError::Collision),
        }
    }

    fn ensure_queued(&self, admitted: &mut AdmittedOperation) -> Result<(), ControlOperationError> {
        let owner = self.operations.dispatcher().owner();
        let journal = self.operations.dispatcher().journal();
        if admitted.attempt.state() == OperationState::Persisted {
            admitted.attempt = journal.transition(
                owner,
                admitted.attempt.attempt_id(),
                admitted.attempt.revision(),
                OperationEvent::Authorize(AuthorizationReceipt::new(
                    admitted
                        .spec
                        .intent_digest()
                        .map_err(|error| ControlOperationError::Model(error.to_string()))?,
                    "runtime-control".to_owned(),
                    now(),
                )),
                None,
                Vec::new(),
            )?;
        }
        if admitted.attempt.state() == OperationState::Authorized {
            admitted.attempt = journal.transition(
                owner,
                admitted.attempt.attempt_id(),
                admitted.attempt.revision(),
                OperationEvent::Queue,
                None,
                Vec::new(),
            )?;
        }
        Ok(())
    }
}

fn map_dispatch_error(error: ToolOperationDispatchError) -> ControlOperationError {
    match error {
        ToolOperationDispatchError::Journal(error) => ControlOperationError::Journal(error),
        other => ControlOperationError::Domain(other.to_string()),
    }
}

fn now() -> Timestamp {
    let milliseconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis().min(u64::MAX as u128) as u64)
        .unwrap_or_default();
    Timestamp::from_unix_millis(milliseconds)
}
