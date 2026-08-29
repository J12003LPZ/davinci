//! SQLite `SessionRepo` matching TypeScript sqlite-node + shared conformance.

use pi_session::backend::{
    BackendError, BackendErrorCode, CreateOptions, ForkOptions, MutationSink, Session, SessionMeta,
    SessionRepository, TreeMutation, TreeState,
};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::{apply_migrations, SqliteError};

const MUTATION_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS mutation_log (
  session_id TEXT NOT NULL,
  seq INTEGER NOT NULL,
  kind TEXT NOT NULL,
  payload TEXT NOT NULL,
  PRIMARY KEY (session_id, seq)
);
"#;

struct SqliteDb {
    conn: Connection,
    owner_id: String,
}

struct SqliteSink {
    db: Arc<Mutex<SqliteDb>>,
}

impl MutationSink for SqliteSink {
    fn persist(&self, session_id: &str, mutation: &TreeMutation) -> Result<(), BackendError> {
        let db = self
            .db
            .lock()
            .map_err(|_| BackendError::new(BackendErrorCode::Storage, "sqlite lock poisoned"))?;
        persist_mutation(&db.conn, session_id, mutation)
            .map_err(|e| BackendError::new(BackendErrorCode::Storage, e.to_string()))
    }
}

pub struct SqliteSessionRepo {
    path: PathBuf,
    db: Arc<Mutex<SqliteDb>>,
    sessions: Mutex<HashMap<String, Session>>,
}

