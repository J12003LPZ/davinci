use pi_ai::types::*;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum CustomAgentMessage {
    #[serde(rename = "custom")]
    Custom { content: String, timestamp: i64 },
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum AgentMessage {
    Llm(Message),
    Custom(CustomAgentMessage),
}

impl AgentMessage {
    pub fn role(&self) -> &str {
        match self {
            AgentMessage::Llm(Message::User(_)) => "user",
            AgentMessage::Llm(Message::Assistant(_)) => "assistant",
            AgentMessage::Llm(Message::ToolResult(_)) => "toolResult",
            AgentMessage::Custom(_) => "custom",
        }
    }

    pub fn timestamp(&self) -> i64 {
        match self {
            AgentMessage::Llm(Message::User(u)) => u.timestamp,
            AgentMessage::Llm(Message::Assistant(a)) => a.timestamp,
            AgentMessage::Llm(Message::ToolResult(t)) => t.timestamp,
            AgentMessage::Custom(CustomAgentMessage::Custom { timestamp, .. }) => *timestamp,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum AgentEvent {
    #[serde(rename = "agent_start")]
    AgentStart,
    #[serde(rename = "agent_end")]
    AgentEnd { messages: Vec<AgentMessage> },
    #[serde(rename = "turn_start")]
    TurnStart,
    #[serde(rename = "turn_end")]
    TurnEnd {
        message: AgentMessage,
        tool_results: Vec<ToolResultMessage>,
    },
    #[serde(rename = "message_start")]
    MessageStart { message: AgentMessage },
    #[serde(rename = "message_update")]
    MessageUpdate {
        message: AgentMessage,
        assistant_message_event: AssistantMessageEvent,
    },
    #[serde(rename = "message_end")]
    MessageEnd { message: AgentMessage },
    #[serde(rename = "tool_execution_start")]
    ToolExecutionStart {
        tool_call_id: String,
        tool_name: String,
        args: serde_json::Value,
    },
    #[serde(rename = "tool_execution_update")]
    ToolExecutionUpdate {
        tool_call_id: String,
        tool_name: String,
        args: serde_json::Value,
        partial_result: serde_json::Value,
    },
    #[serde(rename = "tool_execution_end")]
    ToolExecutionEnd {
        tool_call_id: String,
        tool_name: String,
        result: serde_json::Value,
        is_error: bool,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum QueueMode {
    All,
    OneAtATime,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolExecutionMode {
    Sequential,
    Parallel,
}
