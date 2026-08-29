use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JsonlV4Header {
    pub kind: String,
    pub version: u32,
    pub id: String,
    #[serde(rename = "createdAt")]
    pub created_at: u64,
    pub cwd: String,
    #[serde(rename = "parentSessionId", skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    #[serde(
        rename = "legacyParentSessionPath",
        skip_serializing_if = "Option::is_none"
    )]
    pub legacy_parent_session_path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

impl JsonlV4Header {
    pub fn source_format_hint(&self) -> u8 {
        if self.legacy_parent_session_path.is_some() {
            3
        } else {
            4
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionEntry {
    pub id: String,
    #[serde(rename = "type")]
    pub entry_type: String,
    #[serde(rename = "parentId")]
    pub parent_id: Option<String>,
    pub seq: u64,
    pub timestamp: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<Value>,
    #[serde(rename = "customType", skip_serializing_if = "Option::is_none")]
    pub custom_type: Option<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

impl SessionEntry {
    pub fn message(role: &str, content: Value) -> Self {
        Self {
            id: String::new(),
            entry_type: "message".into(),
            parent_id: None,
            seq: 0,
            timestamp: 0,
            message: Some(serde_json::json!({
                "role": role,
                "content": content,
            })),
            custom_type: None,
            extra: serde_json::Map::new(),
        }
    }

    pub fn label_change(target_id: &str, label: Option<&str>) -> Self {
        let mut extra = serde_json::Map::new();
        extra.insert("targetId".into(), Value::String(target_id.to_string()));
        if let Some(label) = label {
            extra.insert("label".into(), Value::String(label.to_string()));
        }
        Self {
            id: String::new(),
            entry_type: "label".into(),
            parent_id: None,
            seq: 0,
            timestamp: 0,
            message: None,
            custom_type: None,
            extra,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LaneRecord {
    pub id: String,
    #[serde(rename = "type")]
    pub record_type: String,
    pub seq: u64,
    pub timestamp: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lane: Option<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum SessionMutation {
    #[serde(rename = "entry")]
    Entry {
        #[serde(skip_serializing_if = "Option::is_none")]
        lane: Option<String>,
        entry: SessionEntry,
    },
    #[serde(rename = "record")]
    Record {
        #[serde(skip_serializing_if = "Option::is_none")]
        lane: Option<String>,
        record: LaneRecord,
    },
}

pub const ENTRY_TYPES: &[&str] = &[
    "message",
    "model_change",
    "thinking_level_change",
    "active_tools_change",
    "compaction",
    "branch_summary",
    "custom",
    "custom_message",
    "label",
    "session_info",
];

pub const RECORD_TYPES: &[&str] = &[
    "operation_started",
    "abort_requested",
    "operation_finished",
    "step_attempt",
    "tool_started",
    "queue_enqueued",
    "queue_cancelled",
    "write_deferred",
    "usage",
];
