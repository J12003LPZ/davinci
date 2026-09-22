use super::store_api::*;
use super::{AttemptId, OperationId, OperationState, RootNamespaceId};
use crate::runtime::cache::directory::{Directory, DirectoryLease};
use rusqlite::{params, Connection, Transaction, TransactionBehavior};
use serde::{de::DeserializeOwned, Serialize};
use std::io::{self, Write};
use std::str::FromStr;
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;
pub(super) fn collect_json<T: DeserializeOwned>(
    transaction: &Transaction<'_>,
    sql: &str,
    root: RootNamespaceId,
) -> Result<Vec<T>, JournalError> {
    let mut statement = transaction.prepare(sql).map_err(sqlite_error)?;
    let rows = statement
        .query_map([root.to_string()], |row| row.get::<_, String>(0))
        .map_err(sqlite_error)?;
    rows.map(|row| decode_json(&row.map_err(sqlite_error)?))
        .collect()
}

pub(super) fn collect_events(
    transaction: &Transaction<'_>,
    root: RootNamespaceId,
) -> Result<Vec<StoredEvent>, JournalError> {
    let mut statement = transaction
        .prepare(
            "SELECT event_sequence, operation_id, attempt_id, prior_revision, revision, event_json, created_at_ms
             FROM operation_events WHERE root_namespace_id = ?1 ORDER BY event_sequence",
        )
        .map_err(sqlite_error)?;
    let rows = statement
        .query_map([root.to_string()], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, i64>(6)?,
            ))
        })
        .map_err(sqlite_error)?;
    rows.map(|row| {
        let (sequence, operation, attempt, prior, revision, event, created_at_ms) =
            row.map_err(sqlite_error)?;
        Ok(StoredEvent {
            sequence: from_sql_integer(sequence)?,
            operation_id: parse_id(&operation)?,
            attempt_id: parse_id(&attempt)?,
            prior_revision: from_sql_integer(prior)?,
            revision: from_sql_integer(revision)?,
            event: decode_json(&event)?,
            created_at_ms: from_sql_integer(created_at_ms)?,
        })
    })
    .collect()
}

pub(super) fn collect_results(
    transaction: &Transaction<'_>,
    root: RootNamespaceId,
) -> Result<Vec<StoredResult>, JournalError> {
    let mut statement = transaction
        .prepare(
            "SELECT r.result_ref_json, r.payload_json, r.created_at_ms
             FROM operation_results r JOIN operations o USING(operation_id)
             WHERE o.root_namespace_id = ?1 ORDER BY r.created_at_ms, r.result_id",
        )
        .map_err(sqlite_error)?;
    let rows = statement
        .query_map([root.to_string()], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, i64>(2)?,
            ))
        })
        .map_err(sqlite_error)?;
    rows.map(|row| {
        let (reference, payload, created_at_ms) = row.map_err(sqlite_error)?;
        Ok(StoredResult {
            reference: decode_json(&reference)?,
            payload: decode_json(&payload)?,
            created_at_ms: from_sql_integer(created_at_ms)?,
        })
    })
    .collect()
}

pub(super) fn collect_outbox(
    transaction: &Transaction<'_>,
    root: RootNamespaceId,
    pending_only: bool,
    limit: usize,
) -> Result<Vec<StoredOutbox>, JournalError> {
    let sql = if pending_only {
        "SELECT outbox_id, operation_id, attempt_id, consumer, payload_json, state, created_at_ms, acknowledged_at_ms
         FROM operation_outbox WHERE root_namespace_id = ?1 AND state = 'pending'
         ORDER BY created_at_ms, outbox_id LIMIT ?2"
    } else {
        "SELECT outbox_id, operation_id, attempt_id, consumer, payload_json, state, created_at_ms, acknowledged_at_ms
         FROM operation_outbox WHERE root_namespace_id = ?1
         ORDER BY created_at_ms, outbox_id LIMIT ?2"
    };
    let mut statement = transaction.prepare(sql).map_err(sqlite_error)?;
    let rows = statement
        .query_map(params![root.to_string(), limit as i64], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
                row.get::<_, String>(5)?,
                row.get::<_, i64>(6)?,
                row.get::<_, Option<i64>>(7)?,
            ))
        })
        .map_err(sqlite_error)?;
    rows.map(|row| {
        let (id, operation, attempt, consumer, payload, state, created, acknowledged) =
            row.map_err(sqlite_error)?;
        Ok(StoredOutbox {
            id: Uuid::parse_str(&id).map_err(|error| JournalError::Integrity(error.to_string()))?,
            operation_id: parse_id(&operation)?,
            attempt_id: parse_id(&attempt)?,
            consumer,
            payload: decode_json(&payload)?,
            state: match state.as_str() {
                "pending" => OutboxState::Pending,
                "acknowledged" => OutboxState::Acknowledged,
                _ => return Err(JournalError::Integrity("invalid outbox state".into())),
            },
            created_at_ms: from_sql_integer(created)?,
            acknowledged_at_ms: acknowledged.map(from_sql_integer).transpose()?,
        })
    })
    .collect()
}

