use super::{ExecutionOwner, OperationAttempt, OperationId, OperationSpec, StoredResult};

#[derive(Debug, Clone, PartialEq)]
pub struct AdmittedOperation {
    pub spec: OperationSpec,
    pub attempt: OperationAttempt,
    /// Present for a successful terminal attempt. Failed terminal details live
    /// on `attempt.failure()` and are included even when this is `None`.
    pub result: Option<StoredResult>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum OperationAdmission {
    New(AdmittedOperation),
    ExistingInFlight(AdmittedOperation),
    ExistingResult(AdmittedOperation),
    Collision,
}

/// An in-process capability returned only for a durable, authorized queued attempt.
/// It is intentionally neither cloneable nor serializable.
#[must_use = "a dispatch claim must be latched or explicitly reconciled"]
pub struct DispatchPermit {
    pub(super) journal_id: super::JournalId,
    pub(super) root_namespace_id: super::RootNamespaceId,
    pub(super) operation_id: OperationId,
    pub(super) attempt_id: super::AttemptId,
    pub(super) owner: ExecutionOwner,
    pub(super) revision: u64,
    pub(super) nonce: uuid::Uuid,
}

/// A single-use capability returned after the effect-start latch is durable.
/// It cannot be reconstructed from persisted or deserialized operation data.
#[must_use = "the effect capability is single-use"]
pub struct EffectPermit {
    pub(super) journal_id: super::JournalId,
    pub(super) root_namespace_id: super::RootNamespaceId,
    pub(super) operation_id: OperationId,
    pub(super) attempt_id: super::AttemptId,
    pub(super) owner: ExecutionOwner,
}

impl EffectPermit {
    /// Consumes this permit while invoking one host-selected adapter action.
    pub fn dispatch<T>(
        self,
        journal: &super::OperationJournal,
        action: impl FnOnce() -> T,
    ) -> Result<T, super::JournalError> {
        journal.consume_effect_permit(&self)?;
        let Self {
            journal_id,
            root_namespace_id,
            operation_id,
            attempt_id,
            owner,
        } = self;
        let _durable_identity = (
            journal_id,
            root_namespace_id,
            operation_id,
            attempt_id,
            owner,
        );
        Ok(action())
    }
}
