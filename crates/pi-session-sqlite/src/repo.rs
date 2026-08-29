use crate::leases::{
    claim_writer_lease, delete_writer_lease, read_writer_lease, release_writer_lease,
    renew_writer_lease,
};
use crate::schema::INITIAL_SCHEMA;
use pi_core::{
    AgentMessage, Entry, Role, SessionError, SessionErrorCode, SessionMetadata, WriterLease,
    WriterLeaseOptions,
};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde_json::Value;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error(transparent)]
    Session(#[from] SessionError),
    #[error("database: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
}

pub type Result<T> = std::result::Result<T, StoreError>;

#[derive(Clone)]
pub struct SqliteSessionRepository {
    inner: Arc<Mutex<Inner>>,
}

struct Inner {
    db: Connection,
    path: PathBuf,
    lease_options: WriterLeaseOptions,
    active: HashMap<String, ActiveStorage>,
}

struct ActiveStorage {
    lease: WriterLease,
    lease_error: Option<SessionError>,
    closed: bool,
}

#[derive(Debug, Clone)]
pub struct SessionHandle {
    pub metadata: SessionMetadata,
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_millis() as i64
}

impl SqliteSessionRepository {
    pub fn open_in_memory(lease_options: WriterLeaseOptions) -> Result<Self> {
        lease_options
            .validate()
            .map_err(|m| SessionError::new(SessionErrorCode::Storage, m))?;
        let db = Connection::open_in_memory()?;
        configure(&db)?;
        apply_migrations(&db)?;
        Ok(Self {
            inner: Arc::new(Mutex::new(Inner {
                db,
                path: PathBuf::from(":memory:"),
                lease_options,
                active: HashMap::new(),
            })),
        })
    }

