use super::store_api::JournalError;
use super::store_support::{decode_json, now_unix_millis, sqlite_error};
use super::{
    EffectStatus, ExecutionOwner, OperationAttempt, OperationId, OperationJournal, OperationState,
    PayloadDigest,
};
use crate::runtime::EvidenceId;
use rusqlite::{params, OptionalExtension};

pub use super::recovery_types::*;

const MAX_POLICY_VERSION_BYTES: usize = 128;
const MAX_RECOVERY_DECISION_BYTES: usize = 262_144;

pub struct RecoveryEngine;

impl RecoveryEngine {
    pub fn reduce(input: &RecoveryInput) -> RecoveryDecision {
        let (classification, action, reason_code) = classify(input);
        let evidence = serde_json::to_vec(&(
            &input.spec,
            &input.attempts,
            input.phase,
            &input.observations,
            &input.policy_version,
        ))
        .expect("recovery inputs are serializable");
        RecoveryDecision {
            operation_id: input.spec.operation_id(),
            classification,
            action,
            reason_code,
            policy_version: input.policy_version.clone(),
            input_evidence_digests: vec![PayloadDigest::of_bytes(&evidence)],
        }
    }

    /// The decision is committed first, then current authorization is checked,
    /// and only then can retry/probe work be scheduled.
    pub fn run_authorized<T>(
        journal: &OperationJournal,
        owner: ExecutionOwner,
        input: RecoveryInput,
        replacement_owner: Option<ExecutionOwner>,
        evidence: EvidenceId,
        quiesce_owner: impl FnOnce(ExecutionOwner) -> Result<(), String>,
        authorize: impl FnOnce(&RecoveryDecision) -> Result<(), String>,
        schedule: impl FnOnce(&RecoveryDecision, Option<&OperationAttempt>) -> Result<T, String>,
    ) -> Result<RecoveryRunResult<T>, RecoveryRunError> {
        journal.validate_spec_binding(&input.spec)?;
        if let Some(latest) = latest_input_attempt(&input) {
            if latest.owner() != owner {
                return Err(JournalError::OwnerFenced {
                    expected: owner,
                    actual: latest.owner(),
                }
                .into());
            }
        }
        let decision = Self::reduce(&input);
        if decision.action == RecoveryAction::AdmitIntent {
            let initial_attempt = OperationAttempt::new(input.spec.operation_id(), 1, owner)
                .map_err(|error| JournalError::Serialization(error.to_string()))?;
            match journal.admit(&input.spec, &initial_attempt)? {
                super::OperationAdmission::New(_) => {}
                super::OperationAdmission::ExistingInFlight(admitted)
                    if admitted.attempt.owner() != owner =>
                {
                    return Err(JournalError::OwnerFenced {
                        expected: owner,
                        actual: admitted.attempt.owner(),
                    }
                    .into());
                }
                super::OperationAdmission::ExistingInFlight(admitted)
                    if admitted.attempt.state() == OperationState::Persisted => {}
                super::OperationAdmission::ExistingInFlight(_) => {
                    return Err(RecoveryRunError::UnsafeRecoveryAction);
                }
                super::OperationAdmission::ExistingResult(_) => {
                    return Err(RecoveryRunError::UnsafeRecoveryAction);
                }
                super::OperationAdmission::Collision => {
                    return Err(JournalError::IdempotencyCollision.into());
                }
            }
        }
        journal.persist_recovery_decision(&decision, evidence)?;

        if decision.action == RecoveryAction::Stop {
            return Ok(RecoveryRunResult {
                decision,
                retry_attempt: None,
                scheduled: false,
                result: None,
            });
        }
        authorize(&decision).map_err(RecoveryRunError::AuthorizationDenied)?;

        let retry_attempt = if decision.action == RecoveryAction::StartSafeRetry {
            if decision.classification != RecoveryClassification::SafeToRetry {
                return Err(RecoveryRunError::UnsafeRecoveryAction);
            }
            let latest =
                latest_input_attempt(&input).ok_or(RecoveryRunError::AttemptRecordRequired)?;
            let new_owner = replacement_owner.ok_or(RecoveryRunError::ReplacementOwnerRequired)?;
            if new_owner.generation <= latest.owner().generation {
                return Err(RecoveryRunError::ReplacementOwnerRequired);
            }
            if latest.state() == OperationState::Queued {
                if input.observations.owner != OwnerObservation::Quiescent
                    || input.observations.dispatch != DispatchObservation::ClaimedUnstarted
                    || latest.effect_status() != EffectStatus::NotStarted
                {
                    return Err(RecoveryRunError::UnsafeRecoveryAction);
                }
                journal.fence_owner_after_quiescence(
                    latest.operation_id(),
                    latest.owner(),
                    new_owner,
                    quiesce_owner,
                )?;
                let recovered = journal.recover_unstarted_dispatch_after_owner_fence(
                    latest.attempt_id(),
                    latest.revision(),
                    new_owner,
                    evidence,
                )?;
                Some(journal.admit_retry_after_owner_fence(
                    latest.attempt_id(),
                    recovered.revision(),
                    new_owner,
                    evidence,
                )?)
            } else if latest.state() != OperationState::RecoveryRequired {
                return Err(RecoveryRunError::UnsafeRecoveryAction);
            } else {
                Some(journal.admit_retry_after_owner_fence(
                    latest.attempt_id(),
                    latest.revision(),
                    new_owner,
                    evidence,
                )?)
            }
        } else {
            None
        };

        let result = schedule(&decision, retry_attempt.as_ref())
            .map_err(RecoveryRunError::ScheduleFailed)?;
        Ok(RecoveryRunResult {
            decision,
            retry_attempt,
            scheduled: true,
            result: Some(result),
        })
    }
}

