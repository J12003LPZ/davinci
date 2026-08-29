use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use pi_core::{next_id, now_ms, SessionError};
use pi_session::{
    assign_storage_fields, Entry, EntryQuery, ForkOptions, ForkScope, LaneRecord, LogItem,
    QueryOrder, SessionCreateOptions, SessionMetadata, SessionRepository, SessionStats,
    SessionStore,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::leases::{
    acquire_writer_lease, active_writer_error, delete_writer_lease, lost_writer_error,
    release_writer_lease, renew_writer_lease, WriterLease, WriterLeaseOptions,
};

const SCHEMA: &str = include_str!("../migrations/001_initial.sql");

pub fn apply_migrations(db: &Connection) -> Result<(), SessionError> {
    db.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=FULL;
         PRAGMA busy_timeout=5000;
         CREATE TABLE IF NOT EXISTS migrations (
            id TEXT PRIMARY KEY,
            applied_at TEXT NOT NULL
         );",
    )
    .map_err(|error| SessionError::storage(error.to_string()))?;
    let applied: Option<String> = db
        .query_row(
            "SELECT id FROM migrations WHERE id = '001_initial'",
            [],
            |row| row.get(0),
        )
        .optional()
        .map_err(|error| SessionError::storage(error.to_string()))?;
    if applied.is_none() {
        db.execute_batch(SCHEMA)
            .map_err(|error| SessionError::storage(error.to_string()))?;
        db.execute(
            "INSERT INTO migrations (id, applied_at) VALUES ('001_initial', datetime('now'))",
            [],
        )
        .map_err(|error| SessionError::storage(error.to_string()))?;
    }
    Ok(())
}

pub fn open_database(path: &Path) -> Result<Connection, SessionError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| SessionError::storage(error.to_string()))?;
    }
    let db = Connection::open(path).map_err(|error| SessionError::storage(error.to_string()))?;
    apply_migrations(&db)?;
    Ok(db)
}

fn claim_writer_lease(
    db: &Connection,
    session_id: &str,
    options: WriterLeaseOptions,
    now: i64,
) -> Result<WriterLease, SessionError> {
    let owner_id = Uuid::now_v7().to_string();
    acquire_writer_lease(db, session_id, &owner_id, now, now + options.ttl_ms)?
        .ok_or_else(|| active_writer_error(session_id))
}

fn next_sequence(db: &Connection, session_id: &str) -> Result<i64, SessionError> {
    let seq: i64 = db
        .query_row(
            "SELECT next_seq FROM session_sequences WHERE session_id = ?1",
            params![session_id],
            |row| row.get(0),
        )
        .map_err(|error| SessionError::storage(error.to_string()))?;
    db.execute(
        "UPDATE session_sequences SET next_seq = ?1 WHERE session_id = ?2",
        params![seq + 1, session_id],
    )
    .map_err(|error| SessionError::storage(error.to_string()))?;
    Ok(seq)
}

fn entry_payload(entry: &Entry) -> Result<String, SessionError> {
    let mut value = serde_json::to_value(entry)
        .map_err(|error| SessionError::invalid_payload(error.to_string()))?;
    if let Value::Object(map) = &mut value {
        map.remove("type");
        map.remove("id");
        map.remove("seq");
        map.remove("parentId");
        map.remove("timestamp");
    }
    serde_json::to_string(&value).map_err(|error| SessionError::invalid_payload(error.to_string()))
}

fn decode_entry(
    id: String,
    seq: i64,
    parent_id: Option<String>,
    entry_type: &str,
    timestamp: i64,
    payload: &str,
) -> Result<Entry, SessionError> {
    let mut body: Value =
        serde_json::from_str(payload).map_err(|error| SessionError::storage(error.to_string()))?;
    if let Value::Object(map) = &mut body {
        map.insert("type".into(), json!(entry_type));
        map.insert("id".into(), json!(id));
        map.insert("seq".into(), json!(seq));
        map.insert(
            "parentId".into(),
            match parent_id {
                Some(parent) => json!(parent),
                None => Value::Null,
            },
        );
        map.insert("timestamp".into(), json!(timestamp));
    }
    serde_json::from_value(body).map_err(|error| SessionError::storage(error.to_string()))
}