    pub fn open_path(path: impl AsRef<Path>, lease_options: WriterLeaseOptions) -> Result<Self> {
        lease_options
            .validate()
            .map_err(|m| SessionError::new(SessionErrorCode::Storage, m))?;
        if let Some(parent) = path.as_ref().parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| SessionError::new(SessionErrorCode::Storage, e.to_string()))?;
        }
        let db = Connection::open(path.as_ref())?;
        configure(&db)?;
        apply_migrations(&db)?;
        Ok(Self {
            inner: Arc::new(Mutex::new(Inner {
                db,
                path: path.as_ref().to_path_buf(),
                lease_options,
                active: HashMap::new(),
            })),
        })
    }

    pub fn create(
        &self,
        id: Option<&str>,
        cwd: &str,
        parent_session_id: Option<&str>,
        metadata: Option<&Value>,
    ) -> Result<SessionHandle> {
        let mut inner = self.inner.lock().unwrap();
        let id = id
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| Uuid::now_v7().to_string());
        let created_at = now_ms();
        let meta_json = metadata.map(serde_json::to_string).transpose()?;
        let ttl_ms = inner.lease_options.ttl_ms;
        let path = inner.path.display().to_string();
        let tx = inner
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let exists: Option<String> = tx
            .query_row(
                "SELECT id FROM sessions WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .optional()?;
        if exists.is_some() {
            return Err(SessionError::new(
                SessionErrorCode::AlreadyExists,
                format!("Session already exists: {id}"),
            )
            .into());
        }
        tx.execute(
            "INSERT INTO sessions (id, created_at, cwd, parent_session_id, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, created_at, cwd, parent_session_id, meta_json],
        )?;
        tx.execute(
            "INSERT INTO session_sequences (session_id, next_seq) VALUES (?1, 1)",
            params![id],
        )?;
        tx.execute(
            "INSERT INTO session_stats (session_id, message_count, cached_tokens, uncached_tokens, total_tokens, cost_total)
             VALUES (?1, 0, 0, 0, 0, 0)",
            params![id],
        )?;
        tx.execute(
            "INSERT INTO lanes (session_id, lane, leaf_id, open_operation_id) VALUES (?1, 'main', NULL, NULL)",
            params![id],
        )?;
        let lease = claim_writer_lease(&tx, &id, ttl_ms, created_at)?;
        tx.commit()?;
        let metadata = SessionMetadata {
            id: id.clone(),
            created_at,
            updated_at: Some(created_at),
            parent_session_id: parent_session_id.map(ToOwned::to_owned),
            session_name: None,
            cwd: Some(cwd.to_string()),
            path: Some(path),
            metadata: metadata.cloned(),
        };
        inner.active.insert(
            id,
            ActiveStorage {
                lease,
                lease_error: None,
                closed: false,
            },
        );
        Ok(SessionHandle { metadata })
    }

    pub fn open(&self, id: &str) -> Result<SessionHandle> {
        let mut inner = self.inner.lock().unwrap();
        if let Some(active) = inner.active.get(id) {
            if !active.closed {
                return Ok(SessionHandle {
                    metadata: read_metadata(&inner.db, id, &inner.path)?,
                });
            }
        }
        let now = now_ms();
        let ttl_ms = inner.lease_options.ttl_ms;
        let path = inner.path.clone();
        let tx = inner
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        require_session(&tx, id)?;
        let lease = claim_writer_lease(&tx, id, ttl_ms, now)?;
        let metadata = read_metadata(&tx, id, &path)?;
        tx.commit()?;
        inner.active.insert(
            id.to_string(),
            ActiveStorage {
                lease,
                lease_error: None,
                closed: false,
            },
        );
        Ok(SessionHandle { metadata })
    }

    pub fn list(&self, cwd: Option<&str>) -> Result<Vec<SessionMetadata>> {
        let inner = self.inner.lock().unwrap();
        let mut sql =
            String::from("SELECT id, created_at, cwd, parent_session_id, metadata FROM sessions");
        if cwd.is_some() {
            sql.push_str(" WHERE cwd = ?1");
        }
        sql.push_str(" ORDER BY created_at DESC");
        let mut stmt = inner.db.prepare(&sql)?;
        let mut raw_rows = Vec::new();
        {
            let mut rows = if let Some(cwd) = cwd {
                stmt.query(params![cwd])?
            } else {
                stmt.query([])?
            };
            while let Some(row) = rows.next()? {
                let metadata_raw: Option<String> = row.get(4)?;
                raw_rows.push((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, Option<String>>(3)?,
                    metadata_raw,
                ));
            }
        }
        drop(stmt);
        let mut out = Vec::new();
        for (id, created_at, cwd_val, parent, metadata_raw) in raw_rows {
            let metadata = metadata_raw
                .as_deref()
                .map(serde_json::from_str)
                .transpose()?;
            let name = latest_name(&inner.db, &id)?;
            out.push(SessionMetadata {
                id,
                created_at,
                updated_at: None,
                parent_session_id: parent,
                session_name: name,
                cwd: Some(cwd_val),
                path: Some(inner.path.display().to_string()),
                metadata,
            });
        }
        Ok(out)
    }

    pub fn release(&self, session_id: &str) -> Result<()> {
        let mut inner = self.inner.lock().unwrap();
        if let Some(mut active) = inner.active.remove(session_id) {
            active.closed = true;
            release_writer_lease(&inner.db, session_id, &active.lease)?;
        }
        Ok(())
    }

    pub fn delete(&self, session_id: &str) -> Result<()> {
        let mut inner = self.inner.lock().unwrap();
        if let Some(active) = inner.active.remove(session_id) {
            release_writer_lease(&inner.db, session_id, &active.lease)?;
        }
        let now = now_ms();
        let ttl_ms = inner.lease_options.ttl_ms;
        let tx = inner
            .db
            .transaction_with_behavior(TransactionBehavior::Immediate)?;
        let exists: Option<String> = tx
            .query_row(
                "SELECT id FROM sessions WHERE id = ?1",
                params![session_id],
                |row| row.get(0),
            )
            .optional()?;
        if exists.is_some() {
            let _lease = claim_writer_lease(&tx, session_id, ttl_ms, now)?;
            tx.execute(
                "DELETE FROM branch_entries WHERE session_id = ?1",
                params![session_id],
            )?;
            tx.execute(
                "DELETE FROM branch_tips WHERE session_id = ?1",
                params![session_id],
            )?;
            tx.execute(
                "DELETE FROM facts WHERE session_id = ?1",
                params![session_id],
            )?;
            tx.execute(
                "DELETE FROM lane_moves WHERE session_id = ?1",
                params![session_id],
            )?;
            tx.execute(
                "DELETE FROM lanes WHERE session_id = ?1",
                params![session_id],
            )?;
            tx.execute(
                "DELETE FROM records WHERE session_id = ?1",
                params![session_id],
            )?;
            tx.execute(
                "DELETE FROM entries WHERE session_id = ?1",
                params![session_id],
            )?;
            delete_writer_lease(&tx, session_id)?;
            tx.execute(
                "DELETE FROM session_stats WHERE session_id = ?1",
                params![session_id],
            )?;
            tx.execute(
                "DELETE FROM session_sequences WHERE session_id = ?1",
                params![session_id],
            )?;
            tx.execute("DELETE FROM sessions WHERE id = ?1", params![session_id])?;
        } else {
            delete_writer_lease(&tx, session_id)?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn append_message(
        &self,
        session_id: &str,
        lane: &str,
        id: &str,
        role: Role,
        content: &str,
    ) -> Result<Entry> {
        self.with_write(session_id, |db, _lease| {
            let parent_id: Option<String> = db
                .query_row(
                    "SELECT leaf_id FROM lanes WHERE session_id = ?1 AND lane = ?2",
                    params![session_id, lane],
                    |row| row.get(0),
                )
                .optional()?
                .flatten();
            if parent_id.is_none() {
                let lane_exists: Option<String> = db
                    .query_row(
                        "SELECT lane FROM lanes WHERE session_id = ?1 AND lane = ?2",
                        params![session_id, lane],
                        |row| row.get(0),
                    )
                    .optional()?;
                if lane_exists.is_none() {
                    return Err(SessionError::new(
                        SessionErrorCode::InvalidLane,
                        format!("Lane not found: {lane}"),
                    )
                    .into());
                }
            }
            let exists: Option<String> = db
                .query_row(
                    "SELECT id FROM entries WHERE session_id = ?1 AND id = ?2",
                    params![session_id, id],
                    |row| row.get(0),
                )
                .optional()?;
            if exists.is_some() {
                return Err(SessionError::new(
                    SessionErrorCode::AlreadyExists,
                    format!("ID already exists: {id}"),
                )
                .into());
            }
            let seq: i64 = db.query_row(
                "SELECT next_seq FROM session_sequences WHERE session_id = ?1",
                params![session_id],
                |row| row.get(0),
            )?;
            let timestamp = now_ms();
            let payload = serde_json::json!({
                "message": { "role": role, "content": content }
            });
            db.execute(
                "INSERT INTO entries (session_id, seq, id, parent_id, type, timestamp, payload)
                 VALUES (?1, ?2, ?3, ?4, 'message', ?5, ?6)",
                params![
                    session_id,
                    seq,
                    id,
                    parent_id,
                    timestamp,
                    payload.to_string()
                ],
            )?;
            db.execute(
                "UPDATE lanes SET leaf_id = ?1 WHERE session_id = ?2 AND lane = ?3",
                params![id, session_id, lane],
            )?;
            append_branch_cache(db, session_id, id, parent_id.as_deref(), seq, "message")?;
            db.execute(
                "UPDATE session_stats SET message_count = message_count + 1 WHERE session_id = ?1",
                params![session_id],
            )?;
            db.execute(
                "UPDATE session_sequences SET next_seq = ?1 WHERE session_id = ?2",
                params![seq + 1, session_id],
            )?;
            Ok(Entry::Message {
                id: id.to_string(),
                seq,
                parent_id,
                timestamp,
                message: AgentMessage {
                    role,
                    content: content.to_string(),
                    tool_calls: None,
                    tool_call_id: None,
                },
                terminate: None,
            })
        })
    }

    pub fn create_lane(&self, session_id: &str, lane: &str) -> Result<()> {
        self.with_write(session_id, |db, _| {
            let exists: Option<String> = db
                .query_row(
                    "SELECT lane FROM lanes WHERE session_id = ?1 AND lane = ?2",
                    params![session_id, lane],
                    |row| row.get(0),
                )
                .optional()?;
            if exists.is_some() {
                return Err(SessionError::new(
                    SessionErrorCode::AlreadyExists,
                    format!("Lane already exists: {lane}"),
                )
                .into());
            }
            let seq: i64 = db.query_row(
                "SELECT next_seq FROM session_sequences WHERE session_id = ?1",
                params![session_id],
                |row| row.get(0),
            )?;
            db.execute(
                "INSERT INTO lanes (session_id, lane, leaf_id, open_operation_id) VALUES (?1, ?2, NULL, NULL)",
                params![session_id, lane],
            )?;
            db.execute(
                "INSERT INTO lane_moves (session_id, seq, lane, leaf_id) VALUES (?1, ?2, ?3, NULL)",
                params![session_id, seq, lane],
            )?;
            db.execute(
                "UPDATE session_sequences SET next_seq = ?1 WHERE session_id = ?2",
                params![seq + 1, session_id],
            )?;
            Ok(())
        })
    }

    pub fn set_name(&self, session_id: &str, name: Option<&str>) -> Result<()> {
        self.with_write(session_id, |db, _| {
            let seq: i64 = db.query_row(
                "SELECT next_seq FROM session_sequences WHERE session_id = ?1",
                params![session_id],
                |row| row.get(0),
            )?;
            let value = name.map(|n| serde_json::to_string(&n)).transpose()?;
            db.execute(
                "INSERT INTO facts (session_id, seq, kind, key, value) VALUES (?1, ?2, 'name', NULL, ?3)",
                params![session_id, seq, value],
            )?;
            db.execute(
                "UPDATE session_sequences SET next_seq = ?1 WHERE session_id = ?2",
                params![seq + 1, session_id],
            )?;
            Ok(())
        })
    }

    pub fn entries(&self, session_id: &str) -> Result<Vec<Entry>> {
        let inner = self.inner.lock().unwrap();
        let mut stmt = inner.db.prepare(
            "SELECT id, seq, parent_id, timestamp, payload FROM entries
             WHERE session_id = ?1 AND type = 'message' ORDER BY seq ASC",
        )?;
        let rows = stmt.query_map(params![session_id], |row| {
            let payload: String = row.get(4)?;
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, i64>(3)?,
                payload,
            ))
        })?;
        let mut out = Vec::new();
        for row in rows {
            let (id, seq, parent_id, timestamp, payload) = row?;
            let value: Value = serde_json::from_str(&payload)?;
            let message: AgentMessage = serde_json::from_value(value["message"].clone())?;
            out.push(Entry::Message {
                id,
                seq,
                parent_id,
                timestamp,
                message,
                terminate: None,
            });
        }
        Ok(out)
    }

    pub fn fork(
        &self,
        source_id: &str,
        new_id: &str,
        cwd: &str,
        scope_tree: bool,
    ) -> Result<SessionHandle> {
        let source_entries = self.entries(source_id)?;
        let handle = self.create(Some(new_id), cwd, Some(source_id), None)?;
        if scope_tree {
            for entry in source_entries {
                if let Entry::Message { id, message, .. } = entry {
                    self.append_message(new_id, "main", &id, message.role, &message.content)?;
                }
            }
        } else if let Some(Entry::Message { id, .. }) = source_entries.last() {
            // Branch fork: copy the path to the last message only (already a linear path in this port).
            for entry in &source_entries {
                if let Entry::Message {
                    id: eid,
                    message: msg,
                    ..
                } = entry
                {
                    self.append_message(new_id, "main", eid, msg.role, &msg.content)?;
                    if eid == id {
                        break;
                    }
                }
            }
        }
        Ok(handle)
    }

    pub fn current_lease(&self, session_id: &str) -> Result<Option<WriterLease>> {
        let inner = self.inner.lock().unwrap();
        Ok(read_writer_lease(&inner.db, session_id)?)
    }

    pub fn force_expire_lease(&self, session_id: &str, expires_at_ms: i64) -> Result<()> {
        let inner = self.inner.lock().unwrap();
        inner.db.execute(
            "UPDATE writer_leases SET expires_at_ms = ?1 WHERE session_id = ?2",
            params![expires_at_ms, session_id],
        )?;
        Ok(())
    }

    pub fn heartbeat(&self, session_id: &str) -> Result<bool> {
        let mut inner = self.inner.lock().unwrap();
        let now = now_ms();
        let ttl = inner.lease_options.ttl_ms;
        let mut lease = inner
            .active
            .get(session_id)
            .ok_or_else(|| SessionError::closed(session_id))?
            .lease
            .clone();
        let ok = renew_writer_lease(&inner.db, session_id, &mut lease, now, now + ttl)?;
        if let Some(active) = inner.active.get_mut(session_id) {
            active.lease = lease;
        }
        Ok(ok)
    }

    fn with_write<T>(
        &self,
        session_id: &str,
        op: impl FnOnce(&Connection, &WriterLease) -> Result<T>,
    ) -> Result<T> {
        let mut inner = self.inner.lock().unwrap();
        let now = now_ms();
        let ttl = inner.lease_options.ttl_ms;
        {
            let active = inner
                .active
                .get(session_id)
                .ok_or_else(|| SessionError::closed(session_id))?;
            if let Some(err) = &active.lease_error {
                return Err(err.clone().into());
            }
            if active.closed {
                return Err(SessionError::closed(session_id).into());
            }
        }
        let mut lease = inner.active.get(session_id).unwrap().lease.clone();
        if !renew_writer_lease(&inner.db, session_id, &mut lease, now, now + ttl)? {
            let err = SessionError::lost_writer(session_id);
            if let Some(active) = inner.active.get_mut(session_id) {
                active.lease_error = Some(err.clone());
            }
            return Err(err.into());
        }
        if let Some(active) = inner.active.get_mut(session_id) {
            active.lease = lease.clone();
        }
        op(&inner.db, &lease)
    }
}

