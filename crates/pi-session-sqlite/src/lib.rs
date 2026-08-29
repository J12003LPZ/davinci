//! SQLite session backend matching `@earendil-works/pi-session-backend-sqlite-node`.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use pi_session::{now_ms, JsonlSession, SessionEntry, SessionError, SessionSummary};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::Value;
use uuid::Uuid;

pub const INITIAL_MIGRATION_SQL: &str = include_str!("migrations/001_initial.sql");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WriterLease {
    pub owner_id: String,
    pub fence: i64,
    pub expires_at_ms: i64,
}

pub struct SqliteSessionStore {
    conn: Connection,
}

impl SqliteSessionStore {
    pub fn open(path: &Path) -> Result<Self, SessionError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|err| {
                SessionError::storage(format!("Unable to create sqlite directory: {err}"))
            })?;
        }
        let conn = Connection::open(path).map_err(|err| {
            SessionError::storage(format!("Unable to open sqlite database: {err}"))
        })?;
        conn.pragma_update(None, "journal_mode", "WAL")
            .map_err(|err| SessionError::storage(format!("Unable to set WAL: {err}")))?;
        conn.pragma_update(None, "synchronous", "FULL")
            .map_err(|err| SessionError::storage(format!("Unable to set synchronous: {err}")))?;
        conn.busy_timeout(std::time::Duration::from_millis(5000))
            .map_err(|err| SessionError::storage(format!("Unable to set busy timeout: {err}")))?;
        let store = Self { conn };
        store.apply_migrations()?;
        store.ensure_search_schema()?;
        Ok(store)
    }

    pub fn apply_migrations(&self) -> Result<(), SessionError> {
        self.conn
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS migrations (
                    id TEXT PRIMARY KEY,
                    applied_at TEXT NOT NULL
                );",
            )
            .map_err(|err| {
                SessionError::storage(format!("Unable to create migrations table: {err}"))
            })?;
        let applied: Option<String> = self
            .conn
            .query_row(
                "SELECT id FROM migrations WHERE id = ?1",
                ["001_initial.sql"],
                |row| row.get(0),
            )
            .optional()
            .map_err(|err| SessionError::storage(format!("Unable to read migrations: {err}")))?;
        if applied.is_none() {
            self.conn
                .execute_batch(INITIAL_MIGRATION_SQL)
                .map_err(|err| {
                    SessionError::storage(format!("Unable to apply 001_initial.sql: {err}"))
                })?;
            self.conn
                .execute(
                    "INSERT INTO migrations (id, applied_at) VALUES (?1, ?2)",
                    params!["001_initial.sql", chrono_now()],
                )
                .map_err(|err| {
                    SessionError::storage(format!("Unable to record migration: {err}"))
                })?;
        }
        Ok(())
    }

    pub fn ensure_search_schema(&self) -> Result<(), SessionError> {
        self.conn
            .execute_batch(
                "CREATE VIRTUAL TABLE IF NOT EXISTS session_search_fts USING fts5(
                    payload,
                    content='entries',
                    content_rowid='rowid',
                    tokenize='trigram remove_diacritics 1'
                );
                CREATE TRIGGER IF NOT EXISTS session_search_fts_ai AFTER INSERT ON entries BEGIN
                    INSERT INTO session_search_fts(rowid, payload) VALUES (new.rowid, new.payload);
                END;
                CREATE TRIGGER IF NOT EXISTS session_search_fts_ad AFTER DELETE ON entries BEGIN
                    INSERT INTO session_search_fts(session_search_fts, rowid, payload) VALUES('delete', old.rowid, old.payload);
                END;
                CREATE TRIGGER IF NOT EXISTS session_search_fts_au AFTER UPDATE ON entries BEGIN
                    INSERT INTO session_search_fts(session_search_fts, rowid, payload) VALUES('delete', old.rowid, old.payload);
                    INSERT INTO session_search_fts(rowid, payload) VALUES (new.rowid, new.payload);
                END;",
            )
            .map_err(|err| SessionError::storage(format!("Unable to create FTS schema: {err}")))
    }

    pub fn upsert_session(&self, session: &JsonlSession) -> Result<(), SessionError> {
        self.conn
            .execute(
                "INSERT INTO sessions (id, created_at, cwd, parent_session_id, metadata)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(id) DO UPDATE SET
                    cwd = excluded.cwd,
                    parent_session_id = excluded.parent_session_id,
                    metadata = excluded.metadata",
                params![
                    session.header.id,
                    session.header.created_at as i64,
                    session.header.cwd,
                    session.header.parent_session_id,
                    session
                        .header
                        .metadata
                        .as_ref()
                        .map(|value| value.to_string())
                ],
            )
            .map_err(|err| SessionError::storage(format!("Unable to upsert session: {err}")))?;
        for entry in &session.entries {
            self.insert_entry(&session.header.id, entry)?;
        }
        Ok(())
    }

    pub fn insert_entry(&self, session_id: &str, entry: &SessionEntry) -> Result<(), SessionError> {
        let payload = serde_json::to_string(entry)
            .map_err(|err| SessionError::storage(format!("Unable to encode entry: {err}")))?;
        self.conn
            .execute(
                "INSERT OR REPLACE INTO entries (session_id, seq, id, parent_id, type, timestamp, payload)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    session_id,
                    entry.seq as i64,
                    entry.id,
                    entry.parent_id,
                    entry.entry_type,
                    entry.timestamp as i64,
                    payload
                ],
            )
            .map_err(|err| SessionError::storage(format!("Unable to insert entry: {err}")))?;
        Ok(())
    }

    pub fn list_sessions(&self, cwd: Option<&str>) -> Result<Vec<SessionSummary>, SessionError> {
        let mut stmt = if cwd.is_some() {
            self.conn.prepare(
                "SELECT id, created_at, cwd, parent_session_id, metadata FROM sessions WHERE cwd = ?1 ORDER BY created_at DESC",
            )
        } else {
            self.conn.prepare(
                "SELECT id, created_at, cwd, parent_session_id, metadata FROM sessions ORDER BY created_at DESC",
            )
        }
        .map_err(|err| SessionError::storage(format!("Unable to list sessions: {err}")))?;
        let rows = if let Some(cwd) = cwd {
            stmt.query_map([cwd], map_session_row)
        } else {
            stmt.query_map([], map_session_row)
        }
        .map_err(|err| SessionError::storage(format!("Unable to query sessions: {err}")))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|err| SessionError::storage(format!("Unable to read sessions: {err}")))
    }

    pub fn search(&self, query: &str) -> Result<Vec<(String, String)>, SessionError> {
        let escaped = format!("\"{}\"", query.replace('"', "\"\""));
        let mut stmt = self
            .conn
            .prepare(
                "SELECT entries.session_id, entries.payload
                 FROM session_search_fts
                 JOIN entries ON entries.rowid = session_search_fts.rowid
                 WHERE session_search_fts MATCH ?1
                 LIMIT 50",
            )
            .map_err(|err| SessionError::storage(format!("Unable to prepare FTS query: {err}")))?;
        let rows = stmt
            .query_map([escaped], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })
            .map_err(|err| SessionError::storage(format!("Unable to search sessions: {err}")))?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|err| SessionError::storage(format!("Unable to read search hits: {err}")))
    }

    pub fn acquire_writer_lease(
        &self,
        session_id: &str,
        owner_id: &str,
        now: i64,
        expires_at_ms: i64,
    ) -> Result<Option<WriterLease>, SessionError> {
        let row = self
            .conn
            .query_row(
                "INSERT INTO writer_leases (session_id, owner_id, fence, expires_at_ms)
                 VALUES (?1, ?2, 1, ?3)
                 ON CONFLICT(session_id) DO UPDATE SET
                    owner_id = excluded.owner_id,
                    fence = writer_leases.fence + 1,
                    expires_at_ms = excluded.expires_at_ms
                 WHERE writer_leases.expires_at_ms <= ?4
                 RETURNING owner_id, fence, expires_at_ms",
                params![session_id, owner_id, expires_at_ms, now],
                |row| {
                    Ok(WriterLease {
                        owner_id: row.get(0)?,
                        fence: row.get(1)?,
                        expires_at_ms: row.get(2)?,
                    })
                },
            )
            .optional()
            .map_err(|err| {
                SessionError::storage(format!("Unable to acquire writer lease: {err}"))
            })?;
        Ok(row)
    }

    pub fn renew_writer_lease(
        &self,
        session_id: &str,
        lease: &mut WriterLease,
        now: i64,
        expires_at_ms: i64,
    ) -> Result<bool, SessionError> {
        let changed = self
            .conn
            .execute(
                "UPDATE writer_leases
                 SET expires_at_ms = ?1
                 WHERE session_id = ?2 AND owner_id = ?3 AND fence = ?4 AND expires_at_ms > ?5",
                params![expires_at_ms, session_id, lease.owner_id, lease.fence, now],
            )
            .map_err(|err| SessionError::storage(format!("Unable to renew writer lease: {err}")))?;
        if changed == 1 {
            lease.expires_at_ms = expires_at_ms;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn release_writer_lease(
        &self,
        session_id: &str,
        lease: &WriterLease,
    ) -> Result<(), SessionError> {
        self.conn
            .execute(
                "DELETE FROM writer_leases WHERE session_id = ?1 AND owner_id = ?2 AND fence = ?3",
                params![session_id, lease.owner_id, lease.fence],
            )
            .map_err(|err| {
                SessionError::storage(format!("Unable to release writer lease: {err}"))
            })?;
        Ok(())
    }

    pub fn import_jsonl(&self, session: &JsonlSession) -> Result<(), SessionError> {
        self.upsert_session(session)
    }
}

fn map_session_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionSummary> {
    let metadata: Option<String> = row.get(4)?;
    let name = metadata
        .as_deref()
        .and_then(|raw| serde_json::from_str::<Value>(raw).ok())
        .and_then(|value| {
            value
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string)
        });
    Ok(SessionSummary {
        id: row.get(0)?,
        path: std::path::PathBuf::new(),
        cwd: row.get(2)?,
        created_at: row.get::<_, i64>(1)? as u64,
        modified_at: row.get::<_, i64>(1)? as u64,
        name,
        parent_session_id: row.get(3)?,
        source_format: 4,
        all_messages_text: String::new(),
    })
}

