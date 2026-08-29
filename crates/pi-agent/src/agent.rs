use crate::types::*;
use async_trait::async_trait;
use pi_ai::types::*;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentToolResult {
    pub content: Vec<UserContent>,
    pub details: serde_json::Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub added_tool_names: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub terminate: Option<bool>,
}

#[async_trait]
pub trait AgentTool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters(&self) -> serde_json::Value;
    async fn execute(
        &self,
        tool_call_id: &str,
        params: serde_json::Value,
    ) -> Result<AgentToolResult, String>;
}

pub struct Agent {
    pub system_prompt: String,
    pub model: Model,
    pub thinking_level: ThinkingLevel,
    pub tools: Vec<Arc<dyn AgentTool>>,
    pub messages: Vec<AgentMessage>,
    pub steering_queue: Vec<AgentMessage>,
    pub follow_up_queue: Vec<AgentMessage>,
    pub steering_mode: QueueMode,
    pub follow_up_mode: QueueMode,
    pub is_streaming: bool,
}

impl Agent {
    pub fn new(model: Model) -> Self {
        Self {
            system_prompt: String::new(),
            model,
            thinking_level: ThinkingLevel::Off,
            tools: Vec::new(),
            messages: Vec::new(),
            steering_queue: Vec::new(),
            follow_up_queue: Vec::new(),
            steering_mode: QueueMode::OneAtATime,
            follow_up_mode: QueueMode::OneAtATime,
            is_streaming: false,
        }
    }

    pub fn add_tool(&mut self, tool: Arc<dyn AgentTool>) {
        self.tools.push(tool);
    }

    pub fn prompt(&mut self, text: &str) {
        let msg = AgentMessage::Llm(Message::User(UserMessage {
            role: "user".to_string(),
            content: vec![UserContent::Text(TextContent {
                content_type: "text".to_string(),
                text: text.to_string(),
                text_signature: None,
            })],
            timestamp: chrono::Utc::now().timestamp_millis(),
        }));
        self.messages.push(msg);
    }

    pub fn steer(&mut self, text: &str) {
        let msg = AgentMessage::Llm(Message::User(UserMessage {
            role: "user".to_string(),
            content: vec![UserContent::Text(TextContent {
                content_type: "text".to_string(),
                text: text.to_string(),
                text_signature: None,
            })],
            timestamp: chrono::Utc::now().timestamp_millis(),
        }));
        self.steering_queue.push(msg);
    }

    pub fn follow_up(&mut self, text: &str) {
        let msg = AgentMessage::Llm(Message::User(UserMessage {
            role: "user".to_string(),
            content: vec![UserContent::Text(TextContent {
                content_type: "text".to_string(),
                text: text.to_string(),
                text_signature: None,
            })],
            timestamp: chrono::Utc::now().timestamp_millis(),
        }));
        self.follow_up_queue.push(msg);
    }
}