fn configure(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(
        "PRAGMA journal_mode=WAL;
         PRAGMA synchronous=FULL;
         PRAGMA busy_timeout=5000;",
    )
}

fn apply_migrations(db: &Connection) -> rusqlite::Result<()> {
    db.execute_batch(INITIAL_SCHEMA)?;
    let applied: Option<String> = db
        .query_row(
            "SELECT id FROM migrations WHERE id = '001_initial'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if applied.is_none() {
        db.execute(
            "INSERT INTO migrations (id, applied_at) VALUES ('001_initial', ?1)",
            params![chrono_now()],
        )?;
    }
    Ok(())
}

fn chrono_now() -> String {
    // RFC3339-ish UTC without extra deps: official uses Date.toISOString().
    let ms = now_ms();
    let secs = ms / 1000;
    format!("{secs}.{:03}Z", ms % 1000)
}

fn require_session(db: &Connection, id: &str) -> Result<()> {
    let exists: Option<String> = db
        .query_row(
            "SELECT id FROM sessions WHERE id = ?1",
            params![id],
            |row| row.get(0),
        )
        .optional()?;
    if exists.is_none() {
        return Err(SessionError::new(
            SessionErrorCode::NotFound,
            format!("Session not found: {id}"),
        )
        .into());
    }
    Ok(())
}