fn append_entry_to_branch_cache(
    db: &Connection,
    session_id: &str,
    entry: &Entry,
) -> Result<(), SessionError> {
    match entry.parent_id() {
        None => {
            let branch_id = next_id();
            db.execute(
                "INSERT INTO branch_entries (session_id, branch_id, entry_id, entry_seq, entry_type, custom_type)
                 VALUES (?1, ?2, ?3, ?4, ?5, NULL)",
                params![session_id, branch_id, entry.id(), entry.seq(), entry.entry_type()],
            )
            .map_err(|error| SessionError::storage(error.to_string()))?;
            db.execute(
                "INSERT INTO branch_tips (session_id, branch_id, tip_id) VALUES (?1, ?2, ?3)",
                params![session_id, branch_id, entry.id()],
            )
            .map_err(|error| SessionError::storage(error.to_string()))?;
        }
        Some(parent_id) => {
            let tip: Option<String> = db
                .query_row(
                    "SELECT branch_id FROM branch_tips WHERE session_id = ?1 AND tip_id = ?2",
                    params![session_id, parent_id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|error| SessionError::storage(error.to_string()))?;
            if let Some(branch_id) = tip {
                db.execute(
                    "INSERT INTO branch_entries (session_id, branch_id, entry_id, entry_seq, entry_type, custom_type)
                     VALUES (?1, ?2, ?3, ?4, ?5, NULL)",
                    params![session_id, branch_id, entry.id(), entry.seq(), entry.entry_type()],
                )
                .map_err(|error| SessionError::storage(error.to_string()))?;
                db.execute(
                    "UPDATE branch_tips SET tip_id = ?1 WHERE session_id = ?2 AND branch_id = ?3 AND tip_id = ?4",
                    params![entry.id(), session_id, branch_id, parent_id],
                )
                .map_err(|error| SessionError::storage(error.to_string()))?;
            } else {
                let source_branch: String = db
                    .query_row(
                        "SELECT branch_id FROM branch_entries WHERE session_id = ?1 AND entry_id = ?2 LIMIT 1",
                        params![session_id, parent_id],
                        |row| row.get(0),
                    )
                    .map_err(|_| SessionError::storage("parent is not on a cached branch".to_string()))?;
                let parent_seq: i64 = db
                    .query_row(
                        "SELECT entry_seq FROM branch_entries WHERE session_id = ?1 AND branch_id = ?2 AND entry_id = ?3",
                        params![session_id, source_branch, parent_id],
                        |row| row.get(0),
                    )
                    .map_err(|error| SessionError::storage(error.to_string()))?;
                let new_branch = next_id();
                db.execute(
                    "INSERT INTO branch_entries (session_id, branch_id, entry_id, entry_seq, entry_type, custom_type)
                     SELECT session_id, ?1, entry_id, entry_seq, entry_type, custom_type
                     FROM branch_entries
                     WHERE session_id = ?2 AND branch_id = ?3 AND entry_seq <= ?4",
                    params![new_branch, session_id, source_branch, parent_seq],
                )
                .map_err(|error| SessionError::storage(error.to_string()))?;
                db.execute(
                    "INSERT INTO branch_entries (session_id, branch_id, entry_id, entry_seq, entry_type, custom_type)
                     VALUES (?1, ?2, ?3, ?4, ?5, NULL)",
                    params![session_id, new_branch, entry.id(), entry.seq(), entry.entry_type()],
                )
                .map_err(|error| SessionError::storage(error.to_string()))?;
                db.execute(
                    "INSERT INTO branch_tips (session_id, branch_id, tip_id) VALUES (?1, ?2, ?3)",
                    params![session_id, new_branch, entry.id()],
                )
                .map_err(|error| SessionError::storage(error.to_string()))?;
            }
        }
    }
    Ok(())
}

#[derive(Clone)]
pub struct SqliteSessionStorage {
    db: Arc<Mutex<Connection>>,
    metadata: SessionMetadata,
    lease: WriterLease,
    options: WriterLeaseOptions,
    closed: bool,
}

impl SqliteSessionStorage {
    fn with_lease<T>(
        &mut self,
        op: impl FnOnce(&Connection, &mut WriterLease) -> Result<T, SessionError>,
    ) -> Result<T, SessionError> {
        if self.closed {
            return Err(SessionError::storage(format!(
                "SQLite session {} is closed",
                self.metadata.id
            )));
        }
        let db = self
            .db
            .lock()
            .map_err(|error| SessionError::storage(error.to_string()))?;
        let tx = db
            .unchecked_transaction()
            .map_err(|error| SessionError::storage(error.to_string()))?;
        let now = now_ms();
        if !renew_writer_lease(
            &tx,
            &self.metadata.id,
            &mut self.lease,
            now,
            now + self.options.ttl_ms,
        )? {
            return Err(lost_writer_error(&self.metadata.id));
        }
        let result = op(&tx, &mut self.lease);
        match result {
            Ok(value) => {
                tx.commit()
                    .map_err(|error| SessionError::storage(error.to_string()))?;
                Ok(value)
            }
            Err(error) => {
                let _ = tx.rollback();
                Err(error)
            }
        }
    }

    #[allow(dead_code)]
    pub fn expire_lease_for_test(&self) -> Result<(), SessionError> {
        let db = self
            .db
            .lock()
            .map_err(|error| SessionError::storage(error.to_string()))?;
        db.execute(
            "UPDATE writer_leases SET expires_at_ms = 0 WHERE session_id = ?1",
            params![self.metadata.id],
        )
        .map_err(|error| SessionError::storage(error.to_string()))?;
        Ok(())
    }

    #[allow(dead_code)]
    pub fn current_fence(&self) -> Result<i64, SessionError> {
        let db = self
            .db
            .lock()
            .map_err(|error| SessionError::storage(error.to_string()))?;
        db.query_row(
            "SELECT fence FROM writer_leases WHERE session_id = ?1",
            params![self.metadata.id],
            |row| row.get(0),
        )
        .map_err(|error| SessionError::storage(error.to_string()))
    }
}

impl SessionStore for SqliteSessionStorage {
    fn metadata(&self) -> Result<SessionMetadata, SessionError> {
        let db = self
            .db
            .lock()
            .map_err(|error| SessionError::storage(error.to_string()))?;
        read_metadata(&db, &self.metadata.id, &self.metadata.path)
    }

    fn append_entry(&mut self, entry: Entry, lane: &str) -> Result<Entry, SessionError> {
        let session_id = self.metadata.id.clone();
        self.with_lease(|db, _| {
            let parent_id: Option<String> = db
                .query_row(
                    "SELECT leaf_id FROM lanes WHERE session_id = ?1 AND lane = ?2",
                    params![session_id, lane],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|error| SessionError::storage(error.to_string()))?
                .flatten();
            let exists: Option<i64> = db
                .query_row(
                    "SELECT 1 FROM entries WHERE session_id = ?1 AND id = ?2
                     UNION ALL
                     SELECT 1 FROM records WHERE session_id = ?1 AND id = ?2",
                    params![session_id, entry.id()],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|error| SessionError::storage(error.to_string()))?;
            if exists.is_some() {
                return Err(SessionError::invalid_payload(format!(
                    "duplicate id {}",
                    entry.id()
                )));
            }
            let seq = next_sequence(db, &session_id)?;
            let stored = assign_storage_fields(entry, seq, parent_id, now_ms());
            db.execute(
                "INSERT INTO entries (session_id, seq, id, parent_id, type, timestamp, payload)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    session_id,
                    stored.seq(),
                    stored.id(),
                    stored.parent_id(),
                    stored.entry_type(),
                    match &stored {
                        Entry::Message { timestamp, .. }
                        | Entry::ModelChange { timestamp, .. }
                        | Entry::ThinkingLevelChange { timestamp, .. }
                        | Entry::Custom { timestamp, .. } => *timestamp,
                    },
                    entry_payload(&stored)?,
                ],
            )
            .map_err(|error| SessionError::storage(error.to_string()))?;
            db.execute(
                "UPDATE lanes SET leaf_id = ?1 WHERE session_id = ?2 AND lane = ?3",
                params![stored.id(), session_id, lane],
            )
            .map_err(|error| SessionError::storage(error.to_string()))?;
            append_entry_to_branch_cache(db, &session_id, &stored)?;
            if stored.is_message() {
                db.execute(
                    "UPDATE session_stats SET message_count = message_count + 1 WHERE session_id = ?1",
                    params![session_id],
                )
                .map_err(|error| SessionError::storage(error.to_string()))?;
            }
            Ok(stored)
        })
    }

    fn find_entries(&self, query: EntryQuery) -> Result<Vec<Entry>, SessionError> {
        let db = self
            .db
            .lock()
            .map_err(|error| SessionError::storage(error.to_string()))?;
        let order = if matches!(query.order, Some(QueryOrder::NewestFirst)) {
            "DESC"
        } else {
            "ASC"
        };
        let sql = format!(
            "SELECT id, seq, parent_id, type, timestamp, payload FROM entries
             WHERE session_id = ?1 {} ORDER BY seq {order}",
            if query.entry_type.is_some() {
                "AND type = ?2"
            } else {
                ""
            }
        );
        let mut stmt = db
            .prepare(&sql)
            .map_err(|error| SessionError::storage(error.to_string()))?;
        let mapped = if let Some(entry_type) = query.entry_type.as_deref() {
            stmt.query_map(params![self.metadata.id, entry_type], row_to_entry)
        } else {
            stmt.query_map(params![self.metadata.id], row_to_entry)
        }
        .map_err(|error| SessionError::storage(error.to_string()))?;
        let mut entries = mapped
            .collect::<Result<Vec<_>, _>>()
            .map_err(|error| SessionError::storage(error.to_string()))?;
        if let Some(limit) = query.limit {
            entries.truncate(limit);
        }
        Ok(entries)
    }

    fn find_entries_on_branch(&self, start: &str) -> Result<Vec<Entry>, SessionError> {
        let db = self
            .db
            .lock()
            .map_err(|error| SessionError::storage(error.to_string()))?;
        let branch_id: String = db
            .query_row(
                "SELECT branch_id FROM branch_entries WHERE session_id = ?1 AND entry_id = ?2 LIMIT 1",
                params![self.metadata.id, start],
                |row| row.get(0),
            )
            .map_err(|_| SessionError::not_found(format!("entry {start} not found")))?;
        let mut stmt = db
            .prepare(
                "SELECT e.id, e.seq, e.parent_id, e.type, e.timestamp, e.payload
                 FROM branch_entries b
                 JOIN entries e ON e.session_id = b.session_id AND e.id = b.entry_id
                 WHERE b.session_id = ?1 AND b.branch_id = ?2 AND b.entry_seq <= (
                    SELECT entry_seq FROM branch_entries
                    WHERE session_id = ?1 AND branch_id = ?2 AND entry_id = ?3
                 )
                 ORDER BY b.entry_seq ASC",
            )
            .map_err(|error| SessionError::storage(error.to_string()))?;
        let rows = stmt
            .query_map(params![self.metadata.id, branch_id, start], row_to_entry)
            .map_err(|error| SessionError::storage(error.to_string()))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|error| SessionError::storage(error.to_string()))
    }

    fn append_record(&mut self, record: LaneRecord) -> Result<LaneRecord, SessionError> {
        let session_id = self.metadata.id.clone();
        self.with_lease(|db, _| {
            let seq = next_sequence(db, &session_id)?;
            let payload = serde_json::to_string(&record)
                .map_err(|error| SessionError::invalid_payload(error.to_string()))?;
            db.execute(
                "INSERT INTO records (session_id, seq, id, lane, run_id, type, op_kind, timestamp, payload)
                 VALUES (?1, ?2, ?3, ?4, NULL, ?5, NULL, ?6, ?7)",
                params![
                    session_id,
                    seq,
                    record.id(),
                    record.lane(),
                    record.record_type(),
                    now_ms(),
                    payload
                ],
            )
            .map_err(|error| SessionError::storage(error.to_string()))?;
            Ok(record)
        })
    }

    fn find_records(&self, lane: Option<&str>) -> Result<Vec<LaneRecord>, SessionError> {
        let db = self
            .db
            .lock()
            .map_err(|error| SessionError::storage(error.to_string()))?;
        let mut stmt = if lane.is_some() {
            db.prepare(
                "SELECT payload FROM records WHERE session_id = ?1 AND lane = ?2 ORDER BY seq",
            )
        } else {
            db.prepare("SELECT payload FROM records WHERE session_id = ?1 ORDER BY seq")
        }
        .map_err(|error| SessionError::storage(error.to_string()))?;
        let mut payloads = Vec::new();
        if let Some(lane) = lane {
            let mut rows = stmt
                .query(params![self.metadata.id, lane])
                .map_err(|error| SessionError::storage(error.to_string()))?;
            while let Some(row) = rows
                .next()
                .map_err(|error| SessionError::storage(error.to_string()))?
            {
                payloads.push(
                    row.get::<_, String>(0)
                        .map_err(|error| SessionError::storage(error.to_string()))?,
                );
            }
        } else {
            let mut rows = stmt
                .query(params![self.metadata.id])
                .map_err(|error| SessionError::storage(error.to_string()))?;
            while let Some(row) = rows
                .next()
                .map_err(|error| SessionError::storage(error.to_string()))?
            {
                payloads.push(
                    row.get::<_, String>(0)
                        .map_err(|error| SessionError::storage(error.to_string()))?,
                );
            }
        }
        payloads
            .into_iter()
            .map(|payload| {
                serde_json::from_str(&payload)
                    .map_err(|error| SessionError::storage(error.to_string()))
            })
            .collect()
    }

    fn get_log(&self, limit: Option<usize>) -> Result<Vec<LogItem>, SessionError> {
        let entries = self.find_entries(EntryQuery {
            order: Some(QueryOrder::OldestFirst),
            ..EntryQuery::default()
        })?;
        let records = self.find_records(None)?;
        let mut items: Vec<LogItem> = entries
            .into_iter()
            .map(|entry| LogItem::Entry {
                seq: entry.seq(),
                entry,
            })
            .chain(records.into_iter().map(|record| LogItem::Record {
                seq: record.seq(),
                record,
            }))
            .collect();
        items.sort_by_key(|item| match item {
            LogItem::Entry { seq, .. } | LogItem::Record { seq, .. } => *seq,
            _ => 0,
        });
        if let Some(limit) = limit {
            let start = items.len().saturating_sub(limit);
            items = items.split_off(start);
        }
        Ok(items)
    }

    fn set_name(&mut self, name: Option<&str>) -> Result<(), SessionError> {
        let session_id = self.metadata.id.clone();
        let stored = name.map(str::to_string);
        self.with_lease(|db, _| {
            let seq = next_sequence(db, &session_id)?;
            db.execute(
                "INSERT INTO facts (session_id, seq, kind, key, value) VALUES (?1, ?2, 'name', NULL, ?3)",
                params![session_id, seq, stored.as_deref()],
            )
            .map_err(|error| SessionError::storage(error.to_string()))?;
            Ok(())
        })
    }

    fn get_name(&self) -> Result<Option<String>, SessionError> {
        Ok(self.metadata()?.name)
    }

    fn get_stats(&self) -> Result<SessionStats, SessionError> {
        let db = self
            .db
            .lock()
            .map_err(|error| SessionError::storage(error.to_string()))?;
        db.query_row(
            "SELECT message_count, cached_tokens, uncached_tokens, total_tokens, cost_total
             FROM session_stats WHERE session_id = ?1",
            params![self.metadata.id],
            |row| {
                Ok(SessionStats {
                    message_count: row.get(0)?,
                    cached_tokens: row.get(1)?,
                    uncached_tokens: row.get(2)?,
                    total_tokens: row.get(3)?,
                    cost_total: row.get(4)?,
                })
            },
        )
        .map_err(|error| SessionError::storage(error.to_string()))
    }

    fn release(&mut self) -> Result<(), SessionError> {
        if self.closed {
            return Ok(());
        }
        let db = self
            .db
            .lock()
            .map_err(|error| SessionError::storage(error.to_string()))?;
        release_writer_lease(&db, &self.metadata.id, &self.lease)?;
        self.closed = true;
        Ok(())
    }
}

