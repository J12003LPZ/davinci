//! JSONL session store matching `@earendil-works/pi-agent-core` session harness.

mod codec;
mod discovery;
mod errors;
mod tree;
mod types;

pub use codec::{
    encode_header, encode_mutation, metadata_from_header, migrate_v3_to_v4, parse_header,
    parse_mutation,
};
pub use discovery::{
    cwd_encoded_dir, default_agent_dir, default_session_dir, discover_sessions,
    encode_cwd_component, expand_tilde, latest_session, resolve_session_dir,
    resolve_session_dir_from, resolve_session_ref, SessionSummary,
};
pub use errors::{JsonlDecodeError, SessionError};
pub use tree::{
    branch_entries, build_context_entries, build_session_path, build_session_tree, entries_since,
    export_session_jsonl, fork_user_messages, resolved_labels, session_usage_stats,
    SessionUsageStats,
};
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

    pub fn set_leaf(&mut self, leaf_id: Option<String>) {
        self.leaf_id = leaf_id;
    }

    /// TS `branchWithSummary`: move the leaf to `branch_from_id`, then append a
    /// `branch_summary` whose `fromId` is the abandoned leaf.
    pub fn branch_with_summary(
        &mut self,
        branch_from_id: Option<String>,
        summary: &str,
        details: serde_json::Value,
        usage: Option<serde_json::Value>,
        from_hook: bool,
    ) -> Result<String, SessionError> {
        if let Some(id) = branch_from_id.as_deref() {
            if !id.is_empty() && !self.entries.iter().any(|entry| entry.id == id) {
                return Err(SessionError::not_found(format!("Entry {id} not found")));
            }
        }
        let from_id = self.leaf_id.clone().unwrap_or_else(|| "root".into());
        self.leaf_id = branch_from_id;
        let mut extra = serde_json::Map::new();
        extra.insert("fromId".into(), serde_json::Value::String(from_id));
        extra.insert("summary".into(), serde_json::Value::String(summary.into()));
        extra.insert("details".into(), details);
        extra.insert("fromHook".into(), serde_json::Value::Bool(from_hook));
        if let Some(usage) = usage {
            extra.insert("usage".into(), usage);
        }
        self.append_entry(SessionEntry {
            id: String::new(),
            entry_type: "branch_summary".into(),
            parent_id: None,
            seq: 0,
            timestamp: 0,
            message: None,
            custom_type: None,
            extra,
        })?;
        Ok(self.leaf_id.clone().unwrap_or_default())
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
        if name.is_empty() {
            map.remove("name");
        } else {
            map.insert("name".into(), serde_json::Value::String(name.to_string()));
        }
        self.header.metadata = if map.is_empty() {
            None
        } else {
            Some(serde_json::Value::Object(map))
        };
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

    #[test]
    fn tree_fork_stats_and_jsonl_export_match_ts_shapes() {
        let dir = tempdir().unwrap();
        let mut session = JsonlSession::create(dir.path(), "/tmp/project", Some("demo")).unwrap();
        session
            .append_entry(SessionEntry::message(
                "user",
                serde_json::json!([{"type":"text","text":"hello"}]),
            ))
            .unwrap();
        let mut assistant = SessionEntry::message(
            "assistant",
            serde_json::json!([
                {"type":"text","text":"hi"},
                {"type":"toolCall","name":"read"}
            ]),
        );
        assistant.message = Some(serde_json::json!({
            "role": "assistant",
            "content": [
                {"type":"text","text":"hi"},
                {"type":"toolCall","name":"read"}
            ],
            "usage": { "input": 10, "output": 4, "cacheRead": 1, "cacheWrite": 2, "cost": { "total": 0.5 } }
        }));
        session.append_entry(assistant).unwrap();
        session
            .append_entry(SessionEntry::label_change(
                &session.entries[0].id,
                Some("root"),
            ))
            .unwrap();

        let tree = build_session_tree(&session.entries);
        assert_eq!(tree.as_array().unwrap().len(), 1);
        assert_eq!(tree[0]["label"], "root");
        assert_eq!(tree[0]["children"].as_array().unwrap().len(), 1);

        let fork = fork_user_messages(&session.entries);
        assert_eq!(fork[0]["text"], "hello");
        assert_eq!(fork[0]["entryId"], session.entries[0].id);

        let missing = entries_since(&session.entries, Some("missing")).unwrap_err();
        assert_eq!(missing, "Entry not found: missing");
        assert_eq!(
            entries_since(&session.entries, Some(&session.entries[0].id))
                .unwrap()
                .len(),
            2
        );

        let stats = session_usage_stats(&session.entries);
        assert_eq!(stats.user_messages, 1);
        assert_eq!(stats.assistant_messages, 1);
        assert_eq!(stats.tool_calls, 1);
        assert_eq!(stats.input, 10);
        assert_eq!(stats.token_total(), 17);
        assert!((stats.cost - 0.5).abs() < f64::EPSILON);

        let out = dir.path().join("export.jsonl");
        export_session_jsonl(&session, &out).unwrap();
        let raw = std::fs::read_to_string(&out).unwrap();
        let header: serde_json::Value = serde_json::from_str(raw.lines().next().unwrap()).unwrap();
        assert_eq!(header["type"], "session");
        assert_eq!(header["id"], session.header.id);
        assert!(raw.contains("\"parentId\":null"));
    }

    #[test]
    fn build_context_entries_follows_leaf_and_latest_compaction() {
        fn msg(id: &str, parent: Option<&str>, role: &str, text: &str) -> SessionEntry {
            SessionEntry {
                id: id.into(),
                entry_type: "message".into(),
                parent_id: parent.map(str::to_string),
                seq: 0,
                timestamp: 0,
                message: Some(serde_json::json!({
                    "role": role,
                    "content": [{"type": "text", "text": text}],
                })),
                custom_type: None,
                extra: serde_json::Map::new(),
            }
        }
        fn compaction(id: &str, parent: &str, summary: &str, first_kept: &str) -> SessionEntry {
            let mut extra = serde_json::Map::new();
            extra.insert("summary".into(), serde_json::json!(summary));
            extra.insert("firstKeptEntryId".into(), serde_json::json!(first_kept));
            SessionEntry {
                id: id.into(),
                entry_type: "compaction".into(),
                parent_id: Some(parent.into()),
                seq: 0,
                timestamp: 0,
                message: None,
                custom_type: None,
                extra,
            }
        }
        fn branch_summary(id: &str, parent: &str, summary: &str, from: &str) -> SessionEntry {
            let mut extra = serde_json::Map::new();
            extra.insert("summary".into(), serde_json::json!(summary));
            extra.insert("fromId".into(), serde_json::json!(from));
            SessionEntry {
                id: id.into(),
                entry_type: "branch_summary".into(),
                parent_id: Some(parent.into()),
                seq: 0,
                timestamp: 0,
                message: None,
                custom_type: None,
                extra,
            }
        }

        let entries = vec![
            msg("1", None, "user", "start"),
            msg("2", Some("1"), "assistant", "r1"),
            msg("3", Some("2"), "user", "q2"),
            msg("4", Some("3"), "assistant", "r2"),
            compaction("5", "4", "Compacted history", "3"),
            msg("6", Some("5"), "user", "q3"),
            msg("7", Some("6"), "assistant", "r3"),
            msg("8", Some("3"), "user", "wrong path"),
            msg("9", Some("8"), "assistant", "wrong response"),
            branch_summary("10", "3", "Tried wrong approach", "9"),
            msg("11", Some("10"), "user", "better approach"),
        ];
        assert_eq!(
            build_context_entries(&entries, Some("7"))
                .iter()
                .map(|entry| entry.id.as_str())
                .collect::<Vec<_>>(),
            vec!["5", "3", "4", "6", "7"]
        );
        assert_eq!(
            build_context_entries(&entries, Some("11"))
                .iter()
                .map(|entry| entry.id.as_str())
                .collect::<Vec<_>>(),
            vec!["1", "2", "3", "10", "11"]
        );
        assert_eq!(
            build_context_entries(&entries, Some("missing"))
                .iter()
                .map(|entry| entry.id.as_str())
                .collect::<Vec<_>>(),
            vec!["1", "2", "3", "10", "11"]
        );
    }
}
