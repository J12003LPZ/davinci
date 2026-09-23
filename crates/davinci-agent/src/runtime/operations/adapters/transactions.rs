use super::super::{OperationAttempt, ProcessOperationBinding};
use crate::runtime::transactions::{TransactionState, TransactionSummary};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// The operation identity and source boundary captured by a transaction
/// coordinator. It is evidence for reconciliation, never a write permit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransactionOperationLink {
    pub operation_id: String,
    pub attempt_id: String,
    pub owner_id: String,
    pub owner_generation: u64,
    pub workspace_identity: String,
}

impl TransactionOperationLink {
    pub fn from_binding(
        binding: &ProcessOperationBinding,
        workspace_identity: impl Into<String>,
    ) -> Self {
        Self {
            operation_id: binding.operation_id.to_string(),
            attempt_id: binding.attempt_id.to_string(),
            owner_id: binding.owner_id.to_string(),
            owner_generation: binding.owner_generation,
            workspace_identity: workspace_identity.into(),
        }
    }
}

/// A stable, idempotent phase receipt. The map key is the phase name, so a
/// replay updates evidence for the same phase instead of creating a second
/// effect record.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransactionPhaseReceipt {
    pub phase: String,
    pub sequence: u64,
    pub operation_id: Option<String>,
    pub attempt_id: Option<String>,
}

pub fn phase_receipts_match_summary(summary: &TransactionSummary) -> bool {
    summary.phase_receipts.iter().all(|(key, receipt)| {
        key == &receipt.phase
            && !receipt.phase.is_empty()
            && receipt.phase.len() <= 64
            && receipt.sequence > 0
            && receipt.sequence <= summary.sequence
            && receipt.operation_id == summary.operation_id
            && receipt.attempt_id == summary.attempt_id
    })
}

/// Reconcile only from durable journal facts. A missing outer operation result
/// never causes a transaction to be applied again.
pub fn reconcile_transaction_phase(summary: &TransactionSummary) -> TransactionRecovery {
    if !phase_receipts_match_summary(summary) {
        return TransactionRecovery::Corrupt;
    }
    match summary.state {
        TransactionState::Applied | TransactionState::Verified | TransactionState::Committed => {
            TransactionRecovery::AlreadyApplied
        }
        TransactionState::RolledBack => TransactionRecovery::RolledBack,
        TransactionState::Conflicted => TransactionRecovery::Conflict,
        TransactionState::Applying | TransactionState::RollingBack => {
            TransactionRecovery::NeedsRecovery
        }
        TransactionState::Draft | TransactionState::Previewed => TransactionRecovery::NotStarted,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransactionRecovery {
    NotStarted,
    AlreadyApplied,
    RolledBack,
    NeedsRecovery,
    Conflict,
    Corrupt,
}

/// A tiny projection ledger used by hosts that persist an operation result and
/// a session projection separately. It makes the ordering explicit without
/// claiming those stores share one atomic commit.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TransactionProjectionLedger {
    pub transaction_receipt: bool,
    pub operation_result: bool,
    pub session_projection: bool,
}

impl TransactionProjectionLedger {
    pub fn record_transaction_receipt(&mut self) {
        self.transaction_receipt = true;
    }

    pub fn record_operation_result(&mut self) -> Result<(), &'static str> {
        if !self.transaction_receipt {
            return Err("operation result requires a durable transaction receipt");
        }
        self.operation_result = true;
        Ok(())
    }

    pub fn record_session_projection(&mut self) -> Result<(), &'static str> {
        if !self.operation_result {
            return Err("session projection requires a durable operation result");
        }
        self.session_projection = true;
        Ok(())
    }
}

/// Keep the adapter independent of the concrete journal store while allowing
/// tests and hosts to carry the operation attempt identity through a phase.
pub fn phase_receipt_for_attempt(
    phase: impl Into<String>,
    sequence: u64,
    attempt: &OperationAttempt,
) -> TransactionPhaseReceipt {
    TransactionPhaseReceipt {
        phase: phase.into(),
        sequence,
        operation_id: Some(attempt.operation_id().to_string()),
        attempt_id: Some(attempt.attempt_id.to_string()),
    }
}

pub fn phase_map(receipt: TransactionPhaseReceipt) -> BTreeMap<String, TransactionPhaseReceipt> {
    BTreeMap::from([(receipt.phase.clone(), receipt)])
}
