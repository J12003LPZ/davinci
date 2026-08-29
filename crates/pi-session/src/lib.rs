//! Session harness types shared by JSONL and SQLite backends.
//!
//! Mirrors `vendor/pi/packages/agent/src/harness/session/types.ts`.

mod conformance;
mod jsonl;
mod memory;

use std::collections::BTreeMap;

use pi_core::{next_id, now_ms, SessionError};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use conformance::{run_conformance, ConformanceReport};
pub use jsonl::{JsonlSession, JsonlSessionRepository};
pub use memory::MemorySessionRepository;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Entry {
    Message {
        id: String,
        seq: i64,
        #[serde(rename = "parentId")]
        parent_id: Option<String>,
        timestamp: i64,
        message: Value,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        terminate: Option<bool>,
    },
    ModelChange {
        id: String,
        seq: i64,
        #[serde(rename = "parentId")]
        parent_id: Option<String>,
        timestamp: i64,
        provider: String,
        #[serde(rename = "modelId")]
        model_id: String,
    },
    ThinkingLevelChange {
        id: String,
        seq: i64,
        #[serde(rename = "parentId")]
        parent_id: Option<String>,
        timestamp: i64,
        #[serde(rename = "thinkingLevel")]
        thinking_level: String,
    },
    Custom {
        id: String,
        seq: i64,
        #[serde(rename = "parentId")]
        parent_id: Option<String>,
        timestamp: i64,
        #[serde(rename = "customType")]
        custom_type: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        data: Option<Value>,
    },
}

impl Entry {
    pub fn id(&self) -> &str {
        match self {
            Self::Message { id, .. }
            | Self::ModelChange { id, .. }
            | Self::ThinkingLevelChange { id, .. }
            | Self::Custom { id, .. } => id,
        }
    }

    pub fn seq(&self) -> i64 {
        match self {
            Self::Message { seq, .. }
            | Self::ModelChange { seq, .. }
            | Self::ThinkingLevelChange { seq, .. }
            | Self::Custom { seq, .. } => *seq,
        }
    }

    pub fn parent_id(&self) -> Option<&str> {
        self.parent()
    }

    pub fn entry_type(&self) -> &'static str {
        match self {
            Self::Message { .. } => "message",
            Self::ModelChange { .. } => "model_change",
            Self::ThinkingLevelChange { .. } => "thinking_level_change",
            Self::Custom { .. } => "custom",
        }
    }

    pub fn is_message(&self) -> bool {
        matches!(self, Self::Message { .. })
    }
}

