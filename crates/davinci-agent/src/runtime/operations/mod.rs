mod adapters;
mod coordinator;
mod coordinator_api;
mod failure;
mod fault;
mod identity;
mod migrations;
mod model;
mod outbox;
mod planner;
mod recovery;
mod recovery_types;
mod retry_safety;
mod store;
mod store_api;
mod store_support;
mod transitions;

pub use adapters::{
    artifact_digests, bind_verification_evidence, classify_file_observation, output_is_complete,
    phase_map, phase_receipt_for_attempt, phase_receipts_match_summary,
    reconcile_transaction_phase, redact_browser_value, redacted_browser_tool_result,
    redacted_external_tool_result, transaction_link_for_verification, AgentLaunchDisposition,
    AgentOperationAdapter, AgentOperationError, AgentOperationHandle, BrowserActionBinding,
    BrowserOperationAdapter, BrowserOperationDisposition, BrowserOperationError,
    BrowserOperationHandle, ChildExecutionContext, ChildExecutionKind, ControlOperationAdapter,
    ControlOperationError, ControlOperationReceipt, ControlReceiptValue, ExternalEndpointContract,
    ExternalEndpointIdentity, ExternalInvocationBinding, ExternalOperationAdapter,
    ExternalOperationDisposition, ExternalOperationError, ExternalOperationHandle,
    FilesystemEffectObservation, FilesystemOperationLink, ProcessOperationBinding,
    ToolOperationDispatchError, ToolOperationDispatcher, ToolOperationRuntime,
    TransactionOperationLink, TransactionPhaseReceipt, TransactionProjectionLedger,
    TransactionRecovery, UnresolvedChild, VerificationEvidenceBinding, VerificationOperationLink,
    CONTROL_OPERATION_SCHEMA_VERSION, EXTERNAL_OPERATION_SCHEMA_VERSION,
    VERIFICATION_EVIDENCE_SCHEMA_VERSION,
};
pub use coordinator_api::{AdmittedOperation, DispatchPermit, EffectPermit, OperationAdmission};
pub use failure::{FailureReasonCode, FailureSubsystem, OperationFailure};
pub use fault::{FaultInjector, FaultPoint};
pub use identity::{
    AttemptId, ExecutionOwnerId, IdempotencyScope, IdentityError, JournalId, OperationId,
    PayloadDigest, ResultId, RootNamespaceId, ScopedIdempotencyKey, WorkspaceId,
};
pub use model::{
    ArtifactReference, AuthorizationReceipt, CallerType, EffectClass, EffectProfile, EffectStatus,
    ExecutionOwner, GraphRunBinding, ModelError, OperationAttempt, OperationContext, OperationKind,
    OperationLink, OperationLinkKind, OperationPayload, OperationSpec, OperationState,
    Precondition, PreconditionKind, PublicationState, ResultRef, Timestamp, VerificationState,
    WorkspaceIdentity, OPERATION_SCHEMA_VERSION,
};
pub use outbox::{OperationResultReady, SESSION_RESULT_CONSUMER};
pub use planner::{PlannedToolOperation, ToolOperationPlanError, ToolOperationPlanner};
pub use recovery::{
    AuthorizationObservation, DispatchObservation, JournalIntegrityObservation,
    MigrationObservation, OperationRecoveryDecision, OwnerObservation, PostconditionObservation,
    ProcessObservation, PublicationObservation, RecoveryAction, RecoveryClassification,
    RecoveryDecision, RecoveryEngine, RecoveryInput, RecoveryObservations, RecoveryPhase,
    RecoveryReasonCode, RecoveryRunError, RecoveryRunResult, RemoteObservation,
    SessionProjectionObservation, TransactionObservation, VerificationObservation,
    WorkerObservation,
};
pub use store::OperationJournal;
pub use store_api::{
    JournalError, JournalIdentity, OperationJournalSnapshot, OutboxDraft, OutboxState, StoredEvent,
    StoredOutbox, StoredResult, JOURNAL_DATABASE_FILE_NAME, MAX_OUTBOX_BATCH_SIZE,
};
pub use transitions::{admit_retry, transition_attempt, OperationEvent, TransitionError};
