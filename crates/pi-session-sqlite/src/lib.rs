use async_trait::async_trait;
use pi_core::{Message, Role, SessionMetadata, ToolCall, WriterLease};
use rusqlite::{params, Connection, OptionalExtension};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SessionStoreError {
    #[error("Database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("Serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("Session not found: {0}")]
    SessionNotFound(String),
    #[error("Writer lease conflict for session: {0}")]
    LeaseConflict(String),
}

pub type Result<T> = std::result::Result<T, SessionStoreError>;

#[async_trait]
pub trait SessionStore: Send + Sync {
    async fn create_session(
        &self,
        id: &str,
        title: &str,
        tags: &[String],
    ) -> Result<SessionMetadata>;
    async fn get_session(&self, id: &str) -> Result<Option<SessionMetadata>>;
    async fn list_sessions(&self) -> Result<Vec<SessionMetadata>>;
    async fn delete_session(&self, id: &str) -> Result<bool>;

    async fn append_message(&self, message: &Message) -> Result<()>;
    async fn get_messages(&self, session_id: &str) -> Result<Vec<Message>>;

    async fn acquire_writer_lease(
        &self,
        session_id: &str,
        holder_id: &str,
        ttl_ms: i64,
    ) -> Result<bool>;
    async fn renew_writer_lease(
        &self,
        session_id: &str,
        holder_id: &str,
        ttl_ms: i64,
    ) -> Result<bool>;
    async fn release_writer_lease(&self, session_id: &str, holder_id: &str) -> Result<bool>;
    async fn get_current_lease(&self, session_id: &str) -> Result<Option<WriterLease>>;
}

#[derive(Clone)]
pub struct SqliteSessionStore {
    conn: Arc<Mutex<Connection>>,
    _db_path: Option<PathBuf>,
}

impl SqliteSessionStore {
    pub fn new_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
            _db_path: None,
        };
        store.init_schema()?;
        Ok(store)
    }

    pub fn new<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open(&path)?;
        let store = Self {
            conn: Arc::new(Mutex::new(conn)),
            _db_path: Some(path.as_ref().to_path_buf()),
        };
        store.init_schema()?;
        Ok(store)
    }

    fn init_schema(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(
            "
            PRAGMA journal_mode = WAL;
            PRAGMA synchronous = NORMAL;
            PRAGMA foreign_keys = ON;

            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                tags_json TEXT NOT NULL DEFAULT '[]'
            );

            CREATE TABLE IF NOT EXISTS messages (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                role TEXT NOT NULL,
                content TEXT NOT NULL,
                tool_calls_json TEXT,
                tool_call_id TEXT,
                timestamp INTEGER NOT NULL,
                FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE
            );

            CREATE INDEX IF NOT EXISTS idx_messages_session ON messages(session_id, timestamp);

            CREATE TABLE IF NOT EXISTS writer_leases (
                session_id TEXT PRIMARY KEY,
                holder_id TEXT NOT NULL,
                acquired_at INTEGER NOT NULL,
                expires_at INTEGER NOT NULL,
                FOREIGN KEY(session_id) REFERENCES sessions(id) ON DELETE CASCADE
            );
            ",
        )?;
        Ok(())
    }

    fn now_ms() -> i64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64
    }
}

#[async_trait]
impl SessionStore for SqliteSessionStore {
    async fn create_session(
        &self,
        id: &str,
        title: &str,
        tags: &[String],
    ) -> Result<SessionMetadata> {
        let now = Self::now_ms();
        let tags_json = serde_json::to_string(tags)?;
        let conn = self.conn.lock().unwrap();

        conn.execute(
            "INSERT INTO sessions (id, title, created_at, updated_at, tags_json)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![id, title, now, now, tags_json],
        )?;

        Ok(SessionMetadata {
            id: id.to_string(),
            title: title.to_string(),
            created_at: now,
            updated_at: now,
            tags: tags.to_vec(),
        })
    }

