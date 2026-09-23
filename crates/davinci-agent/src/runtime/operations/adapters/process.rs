use super::super::{AdmittedOperation, AttemptId, ExecutionOwnerId, JournalId, OperationId};
use serde::{Deserialize, Serialize};

/// Durable operation ownership carried through the private process protocol.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessOperationBinding {
    pub journal_id: JournalId,
    pub operation_id: OperationId,
    pub attempt_id: AttemptId,
    pub owner_id: ExecutionOwnerId,
    pub owner_generation: u64,
}

impl ProcessOperationBinding {
    pub fn from_admitted(admitted: &AdmittedOperation) -> Self {
        let owner = admitted.attempt.owner();
        Self {
            journal_id: admitted.spec.context().journal_id,
            operation_id: admitted.spec.operation_id(),
            attempt_id: admitted.attempt.attempt_id(),
            owner_id: owner.id,
            owner_generation: owner.generation,
        }
    }
}
