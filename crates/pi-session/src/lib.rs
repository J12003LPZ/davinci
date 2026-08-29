//! JSONL session store matching `@earendil-works/pi-agent-core` session harness.

mod codec;
mod discovery;
mod errors;
mod types;

pub use codec::{
    encode_header, encode_mutation, metadata_from_header, migrate_v3_to_v4, parse_header,
    parse_mutation,
};
pub use discovery::{
    cwd_encoded_dir, default_agent_dir, default_session_dir, discover_sessions,
    encode_cwd_component, expand_tilde, latest_session, resolve_session_dir, resolve_session_ref,
    SessionSummary,
};
pub use errors::{JsonlDecodeError, SessionError};
pub use types::*;

use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use uuid::Uuid;

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[derive(Debug, Clone)]
pub struct JsonlSession {
    pub path: PathBuf,
    pub header: JsonlV4Header,
    pub entries: Vec<SessionEntry>,
    pub records: Vec<LaneRecord>,
    pub leaf_id: Option<String>,
}

impl JsonlSession {
    pub fn create(
        sessions_root: &Path,
        cwd: &str,
        name: Option<&str>,
    ) -> Result<Self, SessionError> {
        fs::create_dir_all(sessions_root).map_err(|err| {
            SessionError::storage(format!("Unable to create session directory: {err}"))
        })?;
        let dir = sessions_root.join(encode_cwd_component(cwd));
        fs::create_dir_all(&dir).map_err(|err| {
            SessionError::storage(format!("Unable to create cwd session directory: {err}"))
        })?;
        let id = Uuid::new_v4().to_string();
        let path = dir.join(format!("{id}.jsonl"));
        let header = JsonlV4Header {
            kind: "header".into(),
            version: 4,
            id: id.clone(),
            created_at: now_ms(),
            cwd: cwd.to_string(),
            parent_session_id: None,
            legacy_parent_session_path: None,
            metadata: name.map(|n| {
                let mut map = serde_json::Map::new();
                map.insert("name".into(), serde_json::Value::String(n.to_string()));
                serde_json::Value::Object(map)
            }),
        };
        let mut file = File::create(&path).map_err(|err| {
            SessionError::storage(format!("Unable to create session file: {err}"))
        })?;
        file.write_all(encode_header(&header).as_bytes())
            .map_err(|err| {
                SessionError::storage(format!("Unable to write session header: {err}"))
            })?;
        Ok(Self {
            path,
            header,
            entries: Vec::new(),
            records: Vec::new(),
            leaf_id: None,
        })
    }