fn row_to_entry(row: &rusqlite::Row<'_>) -> rusqlite::Result<Entry> {
    let id: String = row.get(0)?;
    let seq: i64 = row.get(1)?;
    let parent_id: Option<String> = row.get(2)?;
    let entry_type: String = row.get(3)?;
    let timestamp: i64 = row.get(4)?;
    let payload: String = row.get(5)?;
    decode_entry(id, seq, parent_id, &entry_type, timestamp, &payload).map_err(|error| {
        rusqlite::Error::ToSqlConversionFailure(Box::new(std::io::Error::other(error.to_string())))
    })
}

fn read_metadata(
    db: &Connection,
    session_id: &str,
    path: &str,
) -> Result<SessionMetadata, SessionError> {
    db.query_row(
        "SELECT s.id, s.created_at, s.cwd, s.parent_session_id, s.metadata,
                name_fact.value AS session_name
         FROM sessions AS s
         LEFT JOIN facts AS name_fact
            ON name_fact.session_id = s.id
            AND name_fact.kind = 'name'
            AND name_fact.key IS NULL
            AND name_fact.seq = (
                SELECT MAX(f.seq) FROM facts AS f
                WHERE f.session_id = s.id AND f.kind = 'name' AND f.key IS NULL
            )
         WHERE s.id = ?1",
        params![session_id],
        |row| {
            let metadata_raw: Option<String> = row.get(4)?;
            let name: Option<String> = row.get(5)?;
            Ok(SessionMetadata {
                id: row.get(0)?,
                created_at: row.get(1)?,
                cwd: row.get(2)?,
                path: path.to_string(),
                parent_session_id: row.get(3)?,
                name,
                metadata: metadata_raw.and_then(|raw| serde_json::from_str(&raw).ok()),
            })
        },
    )
    .map_err(|error| SessionError::storage(error.to_string()))
}

