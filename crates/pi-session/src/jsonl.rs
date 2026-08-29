//! JSONL v4 session repository.
//!
//! Scans a directory for `*.jsonl` files whose first line is a v4 header
//! (`{"kind":"header","version":4,...}`) and appends mutations as JSON lines.

use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use pi_core::{next_id, now_ms, SessionError};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::{
    assign_storage_fields, Entry, EntryQuery, ForkOptions, ForkPosition, ForkScope, LaneRecord,
    LogItem, QueryOrder, SessionCreateOptions, SessionMetadata, SessionRepository, SessionStats,
    SessionStore,
};

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

struct JsonlInner {
    metadata: SessionMetadata,
    next_seq: i64,
    entries: Vec<Entry>,
    records: Vec<LaneRecord>,
    lanes: BTreeMap<String, Option<String>>,
    name: Option<String>,
    facts: Vec<LogItem>,
    stats: SessionStats,
}

impl JsonlInner {
    fn new(metadata: SessionMetadata) -> Self {
        let mut lanes = BTreeMap::new();
        lanes.insert("main".to_string(), None);
        Self {
            metadata,
            next_seq: 1,
            entries: Vec::new(),
            records: Vec::new(),
            lanes,
            name: None,
            facts: Vec::new(),
            stats: SessionStats::default(),
        }
    }

    fn allocate_seq(&mut self) -> i64 {
        let seq = self.next_seq;
        self.next_seq += 1;
        seq
    }

    fn apply_entry(&mut self, entry: Entry, lane: &str) -> Result<Entry, SessionError> {
        if !self.lanes.contains_key(lane) {
            return Err(SessionError::invalid_payload(format!(
                "unknown lane {lane}"
            )));
        }
        if self
            .entries
            .iter()
            .any(|existing| existing.id() == entry.id())
            || self
                .records
                .iter()
                .any(|existing| existing.id() == entry.id())
        {
            return Err(SessionError::invalid_payload(format!(
                "duplicate id {}",
                entry.id()
            )));
        }
        if entry.seq() >= self.next_seq {
            self.next_seq = entry.seq() + 1;
        }
        if entry.is_message() {
            self.stats.message_count += 1;
        }
        if let Some(leaf) = self.lanes.get_mut(lane) {
            *leaf = Some(entry.id().to_string());
        }
        self.entries.push(entry.clone());
        Ok(entry)
    }

    fn apply_record(&mut self, record: LaneRecord) -> LaneRecord {
        if record.seq() >= self.next_seq {
            self.next_seq = record.seq() + 1;
        }
        self.records.push(record.clone());
        record
    }

    fn apply_name(&mut self, seq: i64, name: Option<String>) {
        if seq >= self.next_seq {
            self.next_seq = seq + 1;
        }
        self.name = name.clone();
        self.facts.push(LogItem::Fact {
            seq,
            fact: "name".to_string(),
            name,
            target_id: None,
            label: None,
        });
    }

    fn branch_path(&self, start: &str) -> Result<Vec<Entry>, SessionError> {
        let mut path = Vec::new();
        let mut current = Some(start.to_string());
        let mut seen = std::collections::BTreeSet::new();
        while let Some(id) = current {
            if !seen.insert(id.clone()) {
                return Err(SessionError::storage("cycle in entry parent chain"));
            }
            let entry = self
                .entries
                .iter()
                .find(|entry| entry.id() == id)
                .cloned()
                .ok_or_else(|| SessionError::not_found(format!("entry {id} not found")))?;
            current = entry.parent_id().map(str::to_string);
            path.push(entry);
        }
        path.reverse();
        Ok(path)
    }
}

pub struct JsonlSession {
    path: PathBuf,
    inner: Arc<Mutex<JsonlInner>>,
}

impl JsonlSession {
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, JsonlInner>, SessionError> {
        self.inner
            .lock()
            .map_err(|error| SessionError::storage(error.to_string()))
    }

