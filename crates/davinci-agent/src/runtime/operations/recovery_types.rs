use super::store_api::JournalError;
use super::{OperationAttempt, OperationId, OperationSpec, PayloadDigest};
use serde::{Deserialize, Serialize};
use thiserror::Error;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryPhase {
    BeforeIntentCommit,
    AfterIntentBeforeAuthorization,
    AfterAuthorizationBeforeClaim,
    AfterClaimBeforeUnsafeBoundary,
    AfterEffectLatchBeforeSyscall,
    AfterProcessSpawnBeforeReceipt,
    AfterFileEffectBeforeResult,
    DuringMultiFileApplyOrRollback,
    AfterExternalRequestBeforeResponse,
    AfterResultBeforeNotification,
    DuringSessionAppend,
    DuringVerification,
    DuringGraphWorkerCompletion,
    DuringCancellationOrTermination,
    DuringSchemaMigration,
    DuringJournalWriteOrCorruption,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryClassification {
    SafeToRetry,
    AlreadyCompleted,
    NeedsVerification,
    NeedsReconciliation,
    NeedsHumanDecision,
    TerminalFailure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryAction {
    AdmitIntent,
    Authorize,
    ClaimDispatch,
    StartSafeRetry,
    ProbePostcondition,
    ReconcileDomain,
    RecordObservedOutcome,
    ReplayPublication,
    RepairProjection,
    RunVerification,
    ReconcileTermination,
    ResumeMigration,
    Stop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryReasonCode {
    NoIntentCommitted,
    AuthorizationCurrent,
    AuthorizationRevoked,
    UnclaimedAuthorizedIntent,
    QuiescentUnstartedDispatch,
    EffectMayHaveStarted,
    ProcessReceiptMissing,
    PostconditionObserved,
    TransactionInProgress,
    RemoteOutcomeUnknown,
    RemoteEffectObserved,
    ResultAwaitingPublication,
    SessionProjectionTorn,
    VerificationIncomplete,
    BoundWorkerResultAvailable,
    ProcessMayStillBeLive,
    MigrationSourceValid,
    JournalCorrupt,
    InconsistentJournalFacts,
    OwnerNotQuiescent,
    ProbeNotAuthorized,
    PolicyVersionInvalid,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnerObservation {
    Unknown,
    Quiescent,
    Live,
}

impl Default for OwnerObservation {
    fn default() -> Self {
        Self::Unknown
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AuthorizationObservation {
    Unknown,
    Current,
    Revoked,
}

impl Default for AuthorizationObservation {
    fn default() -> Self {
        Self::Unknown
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DispatchObservation {
    Unknown,
    Unclaimed,
    ClaimedUnstarted,
    EffectLatched,
}

impl Default for DispatchObservation {
    fn default() -> Self {
        Self::Unknown
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PostconditionObservation {
    Unknown,
    Satisfied,
    NotSatisfied,
}

impl Default for PostconditionObservation {
    fn default() -> Self {
        Self::Unknown
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessObservation {
    Unknown,
    Live,
    Exited,
    NotStarted,
    MissingReceipt,
}

impl Default for ProcessObservation {
    fn default() -> Self {
        Self::Unknown
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransactionObservation {
    Unknown,
    InProgress,
    Committed,
    RolledBack,
}

impl Default for TransactionObservation {
    fn default() -> Self {
        Self::Unknown
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RemoteObservation {
    Unknown,
    Applied,
    NotApplied,
    Rejected,
}

impl Default for RemoteObservation {
    fn default() -> Self {
        Self::Unknown
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PublicationObservation {
    Unknown,
    Pending,
    Acknowledged,
}

impl Default for PublicationObservation {
    fn default() -> Self {
        Self::Unknown
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionProjectionObservation {
    Unknown,
    ValidEntry,
    TornTail,
    Missing,
}

impl Default for SessionProjectionObservation {
    fn default() -> Self {
        Self::Unknown
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum VerificationObservation {
    Unknown,
    Incomplete,
    Passed,
    Failed,
}

impl Default for VerificationObservation {
    fn default() -> Self {
        Self::Unknown
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerObservation {
    Unknown,
    ValidBoundResult,
    MissingOrStale,
}

impl Default for WorkerObservation {
    fn default() -> Self {
        Self::Unknown
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MigrationObservation {
    Unknown,
    SourceValid,
    TargetCommitted,
    Invalid,
}

impl Default for MigrationObservation {
    fn default() -> Self {
        Self::Unknown
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JournalIntegrityObservation {
    Healthy,
    Corrupt,
    Unknown,
}

impl Default for JournalIntegrityObservation {
    fn default() -> Self {
        Self::Healthy
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct RecoveryObservations {
    pub owner: OwnerObservation,
    pub authorization: AuthorizationObservation,
    pub dispatch: DispatchObservation,
    pub postcondition: PostconditionObservation,
    pub process: ProcessObservation,
    pub transaction: TransactionObservation,
    pub remote: RemoteObservation,
    pub publication: PublicationObservation,
    pub session: SessionProjectionObservation,
    pub verification: VerificationObservation,
    pub probe_authorized: bool,
    pub worker: WorkerObservation,
    pub migration: MigrationObservation,
    pub journal: JournalIntegrityObservation,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryInput {
    pub spec: OperationSpec,
    pub attempts: Vec<OperationAttempt>,
    pub phase: RecoveryPhase,
    pub observations: RecoveryObservations,
    pub policy_version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryDecision {
    pub operation_id: OperationId,
    pub classification: RecoveryClassification,
    pub action: RecoveryAction,
    pub reason_code: RecoveryReasonCode,
    pub policy_version: String,
    pub input_evidence_digests: Vec<PayloadDigest>,
}

pub type OperationRecoveryDecision = RecoveryDecision;

#[derive(Debug, Error)]
pub enum RecoveryRunError {
    #[error(transparent)]
    Journal(#[from] JournalError),
    #[error("recovery action was denied: {0}")]
    AuthorizationDenied(String),
    #[error("recovery action could not be scheduled: {0}")]
    ScheduleFailed(String),
    #[error("safe retry requires a fenced replacement owner")]
    ReplacementOwnerRequired,
    #[error("recovery action requires durable attempt records")]
    AttemptRecordRequired,
    #[error("recovery action does not permit automatic execution")]
    UnsafeRecoveryAction,
}

#[derive(Debug)]
pub struct RecoveryRunResult<T> {
    pub decision: RecoveryDecision,
    pub retry_attempt: Option<OperationAttempt>,
    pub scheduled: bool,
    pub result: Option<T>,
}