fn read_metadata(db: &Connection, id: &str, path: &Path) -> Result<SessionMetadata> {
    let row = db.query_row(
        "SELECT created_at, cwd, parent_session_id, metadata FROM sessions WHERE id = ?1",
        params![id],
        |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<String>>(3)?,
            ))
        },
    )?;
    let metadata = row
        .3
        .as_deref()
        .map(serde_json::from_str)
        .transpose()
        .map_err(|e| SessionError::new(SessionErrorCode::Storage, e.to_string()))?;
    Ok(SessionMetadata {
        id: id.to_string(),
        created_at: row.0,
        updated_at: Some(row.0),
        parent_session_id: row.2,
        session_name: latest_name(db, id).ok().flatten(),
        cwd: Some(row.1),
        path: Some(path.display().to_string()),
        metadata,
    })
}

fn latest_name(db: &Connection, session_id: &str) -> Result<Option<String>> {
    let value: Option<Option<String>> = db
        .query_row(
            "SELECT value FROM facts WHERE session_id = ?1 AND kind = 'name' ORDER BY seq DESC LIMIT 1",
            params![session_id],
            |row| row.get(0),
        )
        .optional()?;
    match value.flatten() {
        Some(raw) => Ok(Some(serde_json::from_str(&raw)?)),
        None => Ok(None),
    }
}

