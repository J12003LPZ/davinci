use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PROTOCOL_VERSION: i64 = 1;
pub const DEFAULT_WRITER_LEASE_TTL_MS: i64 = 30_000;
pub const DEFAULT_WRITER_LEASE_HEARTBEAT_MS: i64 = 10_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriterLease {
    pub owner_id: String,
    pub fence: i64,
    pub expires_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WriterLeaseOptions {
    #[serde(default = "default_ttl")]
    pub ttl_ms: i64,
    #[serde(default = "default_heartbeat")]
    pub heartbeat_interval_ms: i64,
}

fn default_ttl() -> i64 {
    DEFAULT_WRITER_LEASE_TTL_MS
}

fn default_heartbeat() -> i64 {
    DEFAULT_WRITER_LEASE_HEARTBEAT_MS
}

impl Default for WriterLeaseOptions {
    fn default() -> Self {
        Self {
            ttl_ms: DEFAULT_WRITER_LEASE_TTL_MS,
            heartbeat_interval_ms: DEFAULT_WRITER_LEASE_HEARTBEAT_MS,
        }
    }
}

impl WriterLeaseOptions {
    pub fn validate(&self) -> Result<(), String> {
        if self.ttl_ms <= 0 {
            return Err("writerLease.ttlMs must be positive".into());
        }
        if self.heartbeat_interval_ms <= 0 || self.heartbeat_interval_ms >= self.ttl_ms {
            return Err(
                "writerLease.heartbeatIntervalMs must be positive and less than ttlMs".into(),
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionMetadata {
    pub id: String,
    pub created_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub session_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelRef {
    pub provider: String,
    pub id: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThinkingLevel {
    Off,
    Minimal,
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SessionPhase {
    Idle,
    Turn,
    Compaction,
    BranchSummary,
    Retry,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    System,
    User,
    Assistant,
    Tool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub arguments: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    pub id: String,
    pub session_id: String,
    pub role: Role,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
    pub timestamp: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StopReason {
    Stop,
    Length,
    ToolUse,
    Error,
    Aborted,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Usage {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
    pub cache_write: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total_tokens: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost: Option<UsageCost>,
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Entry {
    #[serde(rename_all = "camelCase")]
    Message {
        id: String,
        seq: i64,
        #[serde(skip_serializing_if = "Option::is_none")]
        parent_id: Option<String>,
        timestamp: i64,
        message: AgentMessage,
        #[serde(skip_serializing_if = "Option::is_none")]
        terminate: Option<bool>,
    },
    #[serde(rename_all = "camelCase")]
    ModelChange {
        id: String,
        seq: i64,
        #[serde(skip_serializing_if = "Option::is_none")]
        parent_id: Option<String>,
        timestamp: i64,
        provider: String,
        model_id: String,
    },
    #[serde(rename_all = "camelCase")]
    Custom {
        id: String,
        seq: i64,
        #[serde(skip_serializing_if = "Option::is_none")]
        parent_id: Option<String>,
        timestamp: i64,
        custom_type: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        data: Option<Value>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentMessage {
    pub role: Role,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_calls: Option<Vec<ToolCall>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool_call_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AgentEvent {
    #[serde(rename_all = "camelCase")]
    Started { session_id: String, timestamp: i64 },
    #[serde(rename_all = "camelCase")]
    MessageStart {
        session_id: String,
        message_id: String,
    },
    #[serde(rename_all = "camelCase")]
    MessageUpdate {
        session_id: String,
        message_id: String,
        chunk: String,
    },
    #[serde(rename_all = "camelCase")]
    MessageEnd {
        session_id: String,
        message_id: String,
    },
    #[serde(rename_all = "camelCase")]
    ToolCallStart {
        session_id: String,
        tool_call: ToolCall,
    },
    #[serde(rename_all = "camelCase")]
    ToolExecutionEnd {
        session_id: String,
        tool_call_id: String,
        result: String,
    },
    #[serde(rename_all = "camelCase")]
    TurnEnd { session_id: String },
    #[serde(rename_all = "camelCase")]
    Completed { session_id: String, timestamp: i64 },
    #[serde(rename_all = "camelCase")]
    Error { session_id: String, error: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AssistantMessageEvent {
    Start { message_id: String },
    TextDelta { message_id: String, delta: String },
    ThinkingDelta { message_id: String, delta: String },
    ToolCallStart { id: String, name: String },
    ToolCallDelta { id: String, delta: String },
    ToolCallEnd { id: String, arguments: String },
    Done { stop_reason: StopReason },
    Error { message: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionLeaseMode {
    Shared,
    Exclusive,
}
