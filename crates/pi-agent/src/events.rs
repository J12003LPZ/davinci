use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct AgentMessage {
    pub role: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub images: Vec<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type")]
pub enum AgentEvent {
    #[serde(rename = "agent_start")]
    AgentStart,
    #[serde(rename = "turn_start")]
    TurnStart { turn: u32 },
    #[serde(rename = "message")]
    Message { message: AgentMessage },
    #[serde(rename = "tool_start")]
    ToolStart { name: String, id: String },
    #[serde(rename = "tool_end")]
    ToolEnd {
        name: String,
        id: String,
        is_error: bool,
        output: String,
    },
    #[serde(rename = "usage")]
    Usage { usage: pi_ai::Usage },
    #[serde(rename = "compaction")]
    Compaction { summary: String },
    #[serde(rename = "retry")]
    Retry { attempt: u32, message: String },
    #[serde(rename = "agent_end")]
    AgentEnd,
    #[serde(rename = "error")]
    Error { message: String },
}