pub struct SqliteSessionRepository {
    db: Arc<Mutex<Connection>>,
    path: PathBuf,
    options: WriterLeaseOptions,
}

impl SqliteSessionRepository {
    pub fn open(path: impl AsRef<Path>, options: WriterLeaseOptions) -> Result<Self, SessionError> {
        let path = path.as_ref().to_path_buf();
        let db = open_database(&path)?;
        Ok(Self {
            db: Arc::new(Mutex::new(db)),
            path,
            options,
        })
    }

    pub fn open_default(path: impl AsRef<Path>) -> Result<Self, SessionError> {
        Self::open(path, WriterLeaseOptions::default())
    }

    pub fn inspect_leases(&self) -> Result<Vec<(String, String, i64, i64)>, SessionError> {
        let db = self
            .db
            .lock()
            .map_err(|error| SessionError::storage(error.to_string()))?;
        crate::leases::read_writer_leases(&db)
    }

    fn insert_session_rows(
        db: &Connection,
        metadata: &SessionMetadata,
    ) -> Result<(), SessionError> {
        let metadata_json = metadata
            .metadata
            .as_ref()
            .map(serde_json::to_string)
            .transpose()
            .map_err(|error| SessionError::invalid_payload(error.to_string()))?;
        db.execute(
            "INSERT INTO sessions (id, created_at, cwd, parent_session_id, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                metadata.id,
                metadata.created_at,
                metadata.cwd,
                metadata.parent_session_id,
                metadata_json
            ],
        )
        .map_err(|error| SessionError::storage(error.to_string()))?;
        db.execute(
            "INSERT INTO session_sequences (session_id, next_seq) VALUES (?1, 1)",
            params![metadata.id],
        )
        .map_err(|error| SessionError::storage(error.to_string()))?;
        db.execute(
            "INSERT INTO session_stats (session_id, message_count, cached_tokens, uncached_tokens, total_tokens, cost_total)
             VALUES (?1, 0, 0, 0, 0, 0)",
            params![metadata.id],
        )
        .map_err(|error| SessionError::storage(error.to_string()))?;
        db.execute(
            "INSERT INTO lanes (session_id, lane, leaf_id, open_operation_id) VALUES (?1, 'main', NULL, NULL)",
            params![metadata.id],
        )
        .map_err(|error| SessionError::storage(error.to_string()))?;
        Ok(())
    }

    fn claim_storage(
        &self,
        metadata: SessionMetadata,
    ) -> Result<SqliteSessionStorage, SessionError> {
        let db = self
            .db
            .lock()
            .map_err(|error| SessionError::storage(error.to_string()))?;
        let lease = claim_writer_lease(&db, &metadata.id, self.options, now_ms())?;
        Ok(SqliteSessionStorage {
            db: Arc::clone(&self.db),
            metadata,
            lease,
            options: self.options,
            closed: false,
        })
    }
}

