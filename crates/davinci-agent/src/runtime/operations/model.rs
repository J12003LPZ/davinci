use super::failure::OperationFailure;
use super::identity::{
    AttemptId, ExecutionOwnerId, IdentityError, JournalId, OperationId, PayloadDigest, ResultId,
    RootNamespaceId, ScopedIdempotencyKey, WorkspaceId,
};
use crate::runtime::{AgentId, EvidenceId, RunId, TaskId};
use serde::de::Error as DeError;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const OPERATION_SCHEMA_VERSION: u32 = 1;

pub type OperationPayload = Value;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceIdentity {
    pub id: WorkspaceId,
    pub binding_version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GraphRunBinding {
    pub graph_run_id: String,
    pub graph_task_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CallerType {
    ProviderToolCall,
    BatchToolCall,
    HostControl,
    GraphWorker,
    Subagent,
    Browser,
    Mcp,
    TaskTransport,
    Verification,
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationContext {
    pub journal_id: JournalId,
    pub root_namespace_id: RootNamespaceId,
    pub session_id: String,
    pub runtime_run_id: RunId,
    pub parent_operation_id: Option<OperationId>,
    pub agent_id: AgentId,
    pub worker_id: Option<String>,
    pub task_id: Option<TaskId>,
    pub graph: Option<GraphRunBinding>,
    pub workspace: WorkspaceIdentity,
    pub caller: CallerType,
    pub wire_tool_call_id: Option<String>,
}

impl OperationContext {
    pub fn validate(&self) -> Result<(), ModelError> {
        if self.session_id.trim().is_empty() {
            return Err(ModelError::EmptyBinding("session_id"));
        }
        if self.workspace.binding_version == 0 {
            return Err(ModelError::InvalidBindingVersion);
        }
        if self
            .worker_id
            .as_deref()
            .is_some_and(|value| value.trim().is_empty())
        {
            return Err(ModelError::EmptyBinding("worker_id"));
        }
        if self
            .graph
            .as_ref()
            .is_some_and(|graph| graph.graph_run_id.trim().is_empty())
        {
            return Err(ModelError::EmptyBinding("graph_run_id"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationKind {
    ToolInvocation,
    ShellCommand,
    FileWrite,
    FileEdit,
    FileDelete,
    FileMove,
    GitMutation,
    ProcessSpawn,
    ProcessStdin,
    ProcessSignal,
    ProcessTermination,
    TransactionApply,
    TransactionRollback,
    TransactionCommitObservation,
    Verification,
    GraphWorkerLaunch,
    SubagentLaunch,
    GraphControl,
    TaskControl,
    BrowserAction,
    McpCall,
    NetworkAction,
    CustomExternalAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectClass {
    ReadOnly,
    IdempotentMutation,
    ReversibleMutation,
    IrreversibleMutation,
    ExternalMutation,
    ProcessMutation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EffectProfile {
    pub classification: EffectClass,
    pub supports_idempotency_key: bool,
    pub supports_postcondition_probe: bool,
    pub supports_compensation: bool,
    pub requires_live_owner: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreconditionKind {
    WorkspaceBinding,
    ResourceVersion,
    ProcessIdentity,
    ParentOperation,
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Precondition {
    pub kind: PreconditionKind,
    pub resource: String,
    pub expected_digest: Option<PayloadDigest>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ModelError {
    #[error("unsupported operation schema version {0}")]
    UnsupportedSchema(u32),
    #[error("operation payload digest does not match its payload")]
    PayloadDigestMismatch,
    #[error(transparent)]
    InvalidIdempotencyKey(#[from] IdentityError),
    #[error("operation payload could not be encoded: {0}")]
    PayloadEncoding(String),
    #[error("required binding is empty: {0}")]
    EmptyBinding(&'static str),
    #[error("workspace binding version must be nonzero")]
    InvalidBindingVersion,
    #[error("attempt number must be nonzero")]
    InvalidAttemptNumber,
    #[error("execution owner generation must be nonzero")]
    InvalidOwnerGeneration,
    #[error("successful attempt must have a result and known effect certainty")]
    InvalidSuccessRecord,
    #[error("failed attempt must have a typed failure")]
    InvalidFailureRecord,
    #[error("superseded attempt must identify its replacement")]
    MissingReplacement,
    #[error("attempt contains conflicting result and failure facts")]
    ConflictingOutcomeFacts,
    #[error("attempt carries outcome facts outside a terminal outcome")]
    OutcomeOnNonterminal,
    #[error("effect_possible state cannot have not_started certainty")]
    InvalidEffectState,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OperationSpec {
    operation_id: OperationId,
    schema_version: u32,
    context: OperationContext,
    caller_key: ScopedIdempotencyKey,
    payload_digest: PayloadDigest,
    kind: OperationKind,
    effects: EffectProfile,
    preconditions: Vec<Precondition>,
    payload: OperationPayload,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OperationSpecWire {
    operation_id: OperationId,
    schema_version: u32,
    context: OperationContext,
    caller_key: ScopedIdempotencyKey,
    payload_digest: PayloadDigest,
    kind: OperationKind,
    effects: EffectProfile,
    preconditions: Vec<Precondition>,
    payload: OperationPayload,
}

impl OperationSpec {
    pub fn new(
        context: OperationContext,
        caller_key: ScopedIdempotencyKey,
        kind: OperationKind,
        effects: EffectProfile,
        payload: OperationPayload,
        preconditions: Vec<Precondition>,
    ) -> Result<Self, ModelError> {
        context.validate()?;
        let payload_digest = PayloadDigest::of_json(&payload)
            .map_err(|error| ModelError::PayloadEncoding(error.to_string()))?;
        Ok(Self {
            operation_id: OperationId::new(),
            schema_version: OPERATION_SCHEMA_VERSION,
            context,
            caller_key,
            payload_digest,
            kind,
            effects,
            preconditions,
            payload,
        })
    }

    pub fn validate(&self) -> Result<(), ModelError> {
        if self.schema_version != OPERATION_SCHEMA_VERSION {
            return Err(ModelError::UnsupportedSchema(self.schema_version));
        }
        self.context.validate()?;
        self.caller_key.validate()?;
        let digest = PayloadDigest::of_json(&self.payload)
            .map_err(|error| ModelError::PayloadEncoding(error.to_string()))?;
        if digest != self.payload_digest {
            return Err(ModelError::PayloadDigestMismatch);
        }
        Ok(())
    }

    pub fn operation_id(&self) -> OperationId {
        self.operation_id
    }

    pub fn schema_version(&self) -> u32 {
        self.schema_version
    }

    pub fn context(&self) -> &OperationContext {
        &self.context
    }

    pub fn caller_key(&self) -> &ScopedIdempotencyKey {
        &self.caller_key
    }

    pub fn payload_digest(&self) -> PayloadDigest {
        self.payload_digest
    }

    /// Digest of the caller's complete operation intent, including its command
    /// kind and stable session/call lineage as well as arguments.
    pub fn intent_digest(&self) -> Result<PayloadDigest, ModelError> {
        let bytes = serde_json::to_vec(&(
            self.context.journal_id,
            self.context.root_namespace_id,
            &self.context.session_id,
            self.context.parent_operation_id,
            self.context.caller,
            &self.context.wire_tool_call_id,
            self.caller_key.scope,
            &self.caller_key.key,
            self.kind,
            self.effects,
            &self.preconditions,
            &self.payload,
        ))
        .map_err(|error| ModelError::PayloadEncoding(error.to_string()))?;
        Ok(PayloadDigest::of_bytes(&bytes))
    }

    pub fn kind(&self) -> OperationKind {
        self.kind
    }

    pub fn effects(&self) -> EffectProfile {
        self.effects
    }

    pub fn preconditions(&self) -> &[Precondition] {
        &self.preconditions
    }

    pub fn payload(&self) -> &OperationPayload {
        &self.payload
    }
}

impl<'de> Deserialize<'de> for OperationSpec {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = OperationSpecWire::deserialize(deserializer)?;
        let spec = Self {
            operation_id: wire.operation_id,
            schema_version: wire.schema_version,
            context: wire.context,
            caller_key: wire.caller_key,
            payload_digest: wire.payload_digest,
            kind: wire.kind,
            effects: wire.effects,
            preconditions: wire.preconditions,
            payload: wire.payload,
        };
        spec.validate().map_err(D::Error::custom)?;
        Ok(spec)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationState {
    Created,
    Persisted,
    Authorized,
    Queued,
    Running,
    EffectPossible,
    Succeeded,
    Failed,
    Cancelled,
    Interrupted,
    RecoveryRequired,
    Blocked,
    Superseded,
}

impl OperationState {
    pub fn is_terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Failed | Self::Cancelled | Self::Blocked | Self::Superseded
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EffectStatus {
    NotStarted,
    Possible,
    EffectsObserved,
    KnownNoEffect,
    Unknown,
    Compensated,
}

impl Default for EffectStatus {
    fn default() -> Self {
        Self::Unknown
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationState {
    NotRequired,
    Pending,
    Running,
    Passed,
    Failed,
    Inconclusive,
}

impl Default for VerificationState {
    fn default() -> Self {
        Self::NotRequired
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicationState {
    Pending,
    Acknowledged,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Timestamp(pub u64);

impl Timestamp {
    pub fn from_unix_millis(value: u64) -> Self {
        Self(value)
    }

    pub fn unix_millis(self) -> u64 {
        self.0
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AuthorizationReceipt {
    pub approved_payload_digest: PayloadDigest,
    pub policy_revision: String,
    pub authorized_at: Timestamp,
}

impl AuthorizationReceipt {
    pub fn new(
        approved_payload_digest: PayloadDigest,
        policy_revision: String,
        authorized_at: Timestamp,
    ) -> Self {
        Self {
            approved_payload_digest,
            policy_revision,
            authorized_at,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionOwner {
    pub id: ExecutionOwnerId,
    pub generation: u64,
}

impl ExecutionOwner {
    pub fn new(id: ExecutionOwnerId, generation: u64) -> Result<Self, ModelError> {
        if generation == 0 {
            return Err(ModelError::InvalidOwnerGeneration);
        }
        Ok(Self { id, generation })
    }

    pub fn validate(self) -> Result<(), ModelError> {
        if self.generation == 0 {
            return Err(ModelError::InvalidOwnerGeneration);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArtifactReference {
    pub artifact_id: String,
    pub digest: PayloadDigest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationLinkKind {
    SessionEntry,
    GraphTask,
    TaskReceipt,
    Process,
    Transaction,
    DomainReceipt,
    Replacement,
    CausalParent,
    Custom,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OperationLink {
    pub kind: OperationLinkKind,
    pub target_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ResultRef {
    pub result_id: ResultId,
    pub payload_digest: PayloadDigest,
    pub artifacts: Vec<ArtifactReference>,
    pub evidence: Vec<EvidenceId>,
    pub links: Vec<OperationLink>,
}

impl ResultRef {
    pub fn new(payload_digest: PayloadDigest) -> Self {
        Self {
            result_id: ResultId::new(),
            payload_digest,
            artifacts: Vec::new(),
            evidence: Vec::new(),
            links: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OperationAttempt {
    pub(crate) attempt_id: AttemptId,
    pub(crate) operation_id: OperationId,
    pub(crate) attempt_number: u32,
    pub(crate) owner: ExecutionOwner,
    pub(crate) revision: u64,
    pub(crate) state: OperationState,
    pub(crate) effect_status: EffectStatus,
    pub(crate) authorization: Option<AuthorizationReceipt>,
    pub(crate) started_at: Option<Timestamp>,
    pub(crate) finished_at: Option<Timestamp>,
    pub(crate) result: Option<ResultRef>,
    pub(crate) failure: Option<OperationFailure>,
    pub(crate) verification: VerificationState,
    pub(crate) publications: BTreeMap<String, PublicationState>,
    pub(crate) cancellation_requested: bool,
    pub(crate) retries_attempt_id: Option<AttemptId>,
    pub(crate) recovery_evidence: Option<EvidenceId>,
    pub(crate) superseded_by: Option<OperationId>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OperationAttemptWire {
    attempt_id: AttemptId,
    operation_id: OperationId,
    attempt_number: u32,
    owner: ExecutionOwner,
    #[serde(default)]
    revision: u64,
    state: OperationState,
    #[serde(default = "legacy_effect_unknown")]
    effect_status: EffectStatus,
    #[serde(default)]
    authorization: Option<AuthorizationReceipt>,
    #[serde(default)]
    started_at: Option<Timestamp>,
    #[serde(default)]
    finished_at: Option<Timestamp>,
    #[serde(default)]
    result: Option<ResultRef>,
    #[serde(default)]
    failure: Option<OperationFailure>,
    #[serde(default)]
    verification: VerificationState,
    #[serde(default)]
    publications: BTreeMap<String, PublicationState>,
    #[serde(default)]
    cancellation_requested: bool,
    #[serde(default)]
    retries_attempt_id: Option<AttemptId>,
    #[serde(default)]
    recovery_evidence: Option<EvidenceId>,
    #[serde(default)]
    superseded_by: Option<OperationId>,
}

fn legacy_effect_unknown() -> EffectStatus {
    EffectStatus::Unknown
}

impl OperationAttempt {
    pub fn new(
        operation_id: OperationId,
        attempt_number: u32,
        owner: ExecutionOwner,
    ) -> Result<Self, ModelError> {
        if attempt_number == 0 {
            return Err(ModelError::InvalidAttemptNumber);
        }
        owner.validate()?;
        Ok(Self {
            attempt_id: AttemptId::new(),
            operation_id,
            attempt_number,
            owner,
            revision: 0,
            state: OperationState::Created,
            effect_status: EffectStatus::NotStarted,
            authorization: None,
            started_at: None,
            finished_at: None,
            result: None,
            failure: None,
            verification: VerificationState::NotRequired,
            publications: BTreeMap::new(),
            cancellation_requested: false,
            retries_attempt_id: None,
            recovery_evidence: None,
            superseded_by: None,
        })
    }

    pub fn validate(&self) -> Result<(), ModelError> {
        if self.attempt_number == 0 {
            return Err(ModelError::InvalidAttemptNumber);
        }
        self.owner.validate()?;
        if self.result.is_some() && self.failure.is_some() {
            return Err(ModelError::ConflictingOutcomeFacts);
        }
        if self.state == OperationState::Succeeded
            && (self.result.is_none()
                || self.effect_status == EffectStatus::NotStarted
                || self.failure.is_some())
        {
            return Err(ModelError::InvalidSuccessRecord);
        }
        if self.state == OperationState::Failed && (self.failure.is_none() || self.result.is_some())
        {
            return Err(ModelError::InvalidFailureRecord);
        }
        if self.state == OperationState::Superseded && self.superseded_by.is_none() {
            return Err(ModelError::MissingReplacement);
        }
        if self.state == OperationState::EffectPossible
            && self.effect_status == EffectStatus::NotStarted
        {
            return Err(ModelError::InvalidEffectState);
        }
        if self.state != OperationState::Superseded
            && (self.superseded_by.is_some()
                || (self.result.is_some() && self.state != OperationState::Succeeded)
                || (self.failure.is_some() && self.state != OperationState::Failed))
        {
            return Err(ModelError::OutcomeOnNonterminal);
        }
        Ok(())
    }

    pub fn attempt_id(&self) -> AttemptId {
        self.attempt_id
    }
    pub fn operation_id(&self) -> OperationId {
        self.operation_id
    }
    pub fn attempt_number(&self) -> u32 {
        self.attempt_number
    }
    pub fn owner(&self) -> ExecutionOwner {
        self.owner
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn state(&self) -> OperationState {
        self.state
    }
    pub fn effect_status(&self) -> EffectStatus {
        self.effect_status
    }
    pub fn authorization(&self) -> Option<&AuthorizationReceipt> {
        self.authorization.as_ref()
    }
    pub fn started_at(&self) -> Option<Timestamp> {
        self.started_at
    }
    pub fn finished_at(&self) -> Option<Timestamp> {
        self.finished_at
    }
    pub fn result(&self) -> Option<&ResultRef> {
        self.result.as_ref()
    }
    pub fn failure(&self) -> Option<&OperationFailure> {
        self.failure.as_ref()
    }
    pub fn verification(&self) -> VerificationState {
        self.verification
    }
    pub fn publications(&self) -> &BTreeMap<String, PublicationState> {
        &self.publications
    }
    pub fn cancellation_requested(&self) -> bool {
        self.cancellation_requested
    }
    pub fn retries_attempt_id(&self) -> Option<AttemptId> {
        self.retries_attempt_id
    }
    pub fn recovery_evidence(&self) -> Option<EvidenceId> {
        self.recovery_evidence
    }
    pub fn superseded_by(&self) -> Option<OperationId> {
        self.superseded_by
    }
}

impl<'de> Deserialize<'de> for OperationAttempt {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = OperationAttemptWire::deserialize(deserializer)?;
        let attempt = Self {
            attempt_id: wire.attempt_id,
            operation_id: wire.operation_id,
            attempt_number: wire.attempt_number,
            owner: wire.owner,
            revision: wire.revision,
            state: wire.state,
            effect_status: wire.effect_status,
            authorization: wire.authorization,
            started_at: wire.started_at,
            finished_at: wire.finished_at,
            result: wire.result,
            failure: wire.failure,
            verification: wire.verification,
            publications: wire.publications,
            cancellation_requested: wire.cancellation_requested,
            retries_attempt_id: wire.retries_attempt_id,
            recovery_evidence: wire.recovery_evidence,
            superseded_by: wire.superseded_by,
        };
        attempt.validate().map_err(D::Error::custom)?;
        Ok(attempt)
    }
}