    fn append_line(&self, line: &str) -> Result<(), SessionError> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(|error| SessionError::storage(error.to_string()))?;
        file.write_all(line.as_bytes())
            .map_err(|error| SessionError::storage(error.to_string()))?;
        Ok(())
    }
}

fn encode_entry_line(entry: &Entry, lane: &str) -> Result<String, SessionError> {
    let mut value = serde_json::to_value(entry)
        .map_err(|error| SessionError::invalid_payload(error.to_string()))?;
    if let Value::Object(map) = &mut value {
        map.insert("kind".into(), json!("entry"));
        map.insert("lane".into(), json!(lane));
    }
    Ok(format!("{value}\n"))
}

fn encode_record_line(record: &LaneRecord) -> Result<String, SessionError> {
    let mut value = serde_json::to_value(record)
        .map_err(|error| SessionError::invalid_payload(error.to_string()))?;
    if let Value::Object(map) = &mut value {
        map.insert("kind".into(), json!("record"));
    }
    Ok(format!("{value}\n"))
}

fn encode_name_line(seq: i64, name: Option<&str>) -> String {
    let mut value = json!({
        "kind": "fact",
        "seq": seq,
        "fact": "name",
    });
    if let Some(name) = name {
        value["name"] = json!(name);
    }
    format!("{value}\n")
}

fn parse_header_line(line: &str) -> Option<JsonlHeader> {
    let header: JsonlHeader = serde_json::from_str(line).ok()?;
    if header.kind == "header" && header.version == 4 && !header.id.is_empty() {
        Some(header)
    } else {
        None
    }
}

fn apply_line(inner: &mut JsonlInner, line: &str) -> Result<(), SessionError> {
    let value: Value =
        serde_json::from_str(line).map_err(|error| SessionError::storage(error.to_string()))?;
    let kind = value.get("kind").and_then(Value::as_str).unwrap_or("");
    match kind {
        "entry" => {
            let lane = value
                .get("lane")
                .and_then(Value::as_str)
                .unwrap_or("main")
                .to_string();
            let mut entry_value = value;
            if let Value::Object(map) = &mut entry_value {
                map.remove("kind");
                map.remove("lane");
            }
            let entry: Entry = serde_json::from_value(entry_value)
                .map_err(|error| SessionError::storage(error.to_string()))?;
            inner.apply_entry(entry, &lane)?;
        }
        "record" => {
            let mut record_value = value;
            if let Value::Object(map) = &mut record_value {
                map.remove("kind");
            }
            let record: LaneRecord = serde_json::from_value(record_value)
                .map_err(|error| SessionError::storage(error.to_string()))?;
            inner.apply_record(record);
        }
        "fact" => {
            let seq = value.get("seq").and_then(Value::as_i64).unwrap_or(0);
            let name = value
                .get("name")
                .and_then(Value::as_str)
                .map(str::to_string);
            inner.apply_name(seq, name);
        }
        _ => {}
    }
    Ok(())
}

fn metadata_from_header(header: &JsonlHeader, path: &Path) -> SessionMetadata {
    SessionMetadata {
        id: header.id.clone(),
        created_at: header.created_at,
        cwd: header.cwd.clone(),
        path: path.to_string_lossy().into_owned(),
        parent_session_id: header.parent_session_id.clone(),
        name: None,
        metadata: header.metadata.clone(),
    }
}

