use super::migrations;
use super::store_api::*;
use super::store_support::*;
use super::transitions::{transition_attempt, OperationEvent, TransitionError};
use super::{
    AttemptId, ExecutionOwner, OperationAdmission, OperationAttempt, OperationSpec, PayloadDigest,
    RootNamespaceId, Timestamp,
};
use crate::runtime::cache::directory::{Directory, DirectoryLease};
use rusqlite::{
    params, Connection, OpenFlags, OptionalExtension, Transaction, TransactionBehavior,
};
use serde_json::Value;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard, TryLockError};
use std::time::Duration;
use uuid::Uuid;

const BUSY_TIMEOUT: Duration = Duration::from_secs(5);
const LEASE_DIRECTORY: &str = ".operation-journal-leases";
const MIGRATION_LEASE_DIRECTORY: &str = "migration";
pub(super) struct JournalState {
    connection: Connection,
}

/// A synchronous durable boundary for one root namespace. Writes use a single
/// nonblocking lane: contention returns `WriterBusy`, so no unbounded queue can
/// accumulate before a durable intent acknowledgement.
pub struct OperationJournal {
    pub(super) identity: JournalIdentity,
    pub(super) root_namespace_id: RootNamespaceId,
    directory: Directory,
    _root_lease_directory: Directory,
    _root_lease: DirectoryLease,
    poisoned: AtomicBool,
    state: Mutex<JournalState>,
}

impl OperationJournal {
    pub fn open(
        directory_path: &Path,
        identity: JournalIdentity,
        root_namespace_id: RootNamespaceId,
    ) -> Result<Self, JournalError> {
        if !directory_path.is_absolute() {
            return Err(JournalError::RelativeDirectory);
        }
        let directory = Directory::open(directory_path, true).map_err(io_error)?;
        match directory.file(JOURNAL_DATABASE_FILE_NAME, true) {
            Ok(file) => drop(file),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                drop(
                    directory
                        .file(JOURNAL_DATABASE_FILE_NAME, false)
                        .map_err(io_error)?,
                );
            }
            Err(error) => return Err(io_error(error)),
        }

        let lease_root =
            Directory::open(&directory.path.join(LEASE_DIRECTORY), true).map_err(io_error)?;
        let migration_directory =
            Directory::open(&lease_root.path.join(MIGRATION_LEASE_DIRECTORY), true)
                .map_err(io_error)?;
        let migration_lease = acquire_lease(&migration_directory, true)?;
        let database_path = directory.path.join(JOURNAL_DATABASE_FILE_NAME);
        let mut connection = Connection::open_with_flags(
            &database_path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NOFOLLOW
                | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
        )
        .map_err(sqlite_error)?;
        connection
            .busy_timeout(BUSY_TIMEOUT)
            .map_err(sqlite_error)?;
        connection
            .pragma_update(None, "foreign_keys", "ON")
            .map_err(sqlite_error)?;
        connection
            .pragma_update(None, "trusted_schema", "OFF")
            .map_err(sqlite_error)?;
        connection
            .pragma_update(None, "synchronous", "FULL")
            .map_err(sqlite_error)?;
        migrations::initialize(&mut connection, &identity)?;
        connection
            .pragma_update(None, "journal_mode", "WAL")
            .map_err(sqlite_error)?;
        connection
            .pragma_update(None, "max_page_count", MAX_DATABASE_PAGES)
            .map_err(sqlite_error)?;
        let page_count: i64 = connection
            .pragma_query_value(None, "page_count", |row| row.get(0))
            .map_err(sqlite_error)?;
        if page_count > MAX_DATABASE_PAGES {
            return Err(JournalError::Capacity("journal database page bound"));
        }
        drop(migration_lease);
        drop(migration_directory);
        drop(lease_root);

        let root_lease_directory = Directory::open(
            &directory
                .path
                .join(LEASE_DIRECTORY)
                .join(root_namespace_id.to_string()),
            true,
        )
        .map_err(io_error)?;
        let root_lease = acquire_lease(&root_lease_directory, false)?;
        register_root(&mut connection, &identity, root_namespace_id)?;

