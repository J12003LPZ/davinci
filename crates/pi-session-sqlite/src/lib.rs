//! SQLite session backend with FTS, writer leases, and v3→v4 migrate.

use pi_session::{discover_sessions, migrate_v3_to_v4, parse_header, SessionInfo};
use rusqlite::{params, Connection, OptionalExtension};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SqliteError {
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error(transparent)]
    Session(#[from] pi_session::SessionError),
}

pub struct SessionSqlite {
    conn: Connection,
    path: PathBuf,
}

impl SessionSqlite {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, SqliteError> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| SqliteError::Message(e.to_string()))?;
        }
        let conn = Connection::open(&path)?;
        conn.execute_batch(
            r#"
            PRAGMA journal_mode=WAL;
            CREATE TABLE IF NOT EXISTS schema_migrations (
                version INTEGER PRIMARY KEY,
                applied_at INTEGER NOT NULL
            );
            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                path TEXT NOT NULL,
                cwd TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                modified_at INTEGER NOT NULL,
                source_format INTEGER NOT NULL,
                name TEXT,
                parent_session_id TEXT
            );
            CREATE TABLE IF NOT EXISTS entries (
                session_id TEXT NOT NULL,
                seq INTEGER NOT NULL,
                type TEXT NOT NULL,
                body TEXT NOT NULL,
                PRIMARY KEY (session_id, seq)
            );
            CREATE VIRTUAL TABLE IF NOT EXISTS entries_fts USING fts5(
                session_id, type, body, content='entries', content_rowid='seq'
            );
            CREATE TABLE IF NOT EXISTS writer_leases (
                session_id TEXT PRIMARY KEY,
                owner TEXT NOT NULL,
                expires_at INTEGER NOT NULL
            );
            "#,
        )?;
        migrate_schema(&conn)?;
        Ok(Self { conn, path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn upsert_session(&self, info: &SessionInfo) -> Result<(), SqliteError> {
        self.conn.execute(
            "INSERT INTO sessions(id, path, cwd, created_at, modified_at, source_format, name, parent_session_id)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(id) DO UPDATE SET
                path=excluded.path,
                cwd=excluded.cwd,
                modified_at=excluded.modified_at,
                name=excluded.name,
                parent_session_id=excluded.parent_session_id",
            params![
                info.id,
                info.path.to_string_lossy(),
                info.cwd,
                info.created_at,
                info.modified_at,
                info.source_format,
                info.name,
                info.parent_session_id
            ],
        )?;
        Ok(())
    }

    pub fn index_jsonl(&self, path: &Path) -> Result<SessionInfo, SqliteError> {
        let raw = fs::read_to_string(path).map_err(|e| SqliteError::Message(e.to_string()))?;
        let first = raw.lines().next().unwrap_or("");
        let mut info = if parse_header(first).is_ok() {
            pi_session::read_session_info(path)?
        } else {
            let id = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("migrated")
                .to_string();
            let migrated = migrate_v3_to_v4(&raw, path, "", &id)?;
            fs::write(path, migrated).map_err(|e| SqliteError::Message(e.to_string()))?;
            pi_session::read_session_info(path)?
        };
        info.source_format = 4;
        self.upsert_session(&info)?;
        self.conn
            .execute("DELETE FROM entries WHERE session_id = ?1", [&info.id])?;
        for (seq, line) in fs::read_to_string(path)
            .map_err(|e| SqliteError::Message(e.to_string()))?
            .lines()
            .enumerate()
        {
            if line.trim().is_empty() {
                continue;
            }
            let value: serde_json::Value =
                serde_json::from_str(line).unwrap_or(serde_json::Value::Null);
            let ty = value
                .get("type")
                .or_else(|| value.get("kind"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            self.conn.execute(
                "INSERT OR REPLACE INTO entries(session_id, seq, type, body) VALUES (?1, ?2, ?3, ?4)",
                params![info.id, (seq as i64) + 1, ty, line],
            )?;
        }
        Ok(info)
    }

    pub fn search(&self, query: &str) -> Result<Vec<SessionInfo>, SqliteError> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT s.id, s.path, s.cwd, s.created_at, s.modified_at, s.source_format, s.name, s.parent_session_id
             FROM sessions s
             JOIN entries e ON e.session_id = s.id
             WHERE e.body LIKE ?1 OR s.name LIKE ?1 OR s.id LIKE ?1
             ORDER BY s.modified_at DESC",
        )?;
        let pattern = format!("%{query}%");
        let rows = stmt.query_map([&pattern], |row| {
            Ok(SessionInfo {
                id: row.get(0)?,
                path: PathBuf::from(row.get::<_, String>(1)?),
                cwd: row.get(2)?,
                created_at: row.get(3)?,
                modified_at: row.get(4)?,
                source_format: row.get(5)?,
                name: row.get(6)?,
                parent_session_id: row.get(7)?,
            })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn acquire_lease(
        &self,
        session_id: &str,
        owner: &str,
        ttl_ms: i64,
    ) -> Result<bool, SqliteError> {
        let now = now_ms();
        let expires = now + ttl_ms;
        let existing: Option<(String, i64)> = self
            .conn
            .query_row(
                "SELECT owner, expires_at FROM writer_leases WHERE session_id = ?1",
                [session_id],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?;
        if let Some((current_owner, exp)) = existing {
            if exp > now && current_owner != owner {
                return Ok(false);
            }
        }
        self.conn.execute(
            "INSERT INTO writer_leases(session_id, owner, expires_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(session_id) DO UPDATE SET owner=excluded.owner, expires_at=excluded.expires_at",
            params![session_id, owner, expires],
        )?;
        Ok(true)
    }

    pub fn release_lease(&self, session_id: &str, owner: &str) -> Result<(), SqliteError> {
        self.conn.execute(
            "DELETE FROM writer_leases WHERE session_id = ?1 AND owner = ?2",
            params![session_id, owner],
        )?;
        Ok(())
    }

    pub fn index_tree(&self, root: &Path) -> Result<usize, SqliteError> {
        let sessions = discover_sessions(root, None)?;
        for session in &sessions {
            let _ = self.index_jsonl(&session.path);
        }
        Ok(sessions.len())
    }
}

fn migrate_schema(conn: &Connection) -> Result<(), SqliteError> {
    let version: i64 = conn
        .query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);
    if version < 1 {
        conn.execute(
            "INSERT OR IGNORE INTO schema_migrations(version, applied_at) VALUES (1, ?1)",
            [now_ms()],
        )?;
    }
    Ok(())
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pi_session::create_session;
    use tempfile::tempdir;

    #[test]
    fn fts_and_leases() {
        let dir = tempdir().unwrap();
        let db = SessionSqlite::open(dir.path().join("sessions.db")).unwrap();
        let session = create_session(dir.path(), "/ws", Some("lease-1")).unwrap();
        pi_session::append_entry(
            &session.path,
            &serde_json::json!({"type":"message","id":"m1","text":"hello writer leases"}),
        )
        .unwrap();
        db.index_jsonl(&session.path).unwrap();
        let hits = db.search("writer leases").unwrap();
        assert_eq!(hits[0].id, "lease-1");
        assert!(db.acquire_lease("lease-1", "owner-a", 60_000).unwrap());
        assert!(!db.acquire_lease("lease-1", "owner-b", 60_000).unwrap());
        db.release_lease("lease-1", "owner-a").unwrap();
        assert!(db.acquire_lease("lease-1", "owner-b", 60_000).unwrap());
    }
}
