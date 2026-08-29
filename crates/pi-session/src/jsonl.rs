//! JSONL v4 session repository.
//!
//! Compatible with the header used by `vendor/pi/packages/agent` JSONL storage:
//! first line is `{ kind: "header", version: 4, id, createdAt, cwd, ... }`.

use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use pi_core::{next_id, now_ms, SessionError};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{
    assign_storage_fields, Entry, EntryQuery, ForkOptions, ForkScope, LaneRecord, LogItem,
    QueryOrder, SessionCreateOptions, SessionMetadata, SessionRepository, SessionStats,
    SessionStore,
};

fn validate_session_id(id: &str) -> Result<(), SessionError> {
    let ok = id.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && id.chars().last().is_some_and(|c| c.is_ascii_alphanumeric())
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
    if ok && !id.is_empty() {
        Ok(())
    } else {
        Err(SessionError::invalid_payload(
            "Session id must be non-empty, contain only alphanumeric characters, '-', '_', and '.', and start and end with an alphanumeric character",
        ))
    }
}

pub fn jsonl_session_directory_name(cwd: &str) -> String {
    format!(
        "--{}--",
        cwd.trim_start_matches(['/', '\\'])
            .replace(['/', '\\', ':'], "-")
    )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct JsonlHeader {
    kind: String,
    version: u32,
    id: String,
    #[serde(rename = "createdAt")]
    created_at: i64,
    cwd: String,
    #[serde(
        rename = "parentSessionId",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    parent_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    metadata: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum JsonlLine {
    Entry {
        entry: Entry,
    },
    Record {
        record: LaneRecord,
    },
    Fact {
        fact: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
    },
}

struct JsonlInner {
    path: PathBuf,
    metadata: SessionMetadata,
    next_seq: i64,
    entries: Vec<Entry>,
    records: Vec<LaneRecord>,
    name: Option<String>,
    stats: SessionStats,
}

impl JsonlInner {
    fn persist_line(&self, line: &JsonlLine) -> Result<(), SessionError> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|error| SessionError::storage(error.to_string()))?;
        let mut encoded = serde_json::to_string(line)
            .map_err(|error| SessionError::invalid_payload(error.to_string()))?;
        encoded.push('\n');
        file.write_all(encoded.as_bytes())
            .map_err(|error| SessionError::storage(error.to_string()))?;
        Ok(())
    }
}

pub struct JsonlSessionStorage {
    inner: Arc<Mutex<JsonlInner>>,
    closed: bool,
}

impl SessionStore for JsonlSessionStorage {
    fn metadata(&self) -> Result<SessionMetadata, SessionError> {
        let inner = self
            .inner
            .lock()
            .map_err(|error| SessionError::storage(error.to_string()))?;
        let mut metadata = inner.metadata.clone();
        metadata.name = inner.name.clone();
        Ok(metadata)
    }

    fn append_entry(&mut self, entry: Entry, _lane: &str) -> Result<Entry, SessionError> {
        if self.closed {
            return Err(SessionError::storage("JSONL session is closed"));
        }
        let mut inner = self
            .inner
            .lock()
            .map_err(|error| SessionError::storage(error.to_string()))?;
        if inner
            .entries
            .iter()
            .any(|existing| existing.id() == entry.id())
            || inner
                .records
                .iter()
                .any(|existing| existing.id() == entry.id())
        {
            return Err(SessionError::invalid_payload(format!(
                "duplicate id {}",
                entry.id()
            )));
        }
        let parent_id = inner.entries.last().map(|entry| entry.id().to_string());
        let seq = inner.next_seq;
        inner.next_seq += 1;
        let stored = assign_storage_fields(entry, seq, parent_id, now_ms());
        if stored.is_message() {
            inner.stats.message_count += 1;
        }
        inner.persist_line(&JsonlLine::Entry {
            entry: stored.clone(),
        })?;
        inner.entries.push(stored.clone());
        Ok(stored)
    }

    fn find_entries(&self, query: EntryQuery) -> Result<Vec<Entry>, SessionError> {
        let inner = self
            .inner
            .lock()
            .map_err(|error| SessionError::storage(error.to_string()))?;
        let mut entries = inner.entries.clone();
        if let Some(entry_type) = query.entry_type.as_deref() {
            entries.retain(|entry| entry.entry_type() == entry_type);
        }
        if matches!(query.order, Some(QueryOrder::NewestFirst)) {
            entries.reverse();
        }
        if let Some(limit) = query.limit {
            entries.truncate(limit);
        }
        Ok(entries)
    }

    fn find_entries_on_branch(&self, start: &str) -> Result<Vec<Entry>, SessionError> {
        let inner = self
            .inner
            .lock()
            .map_err(|error| SessionError::storage(error.to_string()))?;
        let Some(end) = inner.entries.iter().position(|entry| entry.id() == start) else {
            return Err(SessionError::not_found(format!("entry {start} not found")));
        };
        Ok(inner.entries[..=end].to_vec())
    }