struct BoundedBuffer {
    bytes: Vec<u8>,
    limit: usize,
}

impl Write for BoundedBuffer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.bytes.len().saturating_add(bytes.len()) > self.limit {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "record limit exceeded",
            ));
        }
        self.bytes.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

pub(super) fn encode_bounded<T: Serialize>(
    value: &T,
    limit: usize,
    record: &'static str,
) -> Result<String, JournalError> {
    let mut buffer = BoundedBuffer {
        bytes: Vec::with_capacity(limit.min(8192)),
        limit,
    };
    if let Err(error) = serde_json::to_writer(&mut buffer, value) {
        if error.is_io() && error.io_error_kind() == Some(io::ErrorKind::InvalidData) {
            return Err(JournalError::OversizedRecord { record, limit });
        }
        return Err(JournalError::Serialization(error.to_string()));
    }
    String::from_utf8(buffer.bytes).map_err(|error| JournalError::Serialization(error.to_string()))
}

pub(super) fn decode_json<T: DeserializeOwned>(value: &str) -> Result<T, JournalError> {
    serde_json::from_str(value).map_err(|error| JournalError::Integrity(error.to_string()))
}

pub(super) fn state_name(state: OperationState) -> Result<String, JournalError> {
    serde_json::to_value(state)
        .map_err(|error| JournalError::Serialization(error.to_string()))?
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| JournalError::Serialization("invalid operation state".into()))
}

pub(super) fn parse_id<T>(value: &str) -> Result<T, JournalError>
where
    T: FromStr,
    T::Err: std::fmt::Display,
{
    value
        .parse()
        .map_err(|error: T::Err| JournalError::Integrity(error.to_string()))
}

pub(super) fn to_sql_integer(value: u64) -> Result<i64, JournalError> {
    i64::try_from(value).map_err(|_| JournalError::Capacity("SQLite integer range"))
}

pub(super) fn from_sql_integer(value: i64) -> Result<u64, JournalError> {
    u64::try_from(value).map_err(|_| JournalError::Integrity("negative integer in journal".into()))
}

pub(super) fn io_error(error: io::Error) -> JournalError {
    JournalError::Io(error.to_string())
}

pub(super) fn sqlite_error(error: rusqlite::Error) -> JournalError {
    JournalError::Sqlite(error)
}

pub(super) fn acquire_lease(
    directory: &Directory,
    migration: bool,
) -> Result<DirectoryLease, JournalError> {
    directory.lease().map_err(|error| {
        let conflict = error.kind() == io::ErrorKind::WouldBlock
            || matches!(error.raw_os_error(), Some(32 | 33));
        if conflict && migration {
            JournalError::MigrationBusy
        } else if conflict {
            JournalError::CoordinatorAlreadyOwned
        } else {
            io_error(error)
        }
    })
}

pub(super) fn register_root(
    connection: &mut Connection,
    identity: &JournalIdentity,
    root: RootNamespaceId,
) -> Result<(), JournalError> {
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(sqlite_error)?;
    let root_text = root.to_string();
    transaction
        .execute(
            "INSERT OR IGNORE INTO journal_roots
             (root_namespace_id, journal_id, workspace_id, workspace_binding_version, created_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                root_text,
                identity.journal_id.to_string(),
                identity.workspace.id.to_string(),
                identity.workspace.binding_version,
                now_unix_millis()
            ],
        )
        .map_err(sqlite_error)?;
    let stored: (String, String, u32) = transaction
        .query_row(
            "SELECT journal_id, workspace_id, workspace_binding_version
             FROM journal_roots WHERE root_namespace_id = ?1",
            [root.to_string()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .map_err(sqlite_error)?;
    if stored.0 != identity.journal_id.to_string()
        || stored.1 != identity.workspace.id.to_string()
        || stored.2 != identity.workspace.binding_version
    {
        return Err(JournalError::Integrity(
            "root binding disagrees with journal metadata".into(),
        ));
    }
    transaction.commit().map_err(sqlite_error)
}

pub(super) fn insert_event(
    transaction: &Transaction<'_>,
    root: RootNamespaceId,
    operation: OperationId,
    attempt: AttemptId,
    prior_revision: u64,
    revision: u64,
    event_json: &str,
) -> Result<i64, JournalError> {
    transaction
        .execute(
            "INSERT INTO operation_events
             (root_namespace_id, operation_id, attempt_id, prior_revision, revision, event_json, created_at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                root.to_string(),
                operation.to_string(),
                attempt.to_string(),
                to_sql_integer(prior_revision)?,
                to_sql_integer(revision)?,
                event_json,
                now_unix_millis()
            ],
        )
        .map_err(sqlite_error)?;
    Ok(transaction.last_insert_rowid())
}

pub(super) fn now_unix_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .min(i64::MAX as u128) as i64
}