fn classify(input: &RecoveryInput) -> (RecoveryClassification, RecoveryAction, RecoveryReasonCode) {
    use AuthorizationObservation as A;
    use DispatchObservation as D;
    use JournalIntegrityObservation as J;
    use OwnerObservation as O;
    use RecoveryAction as X;
    use RecoveryClassification as C;
    use RecoveryPhase as P;
    use RecoveryReasonCode as R;

    if input.policy_version.trim().is_empty()
        || input.policy_version.len() > MAX_POLICY_VERSION_BYTES
    {
        return (C::TerminalFailure, X::Stop, R::PolicyVersionInvalid);
    }
    if input.observations.journal == J::Corrupt || input.phase == P::DuringJournalWriteOrCorruption
    {
        return (C::TerminalFailure, X::Stop, R::JournalCorrupt);
    }

    match input.phase {
        P::BeforeIntentCommit if input.attempts.is_empty() => {
            (C::SafeToRetry, X::AdmitIntent, R::NoIntentCommitted)
        }
        P::BeforeIntentCommit => inconsistent(),
        P::AfterIntentBeforeAuthorization => {
            if latest_input_attempt(input)
                .is_some_and(|attempt| attempt.state() == OperationState::Persisted)
            {
                match input.observations.authorization {
                    A::Current => (C::SafeToRetry, X::Authorize, R::AuthorizationCurrent),
                    A::Revoked => (C::TerminalFailure, X::Stop, R::AuthorizationRevoked),
                    A::Unknown => (C::NeedsHumanDecision, X::Stop, R::AuthorizationRevoked),
                }
            } else {
                inconsistent()
            }
        }
        P::AfterAuthorizationBeforeClaim => {
            if latest_input_attempt(input).is_some_and(|attempt| {
                attempt.state() == OperationState::Authorized
                    && input.observations.authorization == A::Current
            }) {
                (
                    C::SafeToRetry,
                    X::ClaimDispatch,
                    R::UnclaimedAuthorizedIntent,
                )
            } else if input.observations.authorization == A::Revoked {
                (C::TerminalFailure, X::Stop, R::AuthorizationRevoked)
            } else {
                inconsistent()
            }
        }
        P::AfterClaimBeforeUnsafeBoundary => {
            if input.observations.owner == O::Quiescent
                && input.observations.dispatch == D::ClaimedUnstarted
                && input.observations.authorization == A::Current
                && latest_input_attempt(input).is_some_and(|attempt| {
                    attempt.state() == OperationState::Queued
                        && attempt.effect_status() == EffectStatus::NotStarted
                })
            {
                (
                    C::SafeToRetry,
                    X::StartSafeRetry,
                    R::QuiescentUnstartedDispatch,
                )
            } else if input.observations.owner != O::Quiescent {
                (C::NeedsHumanDecision, X::Stop, R::OwnerNotQuiescent)
            } else {
                inconsistent()
            }
        }
        P::AfterEffectLatchBeforeSyscall => {
            if input.observations.postcondition == PostconditionObservation::Satisfied {
                (
                    C::AlreadyCompleted,
                    X::RecordObservedOutcome,
                    R::PostconditionObserved,
                )
            } else if input.spec.effects().supports_postcondition_probe
                && input.observations.authorization == A::Current
            {
                (
                    C::NeedsVerification,
                    X::ProbePostcondition,
                    R::EffectMayHaveStarted,
                )
            } else {
                (
                    C::NeedsReconciliation,
                    X::ReconcileDomain,
                    R::EffectMayHaveStarted,
                )
            }
        }
        P::AfterProcessSpawnBeforeReceipt => (
            C::NeedsReconciliation,
            X::ReconcileDomain,
            R::ProcessReceiptMissing,
        ),
        P::AfterFileEffectBeforeResult => {
            if input.observations.postcondition == PostconditionObservation::Satisfied {
                (
                    C::AlreadyCompleted,
                    X::RecordObservedOutcome,
                    R::PostconditionObserved,
                )
            } else {
                (
                    C::NeedsReconciliation,
                    X::ReconcileDomain,
                    R::EffectMayHaveStarted,
                )
            }
        }
        P::DuringMultiFileApplyOrRollback => {
            if matches!(
                input.observations.transaction,
                TransactionObservation::Committed | TransactionObservation::RolledBack
            ) {
                (
                    C::NeedsVerification,
                    X::ProbePostcondition,
                    R::TransactionInProgress,
                )
            } else {
                (
                    C::NeedsReconciliation,
                    X::ReconcileDomain,
                    R::TransactionInProgress,
                )
            }
        }
        P::AfterExternalRequestBeforeResponse => match input.observations.remote {
            RemoteObservation::Applied => (
                C::AlreadyCompleted,
                X::RecordObservedOutcome,
                R::RemoteEffectObserved,
            ),
            RemoteObservation::Rejected => (C::TerminalFailure, X::Stop, R::RemoteEffectObserved),
            RemoteObservation::Unknown | RemoteObservation::NotApplied => (
                C::NeedsReconciliation,
                X::ReconcileDomain,
                R::RemoteOutcomeUnknown,
            ),
        },
        P::AfterResultBeforeNotification => {
            if has_durable_success(input)
                && input.observations.publication == PublicationObservation::Pending
            {
                (
                    C::AlreadyCompleted,
                    X::ReplayPublication,
                    R::ResultAwaitingPublication,
                )
            } else {
                inconsistent()
            }
        }
        P::DuringSessionAppend => match input.observations.session {
            SessionProjectionObservation::ValidEntry => (
                C::AlreadyCompleted,
                X::ReplayPublication,
                R::ResultAwaitingPublication,
            ),
            SessionProjectionObservation::TornTail => (
                C::NeedsReconciliation,
                X::RepairProjection,
                R::SessionProjectionTorn,
            ),
            _ => (
                C::NeedsReconciliation,
                X::RepairProjection,
                R::SessionProjectionTorn,
            ),
        },
        P::DuringVerification => {
            if input.observations.verification == VerificationObservation::Incomplete
                && input.observations.probe_authorized
            {
                (
                    C::NeedsVerification,
                    X::RunVerification,
                    R::VerificationIncomplete,
                )
            } else if input.observations.probe_authorized {
                (
                    C::NeedsVerification,
                    X::RunVerification,
                    R::VerificationIncomplete,
                )
            } else {
                (C::NeedsHumanDecision, X::Stop, R::ProbeNotAuthorized)
            }
        }
        P::DuringGraphWorkerCompletion => {
            if input.observations.worker == WorkerObservation::ValidBoundResult {
                (
                    C::AlreadyCompleted,
                    X::ReplayPublication,
                    R::BoundWorkerResultAvailable,
                )
            } else {
                (
                    C::NeedsReconciliation,
                    X::ReconcileDomain,
                    R::BoundWorkerResultAvailable,
                )
            }
        }
        P::DuringCancellationOrTermination => {
            if input.observations.process == ProcessObservation::Exited
                && input.observations.owner == O::Quiescent
            {
                (
                    C::AlreadyCompleted,
                    X::ReconcileTermination,
                    R::ProcessMayStillBeLive,
                )
            } else {
                (
                    C::NeedsHumanDecision,
                    X::ReconcileTermination,
                    R::ProcessMayStillBeLive,
                )
            }
        }
        P::DuringSchemaMigration => match input.observations.migration {
            MigrationObservation::SourceValid | MigrationObservation::TargetCommitted => {
                (C::SafeToRetry, X::ResumeMigration, R::MigrationSourceValid)
            }
            MigrationObservation::Invalid | MigrationObservation::Unknown => {
                (C::TerminalFailure, X::Stop, R::JournalCorrupt)
            }
        },
        P::DuringJournalWriteOrCorruption => (C::TerminalFailure, X::Stop, R::JournalCorrupt),
    }
}