fn load_session(path: &Path) -> Result<JsonlSession, SessionError> {
    let file = fs::File::open(path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            SessionError::not_found(format!("session {} not found", path.display()))
        } else {
            SessionError::storage(error.to_string())
        }
    })?;
    let mut lines = BufReader::new(file).lines();
    let first = lines
        .next()
        .transpose()
        .map_err(|error| SessionError::storage(error.to_string()))?
        .ok_or_else(|| SessionError::storage(format!("session {} is empty", path.display())))?;
    let header = parse_header_line(&first).ok_or_else(|| {
        SessionError::storage(format!(
            "session {} is not a JSONL v4 header",
            path.display()
        ))
    })?;
    let mut inner = JsonlInner::new(metadata_from_header(&header, path));
    for line in lines {
        let line = line.map_err(|error| SessionError::storage(error.to_string()))?;
        if line.trim().is_empty() {
            continue;
        }
        apply_line(&mut inner, &line)?;
    }
    inner.metadata.name = inner.name.clone();
    Ok(JsonlSession {
        path: path.to_path_buf(),
        inner: Arc::new(Mutex::new(inner)),
    })
}

fn write_header(path: &Path, header: &JsonlHeader) -> Result<(), SessionError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|error| SessionError::storage(error.to_string()))?;
    }
    let line = serde_json::to_string(header)
        .map_err(|error| SessionError::invalid_payload(error.to_string()))?;
    fs::write(path, format!("{line}\n")).map_err(|error| SessionError::storage(error.to_string()))
}

impl SessionStore for JsonlSession {
    fn metadata(&self) -> Result<SessionMetadata, SessionError> {
        let inner = self.lock()?;
        let mut meta = inner.metadata.clone();
        meta.name = inner.name.clone();
        Ok(meta)
    }

    fn append_entry(&mut self, entry: Entry, lane: &str) -> Result<Entry, SessionError> {
        let stored = {
            let mut inner = self.lock()?;
            let parent_id = inner.lanes.get(lane).cloned().flatten();
            if parent_id.is_none() && !inner.lanes.contains_key(lane) {
                return Err(SessionError::invalid_payload(format!(
                    "unknown lane {lane}"
                )));
            }
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
            let seq = inner.allocate_seq();
            assign_storage_fields(entry, seq, parent_id, now_ms())
        };
        self.append_line(&encode_entry_line(&stored, lane)?)?;
        self.lock()?.apply_entry(stored, lane)
    }

    fn find_entries(&self, query: EntryQuery) -> Result<Vec<Entry>, SessionError> {
        let inner = self.lock()?;
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
        self.lock()?.branch_path(start)
    }

    fn append_record(&mut self, mut record: LaneRecord) -> Result<LaneRecord, SessionError> {
        {
            let mut inner = self.lock()?;
            let seq = inner.allocate_seq();
            record = match record {
                LaneRecord::OperationStarted {
                    id,
                    lane,
                    timestamp,
                    run_id,
                    extra,
                    ..
                } => LaneRecord::OperationStarted {
                    id,
                    seq,
                    lane,
                    timestamp: if timestamp == 0 { now_ms() } else { timestamp },
                    run_id,
                    extra,
                },
                LaneRecord::OperationFinished {
                    id,
                    lane,
                    timestamp,
                    run_id,
                    outcome,
                    ..
                } => LaneRecord::OperationFinished {
                    id,
                    seq,
                    lane,
                    timestamp: if timestamp == 0 { now_ms() } else { timestamp },
                    run_id,
                    outcome,
                },
                LaneRecord::Usage {
                    id,
                    lane,
                    timestamp,
                    usage,
                    extra,
                    ..
                } => LaneRecord::Usage {
                    id,
                    seq,
                    lane,
                    timestamp: if timestamp == 0 { now_ms() } else { timestamp },
                    usage,
                    extra,
                },
                LaneRecord::Other {
                    id,
                    lane,
                    timestamp,
                    record_type,
                    extra,
                    ..
                } => LaneRecord::Other {
                    id,
                    seq,
                    lane,
                    timestamp: if timestamp == 0 { now_ms() } else { timestamp },
                    record_type,
                    extra,
                },
            };
        }
        self.append_line(&encode_record_line(&record)?)?;
        Ok(self.lock()?.apply_record(record))
    }

