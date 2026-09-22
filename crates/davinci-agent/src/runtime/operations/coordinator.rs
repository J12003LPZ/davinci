use super::coordinator_api::{AdmittedOperation, DispatchPermit, EffectPermit, OperationAdmission};
use super::retry_safety::ensure_retry_dispatch_is_unstarted;
use super::store_api::*;
use super::store_support::{
    decode_json, encode_bounded, from_sql_integer, insert_event, now_unix_millis, parse_id,
    state_name, to_sql_integer,
};
use super::transitions::{admit_retry, transition_attempt, OperationEvent, TransitionError};
use super::{
    ExecutionOwner, OperationAttempt, OperationJournal, OperationSpec, OperationState, ResultRef,
    RootNamespaceId, StoredResult, Timestamp,
};
use crate::runtime::EvidenceId;
use rusqlite::{params, OptionalExtension, Transaction};
use serde_json::Value;
use uuid::Uuid;

impl OperationJournal {
    /// Atomically resolves a caller key, persists its call lineage, and creates
    /// the first attempt. Concurrent redelivery observes the same durable row.
    pub fn admit(
        &self,
        spec: &OperationSpec,
        initial_attempt: &OperationAttempt,
    ) -> Result<OperationAdmission, JournalError> {
        spec.validate()
            .map_err(|error| JournalError::Serialization(error.to_string()))?;
        self.validate_spec_binding(spec)?;
        if initial_attempt.operation_id() != spec.operation_id()
            || initial_attempt.revision() != 0
            || initial_attempt.state() != OperationState::Created
            || initial_attempt.attempt_number() != 1
        {
            return Err(JournalError::RootBindingMismatch);
        }
        initial_attempt
            .validate()
            .map_err(|error| JournalError::Serialization(error.to_string()))?;

        let spec_json = encode_bounded(spec, MAX_SPEC_BYTES, "operation specification")?;
        let intent_digest = spec
            .intent_digest()
            .map_err(|error| JournalError::Serialization(error.to_string()))?
            .to_string();
        let payload_digest = spec.payload_digest().to_string();
        let scope = idempotency_scope(spec)?;
        let session_id = spec.context().session_id.clone();
        let wire_call_id = spec.context().wire_tool_call_id.clone();
        let parent_id = spec.context().parent_operation_id.map(|id| id.to_string());
        let persisted_attempt = transition_attempt(initial_attempt, 0, OperationEvent::Persist)?;
        let event_json =
            encode_bounded(&OperationEvent::Persist, MAX_EVENT_BYTES, "operation event")?;
        let attempt_json =
            encode_bounded(&persisted_attempt, MAX_ATTEMPT_BYTES, "operation attempt")?;
        let owner = persisted_attempt.owner();
        let operation_id = spec.operation_id().to_string();
        let attempt_id = persisted_attempt.attempt_id().to_string();

        self.write(|transaction| {
            let existing: Option<(String, String, String)> = transaction
                .query_row(
                    "SELECT operation_id, intent_digest, spec_json FROM operations
                     WHERE root_namespace_id = ?1 AND caller_scope = ?2 AND caller_key = ?3",
                    params![self.root_namespace_id.to_string(), scope, spec.caller_key().key],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()
                .map_err(super::store_support::sqlite_error)?;
            if let Some((existing_id, existing_digest, existing_spec_json)) = existing {
                if existing_digest != intent_digest {
                    return Ok(OperationAdmission::Collision);
                }
                let existing_id = parse_id(&existing_id)?;
                let existing_spec: OperationSpec = decode_json(&existing_spec_json)?;
                let existing_attempt = latest_attempt(transaction, self.root_namespace_id, existing_id)?;
                let result = load_result(transaction, existing_attempt.attempt_id())?;
                if existing_attempt.state().is_terminal() {
                    if existing_attempt.state() == OperationState::Succeeded && result.is_none() {
                        return Err(JournalError::Integrity(
                            "successful operation result is missing".into(),
                        ));
                    }
                    return Ok(OperationAdmission::ExistingResult(AdmittedOperation {
                        spec: existing_spec,
                        attempt: existing_attempt,
                        result,
                    }));
                }
                return Ok(OperationAdmission::ExistingInFlight(AdmittedOperation {
                    spec: existing_spec,
                    attempt: existing_attempt,
                    result,
                }));
            }

            if let Some(wire_call_id) = wire_call_id.as_deref() {
                let mapped: bool = transaction
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM operation_call_mappings
                         WHERE root_namespace_id = ?1 AND session_id = ?2 AND wire_tool_call_id = ?3)",
                        params![self.root_namespace_id.to_string(), session_id, wire_call_id],
                        |row| row.get(0),
                    )
                    .map_err(super::store_support::sqlite_error)?;
                if mapped {
                    return Ok(OperationAdmission::Collision);
                }
            }

            let operation_count: i64 = transaction
                .query_row(
                    "SELECT count(*) FROM operations WHERE root_namespace_id = ?1",
                    [self.root_namespace_id.to_string()],
                    |row| row.get(0),
                )
                .map_err(super::store_support::sqlite_error)?;
            if operation_count >= MAX_OPERATIONS_PER_ROOT {
                return Err(JournalError::Capacity("operation count per root"));
            }
            let duplicate_id: bool = transaction
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM operations WHERE operation_id = ?1)",
                    [&operation_id],
                    |row| row.get(0),
                )
                .map_err(super::store_support::sqlite_error)?;
            if duplicate_id {
                return Err(JournalError::DuplicateOperation);
            }

