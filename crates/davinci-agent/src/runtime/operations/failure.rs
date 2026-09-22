use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureSubsystem {
    Persistence,
    Authorization,
    Admission,
    Execution,
    Process,
    Verification,
    Publication,
    Recovery,
    Schema,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureReasonCode {
    Persistence,
    AuthorizationDenied,
    IdempotencyCollision,
    OwnerLoss,
    ProcessIdentityMismatch,
    UnknownEffect,
    VerificationFailed,
    PublicationFailed,
    Corruption,
    UnsupportedSchema,
    InvalidTransition,
    RevisionConflict,
    ResourceConflict,
    UnsupportedOperation,
    Cancellation,
    Timeout,
    Other,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[serde(deny_unknown_fields)]
#[error("{subsystem:?} operation failure ({reason:?})")]
pub struct OperationFailure {
    pub reason: FailureReasonCode,
    pub subsystem: FailureSubsystem,
    #[source]
    pub cause: Option<Box<OperationFailure>>,
}

impl OperationFailure {
    pub fn new(reason: FailureReasonCode, subsystem: FailureSubsystem) -> Self {
        Self {
            reason,
            subsystem,
            cause: None,
        }
    }

    pub fn caused_by(mut self, cause: OperationFailure) -> Self {
        self.cause = Some(Box::new(cause));
        self
    }
}
