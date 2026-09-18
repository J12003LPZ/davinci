//! Host-only source observations. Execution and coverage selection remain host responsibilities.
use super::{files, model::*, transition, Authority, TransactionCoordinator};
use crate::runtime::{
    evidence_store::ExecutionReceipt,
    source_manifest::{compute_manifest_digest, FileKind, ManifestEntry},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug)]
pub struct VerificationObservation {
    id: String,
    sequence: u64,
    workspace_identity: String,
    owner: TransactionOwner,
    source_digest: String,
    observed_at_ms: i64,
    affected_files: Vec<String>,
}

impl VerificationObservation {
    pub(crate) fn covered_by_compiler_roots(&self, roots: &[String]) -> bool {
        self.affected_files.iter().all(|path| roots.contains(path))
    }
}

/// An opaque, host-created observation for binding external evidence to source.
/// It cannot be deserialized or used to mark a transaction verified.
#[derive(Debug)]
pub struct SourceObservation(VerificationObservation);

impl SourceObservation {
    pub fn transaction_id(&self) -> &str {
        &self.0.id
    }

    pub fn sequence(&self) -> u64 {
        self.0.sequence
    }

    pub fn workspace_identity(&self) -> &str {
        &self.0.workspace_identity
    }

    /// Fingerprint of affected files only, not every repository dependency.
    pub fn source_digest(&self) -> &str {
        &self.0.source_digest
    }