fn chrono_now() -> String {
    let ms = now_ms();
    format!("{ms}")
}

pub fn new_lease_owner() -> String {
    Uuid::new_v4().to_string()
}

pub fn now_ms_i64() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn writer_lease_and_fts_roundtrip() {
        let dir = tempdir().unwrap();
        let store = SqliteSessionStore::open(&dir.path().join("sessions.db")).unwrap();
        let mut session = JsonlSession::create(dir.path(), "/tmp/work", Some("fts")).unwrap();
        session
            .append_entry(SessionEntry::message(
                "user",
                serde_json::json!([{"type":"text","text":"unique-lease-token"}]),
            ))
            .unwrap();
        store.import_jsonl(&session).unwrap();
        let hits = store.search("unique-lease-token").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, session.header.id);

        let now = now_ms_i64();
        let owner = new_lease_owner();
        let lease = store
            .acquire_writer_lease(&session.header.id, &owner, now, now + 10_000)
            .unwrap()
            .expect("lease");
        assert_eq!(lease.owner_id, owner);
        let other = store
            .acquire_writer_lease(&session.header.id, "other", now, now + 10_000)
            .unwrap();
        assert!(other.is_none());
        store
            .release_writer_lease(&session.header.id, &lease)
            .unwrap();
        let again = store
            .acquire_writer_lease(&session.header.id, "other", now + 1, now + 20_000)
            .unwrap();
        assert!(again.is_some());
    }

    #[test]
    fn expired_lease_fence_and_cwd_list_match_ts_backend() {
        let dir = tempdir().unwrap();
        let store = SqliteSessionStore::open(&dir.path().join("sessions.db")).unwrap();
        let mut first = JsonlSession::create(dir.path(), "/tmp/one", Some("alpha")).unwrap();
        first
            .append_entry(SessionEntry::message(
                "user",
                serde_json::json!([{"type":"text","text":"search-alpha"}]),
            ))
            .unwrap();
        first
            .append_entry(SessionEntry::label_change(
                &first.entries[0].id,
                Some("keep"),
            ))
            .unwrap();
        store.import_jsonl(&first).unwrap();

        let mut second = JsonlSession::create(dir.path(), "/tmp/two", Some("beta")).unwrap();
        second
            .append_entry(SessionEntry::message(
                "assistant",
                serde_json::json!([{"type":"text","text":"search-beta"}]),
            ))
            .unwrap();
        store.import_jsonl(&second).unwrap();

        let listed = store.list_sessions(Some("/tmp/one")).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, first.header.id);
        assert_eq!(listed[0].name.as_deref(), Some("alpha"));

        let hits = store.search("search-alpha").unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].0, first.header.id);

        let now = now_ms_i64();
        let lease = store
            .acquire_writer_lease(&first.header.id, "owner-a", now, now + 5)
            .unwrap()
            .expect("lease");
        assert_eq!(lease.fence, 1);
        let stolen = store
            .acquire_writer_lease(&first.header.id, "owner-b", now + 10, now + 20)
            .unwrap()
            .expect("expired lease is stealable");
        assert_eq!(stolen.owner_id, "owner-b");
        assert_eq!(stolen.fence, 2);
        assert!(!store
            .renew_writer_lease(&first.header.id, &mut lease.clone(), now + 11, now + 30)
            .unwrap());
        assert!(store
            .renew_writer_lease(&first.header.id, &mut stolen.clone(), now + 11, now + 30)
            .unwrap());
    }
}