    pub fn open(path: &Path) -> Result<Self, SessionError> {
        let file = File::open(path).map_err(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                SessionError::not_found(format!("Session file not found: {}", path.display()))
            } else {
                SessionError::storage(format!("Unable to open session file: {err}"))
            }
        })?;
        let reader = BufReader::new(file);
        let mut lines = reader.lines();
        let first = lines
            .next()
            .ok_or_else(|| {
                SessionError::invalid_entry(format!(
                    "Invalid JSONL v4 session {}: line 1 is empty",
                    path.display()
                ))
            })?
            .map_err(|err| SessionError::storage(format!("Unable to read session file: {err}")))?;
        let header = match parse_header(&first) {
            Ok(header) => header,
            Err(_err) if first.contains("\"type\"") || first.contains("\"role\"") => {
                return migrate_v3_to_v4(path, &first, lines);
            }
            Err(err) => {
                return Err(SessionError::invalid_entry(format!(
                    "Invalid JSONL v4 session {}: line 1 {}",
                    path.display(),
                    err
                )));
            }
        };
        let mut session = Self {
            path: path.to_path_buf(),
            header,
            entries: Vec::new(),
            records: Vec::new(),
            leaf_id: None,
        };
        for (index, line) in lines.enumerate() {
            let line_no = index + 2;
            let line = line.map_err(|err| {
                SessionError::storage(format!("Unable to read session file: {err}"))
            })?;
            if line.trim().is_empty() {
                continue;
            }
            match parse_mutation(&line) {
                Ok(SessionMutation::Entry { entry, .. }) => {
                    session.leaf_id = Some(entry.id.clone());
                    session.entries.push(entry);
                }
                Ok(SessionMutation::Record { record, .. }) => session.records.push(record),
                Err(err) => {
                    return Err(SessionError::invalid_entry(format!(
                        "Invalid JSONL v4 session {}: line {line_no} {}",
                        path.display(),
                        err
                    )));
                }
            }
        }
        Ok(session)
    }

    pub fn append_entry(&mut self, mut entry: SessionEntry) -> Result<(), SessionError> {
        entry.seq = self.next_seq();
        if entry.timestamp == 0 {
            entry.timestamp = now_ms();
        }
        if entry.id.is_empty() {
            entry.id = Uuid::new_v4().to_string();
        }
        entry.parent_id = self.leaf_id.clone();
        self.leaf_id = Some(entry.id.clone());
        self.write_line(&encode_mutation(&SessionMutation::Entry {
            lane: None,
            entry: entry.clone(),
        }))?;
        self.entries.push(entry);
        Ok(())
    }

    pub fn fork(&self, entry_id: &str, sessions_root: &Path) -> Result<Self, SessionError> {
        let index = self
            .entries
            .iter()
            .position(|entry| entry.id == entry_id)
            .ok_or_else(|| SessionError::not_found(format!("Entry {entry_id} not found")))?;
        let mut forked = Self::create(sessions_root, &self.header.cwd, None)?;
        forked.header.parent_session_id = Some(self.header.id.clone());
        forked.rewrite_header()?;
        for entry in self.entries.iter().take(index + 1) {
            let mut clone = entry.clone();
            clone.id = Uuid::new_v4().to_string();
            clone.parent_id = forked.leaf_id.clone();
            forked.append_entry(clone)?;
        }
        Ok(forked)
    }

    pub fn clone_session(&self, sessions_root: &Path) -> Result<Self, SessionError> {
        let mut cloned = Self::create(sessions_root, &self.header.cwd, None)?;
        cloned.header.parent_session_id = Some(self.header.id.clone());
        cloned.rewrite_header()?;
        for entry in &self.entries {
            let mut clone = entry.clone();
            clone.id = Uuid::new_v4().to_string();
            clone.parent_id = cloned.leaf_id.clone();
            cloned.append_entry(clone)?;
        }
        Ok(cloned)
    }

    pub fn set_name(&mut self, name: &str) -> Result<(), SessionError> {
        let mut map = match &self.header.metadata {
            Some(serde_json::Value::Object(map)) => map.clone(),
            _ => serde_json::Map::new(),
        };
        map.insert("name".into(), serde_json::Value::String(name.to_string()));
        self.header.metadata = Some(serde_json::Value::Object(map));
        self.rewrite_header()
    }

    pub fn display_name(&self) -> Option<String> {
        self.header
            .metadata
            .as_ref()
            .and_then(|value| value.get("name"))
            .and_then(|value| value.as_str())
            .map(str::to_string)
    }

    fn next_seq(&self) -> u64 {
        self.entries
            .iter()
            .map(|e| e.seq)
            .chain(self.records.iter().map(|r| r.seq))
            .max()
            .unwrap_or(0)
            + 1
    }

    fn write_line(&self, line: &str) -> Result<(), SessionError> {
        let mut file = OpenOptions::new()
            .append(true)
            .open(&self.path)
            .map_err(|err| {
                SessionError::storage(format!("Unable to append session file: {err}"))
            })?;
        file.write_all(line.as_bytes())
            .map_err(|err| SessionError::storage(format!("Unable to append session file: {err}")))
    }

    fn rewrite_header(&self) -> Result<(), SessionError> {
        let rest = fs::read_to_string(&self.path)
            .map_err(|err| SessionError::storage(format!("Unable to read session file: {err}")))?;
        let mut lines = rest.lines();
        let _ = lines.next();
        let mut body = encode_header(&self.header);
        for line in lines {
            body.push_str(line);
            body.push('\n');
        }
        fs::write(&self.path, body).map_err(|err| {
            SessionError::storage(format!("Unable to rewrite session header: {err}"))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn create_append_and_reopen_v4() {
        let dir = tempdir().unwrap();
        let mut session = JsonlSession::create(dir.path(), "/tmp/project", Some("demo")).unwrap();
        session
            .append_entry(SessionEntry::message(
                "user",
                serde_json::json!([{"type":"text","text":"hello"}]),
            ))
            .unwrap();
        let reopened = JsonlSession::open(&session.path).unwrap();
        assert_eq!(reopened.header.version, 4);
        assert_eq!(reopened.entries.len(), 1);
        assert_eq!(reopened.display_name().as_deref(), Some("demo"));
    }

    #[test]
    fn migrate_v3_headerless_message() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("legacy.jsonl");
        fs::write(
            &path,
            r#"{"type":"message","id":"m1","parentId":null,"timestamp":1,"message":{"role":"user","content":[{"type":"text","text":"hi"}]}}
"#,
        )
        .unwrap();
        let session = JsonlSession::open(&path).unwrap();
        assert_eq!(session.header.version, 4);
        assert_eq!(session.entries.len(), 1);
        assert_eq!(session.header.source_format_hint(), 3);
    }
}