impl SqliteSessionRepo {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, SqliteError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| SqliteError::Message(e.to_string()))?;
        }
        let conn = Connection::open(&path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL;")?;
        conn.busy_timeout(std::time::Duration::from_millis(5000))?;
        apply_migrations(&conn)?;
        conn.execute_batch(MUTATION_SQL)?;
        conn.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS entries_fts USING fts5(session_id, type, body);",
        )?;
        Ok(Self {
            path,
            db: Arc::new(Mutex::new(SqliteDb {
                conn,
                owner_id: uuid::Uuid::new_v4().to_string(),
            })),
            sessions: Mutex::new(HashMap::new()),
        })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn sink(&self) -> Arc<dyn MutationSink> {
        Arc::new(SqliteSink {
            db: Arc::clone(&self.db),
        })
    }

    fn acquire(&self, session_id: &str) -> Result<(), BackendError> {
        let db = self
            .db
            .lock()
            .map_err(|_| BackendError::new(BackendErrorCode::Storage, "sqlite lock poisoned"))?;
        let now = now_ms() as i64;
        let expires = now + 30_000;
        let existing: Option<(String, i64)> = db
            .conn
            .query_row(
                "SELECT owner_id, expires_at_ms FROM writer_leases WHERE session_id = ?1",
                [session_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()
            .map_err(|e| BackendError::new(BackendErrorCode::Storage, e.to_string()))?;
        if let Some((owner, exp)) = existing {
            if exp > now && owner != db.owner_id {
                return Err(BackendError::new(
                    BackendErrorCode::Storage,
                    format!("SQLite session {session_id} already has an active writer"),
                ));
            }
        }
        db.conn
            .execute(
                "INSERT INTO writer_leases(session_id, owner_id, fence, expires_at_ms)
                 VALUES (?1, ?2, 1, ?3)
                 ON CONFLICT(session_id) DO UPDATE SET
                   owner_id=excluded.owner_id,
                   fence=writer_leases.fence + 1,
                   expires_at_ms=excluded.expires_at_ms
                 WHERE writer_leases.expires_at_ms <= ?4 OR writer_leases.owner_id = excluded.owner_id",
                params![session_id, db.owner_id, expires, now as i64],
            )
            .map_err(|e| BackendError::new(BackendErrorCode::Storage, e.to_string()))?;
        Ok(())
    }
}

impl SessionRepository for SqliteSessionRepo {
    fn create(&self, options: CreateOptions) -> Result<Session, BackendError> {
        let id = options
            .id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        pi_session::backend::validate_session_id(&id)?;
        let cwd = options.cwd.clone().unwrap_or_else(|| "/".into());
        let created = now_ms();
        {
            let db = self.db.lock().map_err(|_| {
                BackendError::new(BackendErrorCode::Storage, "sqlite lock poisoned")
            })?;
            db.conn
                .execute(
                    "INSERT INTO sessions(id, created_at, cwd, parent_session_id) VALUES (?1, ?2, ?3, ?4)",
                    params![id, created as i64, cwd, options.parent_session_id],
                )
                .map_err(|_| {
                    BackendError::new(
                        BackendErrorCode::AlreadyExists,
                        format!("Session already exists: {id}"),
                    )
                })?;
            db.conn
                .execute(
                    "INSERT INTO session_sequences(session_id, next_seq) VALUES (?1, 1)",
                    [&id],
                )
                .map_err(|e| BackendError::new(BackendErrorCode::Storage, e.to_string()))?;
            db.conn
                .execute(
                    "INSERT INTO session_stats(session_id, message_count, cached_tokens, uncached_tokens, total_tokens, cost_total)
                     VALUES (?1, 0, 0, 0, 0, 0)",
                    [&id],
                )
                .map_err(|e| BackendError::new(BackendErrorCode::Storage, e.to_string()))?;
            db.conn
                .execute(
                    "INSERT INTO lanes(session_id, lane, leaf_id) VALUES (?1, 'main', NULL)",
                    [&id],
                )
                .map_err(|e| BackendError::new(BackendErrorCode::Storage, e.to_string()))?;
        }
        self.acquire(&id)?;
        let session = Session::from_parts(
            SessionMeta {
                id: id.clone(),
                created_at: created,
                parent_session_id: options.parent_session_id,
                cwd: Some(cwd),
            },
            TreeState::new(),
            Some(self.sink()),
        );
        self.sessions
            .lock()
            .map_err(|_| BackendError::new(BackendErrorCode::Storage, "repo lock poisoned"))?
            .insert(id, session.clone());
        Ok(session)
    }

    fn open(&self, metadata: &SessionMeta) -> Result<Session, BackendError> {
        if let Some(existing) = self
            .sessions
            .lock()
            .map_err(|_| BackendError::new(BackendErrorCode::Storage, "repo lock poisoned"))?
            .get(&metadata.id)
            .cloned()
        {
            return Ok(existing);
        }
        let mutations = {
            let db = self.db.lock().map_err(|_| {
                BackendError::new(BackendErrorCode::Storage, "sqlite lock poisoned")
            })?;
            let exists: Option<String> = db
                .conn
                .query_row(
                    "SELECT id FROM sessions WHERE id = ?1",
                    [&metadata.id],
                    |row| row.get(0),
                )
                .optional()
                .map_err(|e| BackendError::new(BackendErrorCode::Storage, e.to_string()))?;
            if exists.is_none() {
                return Err(BackendError::new(
                    BackendErrorCode::NotFound,
                    format!("Session not found: {}", metadata.id),
                ));
            }
            load_mutations(&db.conn, &metadata.id)
                .map_err(|e| BackendError::new(BackendErrorCode::Storage, e.to_string()))?
        };
        self.acquire(&metadata.id)?;
        let session = Session::from_parts(metadata.clone(), TreeState::new(), Some(self.sink()));
        session.apply_loaded_mutations(mutations)?;
        self.sessions
            .lock()
            .map_err(|_| BackendError::new(BackendErrorCode::Storage, "repo lock poisoned"))?
            .insert(metadata.id.clone(), session.clone());
        Ok(session)
    }

    fn list(&self) -> Result<Vec<SessionMeta>, BackendError> {
        let db = self
            .db
            .lock()
            .map_err(|_| BackendError::new(BackendErrorCode::Storage, "sqlite lock poisoned"))?;
        let mut stmt = db
            .conn
            .prepare("SELECT id, created_at, parent_session_id, cwd FROM sessions ORDER BY created_at DESC")
            .map_err(|e| BackendError::new(BackendErrorCode::Storage, e.to_string()))?;
        let rows = stmt
            .query_map([], |row| {
                Ok(SessionMeta {
                    id: row.get(0)?,
                    created_at: row.get::<_, i64>(1)? as u64,
                    parent_session_id: row.get(2)?,
                    cwd: row.get(3)?,
                })
            })
            .map_err(|e| BackendError::new(BackendErrorCode::Storage, e.to_string()))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| BackendError::new(BackendErrorCode::Storage, e.to_string()))
    }

    fn delete(&self, metadata: &SessionMeta) -> Result<(), BackendError> {
        self.sessions
            .lock()
            .map_err(|_| BackendError::new(BackendErrorCode::Storage, "repo lock poisoned"))?
            .remove(&metadata.id);
        let db = self
            .db
            .lock()
            .map_err(|_| BackendError::new(BackendErrorCode::Storage, "sqlite lock poisoned"))?;
        for (sql, by_id) in [
            ("DELETE FROM mutation_log WHERE session_id = ?1", false),
            ("DELETE FROM entries WHERE session_id = ?1", false),
            ("DELETE FROM records WHERE session_id = ?1", false),
            ("DELETE FROM lanes WHERE session_id = ?1", false),
            ("DELETE FROM lane_moves WHERE session_id = ?1", false),
            ("DELETE FROM facts WHERE session_id = ?1", false),
            ("DELETE FROM session_sequences WHERE session_id = ?1", false),
            ("DELETE FROM session_stats WHERE session_id = ?1", false),
            ("DELETE FROM writer_leases WHERE session_id = ?1", false),
            ("DELETE FROM sessions WHERE id = ?1", true),
        ] {
            let _ = (by_id, db.conn.execute(sql, [&metadata.id]));
        }
        Ok(())
    }

    fn fork(&self, source: &SessionMeta, options: ForkOptions) -> Result<Session, BackendError> {
        let source_session = self.open(source)?;
        let id = options
            .id
            .clone()
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let dest = self.create(CreateOptions {
            id: Some(id),
            parent_session_id: options
                .parent_session_id
                .clone()
                .or_else(|| Some(source.id.clone())),
            cwd: options.cwd.clone().or_else(|| source.cwd.clone()),
        })?;
        let mutations = source_session.fork_mutations(&options)?;
        dest.apply_loaded_mutations(mutations.clone())?;
        for mutation in mutations {
            dest.persist_only(&mutation)?;
        }
        Ok(dest)
    }
}