    async fn get_session(&self, id: &str) -> Result<Option<SessionMetadata>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, title, created_at, updated_at, tags_json FROM sessions WHERE id = ?1",
        )?;

        let res = stmt
            .query_row(params![id], |row| {
                let tags_json: String = row.get(4)?;
                let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
                Ok(SessionMetadata {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    created_at: row.get(2)?,
                    updated_at: row.get(3)?,
                    tags,
                })
            })
            .optional()?;

        Ok(res)
    }

    async fn list_sessions(&self) -> Result<Vec<SessionMetadata>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, title, created_at, updated_at, tags_json FROM sessions ORDER BY updated_at DESC",
        )?;

        let rows = stmt.query_map([], |row| {
            let tags_json: String = row.get(4)?;
            let tags: Vec<String> = serde_json::from_str(&tags_json).unwrap_or_default();
            Ok(SessionMetadata {
                id: row.get(0)?,
                title: row.get(1)?,
                created_at: row.get(2)?,
                updated_at: row.get(3)?,
                tags,
            })
        })?;

        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    }

    async fn delete_session(&self, id: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let changes = conn.execute("DELETE FROM sessions WHERE id = ?1", params![id])?;
        Ok(changes > 0)
    }

    async fn append_message(&self, message: &Message) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        let tool_calls_json = match &message.tool_calls {
            Some(tc) => Some(serde_json::to_string(tc)?),
            None => None,
        };

        let role_str = match message.role {
            Role::System => "system",
            Role::User => "user",
            Role::Assistant => "assistant",
            Role::Tool => "tool",
        };

        conn.execute(
            "INSERT INTO messages (id, session_id, role, content, tool_calls_json, tool_call_id, timestamp)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                message.id,
                message.session_id,
                role_str,
                message.content,
                tool_calls_json,
                message.tool_call_id,
                message.timestamp
            ],
        )?;

        conn.execute(
            "UPDATE sessions SET updated_at = ?1 WHERE id = ?2",
            params![message.timestamp, message.session_id],
        )?;

        Ok(())
    }

    async fn get_messages(&self, session_id: &str) -> Result<Vec<Message>> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, session_id, role, content, tool_calls_json, tool_call_id, timestamp
             FROM messages WHERE session_id = ?1 ORDER BY timestamp ASC",
        )?;

        let rows = stmt.query_map(params![session_id], |row| {
            let role_str: String = row.get(2)?;
            let role = match role_str.as_str() {
                "system" => Role::System,
                "user" => Role::User,
                "assistant" => Role::Assistant,
                "tool" => Role::Tool,
                _ => Role::User,
            };

            let tool_calls_json: Option<String> = row.get(4)?;
            let tool_calls: Option<Vec<ToolCall>> = match tool_calls_json {
                Some(json_str) => serde_json::from_str(&json_str).ok(),
                None => None,
            };

            Ok(Message {
                id: row.get(0)?,
                session_id: row.get(1)?,
                role,
                content: row.get(3)?,
                tool_calls,
                tool_call_id: row.get(5)?,
                timestamp: row.get(6)?,
            })
        })?;

        let mut list = Vec::new();
        for r in rows {
            list.push(r?);
        }
        Ok(list)
    }

    async fn acquire_writer_lease(
        &self,
        session_id: &str,
        holder_id: &str,
        ttl_ms: i64,
    ) -> Result<bool> {
        let now = Self::now_ms();
        let expires_at = now + ttl_ms;
        let conn = self.conn.lock().unwrap();

        // Check if lease is currently active by someone else
        let mut stmt =
            conn.prepare("SELECT holder_id, expires_at FROM writer_leases WHERE session_id = ?1")?;
        let active_lease = stmt
            .query_row(params![session_id], |row| {
                let h: String = row.get(0)?;
                let exp: i64 = row.get(1)?;
                Ok((h, exp))
            })
            .optional()?;

        if let Some((existing_holder, current_expires_at)) = active_lease {
            if current_expires_at > now && existing_holder != holder_id {
                return Ok(false);
            }
        }

        conn.execute(
            "INSERT INTO writer_leases (session_id, holder_id, acquired_at, expires_at)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(session_id) DO UPDATE SET
                holder_id = excluded.holder_id,
                acquired_at = excluded.acquired_at,
                expires_at = excluded.expires_at",
            params![session_id, holder_id, now, expires_at],
        )?;

        Ok(true)
    }

    async fn renew_writer_lease(
        &self,
        session_id: &str,
        holder_id: &str,
        ttl_ms: i64,
    ) -> Result<bool> {
        let now = Self::now_ms();
        let new_expires_at = now + ttl_ms;
        let conn = self.conn.lock().unwrap();

        let changes = conn.execute(
            "UPDATE writer_leases SET expires_at = ?1
             WHERE session_id = ?2 AND holder_id = ?3 AND expires_at > ?4",
            params![new_expires_at, session_id, holder_id, now],
        )?;

        Ok(changes > 0)
    }

    async fn release_writer_lease(&self, session_id: &str, holder_id: &str) -> Result<bool> {
        let conn = self.conn.lock().unwrap();
        let changes = conn.execute(
            "DELETE FROM writer_leases WHERE session_id = ?1 AND holder_id = ?2",
            params![session_id, holder_id],
        )?;
        Ok(changes > 0)
    }

    async fn get_current_lease(&self, session_id: &str) -> Result<Option<WriterLease>> {
        let now = Self::now_ms();
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT session_id, holder_id, acquired_at, expires_at FROM writer_leases
             WHERE session_id = ?1 AND expires_at > ?2",
        )?;

        let res = stmt
            .query_row(params![session_id, now], |row| {
                Ok(WriterLease {
                    session_id: row.get(0)?,
                    holder_id: row.get(1)?,
                    acquired_at: row.get(2)?,
                    expires_at: row.get(3)?,
                })
            })
            .optional()?;

        Ok(res)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_session_lifecycle() {
        let store = SqliteSessionStore::new_in_memory().unwrap();
        let session = store
            .create_session("sess-1", "Test Session", &["ai".to_string()])
            .await
            .unwrap();

        assert_eq!(session.id, "sess-1");
        assert_eq!(session.title, "Test Session");

        let fetched = store.get_session("sess-1").await.unwrap().unwrap();
        assert_eq!(fetched.id, "sess-1");
        assert_eq!(fetched.tags, vec!["ai".to_string()]);

        let sessions = store.list_sessions().await.unwrap();
        assert_eq!(sessions.len(), 1);

        let deleted = store.delete_session("sess-1").await.unwrap();
        assert!(deleted);
        assert!(store.get_session("sess-1").await.unwrap().is_none());
    }

    #[tokio::test]
    async fn test_writer_lease_semantics() {
        let store = SqliteSessionStore::new_in_memory().unwrap();
        store
            .create_session("sess-lease", "Lease Test", &[])
            .await
            .unwrap();

        // Acquire lease
        let ok = store
            .acquire_writer_lease("sess-lease", "worker-1", 10000)
            .await
            .unwrap();
        assert!(ok);

        // Same worker re-acquire/renew succeeds
        let renew_ok = store
            .renew_writer_lease("sess-lease", "worker-1", 20000)
            .await
            .unwrap();
        assert!(renew_ok);

        // Competing worker fails
        let conflict = store
            .acquire_writer_lease("sess-lease", "worker-2", 10000)
            .await
            .unwrap();
        assert!(!conflict);

        // Current lease check
        let lease = store
            .get_current_lease("sess-lease")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(lease.holder_id, "worker-1");

        // Release lease
        let released = store
            .release_writer_lease("sess-lease", "worker-1")
            .await
            .unwrap();
        assert!(released);

        // Competing worker can now acquire
        let ok2 = store
            .acquire_writer_lease("sess-lease", "worker-2", 10000)
            .await
            .unwrap();
        assert!(ok2);
    }
}
