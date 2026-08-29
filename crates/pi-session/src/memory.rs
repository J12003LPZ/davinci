use std::collections::BTreeMap;

use pi_core::{next_id, now_ms, SessionError};
use serde_json::Value;

use crate::{
    assign_storage_fields, Entry, EntryQuery, ForkOptions, ForkScope, LaneRecord, LogItem, QueryOrder,
    SessionCreateOptions, SessionMetadata, SessionRepository, SessionStats, SessionStore,
};

#[derive(Debug, Clone)]
struct LaneState {
    leaf_id: Option<String>,
}

#[derive(Clone)]
struct SessionInner {
    metadata: SessionMetadata,
    next_seq: i64,
    entries: Vec<Entry>,
    records: Vec<LaneRecord>,
    lanes: BTreeMap<String, LaneState>,
    lane_moves: Vec<(i64, String, Option<String>)>,
    name: Option<String>,
    facts: Vec<LogItem>,
    stats: SessionStats,
}

impl SessionInner {
    fn new(metadata: SessionMetadata) -> Self {
        let mut lanes = BTreeMap::new();
        lanes.insert("main".to_string(), LaneState { leaf_id: None });
        Self {
            metadata,
            next_seq: 1,
            entries: Vec::new(),
            records: Vec::new(),
            lanes,
            lane_moves: Vec::new(),
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

    fn append_entry(&mut self, entry: Entry, lane: &str) -> Result<Entry, SessionError> {
        if !self.lanes.contains_key(lane) {
            return Err(SessionError::invalid_payload(format!("unknown lane {lane}")));
        }
        if self.entries.iter().any(|existing| existing.id() == entry.id())
            || self.records.iter().any(|existing| existing.id() == entry.id())
        {
            return Err(SessionError::invalid_payload(format!(
                "duplicate id {}",
                entry.id()
            )));
        }
        let parent_id = self.lanes[lane].leaf_id.clone();
        let seq = self.allocate_seq();
        let stored = assign_storage_fields(entry, seq, parent_id, now_ms());
        if stored.is_message() {
            self.stats.message_count += 1;
        }
        if let Some(state) = self.lanes.get_mut(lane) {
            state.leaf_id = Some(stored.id().to_string());
        }
        self.entries.push(stored.clone());
        Ok(stored)
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

#[derive(Clone)]
pub struct MemorySession {
    inner: SessionInner,
}

impl MemorySession {
    fn from_inner(inner: SessionInner) -> Self {
        Self { inner }
    }
}

impl SessionStore for MemorySession {
    fn metadata(&self) -> Result<SessionMetadata, SessionError> {
        let mut meta = self.inner.metadata.clone();
        meta.name = self.inner.name.clone();
        Ok(meta)
    }

    fn append_entry(&mut self, entry: Entry, lane: &str) -> Result<Entry, SessionError> {
        self.inner.append_entry(entry, lane)
    }

    fn find_entries(&self, query: EntryQuery) -> Result<Vec<Entry>, SessionError> {
        let mut entries = self.inner.entries.clone();
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
        self.inner.branch_path(start)
    }

    fn append_record(&mut self, mut record: LaneRecord) -> Result<LaneRecord, SessionError> {
        let seq = self.inner.allocate_seq();
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
            } => {
                self.inner.stats.total_tokens += usage.total_tokens as f64;
                self.inner.stats.cost_total += usage.cost.total;
                LaneRecord::Usage {
                    id,
                    seq,
                    lane,
                    timestamp: if timestamp == 0 { now_ms() } else { timestamp },
                    usage,
                    extra,
                }
            }
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
        self.inner.records.push(record.clone());
        Ok(record)
    }

    fn find_records(&self, lane: Option<&str>) -> Result<Vec<LaneRecord>, SessionError> {
        Ok(self
            .inner
            .records
            .iter()
            .filter(|record| lane.is_none_or(|wanted| record.lane() == wanted))
            .cloned()
            .collect())
    }

    fn get_log(&self, limit: Option<usize>) -> Result<Vec<LogItem>, SessionError> {
        let mut items = Vec::new();
        for entry in &self.inner.entries {
            items.push(LogItem::Entry {
                seq: entry.seq(),
                entry: entry.clone(),
            });
        }
        for record in &self.inner.records {
            items.push(LogItem::Record {
                seq: record.seq(),
                record: record.clone(),
            });
        }
        for (seq, lane, leaf_id) in &self.inner.lane_moves {
            items.push(LogItem::Lane {
                seq: *seq,
                lane: lane.clone(),
                leaf_id: leaf_id.clone(),
            });
        }
        items.extend(self.inner.facts.iter().cloned());
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
        let seq = self.inner.allocate_seq();
        self.inner.name = name.map(str::to_string);
        self.inner.facts.push(LogItem::Fact {
            seq,
            fact: "name".to_string(),
            name: name.map(str::to_string),
            target_id: None,
            label: None,
        });
        Ok(())
    }

    fn get_name(&self) -> Result<Option<String>, SessionError> {
        Ok(self.inner.name.clone())
    }

    fn get_stats(&self) -> Result<SessionStats, SessionError> {
        Ok(self.inner.stats.clone())
    }

    fn release(&mut self) -> Result<(), SessionError> {
        Ok(())
    }
}

#[derive(Default)]
pub struct MemorySessionRepository {
    sessions: BTreeMap<String, SessionInner>,
    path: String,
}

impl MemorySessionRepository {
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            sessions: BTreeMap::new(),
            path: path.into(),
        }
    }
}

impl SessionRepository for MemorySessionRepository {
    fn create(&mut self, options: SessionCreateOptions) -> Result<Box<dyn SessionStore>, SessionError> {
        let id = options.id.unwrap_or_else(next_id);
        if self.sessions.contains_key(&id) {
            return Err(SessionError::invalid_payload(format!("session {id} exists")));
        }
        let metadata = SessionMetadata {
            id: id.clone(),
            created_at: now_ms(),
            cwd: options.cwd,
            path: self.path.clone(),
            parent_session_id: options.parent_session_id,
            name: options.name.clone(),
            metadata: options.metadata,
        };
        let mut inner = SessionInner::new(metadata);
        if let Some(name) = options.name {
            inner.name = Some(name);
        }
        self.sessions.insert(id.clone(), inner.clone());
        Ok(Box::new(MemorySession::from_inner(inner)))
    }

