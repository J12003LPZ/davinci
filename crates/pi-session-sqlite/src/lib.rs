//! SQLite session backend matching `packages/session-backends/sqlite-node`.

use pi_session::{discover_sessions, migrate_v3_to_v4, parse_header, SessionInfo};
use rusqlite::{params, Connection, OptionalExtension};
use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};
use thiserror::Error;

const INITIAL_SQL: &str = include_str!("migrations/001_initial.sql");

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
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;
        apply_migrations(&conn)?;
        conn.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS entries_fts USING fts5(session_id, type, body);",
        )?;
        Ok(Self { conn, path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn table_names(&self) -> Result<Vec<String>, SqliteError> {
        let mut stmt = self
            .conn
            .prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name")?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn migration_ids(&self) -> Result<Vec<String>, SqliteError> {
        let mut stmt = self.conn.prepare("SELECT id FROM migrations ORDER BY id")?;
        let rows = stmt.query_map([], |row| row.get(0))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn upsert_session(&self, info: &SessionInfo) -> Result<(), SqliteError> {
        let metadata = json!({
            "path": info.path.to_string_lossy(),
            "name": info.name,
            "modified_at": info.modified_at,
            "source_format": info.source_format
        });
        self.conn.execute(
            "INSERT INTO sessions(id, created_at, cwd, parent_session_id, metadata)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(id) DO UPDATE SET
                cwd=excluded.cwd,
                parent_session_id=excluded.parent_session_id,
                metadata=excluded.metadata",
            params![
                info.id,
                info.created_at,
                info.cwd,
                info.parent_session_id,
                metadata.to_string()
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
        let _ = self
            .conn
            .execute("DELETE FROM entries_fts WHERE session_id = ?1", [&info.id]);
        for (seq, line) in fs::read_to_string(path)
            .map_err(|e| SqliteError::Message(e.to_string()))?
            .lines()
            .enumerate()
        {
            if line.trim().is_empty() {
                continue;
            }
            let value: Value = serde_json::from_str(line).unwrap_or(Value::Null);
            let ty = value
                .get("type")
                .or_else(|| value.get("kind"))
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let id = value
                .get("id")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| format!("e{}", seq + 1));
            let ts = value
                .get("timestamp")
                .and_then(|v| v.as_i64())
                .unwrap_or(info.created_at);
            self.conn.execute(
                "INSERT OR REPLACE INTO entries(session_id, seq, id, parent_id, type, timestamp, payload)
                 VALUES (?1, ?2, ?3, NULL, ?4, ?5, ?6)",
                params![info.id, (seq as i64) + 1, id, ty, ts, line],
            )?;
            let _ = self.conn.execute(
                "INSERT INTO entries_fts(session_id, type, body) VALUES (?1, ?2, ?3)",
                params![info.id, ty, line],
            );
        }
        Ok(info)
    }

    pub fn search(&self, query: &str) -> Result<Vec<SessionInfo>, SqliteError> {
        if let Ok(hits) = self.search_fts(query) {
            if !hits.is_empty() {
                return Ok(hits);
            }
        }
        self.search_like(query)
    }

    fn row_to_info(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionInfo> {
        let metadata_raw: Option<String> = row.get(4)?;
        let metadata: Value = metadata_raw
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok())
            .unwrap_or(Value::Null);
        Ok(SessionInfo {
            id: row.get(0)?,
            path: PathBuf::from(
                metadata
                    .get("path")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default(),
            ),
            cwd: row.get(2)?,
            created_at: row.get(1)?,
            modified_at: metadata
                .get("modified_at")
                .and_then(|v| v.as_i64())
                .unwrap_or(0),
            source_format: metadata
                .get("source_format")
                .and_then(|v| v.as_u64())
                .unwrap_or(4) as u8,
            name: metadata
                .get("name")
                .and_then(|v| v.as_str())
                .map(str::to_string),
            parent_session_id: row.get(3)?,
        })
    }

    fn search_fts(&self, query: &str) -> Result<Vec<SessionInfo>, SqliteError> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT s.id, s.created_at, s.cwd, s.parent_session_id, s.metadata
             FROM sessions s
             JOIN entries_fts f ON f.session_id = s.id
             WHERE entries_fts MATCH ?1
             ORDER BY s.created_at DESC",
        )?;
        let rows = stmt.query_map([query], Self::row_to_info)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    fn search_like(&self, query: &str) -> Result<Vec<SessionInfo>, SqliteError> {
        let mut stmt = self.conn.prepare(
            "SELECT DISTINCT s.id, s.created_at, s.cwd, s.parent_session_id, s.metadata
             FROM sessions s
             JOIN entries e ON e.session_id = s.id
             WHERE e.payload LIKE ?1 OR s.id LIKE ?1 OR s.metadata LIKE ?1
             ORDER BY s.created_at DESC",
        )?;
        let pattern = format!("%{query}%");
        let rows = stmt.query_map([&pattern], Self::row_to_info)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn validate_lease_timing(
        ttl_ms: i64,
        heartbeat_interval_ms: i64,
    ) -> Result<(), SqliteError> {
        if ttl_ms <= 0 {
            return Err(SqliteError::Message(
                "writerLease.ttlMs must be positive".into(),
            ));
        }
        if heartbeat_interval_ms <= 0 || heartbeat_interval_ms >= ttl_ms {
            return Err(SqliteError::Message(
                "writerLease.heartbeatIntervalMs must be positive and less than ttlMs".into(),
            ));
        }
        Ok(())
    }

    pub fn acquire_lease(
        &self,
        session_id: &str,
        owner: &str,
        ttl_ms: i64,
    ) -> Result<bool, SqliteError> {
        let now = now_ms();
        let expires = now + ttl_ms;
        let existing: Option<(String, i64, i64)> = self
            .conn
            .query_row(
                "SELECT owner_id, fence, expires_at_ms FROM writer_leases WHERE session_id = ?1",
                [session_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let mut fence = 1;
        if let Some((current_owner, current_fence, exp)) = existing {
            if exp > now && current_owner != owner {
                return Ok(false);
            }
            fence = current_fence + 1;
        }
        self.conn.execute(
            "INSERT INTO writer_leases(session_id, owner_id, fence, expires_at_ms)
             VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(session_id) DO UPDATE SET
                owner_id=excluded.owner_id,
                fence=excluded.fence,
                expires_at_ms=excluded.expires_at_ms",
            params![session_id, owner, fence, expires],
        )?;
        Ok(true)
    }

    pub fn release_lease(&self, session_id: &str, owner: &str) -> Result<(), SqliteError> {
        self.conn.execute(
            "DELETE FROM writer_leases WHERE session_id = ?1 AND owner_id = ?2",
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

    pub fn next_sequence(&self, session_id: &str) -> Result<i64, SqliteError> {
        let current: Option<i64> = self
            .conn
            .query_row(
                "SELECT next_seq FROM session_sequences WHERE session_id = ?1",
                [session_id],
                |row| row.get(0),
            )
            .optional()?;
        let next = current.unwrap_or(1);
        self.conn.execute(
            "INSERT INTO session_sequences(session_id, next_seq) VALUES (?1, ?2)
             ON CONFLICT(session_id) DO UPDATE SET next_seq = excluded.next_seq",
            params![session_id, next + 1],
        )?;
        Ok(next)
    }

    pub fn upsert_stats(
        &self,
        session_id: &str,
        message_count: i64,
        cached: f64,
        uncached: f64,
        total: f64,
        cost: f64,
    ) -> Result<(), SqliteError> {
        self.conn.execute(
            "INSERT INTO session_stats(session_id, message_count, cached_tokens, uncached_tokens, total_tokens, cost_total)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(session_id) DO UPDATE SET
                message_count=excluded.message_count,
                cached_tokens=excluded.cached_tokens,
                uncached_tokens=excluded.uncached_tokens,
                total_tokens=excluded.total_tokens,
                cost_total=excluded.cost_total",
            params![session_id, message_count, cached, uncached, total, cost],
        )?;
        Ok(())
    }

    pub fn get_stats(&self, session_id: &str) -> Result<Option<Value>, SqliteError> {
        self.conn
            .query_row(
                "SELECT message_count, cached_tokens, uncached_tokens, total_tokens, cost_total
                 FROM session_stats WHERE session_id = ?1",
                [session_id],
                |row| {
                    Ok(json!({
                        "messageCount": row.get::<_, i64>(0)?,
                        "cachedTokens": row.get::<_, f64>(1)?,
                        "uncachedTokens": row.get::<_, f64>(2)?,
                        "totalTokens": row.get::<_, f64>(3)?,
                        "costTotal": row.get::<_, f64>(4)?,
                    }))
                },
            )
            .optional()
            .map_err(Into::into)
    }

    pub fn set_branch_tip(
        &self,
        session_id: &str,
        branch_id: &str,
        tip_id: &str,
    ) -> Result<(), SqliteError> {
        self.conn.execute(
            "INSERT INTO branch_tips(session_id, branch_id, tip_id) VALUES (?1, ?2, ?3)
             ON CONFLICT(session_id, branch_id) DO UPDATE SET tip_id = excluded.tip_id",
            params![session_id, branch_id, tip_id],
        )?;
        Ok(())
    }

    pub fn list_branch_tips(&self, session_id: &str) -> Result<Vec<Value>, SqliteError> {
        let mut stmt = self.conn.prepare(
            "SELECT branch_id, tip_id FROM branch_tips WHERE session_id = ?1 ORDER BY branch_id",
        )?;
        let rows = stmt.query_map([session_id], |row| {
            Ok(json!({"branchId": row.get::<_, String>(0)?, "tipId": row.get::<_, String>(1)?}))
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn add_branch_entry(
        &self,
        session_id: &str,
        branch_id: &str,
        entry_id: &str,
        entry_seq: i64,
        entry_type: &str,
    ) -> Result<(), SqliteError> {
        self.conn.execute(
            "INSERT OR REPLACE INTO branch_entries(session_id, branch_id, entry_id, entry_seq, entry_type, custom_type)
             VALUES (?1, ?2, ?3, ?4, ?5, NULL)",
            params![session_id, branch_id, entry_id, entry_seq, entry_type],
        )?;
        Ok(())
    }

    pub fn list_branch_entries(
        &self,
        session_id: &str,
        branch_id: &str,
    ) -> Result<Vec<Value>, SqliteError> {
        let mut stmt = self.conn.prepare(
            "SELECT entry_id, entry_seq, entry_type FROM branch_entries
             WHERE session_id = ?1 AND branch_id = ?2 ORDER BY entry_seq",
        )?;
        let rows = stmt.query_map(params![session_id, branch_id], |row| {
            Ok(json!({
                "entryId": row.get::<_, String>(0)?,
                "seq": row.get::<_, i64>(1)?,
                "type": row.get::<_, Option<String>>(2)?,
            }))
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn upsert_lane(
        &self,
        session_id: &str,
        lane: &str,
        leaf_id: Option<&str>,
        open_operation_id: Option<&str>,
    ) -> Result<(), SqliteError> {
        self.conn.execute(
            "INSERT INTO lanes(session_id, lane, leaf_id, open_operation_id) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(session_id, lane) DO UPDATE SET
                leaf_id=excluded.leaf_id,
                open_operation_id=excluded.open_operation_id",
            params![session_id, lane, leaf_id, open_operation_id],
        )?;
        Ok(())
    }

    pub fn move_lane(
        &self,
        session_id: &str,
        lane: &str,
        leaf_id: Option<&str>,
    ) -> Result<i64, SqliteError> {
        let seq = self.next_sequence(session_id)?;
        self.upsert_lane(session_id, lane, leaf_id, None)?;
        self.conn.execute(
            "INSERT INTO lane_moves(session_id, seq, lane, leaf_id) VALUES (?1, ?2, ?3, ?4)",
            params![session_id, seq, lane, leaf_id],
        )?;
        Ok(seq)
    }

    pub fn list_lanes(&self, session_id: &str) -> Result<Vec<Value>, SqliteError> {
        let mut stmt = self.conn.prepare(
            "SELECT lane, leaf_id, open_operation_id FROM lanes WHERE session_id = ?1 ORDER BY lane",
        )?;
        let rows = stmt.query_map([session_id], |row| {
            Ok(json!({
                "lane": row.get::<_, String>(0)?,
                "leafId": row.get::<_, Option<String>>(1)?,
                "openOperationId": row.get::<_, Option<String>>(2)?,
            }))
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    pub fn put_record(
        &self,
        session_id: &str,
        id: &str,
        lane: &str,
        ty: &str,
        payload: &Value,
    ) -> Result<i64, SqliteError> {
        let seq = self.next_sequence(session_id)?;
        self.conn.execute(
            "INSERT OR REPLACE INTO records(session_id, seq, id, lane, run_id, type, op_kind, timestamp, payload)
             VALUES (?1, ?2, ?3, ?4, NULL, ?5, NULL, ?6, ?7)",
            params![session_id, seq, id, lane, ty, now_ms(), payload.to_string()],
        )?;
        Ok(seq)
    }

    pub fn list_records(
        &self,
        session_id: &str,
        lane: Option<&str>,
    ) -> Result<Vec<Value>, SqliteError> {
        let mut sql =
            String::from("SELECT id, seq, lane, type, payload FROM records WHERE session_id = ?1");
        if lane.is_some() {
            sql.push_str(" AND lane = ?2");
        }
        sql.push_str(" ORDER BY seq");
        let mut stmt = self.conn.prepare(&sql)?;
        let map_row = |row: &rusqlite::Row<'_>| {
            let payload: String = row.get(4)?;
            Ok(json!({
                "id": row.get::<_, String>(0)?,
                "seq": row.get::<_, i64>(1)?,
                "lane": row.get::<_, String>(2)?,
                "type": row.get::<_, String>(3)?,
                "payload": serde_json::from_str::<Value>(&payload).unwrap_or(Value::Null),
            }))
        };
        let rows = if let Some(lane) = lane {
            stmt.query_map(params![session_id, lane], map_row)?
                .collect::<Result<Vec<_>, _>>()?
        } else {
            stmt.query_map(params![session_id], map_row)?
                .collect::<Result<Vec<_>, _>>()?
        };
        Ok(rows)
    }

    pub fn put_fact(
        &self,
        session_id: &str,
        kind: &str,
        key: Option<&str>,
        value: Option<&str>,
    ) -> Result<i64, SqliteError> {
        let seq = self.next_sequence(session_id)?;
        self.conn.execute(
            "INSERT INTO facts(session_id, seq, kind, key, value) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![session_id, seq, kind, key, value],
        )?;
        Ok(seq)
    }

    pub fn list_facts(
        &self,
        session_id: &str,
        kind: Option<&str>,
    ) -> Result<Vec<Value>, SqliteError> {
        let mut sql = String::from("SELECT seq, kind, key, value FROM facts WHERE session_id = ?1");
        if kind.is_some() {
            sql.push_str(" AND kind = ?2");
        }
        sql.push_str(" ORDER BY seq");
        let mut stmt = self.conn.prepare(&sql)?;
        let map_row = |row: &rusqlite::Row<'_>| {
            Ok(json!({
                "seq": row.get::<_, i64>(0)?,
                "kind": row.get::<_, String>(1)?,
                "key": row.get::<_, Option<String>>(2)?,
                "value": row.get::<_, Option<String>>(3)?,
            }))
        };
        let rows = if let Some(kind) = kind {
            stmt.query_map(params![session_id, kind], map_row)?
                .collect::<Result<Vec<_>, _>>()?
        } else {
            stmt.query_map(params![session_id], map_row)?
                .collect::<Result<Vec<_>, _>>()?
        };
        Ok(rows)
    }
}

fn apply_migrations(conn: &Connection) -> Result<(), SqliteError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS migrations (
            id TEXT PRIMARY KEY,
            applied_at TEXT NOT NULL
        );",
    )?;
    let applied: Option<String> = conn
        .query_row(
            "SELECT id FROM migrations WHERE id = '001_initial.sql'",
            [],
            |row| row.get(0),
        )
        .optional()?;
    if applied.is_none() {
        conn.execute_batch(INITIAL_SQL)?;
        conn.execute(
            "INSERT INTO migrations(id, applied_at) VALUES ('001_initial.sql', ?1)",
            [iso_now()],
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

fn iso_now() -> String {
    format!("{}Z", now_ms())
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

    #[test]
    fn ts_migration_conformance() {
        let dir = tempdir().unwrap();
        let db = SessionSqlite::open(dir.path().join("sessions.sqlite")).unwrap();
        apply_migrations(&db.conn).unwrap();
        assert_eq!(db.migration_ids().unwrap(), vec!["001_initial.sql"]);
        let names = db.table_names().unwrap();
        for required in [
            "migrations",
            "sessions",
            "entries",
            "session_sequences",
            "session_stats",
            "branch_entries",
            "branch_tips",
            "lanes",
            "records",
            "lane_moves",
            "facts",
            "writer_leases",
        ] {
            assert!(names.iter().any(|n| n == required), "missing {required}");
        }
        let err = SessionSqlite::validate_lease_timing(0, 1).unwrap_err();
        assert_eq!(err.to_string(), "writerLease.ttlMs must be positive");
        let err = SessionSqlite::validate_lease_timing(100, 100).unwrap_err();
        assert_eq!(
            err.to_string(),
            "writerLease.heartbeatIntervalMs must be positive and less than ttlMs"
        );
    }

    #[test]
    fn repository_lanes_branches_stats_facts() {
        let dir = tempdir().unwrap();
        let db = SessionSqlite::open(dir.path().join("repo.db")).unwrap();
        db.upsert_stats("s1", 2, 1.0, 3.0, 4.0, 0.01).unwrap();
        let stats = db.get_stats("s1").unwrap().unwrap();
        assert_eq!(stats["messageCount"], 2);
        db.set_branch_tip("s1", "main", "e2").unwrap();
        db.add_branch_entry("s1", "main", "e1", 1, "message")
            .unwrap();
        db.add_branch_entry("s1", "main", "e2", 2, "message")
            .unwrap();
        assert_eq!(db.list_branch_tips("s1").unwrap().len(), 1);
        assert_eq!(db.list_branch_entries("s1", "main").unwrap().len(), 2);
        db.upsert_lane("s1", "default", Some("e2"), None).unwrap();
        db.move_lane("s1", "default", Some("e2")).unwrap();
        assert_eq!(db.list_lanes("s1").unwrap()[0]["lane"], "default");
        db.put_record("s1", "r1", "default", "note", &json!({"ok":true}))
            .unwrap();
        assert_eq!(db.list_records("s1", Some("default")).unwrap().len(), 1);
        db.put_fact("s1", "model", Some("provider"), Some("google"))
            .unwrap();
        assert_eq!(db.list_facts("s1", Some("model")).unwrap().len(), 1);
    }
}