impl SessionRepository for SqliteSessionRepository {
    fn create(
        &mut self,
        options: SessionCreateOptions,
    ) -> Result<Box<dyn SessionStore>, SessionError> {
        let id = options.id.unwrap_or_else(next_id);
        let metadata = SessionMetadata {
            id: id.clone(),
            created_at: now_ms(),
            cwd: options.cwd,
            path: self.path.to_string_lossy().into_owned(),
            parent_session_id: options.parent_session_id,
            name: options.name.clone(),
            metadata: options.metadata,
        };
        {
            let db = self
                .db
                .lock()
                .map_err(|error| SessionError::storage(error.to_string()))?;
            let tx = db
                .unchecked_transaction()
                .map_err(|error| SessionError::storage(error.to_string()))?;
            Self::insert_session_rows(&tx, &metadata)?;
            tx.commit()
                .map_err(|error| SessionError::storage(error.to_string()))?;
        }
        let mut storage = self.claim_storage(metadata)?;
        if let Some(name) = options.name {
            storage.set_name(Some(&name))?;
        }
        Ok(Box::new(storage))
    }

    fn open(&mut self, metadata: &SessionMetadata) -> Result<Box<dyn SessionStore>, SessionError> {
        let db = self
            .db
            .lock()
            .map_err(|error| SessionError::storage(error.to_string()))?;
        let resolved = read_metadata(&db, &metadata.id, &self.path.to_string_lossy())?;
        drop(db);
        Ok(Box::new(self.claim_storage(resolved)?))
    }