    fn open(&mut self, metadata: &SessionMetadata) -> Result<Box<dyn SessionStore>, SessionError> {
        let inner = self
            .sessions
            .get(&metadata.id)
            .cloned()
            .ok_or_else(|| SessionError::not_found(format!("session {} not found", metadata.id)))?;
        Ok(Box::new(MemorySession::from_inner(inner)))
    }

    fn list(&self, cwd: Option<&str>) -> Result<Vec<SessionMetadata>, SessionError> {
        Ok(self
            .sessions
            .values()
            .filter(|session| cwd.is_none_or(|wanted| session.metadata.cwd == wanted))
            .map(|session| {
                let mut meta = session.metadata.clone();
                meta.name = session.name.clone();
                meta
            })
            .collect())
    }

    fn delete(&mut self, metadata: &SessionMetadata) -> Result<(), SessionError> {
        self.sessions.remove(&metadata.id);
        Ok(())
    }

    fn fork(
        &mut self,
        source: &dyn SessionStore,
        options: ForkOptions,
    ) -> Result<Box<dyn SessionStore>, SessionError> {
        let source_meta = source.metadata()?;
        let source_inner = self
            .sessions
            .get(&source_meta.id)
            .cloned()
            .ok_or_else(|| SessionError::not_found(format!("session {} not found", source_meta.id)))?;
        let created = self.create(SessionCreateOptions {
            id: None,
            cwd: options.cwd,
            parent_session_id: Some(source_meta.id.clone()),
            metadata: source_meta.metadata.clone(),
            name: None,
        })?;
        let new_id = created.metadata()?.id;
        let dest = self.sessions.get_mut(&new_id).expect("just created");
        let copied = match options.scope {
            ForkScope::Tree => source_inner.entries.clone(),
            ForkScope::Branch => {
                let start = options
                    .entry_id
                    .clone()
                    .or_else(|| source_inner.lanes.get("main").and_then(|lane| lane.leaf_id.clone()))
                    .ok_or_else(|| SessionError::invalid_payload("fork target missing"))?;
                let mut path = source_inner.branch_path(&start)?;
                if matches!(options.position, crate::ForkPosition::Before) {
                    path.pop();
                }
                path
            }
        };
        dest.entries.clear();
        dest.next_seq = 1;
        dest.stats = SessionStats::default();
        dest.lanes.insert("main".to_string(), LaneState { leaf_id: None });
        for entry in copied {
            dest.append_entry(strip_storage(entry), "main")?;
        }
        let inner = dest.clone();
        let _ = Value::Null;
        Ok(Box::new(MemorySession::from_inner(inner)))
    }
}

fn strip_storage(entry: Entry) -> Entry {
    assign_storage_fields(entry, 0, None, 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::provision_message;

    #[test]
    fn append_assigns_parent_and_seq() {
        let mut repo = MemorySessionRepository::new("memory");
        let mut session = repo
            .create(SessionCreateOptions {
                cwd: "/tmp".into(),
                ..SessionCreateOptions::default()
            })
            .unwrap();
        let first = session.append_entry(provision_message("one"), "main").unwrap();
        let second = session.append_entry(provision_message("two"), "main").unwrap();
        assert_eq!(first.seq(), 1);
        assert_eq!(second.seq(), 2);
        assert_eq!(second.parent_id(), Some(first.id()));
        assert_eq!(session.get_stats().unwrap().message_count, 2);
    }
}