    fn find_records(&self, lane: Option<&str>) -> Result<Vec<LaneRecord>, SessionError> {
        Ok(self
            .lock()?
            .records
            .iter()
            .filter(|record| lane.is_none_or(|wanted| record.lane() == wanted))
            .cloned()
            .collect())
    }

    fn get_log(&self, limit: Option<usize>) -> Result<Vec<LogItem>, SessionError> {
        let inner = self.lock()?;
        let mut items = Vec::new();
        for entry in &inner.entries {
            items.push(LogItem::Entry {
                seq: entry.seq(),
                entry: entry.clone(),
            });
        }
        for record in &inner.records {
            items.push(LogItem::Record {
                seq: record.seq(),
                record: record.clone(),
            });
        }
        items.extend(inner.facts.iter().cloned());
        items.sort_by_key(|item| match item {
            LogItem::Entry { seq, .. }
            | LogItem::Record { seq, .. }
            | LogItem::Lane { seq, .. }
            | LogItem::Fact { seq, .. } => *seq,
        });
        if let Some(limit) = limit {
            let start = items.len().saturating_sub(limit);
            items = items.split_off(start);
        }
        Ok(items)
    }

    fn set_name(&mut self, name: Option<&str>) -> Result<(), SessionError> {
        let seq = self.lock()?.allocate_seq();
        self.append_line(&encode_name_line(seq, name))?;
        self.lock()?.apply_name(seq, name.map(str::to_string));
        Ok(())
    }

    fn get_name(&self) -> Result<Option<String>, SessionError> {
        Ok(self.lock()?.name.clone())
    }

    fn get_stats(&self) -> Result<SessionStats, SessionError> {
        Ok(self.lock()?.stats.clone())
    }

    fn release(&mut self) -> Result<(), SessionError> {
        Ok(())
    }
}

/// Directory-backed JSONL v4 session repository.
pub struct JsonlSessionRepository {
    dir: PathBuf,
}

impl JsonlSessionRepository {
    pub fn new(dir: impl Into<PathBuf>) -> Result<Self, SessionError> {
        let dir = dir.into();
        fs::create_dir_all(&dir).map_err(|error| SessionError::storage(error.to_string()))?;
        Ok(Self { dir })
    }

    fn session_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.jsonl"))
    }

    fn read_header(path: &Path) -> Option<JsonlHeader> {
        let file = fs::File::open(path).ok()?;
        let first = BufReader::new(file).lines().next()?.ok()?;
        parse_header_line(&first)
    }
}

impl SessionRepository for JsonlSessionRepository {
    fn create(
        &mut self,
        options: SessionCreateOptions,
    ) -> Result<Box<dyn SessionStore>, SessionError> {
        let id = options.id.unwrap_or_else(next_id);
        let path = self.session_path(&id);
        if path.exists() {
            return Err(SessionError::invalid_payload(format!(
                "session {id} exists"
            )));
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
        write_header(&path, &header)?;
        let mut session = load_session(&path)?;
        if let Some(name) = options.name {
            session.set_name(Some(&name))?;
        }
        Ok(Box::new(session))
    }

    fn open(&mut self, metadata: &SessionMetadata) -> Result<Box<dyn SessionStore>, SessionError> {
        let path = if !metadata.path.is_empty() && Path::new(&metadata.path).exists() {
            PathBuf::from(&metadata.path)
        } else {
            self.session_path(&metadata.id)
        };
        Ok(Box::new(load_session(&path)?))
    }

    fn list(&self, cwd: Option<&str>) -> Result<Vec<SessionMetadata>, SessionError> {
        let mut listed = Vec::new();
        let entries =
            fs::read_dir(&self.dir).map_err(|error| SessionError::storage(error.to_string()))?;
        for entry in entries {
            let entry = entry.map_err(|error| SessionError::storage(error.to_string()))?;
            let path = entry.path();
            if path.extension().and_then(|ext| ext.to_str()) != Some("jsonl") {
                continue;
            }
            let Some(header) = Self::read_header(&path) else {
                continue;
            };
            if cwd.is_some_and(|wanted| header.cwd != wanted) {
                continue;
            }
            let session = load_session(&path)?;
            listed.push(session.metadata()?);
        }
        listed.sort_by_key(|item| item.created_at);
        Ok(listed)
    }

    fn delete(&mut self, metadata: &SessionMetadata) -> Result<(), SessionError> {
        let path = if !metadata.path.is_empty() {
            PathBuf::from(&metadata.path)
        } else {
            self.session_path(&metadata.id)
        };
        match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(SessionError::storage(error.to_string())),
        }
    }

