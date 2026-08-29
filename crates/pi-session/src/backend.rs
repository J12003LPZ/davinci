//! Session tree / repository matching TypeScript `harness/session`.

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use thiserror::Error;
use uuid::Uuid;

pub const SESSION_ID_RULE: &str =
    "Session id must be non-empty, contain only alphanumeric characters, '-', '_', and '.', and start and end with an alphanumeric character";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackendErrorCode {
    NotFound,
    AlreadyExists,
    InvalidEntry,
    InvalidPayload,
    InvalidLane,
    InvalidQuery,
    InvalidForkTarget,
    Storage,
}

impl BackendErrorCode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NotFound => "not_found",
            Self::AlreadyExists => "already_exists",
            Self::InvalidEntry => "invalid_entry",
            Self::InvalidPayload => "invalid_payload",
            Self::InvalidLane => "invalid_lane",
            Self::InvalidQuery => "invalid_query",
            Self::InvalidForkTarget => "invalid_fork_target",
            Self::Storage => "storage",
        }
    }
}

#[derive(Debug, Error, Clone)]
#[error("{message}")]
pub struct BackendError {
    pub code: BackendErrorCode,
    pub message: String,
}

impl BackendError {
    pub fn new(code: BackendErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
        }
    }

    pub fn durable_payload(reason: &str) -> Self {
        Self::new(
            BackendErrorCode::InvalidPayload,
            format!("Durable payload {reason}"),
        )
    }
}

pub fn validate_session_id(id: &str) -> Result<(), BackendError> {
    let valid = !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        && id.chars().next().is_some_and(|c| c.is_ascii_alphanumeric())
        && id.chars().last().is_some_and(|c| c.is_ascii_alphanumeric());
    if valid {
        Ok(())
    } else {
        Err(BackendError::new(
            BackendErrorCode::InvalidPayload,
            SESSION_ID_RULE,
        ))
    }
}

pub fn assert_json_value(value: &Value) -> Result<(), BackendError> {
    fn walk(value: &Value) -> Result<(), BackendError> {
        match value {
            Value::Null | Value::Bool(_) | Value::String(_) => Ok(()),
            Value::Number(n) => {
                if n.as_f64().is_some_and(|f| !f.is_finite()) {
                    Err(BackendError::durable_payload(
                        "contains a non-finite number",
                    ))
                } else {
                    Ok(())
                }
            }
            Value::Array(items) => items.iter().try_for_each(walk),
            Value::Object(map) => map.values().try_for_each(walk),
        }
    }
    walk(value)
}

