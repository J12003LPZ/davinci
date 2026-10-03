use serde::{Deserialize, Serialize};
use serde_json::Value as JsonValue;

pub const PROTOCOL_VERSION: u32 = 1;

pub type Timestamp = u64;

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

impl ThinkingLevel {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Minimal => "minimal",
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::Xhigh => "xhigh",
            Self::Max => "max",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "off" => Some(Self::Off),
            "minimal" => Some(Self::Minimal),
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            "xhigh" => Some(Self::Xhigh),
            "max" => Some(Self::Max),
            _ => None,
        }
    }

    pub fn all() -> &'static [Self] {
        &[
            Self::Off,
            Self::Minimal,
            Self::Low,
            Self::Medium,
            Self::High,
            Self::Xhigh,
            Self::Max,
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionPhase {
    Idle,
    Turn,
    Compaction,
    BranchSummary,
    Retry,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelRef {
    pub provider: String,
    pub id: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelCost {
    pub input: f64,
    pub output: f64,
    #[serde(rename = "cacheRead")]
    pub cache_read: f64,
    #[serde(rename = "cacheWrite")]
    pub cache_write: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelMetadata {
    pub provider: String,
    pub id: String,
    pub name: String,
    pub api: String,
    pub reasoning: bool,
    pub input: Vec<String>,
    #[serde(rename = "contextWindow")]
    pub context_window: u64,
    #[serde(rename = "maxTokens")]
    pub max_tokens: u64,
    pub cost: ModelCost,
    #[serde(rename = "supportedThinkingLevels")]
    pub supported_thinking_levels: Vec<ThinkingLevel>,
    pub authenticated: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum TextOrImage {
    Text { text: String },
    Image { data: String, mime_type: String },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum AssistantContent {
    Text {
        text: String,
    },
    Thinking {
        thinking: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        redacted: Option<bool>,
    },
    #[serde(rename = "toolCall")]
    ToolCall {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        input: JsonValue,
    },
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct Usage {
    pub input: u64,
    pub output: u64,
    #[serde(rename = "cacheRead")]
    pub cache_read: u64,
    #[serde(rename = "cacheWrite")]
    pub cache_write: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning: Option<u64>,
    #[serde(rename = "totalTokens")]
    pub total_tokens: u64,
    pub cost: UsageCost,
}

impl Usage {
    pub fn from_tokens(
        input: u64,
        output: u64,
        cache_read: u64,
        cache_write: u64,
        cost: &ModelCost,
    ) -> Self {
        let cost = UsageCost {
            input: (input as f64 / 1_000_000.0) * cost.input,
            output: (output as f64 / 1_000_000.0) * cost.output,
            cache_read: (cache_read as f64 / 1_000_000.0) * cost.cache_read,
            cache_write: (cache_write as f64 / 1_000_000.0) * cost.cache_write,
            total: 0.0,
        };
        let mut usage = Self {
            input,
            output,
            cache_read,
            cache_write,
            reasoning: None,
            total_tokens: input + output + cache_read + cache_write,
            cost,
        };
        usage.cost.total =
            usage.cost.input + usage.cost.output + usage.cost.cache_read + usage.cost.cache_write;
        usage
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "role")]
pub enum TranscriptItem {
    #[serde(rename = "user")]
    User {
        id: String,
        content: Vec<TextOrImage>,
        timestamp: Timestamp,
    },
    #[serde(rename = "assistant")]
    Assistant {
        id: String,
        content: Vec<AssistantContent>,
        model: ModelRef,
        #[serde(rename = "responseModel", skip_serializing_if = "Option::is_none")]
        response_model: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        usage: Option<Usage>,
        timestamp: Timestamp,
        status: String,
        #[serde(rename = "stopReason", skip_serializing_if = "Option::is_none")]
        stop_reason: Option<String>,
        #[serde(rename = "errorMessage", skip_serializing_if = "Option::is_none")]
        error_message: Option<String>,
    },
    #[serde(rename = "tool")]
    Tool {
        id: String,
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        input: JsonValue,
        content: Vec<TextOrImage>,
        #[serde(skip_serializing_if = "Option::is_none")]
        details: Option<JsonValue>,
        #[serde(skip_serializing_if = "Option::is_none")]
        usage: Option<Usage>,
        timestamp: Timestamp,
        status: String,
        #[serde(rename = "isError")]
        is_error: bool,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum TranscriptProgress {
    #[serde(rename = "item_started")]
    ItemStarted { item: TranscriptItem },
    #[serde(rename = "assistant_delta")]
    AssistantDelta {
        #[serde(rename = "messageId")]
        message_id: String,
        #[serde(rename = "contentIndex")]
        content_index: u64,
        kind: String,
        delta: String,
    },
    #[serde(rename = "item_updated")]
    ItemUpdated { item: TranscriptItem },
    #[serde(rename = "item_finished")]
    ItemFinished { item: TranscriptItem },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionMetadata {
    pub id: String,
    #[serde(rename = "createdAt")]
    pub created_at: Timestamp,
    #[serde(rename = "updatedAt", skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<Timestamp>,
    #[serde(rename = "parentSessionId", skip_serializing_if = "Option::is_none")]
    pub parent_session_id: Option<String>,
    #[serde(rename = "sessionName", skip_serializing_if = "Option::is_none")]
    pub session_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionSnapshot {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    pub cwd: String,
    #[serde(rename = "createdAt")]
    pub created_at: Timestamp,
    #[serde(rename = "updatedAt")]
    pub updated_at: Timestamp,
    pub phase: SessionPhase,
    pub model: ModelRef,
    #[serde(rename = "thinkingLevel")]
    pub thinking_level: ThinkingLevel,
    pub attached: bool,
    pub locked: bool,
    pub revision: u64,
    pub transcript: Vec<TranscriptItem>,
    #[serde(rename = "queuedSteer")]
    pub queued_steer: Vec<TranscriptItem>,
    #[serde(rename = "queuedSteerCount")]
    pub queued_steer_count: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ServerSnapshot {
    #[serde(rename = "serverId")]
    pub server_id: String,
    #[serde(rename = "protocolVersion")]
    pub protocol_version: u32,
    pub revision: u64,
    pub sessions: Vec<SessionMetadata>,
    pub models: Vec<ModelMetadata>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProtocolErrorCode {
    Version,
    Busy,
    SessionLocked,
    NotFound,
    InvalidRequest,
    NotImplemented,
    InternalError,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProtocolError {
    pub code: ProtocolErrorCode,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<JsonValue>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command")]
pub enum Command {
    #[serde(rename = "list")]
    List,
    #[serde(rename = "create")]
    Create {
        #[serde(skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        model: Option<ModelRef>,
        #[serde(rename = "thinkingLevel", skip_serializing_if = "Option::is_none")]
        thinking_level: Option<ThinkingLevel>,
    },
    #[serde(rename = "attach")]
    Attach {
        #[serde(rename = "sessionId")]
        session_id: String,
    },
    #[serde(rename = "detach")]
    Detach {
        #[serde(rename = "sessionId")]
        session_id: String,
    },
    #[serde(rename = "prompt")]
    Prompt {
        #[serde(rename = "sessionId")]
        session_id: String,
        text: String,
    },
    #[serde(rename = "steer")]
    Steer {
        #[serde(rename = "sessionId")]
        session_id: String,
        text: String,
    },
    #[serde(rename = "abort")]
    Abort {
        #[serde(rename = "sessionId")]
        session_id: String,
    },
    #[serde(rename = "set_model")]
    SetModel {
        #[serde(rename = "sessionId")]
        session_id: String,
        model: ModelRef,
    },
    #[serde(rename = "set_thinking")]
    SetThinking {
        #[serde(rename = "sessionId")]
        session_id: String,
        #[serde(rename = "thinkingLevel")]
        thinking_level: ThinkingLevel,
    },
}

impl Command {
    pub fn name(&self) -> &'static str {
        match self {
            Self::List => "list",
            Self::Create { .. } => "create",
            Self::Attach { .. } => "attach",
            Self::Detach { .. } => "detach",
            Self::Prompt { .. } => "prompt",
            Self::Steer { .. } => "steer",
            Self::Abort { .. } => "abort",
            Self::SetModel { .. } => "set_model",
            Self::SetThinking { .. } => "set_thinking",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "command")]
pub enum CommandResult {
    #[serde(rename = "list")]
    List { sessions: Vec<SessionMetadata> },
    #[serde(rename = "create")]
    Create { session: SessionSnapshot },
    #[serde(rename = "attach")]
    Attach { session: SessionSnapshot },
    #[serde(rename = "detach")]
    Detach {
        #[serde(rename = "sessionId")]
        session_id: String,
    },
    #[serde(rename = "prompt")]
    Prompt { session: SessionSnapshot },
    #[serde(rename = "steer")]
    Steer { session: SessionSnapshot },
    #[serde(rename = "abort")]
    Abort { session: SessionSnapshot },
    #[serde(rename = "set_model")]
    SetModel { session: SessionSnapshot },
    #[serde(rename = "set_thinking")]
    SetThinking { session: SessionSnapshot },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ClientMessage {
    #[serde(rename = "hello")]
    Hello { version: u32 },
    #[serde(rename = "request")]
    Request { id: String, request: Command },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ServerEvent {
    #[serde(rename = "server_snapshot")]
    ServerSnapshot { snapshot: ServerSnapshot },
    #[serde(rename = "session_snapshot")]
    SessionSnapshot { snapshot: SessionSnapshot },
    #[serde(rename = "session_progress")]
    SessionProgress {
        #[serde(rename = "sessionId")]
        session_id: String,
        progress: TranscriptProgress,
    },
    #[serde(rename = "session_removed")]
    SessionRemoved {
        #[serde(rename = "sessionId")]
        session_id: String,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ServerMessage {
    #[serde(rename = "hello")]
    Hello {
        version: u32,
        #[serde(rename = "connectionId")]
        connection_id: String,
        snapshot: ServerSnapshot,
    },
    #[serde(rename = "hello_error")]
    HelloError { error: ProtocolError },
    #[serde(rename = "response")]
    Response {
        id: String,
        ok: bool,
        #[serde(skip_serializing_if = "Option::is_none")]
        result: Option<CommandResult>,
        #[serde(skip_serializing_if = "Option::is_none")]
        error: Option<ProtocolError>,
    },
    #[serde(rename = "event")]
    Event { event: ServerEvent },
}

pub fn default_model_ref() -> ModelRef {
    ModelRef {
        provider: "google".to_string(),
        id: "gemini-3-flash".to_string(),
    }
}
