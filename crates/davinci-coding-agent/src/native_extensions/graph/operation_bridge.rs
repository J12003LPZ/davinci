//! Durable bridge between graph worker attempts and the runtime operation journal.
//!
//! A graph checkpoint is a projection.  The operation journal is the durable
//! execution boundary that owns launch identity, effect uncertainty, and the
//! worker result.  This module records the immutable link between those two
//! records before a child process is started and writes the result before the
//! graph projection is updated.

use super::store::{atomic_write, now_ms, run_dir};
use super::types::WorkerResult;
use super::worker_sessions::WorkerSessionBinding;
use davinci_agent::runtime::operations::{
    AdmittedOperation, AuthorizationReceipt, CallerType, EffectClass, EffectProfile,
    ExecutionOwnerId, GraphRunBinding, IdempotencyScope, JournalError, OperationAttempt,
    OperationContext, OperationEvent, OperationId, OperationKind, OperationSpec, OperationState,
    PayloadDigest, ResultId, ScopedIdempotencyKey, Timestamp, ToolOperationRuntime,
    WorkspaceIdentity,
};
use davinci_agent::{AgentId, RunId, RuntimeHandle, ToolResult};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::path::{Path, PathBuf};

pub const GRAPH_OPERATION_BRIDGE_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum GraphOperationBridgeError {
    #[error("graph worker operation runtime is not configured")]
    MissingRuntime,
    #[error("graph worker binding is invalid: {0}")]
    InvalidBinding(String),
    #[error("graph worker parent authority changed")]
    ParentAuthorityChanged,
    #[error("graph worker uses an independent operation coordinator")]
    IndependentCoordinator,
    #[error("operation journal error: {0}")]
    Journal(#[from] JournalError),
    #[error("operation model error: {0}")]
    Model(String),
    #[error("graph operation bridge persistence failed: {0}")]
    Persistence(#[from] std::io::Error),
    #[error("graph operation bridge encoding failed: {0}")]
    Encoding(String),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GraphOperationBinding {
    pub schema_version: u32,
    pub graph_run_id: String,
    pub runtime_run_id: RunId,
    pub graph_task_id: String,
    pub launch_operation_id: OperationId,
    pub launch_attempt_id: davinci_agent::runtime::operations::AttemptId,
    pub parent_agent_id: AgentId,
    pub child_agent_id: AgentId,
    pub child_session_id: String,
    pub owner_id: ExecutionOwnerId,
    pub owner_generation: u64,
    pub workspace: WorkspaceIdentity,
    pub parent_operation_id: Option<OperationId>,
    pub contract_digest: Option<String>,
    pub intent_digest: PayloadDigest,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GraphOperationResult {
    pub schema_version: u32,
    pub binding: GraphOperationBinding,
    pub state: OperationState,
    pub result_id: Option<ResultId>,
    pub payload_digest: Option<PayloadDigest>,
    pub artifact_digest: Option<String>,
    pub published_to_outbox: bool,
    pub completed_at: u64,
}

#[derive(Debug, Clone)]
pub struct GraphOperationLaunch {
    pub binding: GraphOperationBinding,
    pub operation: AdmittedOperation,
}

fn safe_component(value: &str, label: &str) -> Result<(), GraphOperationBridgeError> {
    if value.is_empty()
        || value.len() > 160
        || !value
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return Err(GraphOperationBridgeError::InvalidBinding(format!(
            "{label} contains unsafe path characters"
        )));
    }
    Ok(())
}

fn binding_path(cwd: &Path, binding: &GraphOperationBinding) -> PathBuf {
    run_dir(cwd, &binding.graph_run_id)
        .join("artifacts")
        .join(format!(
            "{}.attempt_{}.operation.json",
            binding.graph_task_id, binding.launch_attempt_id
        ))
}

fn result_path(cwd: &Path, binding: &GraphOperationBinding) -> PathBuf {
    run_dir(cwd, &binding.graph_run_id)
        .join("artifacts")
        .join(format!(
            "{}.attempt_{}.operation-result.json",
            binding.graph_task_id, binding.launch_attempt_id
        ))
}

fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, GraphOperationBridgeError> {
    serde_json::to_vec_pretty(value)
        .map_err(|error| GraphOperationBridgeError::Encoding(error.to_string()))
}

fn operation_payload(
    graph_run_id: &str,
    task_id: &str,
    attempt: u32,
    worker: &WorkerSessionBinding,
    contract_digest: Option<&str>,
) -> serde_json::Value {
    json!({
        "graphRunId": graph_run_id,
        "graphTaskId": task_id,
        "attempt": attempt,
        "runtimeRunId": worker.runtime_run,
        "parentAgentId": worker.parent,
        "childAgentId": worker.agent,
        "childSessionId": worker.session_id,
        "contractDigest": contract_digest,
    })
}

fn operation_context(
    operations: &ToolOperationRuntime,
    runtime: &RuntimeHandle,
    graph_run_id: &str,
    task_id: &str,
    worker: &WorkerSessionBinding,
) -> Result<OperationContext, GraphOperationBridgeError> {
    if worker.runtime_run != runtime.run_id || worker.parent != runtime.agent_id {
        return Err(GraphOperationBridgeError::ParentAuthorityChanged);
    }
    operations
        .validate_workspace(&worker.cwd)
        .map_err(GraphOperationBridgeError::InvalidBinding)?;
    let mut context =
        operations.context_for(worker.runtime_run, worker.agent, Some(&worker.session_id));
    context.caller = CallerType::GraphWorker;
    context.graph = Some(GraphRunBinding {
        graph_run_id: graph_run_id.to_owned(),
        graph_task_id: Some(task_id.to_owned()),
    });
    context.task_id = None;
    context.wire_tool_call_id = Some(format!(
        "graph-worker:{graph_run_id}:{task_id}:{}",
        worker.attempt
    ));
    if context.runtime_run_id != worker.runtime_run
        || context.agent_id != worker.agent
        || context
            .graph
            .as_ref()
            .and_then(|graph| graph.graph_task_id.as_deref())
            != Some(task_id)
    {
        return Err(GraphOperationBridgeError::ParentAuthorityChanged);
    }
    Ok(context)
}

fn ensure_queued(
    operations: &ToolOperationRuntime,
    admitted: &mut AdmittedOperation,
) -> Result<(), GraphOperationBridgeError> {
    let owner = operations.dispatcher().owner();
    if admitted.attempt.state() == OperationState::Persisted {
        admitted.attempt = operations.dispatcher().journal().transition(
            owner,
            admitted.attempt.attempt_id(),
            admitted.attempt.revision(),
            OperationEvent::Authorize(AuthorizationReceipt::new(
                admitted
                    .spec
                    .intent_digest()
                    .map_err(|error| GraphOperationBridgeError::Model(error.to_string()))?,
                "graph-worker".to_owned(),
                Timestamp::from_unix_millis(now_ms()),
            )),
            None,
            vec![],
        )?;
    }
    if admitted.attempt.state() == OperationState::Authorized {
        admitted.attempt = operations.dispatcher().journal().transition(
            owner,
            admitted.attempt.attempt_id(),
            admitted.attempt.revision(),
            OperationEvent::Queue,
            None,
            vec![],
        )?;
    }
    Ok(())
}

fn latch_worker_effect(
    operations: &ToolOperationRuntime,
    admitted: &AdmittedOperation,
) -> Result<(), GraphOperationBridgeError> {
    if admitted.attempt.state() != OperationState::Queued {
        return Ok(());
    }
    let dispatcher = operations.dispatcher();
    let claim = dispatcher.journal().claim_dispatch(
        dispatcher.owner(),
        admitted.attempt.attempt_id(),
        admitted.attempt.revision(),
    )?;
    let permit = dispatcher
        .journal()
        .latch_effect_start(claim, Timestamp::from_unix_millis(now_ms()))?;
    permit
        .dispatch(dispatcher.journal(), || ())
        .map_err(GraphOperationBridgeError::Journal)?;
    Ok(())
}

fn binding_from_operation(
    operation: &AdmittedOperation,
    runtime: &RuntimeHandle,
    worker: &WorkerSessionBinding,
    graph_run_id: &str,
    task_id: &str,
) -> Result<GraphOperationBinding, GraphOperationBridgeError> {
    let context = operation.spec.context();
    if context.caller != CallerType::GraphWorker
        || context.runtime_run_id != worker.runtime_run
        || context.agent_id != worker.agent
        || context
            .graph
            .as_ref()
            .map(|graph| graph.graph_run_id.as_str())
            != Some(graph_run_id)
        || context
            .graph
            .as_ref()
            .and_then(|graph| graph.graph_task_id.as_deref())
            != Some(task_id)
        || runtime.agent_id != worker.parent
    {
        return Err(GraphOperationBridgeError::ParentAuthorityChanged);
    }
    let workspace = operation.spec.context().workspace.clone();
    Ok(GraphOperationBinding {
        schema_version: GRAPH_OPERATION_BRIDGE_SCHEMA_VERSION,
        graph_run_id: graph_run_id.to_owned(),
        runtime_run_id: worker.runtime_run,
        graph_task_id: task_id.to_owned(),
        launch_operation_id: operation.spec.operation_id(),
        launch_attempt_id: operation.attempt.attempt_id(),
        parent_agent_id: worker.parent,
        child_agent_id: worker.agent,
        child_session_id: worker.session_id.clone(),
        owner_id: operation.attempt.owner().id,
        owner_generation: operation.attempt.owner().generation,
        workspace,
        parent_operation_id: context.parent_operation_id,
        contract_digest: worker.contract_digest.clone(),
        intent_digest: operation
            .spec
            .intent_digest()
            .map_err(|error| GraphOperationBridgeError::Model(error.to_string()))?,
    })
}

/// Admit and durably latch one graph worker launch.  The returned binding is
/// the only authority used by completion and projection replay.
pub fn launch_worker(
    runtime: &RuntimeHandle,
    cwd: &Path,
    graph_run_id: &str,
    task_id: &str,
    attempt: u32,
    worker: &WorkerSessionBinding,
    contract_digest: Option<&str>,
) -> Result<GraphOperationLaunch, GraphOperationBridgeError> {
    if attempt == 0 || worker.attempt != attempt {
        return Err(GraphOperationBridgeError::InvalidBinding(
            "attempt identity does not match worker session".into(),
        ));
    }
    if !super::store::is_safe_run_id(graph_run_id) || worker.graph_run != graph_run_id {
        return Err(GraphOperationBridgeError::InvalidBinding(
            "graph run identity does not match worker session".into(),
        ));
    }
    safe_component(task_id, "graph task id")?;
    if worker.task_id != task_id
        || worker.cwd
            != cwd
                .canonicalize()
                .map_err(|error| GraphOperationBridgeError::InvalidBinding(error.to_string()))?
    {
        return Err(GraphOperationBridgeError::InvalidBinding(
            "worker task or workspace does not match launch".into(),
        ));
    }
    let operations = runtime
        .operations
        .as_ref()
        .ok_or(GraphOperationBridgeError::MissingRuntime)?;
    let context = operation_context(operations, runtime, graph_run_id, task_id, worker)?;
    let payload = operation_payload(graph_run_id, task_id, attempt, worker, contract_digest);
    let key = ScopedIdempotencyKey::new(
        IdempotencyScope::WorkerLaunch,
        format!("graph-worker:{graph_run_id}:{task_id}:{attempt}"),
    )
    .map_err(|error| GraphOperationBridgeError::Model(error.to_string()))?;
    let spec = OperationSpec::new(
        context,
        key,
        OperationKind::GraphWorkerLaunch,
        EffectProfile {
            classification: EffectClass::ProcessMutation,
            supports_idempotency_key: true,
            supports_postcondition_probe: true,
            supports_compensation: false,
            requires_live_owner: true,
        },
        payload,
        vec![],
    )
    .map_err(|error| GraphOperationBridgeError::Model(error.to_string()))?;
    let initial = OperationAttempt::new(spec.operation_id(), 1, operations.dispatcher().owner())
        .map_err(|error| GraphOperationBridgeError::Model(error.to_string()))?;
    let mut operation = match operations.dispatcher().journal().admit(&spec, &initial)? {
        davinci_agent::runtime::operations::OperationAdmission::New(mut admitted) => {
            ensure_queued(operations, &mut admitted)?;
            admitted
        }
        davinci_agent::runtime::operations::OperationAdmission::ExistingInFlight(mut admitted) => {
            if admitted
                .spec
                .intent_digest()
                .map_err(|error| GraphOperationBridgeError::Model(error.to_string()))?
                != spec
                    .intent_digest()
                    .map_err(|error| GraphOperationBridgeError::Model(error.to_string()))?
            {
                return Err(GraphOperationBridgeError::InvalidBinding(
                    "duplicate launch intent changed".into(),
                ));
            }
            ensure_queued(operations, &mut admitted)?;
            admitted
        }
        davinci_agent::runtime::operations::OperationAdmission::ExistingResult(admitted) => {
            admitted
        }
        davinci_agent::runtime::operations::OperationAdmission::Collision => {
            return Err(GraphOperationBridgeError::InvalidBinding(
                "worker launch idempotency key collided with another intent".into(),
            ));
        }
    };
    let binding = binding_from_operation(&operation, runtime, worker, graph_run_id, task_id)?;
    if operation.attempt.state() == OperationState::Queued {
        latch_worker_effect(operations, &operation)?;
        operation.attempt = operations
            .dispatcher()
            .journal()
            .load_attempt(operation.attempt.attempt_id())?;
    }
    atomic_write(&binding_path(cwd, &binding), &encode(&binding)?)?;
    Ok(GraphOperationLaunch { binding, operation })
}

fn artifact_digest(result: &WorkerResult) -> Result<Option<String>, GraphOperationBridgeError> {
    result
        .artifact
        .as_ref()
        .map(|artifact| {
            serde_json::to_vec(artifact)
                .map(|bytes| super::replay::compute_input_hash(&String::from_utf8_lossy(&bytes)))
                .map_err(|error| GraphOperationBridgeError::Encoding(error.to_string()))
        })
        .transpose()
}

/// Complete a worker operation and persist its result receipt before the graph
/// checkpoint.  Repeated delivery returns the already-terminal operation.
pub fn complete_worker(
    runtime: &RuntimeHandle,
    cwd: &Path,
    binding: &GraphOperationBinding,
    result: &WorkerResult,
) -> Result<GraphOperationResult, GraphOperationBridgeError> {
    binding.validate()?;
    let operations = runtime
        .operations
        .as_ref()
        .ok_or(GraphOperationBridgeError::MissingRuntime)?;
    if runtime.run_id != binding.runtime_run_id || runtime.agent_id != binding.parent_agent_id {
        return Err(GraphOperationBridgeError::ParentAuthorityChanged);
    }
    operations
        .validate_workspace(cwd)
        .map_err(GraphOperationBridgeError::InvalidBinding)?;
    let journal = operations.dispatcher().journal();
    let spec = journal.load_spec(binding.launch_operation_id)?;
    let attempt = journal.load_attempt(binding.launch_attempt_id)?;
    if attempt.operation_id() != binding.launch_operation_id
        || attempt.owner().id != binding.owner_id
        || attempt.owner().generation != binding.owner_generation
    {
        return Err(GraphOperationBridgeError::InvalidBinding(
            "operation attempt binding does not match journal".into(),
        ));
    }
    if attempt.state().is_terminal() {
        let stored = journal.load_result_for_attempt(attempt.attempt_id())?;
        let receipt = GraphOperationResult {
            schema_version: GRAPH_OPERATION_BRIDGE_SCHEMA_VERSION,
            binding: binding.clone(),
            state: attempt.state(),
            result_id: attempt.result().map(|reference| reference.result_id),
            payload_digest: stored
                .as_ref()
                .map(|result| result.reference.payload_digest),
            artifact_digest: artifact_digest(result)?,
            published_to_outbox: spec.context().wire_tool_call_id.is_some(),
            completed_at: attempt
                .finished_at()
                .map_or_else(now_ms, Timestamp::unix_millis),
        };
        atomic_write(&result_path(cwd, binding), &encode(&receipt)?)?;
        return Ok(receipt);
    }
    if attempt.state() != OperationState::EffectPossible {
        return Err(GraphOperationBridgeError::InvalidBinding(format!(
            "worker operation is not ready for completion: {:?}",
            attempt.state()
        )));
    }
    let tool_result = ToolResult {
        content: result.final_text.clone(),
        is_error: !result.ok,
        details: Some(json!({
            "graphRunId": binding.graph_run_id,
            "graphTaskId": binding.graph_task_id,
            "attempt": binding.launch_attempt_id,
            "exitCode": result.exit_code,
            "timedOut": result.timed_out,
            "runDeadlineExceeded": result.run_deadline_exceeded,
            "recoveryRequired": result.recovery_required,
            "failureReason": result.failure_reason,
            "artifactDigest": artifact_digest(result)?,
        })),
    };
    let admitted = AdmittedOperation {
        spec,
        attempt,
        result: None,
    };
    operations
        .dispatcher()
        .complete_for_session(&admitted, &tool_result, true)
        .map_err(|error| {
            GraphOperationBridgeError::Journal(match error {
                davinci_agent::runtime::operations::ToolOperationDispatchError::Journal(error) => {
                    error
                }
                other => JournalError::Serialization(other.to_string()),
            })
        })?;
    let completed = journal.load_attempt(binding.launch_attempt_id)?;
    let stored = journal.load_result_for_attempt(binding.launch_attempt_id)?;
    let receipt = GraphOperationResult {
        schema_version: GRAPH_OPERATION_BRIDGE_SCHEMA_VERSION,
        binding: binding.clone(),
        state: completed.state(),
        result_id: completed.result().map(|reference| reference.result_id),
        payload_digest: stored
            .as_ref()
            .map(|result| result.reference.payload_digest),
        artifact_digest: artifact_digest(result)?,
        published_to_outbox: true,
        completed_at: completed
            .finished_at()
            .map_or_else(now_ms, Timestamp::unix_millis),
    };
    atomic_write(&result_path(cwd, binding), &encode(&receipt)?)?;
    Ok(receipt)
}

/// Read the child operation result after a graph projection failure.  This
/// path is intentionally observational: it never starts a process or creates
/// another operation.
pub fn replay_projection(
    runtime: &RuntimeHandle,
    cwd: &Path,
    binding: &GraphOperationBinding,
) -> Result<GraphOperationResult, GraphOperationBridgeError> {
    binding.validate()?;
    let operations = runtime
        .operations
        .as_ref()
        .ok_or(GraphOperationBridgeError::MissingRuntime)?;
    if runtime.run_id != binding.runtime_run_id || runtime.agent_id != binding.parent_agent_id {
        return Err(GraphOperationBridgeError::ParentAuthorityChanged);
    }
    let journal = operations.dispatcher().journal();
    let attempt = journal.load_attempt(binding.launch_attempt_id)?;
    if attempt.operation_id() != binding.launch_operation_id {
        return Err(GraphOperationBridgeError::InvalidBinding(
            "projection replay references a different operation".into(),
        ));
    }
    let raw = std::fs::read(result_path(cwd, binding))?;
    let receipt: GraphOperationResult = serde_json::from_slice(&raw)
        .map_err(|error| GraphOperationBridgeError::Encoding(error.to_string()))?;
    if receipt.binding != *binding || receipt.state != attempt.state() {
        return Err(GraphOperationBridgeError::InvalidBinding(
            "persisted graph operation result disagrees with the journal".into(),
        ));
    }
    Ok(receipt)
}

/// Check that a worker runtime retains the parent journal and dispatch owner.
/// A child must use the parent-issued runtime transport; a separately opened
/// journal is never accepted as an equivalent authority.
pub fn validate_child_runtime(
    parent: &RuntimeHandle,
    child: &RuntimeHandle,
) -> Result<(), GraphOperationBridgeError> {
    let parent_operations = parent
        .operations
        .as_ref()
        .ok_or(GraphOperationBridgeError::MissingRuntime)?;
    let child_operations = child
        .operations
        .as_ref()
        .ok_or(GraphOperationBridgeError::MissingRuntime)?;
    if parent.run_id != child.run_id
        || child.agent_id == parent.agent_id
        || !parent_operations.shares_dispatch_authority(child_operations)
    {
        return Err(GraphOperationBridgeError::IndependentCoordinator);
    }
    let context =
        child_operations.context_for(child.run_id, child.agent_id, child.session_id.as_deref());
    if context.runtime_run_id != parent.run_id || context.agent_id != child.agent_id {
        return Err(GraphOperationBridgeError::ParentAuthorityChanged);
    }
    Ok(())
}

impl GraphOperationBinding {
    pub fn validate(&self) -> Result<(), GraphOperationBridgeError> {
        if self.schema_version != GRAPH_OPERATION_BRIDGE_SCHEMA_VERSION
            || self.graph_run_id.is_empty()
            || self.graph_task_id.is_empty()
            || self.child_session_id.is_empty()
            || self.owner_generation == 0
        {
            return Err(GraphOperationBridgeError::InvalidBinding(
                "operation bridge identity is incomplete or unsupported".into(),
            ));
        }
        safe_component(&self.graph_task_id, "graph task id")
    }

    pub fn path(&self, cwd: &Path) -> PathBuf {
        binding_path(cwd, self)
    }

    pub fn result_path(&self, cwd: &Path) -> PathBuf {
        result_path(cwd, self)
    }
}