// Helper enum arm used only so the match in parent_id is exhaustive without a dummy.
// The real variants already cover parent_id. Keep parent_id implementation simple:
impl Entry {
    pub fn parent(&self) -> Option<&str> {
        match self {
            Self::Message { parent_id, .. }
            | Self::ModelChange { parent_id, .. }
            | Self::ThinkingLevelChange { parent_id, .. }
            | Self::Custom { parent_id, .. } => parent_id.as_deref(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Usage {
    pub input: i64,
    pub output: i64,
    #[serde(rename = "cacheRead")]
    pub cache_read: i64,
    #[serde(rename = "cacheWrite")]
    pub cache_write: i64,
    #[serde(rename = "totalTokens")]
    pub total_tokens: i64,
    pub cost: UsageCost,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct UsageCost {
    pub input: f64,
    pub output: f64,
    #[serde(rename = "cacheRead")]
    pub cache_read: f64,
    #[serde(rename = "cacheWrite")]
    pub cache_write: f64,
    pub total: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum LaneRecord {
    OperationStarted {
        id: String,
        seq: i64,
        lane: String,
        timestamp: i64,
        #[serde(rename = "runId", default, skip_serializing_if = "Option::is_none")]
        run_id: Option<String>,
        #[serde(flatten)]
        extra: BTreeMap<String, Value>,
    },
    OperationFinished {
        id: String,
        seq: i64,
        lane: String,
        timestamp: i64,
        #[serde(rename = "runId")]
        run_id: String,
        outcome: String,
    },
    Usage {
        id: String,
        seq: i64,
        lane: String,
        timestamp: i64,
        usage: Usage,
        #[serde(flatten)]
        extra: BTreeMap<String, Value>,
    },
    Other {
        id: String,
        seq: i64,
        lane: String,
        timestamp: i64,
        #[serde(rename = "recordType")]
        record_type: String,
        #[serde(flatten)]
        extra: BTreeMap<String, Value>,
    },
}

impl LaneRecord {
    pub fn id(&self) -> &str {
        match self {
            Self::OperationStarted { id, .. }
            | Self::OperationFinished { id, .. }
            | Self::Usage { id, .. }
            | Self::Other { id, .. } => id,
        }
    }

    pub fn seq(&self) -> i64 {
        match self {
            Self::OperationStarted { seq, .. }
            | Self::OperationFinished { seq, .. }
            | Self::Usage { seq, .. }
            | Self::Other { seq, .. } => *seq,
        }
    }

    pub fn lane(&self) -> &str {
        match self {
            Self::OperationStarted { lane, .. }
            | Self::OperationFinished { lane, .. }
            | Self::Usage { lane, .. }
            | Self::Other { lane, .. } => lane,
        }
    }

    pub fn record_type(&self) -> &str {
        match self {
            Self::OperationStarted { .. } => "operation_started",
            Self::OperationFinished { .. } => "operation_finished",
            Self::Usage { .. } => "usage",
            Self::Other { record_type, .. } => record_type,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LogItem {
    Entry {
        seq: i64,
        entry: Entry,
    },
    Record {
        seq: i64,
        record: LaneRecord,
    },
    Lane {
        seq: i64,
        lane: String,
        #[serde(rename = "leafId")]
        leaf_id: Option<String>,
    },
    Fact {
        seq: i64,
        fact: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(rename = "targetId", default, skip_serializing_if = "Option::is_none")]
        target_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        label: Option<String>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct SessionStats {
    #[serde(rename = "messageCount")]
    pub message_count: i64,
    #[serde(rename = "cachedTokens")]
    pub cached_tokens: f64,
    #[serde(rename = "uncachedTokens")]
    pub uncached_tokens: f64,
    #[serde(rename = "totalTokens")]
    pub total_tokens: f64,
    #[serde(rename = "costTotal")]
    pub cost_total: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionMetadata {
    pub id: String,
    #[serde(rename = "createdAt")]
    pub created_at: i64,
    pub cwd: String,
    pub path: String,
    #[serde(
        rename = "parentSessionId",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    pub parent_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, Default)]
pub struct SessionCreateOptions {
    pub id: Option<String>,
    pub cwd: String,
    pub parent_session_id: Option<String>,
    pub metadata: Option<Value>,
    pub name: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryOrder {
    OldestFirst,
    NewestFirst,
}

#[derive(Debug, Clone, Default)]
pub struct EntryQuery {
    pub order: Option<QueryOrder>,
    pub limit: Option<usize>,
    pub entry_type: Option<String>,
}

pub fn user_message(text: &str) -> Value {
    serde_json::json!({
        "role": "user",
        "content": text,
        "timestamp": now_ms(),
    })
}

pub fn message_entry(id: impl Into<String>, message: Value) -> Entry {
    Entry::Message {
        id: id.into(),
        seq: 0,
        parent_id: None,
        timestamp: 0,
        message,
        terminate: None,
    }
}

pub fn assign_storage_fields(
    entry: Entry,
    seq: i64,
    parent_id: Option<String>,
    timestamp: i64,
) -> Entry {
    match entry {
        Entry::Message {
            id,
            message,
            terminate,
            ..
        } => Entry::Message {
            id,
            seq,
            parent_id,
            timestamp,
            message,
            terminate,
        },
        Entry::ModelChange {
            id,
            provider,
            model_id,
            ..
        } => Entry::ModelChange {
            id,
            seq,
            parent_id,
            timestamp,
            provider,
            model_id,
        },
        Entry::ThinkingLevelChange {
            id, thinking_level, ..
        } => Entry::ThinkingLevelChange {
            id,
            seq,
            parent_id,
            timestamp,
            thinking_level,
        },
        Entry::Custom {
            id,
            custom_type,
            data,
            ..
        } => Entry::Custom {
            id,
            seq,
            parent_id,
            timestamp,
            custom_type,
            data,
        },
    }
}

pub fn provision_message(text: &str) -> Entry {
    message_entry(next_id(), user_message(text))
}

#[derive(Debug, Clone)]
pub struct ForkOptions {
    pub cwd: String,
    pub scope: ForkScope,
    pub position: ForkPosition,
    pub entry_id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ForkScope {
    #[default]
    Branch,
    Tree,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ForkPosition {
    #[default]
    At,
    Before,
}

/// Backend-facing session handle.
pub trait SessionStore: Send {
    fn metadata(&self) -> Result<SessionMetadata, SessionError>;
    fn append_entry(&mut self, entry: Entry, lane: &str) -> Result<Entry, SessionError>;
    fn find_entries(&self, query: EntryQuery) -> Result<Vec<Entry>, SessionError>;
    fn find_entries_on_branch(&self, start: &str) -> Result<Vec<Entry>, SessionError>;
    fn append_record(&mut self, record: LaneRecord) -> Result<LaneRecord, SessionError>;
    fn find_records(&self, lane: Option<&str>) -> Result<Vec<LaneRecord>, SessionError>;
    fn get_log(&self, limit: Option<usize>) -> Result<Vec<LogItem>, SessionError>;
    fn set_name(&mut self, name: Option<&str>) -> Result<(), SessionError>;
    fn get_name(&self) -> Result<Option<String>, SessionError>;
    fn get_stats(&self) -> Result<SessionStats, SessionError>;
    fn release(&mut self) -> Result<(), SessionError>;
}

pub trait SessionRepository: Send {
    fn create(
        &mut self,
        options: SessionCreateOptions,
    ) -> Result<Box<dyn SessionStore>, SessionError>;
    fn open(&mut self, metadata: &SessionMetadata) -> Result<Box<dyn SessionStore>, SessionError>;
    fn list(&self, cwd: Option<&str>) -> Result<Vec<SessionMetadata>, SessionError>;
    fn delete(&mut self, metadata: &SessionMetadata) -> Result<(), SessionError>;
    fn fork(
        &mut self,
        source: &dyn SessionStore,
        options: ForkOptions,
    ) -> Result<Box<dyn SessionStore>, SessionError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn user_message_shape_matches_typescript() {
        let msg = user_message("hello");
        assert_eq!(msg["role"], "user");
        assert_eq!(msg["content"], "hello");
        assert!(msg["timestamp"].is_number());
    }
}