            transaction
                .execute(
                    "INSERT INTO operations
                     (operation_id, root_namespace_id, caller_scope, caller_key, payload_digest,
                      spec_json, created_at_ms, intent_digest)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        operation_id,
                        self.root_namespace_id.to_string(),
                        scope,
                        spec.caller_key().key,
                        payload_digest,
                        spec_json,
                        now_unix_millis(),
                        intent_digest
                    ],
                )
                .map_err(super::store_support::sqlite_error)?;
            transaction
                .execute(
                    "INSERT INTO attempts
                     (attempt_id, operation_id, attempt_number, revision, owner_id,
                      owner_generation, state, attempt_json, updated_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        attempt_id,
                        operation_id,
                        i64::from(persisted_attempt.attempt_number()),
                        to_sql_integer(persisted_attempt.revision())?,
                        owner.id.to_string(),
                        to_sql_integer(owner.generation)?,
                        state_name(persisted_attempt.state())?,
                        attempt_json,
                        now_unix_millis()
                    ],
                )
                .map_err(super::store_support::sqlite_error)?;
            transaction
                .execute(
                    "INSERT INTO operation_owners (operation_id, owner_id, generation, updated_at_ms)
                     VALUES (?1, ?2, ?3, ?4)",
                    params![
                        operation_id,
                        owner.id.to_string(),
                        to_sql_integer(owner.generation)?,
                        now_unix_millis()
                    ],
                )
                .map_err(super::store_support::sqlite_error)?;
            transaction
                .execute(
                    "INSERT INTO operation_call_mappings
                     (operation_id, root_namespace_id, session_id, caller_scope, caller_key,
                      wire_tool_call_id, parent_operation_id)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        operation_id,
                        self.root_namespace_id.to_string(),
                        session_id,
                        scope,
                        spec.caller_key().key,
                        wire_call_id,
                        parent_id
                    ],
                )
                .map_err(super::store_support::sqlite_error)?;
            insert_event(
                transaction,
                self.root_namespace_id,
                spec.operation_id(),
                persisted_attempt.attempt_id(),
                0,
                persisted_attempt.revision(),
                &event_json,
            )?;
            Ok(OperationAdmission::New(AdmittedOperation {
                spec: spec.clone(),
                attempt: persisted_attempt.clone(),
                result: None,
            }))
        })
    }

    /// Replaces an owner generation only after a host callback has synchronously
    /// quiesced the old owner. This fences writes but does not create a retry.
    pub fn fence_owner_after_quiescence<E>(
        &self,
        operation_id: super::OperationId,
        expected_owner: ExecutionOwner,
        replacement: ExecutionOwner,
        quiesce: impl FnOnce(ExecutionOwner) -> Result<(), E>,
    ) -> Result<(), JournalError> {
        replacement
            .validate()
            .map_err(|error| JournalError::Serialization(error.to_string()))?;
        if replacement.generation <= expected_owner.generation {
            return Err(JournalError::OwnerGenerationNotAdvanced);
        }
        quiesce(expected_owner).map_err(|_| JournalError::OwnerQuiescenceNotConfirmed)?;

        self.write(|transaction| {
            let current: Option<(String, i64)> = transaction
                .query_row(
                    "SELECT owner_id, generation FROM operation_owners
                     WHERE operation_id = ?1
                       AND operation_id IN (SELECT operation_id FROM operations WHERE root_namespace_id = ?2)",
                    params![operation_id.to_string(), self.root_namespace_id.to_string()],
                    |row| Ok((row.get(0)?, row.get(1)?)),
                )
                .optional()
                .map_err(super::store_support::sqlite_error)?;
            let (current_id, current_generation) = current.ok_or(JournalError::NotFound)?;
            let actual = ExecutionOwner::new(
                parse_id(&current_id)?,
                from_sql_integer(current_generation)?,
            )
            .map_err(|error| JournalError::Integrity(error.to_string()))?;
            if actual != expected_owner {
                return Err(JournalError::OwnerFenced {
                    expected: expected_owner,
                    actual,
                });
            }
            let changed = transaction
                .execute(
                    "UPDATE operation_owners SET owner_id = ?1, generation = ?2, updated_at_ms = ?3
                     WHERE operation_id = ?4 AND owner_id = ?5 AND generation = ?6",
                    params![
                        replacement.id.to_string(),
                        to_sql_integer(replacement.generation)?,
                        now_unix_millis(),
                        operation_id.to_string(),
                        expected_owner.id.to_string(),
                        to_sql_integer(expected_owner.generation)?
                    ],
                )
                .map_err(super::store_support::sqlite_error)?;
            if changed != 1 {
                return Err(JournalError::OwnerFenced {
                    expected: expected_owner,
                    actual,
                });
            }
            Ok(())
        })
    }

    /// Moves a claimed queued attempt into recovery after its owner has been
    /// quiesced and replaced. The durable claim proves dispatch never crossed
    /// the effect-start latch.
    pub fn recover_unstarted_dispatch_after_owner_fence(
        &self,
        attempt_id: super::AttemptId,
        expected_revision: u64,
        new_owner: ExecutionOwner,
        evidence: EvidenceId,
    ) -> Result<OperationAttempt, JournalError> {
        new_owner
            .validate()
            .map_err(|error| JournalError::Serialization(error.to_string()))?;
        self.write(|transaction| {
            let (attempt, active_owner) =
                load_owned_attempt(transaction, self.root_namespace_id, attempt_id)?;
            ensure_current_owner(new_owner, active_owner)?;
            if attempt.revision() != expected_revision {
                return Err(JournalError::Transition(TransitionError::StaleRevision {
                    expected: expected_revision,
                    actual: attempt.revision(),
                }));
            }
            let latest =
                latest_attempt(transaction, self.root_namespace_id, attempt.operation_id())?;
            if latest.attempt_id() != attempt_id {
                return Err(JournalError::Transition(TransitionError::RetryNotReady));
            }
            if attempt.owner().generation >= new_owner.generation {
                return Err(JournalError::OwnerGenerationNotAdvanced);
            }
            if attempt.state() != OperationState::Queued
                || attempt.effect_status() != super::EffectStatus::NotStarted
            {
                return Err(JournalError::DispatchNotReady);
            }

            let claim: Option<(String, i64, i64)> = transaction
                .query_row(
                    "SELECT owner_id, owner_generation, effect_started
                     FROM dispatch_claims WHERE attempt_id = ?1 AND operation_id = ?2",
                    params![attempt_id.to_string(), attempt.operation_id().to_string()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()
                .map_err(super::store_support::sqlite_error)?;
            let Some((claim_owner_id, claim_generation, effect_started)) = claim else {
                return Err(JournalError::DispatchNotReady);
            };
            let claim_owner = ExecutionOwner::new(
                parse_id(&claim_owner_id)?,
                from_sql_integer(claim_generation)?,
            )
            .map_err(|error| JournalError::Integrity(error.to_string()))?;
            if claim_owner != attempt.owner() {
                return Err(JournalError::Integrity(
                    "dispatch claim owner does not match queued attempt".into(),
                ));
            }
            if effect_started != 0 {
                return Err(JournalError::RetryEffectUncertain);
            }

            let event = OperationEvent::RecoverUnstartedDispatch { evidence };
            let recovered = transition_attempt(&attempt, expected_revision, event.clone())?;
            persist_attempt_transition(
                transaction,
                self.root_namespace_id,
                &attempt,
                &recovered,
                &event,
            )?;
            Ok(recovered)
        })
    }

    /// Persists a new monotonic attempt after the old owner has been fenced.
    /// Retries are allowed here only when the prior attempt proves no effect
    /// started; uncertain effects require the recovery policy path.
    pub fn admit_retry_after_owner_fence(
        &self,
        previous_attempt_id: super::AttemptId,
        expected_revision: u64,
        new_owner: ExecutionOwner,
        evidence: EvidenceId,
    ) -> Result<OperationAttempt, JournalError> {
        new_owner
            .validate()
            .map_err(|error| JournalError::Serialization(error.to_string()))?;
        self.write(|transaction| {
            let (previous, active_owner) =
                load_owned_attempt(transaction, self.root_namespace_id, previous_attempt_id)?;
            ensure_current_owner(new_owner, active_owner)?;
            if previous.revision() != expected_revision {
                return Err(JournalError::Transition(TransitionError::StaleRevision {
                    expected: expected_revision,
                    actual: previous.revision(),
                }));
            }
            let latest =
                latest_attempt(transaction, self.root_namespace_id, previous.operation_id())?;
            if latest.attempt_id() != previous_attempt_id {
                return Err(JournalError::Transition(TransitionError::RetryNotReady));
            }
            if previous.owner().generation >= new_owner.generation {
                return Err(JournalError::OwnerGenerationNotAdvanced);
            }
            ensure_retry_dispatch_is_unstarted(transaction, &previous)?;

            let created = admit_retry(&previous, expected_revision, new_owner, evidence)?;
            let event = OperationEvent::Persist;
            let persisted = transition_attempt(&created, 0, event.clone())?;
            let attempt_json = encode_bounded(&persisted, MAX_ATTEMPT_BYTES, "operation attempt")?;
            let event_json = encode_bounded(&event, MAX_EVENT_BYTES, "operation event")?;
            transaction
                .execute(
                    "INSERT INTO attempts
                     (attempt_id, operation_id, attempt_number, revision, owner_id,
                      owner_generation, state, attempt_json, updated_at_ms)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                    params![
                        persisted.attempt_id().to_string(),
                        persisted.operation_id().to_string(),
                        i64::from(persisted.attempt_number()),
                        to_sql_integer(persisted.revision())?,
                        new_owner.id.to_string(),
                        to_sql_integer(new_owner.generation)?,
                        state_name(persisted.state())?,
                        attempt_json,
                        now_unix_millis()
                    ],
                )
                .map_err(super::store_support::sqlite_error)?;
            insert_event(
                transaction,
                self.root_namespace_id,
                persisted.operation_id(),
                persisted.attempt_id(),
                0,
                persisted.revision(),
                &event_json,
            )?;
            Ok(persisted)
        })
    }

    /// Claims an authorized queued attempt without invoking an adapter or
    /// granting authorization. Only this journal can mint the in-process token.
    pub fn claim_dispatch(
        &self,
        owner: ExecutionOwner,
        attempt_id: super::AttemptId,
        expected_revision: u64,
    ) -> Result<DispatchPermit, JournalError> {
        owner
            .validate()
            .map_err(|error| JournalError::Serialization(error.to_string()))?;
        self.write(|transaction| {
            let (attempt, active_owner) = load_owned_attempt(
                transaction,
                self.root_namespace_id,
                attempt_id,
            )?;
            ensure_current_owner(owner, active_owner)?;
            ensure_attempt_owner(owner, attempt.owner())?;
            if attempt.revision() != expected_revision {
                return Err(JournalError::Transition(TransitionError::StaleRevision {
                    expected: expected_revision,
                    actual: attempt.revision(),
                }));
            }
            if attempt.state() != OperationState::Queued || attempt.authorization().is_none() {
                return Err(JournalError::DispatchNotReady);
            }
            let spec_json: String = transaction
                .query_row(
                    "SELECT spec_json FROM operations WHERE operation_id = ?1 AND root_namespace_id = ?2",
                    params![attempt.operation_id().to_string(), self.root_namespace_id.to_string()],
                    |row| row.get(0),
                )
                .map_err(super::store_support::sqlite_error)?;
            let spec: OperationSpec = decode_json(&spec_json)?;
            let intent_digest = spec
                .intent_digest()
                .map_err(|error| JournalError::Serialization(error.to_string()))?;
            if attempt
                .authorization()
                .map_or(true, |receipt| receipt.approved_payload_digest != intent_digest)
            {
                return Err(JournalError::AuthorizationDigestMismatch);
            }
            let already_claimed: bool = transaction
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM dispatch_claims WHERE attempt_id = ?1)",
                    [attempt_id.to_string()],
                    |row| row.get(0),
                )
                .map_err(super::store_support::sqlite_error)?;
            if already_claimed {
                return Err(JournalError::AttemptAlreadyClaimed);
            }
            let nonce = Uuid::now_v7();
            transaction
                .execute(
                    "INSERT INTO dispatch_claims
                     (attempt_id, operation_id, owner_id, owner_generation, permit_nonce, claimed_at_ms, effect_started)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0)",
                    params![
                        attempt_id.to_string(),
                        attempt.operation_id().to_string(),
                        owner.id.to_string(),
                        to_sql_integer(owner.generation)?,
                        nonce.to_string(),
                        now_unix_millis()
                    ],
                )
                .map_err(super::store_support::sqlite_error)?;
            Ok(DispatchPermit {
                journal_id: self.identity.journal_id,
                root_namespace_id: self.root_namespace_id,
                operation_id: attempt.operation_id(),
                attempt_id,
                owner,
                revision: expected_revision,
                nonce,
            })
        })
    }

    /// Durably records both dispatch start and the effect-start latch before
    /// returning a single-use capability to invoke the host adapter.
    pub fn latch_effect_start(
        &self,
        permit: DispatchPermit,
        started_at: Timestamp,
    ) -> Result<EffectPermit, JournalError> {
        if permit.journal_id != self.identity.journal_id
            || permit.root_namespace_id != self.root_namespace_id
        {
            return Err(JournalError::InvalidDispatchPermit);
        }
        self.write(|transaction| {
            let claim: Option<(String, String, i64, String, i64)> = transaction
                .query_row(
                    "SELECT operation_id, owner_id, owner_generation, permit_nonce, effect_started
                     FROM dispatch_claims WHERE attempt_id = ?1",
                    [permit.attempt_id.to_string()],
                    |row| {
                        Ok((
                            row.get(0)?,
                            row.get(1)?,
                            row.get(2)?,
                            row.get(3)?,
                            row.get(4)?,
                        ))
                    },
                )
                .optional()
                .map_err(super::store_support::sqlite_error)?;
            let Some((operation_id, owner_id, owner_generation, nonce, effect_started)) = claim
            else {
                return Err(JournalError::InvalidDispatchPermit);
            };
            let claim_owner =
                ExecutionOwner::new(parse_id(&owner_id)?, from_sql_integer(owner_generation)?)
                    .map_err(|error| JournalError::Integrity(error.to_string()))?;
            if parse_id::<super::OperationId>(&operation_id)? != permit.operation_id
                || claim_owner != permit.owner
                || nonce != permit.nonce.to_string()
                || effect_started != 0
            {
                return Err(JournalError::InvalidDispatchPermit);
            }

            let (attempt, active_owner) =
                load_owned_attempt(transaction, self.root_namespace_id, permit.attempt_id)?;
            ensure_current_owner(permit.owner, active_owner)?;
            ensure_attempt_owner(permit.owner, attempt.owner())?;
            if attempt.operation_id() != permit.operation_id
                || attempt.revision() != permit.revision
                || attempt.state() != OperationState::Queued
                || attempt.authorization().is_none()
            {
                return Err(JournalError::InvalidDispatchPermit);
            }

            let start_event = OperationEvent::Start { at: started_at };
            let started = transition_attempt(&attempt, permit.revision, start_event.clone())?;
            persist_attempt_transition(
                transaction,
                self.root_namespace_id,
                &attempt,
                &started,
                &start_event,
            )?;
            let latch_event = OperationEvent::MarkEffectPossible;
            let latched = transition_attempt(&started, started.revision(), latch_event.clone())?;
            persist_attempt_transition(
                transaction,
                self.root_namespace_id,
                &started,
                &latched,
                &latch_event,
            )?;
            let changed = transaction
                .execute(
                    "UPDATE dispatch_claims SET effect_started = 1
                     WHERE attempt_id = ?1 AND permit_nonce = ?2 AND effect_started = 0",
                    params![permit.attempt_id.to_string(), permit.nonce.to_string()],
                )
                .map_err(super::store_support::sqlite_error)?;
            if changed != 1 {
                return Err(JournalError::InvalidDispatchPermit);
            }
            Ok(EffectPermit {
                journal_id: self.identity.journal_id,
                root_namespace_id: self.root_namespace_id,
                operation_id: permit.operation_id,
                attempt_id: permit.attempt_id,
                owner: permit.owner,
            })
        })
    }

    pub(super) fn consume_effect_permit(&self, permit: &EffectPermit) -> Result<(), JournalError> {
        if permit.journal_id != self.identity.journal_id
            || permit.root_namespace_id != self.root_namespace_id
        {
            return Err(JournalError::InvalidDispatchPermit);
        }
        self.read(|transaction| {
            let (attempt, active_owner) =
                load_owned_attempt(transaction, self.root_namespace_id, permit.attempt_id)?;
            ensure_current_owner(permit.owner, active_owner)?;
            ensure_attempt_owner(permit.owner, attempt.owner())?;
            if attempt.operation_id() != permit.operation_id
                || attempt.state() != OperationState::EffectPossible
                || attempt.effect_status() == super::EffectStatus::NotStarted
            {
                return Err(JournalError::InvalidDispatchPermit);
            }
            let valid_claim: bool = transaction
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM dispatch_claims
                     WHERE attempt_id = ?1 AND operation_id = ?2 AND owner_id = ?3
                       AND owner_generation = ?4 AND effect_started = 1)",
                    params![
                        permit.attempt_id.to_string(),
                        permit.operation_id.to_string(),
                        permit.owner.id.to_string(),
                        to_sql_integer(permit.owner.generation)?
                    ],
                    |row| row.get(0),
                )
                .map_err(super::store_support::sqlite_error)?;
            if !valid_claim {
                return Err(JournalError::InvalidDispatchPermit);
            }
            Ok(())
        })
    }
}