fn persist_mutation(
    conn: &Connection,
    session_id: &str,
    mutation: &TreeMutation,
) -> Result<(), SqliteError> {
    let (seq, kind, payload) = match mutation {
        TreeMutation::Entry { entry, .. } => (
            entry.get("seq").and_then(Value::as_u64).unwrap_or(0),
            "entry",
            entry.clone(),
        ),
        TreeMutation::Record { record } => (
            record.get("seq").and_then(Value::as_u64).unwrap_or(0),
            "record",
            record.clone(),
        ),
        TreeMutation::Lane { seq, lane, leaf_id } => {
            (*seq, "lane", json!({"lane": lane, "leafId": leaf_id}))
        }
        TreeMutation::FactName { seq, name } => (*seq, "fact_name", json!({"name": name})),
        TreeMutation::FactLabel {
            seq,
            target_id,
            label,
        } => (
            *seq,
            "fact_label",
            json!({"targetId": target_id, "label": label}),
        ),
    };
    conn.execute(
        "INSERT INTO mutation_log(session_id, seq, kind, payload) VALUES (?1, ?2, ?3, ?4)",
        params![session_id, seq as i64, kind, payload.to_string()],
    )?;
    match mutation {
        TreeMutation::Entry { entry, .. } => {
            let id = entry.get("id").and_then(Value::as_str).unwrap_or("");
            let typ = entry
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("custom");
            let parent = entry.get("parentId").and_then(Value::as_str);
            let ts = entry.get("timestamp").and_then(Value::as_u64).unwrap_or(0) as i64;
            conn.execute(
                "INSERT OR REPLACE INTO entries(session_id, seq, id, parent_id, type, timestamp, payload)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![session_id, seq as i64, id, parent, typ, ts, entry.to_string()],
            )?;
            let _ = conn.execute(
                "INSERT INTO entries_fts(session_id, type, body) VALUES (?1, ?2, ?3)",
                params![session_id, typ, entry.to_string()],
            );
        }
        TreeMutation::Record { record } => {
            let id = record.get("id").and_then(Value::as_str).unwrap_or("");
            let lane = record.get("lane").and_then(Value::as_str).unwrap_or("main");
            let typ = record.get("type").and_then(Value::as_str).unwrap_or("");
            let run_id = record.get("runId").and_then(Value::as_str);
            let op_kind = record.pointer("/intent/kind").and_then(Value::as_str);
            let ts = record.get("timestamp").and_then(Value::as_u64).unwrap_or(0) as i64;
            conn.execute(
                "INSERT OR REPLACE INTO records(session_id, seq, id, lane, run_id, type, op_kind, timestamp, payload)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![session_id, seq as i64, id, lane, run_id, typ, op_kind, ts, record.to_string()],
            )?;
        }
        TreeMutation::Lane { seq, lane, leaf_id } => {
            conn.execute(
                "INSERT OR REPLACE INTO lanes(session_id, lane, leaf_id) VALUES (?1, ?2, ?3)",
                params![session_id, lane, leaf_id],
            )?;
            conn.execute(
                "INSERT OR REPLACE INTO lane_moves(session_id, seq, lane, leaf_id) VALUES (?1, ?2, ?3, ?4)",
                params![session_id, *seq as i64, lane, leaf_id],
            )?;
        }
        TreeMutation::FactName { seq, name } => {
            conn.execute(
                "INSERT INTO facts(session_id, seq, kind, key, value) VALUES (?1, ?2, 'name', NULL, ?3)",
                params![session_id, *seq as i64, name],
            )?;
        }
        TreeMutation::FactLabel {
            seq,
            target_id,
            label,
        } => {
            conn.execute(
                "INSERT INTO facts(session_id, seq, kind, key, value) VALUES (?1, ?2, 'label', ?3, ?4)",
                params![session_id, *seq as i64, target_id, label],
            )?;
        }
    }
    conn.execute(
        "UPDATE session_sequences SET next_seq = ?1 WHERE session_id = ?2",
        params![(seq + 1) as i64, session_id],
    )?;
    Ok(())
}

fn load_mutations(conn: &Connection, session_id: &str) -> Result<Vec<TreeMutation>, SqliteError> {
    let mut stmt = conn.prepare(
        "SELECT seq, kind, payload FROM mutation_log WHERE session_id = ?1 ORDER BY seq",
    )?;
    let rows = stmt.query_map([session_id], |row| {
        Ok((
            row.get::<_, i64>(0)? as u64,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (seq, kind, payload) = row?;
        let value: Value = serde_json::from_str(&payload).unwrap_or(Value::Null);
        out.push(match kind.as_str() {
            "entry" => TreeMutation::Entry {
                lane: None,
                entry: value,
            },
            "record" => TreeMutation::Record { record: value },
            "lane" => TreeMutation::Lane {
                seq,
                lane: value
                    .get("lane")
                    .and_then(Value::as_str)
                    .unwrap_or("main")
                    .to_string(),
                leaf_id: value
                    .get("leafId")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            },
            "fact_name" => TreeMutation::FactName {
                seq,
                name: value
                    .get("name")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            },
            _ => TreeMutation::FactLabel {
                seq,
                target_id: value
                    .get("targetId")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                label: value
                    .get("label")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            },
        });
    }
    Ok(out)
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
