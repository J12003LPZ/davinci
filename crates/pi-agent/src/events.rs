use pi_ai::{AssistantMessageEvent, ChatMessage};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Events emitted by the Agent, locked to `vendor/pi/packages/agent/src/types.ts`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AgentEvent {
    #[serde(rename = "agent_start")]
    AgentStart,
    #[serde(rename = "agent_end")]
    AgentEnd {
        messages: Vec<ChatMessage>,
        #[serde(rename = "willRetry", default)]
        will_retry: bool,
    },
    #[serde(rename = "turn_start")]
    TurnStart,
    #[serde(rename = "turn_end")]
    TurnEnd {
        message: ChatMessage,
        #[serde(rename = "toolResults")]
        tool_results: Vec<ChatMessage>,
    },
    #[serde(rename = "message_start")]
    MessageStart { message: ChatMessage },
    #[serde(rename = "message_update")]
    MessageUpdate {
        // Arc, not a value: one update fires per stream delta and all of them
        // share the same final message; a value clone per delta made a long
        // response O(n^2) in retained memory.
        message: std::sync::Arc<ChatMessage>,
        #[serde(rename = "assistantMessageEvent")]
        assistant_message_event: AssistantMessageEvent,
    },
    #[serde(rename = "message_end")]
    MessageEnd { message: ChatMessage },
    #[serde(rename = "tool_execution_start")]
    ToolExecutionStart {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        args: Value,
    },
    #[serde(rename = "tool_execution_update")]
    ToolExecutionUpdate {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        args: Value,
        #[serde(rename = "partialResult")]
        partial_result: Value,
    },
    #[serde(rename = "tool_execution_end")]
    ToolExecutionEnd {
        #[serde(rename = "toolCallId")]
        tool_call_id: String,
        #[serde(rename = "toolName")]
        tool_name: String,
        result: Value,
        #[serde(rename = "isError")]
        is_error: bool,
        /// The tool's `details` — an edit's diff, a job's id, a read's
        /// truncation — minus any image payload, so a host can draw the
        /// change without re-reading the file. Absent when the tool gave
        /// none.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        details: Option<Value>,
    },
    #[serde(rename = "auto_retry_start")]
    AutoRetryStart {
        attempt: u32,
        #[serde(rename = "maxAttempts")]
        max_attempts: u32,
        #[serde(rename = "delayMs")]
        delay_ms: u64,
        #[serde(rename = "errorMessage")]
        error_message: String,
    },
    #[serde(rename = "auto_retry_end")]
    AutoRetryEnd {
        success: bool,
        attempt: u32,
        #[serde(rename = "finalError", skip_serializing_if = "Option::is_none")]
        final_error: Option<String>,
    },
}

impl AgentEvent {
    pub fn kind(&self) -> &'static str {
        match self {
            Self::AgentStart => "agent_start",
            Self::AgentEnd { .. } => "agent_end",
            Self::TurnStart => "turn_start",
            Self::TurnEnd { .. } => "turn_end",
            Self::MessageStart { .. } => "message_start",
            Self::MessageUpdate { .. } => "message_update",
            Self::MessageEnd { .. } => "message_end",
            Self::ToolExecutionStart { .. } => "tool_execution_start",
            Self::ToolExecutionUpdate { .. } => "tool_execution_update",
            Self::ToolExecutionEnd { .. } => "tool_execution_end",
            Self::AutoRetryStart { .. } => "auto_retry_start",
            Self::AutoRetryEnd { .. } => "auto_retry_end",
        }
    }
}
