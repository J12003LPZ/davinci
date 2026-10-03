use super::store_api::JournalError;
use super::store_support::{from_sql_integer, parse_id, sqlite_error};
use super::transitions::TransitionError;
use super::{EffectStatus, ExecutionOwner, OperationAttempt, OperationState};
use rusqlite::{OptionalExtension, Transaction};

pub(super) fn ensure_retry_dispatch_is_unstarted(
    transaction: &Transaction<'_>,
    previous: &OperationAttempt,
) -> Result<(), JournalError> {
    if previous.state() != OperationState::RecoveryRequired {
        return Err(JournalError::Transition(TransitionError::RetryNotReady));
    }
    if previous.effect_status() != EffectStatus::NotStarted {
        return Err(JournalError::RetryEffectUncertain);
    }
    if previous.recovery_evidence().is_none() {
        return Err(JournalError::Transition(TransitionError::RetryNotReady));
    }

    let claim: Option<(String, i64, i64)> = transaction
        .query_row(
            "SELECT owner_id, owner_generation, effect_started
             FROM dispatch_claims WHERE attempt_id = ?1 AND operation_id = ?2",
            rusqlite::params![
                previous.attempt_id().to_string(),
                previous.operation_id().to_string()
            ],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(sqlite_error)?;
    let Some((claim_owner_id, claim_generation, effect_started)) = claim else {
        return Err(JournalError::Transition(TransitionError::RetryNotReady));
    };
    let claim_owner = ExecutionOwner::new(
        parse_id(&claim_owner_id)?,
        from_sql_integer(claim_generation)?,
    )
    .map_err(|error| JournalError::Integrity(error.to_string()))?;
    if claim_owner != previous.owner() {
        return Err(JournalError::Integrity(
            "dispatch claim owner does not match recovered attempt".into(),
        ));
    }
    if effect_started != 0 {
        return Err(JournalError::RetryEffectUncertain);
    }
    Ok(())
}