    fn list(&self, cwd: Option<&str>) -> Result<Vec<SessionMetadata>, SessionError> {
        let db = self
            .db
            .lock()
            .map_err(|error| SessionError::storage(error.to_string()))?;
        let path = self.path.to_string_lossy().into_owned();
        let mut stmt = if cwd.is_some() {
            db.prepare("SELECT s.id FROM sessions s WHERE s.cwd = ?1 ORDER BY s.created_at ASC")
        } else {
            db.prepare("SELECT s.id FROM sessions s ORDER BY s.created_at ASC")
        }
        .map_err(|error| SessionError::storage(error.to_string()))?;
        let mut ids = Vec::new();
        if let Some(cwd) = cwd {
            let mut rows = stmt
                .query(params![cwd])
                .map_err(|error| SessionError::storage(error.to_string()))?;
            while let Some(row) = rows
                .next()
                .map_err(|error| SessionError::storage(error.to_string()))?
            {
                ids.push(
                    row.get::<_, String>(0)
                        .map_err(|error| SessionError::storage(error.to_string()))?,
                );
            }
        } else {
            let mut rows = stmt
                .query([])
                .map_err(|error| SessionError::storage(error.to_string()))?;
            while let Some(row) = rows
                .next()
                .map_err(|error| SessionError::storage(error.to_string()))?
            {
                ids.push(
                    row.get::<_, String>(0)
                        .map_err(|error| SessionError::storage(error.to_string()))?,
                );
            }
        }
        ids.into_iter()
            .map(|id| read_metadata(&db, &id, &path))
            .collect()
    }

