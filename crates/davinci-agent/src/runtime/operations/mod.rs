mod failure;
mod identity;
mod migrations;
mod model;
mod store;
mod store_api;
mod store_support;
mod transitions;

pub use failure::{FailureReasonCode, FailureSubsystem, OperationFailure};
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
pub use store::OperationJournal;
pub use store_api::{
    JournalError, JournalIdentity, OperationJournalSnapshot, OutboxDraft, OutboxState, StoredEvent,
    StoredOutbox, StoredResult, JOURNAL_DATABASE_FILE_NAME, MAX_OUTBOX_BATCH_SIZE,
};
pub use transitions::{admit_retry, transition_attempt, OperationEvent, TransitionError};