fn append_branch_cache(
    db: &Connection,
    session_id: &str,
    entry_id: &str,
    parent_id: Option<&str>,
    seq: i64,
    entry_type: &str,
) -> Result<()> {
    match parent_id {
        None => {
            let branch_id = Uuid::now_v7().to_string();
            db.execute(
                "INSERT INTO branch_entries (session_id, branch_id, entry_id, entry_seq, entry_type, custom_type)
                 VALUES (?1, ?2, ?3, ?4, ?5, NULL)",
                params![session_id, branch_id, entry_id, seq, entry_type],
            )?;
            db.execute(
                "INSERT INTO branch_tips (session_id, branch_id, tip_id) VALUES (?1, ?2, ?3)",
                params![session_id, branch_id, entry_id],
            )?;
        }
        Some(parent) => {
            let tip: Option<String> = db
                .query_row(
                    "SELECT branch_id FROM branch_tips WHERE session_id = ?1 AND tip_id = ?2",
                    params![session_id, parent],
                    |row| row.get(0),
                )
                .optional()?;
            if let Some(branch_id) = tip {
                db.execute(
                    "INSERT INTO branch_entries (session_id, branch_id, entry_id, entry_seq, entry_type, custom_type)
                     VALUES (?1, ?2, ?3, ?4, ?5, NULL)",
                    params![session_id, branch_id, entry_id, seq, entry_type],
                )?;
                db.execute(
                    "UPDATE branch_tips SET tip_id = ?1 WHERE session_id = ?2 AND branch_id = ?3",
                    params![entry_id, session_id, branch_id],
                )?;
            } else {
                let parent_branch: Option<(String, i64)> = db
                    .query_row(
                        "SELECT branch_id, entry_seq FROM branch_entries
                         WHERE session_id = ?1 AND entry_id = ?2
                         ORDER BY entry_seq DESC LIMIT 1",
                        params![session_id, parent],
                        |row| Ok((row.get(0)?, row.get(1)?)),
                    )
                    .optional()?;
                let (source_branch, parent_seq) = parent_branch.ok_or_else(|| {
                    SessionError::new(
                        SessionErrorCode::InvalidEntry,
                        format!("Branch cache has no branch containing parent entry {parent}"),
                    )
                })?;
                let branch_id = Uuid::now_v7().to_string();
                db.execute(
                    "INSERT INTO branch_entries (session_id, branch_id, entry_id, entry_seq, entry_type, custom_type)
                     SELECT session_id, ?1, entry_id, entry_seq, entry_type, custom_type
                     FROM branch_entries
                     WHERE session_id = ?2 AND branch_id = ?3 AND entry_seq <= ?4",
                    params![branch_id, session_id, source_branch, parent_seq],
                )?;
                db.execute(
                    "INSERT INTO branch_entries (session_id, branch_id, entry_id, entry_seq, entry_type, custom_type)
                     VALUES (?1, ?2, ?3, ?4, ?5, NULL)",
                    params![session_id, branch_id, entry_id, seq, entry_type],
                )?;
                db.execute(
                    "INSERT INTO branch_tips (session_id, branch_id, tip_id) VALUES (?1, ?2, ?3)",
                    params![session_id, branch_id, entry_id],
                )?;
            }
        }
    }
    Ok(())
}