        Ok(Self {
            identity,
            root_namespace_id,
            directory,
            _root_lease_directory: root_lease_directory,
            _root_lease: root_lease,
            poisoned: AtomicBool::new(false),
            state: Mutex::new(JournalState { connection }),
        })
    }

    pub fn persist_intent(
        &self,
        spec: &OperationSpec,
        initial_attempt: &OperationAttempt,
    ) -> Result<OperationAttempt, JournalError> {
        match self.admit(spec, initial_attempt)? {
            OperationAdmission::New(admitted) => Ok(admitted.attempt),
            OperationAdmission::ExistingInFlight(admitted)
            | OperationAdmission::ExistingResult(admitted) => Err(JournalError::DuplicateIntent(
                admitted.attempt.operation_id(),
            )),
            OperationAdmission::Collision => Err(JournalError::IdempotencyCollision),
        }
    }

    pub fn transition(
        &self,
        owner: ExecutionOwner,
        attempt_id: AttemptId,
        expected_revision: u64,
        event: OperationEvent,
        result_payload: Option<Value>,
        outbox: Vec<OutboxDraft>,
    ) -> Result<OperationAttempt, JournalError> {
        if outbox.len() > MAX_OUTBOX_BATCH_SIZE {
            return Err(JournalError::OutboxBatchTooLarge {
                actual: outbox.len(),
                limit: MAX_OUTBOX_BATCH_SIZE,
            });
        }
        for draft in &outbox {
            if draft.consumer.trim().is_empty() || draft.consumer.len() > MAX_CONSUMER_BYTES {
                return Err(JournalError::InvalidConsumer);
            }
        }
        let event_json = encode_bounded(&event, MAX_EVENT_BYTES, "operation event")?;
        let encoded_outbox = outbox
            .iter()
            .map(|draft| encode_bounded(&draft.payload, MAX_OUTBOX_BYTES, "outbox payload"))
            .collect::<Result<Vec<_>, _>>()?;
        let encoded_payload = result_payload
            .as_ref()
            .map(|payload| encode_bounded(payload, MAX_RESULT_BYTES, "result payload"))
            .transpose()?;

        self.write(|transaction| {
            let row: Option<(String, String, i64, String)> = transaction
                .query_row(
                    "SELECT a.attempt_json, oo.owner_id, oo.generation, o.spec_json
                     FROM attempts a
                     JOIN operations o ON o.operation_id = a.operation_id
                     JOIN operation_owners oo ON oo.operation_id = o.operation_id
                     WHERE a.attempt_id = ?1 AND o.root_namespace_id = ?2",
                    params![attempt_id.to_string(), self.root_namespace_id.to_string()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
                )
                .optional()
                .map_err(sqlite_error)?;
            let (current_json, owner_id, owner_generation, spec_json) =
                row.ok_or(JournalError::NotFound)?;
            let active_owner = ExecutionOwner::new(
                parse_id(&owner_id)?,
                from_sql_integer(owner_generation)?,
            )
            .map_err(|error| JournalError::Integrity(error.to_string()))?;
            if owner != active_owner {
                return Err(JournalError::OwnerFenced {
                    expected: owner,
                    actual: active_owner,
                });
            }
            let current: OperationAttempt = decode_json(&current_json)?;
            if current.owner() != owner {
                return Err(JournalError::OwnerFenced {
                    expected: owner,
                    actual: current.owner(),
                });
            }
            if let OperationEvent::Authorize(receipt) = &event {
                let spec: OperationSpec = decode_json(&spec_json)?;
                let intent_digest = spec
                    .intent_digest()
                    .map_err(|error| JournalError::Serialization(error.to_string()))?;
                if receipt.approved_payload_digest != intent_digest {
                    return Err(JournalError::AuthorizationDigestMismatch);
                }
            }
            if matches!(
                &event,
                OperationEvent::Start { .. } | OperationEvent::MarkEffectPossible
            ) {
                return Err(JournalError::DispatchPermitRequired);
            }
            if matches!(&event, OperationEvent::RecoverUnstartedDispatch { .. }) {
                return Err(JournalError::CoordinatorTransitionRequired);
            }
            let next = transition_attempt(&current, expected_revision, event.clone())?;
            let new_result = current.result().is_none() && next.result().is_some();
            let result_record = match (new_result, result_payload.as_ref(), encoded_payload.as_ref()) {
                (true, Some(payload), Some(payload_json)) => {
                    let reference = next.result().expect("new result is present");
                    if PayloadDigest::of_json(payload)
                        .map_err(|error| JournalError::Serialization(error.to_string()))?
                        != reference.payload_digest
                    {
                        return Err(JournalError::ResultDigestMismatch);
                    }
                    let reference_json = encode_bounded(
                        reference,
                        MAX_RESULT_REF_BYTES,
                        "result reference",
                    )?;
                    Some((reference.result_id.to_string(), reference_json, payload_json.clone()))
                }
                (true, _, _) => return Err(JournalError::MissingResultPayload),
                (false, Some(_), _) => return Err(JournalError::UnexpectedResultPayload),
                (false, None, None) => None,
                (false, None, Some(_)) => return Err(JournalError::UnexpectedResultPayload),
            };
            if outbox.len() > 0 {
                let pending: i64 = transaction
                    .query_row(
                        "SELECT count(*) FROM operation_outbox
                         WHERE root_namespace_id = ?1 AND state = 'pending'",
                        [self.root_namespace_id.to_string()],
                        |row| row.get(0),
                    )
                    .map_err(sqlite_error)?;
                if pending.saturating_add(outbox.len() as i64) > MAX_PENDING_OUTBOX as i64 {
                    return Err(JournalError::Capacity("pending outbox messages per root"));
                }
            }
            let prior_revision = current.revision();
            let attempt_json = encode_bounded(&next, MAX_ATTEMPT_BYTES, "operation attempt")?;
            let changed = transaction
                .execute(
                    "UPDATE attempts SET revision = ?1, state = ?2, attempt_json = ?3, updated_at_ms = ?4
                     WHERE attempt_id = ?5 AND revision = ?6",
                    params![
                        to_sql_integer(next.revision())?,
                        state_name(next.state())?,
                        attempt_json,
                        now_unix_millis(),
                        attempt_id.to_string(),
                        to_sql_integer(expected_revision)?
                    ],
                )
                .map_err(sqlite_error)?;
            if changed != 1 {
                return Err(JournalError::Transition(TransitionError::StaleRevision {
                    expected: expected_revision,
                    actual: current.revision(),
                }));
            }
            let event_sequence = insert_event(
                transaction,
                self.root_namespace_id,
                next.operation_id(),
                next.attempt_id(),
                prior_revision,
                next.revision(),
                &event_json,
            )?;
            if let Some((result_id, reference_json, payload_json)) = result_record {
                transaction
                    .execute(
                        "INSERT INTO operation_results
                         (result_id, operation_id, attempt_id, payload_digest, result_ref_json, payload_json, created_at_ms)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                        params![
                            result_id,
                            next.operation_id().to_string(),
                            next.attempt_id().to_string(),
                            next.result().expect("new result is present").payload_digest.to_string(),
                            reference_json,
                            payload_json,
                            now_unix_millis()
                        ],
                    )
                    .map_err(sqlite_error)?;
            }
            for (draft, payload_json) in outbox.iter().zip(encoded_outbox.iter()) {
                transaction
                    .execute(
                        "INSERT INTO operation_outbox
                         (outbox_id, root_namespace_id, operation_id, attempt_id, event_sequence,
                          consumer, payload_json, state, created_at_ms, acknowledged_at_ms)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'pending', ?8, NULL)",
                        params![
                            Uuid::now_v7().to_string(),
                            self.root_namespace_id.to_string(),
                            next.operation_id().to_string(),
                            next.attempt_id().to_string(),
                            event_sequence,
                            draft.consumer,
                            payload_json,
                            now_unix_millis()
                        ],
                    )
                    .map_err(sqlite_error)?;
            }
            Ok(next)
        })
    }

    pub fn load_attempt(&self, attempt_id: AttemptId) -> Result<OperationAttempt, JournalError> {
        self.read(|transaction| {
            let json: Option<String> = transaction
                .query_row(
                    "SELECT a.attempt_json FROM attempts a
                     JOIN operations o ON o.operation_id = a.operation_id
                     WHERE a.attempt_id = ?1 AND o.root_namespace_id = ?2",
                    params![attempt_id.to_string(), self.root_namespace_id.to_string()],
                    |row| row.get(0),
                )
                .optional()
                .map_err(sqlite_error)?;
            decode_json(&json.ok_or(JournalError::NotFound)?)
        })
    }

    pub fn snapshot(&self) -> Result<OperationJournalSnapshot, JournalError> {
        self.read(|transaction| {
            let counts: (i64, i64, i64, i64, i64) = transaction
                .query_row(
                    "SELECT
                       (SELECT count(*) FROM operations WHERE root_namespace_id = ?1),
                       (SELECT count(*) FROM attempts a JOIN operations o USING(operation_id) WHERE o.root_namespace_id = ?1),
                       (SELECT count(*) FROM operation_events WHERE root_namespace_id = ?1),
                       (SELECT count(*) FROM operation_results r JOIN operations o USING(operation_id) WHERE o.root_namespace_id = ?1),
                       (SELECT count(*) FROM operation_outbox WHERE root_namespace_id = ?1)",
                    [self.root_namespace_id.to_string()],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?)),
                )
                .map_err(sqlite_error)?;
            let count = counts.0 + counts.1 + counts.2 + counts.3 + counts.4;
            if count > MAX_SNAPSHOT_RECORDS {
                return Err(JournalError::SnapshotTooLarge {
                    actual: count,
                    limit: MAX_SNAPSHOT_RECORDS,
                });
            }
            let operations = collect_json::<OperationSpec>(
                transaction,
                "SELECT spec_json FROM operations WHERE root_namespace_id = ?1 ORDER BY created_at_ms, operation_id",
                self.root_namespace_id,
            )?;
            let attempts = collect_json::<OperationAttempt>(
                transaction,
                "SELECT a.attempt_json FROM attempts a JOIN operations o USING(operation_id)
                 WHERE o.root_namespace_id = ?1 ORDER BY o.created_at_ms, a.attempt_number",
                self.root_namespace_id,
            )?;
            let events = collect_events(transaction, self.root_namespace_id)?;
            let results = collect_results(transaction, self.root_namespace_id)?;
            let outbox = collect_outbox(
                transaction,
                self.root_namespace_id,
                false,
                MAX_SNAPSHOT_RECORDS as usize,
            )?;
            Ok(OperationJournalSnapshot {
                journal_id: self.identity.journal_id,
                root_namespace_id: self.root_namespace_id,
                workspace: self.identity.workspace.clone(),
                operations,
                attempts,
                events,
                results,
                outbox,
            })
        })
    }

    pub fn pending_outbox(&self, limit: usize) -> Result<Vec<StoredOutbox>, JournalError> {
        if limit > MAX_OUTBOX_BATCH_SIZE {
            return Err(JournalError::OutboxBatchTooLarge {
                actual: limit,
                limit: MAX_OUTBOX_BATCH_SIZE,
            });
        }
        self.read(|transaction| collect_outbox(transaction, self.root_namespace_id, true, limit))
    }

    pub fn acknowledge_outbox(
        &self,
        id: Uuid,
        acknowledged_at: Timestamp,
    ) -> Result<bool, JournalError> {
        self.write(|transaction| {
            let exists: bool = transaction
                .query_row(
                    "SELECT EXISTS(SELECT 1 FROM operation_outbox
                     WHERE outbox_id = ?1 AND root_namespace_id = ?2)",
                    params![id.to_string(), self.root_namespace_id.to_string()],
                    |row| row.get(0),
                )
                .map_err(sqlite_error)?;
            if !exists {
                return Err(JournalError::OutboxNotFound);
            }
            let updated = transaction
                .execute(
                    "UPDATE operation_outbox SET state = 'acknowledged', acknowledged_at_ms = ?1
                     WHERE outbox_id = ?2 AND root_namespace_id = ?3 AND state = 'pending'",
                    params![
                        to_sql_integer(acknowledged_at.unix_millis())?,
                        id.to_string(),
                        self.root_namespace_id.to_string()
                    ],
                )
                .map_err(sqlite_error)?;
            Ok(updated == 1)
        })
    }

    /// Export a SQLite-consistent snapshot. The backup API reads committed WAL
    /// pages and produces one standalone database in a separate private folder.
    pub fn backup_to(&self, destination_directory: &Path) -> Result<(), JournalError> {
        if !destination_directory.is_absolute() {
            return Err(JournalError::InvalidBackupPath);
        }
        let destination = Directory::open(destination_directory, true).map_err(io_error)?;
        if destination.identity().map_err(io_error)?
            == self.directory.identity().map_err(io_error)?
        {
            return Err(JournalError::InvalidBackupPath);
        }
        let file = match destination.file(JOURNAL_DATABASE_FILE_NAME, true) {
            Ok(file) => file,
            Err(error) => return Err(io_error(error)),
        };
        let result = (|| {
            let mut target = Connection::open_with_flags(
                &destination.path.join(JOURNAL_DATABASE_FILE_NAME),
                OpenFlags::SQLITE_OPEN_READ_WRITE
                    | OpenFlags::SQLITE_OPEN_CREATE
                    | OpenFlags::SQLITE_OPEN_NOFOLLOW
                    | OpenFlags::SQLITE_OPEN_FULL_MUTEX,
            )
            .map_err(sqlite_error)?;
            let state = self.try_state()?;
            if self.poisoned.load(Ordering::Acquire) {
                return Err(JournalError::Poisoned);
            }
            let backup = rusqlite::backup::Backup::new(&state.connection, &mut target)
                .map_err(sqlite_error)?;
            backup
                .run_to_completion(128, Duration::from_millis(10), None)
                .map_err(sqlite_error)?;
            drop(backup);
            target
                .pragma_update(None, "journal_mode", "DELETE")
                .map_err(sqlite_error)?;
            let integrity: String = target
                .query_row("PRAGMA quick_check(1)", [], |row| row.get(0))
                .map_err(sqlite_error)?;
            if integrity != "ok" {
                return Err(JournalError::Integrity(integrity));
            }
            drop(target);
            file.sync_all().map_err(io_error)?;
            destination.sync().map_err(io_error)?;
            Ok(())
        })();
        if result.is_err() {
            let _ = destination.remove_source(JOURNAL_DATABASE_FILE_NAME);
            let _ = destination.remove_source(&format!("{JOURNAL_DATABASE_FILE_NAME}-wal"));
            let _ = destination.remove_source(&format!("{JOURNAL_DATABASE_FILE_NAME}-shm"));
        }
        result
    }

    pub(super) fn validate_spec_binding(&self, spec: &OperationSpec) -> Result<(), JournalError> {
        let context = spec.context();
        if context.journal_id != self.identity.journal_id
            || context.workspace.id != self.identity.workspace.id
            || context.workspace.binding_version != self.identity.workspace.binding_version
        {
            return Err(JournalError::RootBindingMismatch);
        }
        if context.root_namespace_id != self.root_namespace_id {
            return Err(JournalError::RootMismatch {
                expected: self.root_namespace_id,
                actual: context.root_namespace_id,
            });
        }
        Ok(())
    }

    fn try_state(&self) -> Result<MutexGuard<'_, JournalState>, JournalError> {
        match self.state.try_lock() {
            Ok(state) => Ok(state),
            Err(TryLockError::WouldBlock) => Err(JournalError::WriterBusy),
            Err(TryLockError::Poisoned(_)) => Err(JournalError::Poisoned),
        }
    }

    pub(super) fn write<R>(
        &self,
        action: impl FnOnce(&Transaction<'_>) -> Result<R, JournalError>,
    ) -> Result<R, JournalError> {
        let mut state = self.try_state()?;
        if self.poisoned.load(Ordering::Acquire) {
            return Err(JournalError::Poisoned);
        }
        let transaction = match state
            .connection
            .transaction_with_behavior(TransactionBehavior::Immediate)
        {
            Ok(transaction) => transaction,
            Err(error) => {
                self.poisoned.store(true, Ordering::Release);
                return Err(sqlite_error(error));
            }
        };
        match action(&transaction) {
            Ok(value) => match transaction.commit() {
                Ok(()) => Ok(value),
                Err(error) => {
                    self.poisoned.store(true, Ordering::Release);
                    Err(sqlite_error(error))
                }
            },
            Err(error) => {
                if matches!(error, JournalError::Sqlite(_) | JournalError::Integrity(_)) {
                    self.poisoned.store(true, Ordering::Release);
                }
                drop(transaction);
                Err(error)
            }
        }
    }

    pub(super) fn read<R>(
        &self,
        action: impl FnOnce(&Transaction<'_>) -> Result<R, JournalError>,
    ) -> Result<R, JournalError> {
        let mut state = self.try_state()?;
        if self.poisoned.load(Ordering::Acquire) {
            return Err(JournalError::Poisoned);
        }
        let transaction = match state
            .connection
            .transaction_with_behavior(TransactionBehavior::Deferred)
        {
            Ok(transaction) => transaction,
            Err(error) => {
                self.poisoned.store(true, Ordering::Release);
                return Err(sqlite_error(error));
            }
        };
        match action(&transaction) {
            Ok(value) => match transaction.commit() {
                Ok(()) => Ok(value),
                Err(error) => {
                    self.poisoned.store(true, Ordering::Release);
                    Err(sqlite_error(error))
                }
            },
            Err(error) => {
                if matches!(error, JournalError::Sqlite(_) | JournalError::Integrity(_)) {
                    self.poisoned.store(true, Ordering::Release);
                }
                drop(transaction);
                Err(error)
            }
        }
    }
}
