use pi_session::types::*;
use rusqlite::{params, Connection, Result};
use std::path::Path;

pub struct SqliteSessionBackend {
    conn: Connection,
}

impl SqliteSessionBackend {
    pub fn open(path: impl AsRef<Path>) -> Result<Self> {
        let conn = Connection::open(path)?;
        let backend = Self { conn };
        backend.init_schema()?;
        Ok(backend)
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        let backend = Self { conn };
        backend.init_schema()?;
        Ok(backend)
    }

    fn init_schema(&self) -> Result<()> {
        self.conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS sessions (
                id TEXT PRIMARY KEY,
                version INTEGER NOT NULL,
                timestamp TEXT NOT NULL,
                cwd TEXT NOT NULL,
                parent_session TEXT
            );

            CREATE TABLE IF NOT EXISTS entries (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                parent_id TEXT,
                entry_type TEXT NOT NULL,
                timestamp TEXT NOT NULL,
                payload TEXT NOT NULL,
                FOREIGN KEY(session_id) REFERENCES sessions(id)
            );

            CREATE VIRTUAL TABLE IF NOT EXISTS entries_fts USING fts5(
                id UNINDEXED,
                session_id UNINDEXED,
                content
            );
            ",
        )?;
        Ok(())
    }

    pub fn insert_session(&self, header: &SessionHeader) -> Result<()> {
        self.conn.execute(
            "INSERT INTO sessions (id, version, timestamp, cwd, parent_session) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![
                header.id,
                header.version,
                header.timestamp,
                header.cwd,
                header.parent_session
            ],
        )?;
        Ok(())
    }

    pub fn insert_entry(&self, session_id: &str, entry: &SessionEntry) -> Result<()> {
        let (id, parent_id, entry_type, timestamp, payload) = match entry {
            SessionEntry::Message {
                id,
                parent_id,
                timestamp,
                message,
            } => (
                id,
                parent_id.as_deref(),
                "message",
                timestamp,
                message.to_string(),
            ),
            SessionEntry::ThinkingLevelChange {
                id,
                parent_id,
                timestamp,
                thinking_level,
            } => (
                id,
                parent_id.as_deref(),
                "thinking_level_change",
                timestamp,
                thinking_level.clone(),
            ),
            SessionEntry::ModelChange {
                id,
                parent_id,
                timestamp,
                provider,
                model_id,
            } => (
                id,
                parent_id.as_deref(),
                "model_change",
                timestamp,
                format!("{}/{}", provider, model_id),
            ),
            SessionEntry::Compaction {
                id,
                parent_id,
                timestamp,
                summary,
                ..
            } => (
                id,
                parent_id.as_deref(),
                "compaction",
                timestamp,
                summary.clone(),
            ),
            SessionEntry::BranchSummary {
                id,
                parent_id,
                timestamp,
                summary,
                ..
            } => (
                id,
                parent_id.as_deref(),
                "branch_summary",
                timestamp,
                summary.clone(),
            ),
            SessionEntry::Custom {
                id,
                parent_id,
                timestamp,
                custom_type,
                data,
            } => (
                id,
                parent_id.as_deref(),
                custom_type.as_str(),
                timestamp,
                data.to_string(),
            ),
        };

        self.conn.execute(
            "INSERT INTO entries (id, session_id, parent_id, entry_type, timestamp, payload) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![id, session_id, parent_id, entry_type, timestamp, payload],
        )?;

        self.conn.execute(
            "INSERT INTO entries_fts (id, session_id, content) VALUES (?1, ?2, ?3)",
            params![id, session_id, payload],
        )?;

        Ok(())
    }

    pub fn search_fts(&self, session_id: &str, query: &str) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT id FROM entries_fts WHERE session_id = ?1 AND entries_fts MATCH ?2")?;
        let rows = stmt.query_map(params![session_id, query], |row| row.get(0))?;
        let mut results = Vec::new();
        for id in rows {
            results.push(id?);
        }
        Ok(results)
    }
}
