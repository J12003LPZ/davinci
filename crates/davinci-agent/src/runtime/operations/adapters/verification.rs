use super::super::TransactionOperationLink;
use crate::runtime::{evidence_store::ExecutionReceipt, transactions::TransactionSummary};
use serde::{Deserialize, Serialize};

pub const VERIFICATION_EVIDENCE_SCHEMA_VERSION: u32 = 1;

/// The verification command is a child observation of the transaction
/// operation. Its identity and source digest are persisted with the evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationOperationLink {
    pub verification_operation_id: String,
    pub verification_attempt_id: Option<String>,
    pub parent_operation_id: Option<String>,
    pub parent_attempt_id: Option<String>,
    pub workspace_identity: String,
    pub source_manifest_digest: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VerificationEvidenceBinding {
    pub schema_version: u32,
    pub link: VerificationOperationLink,
    pub output_complete: bool,
    pub artifact_digests: Vec<String>,
}

pub fn artifact_digests(receipt: &ExecutionReceipt) -> Vec<String> {
    let mut digests = receipt
        .stdout_artifact
        .iter()
        .chain(receipt.stderr_artifact.iter())
        .map(|artifact| artifact.sha256.clone())
        .collect::<Vec<_>>();
    digests.sort();
    digests.dedup();
    digests
}

pub fn output_is_complete(receipt: &ExecutionReceipt) -> bool {
    receipt.stdout_hash.is_some() && receipt.stderr_hash.is_some()
}

pub fn bind_verification_evidence(
    summary: &TransactionSummary,
    receipt: &ExecutionReceipt,
    source_manifest_digest: impl Into<String>,
) -> Result<VerificationEvidenceBinding, String> {
    let source_manifest_digest = source_manifest_digest.into();
    if source_manifest_digest.is_empty() {
        return Err("verification source manifest digest is empty".into());
    }
    if receipt.operation_id.is_empty() {
        return Err("verification receipt has no operation identity".into());
    }
    if summary
        .operation_id
        .as_deref()
        .is_some_and(|operation_id| operation_id != receipt.operation_id)
    {
        return Err("verification receipt operation does not match transaction operation".into());
    }
    if summary
        .attempt_id
        .as_deref()
        .is_some_and(|attempt_id| receipt.attempt_id.as_deref() != Some(attempt_id))
    {
        return Err("verification receipt attempt does not match transaction attempt".into());
    }
    if summary
        .operation_workspace_identity
        .as_deref()
        .is_some_and(|identity| identity != summary.workspace_identity)
    {
        return Err("transaction operation source identity is stale".into());
    }
    Ok(VerificationEvidenceBinding {
        schema_version: VERIFICATION_EVIDENCE_SCHEMA_VERSION,
        link: VerificationOperationLink {
            verification_operation_id: receipt.operation_id.clone(),
            verification_attempt_id: receipt.attempt_id.clone(),
            parent_operation_id: summary.operation_id.clone(),
            parent_attempt_id: summary.attempt_id.clone(),
            workspace_identity: summary.workspace_identity.clone(),
            source_manifest_digest,
        },
        output_complete: output_is_complete(receipt),
        artifact_digests: artifact_digests(receipt),
    })
}

pub fn transaction_link_for_verification(
    summary: &TransactionSummary,
) -> Option<TransactionOperationLink> {
    Some(TransactionOperationLink {
        operation_id: summary.operation_id.clone()?,
        attempt_id: summary.attempt_id.clone()?,
        owner_id: summary.operation_owner_id.clone()?,
        owner_generation: summary.operation_owner_generation?,
        workspace_identity: summary.operation_workspace_identity.clone()?,
    })
}