    fn append_record(&mut self, record: LaneRecord) -> Result<LaneRecord, SessionError> {
        let mut inner = self
            .inner
            .lock()
            .map_err(|error| SessionError::storage(error.to_string()))?;
        inner.persist_line(&JsonlLine::Record {
            record: record.clone(),
        })?;
        inner.records.push(record.clone());
        Ok(record)
    }

    fn find_records(&self, lane: Option<&str>) -> Result<Vec<LaneRecord>, SessionError> {
        let inner = self
            .inner
            .lock()
            .map_err(|error| SessionError::storage(error.to_string()))?;
        Ok(inner
            .records
            .iter()
            .filter(|record| lane.is_none_or(|lane| record.lane() == lane))
            .cloned()
            .collect())
    }

    fn get_log(&self, limit: Option<usize>) -> Result<Vec<LogItem>, SessionError> {
        let inner = self
            .inner
            .lock()
            .map_err(|error| SessionError::storage(error.to_string()))?;
        let mut items: Vec<LogItem> = inner
            .entries
            .iter()
            .map(|entry| LogItem::Entry {
                seq: entry.seq(),
                entry: entry.clone(),
            })
            .chain(inner.records.iter().map(|record| LogItem::Record {
                seq: record.seq(),
                record: record.clone(),
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
        let mut inner = self
            .inner
            .lock()
            .map_err(|error| SessionError::storage(error.to_string()))?;
        inner.name = name.map(str::to_string);
        inner.persist_line(&JsonlLine::Fact {
            fact: "name".into(),
            name: inner.name.clone(),
        })
    }

    fn get_name(&self) -> Result<Option<String>, SessionError> {
        Ok(self
            .inner
            .lock()
            .map_err(|error| SessionError::storage(error.to_string()))?
            .name
            .clone())
    }

    fn get_stats(&self) -> Result<SessionStats, SessionError> {
        Ok(self
            .inner
            .lock()
            .map_err(|error| SessionError::storage(error.to_string()))?
            .stats
            .clone())
    }

    fn release(&mut self) -> Result<(), SessionError> {
        self.closed = true;
        Ok(())
    }
}

pub struct JsonlSessionRepository {
    root: PathBuf,
}

impl JsonlSessionRepository {
    pub fn open(root: impl AsRef<Path>) -> Result<Self, SessionError> {
        let root = root.as_ref().to_path_buf();
        fs::create_dir_all(&root).map_err(|error| SessionError::storage(error.to_string()))?;
        Ok(Self { root })
    }

    fn session_path(&self, cwd: &str, id: &str) -> PathBuf {
        self.root
            .join(jsonl_session_directory_name(cwd))
            .join(format!("{id}.jsonl"))
    }

    fn load_path(&self, path: &Path) -> Result<JsonlInner, SessionError> {
        let file = fs::File::open(path).map_err(|_| {
            SessionError::not_found(format!("Session not found: {}", path.display()))
        })?;
        let reader = BufReader::new(file);
        let mut header: Option<JsonlHeader> = None;
        let mut entries = Vec::new();
        let mut records = Vec::new();
        let mut name = None;
        let mut next_seq = 1;
        for line in reader.lines() {
            let line = line.map_err(|error| SessionError::storage(error.to_string()))?;
            if line.trim().is_empty() {
                continue;
            }
            let parsed: Value = serde_json::from_str(&line)
                .map_err(|error| SessionError::storage(error.to_string()))?;
            if parsed.get("kind").and_then(Value::as_str) == Some("header") {
                header = Some(
                    serde_json::from_value(parsed)
                        .map_err(|error| SessionError::storage(error.to_string()))?,
                );
                continue;
            }
            match serde_json::from_value::<JsonlLine>(parsed) {
                Ok(JsonlLine::Entry { entry }) => {
                    next_seq = next_seq.max(entry.seq() + 1);
                    entries.push(entry);
                }
                Ok(JsonlLine::Record { record }) => records.push(record),
                Ok(JsonlLine::Fact {
                    fact,
                    name: fact_name,
                }) if fact == "name" => name = fact_name,
                _ => {}
            }
        }
        let header = header.ok_or_else(|| SessionError::storage("missing JSONL header"))?;
        Ok(JsonlInner {
            path: path.to_path_buf(),
            metadata: SessionMetadata {
                id: header.id,
                created_at: header.created_at,
                cwd: header.cwd,
                path: path.to_string_lossy().into_owned(),
                parent_session_id: header.parent_session_id,
                name: name.clone(),
                metadata: header.metadata,
            },
            next_seq,
            entries,
            records,
            name,
            stats: SessionStats::default(),
        })
    }

    fn into_store(inner: JsonlInner) -> Box<dyn SessionStore> {
        Box::new(JsonlSessionStorage {
            inner: Arc::new(Mutex::new(inner)),
            closed: false,
        })
    }
}

impl SessionRepository for JsonlSessionRepository {
    fn create(
        &mut self,
        options: SessionCreateOptions,
    ) -> Result<Box<dyn SessionStore>, SessionError> {
        let id = options.id.unwrap_or_else(next_id);
        validate_session_id(&id)?;
        let path = self.session_path(&options.cwd, &id);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|error| SessionError::storage(error.to_string()))?;
        }
        let header = JsonlHeader {
            kind: "header".into(),
            version: 4,
            id: id.clone(),
            created_at: now_ms(),
            cwd: options.cwd.clone(),
            parent_session_id: options.parent_session_id.clone(),
            metadata: options.metadata.clone(),
        };
        let mut file =
            fs::File::create(&path).map_err(|error| SessionError::storage(error.to_string()))?;
        writeln!(
            file,
            "{}",
            serde_json::to_string(&header)
                .map_err(|error| SessionError::invalid_payload(error.to_string()))?
        )
        .map_err(|error| SessionError::storage(error.to_string()))?;
        let mut inner = JsonlInner {
            path: path.clone(),
            metadata: SessionMetadata {
                id,
                created_at: header.created_at,
                cwd: options.cwd,
                path: path.to_string_lossy().into_owned(),
                parent_session_id: options.parent_session_id,
                name: options.name.clone(),
                metadata: options.metadata,
            },
            next_seq: 1,
            entries: Vec::new(),
            records: Vec::new(),
            name: None,
            stats: SessionStats::default(),
        };
        if let Some(name) = options.name {
            inner.name = Some(name.clone());
            inner.persist_line(&JsonlLine::Fact {
                fact: "name".into(),
                name: Some(name),
            })?;
        }
        Ok(Self::into_store(inner))
    }

    fn open(&mut self, metadata: &SessionMetadata) -> Result<Box<dyn SessionStore>, SessionError> {
        let path = if Path::new(&metadata.path).exists() {
            PathBuf::from(&metadata.path)
        } else {
            self.session_path(&metadata.cwd, &metadata.id)
        };
        Ok(Self::into_store(self.load_path(&path)?))
    }

    fn list(&self, cwd: Option<&str>) -> Result<Vec<SessionMetadata>, SessionError> {
        let mut sessions = Vec::new();
        if !self.root.exists() {
            return Ok(sessions);
        }
        let dirs =
            fs::read_dir(&self.root).map_err(|error| SessionError::storage(error.to_string()))?;
        for entry in dirs {
            let entry = entry.map_err(|error| SessionError::storage(error.to_string()))?;
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let files =
                fs::read_dir(&path).map_err(|error| SessionError::storage(error.to_string()))?;
            for file in files {
                let file = file.map_err(|error| SessionError::storage(error.to_string()))?;
                let file_path = file.path();
                if file_path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
                    continue;
                }
                if let Ok(inner) = self.load_path(&file_path) {
                    if cwd.is_none_or(|cwd| inner.metadata.cwd == cwd) {
                        let mut metadata = inner.metadata;
                        metadata.name = inner.name;
                        sessions.push(metadata);
                    }
                }
            }
        }
        sessions.sort_by_key(|session| session.created_at);
        Ok(sessions)
    }