fn latest_input_attempt(input: &RecoveryInput) -> Option<&OperationAttempt> {
    input
        .attempts
        .iter()
        .filter(|attempt| attempt.operation_id() == input.spec.operation_id())
        .max_by_key(|attempt| attempt.attempt_number())
}

fn has_durable_success(input: &RecoveryInput) -> bool {
    latest_input_attempt(input).is_some_and(|attempt| {
        attempt.state() == OperationState::Succeeded
            && attempt.result().is_some()
            && attempt.effect_status() != EffectStatus::NotStarted
    })
}

fn inconsistent() -> (RecoveryClassification, RecoveryAction, RecoveryReasonCode) {
    (
        RecoveryClassification::TerminalFailure,
        RecoveryAction::Stop,
        RecoveryReasonCode::InconsistentJournalFacts,
    )
}

impl OperationJournal {
    pub(super) fn persist_recovery_decision(
        &self,
        decision: &RecoveryDecision,
        evidence: EvidenceId,
    ) -> Result<(), JournalError> {
        if decision.policy_version.trim().is_empty()
            || decision.policy_version.len() > MAX_POLICY_VERSION_BYTES
        {
            return Err(JournalError::Serialization(
                "invalid recovery policy version".into(),
            ));
        }
        let encoded = serde_json::to_string(decision)
            .map_err(|error| JournalError::Serialization(error.to_string()))?;
        if encoded.len() > MAX_RECOVERY_DECISION_BYTES {
            return Err(JournalError::OversizedRecord {
                record: "recovery decision",
                limit: MAX_RECOVERY_DECISION_BYTES,
            });
        }
        self.write(|transaction| {
            let belongs_to_root: bool = transaction
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM operations
                     WHERE operation_id = ?1 AND root_namespace_id = ?2)",
                    params![
                        decision.operation_id.to_string(),
                        self.root_namespace_id.to_string()
                    ],
                    |row| row.get(0),
                )
                .map_err(sqlite_error)?;
            if !belongs_to_root {
                return Err(JournalError::NotFound);
            }
            transaction
                .execute(
                    "INSERT INTO operation_recovery_decisions
                     (decision_id, root_namespace_id, operation_id, evidence_id,
                      policy_version, decision_json, created_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        uuid::Uuid::now_v7().to_string(),
                        self.root_namespace_id.to_string(),
                        decision.operation_id.to_string(),
                        evidence.to_string(),
                        decision.policy_version,
                        encoded,
                        now_unix_millis()
                    ],
                )
                .map_err(sqlite_error)?;
            Ok(())
        })
    }

    pub fn latest_recovery_decision(
        &self,
        operation_id: OperationId,
    ) -> Result<Option<RecoveryDecision>, JournalError> {
        self.read(|transaction| {
            let encoded: Option<String> = transaction
                .query_row(
                    "SELECT decision_json FROM operation_recovery_decisions
                     WHERE operation_id = ?1 AND root_namespace_id = ?2
                     ORDER BY decision_sequence DESC LIMIT 1",
                    params![operation_id.to_string(), self.root_namespace_id.to_string()],
                    |row| row.get(0),
                )
                .optional()
                .map_err(sqlite_error)?;
            encoded.map(|value| decode_json(&value)).transpose()
        })
    }
}