    fn delete(&mut self, metadata: &SessionMetadata) -> Result<(), SessionError> {
        let db = self
            .db
            .lock()
            .map_err(|error| SessionError::storage(error.to_string()))?;
        let tx = db
            .unchecked_transaction()
            .map_err(|error| SessionError::storage(error.to_string()))?;
        for table in [
            "branch_entries",
            "branch_tips",
            "facts",
            "lane_moves",
            "lanes",
            "records",
            "entries",
            "session_stats",
            "session_sequences",
            "sessions",
        ] {
            tx.execute(
                &format!("DELETE FROM {table} WHERE session_id = ?1"),
                params![metadata.id],
            )
            .or_else(|_| {
                if table == "sessions" {
                    tx.execute("DELETE FROM sessions WHERE id = ?1", params![metadata.id])
                } else {
                    Ok(0)
                }
            })
            .map_err(|error| SessionError::storage(error.to_string()))?;
        }
        delete_writer_lease(&tx, &metadata.id)?;
        tx.commit()
            .map_err(|error| SessionError::storage(error.to_string()))?;
        Ok(())
    }

    fn fork(
        &mut self,
        source: &dyn SessionStore,
        options: ForkOptions,
    ) -> Result<Box<dyn SessionStore>, SessionError> {
        let source_meta = source.metadata()?;
        let start = options.entry_id.clone().or_else(|| {
            source
                .find_entries(EntryQuery {
                    order: Some(QueryOrder::NewestFirst),
                    limit: Some(1),
                    ..EntryQuery::default()
                })
                .ok()
                .and_then(|entries| {
                    entries
                        .into_iter()
                        .next()
                        .map(|entry| entry.id().to_string())
                })
        });
        let copied = match options.scope {
            ForkScope::Tree => source.find_entries(EntryQuery::default())?,
            ForkScope::Branch => {
                let start =
                    start.ok_or_else(|| SessionError::invalid_payload("fork target missing"))?;
                let mut path = source.find_entries_on_branch(&start)?;
                if matches!(options.position, pi_session::ForkPosition::Before) {
                    path.pop();
                }
                path
            }
        };
        let mut created = self.create(SessionCreateOptions {
            cwd: options.cwd,
            parent_session_id: Some(source_meta.id),
            metadata: source_meta.metadata,
            ..SessionCreateOptions::default()
        })?;
        for entry in copied {
            created.append_entry(assign_storage_fields(entry, 0, None, 0), "main")?;
        }
        Ok(created)
    }
}