#[derive(Debug, Clone, Default)]
pub struct CreateOptions {
    pub id: Option<String>,
    pub parent_session_id: Option<String>,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ForkScope {
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
    pub id: Option<String>,
    pub parent_session_id: Option<String>,
    pub cwd: Option<String>,
    pub scope: Option<ForkScope>,
    pub entry_id: Option<String>,
    pub position: Option<ForkPosition>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMeta {
    pub id: String,
    pub created_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStats {
    pub message_count: i64,
    pub cached_tokens: f64,
    pub uncached_tokens: f64,
    pub total_tokens: f64,
    pub cost_total: f64,
}

impl Default for SessionStats {
    fn default() -> Self {
        Self {
            message_count: 0,
            cached_tokens: 0.0,
            uncached_tokens: 0.0,
            total_tokens: 0.0,
            cost_total: 0.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LanePointer {
    pub lane: String,
    pub leaf_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum LogItem {
    #[serde(rename = "entry")]
    Entry { seq: u64, entry: Value },
    #[serde(rename = "record")]
    Record { seq: u64, record: Value },
    #[serde(rename = "lane")]
    Lane {
        seq: u64,
        lane: String,
        #[serde(rename = "leafId")]
        leaf_id: Option<String>,
    },
    #[serde(rename = "fact")]
    Fact {
        seq: u64,
        fact: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(rename = "targetId", skip_serializing_if = "Option::is_none")]
        target_id: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        label: Option<String>,
        #[serde(default, skip_serializing)]
        name_cleared: bool,
        #[serde(default, skip_serializing)]
        label_cleared: bool,
    },
}

#[derive(Debug, Clone, Default)]
pub struct EntryQuery {
    pub type_name: Option<String>,
    pub custom_type: Option<String>,
    pub order: Option<String>,
    pub limit: Option<i64>,
    pub after_seq: Option<i64>,
    pub start: Option<String>,
    pub stop_at_type: Option<String>,
    pub stop_at_id: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct RecordQuery {
    pub lane: Option<String>,
    pub type_name: Option<String>,
    pub run_id: Option<String>,
    pub operation_kind: Option<String>,
    pub after_seq: Option<i64>,
    pub order: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Debug, Clone, Default)]
pub struct LogOptions {
    pub after_seq: Option<i64>,
    pub limit: Option<i64>,
}

#[derive(Debug, Clone)]
pub enum TreeMutation {
    Entry {
        lane: Option<String>,
        entry: Value,
    },
    Record {
        record: Value,
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

pub trait MutationSink: Send + Sync {
    fn persist(&self, session_id: &str, mutation: &TreeMutation) -> Result<(), BackendError>;
}

#[derive(Debug, Default)]
pub struct TreeState {
    sequence: u64,
    used_ids: HashSet<String>,
    entries: Vec<Value>,
    entries_by_id: HashMap<String, Value>,
    records: Vec<Value>,
    open_operations_by_lane: HashMap<String, IndexMap<String, Value>>,
    lanes: IndexMap<String, Option<String>>,
    log: Vec<LogItem>,
    stats: SessionStats,
    name: Option<String>,
    labels: HashMap<String, String>,
}

impl TreeState {
    pub fn new() -> Self {
        let mut lanes = IndexMap::new();
        lanes.insert("main".into(), None);
        Self {
            lanes,
            ..Self::default()
        }
    }

    pub fn next_sequence(&self) -> u64 {
        self.sequence + 1
    }

    pub fn get_lanes(&self) -> Vec<LanePointer> {
        self.lanes
            .iter()
            .map(|(lane, leaf_id)| LanePointer {
                lane: lane.clone(),
                leaf_id: leaf_id.clone(),
            })
            .collect()
    }

    pub fn require_lane(&self, lane: &str) -> Result<Option<String>, BackendError> {
        self.lanes.get(lane).cloned().ok_or_else(|| {
            BackendError::new(
                BackendErrorCode::InvalidLane,
                format!("Lane not found: {lane}"),
            )
        })
    }

    pub fn validate_new_lane(&self, lane: &str) -> Result<(), BackendError> {
        if self.lanes.contains_key(lane) {
            Err(BackendError::new(
                BackendErrorCode::AlreadyExists,
                format!("Lane already exists: {lane}"),
            ))
        } else {
            Ok(())
        }
    }

    pub fn validate_target(&self, target_id: Option<&str>) -> Result<(), BackendError> {
        match target_id {
            None => Ok(()),
            Some(id) if self.entries_by_id.contains_key(id) => Ok(()),
            Some(id) => Err(BackendError::new(
                BackendErrorCode::NotFound,
                format!("Entry not found: {id}"),
            )),
        }
    }

    pub fn validate_unused_id(&self, id: &str) -> Result<(), BackendError> {
        if self.used_ids.contains(id) {
            Err(BackendError::new(
                BackendErrorCode::AlreadyExists,
                format!("Session id already exists: {id}"),
            ))
        } else {
            Ok(())
        }
    }

    pub fn apply_mutation(&mut self, mutation: TreeMutation) -> Result<(), BackendError> {
        let seq = match &mutation {
            TreeMutation::Entry { entry, .. } => {
                entry.get("seq").and_then(Value::as_u64).unwrap_or(0)
            }
            TreeMutation::Record { record } => {
                record.get("seq").and_then(Value::as_u64).unwrap_or(0)
            }
            TreeMutation::Lane { seq, .. }
            | TreeMutation::FactName { seq, .. }
            | TreeMutation::FactLabel { seq, .. } => *seq,
        };
        if seq != self.sequence + 1 {
            return Err(BackendError::new(
                BackendErrorCode::InvalidEntry,
                format!("Invalid session mutation: has non-consecutive seq {seq}"),
            ));
        }
        match mutation {
            TreeMutation::Entry { lane, entry } => {
                let id = entry
                    .get("id")
                    .and_then(Value::as_str)
                    .ok_or_else(|| {
                        BackendError::new(
                            BackendErrorCode::InvalidEntry,
                            "Invalid session mutation: has invalid id",
                        )
                    })?
                    .to_string();
                if self.used_ids.contains(&id) {
                    return Err(BackendError::new(
                        BackendErrorCode::InvalidEntry,
                        format!("Invalid session mutation: contains duplicate id {id}"),
                    ));
                }
                let parent = match entry.get("parentId") {
                    Some(Value::Null) | None => None,
                    Some(Value::String(s)) => Some(s.clone()),
                    _ => None,
                };
                if let Some(lane) = &lane {
                    let leaf = self.lanes.get(lane).ok_or_else(|| {
                        BackendError::new(
                            BackendErrorCode::InvalidEntry,
                            format!("Invalid session mutation: references missing lane {lane}"),
                        )
                    })?;
                    if parent.as_ref() != leaf.as_ref() {
                        return Err(BackendError::new(
                            BackendErrorCode::InvalidEntry,
                            "Invalid session mutation: does not chain to the lane leaf",
                        ));
                    }
                }
                if let Some(parent_id) = &parent {
                    if !self.entries_by_id.contains_key(parent_id) {
                        return Err(BackendError::new(
                            BackendErrorCode::InvalidEntry,
                            format!(
                                "Invalid session mutation: references missing parent {parent_id}"
                            ),
                        ));
                    }
                }
                self.sequence = seq;
                self.used_ids.insert(id.clone());
                self.entries.push(entry.clone());
                self.entries_by_id.insert(id.clone(), entry.clone());
                if let Some(lane) = lane {
                    self.lanes.insert(lane, Some(id));
                }
                if entry.get("type").and_then(Value::as_str) == Some("message") {
                    self.stats.message_count += 1;
                }
                self.log.push(LogItem::Entry { seq, entry });
            }
            TreeMutation::Record { record } => {
                let lane = record
                    .get("lane")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if !self.lanes.contains_key(&lane) {
                    return Err(BackendError::new(
                        BackendErrorCode::InvalidEntry,
                        format!("Invalid session mutation: references missing lane {lane}"),
                    ));
                }
                let id = record
                    .get("id")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                if self.used_ids.contains(&id) {
                    return Err(BackendError::new(
                        BackendErrorCode::InvalidEntry,
                        format!("Invalid session mutation: contains duplicate id {id}"),
                    ));
                }
                self.sequence = seq;
                self.used_ids.insert(id.clone());
                self.records.push(record.clone());
                let typ = record.get("type").and_then(Value::as_str).unwrap_or("");
                if typ == "operation_started" {
                    self.open_operations_by_lane
                        .entry(lane)
                        .or_default()
                        .insert(id, record.clone());
                } else if typ == "operation_finished" {
                    if let Some(run_id) = record.get("runId").and_then(Value::as_str) {
                        if let Some(open) = self.open_operations_by_lane.get_mut(&lane) {
                            open.shift_remove(run_id);
                        }
                    }
                } else if typ == "usage" {
                    if let Some(usage) = record.get("usage") {
                        self.stats.cached_tokens += num(usage, "cacheRead");
                        self.stats.uncached_tokens +=
                            num(usage, "input") + num(usage, "cacheWrite");
                        self.stats.total_tokens += num(usage, "totalTokens");
                        self.stats.cost_total += usage
                            .get("cost")
                            .and_then(|c| c.get("total"))
                            .and_then(Value::as_f64)
                            .unwrap_or(0.0);
                    }
                }
                self.log.push(LogItem::Record { seq, record });
            }
            TreeMutation::Lane { seq, lane, leaf_id } => {
                if let Some(target) = &leaf_id {
                    if !self.entries_by_id.contains_key(target) {
                        return Err(BackendError::new(
                            BackendErrorCode::InvalidEntry,
                            format!(
                                "Invalid session mutation: references missing lane target {target}"
                            ),
                        ));
                    }
                }
                self.sequence = seq;
                self.lanes.insert(lane.clone(), leaf_id.clone());
                self.log.push(LogItem::Lane { seq, lane, leaf_id });
            }
            TreeMutation::FactName { seq, name } => {
                self.sequence = seq;
                self.name = name.clone();
                self.log.push(LogItem::Fact {
                    seq,
                    fact: "name".into(),
                    name,
                    target_id: None,
                    label: None,
                    name_cleared: true,
                    label_cleared: false,
                });
            }
            TreeMutation::FactLabel {
                seq,
                target_id,
                label,
            } => {
                if !self.entries_by_id.contains_key(&target_id) {
                    return Err(BackendError::new(
                        BackendErrorCode::InvalidEntry,
                        format!(
                            "Invalid session mutation: references missing label target {target_id}"
                        ),
                    ));
                }
                self.sequence = seq;
                match &label {
                    Some(value) => {
                        self.labels.insert(target_id.clone(), value.clone());
                    }
                    None => {
                        self.labels.remove(&target_id);
                    }
                }
                self.log.push(LogItem::Fact {
                    seq,
                    fact: "label".into(),
                    name: None,
                    target_id: Some(target_id),
                    label,
                    name_cleared: false,
                    label_cleared: true,
                });
            }
        }
        Ok(())
    }

    pub fn get_entry(&self, id: &str) -> Option<Value> {
        self.entries_by_id.get(id).cloned()
    }

    pub fn find_entries(&self, query: &EntryQuery) -> Result<Vec<Value>, BackendError> {
        assert_valid_limit(query.limit)?;
        assert_valid_cursor(query.after_seq)?;
        let mut results = Vec::new();
        for entry in ordered(&self.entries, query.order.as_deref()) {
            if !self.matches_entry(entry, query) {
                continue;
            }
            results.push(entry.clone());
            if query
                .limit
                .is_some_and(|limit| results.len() as i64 == limit)
            {
                break;
            }
        }
        Ok(results)
    }

    pub fn find_entries_on_branch(&self, query: &EntryQuery) -> Result<Vec<Value>, BackendError> {
        assert_valid_limit(query.limit)?;
        assert_valid_cursor(query.after_seq)?;
        let start = query.start.as_deref().ok_or_else(|| {
            BackendError::new(BackendErrorCode::InvalidQuery, "start is required")
        })?;
        let mut results = Vec::new();
        if query.order.as_deref() == Some("oldestFirst") {
            let walked: Vec<Value> = self
                .walk_to_root(start, None, None)?
                .into_iter()
                .rev()
                .collect();
            for entry in walked {
                let reached = query
                    .stop_at_id
                    .as_deref()
                    .is_some_and(|id| entry.get("id").and_then(Value::as_str) == Some(id))
                    || query
                        .stop_at_type
                        .as_deref()
                        .is_some_and(|ty| entry.get("type").and_then(Value::as_str) == Some(ty));
                if self.matches_entry(&entry, query) {
                    results.push(entry);
                }
                if reached
                    || query
                        .limit
                        .is_some_and(|limit| results.len() as i64 == limit)
                {
                    break;
                }
            }
        } else {
            for entry in self.walk_to_root(
                start,
                query.stop_at_id.as_deref(),
                query.stop_at_type.as_deref(),
            )? {
                if self.matches_entry(&entry, query) {
                    results.push(entry);
                }
                if query
                    .limit
                    .is_some_and(|limit| results.len() as i64 == limit)
                {
                    break;
                }
            }
        }
        Ok(results)
    }

    pub fn find_records(&self, query: &RecordQuery) -> Result<Vec<Value>, BackendError> {
        assert_valid_limit(query.limit)?;
        assert_valid_cursor(query.after_seq)?;
        let mut results = Vec::new();
        for record in ordered(&self.records, query.order.as_deref()) {
            if !self.matches_record(record, query) {
                continue;
            }
            results.push(record.clone());
            if query
                .limit
                .is_some_and(|limit| results.len() as i64 == limit)
            {
                break;
            }
        }
        Ok(results)
    }

    pub fn find_open_operations(
        &self,
        lane: &str,
        limit: Option<i64>,
    ) -> Result<Vec<Value>, BackendError> {
        assert_valid_limit(limit)?;
        let open = self
            .open_operations_by_lane
            .get(lane)
            .map(|map| map.values().rev().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        Ok(match limit {
            Some(n) => open.into_iter().take(n as usize).collect(),
            None => open,
        })
    }

    pub fn get_log(&self, options: &LogOptions) -> Result<Vec<LogItem>, BackendError> {
        assert_valid_limit(options.limit)?;
        assert_valid_cursor(options.after_seq)?;
        let mut results = Vec::new();
        for item in &self.log {
            let seq = log_seq(item);
            if options.after_seq.is_some_and(|after| seq <= after as u64) {
                continue;
            }
            results.push(item.clone());
            if options
                .limit
                .is_some_and(|limit| results.len() as i64 == limit)
            {
                break;
            }
        }
        Ok(results)
    }

    pub fn get_name(&self) -> Option<String> {
        self.name.clone()
    }

    pub fn get_label(&self, id: &str) -> Option<String> {
        self.labels.get(id).cloned()
    }

    pub fn get_stats(&self) -> SessionStats {
        self.stats.clone()
    }

    pub fn create_fork_mutations(
        &self,
        options: &ForkOptions,
    ) -> Result<Vec<TreeMutation>, BackendError> {
        let (copied, fork_lanes) = if options.scope == Some(ForkScope::Tree) {
            (
                self.find_entries(&EntryQuery {
                    order: Some("oldestFirst".into()),
                    ..EntryQuery::default()
                })?,
                self.get_lanes(),
            )
        } else {
            let selected = match &options.entry_id {
                Some(id) => Some(id.clone()),
                None => self.require_lane("main")?,
            };
            let mut target_id = None;
            if let Some(selected_id) = selected {
                let entry = self.get_entry(&selected_id);
                if entry
                    .as_ref()
                    .and_then(|e| e.get("type").and_then(Value::as_str))
                    != Some("message")
                {
                    return Err(BackendError::new(
                        BackendErrorCode::InvalidForkTarget,
                        format!("Fork target is not a message entry: {selected_id}"),
                    ));
                }
                let position = options.position.unwrap_or(if options.entry_id.is_none() {
                    ForkPosition::At
                } else {
                    ForkPosition::Before
                });
                target_id = match position {
                    ForkPosition::At => Some(selected_id),
                    ForkPosition::Before => entry.and_then(|e| {
                        e.get("parentId")
                            .and_then(|v| v.as_str().map(str::to_string))
                    }),
                };
            }
            let copied = match &target_id {
                None => Vec::new(),
                Some(id) => self.find_entries_on_branch(&EntryQuery {
                    start: Some(id.clone()),
                    order: Some("oldestFirst".into()),
                    ..EntryQuery::default()
                })?,
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
        for source in copied.iter() {
            let mut entry = source.clone();
            if let Some(obj) = entry.as_object_mut() {
                obj.insert("seq".into(), json!(sequence));
            }
            mutations.push(TreeMutation::Entry { lane: None, entry });
            sequence += 1;
        }
        for pointer in fork_lanes {
            mutations.push(TreeMutation::Lane {
                seq: sequence,
                lane: pointer.lane,
                leaf_id: pointer.leaf_id,
            });
            sequence += 1;
        }
        if let Some(name) = &self.name {
            mutations.push(TreeMutation::FactName {
                seq: sequence,
                name: Some(name.clone()),
            });
            sequence += 1;
        }
        for entry in &copied {
            if let Some(id) = entry.get("id").and_then(Value::as_str) {
                if let Some(label) = self.labels.get(id) {
                    mutations.push(TreeMutation::FactLabel {
                        seq: sequence,
                        target_id: id.to_string(),
                        label: Some(label.clone()),
                    });
                    sequence += 1;
                }
            }
        }
        let _ = sequence;
        Ok(mutations)
    }

    fn walk_to_root(
        &self,
        start: &str,
        stop_at_id: Option<&str>,
        stop_at_type: Option<&str>,
    ) -> Result<Vec<Value>, BackendError> {
        let mut out = Vec::new();
        let mut visited = HashSet::new();
        let mut current = self.entries_by_id.get(start).cloned().ok_or_else(|| {
            BackendError::new(
                BackendErrorCode::NotFound,
                format!("Entry not found: {start}"),
            )
        })?;
        loop {
            let id = current
                .get("id")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            if !visited.insert(id.clone()) {
                return Err(BackendError::new(
                    BackendErrorCode::InvalidEntry,
                    format!("Session branch contains a cycle at {id}"),
                ));
            }
            let typ = current
                .get("type")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let parent = current
                .get("parentId")
                .and_then(|v| v.as_str().map(str::to_string));
            out.push(current);
            if stop_at_id == Some(id.as_str())
                || stop_at_type == Some(typ.as_str())
                || parent.is_none()
            {
                break;
            }
            let parent_id = parent.unwrap();
            current = self.entries_by_id.get(&parent_id).cloned().ok_or_else(|| {
                BackendError::new(
                    BackendErrorCode::InvalidEntry,
                    format!("Entry not found: {parent_id}"),
                )
            })?;
        }
        Ok(out)
    }

    fn matches_entry(&self, entry: &Value, query: &EntryQuery) -> bool {
        if query
            .type_name
            .as_deref()
            .is_some_and(|ty| entry.get("type").and_then(Value::as_str) != Some(ty))
        {
            return false;
        }
        if let Some(custom) = &query.custom_type {
            if entry.get("type").and_then(Value::as_str) != Some("custom")
                || entry.get("customType").and_then(Value::as_str) != Some(custom)
            {
                return false;
            }
        }
        if let Some(after) = query.after_seq {
            let seq = entry.get("seq").and_then(Value::as_u64).unwrap_or(0) as i64;
            if query.order.as_deref() == Some("oldestFirst") {
                if seq <= after {
                    return false;
                }
            } else if seq >= after {
                return false;
            }
        }
        true
    }

    fn matches_record(&self, record: &Value, query: &RecordQuery) -> bool {
        if query
            .lane
            .as_deref()
            .is_some_and(|lane| record.get("lane").and_then(Value::as_str) != Some(lane))
        {
            return false;
        }
        if query
            .type_name
            .as_deref()
            .is_some_and(|ty| record.get("type").and_then(Value::as_str) != Some(ty))
        {
            return false;
        }
        if let Some(run_id) = &query.run_id {
            let matches = if record.get("type").and_then(Value::as_str) == Some("operation_started")
            {
                record.get("id").and_then(Value::as_str) == Some(run_id)
            } else {
                record.get("runId").and_then(Value::as_str) == Some(run_id)
            };
            if !matches {
                return false;
            }
        }
        if let Some(kind) = &query.operation_kind {
            if record.get("type").and_then(Value::as_str) != Some("operation_started")
                || record.pointer("/intent/kind").and_then(Value::as_str) != Some(kind)
            {
                return false;
            }
        }
        if let Some(after) = query.after_seq {
            if record.get("seq").and_then(Value::as_u64).unwrap_or(0) as i64 <= after {
                return false;
            }
        }
        true
    }
}

fn num(value: &Value, key: &str) -> f64 {
    value
        .get(key)
        .and_then(Value::as_f64)
        .or_else(|| value.get(key).and_then(Value::as_i64).map(|n| n as f64))
        .unwrap_or(0.0)
}

fn log_seq(item: &LogItem) -> u64 {
    match item {
        LogItem::Entry { seq, .. }
        | LogItem::Record { seq, .. }
        | LogItem::Lane { seq, .. }
        | LogItem::Fact { seq, .. } => *seq,
    }
}

fn ordered<'a>(items: &'a [Value], order: Option<&str>) -> Vec<&'a Value> {
    if order == Some("oldestFirst") {
        items.iter().collect()
    } else {
        items.iter().rev().collect()
    }
}

fn assert_valid_limit(limit: Option<i64>) -> Result<(), BackendError> {
    if let Some(limit) = limit {
        if limit <= 0 {
            return Err(BackendError::new(
                BackendErrorCode::InvalidQuery,
                "limit must be a positive integer",
            ));
        }
    }
    Ok(())
}

fn assert_valid_cursor(after_seq: Option<i64>) -> Result<(), BackendError> {
    if let Some(after) = after_seq {
        if after < 0 {
            return Err(BackendError::new(
                BackendErrorCode::InvalidQuery,
                "cursor sequence must be a non-negative integer",
            ));
        }
    }
    Ok(())
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

struct SessionInner {
    metadata: SessionMeta,
    tree: TreeState,
    persist: Option<Arc<dyn MutationSink>>,
}

impl SessionInner {
    fn persist_mut(&self, mutation: &TreeMutation) -> Result<(), BackendError> {
        if let Some(sink) = &self.persist {
            sink.persist(&self.metadata.id, mutation)?;
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct Session {
    inner: Arc<Mutex<SessionInner>>,
}

impl Session {
    fn new(metadata: SessionMeta, persist: Option<Arc<dyn MutationSink>>) -> Self {
        Self {
            inner: Arc::new(Mutex::new(SessionInner {
                metadata,
                tree: TreeState::new(),
                persist,
            })),
        }
    }

    pub fn from_parts(
        metadata: SessionMeta,
        tree: TreeState,
        persist: Option<Arc<dyn MutationSink>>,
    ) -> Self {
        Self {
            inner: Arc::new(Mutex::new(SessionInner {
                metadata,
                tree,
                persist,
            })),
        }
    }

    pub fn fork_mutations(&self, options: &ForkOptions) -> Result<Vec<TreeMutation>, BackendError> {
        self.lock()?.tree.create_fork_mutations(options)
    }

    pub fn persist_only(&self, mutation: &TreeMutation) -> Result<(), BackendError> {
        self.lock()?.persist_mut(mutation)
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, SessionInner>, BackendError> {
        self.inner
            .lock()
            .map_err(|_| BackendError::new(BackendErrorCode::Storage, "session lock poisoned"))
    }

    pub fn get_metadata(&self) -> Result<SessionMeta, BackendError> {
        Ok(self.lock()?.metadata.clone())
    }

    pub fn get_lanes(&self) -> Result<Vec<LanePointer>, BackendError> {
        Ok(self.lock()?.tree.get_lanes())
    }

    pub fn create_lane(&self, lane: &str, at: Option<&str>) -> Result<(), BackendError> {
        let mut inner = self.lock()?;
        inner.tree.validate_new_lane(lane)?;
        inner.tree.validate_target(at)?;
        let mutation = TreeMutation::Lane {
            seq: inner.tree.next_sequence(),
            lane: lane.to_string(),
            leaf_id: at.map(str::to_string),
        };
        inner.tree.apply_mutation(mutation.clone())?;
        inner.persist_mut(&mutation)
    }

    pub fn move_lane(&self, lane: &str, to: Option<&str>) -> Result<(), BackendError> {
        let mut inner = self.lock()?;
        inner.tree.require_lane(lane)?;
        inner.tree.validate_target(to)?;
        let mutation = TreeMutation::Lane {
            seq: inner.tree.next_sequence(),
            lane: lane.to_string(),
            leaf_id: to.map(str::to_string),
        };
        inner.tree.apply_mutation(mutation.clone())?;
        inner.persist_mut(&mutation)
    }

    pub fn append_entry(&self, mut entry: Value, lane: &str) -> Result<Value, BackendError> {
        assert_json_value(&entry)?;
        let mut inner = self.lock()?;
        let parent = inner.tree.require_lane(lane)?;
        let id = entry
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| BackendError::new(BackendErrorCode::InvalidEntry, "has invalid id"))?
            .to_string();
        inner.tree.validate_unused_id(&id)?;
        let obj = entry.as_object_mut().ok_or_else(|| {
            BackendError::new(BackendErrorCode::InvalidPayload, "entry must be an object")
        })?;
        obj.insert(
            "parentId".into(),
            parent.clone().map(Value::String).unwrap_or(Value::Null),
        );
        obj.insert("seq".into(), json!(inner.tree.next_sequence()));
        obj.insert("timestamp".into(), json!(now_ms()));
        let mutation = TreeMutation::Entry {
            lane: Some(lane.to_string()),
            entry: entry.clone(),
        };
        inner.tree.apply_mutation(mutation.clone())?;
        inner.persist_mut(&mutation)?;
        Ok(entry)
    }

    pub fn append_record(&self, mut record: Value) -> Result<Value, BackendError> {
        assert_json_value(&record)?;
        let mut inner = self.lock()?;
        let lane = record
            .get("lane")
            .and_then(Value::as_str)
            .ok_or_else(|| BackendError::new(BackendErrorCode::InvalidLane, "Lane not found: "))?
            .to_string();
        inner.tree.require_lane(&lane)?;
        let id = record
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        inner.tree.validate_unused_id(&id)?;
        if record.get("type").and_then(Value::as_str) == Some("operation_started") {
            if let Some(open) = inner.tree.find_open_operations(&lane, Some(1))?.first() {
                let open_id = open.get("id").and_then(Value::as_str).unwrap_or("");
                return Err(BackendError::new(
                    BackendErrorCode::Storage,
                    format!("Lane {lane} already has an open operation {open_id}"),
                ));
            }
        }
        let obj = record.as_object_mut().ok_or_else(|| {
            BackendError::new(BackendErrorCode::InvalidPayload, "record must be an object")
        })?;
        obj.insert("seq".into(), json!(inner.tree.next_sequence()));
        obj.insert("timestamp".into(), json!(now_ms()));
        let mutation = TreeMutation::Record {
            record: record.clone(),
        };
        inner.tree.apply_mutation(mutation.clone())?;
        inner.persist_mut(&mutation)?;
        Ok(record)
    }

    pub fn get_entry(&self, id: &str) -> Result<Option<Value>, BackendError> {
        Ok(self.lock()?.tree.get_entry(id))
    }

    pub fn find_entries(&self, query: EntryQuery) -> Result<Vec<Value>, BackendError> {
        assert_valid_limit(query.limit)?;
        assert_valid_cursor(query.after_seq)?;
        self.lock()?.tree.find_entries(&query)
    }

    pub fn find_entry(&self, query: EntryQuery) -> Result<Option<Value>, BackendError> {
        let mut query = query;
        assert_valid_limit(query.limit)?;
        assert_valid_cursor(query.after_seq)?;
        query.limit = Some(query.limit.unwrap_or(1).min(1));
        Ok(self.lock()?.tree.find_entries(&query)?.into_iter().next())
    }

    pub fn find_entries_on_branch(
        &self,
        lane: &str,
        mut query: EntryQuery,
    ) -> Result<Vec<Value>, BackendError> {
        assert_valid_limit(query.limit)?;
        assert_valid_cursor(query.after_seq)?;
        let inner = self.lock()?;
        if query.start.is_none() {
            query.start = inner.tree.require_lane(lane)?;
        }
        if query.start.is_none() {
            return Ok(Vec::new());
        }
        inner.tree.find_entries_on_branch(&query)
    }

    pub fn find_entry_on_branch(
        &self,
        lane: &str,
        mut query: EntryQuery,
    ) -> Result<Option<Value>, BackendError> {
        assert_valid_limit(query.limit)?;
        assert_valid_cursor(query.after_seq)?;
        query.limit = Some(query.limit.unwrap_or(1).min(1));
        Ok(self.find_entries_on_branch(lane, query)?.into_iter().next())
    }

    pub fn find_records(&self, query: RecordQuery) -> Result<Vec<Value>, BackendError> {
        assert_valid_limit(query.limit)?;
        assert_valid_cursor(query.after_seq)?;
        if query.operation_kind.is_some() && query.type_name.as_deref() != Some("operation_started")
        {
            return Err(BackendError::new(
                BackendErrorCode::InvalidQuery,
                "operationKind requires type \"operation_started\"",
            ));
        }
        self.lock()?.tree.find_records(&query)
    }

    pub fn find_open_operations(
        &self,
        lane: &str,
        limit: Option<i64>,
    ) -> Result<Vec<Value>, BackendError> {
        assert_valid_limit(limit)?;
        self.lock()?.tree.find_open_operations(lane, limit)
    }

    pub fn get_log(&self, options: LogOptions) -> Result<Vec<LogItem>, BackendError> {
        assert_valid_limit(options.limit)?;
        assert_valid_cursor(options.after_seq)?;
        self.lock()?.tree.get_log(&options)
    }

    pub fn get_name(&self) -> Result<Option<String>, BackendError> {
        Ok(self.lock()?.tree.get_name())
    }

    pub fn set_name(&self, name: Option<&str>) -> Result<(), BackendError> {
        let mut inner = self.lock()?;
        let mutation = TreeMutation::FactName {
            seq: inner.tree.next_sequence(),
            name: name.map(str::to_string),
        };
        inner.tree.apply_mutation(mutation.clone())?;
        inner.persist_mut(&mutation)
    }

    pub fn get_label(&self, id: &str) -> Result<Option<String>, BackendError> {
        Ok(self.lock()?.tree.get_label(id))
    }

    pub fn set_label(&self, id: &str, label: Option<&str>) -> Result<(), BackendError> {
        let mut inner = self.lock()?;
        inner.tree.validate_target(Some(id))?;
        let mutation = TreeMutation::FactLabel {
            seq: inner.tree.next_sequence(),
            target_id: id.to_string(),
            label: label.map(str::to_string),
        };
        inner.tree.apply_mutation(mutation.clone())?;
        inner.persist_mut(&mutation)
    }

    pub fn get_stats(&self) -> Result<SessionStats, BackendError> {
        Ok(self.lock()?.tree.get_stats())
    }

    pub fn get_leaf_id(&self, lane: &str) -> Result<Option<String>, BackendError> {
        self.lock()?.tree.require_lane(lane)
    }

    pub fn append_message(&self, message: Value, lane: &str) -> Result<String, BackendError> {
        let id = Uuid::new_v4().to_string();
        let entry = json!({
            "type": "message",
            "id": id,
            "message": message,
        });
        let stored = self.append_entry(entry, lane)?;
        Ok(stored
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string())
    }

    pub fn append_custom_entry(
        &self,
        custom_type: &str,
        data: Option<Value>,
        lane: &str,
    ) -> Result<String, BackendError> {
        let id = Uuid::new_v4().to_string();
        let mut map = Map::new();
        map.insert("type".into(), json!("custom"));
        map.insert("id".into(), json!(id));
        map.insert("customType".into(), json!(custom_type));
        if let Some(data) = data {
            map.insert("data".into(), data);
        }
        let stored = self.append_entry(Value::Object(map), lane)?;
        Ok(stored
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string())
    }

    pub fn view(&self, _lane: &str) -> Session {
        self.clone()
    }

    pub fn apply_loaded_mutations(&self, mutations: Vec<TreeMutation>) -> Result<(), BackendError> {
        let mut inner = self.lock()?;
        for mutation in mutations {
            inner.tree.apply_mutation(mutation)?;
        }
        Ok(())
    }
}

pub trait SessionRepository {
    fn create(&self, options: CreateOptions) -> Result<Session, BackendError>;
    fn open(&self, metadata: &SessionMeta) -> Result<Session, BackendError>;
    fn list(&self) -> Result<Vec<SessionMeta>, BackendError>;
    fn delete(&self, metadata: &SessionMeta) -> Result<(), BackendError>;
    fn fork(&self, source: &SessionMeta, options: ForkOptions) -> Result<Session, BackendError>;
}

#[derive(Default)]
pub struct MemorySessionRepo {
    sessions: Mutex<HashMap<String, Session>>,
}

impl MemorySessionRepo {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SessionRepository for MemorySessionRepo {
    fn create(&self, options: CreateOptions) -> Result<Session, BackendError> {
        let id = options
            .id
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        validate_session_id(&id)?;
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| BackendError::new(BackendErrorCode::Storage, "repo lock poisoned"))?;
        if sessions.contains_key(&id) {
            return Err(BackendError::new(
                BackendErrorCode::AlreadyExists,
                format!("Session already exists: {id}"),
            ));
        }
        let session = Session::new(
            SessionMeta {
                id: id.clone(),
                created_at: now_ms(),
                parent_session_id: options.parent_session_id,
                cwd: options.cwd,
            },
            None,
        );
        sessions.insert(id, session.clone());
        Ok(session)
    }

    fn open(&self, metadata: &SessionMeta) -> Result<Session, BackendError> {
        self.sessions
            .lock()
            .map_err(|_| BackendError::new(BackendErrorCode::Storage, "repo lock poisoned"))?
            .get(&metadata.id)
            .cloned()
            .ok_or_else(|| {
                BackendError::new(
                    BackendErrorCode::NotFound,
                    format!("Session not found: {}", metadata.id),
                )
            })
    }

    fn list(&self) -> Result<Vec<SessionMeta>, BackendError> {
        let sessions = self
            .sessions
            .lock()
            .map_err(|_| BackendError::new(BackendErrorCode::Storage, "repo lock poisoned"))?;
        sessions
            .values()
            .map(|s| s.get_metadata())
            .collect::<Result<Vec<_>, _>>()
    }

    fn delete(&self, metadata: &SessionMeta) -> Result<(), BackendError> {
        self.sessions
            .lock()
            .map_err(|_| BackendError::new(BackendErrorCode::Storage, "repo lock poisoned"))?
            .remove(&metadata.id);
        Ok(())
    }

    fn fork(&self, source: &SessionMeta, options: ForkOptions) -> Result<Session, BackendError> {
        let source_session = self.open(source)?;
        let id = options
            .id
            .clone()
            .unwrap_or_else(|| Uuid::new_v4().to_string());
        validate_session_id(&id)?;
        let mut sessions = self
            .sessions
            .lock()
            .map_err(|_| BackendError::new(BackendErrorCode::Storage, "repo lock poisoned"))?;
        if sessions.contains_key(&id) {
            return Err(BackendError::new(
                BackendErrorCode::AlreadyExists,
                format!("Session already exists: {id}"),
            ));
        }
        let mutations = source_session
            .lock()?
            .tree
            .create_fork_mutations(&options)?;
        let dest = Session::from_parts(
            SessionMeta {
                id: id.clone(),
                created_at: now_ms(),
                parent_session_id: options
                    .parent_session_id
                    .clone()
                    .or_else(|| Some(source.id.clone())),
                cwd: options.cwd.or_else(|| source.cwd.clone()),
            },
            TreeState::new(),
            None,
        );
        dest.apply_loaded_mutations(mutations)?;
        sessions.insert(id, dest.clone());
        Ok(dest)
    }
}

pub fn user_message(text: &str) -> Value {
    json!({
        "role": "user",
        "content": [{"type": "text", "text": text}],
        "timestamp": 1
    })
}

pub fn assistant_message(text: &str) -> Value {
    json!({
        "role": "assistant",
        "content": [{"type": "text", "text": text}],
        "api": "anthropic-messages",
        "provider": "anthropic",
        "model": "claude-sonnet-4-5",
        "usage": {
            "input": 0,
            "output": 0,
            "cacheRead": 0,
            "cacheWrite": 0,
            "totalTokens": 0,
            "cost": {"input": 0, "output": 0, "cacheRead": 0, "cacheWrite": 0, "total": 0}
        },
        "stopReason": "stop",
        "timestamp": 1
    })
}

pub fn operation_started(id: &str, lane: &str, kind: &str) -> Value {
    let intent = match kind {
        "compaction" => json!({"kind": "compaction", "resultEntryId": format!("{id}-result")}),
        "navigation" => json!({"kind": "navigation", "targetId": null, "summarize": false}),
        _ => json!({"kind": "run", "originalPrompt": [], "initialMessages": []}),
    };
    json!({
        "type": "operation_started",
        "id": id,
        "lane": lane,
        "sourceLeafId": null,
        "intent": intent
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_id_rule_matches_ts() {
        assert!(validate_session_id("abc").is_ok());
        assert_eq!(
            validate_session_id("-bad").unwrap_err().message,
            SESSION_ID_RULE
        );
    }
}
