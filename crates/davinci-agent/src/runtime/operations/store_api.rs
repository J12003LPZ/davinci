use super::transitions::{OperationEvent, TransitionError};
use super::{
    AttemptId, ExecutionOwner, JournalId, OperationAttempt, OperationId, OperationSpec, ResultRef,
    RootNamespaceId, WorkspaceIdentity,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use thiserror::Error;
use uuid::Uuid;
pub const JOURNAL_DATABASE_FILE_NAME: &str = "operations.sqlite3";
pub const MAX_OUTBOX_BATCH_SIZE: usize = 64;
pub(super) const MAX_PENDING_OUTBOX: usize = 4096;
pub(super) const MAX_OPERATIONS_PER_ROOT: i64 = 100_000;
pub(super) const MAX_SNAPSHOT_RECORDS: i64 = 50_000;
pub(super) const MAX_DATABASE_PAGES: i64 = 16_384;
pub(super) const MAX_SPEC_BYTES: usize = 1024 * 1024;
pub(super) const MAX_ATTEMPT_BYTES: usize = 64 * 1024;
pub(super) const MAX_EVENT_BYTES: usize = 64 * 1024;
pub(super) const MAX_RESULT_BYTES: usize = 1024 * 1024;
pub(super) const MAX_RESULT_REF_BYTES: usize = 64 * 1024;
pub(super) const MAX_OUTBOX_BYTES: usize = 256 * 1024;
pub(super) const MAX_CONSUMER_BYTES: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalIdentity {
    pub journal_id: JournalId,
    pub workspace: WorkspaceIdentity,
}

impl JournalIdentity {
    pub fn new(journal_id: JournalId, workspace: WorkspaceIdentity) -> Result<Self, JournalError> {
        if workspace.binding_version == 0 {
            return Err(JournalError::InvalidWorkspaceBindingVersion);
        }
        Ok(Self {
            journal_id,
            workspace,
        })
    }
}

#[derive(Debug, Error)]
pub enum JournalError {
    #[error("journal directory must be absolute")]
    RelativeDirectory,
    #[error("journal database could not be opened safely: {0}")]
    Io(String),
    #[error("SQLite journal operation failed: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("journal JSON could not be encoded or decoded: {0}")]
    Serialization(String),
    #[error("journal schema {0} is not supported")]
    UnsupportedSchema(u32),
    #[error("database application id {0} does not identify an operation journal")]
    UnsupportedDatabase(i64),
    #[error("database has objects but no recognized operation journal schema")]
    UnexpectedSchema,
    #[error("journal integrity check failed: {0}")]
    Integrity(String),
    #[error("journal identity mismatch: expected {expected}, found {actual}")]
    JournalMismatch { expected: String, actual: String },
    #[error("workspace binding mismatch: expected {expected}, found {actual}")]
    WorkspaceMismatch { expected: String, actual: String },
    #[error("root binding mismatch: expected {expected}, found {actual}")]
    RootMismatch {
        expected: RootNamespaceId,
        actual: RootNamespaceId,
    },
    #[error("workspace binding version must be nonzero")]
    InvalidWorkspaceBindingVersion,
    #[error("another coordinator owns this root namespace")]
    CoordinatorAlreadyOwned,
    #[error("operation journal migration is currently owned by another process")]
    MigrationBusy,
    #[error("journal writer is busy; retry after the active transaction completes")]
    WriterBusy,
    #[error("journal is poisoned after a failed database operation; reopen and reconcile it")]
    Poisoned,
    #[error("serialized {record} exceeds its {limit}-byte bound")]
    OversizedRecord { record: &'static str, limit: usize },
    #[error("journal capacity exceeded: {0}")]
    Capacity(&'static str),
    #[error("outbox batch has {actual} items; limit is {limit}")]
    OutboxBatchTooLarge { actual: usize, limit: usize },
    #[error("outbox consumer name must be 1 to 128 non-whitespace bytes")]
    InvalidConsumer,
    #[error("attempt does not belong to this journal root")]
    RootBindingMismatch,
    #[error("idempotency key already identifies operation {0}")]
    DuplicateIntent(OperationId),
    #[error("same scoped idempotency key was used with a different payload")]
    IdempotencyCollision,
    #[error("current owner {actual:?} does not match caller owner {expected:?}")]
    OwnerFenced {
        expected: ExecutionOwner,
        actual: ExecutionOwner,
    },
    #[error("host did not confirm that the prior operation owner is quiescent")]
    OwnerQuiescenceNotConfirmed,
    #[error("attempt already has a dispatch claim")]
    AttemptAlreadyClaimed,
    #[error("dispatch permit is invalid, stale, or belongs to another journal")]
    InvalidDispatchPermit,
    #[error("attempt is not authorized and queued for dispatch")]
    DispatchNotReady,
    #[error("dispatch start and effect latch require a host-owned dispatch permit")]
    DispatchPermitRequired,
    #[error("operation transition can only be written through the coordinator")]
    CoordinatorTransitionRequired,
    #[error("retry is unsafe because the previous effect may have started")]
    RetryEffectUncertain,
    #[error("replacement owner generation must be greater than the current generation")]
    OwnerGenerationNotAdvanced,
    #[error("authorization receipt does not approve this operation intent")]
    AuthorizationDigestMismatch,
    #[error("operation identity already exists")]
    DuplicateOperation,
    #[error("unresolved effect for operation {operation_id} holds resource claim {resource}")]
    UnresolvedEffectConflict {
        operation_id: String,
        resource: String,
    },
    #[error("operation or attempt was not found in this root namespace")]
    NotFound,
    #[error("attempt transition requires a new result payload")]
    MissingResultPayload,
    #[error("result payload was supplied for a transition that creates no result")]
    UnexpectedResultPayload,
    #[error("result payload digest does not match its result reference")]
    ResultDigestMismatch,
    #[error("result payload is already committed for this attempt")]
    DuplicateResult,
    #[error("outbox item was not found")]
    OutboxNotFound,
    #[error("snapshot has {actual} rows; limit is {limit}")]
    SnapshotTooLarge { actual: i64, limit: i64 },
    #[error("backup destination must be a new, separate directory")]
    InvalidBackupPath,
    #[error(transparent)]
    Transition(#[from] TransitionError),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutboxDraft {
    pub(super) consumer: String,
    pub(super) payload: Value,
}

impl OutboxDraft {
    pub fn new(consumer: impl Into<String>, payload: Value) -> Result<Self, JournalError> {
        let consumer = consumer.into();
        if consumer.trim().is_empty() || consumer.len() > MAX_CONSUMER_BYTES {
            return Err(JournalError::InvalidConsumer);
        }
        Ok(Self { consumer, payload })
    }

    pub fn consumer(&self) -> &str {
        &self.consumer
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredEvent {
    pub sequence: u64,
    pub operation_id: OperationId,
    pub attempt_id: AttemptId,
    pub prior_revision: u64,
    pub revision: u64,
    pub event: OperationEvent,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredResult {
    pub reference: ResultRef,
    pub payload: Value,
    pub created_at_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutboxState {
    Pending,
    Acknowledged,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StoredOutbox {
    pub id: Uuid,
    pub operation_id: OperationId,
    pub attempt_id: AttemptId,
    pub consumer: String,
    pub payload: Value,
    pub state: OutboxState,
    pub created_at_ms: u64,
    pub acknowledged_at_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OperationJournalSnapshot {
    pub journal_id: JournalId,
    pub root_namespace_id: RootNamespaceId,
    pub workspace: WorkspaceIdentity,
    pub operations: Vec<OperationSpec>,
    pub attempts: Vec<OperationAttempt>,
    pub events: Vec<StoredEvent>,
    pub results: Vec<StoredResult>,
    pub outbox: Vec<StoredOutbox>,
}

/// Observable occupancy for callers that need to apply their own bounded
/// admission or draining policy. Counts are scoped to one journal root and do
/// not authorize evicting recovery evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct JournalCapacity {
    pub operation_count: u64,
    pub pending_outbox_count: u64,
    pub snapshot_record_count: u64,
    pub max_operations: u64,
    pub max_pending_outbox: u64,
    pub max_snapshot_records: u64,
}