    pub fn affected_files(&self) -> &[String] {
        &self.0.affected_files
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransactionVerification {
    pub receipt: ExecutionReceipt,
    /// Fingerprint of the transaction's affected files, including deletions and modes.
    /// This is not a claim that every repository dependency was tested.
    pub source_digest: String,
}

impl TransactionCoordinator {
    /// Observe applied source without changing journal state or older verification.
    /// The host must supply current read authority, never authority from a record.
    pub fn observe_source(
        &self,
        id: &str,
        authority: Authority<'_>,
    ) -> Result<SourceObservation, String> {
        self.locked(|store| {
            let record = store.load(id, &self.root, &self.owner, self.allow_session_recovery)?;
            if !matches!(
                record.summary.state,
                TransactionState::Applied | TransactionState::Verified
            ) {
                return Err("transaction state does not allow source observation".into());
            }
            self.check_verification_images(&record, authority)?;
            Ok(SourceObservation(VerificationObservation {
                id: id.into(),
                sequence: record.summary.sequence,
                workspace_identity: record.summary.workspace_identity.clone(),
                owner: self.owner.clone(),
                source_digest: source_digest(&record),
                observed_at_ms: now(),
                affected_files: record.summary.affected_files.clone(),
            }))
        })
    }

    /// Recheck immediately before and after external evidence collection. A match
    /// binds evidence to source; it does not prove assertions or coverage passed.
    pub fn check_source_observation(
        &self,
        observation: &SourceObservation,
        authority: Authority<'_>,
    ) -> Result<(), String> {
        let observation = &observation.0;
        if observation.owner != self.owner {
            return Err("source observation belongs to another owner".into());
        }
        self.locked(|store| {
            let record = store.load(
                &observation.id,
                &self.root,
                &self.owner,
                self.allow_session_recovery,
            )?;
            if record.summary.sequence != observation.sequence
                || record.summary.workspace_identity != observation.workspace_identity
                || source_digest(&record) != observation.source_digest
                || !matches!(
                    record.summary.state,
                    TransactionState::Applied | TransactionState::Verified
                )
            {
                return Err("transaction changed since source observation".into());
            }
            self.check_verification_images(&record, authority)
        })
    }

    /// Trusted host API: call immediately before an authorized verification command
    /// whose coverage includes this transaction. Never accept observations from model JSON.
    pub fn begin_verification(
        &self,
        id: &str,
        authority: Authority<'_>,
    ) -> Result<VerificationObservation, String> {
        self.locked(|store| {
            let mut record =
                store.load(id, &self.root, &self.owner, self.allow_session_recovery)?;
            if !matches!(
                record.summary.state,
                TransactionState::Applied | TransactionState::Verified
            ) {
                return Err("transaction state does not allow verification".into());
            }
            self.check_verification_images(&record, authority)?;
            // A failed or interrupted recheck must not leave an older success current.
            record.summary.verification_state = "verification pending".into();
            transition(&mut record, TransactionState::Applied);
            store.save(&record)?;
            Ok(VerificationObservation {
                id: id.into(),
                sequence: record.summary.sequence,
                workspace_identity: record.summary.workspace_identity.clone(),
                owner: self.owner.clone(),
                source_digest: source_digest(&record),
                observed_at_ms: now(),
                affected_files: record.summary.affected_files.clone(),
            })
        })
    }

    /// Bind an actual host execution receipt to the unchanged observation. This method
    /// performs no command execution and must never be exposed as a model-facing tool.
    pub fn finish_verification(
        &self,
        observation: VerificationObservation,
        mut receipt: ExecutionReceipt,
        authority: Authority<'_>,
    ) -> Result<TransactionSummary, String> {
        validate_receipt(&receipt)?;
        if files::root(std::path::Path::new(&receipt.cwd))? != self.root
            || receipt.started_at_ms < observation.observed_at_ms
            || receipt.task_id != self.owner.task_id
            || observation.owner != self.owner
        {
            return Err("verification receipt does not match its source observation".into());
        }
        // Store the observed canonical workspace, so journal validation can check
        // this binding without following paths supplied by persisted evidence.
        receipt.cwd = self.root.to_string_lossy().into_owned();
        self.locked(|store| {
            let mut record = store.load(
                &observation.id,
                &self.root,
                &self.owner,
                self.allow_session_recovery,
            )?;
            if record.summary.sequence != observation.sequence
                || record.summary.workspace_identity != observation.workspace_identity
                || source_digest(&record) != observation.source_digest
                || !matches!(
                    record.summary.state,
                    TransactionState::Applied | TransactionState::Verified
                )
            {
                return Err("transaction changed during verification".into());
            }
            self.check_verification_images(&record, authority)?;
            record.summary.verification = Some(TransactionVerification {
                receipt,
                source_digest: observation.source_digest,
            });
            record.summary.verification_state = "verified for affected-file scope".into();
            transition(&mut record, TransactionState::Verified);
            store.save(&record)?;
            Ok(record.summary)
        })
    }

    pub(super) fn refresh_verification(&self, id: &str) -> Result<TransactionSummary, String> {
        self.locked(|store| {
            let mut record =
                store.load(id, &self.root, &self.owner, self.allow_session_recovery)?;
            if record.summary.state == TransactionState::Verified
                && self
                    .check_verification_images(&record, &|_| Ok(()))
                    .is_err()
            {
                record.summary.verification_state =
                    "stale: affected files changed or could not be observed".into();
                transition(&mut record, TransactionState::Applied);
                store.save(&record)?;
            }
            Ok(record.summary)
        })
    }

    pub(super) fn check_verification_images(
        &self,
        record: &Record,
        authority: Authority<'_>,
    ) -> Result<(), String> {
        self.check_root()?;
        for change in &record.changes {
            authority(&self.root.join(&change.path))?;
            let (current, _) = files::capture(&self.root, &change.path)?;
            if current != change.proposed {
                return Err(format!("verification source changed: {}", change.path));
            }
        }
        Ok(())
    }
}

pub(super) fn validate(record: &Record) -> Result<(), String> {
    if let Some(evidence) = &record.summary.verification {
        validate_receipt(&evidence.receipt)?;
        if evidence.source_digest != source_digest(record)
            || evidence.receipt.cwd != record.summary.workspace
            || evidence.receipt.task_id != record.summary.owner.task_id
        {
            return Err("invalid transaction verification source binding".into());
        }
    } else if record.summary.state == TransactionState::Verified {
        return Err("verified transaction has no execution evidence".into());
    }
    Ok(())
}

fn validate_receipt(receipt: &ExecutionReceipt) -> Result<(), String> {
    let hash = |value: &Option<String>| {
        value
            .as_ref()
            .is_some_and(|v| v.len() == 64 && v.bytes().all(|b| b.is_ascii_hexdigit()))
    };
    if !receipt.is_passed()
        || receipt.started_at_ms <= 0
        || receipt.finished_at_ms < receipt.started_at_ms
        || receipt.operation_id.is_empty()
        || receipt.operation_id.len() > 256
        || receipt.tool_name.is_empty()
        || receipt.tool_name.len() > 128
        || receipt.argv.is_empty()
        || receipt.argv.len() > 128
        || receipt.argv.iter().map(String::len).sum::<usize>() > 16 * 1024
        || receipt.cwd.len() > 8192
        || !hash(&receipt.stdout_hash)
        || !hash(&receipt.stderr_hash)
        || serde_json::to_vec(receipt)
            .map_err(|e| e.to_string())?
            .len()
            > 64 * 1024
    {
        return Err("verification requires a bounded, successful actual execution receipt".into());
    }
    Ok(())
}

fn source_digest(record: &Record) -> String {
    let mut entries: Vec<_> = record
        .changes
        .iter()
        .map(|change| ManifestEntry {
            relative_path: change.path.clone(),
            kind: if change.proposed.hash.is_some() {
                FileKind::File
            } else {
                FileKind::Missing
            },
            content_hash: change.proposed.hash.clone(),
            size: change
                .proposed_bytes
                .as_ref()
                .map(|bytes| bytes.len() as u64),
            is_executable: change.proposed.mode.is_some_and(|mode| mode & 0o111 != 0),
        })
        .collect();
    entries.sort_by(|a, b| a.relative_path.cmp(&b.relative_path));
    compute_manifest_digest(&entries, &BTreeMap::new(), None, None)
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}