fn idempotency_scope(spec: &OperationSpec) -> Result<String, JournalError> {
    serde_json::to_value(spec.caller_key().scope)
        .map_err(|error| JournalError::Serialization(error.to_string()))?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| JournalError::Serialization("invalid idempotency scope".into()))
}

fn latest_attempt(
    transaction: &Transaction<'_>,
    root_namespace_id: RootNamespaceId,
    operation_id: super::OperationId,
) -> Result<OperationAttempt, JournalError> {
    let value: Option<String> = transaction
        .query_row(
            "SELECT a.attempt_json FROM attempts a
             JOIN operations o ON o.operation_id = a.operation_id
             WHERE a.operation_id = ?1 AND o.root_namespace_id = ?2
             ORDER BY a.attempt_number DESC LIMIT 1",
            params![operation_id.to_string(), root_namespace_id.to_string()],
            |row| row.get(0),
        )
        .optional()
        .map_err(super::store_support::sqlite_error)?;
    decode_json(&value.ok_or(JournalError::NotFound)?)
}

fn load_result(
    transaction: &Transaction<'_>,
    attempt_id: super::AttemptId,
) -> Result<Option<StoredResult>, JournalError> {
    let value: Option<(String, String, i64)> = transaction
        .query_row(
            "SELECT result_ref_json, payload_json, created_at_ms
             FROM operation_results WHERE attempt_id = ?1",
            [attempt_id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(super::store_support::sqlite_error)?;
    value
        .map(|(reference, payload, created_at_ms)| {
            Ok(StoredResult {
                reference: decode_json::<ResultRef>(&reference)?,
                payload: decode_json::<Value>(&payload)?,
                created_at_ms: from_sql_integer(created_at_ms)?,
            })
        })
        .transpose()
}

fn load_owned_attempt(
    transaction: &Transaction<'_>,
    root_namespace_id: RootNamespaceId,
    attempt_id: super::AttemptId,
) -> Result<(OperationAttempt, ExecutionOwner), JournalError> {
    let row: Option<(String, String, i64)> = transaction
        .query_row(
            "SELECT a.attempt_json, oo.owner_id, oo.generation
             FROM attempts a
             JOIN operations o ON o.operation_id = a.operation_id
             JOIN operation_owners oo ON oo.operation_id = o.operation_id
             WHERE a.attempt_id = ?1 AND o.root_namespace_id = ?2",
            params![attempt_id.to_string(), root_namespace_id.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .map_err(super::store_support::sqlite_error)?;
    let (attempt_json, owner_id, generation) = row.ok_or(JournalError::NotFound)?;
    let attempt = decode_json(&attempt_json)?;
    let owner = ExecutionOwner::new(parse_id(&owner_id)?, from_sql_integer(generation)?)
        .map_err(|error| JournalError::Integrity(error.to_string()))?;
    Ok((attempt, owner))
}

fn ensure_current_owner(
    expected: ExecutionOwner,
    actual: ExecutionOwner,
) -> Result<(), JournalError> {
    if expected == actual {
        Ok(())
    } else {
        Err(JournalError::OwnerFenced { expected, actual })
    }
}

fn ensure_attempt_owner(
    expected: ExecutionOwner,
    actual: ExecutionOwner,
) -> Result<(), JournalError> {
    if expected == actual {
        Ok(())
    } else {
        Err(JournalError::OwnerFenced { expected, actual })
    }
}

fn persist_attempt_transition(
    transaction: &Transaction<'_>,
    root_namespace_id: RootNamespaceId,
    current: &OperationAttempt,
    next: &OperationAttempt,
    event: &OperationEvent,
) -> Result<(), JournalError> {
    let event_json = encode_bounded(event, MAX_EVENT_BYTES, "operation event")?;
    let attempt_json = encode_bounded(next, MAX_ATTEMPT_BYTES, "operation attempt")?;
    let changed = transaction
        .execute(
            "UPDATE attempts SET revision = ?1, state = ?2, attempt_json = ?3, updated_at_ms = ?4
             WHERE attempt_id = ?5 AND revision = ?6",
            params![
                to_sql_integer(next.revision())?,
                state_name(next.state())?,
                attempt_json,
                now_unix_millis(),
                next.attempt_id().to_string(),
                to_sql_integer(current.revision())?
            ],
        )
        .map_err(super::store_support::sqlite_error)?;
    if changed != 1 {
        return Err(JournalError::Transition(TransitionError::StaleRevision {
            expected: current.revision(),
            actual: next.revision(),
        }));
    }
    insert_event(
        transaction,
        root_namespace_id,
        next.operation_id(),
        next.attempt_id(),
        current.revision(),
        next.revision(),
        &event_json,
    )?;
    Ok(())
}