    fn delete(&mut self, metadata: &SessionMetadata) -> Result<(), SessionError> {
        let path = if Path::new(&metadata.path).exists() {
            PathBuf::from(&metadata.path)
        } else {
            self.session_path(&metadata.cwd, &metadata.id)
        };
        if path.exists() {
            fs::remove_file(&path).map_err(|error| SessionError::storage(error.to_string()))?;
        }
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
                if matches!(options.position, crate::ForkPosition::Before) {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::run_conformance;

    #[test]
    fn jsonl_header_is_version_4() {
        let dir = tempfile::tempdir().unwrap();
        let mut repo = JsonlSessionRepository::open(dir.path()).unwrap();
        let session = repo
            .create(SessionCreateOptions {
                cwd: "/tmp/project".into(),
                id: Some("sess1".into()),
                ..SessionCreateOptions::default()
            })
            .unwrap();
        let path = session.metadata().unwrap().path;
        let first = fs::read_to_string(&path)
            .unwrap()
            .lines()
            .next()
            .unwrap()
            .to_string();
        let header: Value = serde_json::from_str(&first).unwrap();
        assert_eq!(header["kind"], "header");
        assert_eq!(header["version"], 4);
        assert_eq!(header["id"], "sess1");
        assert_eq!(header["cwd"], "/tmp/project");
    }

    #[test]
    fn jsonl_backend_passes_session_conformance() {
        let dir = tempfile::tempdir().unwrap();
        let mut repo = JsonlSessionRepository::open(dir.path()).unwrap();
        let report = run_conformance(&mut repo);
        assert!(report.ok(), "conformance failures: {:?}", report.failed);
    }

    #[test]
    fn directory_name_encodes_cwd() {
        assert_eq!(
            jsonl_session_directory_name("/Users/me/proj"),
            "--Users-me-proj--"
        );
    }
}
