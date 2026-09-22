use super::failure::OperationFailure;
use super::identity::OperationId;
use super::model::{
    AuthorizationReceipt, EffectStatus, ExecutionOwner, OperationAttempt, OperationState,
    PublicationState, ResultRef, Timestamp, VerificationState,
};
use crate::runtime::EvidenceId;
use std::collections::BTreeMap;
use thiserror::Error;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OperationEvent {
    Persist,
    Authorize(AuthorizationReceipt),
    Queue,
    Start {
        at: Timestamp,
    },
    MarkEffectPossible,
    CompleteSuccess {
        result: ResultRef,
        effect_status: EffectStatus,
        finished_at: Timestamp,
    },
    CompleteFailure {
        failure: OperationFailure,
        effect_status: EffectStatus,
        finished_at: Timestamp,
    },
    CancelBeforeStart {
        finished_at: Timestamp,
    },
    RequestCancellation,
    Interrupt {
        at: Timestamp,
    },
    RequireRecovery,
    FinalizeRecoveredSuccess {
        result: ResultRef,
        effect_status: EffectStatus,
        evidence: EvidenceId,
        finished_at: Timestamp,
    },
    FinalizeRecoveredFailure {
        failure: OperationFailure,
        effect_status: EffectStatus,
        evidence: EvidenceId,
        finished_at: Timestamp,
    },
    BlockRecovery,
    MarkVerification(VerificationState),
    Publish {
        consumer: String,
        state: PublicationState,
    },
    Supersede {
        replacement: OperationId,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TransitionError {
    #[error("stale operation revision: expected {expected}, actual {actual}")]
    StaleRevision { expected: u64, actual: u64 },
    #[error("event {event} is not valid from state {from:?}")]
    InvalidTransition {
        from: OperationState,
        event: &'static str,
    },
    #[error("effect certainty cannot be not_started after execution completes")]
    MissingEffectCertainty,
    #[error("revision counter overflow")]
    RevisionOverflow,
    #[error("retry requires a recovery-required attempt")]
    RetryNotReady,
    #[error("attempt number overflow")]
    AttemptNumberOverflow,
    #[error("verification transition is invalid")]
    InvalidVerificationTransition,
    #[error("publication transition is invalid")]
    InvalidPublicationTransition,
    #[error("replacement operation must have a different identity")]
    InvalidReplacement,
}

pub fn transition_attempt(
    current: &OperationAttempt,
    expected_revision: u64,
    event: OperationEvent,
) -> Result<OperationAttempt, TransitionError> {
    if current.revision != expected_revision {
        return Err(TransitionError::StaleRevision {
            expected: expected_revision,
            actual: current.revision,
        });
    }

    let mut next = current.clone();
    match event {
        OperationEvent::Persist => {
            require_state(current, OperationState::Created, "persist")?;
            next.state = OperationState::Persisted;
        }
        OperationEvent::Authorize(receipt) => {
            require_state(current, OperationState::Persisted, "authorize")?;
            next.authorization = Some(receipt);
            next.state = OperationState::Authorized;
        }
        OperationEvent::Queue => {
            require_state(current, OperationState::Authorized, "queue")?;
            next.state = OperationState::Queued;
        }
        OperationEvent::Start { at } => {
            require_state(current, OperationState::Queued, "start")?;
            if current.authorization.is_none() {
                return Err(invalid(current, "start_without_authorization"));
            }
            next.started_at = Some(at);
            next.state = OperationState::Running;
        }
        OperationEvent::MarkEffectPossible => {
            require_state(current, OperationState::Running, "mark_effect_possible")?;
            next.state = OperationState::EffectPossible;
            next.effect_status = EffectStatus::Possible;
        }
        OperationEvent::CompleteSuccess {
            result,
            effect_status,
            finished_at,
        } => {
            require_running(current, "complete_success")?;
            if effect_status == EffectStatus::NotStarted {
                return Err(TransitionError::MissingEffectCertainty);
            }
            next.state = OperationState::Succeeded;
            next.effect_status = effect_status;
            next.finished_at = Some(finished_at);
            next.result = Some(result);
            next.failure = None;
        }
        OperationEvent::CompleteFailure {
            failure,
            effect_status,
            finished_at,
        } => {
            require_running(current, "complete_failure")?;
            if effect_status == EffectStatus::NotStarted {
                return Err(TransitionError::MissingEffectCertainty);
            }
            next.state = OperationState::Failed;
            next.effect_status = effect_status;
            next.finished_at = Some(finished_at);
            next.failure = Some(failure);
            next.result = None;
        }
        OperationEvent::CancelBeforeStart { finished_at } => {
            if !matches!(
                current.state,
                OperationState::Persisted | OperationState::Authorized | OperationState::Queued
            ) {
                return Err(invalid(current, "cancel_before_start"));
            }
            next.state = OperationState::Cancelled;
            next.finished_at = Some(finished_at);
        }
        OperationEvent::RequestCancellation => {
            if !matches!(
                current.state,
                OperationState::Queued | OperationState::Running | OperationState::EffectPossible
            ) || current.cancellation_requested
            {
                return Err(invalid(current, "request_cancellation"));
            }
            next.cancellation_requested = true;
        }
        OperationEvent::Interrupt { at } => {
            if !matches!(
                current.state,
                OperationState::Running | OperationState::EffectPossible
            ) {
                return Err(invalid(current, "interrupt"));
            }
            next.state = OperationState::Interrupted;
            next.finished_at = Some(at);
        }
        OperationEvent::RequireRecovery => {
            require_state(current, OperationState::Interrupted, "require_recovery")?;
            next.state = OperationState::RecoveryRequired;
        }
        OperationEvent::FinalizeRecoveredSuccess {
            result,
            effect_status,
            evidence,
            finished_at,
        } => {
            require_state(
                current,
                OperationState::RecoveryRequired,
                "finalize_recovered_success",
            )?;
            if effect_status == EffectStatus::NotStarted {
                return Err(TransitionError::MissingEffectCertainty);
            }
            next.state = OperationState::Succeeded;
            next.effect_status = effect_status;
            next.finished_at = Some(finished_at);
            next.result = Some(result);
            next.failure = None;
            next.recovery_evidence = Some(evidence);
        }
        OperationEvent::FinalizeRecoveredFailure {
            failure,
            effect_status,
            evidence,
            finished_at,
        } => {
            require_state(
                current,
                OperationState::RecoveryRequired,
                "finalize_recovered_failure",
            )?;
            if effect_status == EffectStatus::NotStarted {
                return Err(TransitionError::MissingEffectCertainty);
            }
            next.state = OperationState::Failed;
            next.effect_status = effect_status;
            next.finished_at = Some(finished_at);
            next.failure = Some(failure);
            next.result = None;
            next.recovery_evidence = Some(evidence);
        }
        OperationEvent::BlockRecovery => {
            require_state(current, OperationState::RecoveryRequired, "block_recovery")?;
            next.state = OperationState::Blocked;
        }
        OperationEvent::MarkVerification(state) => {
            if current.state != OperationState::Succeeded
                || !valid_verification_transition(current.verification, state)
            {
                return Err(TransitionError::InvalidVerificationTransition);
            }
            next.verification = state;
        }
        OperationEvent::Publish { consumer, state } => {
            if consumer.trim().is_empty()
                || !valid_publication_transition(
                    current.publications.get(&consumer).copied(),
                    state,
                )
            {
                return Err(TransitionError::InvalidPublicationTransition);
            }
            next.publications = BTreeMap::from_iter(current.publications.clone());
            next.publications.insert(consumer, state);
        }
        OperationEvent::Supersede { replacement } => {
            if !current.state.is_terminal()
                || current.state == OperationState::Superseded
                || replacement == current.operation_id
            {
                return Err(TransitionError::InvalidReplacement);
            }
            next.state = OperationState::Superseded;
            next.superseded_by = Some(replacement);
        }
    }

    next.revision = current
        .revision
        .checked_add(1)
        .ok_or(TransitionError::RevisionOverflow)?;
    next.validate()
        .map_err(|_| TransitionError::InvalidTransition {
            from: current.state,
            event: "record_validation",
        })?;
    Ok(next)
}

/// Creates a fresh attempt after a durable recovery decision; it never rewrites the prior attempt.
pub fn admit_retry(
    previous: &OperationAttempt,
    expected_revision: u64,
    owner: ExecutionOwner,
    evidence: EvidenceId,
) -> Result<OperationAttempt, TransitionError> {
    if previous.revision != expected_revision {
        return Err(TransitionError::StaleRevision {
            expected: expected_revision,
            actual: previous.revision,
        });
    }
    if previous.state != OperationState::RecoveryRequired {
        return Err(TransitionError::RetryNotReady);
    }

    let attempt_number = previous
        .attempt_number
        .checked_add(1)
        .ok_or(TransitionError::AttemptNumberOverflow)?;
    let mut next = OperationAttempt::new(previous.operation_id, attempt_number, owner)
        .map_err(|_| TransitionError::RetryNotReady)?;
    next.retries_attempt_id = Some(previous.attempt_id);
    next.recovery_evidence = Some(evidence);
    Ok(next)
}

fn require_running(current: &OperationAttempt, event: &'static str) -> Result<(), TransitionError> {
    if matches!(
        current.state,
        OperationState::Running | OperationState::EffectPossible
    ) {
        Ok(())
    } else {
        Err(invalid(current, event))
    }
}

fn require_state(
    current: &OperationAttempt,
    expected: OperationState,
    event: &'static str,
) -> Result<(), TransitionError> {
    if current.state == expected {
        Ok(())
    } else {
        Err(invalid(current, event))
    }
}

fn invalid(current: &OperationAttempt, event: &'static str) -> TransitionError {
    TransitionError::InvalidTransition {
        from: current.state,
        event,
    }
}

fn valid_verification_transition(from: VerificationState, to: VerificationState) -> bool {
    matches!(
        (from, to),
        (VerificationState::NotRequired, VerificationState::Pending)
            | (VerificationState::Pending, VerificationState::Running)
            | (VerificationState::Running, VerificationState::Passed)
            | (VerificationState::Running, VerificationState::Failed)
            | (VerificationState::Running, VerificationState::Inconclusive)
            | (VerificationState::Failed, VerificationState::Pending)
            | (VerificationState::Inconclusive, VerificationState::Pending)
    )
}

fn valid_publication_transition(from: Option<PublicationState>, to: PublicationState) -> bool {
    matches!(
        (from, to),
        (None, PublicationState::Pending)
            | (
                Some(PublicationState::Pending),
                PublicationState::Acknowledged
            )
            | (Some(PublicationState::Pending), PublicationState::Failed)
            | (Some(PublicationState::Failed), PublicationState::Pending)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::runtime::operations::{
        AuthorizationReceipt, ExecutionOwnerId, OperationId, PayloadDigest,
    };

    #[test]
    fn retry_admission_creates_a_new_attempt_and_keeps_lineage() {
        let owner = ExecutionOwner::new(ExecutionOwnerId::new(), 1).unwrap();
        let original = OperationAttempt::new(OperationId::new(), 1, owner).unwrap();
        let mut current = transition_attempt(&original, 0, OperationEvent::Persist).unwrap();
        current = transition_attempt(
            &current,
            1,
            OperationEvent::Authorize(AuthorizationReceipt::new(
                PayloadDigest::of_bytes(b"p"),
                "policy".into(),
                Timestamp::from_unix_millis(1),
            )),
        )
        .unwrap();
        current = transition_attempt(&current, 2, OperationEvent::Queue).unwrap();
        current = transition_attempt(
            &current,
            3,
            OperationEvent::Start {
                at: Timestamp::from_unix_millis(2),
            },
        )
        .unwrap();
        current = transition_attempt(
            &current,
            4,
            OperationEvent::Interrupt {
                at: Timestamp::from_unix_millis(3),
            },
        )
        .unwrap();
        current = transition_attempt(&current, 5, OperationEvent::RequireRecovery).unwrap();
        let next = admit_retry(
            &current,
            6,
            ExecutionOwner::new(ExecutionOwnerId::new(), 2).unwrap(),
            EvidenceId::new(),
        )
        .unwrap();

        assert_eq!(next.operation_id(), current.operation_id());
        assert_eq!(next.attempt_number(), 2);
        assert_ne!(next.attempt_id(), current.attempt_id());
        assert_eq!(next.retries_attempt_id(), Some(current.attempt_id()));
        assert_eq!(next.state(), OperationState::Created);
    }
}
