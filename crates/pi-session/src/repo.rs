//! SessionRepo / SessionState matching `vendor/pi/packages/agent/src/harness/session`.

use std::collections::{HashMap, HashSet};

use serde_json::Value;
use uuid::Uuid;

use crate::{now_ms, LaneRecord, SessionEntry, SessionError};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LanePointer {
    pub lane: String,
    pub leaf_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LogItem {
    Entry {
        seq: u64,
        entry: SessionEntry,
    },
    Record {
        seq: u64,
        record: LaneRecord,
    },
    Lane {
        seq: u64,
        lane: String,
        leaf_id: Option<String>,
    },
    FactName {
        seq: u64,
        name: Option<String>,
    },
    FactLabel {
        seq: u64,
        target_id: String,
        label: Option<String>,
    },
}

impl LogItem {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Entry { .. } => "entry",
            Self::Record { .. } => "record",
            Self::Lane { .. } => "lane",
            Self::FactName { .. } | Self::FactLabel { .. } => "fact",
        }
    }

    pub fn seq(&self) -> u64 {
        match self {
            Self::Entry { seq, .. }
            | Self::Record { seq, .. }
            | Self::Lane { seq, .. }
            | Self::FactName { seq, .. }
            | Self::FactLabel { seq, .. } => *seq,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct EntryQuery {
    pub entry_type: Option<String>,
    pub custom_type: Option<String>,
    pub order: EntryOrder,
    pub limit: Option<usize>,
    pub after_seq: Option<i64>,
}

#[derive(Debug, Clone, Default)]
pub struct RecordQuery {
    pub lane: Option<String>,
    pub record_type: Option<String>,
    pub run_id: Option<String>,
    pub operation_kind: Option<String>,
    pub after_seq: Option<i64>,
    pub order: EntryOrder,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum EntryOrder {
    #[default]
    NewestFirst,
    OldestFirst,
}

#[derive(Debug, Clone, Default)]
pub struct BranchBounds {
    pub start: Option<String>,
    pub stop_at_type: Option<String>,
    pub stop_at_id: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct LogOptions {
    pub after_seq: Option<i64>,
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct SessionStats {
    pub message_count: u64,
    pub cached_tokens: f64,
    pub uncached_tokens: f64,
    pub total_tokens: f64,
    pub cost_total: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionMetadata {
    pub id: String,
    pub created_at: u64,
    pub parent_session_id: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct SessionCreateOptions {
    pub id: Option<String>,
    pub parent_session_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ForkScope {
    #[default]
    Branch,
    Tree,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForkPosition {
    Before,
    At,
}

#[derive(Debug, Clone, Default)]
pub struct ForkOptions {
    pub scope: ForkScope,
    pub entry_id: Option<String>,
    pub position: Option<ForkPosition>,
    pub id: Option<String>,
    pub parent_session_id: Option<String>,
}

fn assert_valid_limit(limit: Option<usize>) -> Result<(), SessionError> {
    if matches!(limit, Some(0)) {
        return Err(SessionError::invalid_query(
            "limit must be a positive integer",
        ));
    }
    Ok(())
}

fn assert_valid_cursor(after_seq: Option<i64>) -> Result<(), SessionError> {
    if after_seq.is_some_and(|seq| seq < 0) {
        return Err(SessionError::invalid_query(
            "cursor sequence must be a non-negative integer",
        ));
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct SessionState {
    sequence: u64,
    used_ids: HashSet<String>,
    entries: Vec<SessionEntry>,
    entries_by_id: HashMap<String, SessionEntry>,
    records: Vec<LaneRecord>,
    open_operations: HashMap<String, HashMap<String, LaneRecord>>,
    lane_order: Vec<String>,
    lanes: HashMap<String, Option<String>>,
    log: Vec<LogItem>,
    stats: SessionStats,
    name: Option<String>,
    labels: HashMap<String, String>,
}

impl Default for SessionState {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionState {
    pub fn new() -> Self {
        let mut lanes = HashMap::new();
        lanes.insert("main".into(), None);
        Self {
            sequence: 0,
            used_ids: HashSet::new(),
            entries: Vec::new(),
            entries_by_id: HashMap::new(),
            records: Vec::new(),
            open_operations: HashMap::new(),
            lane_order: vec!["main".into()],
            lanes,
            log: Vec::new(),
            stats: SessionStats::default(),
            name: None,
            labels: HashMap::new(),
        }
    }

    pub fn next_sequence(&self) -> u64 {
        self.sequence + 1
    }

    pub fn get_lanes(&self) -> Vec<LanePointer> {
        self.lane_order
            .iter()
            .filter_map(|lane| {
                self.lanes.get(lane).map(|leaf| LanePointer {
                    lane: lane.clone(),
                    leaf_id: leaf.clone(),
                })
            })
            .collect()
    }

    pub fn require_lane(&self, lane: &str) -> Result<Option<String>, SessionError> {
        self.lanes
            .get(lane)
            .cloned()
            .ok_or_else(|| SessionError::invalid_lane(format!("Lane not found: {lane}")))
    }

    pub fn validate_new_lane(&self, lane: &str) -> Result<(), SessionError> {
        if self.lanes.contains_key(lane) {
            return Err(SessionError::already_exists(format!(
                "Lane already exists: {lane}"
            )));
        }
        Ok(())
    }

    pub fn validate_target(&self, target_id: Option<&str>) -> Result<(), SessionError> {
        if let Some(id) = target_id {
            if !self.entries_by_id.contains_key(id) {
                return Err(SessionError::not_found(format!("Entry not found: {id}")));
            }
        }
        Ok(())
    }

    pub fn validate_unused_id(&self, id: &str) -> Result<(), SessionError> {
        if self.used_ids.contains(id) {
            return Err(SessionError::already_exists(format!(
                "Session id already exists: {id}"
            )));
        }
        Ok(())
    }

    pub fn apply_entry(
        &mut self,
        lane: Option<&str>,
        mut entry: SessionEntry,
    ) -> Result<SessionEntry, SessionError> {
        if entry.seq == 0 {
            entry.seq = self.next_sequence();
        }
        if entry.timestamp == 0 {
            entry.timestamp = now_ms();
        }
        if let Some(lane) = lane {
            let leaf = self.require_lane(lane)?;
            if entry.parent_id.is_none() {
                entry.parent_id = leaf;
            }
        }
        self.validate_unused_id(&entry.id)?;
        self.apply_entry_mutation(lane, entry)
    }

    fn apply_entry_mutation(
        &mut self,
        lane: Option<&str>,
        entry: SessionEntry,
    ) -> Result<SessionEntry, SessionError> {
        if entry.seq != self.next_sequence() {
            return Err(SessionError::invalid_entry(format!(
                "Invalid session mutation: has non-consecutive seq {}",
                entry.seq
            )));
        }
        if self.used_ids.contains(&entry.id) {
            return Err(SessionError::invalid_entry(format!(
                "Invalid session mutation: contains duplicate id {}",
                entry.id
            )));
        }
        if let Some(lane) = lane {
            let leaf = self.require_lane(lane)?;
            if entry.parent_id != leaf {
                return Err(SessionError::invalid_entry(
                    "Invalid session mutation: does not chain to the lane leaf",
                ));
            }
        }
        if let Some(parent) = &entry.parent_id {
            if !self.entries_by_id.contains_key(parent) {
                return Err(SessionError::invalid_entry(format!(
                    "Invalid session mutation: references missing parent {parent}"
                )));
            }
        }
        self.sequence = entry.seq;
        self.used_ids.insert(entry.id.clone());
        if let Some(lane) = lane {
            self.lanes.insert(lane.to_string(), Some(entry.id.clone()));
        }
        self.log.push(LogItem::Entry {
            seq: entry.seq,
            entry: entry.clone(),
        });
        if entry.entry_type == "message" {
            self.stats.message_count += 1;
        }
        self.entries_by_id.insert(entry.id.clone(), entry.clone());
        self.entries.push(entry.clone());
        Ok(entry)
    }

    pub fn apply_record(&mut self, mut record: LaneRecord) -> Result<LaneRecord, SessionError> {
        if record.seq == 0 {
            record.seq = self.next_sequence();
        }
        if record.timestamp == 0 {
            record.timestamp = now_ms();
        }
        let lane = record
            .lane
            .clone()
            .ok_or_else(|| SessionError::invalid_lane("Lane not found: ".to_string()))?;
        self.require_lane(&lane)?;
        self.validate_unused_id(&record.id)?;
        if record.record_type == "operation_started" {
            if let Some(open) = self.find_open_operations(&lane, Some(1))?.first() {
                return Err(SessionError::storage(format!(
                    "Lane {lane} already has an open operation {}",
                    open.id
                )));
            }
        }
        if record.seq != self.next_sequence() {
            return Err(SessionError::invalid_entry(format!(
                "Invalid session mutation: has non-consecutive seq {}",
                record.seq
            )));
        }
        self.sequence = record.seq;
        self.used_ids.insert(record.id.clone());
        if record.record_type == "usage" {
            if let Some(usage) = record.extra.get("usage") {
                self.stats.cached_tokens += usage
                    .get("cacheRead")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0);
                self.stats.uncached_tokens +=
                    usage.get("input").and_then(Value::as_f64).unwrap_or(0.0)
                        + usage
                            .get("cacheWrite")
                            .and_then(Value::as_f64)
                            .unwrap_or(0.0);
                self.stats.total_tokens += usage
                    .get("totalTokens")
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0);
                self.stats.cost_total += usage
                    .get("cost")
                    .and_then(|cost| cost.get("total"))
                    .and_then(Value::as_f64)
                    .unwrap_or(0.0);
            }
        }
        if record.record_type == "operation_started" {
            self.open_operations
                .entry(lane.clone())
                .or_default()
                .insert(record.id.clone(), record.clone());
        } else if record.record_type == "operation_finished" {
            if let Some(run_id) = record
                .extra
                .get("runId")
                .and_then(Value::as_str)
                .map(str::to_string)
            {
                if let Some(open) = self.open_operations.get_mut(&lane) {
                    open.remove(&run_id);
                }
            }
        }
        self.log.push(LogItem::Record {
            seq: record.seq,
            record: record.clone(),
        });
        self.records.push(record.clone());
        Ok(record)
    }

    pub fn apply_lane(&mut self, lane: &str, leaf_id: Option<&str>) -> Result<(), SessionError> {
        self.validate_target(leaf_id)?;
        let seq = self.next_sequence();
        self.sequence = seq;
        if !self.lanes.contains_key(lane) {
            self.lane_order.push(lane.to_string());
        }
        self.lanes
            .insert(lane.to_string(), leaf_id.map(str::to_string));
        self.log.push(LogItem::Lane {
            seq,
            lane: lane.to_string(),
            leaf_id: leaf_id.map(str::to_string),
        });
        Ok(())
    }

    pub fn apply_name(&mut self, name: Option<&str>) {
        let seq = self.next_sequence();
        self.sequence = seq;
        self.name = name.map(str::to_string);
        self.log.push(LogItem::FactName {
            seq,
            name: name.map(str::to_string),
        });
    }

    pub fn apply_label(
        &mut self,
        target_id: &str,
        label: Option<&str>,
    ) -> Result<(), SessionError> {
        self.validate_target(Some(target_id))?;
        let seq = self.next_sequence();
        self.sequence = seq;
        if let Some(label) = label {
            self.labels.insert(target_id.to_string(), label.to_string());
        } else {
            self.labels.remove(target_id);
        }
        self.log.push(LogItem::FactLabel {
            seq,
            target_id: target_id.to_string(),
            label: label.map(str::to_string),
        });
        Ok(())
    }

    pub fn get_entry(&self, id: &str) -> Option<&SessionEntry> {
        self.entries_by_id.get(id)
    }

    pub fn find_entries(&self, query: &EntryQuery) -> Result<Vec<SessionEntry>, SessionError> {
        assert_valid_limit(query.limit)?;
        assert_valid_cursor(query.after_seq)?;
        let mut results = Vec::new();
        for entry in ordered(&self.entries, query.order) {
            if !self.matches_entry(entry, query) {
                continue;
            }
            results.push(entry.clone());
            if query.limit.is_some_and(|limit| results.len() == limit) {
                break;
            }
        }
        Ok(results)
    }

    pub fn find_entries_on_branch(
        &self,
        start: &str,
        query: &EntryQuery,
        bounds: &BranchBounds,
    ) -> Result<Vec<SessionEntry>, SessionError> {
        assert_valid_limit(query.limit)?;
        assert_valid_cursor(query.after_seq)?;
        let mut results = Vec::new();
        if query.order == EntryOrder::OldestFirst {
            let walked = self.walk_to_root(start, &BranchBounds::default())?;
            for entry in walked.iter().rev() {
                let reached = bounds.stop_at_id.as_deref() == Some(entry.id.as_str())
                    || bounds.stop_at_type.as_deref() == Some(entry.entry_type.as_str());
                if self.matches_entry(entry, query) {
                    results.push(entry.clone());
                }
                if reached || query.limit.is_some_and(|limit| results.len() == limit) {
                    break;
                }
            }
        } else {
            let walked = self.walk_to_root(start, bounds)?;
            for entry in &walked {
                if self.matches_entry(entry, query) {
                    results.push(entry.clone());
                }
                if query.limit.is_some_and(|limit| results.len() == limit) {
                    break;
                }
            }
        }
        Ok(results)
    }

    pub fn find_records(&self, query: &RecordQuery) -> Result<Vec<LaneRecord>, SessionError> {
        assert_valid_limit(query.limit)?;
        assert_valid_cursor(query.after_seq)?;
        if query.operation_kind.is_some()
            && query.record_type.as_deref() != Some("operation_started")
        {
            return Err(SessionError::invalid_query(
                "operationKind requires type \"operation_started\"",
            ));
        }
        let mut results = Vec::new();
        for record in ordered(&self.records, query.order) {
            if !self.matches_record(record, query) {
                continue;
            }
            results.push(record.clone());
            if query.limit.is_some_and(|limit| results.len() == limit) {
                break;
            }
        }
        Ok(results)
    }

    pub fn find_open_operations(
        &self,
        lane: &str,
        limit: Option<usize>,
    ) -> Result<Vec<LaneRecord>, SessionError> {
        assert_valid_limit(limit)?;
        let mut open = self
            .open_operations
            .get(lane)
            .map(|map| map.values().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        open.reverse();
        if let Some(limit) = limit {
            open.truncate(limit);
        }
        Ok(open)
    }

    pub fn get_log(&self, options: &LogOptions) -> Result<Vec<LogItem>, SessionError> {
        assert_valid_limit(options.limit)?;
        assert_valid_cursor(options.after_seq)?;
        let mut results = Vec::new();
        for item in &self.log {
            if options
                .after_seq
                .is_some_and(|after| item.seq() as i64 <= after)
            {
                continue;
            }
            results.push(item.clone());
            if options.limit.is_some_and(|limit| results.len() == limit) {
                break;
            }
        }
        Ok(results)
    }

    pub fn get_name(&self) -> Option<&str> {
        self.name.as_deref()
    }

    pub fn get_label(&self, id: &str) -> Option<&str> {
        self.labels.get(id).map(String::as_str)
    }

    pub fn get_stats(&self) -> &SessionStats {
        &self.stats
    }

    pub fn restore_lanes(&mut self, pointers: Vec<LanePointer>) {
        self.lanes.clear();
        self.lane_order.clear();
        if pointers.is_empty() {
            self.lanes.insert("main".into(), None);
            self.lane_order.push("main".into());
            return;
        }
        for pointer in pointers {
            self.lane_order.push(pointer.lane.clone());
            self.lanes.insert(pointer.lane, pointer.leaf_id);
        }
    }

    pub fn apply_log_item(&mut self, item: LogItem) -> Result<(), SessionError> {
        match item {
            LogItem::Entry { entry, .. } => {
                self.apply_entry_mutation(None, entry)?;
            }
            LogItem::Record { record, .. } => {
                self.apply_record(record)?;
            }
            LogItem::Lane { lane, leaf_id, .. } => {
                self.apply_lane(&lane, leaf_id.as_deref())?;
            }
            LogItem::FactName { name, .. } => {
                self.apply_name(name.as_deref());
            }
            LogItem::FactLabel {
                target_id, label, ..
            } => {
                self.apply_label(&target_id, label.as_deref())?;
            }
        }
        Ok(())
    }

    pub fn create_fork_mutations(
        &self,
        options: &ForkOptions,
    ) -> Result<Vec<LogItem>, SessionError> {
        let (copied_entries, fork_lanes) = if options.scope == ForkScope::Tree {
            (
                self.find_entries(&EntryQuery {
                    order: EntryOrder::OldestFirst,
                    ..EntryQuery::default()
                })?,
                self.get_lanes(),
            )
        } else {
            let selected_entry_id = options
                .entry_id
                .clone()
                .or_else(|| self.require_lane("main").ok().flatten());
            let mut target_id = None;
            if let Some(selected) = selected_entry_id {
                let entry = self
                    .get_entry(&selected)
                    .filter(|entry| entry.entry_type == "message");
                let Some(entry) = entry else {
                    return Err(SessionError::invalid_fork_target(format!(
                        "Fork target is not a message entry: {selected}"
                    )));
                };
                let position = options.position.unwrap_or(if options.entry_id.is_none() {
                    ForkPosition::At
                } else {
                    ForkPosition::Before
                });
                target_id = match position {
                    ForkPosition::At => Some(entry.id.clone()),
                    ForkPosition::Before => entry.parent_id.clone(),
                };
            }
            let copied = if let Some(target) = &target_id {
                self.find_entries_on_branch(
                    target,
                    &EntryQuery {
                        order: EntryOrder::OldestFirst,
                        ..EntryQuery::default()
                    },
                    &BranchBounds::default(),
                )?
            } else {
                Vec::new()
            };
            (
                copied,
                vec![LanePointer {
                    lane: "main".into(),
                    leaf_id: target_id,
                }],
            )
        };

        let mut mutations = Vec::new();
        let mut sequence = 1u64;
        for source in &copied_entries {
            let mut entry = source.clone();
            entry.seq = sequence;
            mutations.push(LogItem::Entry {
                seq: sequence,
                entry,
            });
            sequence += 1;
        }
        for pointer in fork_lanes {
            mutations.push(LogItem::Lane {
                seq: sequence,
                lane: pointer.lane,
                leaf_id: pointer.leaf_id,
            });
            sequence += 1;
        }
        if let Some(name) = &self.name {
            mutations.push(LogItem::FactName {
                seq: sequence,
                name: Some(name.clone()),
            });
            sequence += 1;
        }
        for entry in &copied_entries {
            if let Some(label) = self.labels.get(&entry.id) {
                mutations.push(LogItem::FactLabel {
                    seq: sequence,
                    target_id: entry.id.clone(),
                    label: Some(label.clone()),
                });
                sequence += 1;
            }
        }
        Ok(mutations)
    }

    fn walk_to_root(
        &self,
        start: &str,
        bounds: &BranchBounds,
    ) -> Result<Vec<SessionEntry>, SessionError> {
        let mut current = self
            .entries_by_id
            .get(start)
            .cloned()
            .ok_or_else(|| SessionError::not_found(format!("Entry not found: {start}")))?;
        let mut visited = HashSet::new();
        let mut out = Vec::new();
        loop {
            if !visited.insert(current.id.clone()) {
                return Err(SessionError::invalid_entry(format!(
                    "Session branch contains a cycle at {}",
                    current.id
                )));
            }
            let reached = bounds.stop_at_id.as_deref() == Some(current.id.as_str())
                || bounds.stop_at_type.as_deref() == Some(current.entry_type.as_str())
                || current.parent_id.is_none();
            out.push(current.clone());
            if reached {
                break;
            }
            let parent = current.parent_id.clone().unwrap();
            current =
                self.entries_by_id.get(&parent).cloned().ok_or_else(|| {
                    SessionError::invalid_entry(format!("Entry not found: {parent}"))
                })?;
        }
        Ok(out)
    }

    fn matches_entry(&self, entry: &SessionEntry, query: &EntryQuery) -> bool {
        if query
            .entry_type
            .as_deref()
            .is_some_and(|ty| ty != entry.entry_type)
        {
            return false;
        }
        if let Some(custom) = &query.custom_type {
            if entry.entry_type != "custom" || entry.custom_type.as_deref() != Some(custom.as_str())
            {
                return false;
            }
        }
        if let Some(after) = query.after_seq {
            return if query.order == EntryOrder::OldestFirst {
                (entry.seq as i64) > after
            } else {
                (entry.seq as i64) < after
            };
        }
        true
    }

    fn matches_record(&self, record: &LaneRecord, query: &RecordQuery) -> bool {
        if query
            .lane
            .as_deref()
            .is_some_and(|lane| record.lane.as_deref() != Some(lane))
        {
            return false;
        }
        if query
            .record_type
            .as_deref()
            .is_some_and(|ty| ty != record.record_type)
        {
            return false;
        }
        if let Some(run_id) = &query.run_id {
            let matches = if record.record_type == "operation_started" {
                record.id == *run_id
            } else {
                record.extra.get("runId").and_then(Value::as_str) == Some(run_id.as_str())
            };
            if !matches {
                return false;
            }
        }
        if let Some(kind) = &query.operation_kind {
            let actual = record
                .extra
                .get("intent")
                .and_then(|value| value.get("kind"))
                .and_then(Value::as_str);
            if record.record_type != "operation_started" || actual != Some(kind.as_str()) {
                return false;
            }
        }
        if query
            .after_seq
            .is_some_and(|after| record.seq as i64 <= after)
        {
            return false;
        }
        true
    }
}

fn ordered<T>(items: &[T], order: EntryOrder) -> Box<dyn Iterator<Item = &T> + '_> {
    match order {
        EntryOrder::OldestFirst => Box::new(items.iter()),
        EntryOrder::NewestFirst => Box::new(items.iter().rev()),
    }
}

#[derive(Debug, Clone)]
pub struct Session {
    pub id: String,
    pub created_at: u64,
    pub parent_session_id: Option<String>,
    state: SessionState,
}

impl Session {
    pub fn new(id: impl Into<String>) -> Self {
        Self::with_metadata(id, now_ms(), None)
    }

    pub fn with_metadata(
        id: impl Into<String>,
        created_at: u64,
        parent_session_id: Option<String>,
    ) -> Self {
        Self {
            id: id.into(),
            created_at,
            parent_session_id,
            state: SessionState::new(),
        }
    }

    pub fn metadata(&self) -> SessionMetadata {
        SessionMetadata {
            id: self.id.clone(),
            created_at: self.created_at,
            parent_session_id: self.parent_session_id.clone(),
        }
    }

    pub fn state(&self) -> &SessionState {
        &self.state
    }

    pub fn state_mut(&mut self) -> &mut SessionState {
        &mut self.state
    }

    pub fn apply_log_item(&mut self, item: LogItem) -> Result<(), SessionError> {
        self.state.apply_log_item(item)
    }

    pub fn apply_fork_mutations(&mut self, mutations: Vec<LogItem>) -> Result<(), SessionError> {
        for item in mutations {
            self.state.apply_log_item(item)?;
        }
        Ok(())
    }

    pub fn get_entry(&self, id: &str) -> Option<&SessionEntry> {
        self.state.get_entry(id)
    }

    pub fn get_name(&self) -> Option<&str> {
        self.state.get_name()
    }

    pub fn get_label(&self, id: &str) -> Option<&str> {
        self.state.get_label(id)
    }

    pub fn get_stats(&self) -> &SessionStats {
        self.state.get_stats()
    }

    pub fn get_leaf_id(&self) -> Option<String> {
        self.state.require_lane("main").ok().flatten()
    }

    pub fn restore_lanes(&mut self, pointers: Vec<LanePointer>) {
        self.state.restore_lanes(pointers);
    }

    pub fn append_entry(
        &mut self,
        entry: SessionEntry,
        lane: &str,
    ) -> Result<SessionEntry, SessionError> {
        self.state.apply_entry(Some(lane), entry)
    }

    pub fn append_record(&mut self, record: LaneRecord) -> Result<LaneRecord, SessionError> {
        self.state.apply_record(record)
    }

    pub fn create_lane(&mut self, lane: &str, at: Option<&str>) -> Result<(), SessionError> {
        self.state.validate_new_lane(lane)?;
        self.state.validate_target(at)?;
        self.state.apply_lane(lane, at)
    }

    pub fn move_lane(&mut self, lane: &str, to: Option<&str>) -> Result<(), SessionError> {
        self.state.require_lane(lane)?;
        self.state.validate_target(to)?;
        self.state.apply_lane(lane, to)
    }

    pub fn set_name(&mut self, name: Option<&str>) {
        self.state.apply_name(name);
    }

    pub fn set_label(&mut self, id: &str, label: Option<&str>) -> Result<(), SessionError> {
        self.state.apply_label(id, label)
    }

    pub fn get_lanes(&self) -> Vec<LanePointer> {
        self.state.get_lanes()
    }

    pub fn get_log(&self, options: &LogOptions) -> Result<Vec<LogItem>, SessionError> {
        self.state.get_log(options)
    }

    pub fn find_entries(&self, query: &EntryQuery) -> Result<Vec<SessionEntry>, SessionError> {
        self.state.find_entries(query)
    }

    pub fn find_entry(&self, query: &EntryQuery) -> Result<Option<SessionEntry>, SessionError> {
        let mut query = query.clone();
        if query.limit.is_none() {
            query.limit = Some(1);
        }
        Ok(self.state.find_entries(&query)?.into_iter().next())
    }

    pub fn find_entries_on_branch(
        &self,
        query: &EntryQuery,
        bounds: &BranchBounds,
    ) -> Result<Vec<SessionEntry>, SessionError> {
        // TS queryBranchEntries: validate first, then empty lane leaf → [].
        assert_valid_limit(query.limit)?;
        assert_valid_cursor(query.after_seq)?;
        let start = bounds
            .start
            .clone()
            .or_else(|| self.state.require_lane("main").ok().flatten());
        let Some(start) = start else {
            return Ok(Vec::new());
        };
        self.state.find_entries_on_branch(&start, query, bounds)
    }

    pub fn find_entry_on_branch(
        &self,
        query: &EntryQuery,
        bounds: &BranchBounds,
    ) -> Result<Option<SessionEntry>, SessionError> {
        let mut query = query.clone();
        if query.limit.is_none() {
            query.limit = Some(1);
        }
        Ok(self
            .find_entries_on_branch(&query, bounds)?
            .into_iter()
            .next())
    }

    pub fn find_records(&self, query: &RecordQuery) -> Result<Vec<LaneRecord>, SessionError> {
        self.state.find_records(query)
    }

    pub fn find_open_operations(
        &self,
        lane: &str,
        limit: Option<usize>,
    ) -> Result<Vec<LaneRecord>, SessionError> {
        self.state.find_open_operations(lane, limit)
    }

    pub fn view(&self, lane: &str) -> SessionView<'_> {
        SessionView {
            session: self,
            lane: lane.to_string(),
        }
    }
}

pub struct SessionView<'a> {
    session: &'a Session,
    lane: String,
}

impl SessionView<'_> {
    pub fn find_entries_on_branch(
        &self,
        query: &EntryQuery,
        bounds: &BranchBounds,
    ) -> Result<Vec<SessionEntry>, SessionError> {
        let mut bounds = bounds.clone();
        if bounds.start.is_none() {
            bounds.start = self.session.state.require_lane(&self.lane)?;
        }
        self.session.find_entries_on_branch(query, &bounds)
    }

    pub fn find_entry_on_branch(
        &self,
        query: &EntryQuery,
        bounds: &BranchBounds,
    ) -> Result<Option<SessionEntry>, SessionError> {
        let mut query = query.clone();
        if query.limit.is_none() {
            query.limit = Some(1);
        }
        Ok(self
            .find_entries_on_branch(&query, bounds)?
            .into_iter()
            .next())
    }
}

#[derive(Debug, Default)]
pub struct InMemorySessionRepo {
    sessions: HashMap<String, Session>,
}

impl InMemorySessionRepo {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create(&mut self, id: Option<&str>) -> Result<&mut Session, SessionError> {
        self.create_with(SessionCreateOptions {
            id: id.map(str::to_string),
            parent_session_id: None,
        })
    }

    pub fn create_with(
        &mut self,
        options: SessionCreateOptions,
    ) -> Result<&mut Session, SessionError> {
        let id = options.id.unwrap_or_else(|| Uuid::new_v4().to_string());
        if self.sessions.contains_key(&id) {
            return Err(SessionError::already_exists(format!(
                "Session already exists: {id}"
            )));
        }
        let mut session = Session::new(id.clone());
        session.parent_session_id = options.parent_session_id;
        self.sessions.insert(id.clone(), session);
        Ok(self.sessions.get_mut(&id).expect("just inserted"))
    }

    pub fn open(&mut self, id: &str) -> Result<&mut Session, SessionError> {
        self.sessions
            .get_mut(id)
            .ok_or_else(|| SessionError::not_found(format!("Session not found: {id}")))
    }

    pub fn list(&self) -> Vec<SessionMetadata> {
        self.sessions.values().map(Session::metadata).collect()
    }

    pub fn delete(&mut self, id: &str) {
        self.sessions.remove(id);
    }

    pub fn fork(
        &mut self,
        source_id: &str,
        options: &ForkOptions,
    ) -> Result<&mut Session, SessionError> {
        let id = options
            .id
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        if self.sessions.contains_key(&id) {
            return Err(SessionError::already_exists(format!(
                "Session already exists: {id}"
            )));
        }
        let mutations = self
            .open(source_id)?
            .state()
            .create_fork_mutations(options)?;
        let parent = options
            .parent_session_id
            .clone()
            .unwrap_or_else(|| source_id.to_string());
        let mut session = Session::with_metadata(id.clone(), now_ms(), Some(parent));
        session.apply_fork_mutations(mutations)?;
        self.sessions.insert(id.clone(), session);
        Ok(self.sessions.get_mut(&id).expect("just inserted"))
    }
}

pub fn operation_started(id: &str, lane: &str, kind: &str) -> LaneRecord {
    let intent = match kind {
        "compaction" => {
            serde_json::json!({ "kind": kind, "resultEntryId": format!("{id}-result") })
        }
        "navigation" => serde_json::json!({ "kind": kind, "targetId": null, "summarize": false }),
        _ => serde_json::json!({ "kind": "run", "originalPrompt": [], "initialMessages": [] }),
    };
    let mut extra = serde_json::Map::new();
    extra.insert("sourceLeafId".into(), Value::Null);
    extra.insert("intent".into(), intent);
    LaneRecord {
        id: id.to_string(),
        record_type: "operation_started".into(),
        seq: 0,
        timestamp: 0,
        lane: Some(lane.to_string()),
        extra,
    }
}

pub fn user_message_entry(id: &str, text: &str) -> SessionEntry {
    SessionEntry {
        id: id.to_string(),
        entry_type: "message".into(),
        parent_id: None,
        seq: 0,
        timestamp: 0,
        message: Some(serde_json::json!({
            "role": "user",
            "content": [{"type": "text", "text": text}],
            "timestamp": 1
        })),
        custom_type: None,
        extra: serde_json::Map::new(),
    }
}

pub fn assistant_message_entry(id: &str, text: &str) -> SessionEntry {
    SessionEntry {
        id: id.to_string(),
        entry_type: "message".into(),
        parent_id: None,
        seq: 0,
        timestamp: 0,
        message: Some(serde_json::json!({
            "role": "assistant",
            "content": [{"type": "text", "text": text}],
            "timestamp": 1
        })),
        custom_type: None,
        extra: serde_json::Map::new(),
    }
}

pub fn custom_entry(id: &str, custom_type: &str, data: Value) -> SessionEntry {
    let mut extra = serde_json::Map::new();
    extra.insert("data".into(), data);
    SessionEntry {
        id: id.to_string(),
        entry_type: "custom".into(),
        parent_id: None,
        seq: 0,
        timestamp: 0,
        message: None,
        custom_type: Some(custom_type.to_string()),
        extra,
    }
}

pub fn compaction_entry(id: &str, summary: &str, tokens_before: u64) -> SessionEntry {
    let mut extra = serde_json::Map::new();
    extra.insert("summary".into(), Value::String(summary.to_string()));
    extra.insert("retainedTail".into(), serde_json::json!([]));
    extra.insert("tokensBefore".into(), Value::from(tokens_before));
    SessionEntry {
        id: id.to_string(),
        entry_type: "compaction".into(),
        parent_id: None,
        seq: 0,
        timestamp: 0,
        message: None,
        custom_type: None,
        extra,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(entries: &[SessionEntry]) -> Vec<&str> {
        entries.iter().map(|entry| entry.id.as_str()).collect()
    }

    #[test]
    fn assigns_parents_and_one_sequence_across_every_mutation() {
        let mut repo = InMemorySessionRepo::new();
        let session = repo.create(Some("session")).unwrap();
        let root = session
            .append_entry(user_message_entry("root", "root"), "main")
            .unwrap();
        session.create_lane("thread", Some(&root.id)).unwrap();
        let child = session
            .append_entry(
                custom_entry("child", "note", serde_json::json!({ "value": 1 })),
                "thread",
            )
            .unwrap();
        let record = session
            .append_record(operation_started("run", "thread", "run"))
            .unwrap();
        session.set_name(Some("Example"));
        session.set_label(&root.id, Some("checkpoint")).unwrap();
        session.move_lane("main", Some(&child.id)).unwrap();

        assert_eq!(root.parent_id, None);
        assert_eq!(root.seq, 1);
        assert_eq!(child.parent_id.as_deref(), Some("root"));
        assert_eq!(child.seq, 3);
        assert_eq!(record.seq, 4);
        assert!(root.timestamp < u64::MAX);
        let log = session.get_log(&LogOptions::default()).unwrap();
        let kinds: Vec<_> = log.iter().map(|item| (item.kind(), item.seq())).collect();
        assert_eq!(
            kinds,
            [
                ("entry", 1),
                ("lane", 2),
                ("entry", 3),
                ("record", 4),
                ("fact", 5),
                ("fact", 6),
                ("lane", 7),
            ]
        );
        assert_eq!(
            session.get_lanes(),
            [
                LanePointer {
                    lane: "main".into(),
                    leaf_id: Some("child".into())
                },
                LanePointer {
                    lane: "thread".into(),
                    leaf_id: Some("child".into())
                },
            ]
        );
    }

    #[test]
    fn commits_records_and_lane_moves_as_separate_mutations() {
        let mut repo = InMemorySessionRepo::new();
        let session = repo.create(Some("session")).unwrap();
        session
            .append_entry(user_message_entry("root", "root"), "main")
            .unwrap();
        let mut extra = serde_json::Map::new();
        extra.insert("runId".into(), Value::String("run".into()));
        extra.insert("outcome".into(), Value::String("completed".into()));
        let finished = session
            .append_record(LaneRecord {
                id: "finish".into(),
                record_type: "operation_finished".into(),
                seq: 0,
                timestamp: 0,
                lane: Some("main".into()),
                extra,
            })
            .unwrap();
        assert_eq!(finished.seq, 2);
        assert_eq!(
            session.get_lanes(),
            [LanePointer {
                lane: "main".into(),
                leaf_id: Some("root".into())
            }]
        );
        session.move_lane("main", None).unwrap();
        assert_eq!(
            session.get_lanes(),
            [LanePointer {
                lane: "main".into(),
                leaf_id: None
            }]
        );
        let log = session.get_log(&LogOptions::default()).unwrap();
        assert_eq!(log[0].kind(), "entry");
        assert_eq!(log[1].kind(), "record");
        assert_eq!(log[2].kind(), "lane");
        assert_eq!(
            session.move_lane("main", Some("missing")).unwrap_err().code,
            "not_found"
        );
        assert_eq!(
            session.find_records(&RecordQuery::default()).unwrap().len(),
            1
        );
    }

    #[test]
    fn rejects_duplicate_ids_without_changing_state() {
        let mut repo = InMemorySessionRepo::new();
        let session = repo.create(Some("session")).unwrap();
        session
            .append_entry(user_message_entry("shared", "root"), "main")
            .unwrap();
        assert_eq!(
            session
                .append_record(operation_started("shared", "main", "run"))
                .unwrap_err()
                .code,
            "already_exists"
        );
        session
            .append_record(operation_started("run", "main", "run"))
            .unwrap();
        assert_eq!(
            session
                .append_entry(custom_entry("run", "note", Value::Null), "main")
                .unwrap_err()
                .code,
            "already_exists"
        );
        let seqs: Vec<_> = session
            .get_log(&LogOptions::default())
            .unwrap()
            .iter()
            .map(LogItem::seq)
            .collect();
        assert_eq!(seqs, [1, 2]);
    }

    #[test]
    fn isolates_lanes_while_sharing_the_tree() {
        let mut repo = InMemorySessionRepo::new();
        let session = repo.create(Some("session")).unwrap();
        session
            .append_entry(user_message_entry("root", "root"), "main")
            .unwrap();
        session.create_lane("thread", Some("root")).unwrap();
        session
            .append_entry(user_message_entry("main-child", "main"), "main")
            .unwrap();
        session
            .append_entry(user_message_entry("thread-child", "thread"), "thread")
            .unwrap();
        assert_eq!(
            session.get_lanes(),
            [
                LanePointer {
                    lane: "main".into(),
                    leaf_id: Some("main-child".into())
                },
                LanePointer {
                    lane: "thread".into(),
                    leaf_id: Some("thread-child".into())
                },
            ]
        );
        assert_eq!(
            ids(&session
                .find_entries_on_branch(
                    &EntryQuery {
                        order: EntryOrder::OldestFirst,
                        ..EntryQuery::default()
                    },
                    &BranchBounds {
                        start: Some("main-child".into()),
                        ..BranchBounds::default()
                    }
                )
                .unwrap()),
            ["root", "main-child"]
        );
        assert_eq!(
            ids(&session
                .find_entries_on_branch(
                    &EntryQuery {
                        order: EntryOrder::OldestFirst,
                        ..EntryQuery::default()
                    },
                    &BranchBounds {
                        start: Some("thread-child".into()),
                        ..BranchBounds::default()
                    }
                )
                .unwrap()),
            ["root", "thread-child"]
        );
    }

    #[test]
    fn rejects_invalid_queries_before_empty_reads() {
        let mut repo = InMemorySessionRepo::new();
        let session = repo.create(Some("invalid-queries")).unwrap();
        session.create_lane("thread", None).unwrap();
        let thread = session.view("thread");
        assert_eq!(
            session
                .find_entries(&EntryQuery {
                    limit: Some(0),
                    ..EntryQuery::default()
                })
                .unwrap_err()
                .code,
            "invalid_query"
        );
        assert_eq!(
            session
                .find_entry(&EntryQuery {
                    limit: Some(0),
                    ..EntryQuery::default()
                })
                .unwrap_err()
                .code,
            "invalid_query"
        );
        assert_eq!(
            session
                .find_entries_on_branch(
                    &EntryQuery {
                        limit: Some(0),
                        ..EntryQuery::default()
                    },
                    &BranchBounds::default()
                )
                .unwrap_err()
                .code,
            "invalid_query"
        );
        assert_eq!(
            thread
                .find_entries_on_branch(
                    &EntryQuery {
                        after_seq: Some(-1),
                        ..EntryQuery::default()
                    },
                    &BranchBounds::default()
                )
                .unwrap_err()
                .code,
            "invalid_query"
        );
        assert_eq!(
            thread
                .find_entry_on_branch(
                    &EntryQuery {
                        limit: Some(0),
                        ..EntryQuery::default()
                    },
                    &BranchBounds::default()
                )
                .unwrap_err()
                .code,
            "invalid_query"
        );
        assert_eq!(
            session
                .find_records(&RecordQuery {
                    limit: Some(0),
                    ..RecordQuery::default()
                })
                .unwrap_err()
                .code,
            "invalid_query"
        );
        assert_eq!(
            session
                .find_records(&RecordQuery {
                    operation_kind: Some("run".into()),
                    ..RecordQuery::default()
                })
                .unwrap_err()
                .code,
            "invalid_query"
        );
        assert_eq!(
            session
                .find_records(&RecordQuery {
                    record_type: Some("step_attempt".into()),
                    operation_kind: Some("run".into()),
                    ..RecordQuery::default()
                })
                .unwrap_err()
                .code,
            "invalid_query"
        );
        assert_eq!(
            session
                .find_open_operations("main", Some(0))
                .unwrap_err()
                .code,
            "invalid_query"
        );
        assert_eq!(
            session
                .get_log(&LogOptions {
                    after_seq: Some(-1),
                    limit: None
                })
                .unwrap_err()
                .code,
            "invalid_query"
        );
        assert_eq!(
            repo.create(Some("invalid-queries")).unwrap_err().code,
            "already_exists"
        );
    }

    #[test]
    fn supports_bounded_filtered_and_cursor_based_queries() {
        let mut repo = InMemorySessionRepo::new();
        let session = repo.create(Some("session")).unwrap();
        session
            .append_entry(user_message_entry("root", "root"), "main")
            .unwrap();
        session
            .append_entry(custom_entry("old-note", "note", Value::from(1)), "main")
            .unwrap();
        session
            .append_entry(compaction_entry("compact", "summary", 10), "main")
            .unwrap();
        session
            .append_entry(custom_entry("new-note", "note", Value::from(2)), "main")
            .unwrap();
        session
            .append_entry(
                SessionEntry {
                    id: "tail".into(),
                    entry_type: "message".into(),
                    parent_id: None,
                    seq: 0,
                    timestamp: 0,
                    message: Some(serde_json::json!({
                        "role": "assistant",
                        "content": [{"type":"text","text":"tail"}]
                    })),
                    custom_type: None,
                    extra: serde_json::Map::new(),
                },
                "main",
            )
            .unwrap();
        assert_eq!(
            ids(&session.find_entries(&EntryQuery::default()).unwrap()),
            ["tail", "new-note", "compact", "old-note", "root"]
        );
        assert_eq!(
            ids(&session
                .find_entries(&EntryQuery {
                    order: EntryOrder::OldestFirst,
                    after_seq: Some(2),
                    limit: Some(2),
                    ..EntryQuery::default()
                })
                .unwrap()),
            ["compact", "new-note"]
        );
        assert_eq!(
            ids(&session
                .find_entries(&EntryQuery {
                    custom_type: Some("note".into()),
                    ..EntryQuery::default()
                })
                .unwrap()),
            ["new-note", "old-note"]
        );
        assert_eq!(
            ids(&session
                .find_entries_on_branch(
                    &EntryQuery {
                        custom_type: Some("note".into()),
                        limit: Some(1),
                        ..EntryQuery::default()
                    },
                    &BranchBounds {
                        start: Some("tail".into()),
                        ..BranchBounds::default()
                    }
                )
                .unwrap()),
            ["new-note"]
        );
        assert_eq!(
            ids(&session
                .find_entries_on_branch(
                    &EntryQuery {
                        entry_type: Some("message".into()),
                        ..EntryQuery::default()
                    },
                    &BranchBounds {
                        start: Some("tail".into()),
                        stop_at_type: Some("compaction".into()),
                        ..BranchBounds::default()
                    }
                )
                .unwrap()),
            ["tail"]
        );
        assert_eq!(
            ids(&session
                .find_entries_on_branch(
                    &EntryQuery {
                        order: EntryOrder::OldestFirst,
                        ..EntryQuery::default()
                    },
                    &BranchBounds {
                        start: Some("tail".into()),
                        stop_at_type: Some("custom".into()),
                        ..BranchBounds::default()
                    }
                )
                .unwrap()),
            ["root", "old-note"]
        );
        assert_eq!(
            session
                .find_entries_on_branch(
                    &EntryQuery::default(),
                    &BranchBounds {
                        start: Some("missing".into()),
                        ..BranchBounds::default()
                    }
                )
                .unwrap_err()
                .code,
            "not_found"
        );
        assert_eq!(
            ids(&session
                .find_entries_on_branch(
                    &EntryQuery {
                        entry_type: Some("custom".into()),
                        ..EntryQuery::default()
                    },
                    &BranchBounds {
                        start: Some("tail".into()),
                        stop_at_id: Some("tail".into()),
                        ..BranchBounds::default()
                    }
                )
                .unwrap()),
            [] as [&str; 0]
        );
    }

    #[test]
    fn keeps_lane_names_permanent_with_recovery_records() {
        let mut repo = InMemorySessionRepo::new();
        let session = repo.create(Some("session")).unwrap();
        session.create_lane("thread", None).unwrap();
        session
            .append_record(operation_started("old-run", "thread", "run"))
            .unwrap();
        let mut extra = serde_json::Map::new();
        extra.insert("queue".into(), Value::String("nextRun".into()));
        extra.insert(
            "target".into(),
            serde_json::json!({ "type": "message", "id": "queued-message" }),
        );
        session
            .append_record(LaneRecord {
                id: "old-next-run".into(),
                record_type: "queue_enqueued".into(),
                seq: 0,
                timestamp: 0,
                lane: Some("thread".into()),
                extra,
            })
            .unwrap();
        let ids: Vec<_> = session
            .find_records(&RecordQuery {
                lane: Some("thread".into()),
                ..RecordQuery::default()
            })
            .unwrap()
            .into_iter()
            .map(|record| record.id)
            .collect();
        assert_eq!(ids, ["old-next-run", "old-run"]);
        assert_eq!(
            session.create_lane("thread", None).unwrap_err().code,
            "already_exists"
        );
    }

    #[test]
    fn filters_records_by_lane_type_run_and_operation_kind() {
        let mut repo = InMemorySessionRepo::new();
        let session = repo.create(Some("session")).unwrap();
        session
            .append_record(operation_started("run-1", "main", "run"))
            .unwrap();
        let mut extra = serde_json::Map::new();
        extra.insert("runId".into(), Value::String("run-1".into()));
        extra.insert("step".into(), Value::String("assistant".into()));
        session
            .append_record(LaneRecord {
                id: "attempt-1".into(),
                record_type: "step_attempt".into(),
                seq: 0,
                timestamp: 0,
                lane: Some("main".into()),
                extra,
            })
            .unwrap();
        session.create_lane("thread", None).unwrap();
        session
            .append_record(operation_started("run-2", "thread", "run"))
            .unwrap();
        let mut extra = serde_json::Map::new();
        extra.insert("runId".into(), Value::String("run-2".into()));
        session
            .append_record(LaneRecord {
                id: "attempt-2".into(),
                record_type: "step_attempt".into(),
                seq: 0,
                timestamp: 0,
                lane: Some("thread".into()),
                extra,
            })
            .unwrap();
        let thread: Vec<_> = session
            .find_records(&RecordQuery {
                lane: Some("thread".into()),
                ..RecordQuery::default()
            })
            .unwrap()
            .into_iter()
            .map(|record| record.id)
            .collect();
        assert_eq!(thread, ["attempt-2", "run-2"]);
        let attempts: Vec<_> = session
            .find_records(&RecordQuery {
                record_type: Some("step_attempt".into()),
                order: EntryOrder::OldestFirst,
                ..RecordQuery::default()
            })
            .unwrap()
            .into_iter()
            .map(|record| record.id)
            .collect();
        assert_eq!(attempts, ["attempt-1", "attempt-2"]);
        let run: Vec<_> = session
            .find_records(&RecordQuery {
                run_id: Some("run-1".into()),
                after_seq: Some(1),
                ..RecordQuery::default()
            })
            .unwrap()
            .into_iter()
            .map(|record| record.id)
            .collect();
        assert_eq!(run, ["attempt-1"]);
    }

    #[test]
    fn creates_lists_opens_and_deletes_sessions() {
        let mut repo = InMemorySessionRepo::new();
        let session = repo.create(Some("one")).unwrap();
        session
            .append_entry(user_message_entry("persisted", "persisted"), "main")
            .unwrap();
        let metadata = session.metadata();
        let listed = repo.list();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, metadata.id);
        assert_eq!(listed[0].created_at, metadata.created_at);
        assert_eq!(listed[0].parent_session_id, metadata.parent_session_id);
        assert_eq!(
            repo.open("one").unwrap().get_entry("persisted").unwrap().id,
            "persisted"
        );
        assert_eq!(repo.create(Some("one")).unwrap_err().code, "already_exists");
        repo.delete("one");
        assert_eq!(repo.open("one").unwrap_err().code, "not_found");
        repo.delete("one");
    }

    #[test]
    fn forks_one_branch_with_selected_facts_and_no_records() {
        let mut repo = InMemorySessionRepo::new();
        let source = repo.create(Some("source")).unwrap();
        source
            .append_entry(user_message_entry("root", "root"), "main")
            .unwrap();
        source
            .append_entry(assistant_message_entry("shared", "shared"), "main")
            .unwrap();
        source.create_lane("thread", Some("shared")).unwrap();
        source
            .append_entry(user_message_entry("thread-child", "thread"), "thread")
            .unwrap();
        source
            .append_entry(user_message_entry("main-child", "main"), "main")
            .unwrap();
        source.set_name(Some("Source"));
        source.set_label("shared", Some("copied")).unwrap();
        source.set_label("thread-child", Some("excluded")).unwrap();
        source
            .append_record(operation_started("run", "main", "run"))
            .unwrap();
        let mut extra = serde_json::Map::new();
        extra.insert("cause".into(), Value::String("adjustment".into()));
        extra.insert(
            "usage".into(),
            serde_json::json!({
                "input": 10, "output": 5, "cacheRead": 3, "cacheWrite": 2,
                "totalTokens": 20,
                "cost": { "input": 1, "output": 2, "cacheRead": 3, "cacheWrite": 4, "total": 10 }
            }),
        );
        source
            .append_record(LaneRecord {
                id: "source-usage".into(),
                record_type: "usage".into(),
                seq: 0,
                timestamp: 0,
                lane: Some("main".into()),
                extra,
            })
            .unwrap();

        let fork = repo
            .fork(
                "source",
                &ForkOptions {
                    scope: ForkScope::Branch,
                    entry_id: Some("main-child".into()),
                    position: Some(ForkPosition::At),
                    id: Some("branch-fork".into()),
                    parent_session_id: None,
                },
            )
            .unwrap();
        assert_eq!(
            ids(&fork
                .find_entries(&EntryQuery {
                    order: EntryOrder::OldestFirst,
                    ..EntryQuery::default()
                })
                .unwrap()),
            ["root", "shared", "main-child"]
        );
        assert_eq!(
            fork.get_lanes(),
            [LanePointer {
                lane: "main".into(),
                leaf_id: Some("main-child".into())
            }]
        );
        assert_eq!(fork.get_name(), Some("Source"));
        assert_eq!(fork.get_label("shared"), Some("copied"));
        assert_eq!(fork.get_label("thread-child"), None);
        assert!(fork
            .find_records(&RecordQuery::default())
            .unwrap()
            .is_empty());
        assert_eq!(
            fork.get_stats(),
            &SessionStats {
                message_count: 3,
                ..SessionStats::default()
            }
        );
        fork.append_entry(user_message_entry("after", "after fork"), "main")
            .unwrap();
        assert_eq!(fork.get_stats().message_count, 4);
        assert_eq!(fork.metadata().id, "branch-fork");
        assert_eq!(fork.metadata().parent_session_id.as_deref(), Some("source"));
    }

    #[test]
    fn forks_a_complete_tree_with_lanes_and_facts() {
        let mut repo = InMemorySessionRepo::new();
        let source = repo.create(Some("source")).unwrap();
        source
            .append_entry(user_message_entry("root", "root"), "main")
            .unwrap();
        source.create_lane("thread", Some("root")).unwrap();
        source
            .append_entry(user_message_entry("main-child", "main"), "main")
            .unwrap();
        source
            .append_entry(user_message_entry("thread-child", "thread"), "thread")
            .unwrap();
        source
            .set_label("thread-child", Some("thread-tip"))
            .unwrap();

        let fork = repo
            .fork(
                "source",
                &ForkOptions {
                    scope: ForkScope::Tree,
                    id: Some("tree-fork".into()),
                    ..ForkOptions::default()
                },
            )
            .unwrap();
        assert_eq!(
            ids(&fork
                .find_entries(&EntryQuery {
                    order: EntryOrder::OldestFirst,
                    ..EntryQuery::default()
                })
                .unwrap()),
            ["root", "main-child", "thread-child"]
        );
        assert_eq!(
            fork.get_lanes(),
            [
                LanePointer {
                    lane: "main".into(),
                    leaf_id: Some("main-child".into())
                },
                LanePointer {
                    lane: "thread".into(),
                    leaf_id: Some("thread-child".into())
                },
            ]
        );
        assert_eq!(fork.get_label("thread-child"), Some("thread-tip"));
        assert_eq!(fork.get_stats().message_count, 3);
        let lanes: Vec<_> = fork
            .get_log(&LogOptions::default())
            .unwrap()
            .into_iter()
            .filter(|item| item.kind() == "lane")
            .collect();
        assert_eq!(
            lanes,
            [
                LogItem::Lane {
                    seq: 4,
                    lane: "main".into(),
                    leaf_id: Some("main-child".into())
                },
                LogItem::Lane {
                    seq: 5,
                    lane: "thread".into(),
                    leaf_id: Some("thread-child".into())
                },
            ]
        );
    }

    #[test]
    fn forks_before_an_entry_without_modifying_the_source() {
        let mut repo = InMemorySessionRepo::new();
        let source = repo.create(Some("source")).unwrap();
        source
            .append_entry(user_message_entry("root", "root"), "main")
            .unwrap();
        source
            .append_entry(user_message_entry("tail", "tail"), "main")
            .unwrap();
        let fork = repo
            .fork(
                "source",
                &ForkOptions {
                    entry_id: Some("tail".into()),
                    id: Some("fork".into()),
                    ..ForkOptions::default()
                },
            )
            .unwrap();
        assert_eq!(
            ids(&fork
                .find_entries(&EntryQuery {
                    order: EntryOrder::OldestFirst,
                    ..EntryQuery::default()
                })
                .unwrap()),
            ["root"]
        );
        assert_eq!(fork.get_leaf_id().as_deref(), Some("root"));
        assert_eq!(
            repo.open("source").unwrap().get_leaf_id().as_deref(),
            Some("tail")
        );
        let before_default = repo
            .fork(
                "source",
                &ForkOptions {
                    position: Some(ForkPosition::Before),
                    id: Some("before-default-target".into()),
                    ..ForkOptions::default()
                },
            )
            .unwrap();
        assert_eq!(
            ids(&before_default
                .find_entries(&EntryQuery {
                    order: EntryOrder::OldestFirst,
                    ..EntryQuery::default()
                })
                .unwrap()),
            ["root"]
        );
        assert_eq!(before_default.get_leaf_id().as_deref(), Some("root"));
        let at_default = repo
            .fork(
                "source",
                &ForkOptions {
                    position: Some(ForkPosition::At),
                    id: Some("at-default-target".into()),
                    ..ForkOptions::default()
                },
            )
            .unwrap();
        assert_eq!(
            ids(&at_default
                .find_entries(&EntryQuery {
                    order: EntryOrder::OldestFirst,
                    ..EntryQuery::default()
                })
                .unwrap()),
            ["root", "tail"]
        );
        assert_eq!(at_default.get_leaf_id().as_deref(), Some("tail"));
        assert_eq!(
            repo.fork(
                "source",
                &ForkOptions {
                    entry_id: Some("missing".into()),
                    ..ForkOptions::default()
                }
            )
            .unwrap_err()
            .code,
            "invalid_fork_target"
        );
    }

    #[test]
    fn validates_the_default_fork_target() {
        let mut repo = InMemorySessionRepo::new();
        let source = repo.create(Some("source-with-custom-leaf")).unwrap();
        source
            .append_entry(custom_entry("custom", "not-a-message", Value::Null), "main")
            .unwrap();
        assert_eq!(
            repo.fork(
                "source-with-custom-leaf",
                &ForkOptions {
                    id: Some("fork".into()),
                    ..ForkOptions::default()
                }
            )
            .unwrap_err()
            .code,
            "invalid_fork_target"
        );
    }
}