    fn fork(
        &mut self,
        source: &dyn SessionStore,
        options: ForkOptions,
    ) -> Result<Box<dyn SessionStore>, SessionError> {
        let source_meta = source.metadata()?;
        let copied = match options.scope {
            ForkScope::Tree => source.find_entries(EntryQuery {
                order: Some(QueryOrder::OldestFirst),
                ..EntryQuery::default()
            })?,
            ForkScope::Branch => {
                let start = options
                    .entry_id
                    .clone()
                    .or_else(|| {
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
                    })
                    .ok_or_else(|| SessionError::invalid_payload("fork target missing"))?;
                let mut path = source.find_entries_on_branch(&start)?;
                if matches!(options.position, ForkPosition::Before) {
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
    use crate::{provision_message, run_conformance};

    fn temp_dir() -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("pi-jsonl-{}-{}", std::process::id(), next_id()));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn lists_only_jsonl_v4_headers() {
        let dir = temp_dir();
        fs::write(
            dir.join("ok.jsonl"),
            r#"{"kind":"header","version":4,"id":"ok","createdAt":1,"cwd":"/proj"}
"#,
        )
        .unwrap();
        fs::write(dir.join("skip.txt"), "not a session\n").unwrap();
        fs::write(
            dir.join("legacy.jsonl"),
            r#"{"type":"session","version":3,"id":"old"}
"#,
        )
        .unwrap();
        let repo = JsonlSessionRepository::new(&dir).unwrap();
        let listed = repo.list(None).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "ok");
        assert_eq!(listed[0].cwd, "/proj");
        assert!(listed[0].path.ends_with("ok.jsonl"));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn create_append_and_reload_message_entries() {
        let dir = temp_dir();
        let mut repo = JsonlSessionRepository::new(&dir).unwrap();
        let mut session = repo
            .create(SessionCreateOptions {
                cwd: "/proj".into(),
                id: Some("chat".into()),
                metadata: Some(json!({"owner":"agent"})),
                ..SessionCreateOptions::default()
            })
            .unwrap();
        let first = session
            .append_entry(provision_message("hello"), "main")
            .unwrap();
        session.set_name(Some("Review")).unwrap();
        let meta = session.metadata().unwrap();
        session.release().unwrap();

        let listed = repo.list(Some("/proj")).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, "chat");
        assert_eq!(listed[0].name.as_deref(), Some("Review"));

        let mut reopened = repo.open(&meta).unwrap();
        let entries = reopened
            .find_entries(EntryQuery {
                order: Some(QueryOrder::OldestFirst),
                ..EntryQuery::default()
            })
            .unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id(), first.id());
        let second = reopened
            .append_entry(provision_message("again"), "main")
            .unwrap();
        assert_eq!(second.parent_id(), Some(first.id()));
        let raw = fs::read_to_string(dir.join("chat.jsonl")).unwrap();
        assert!(raw.lines().next().unwrap().contains("\"kind\":\"header\""));
        assert!(raw.contains("\"kind\":\"entry\""));
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn jsonl_backend_passes_session_conformance() {
        let dir = temp_dir();
        let mut repo = JsonlSessionRepository::new(&dir).unwrap();
        let report = run_conformance(&mut repo);
        let _ = fs::remove_dir_all(dir);
        assert!(report.ok(), "conformance failures: {:?}", report.failed);
    }
}
