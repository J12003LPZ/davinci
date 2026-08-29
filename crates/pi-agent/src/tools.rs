use async_trait::async_trait;
use pi_ai::{ContentBlock, ImageContent, TextContent, Tool, Usage};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentToolResult {
    pub content: Vec<ContentBlock>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub details: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
    #[serde(rename = "isError")]
    pub is_error: bool,
    #[serde(rename = "addedToolNames", skip_serializing_if = "Option::is_none")]
    pub added_tool_names: Option<Vec<String>>,
}

impl AgentToolResult {
    pub fn text(text: impl Into<String>) -> Self {
        Self {
            content: vec![ContentBlock::Text(TextContent::new(text))],
            details: None,
            usage: None,
            is_error: false,
            added_tool_names: None,
        }
    }

    pub fn error(message: impl Into<String>) -> Self {
        Self {
            content: vec![ContentBlock::Text(TextContent::new(message))],
            details: None,
            usage: None,
            is_error: true,
            added_tool_names: None,
        }
    }

    pub fn image(data: impl Into<String>, mime_type: impl Into<String>) -> Self {
        Self {
            content: vec![ContentBlock::Image(ImageContent {
                content_type: "image".to_string(),
                data: data.into(),
                mime_type: mime_type.into(),
            })],
            details: None,
            usage: None,
            is_error: false,
            added_tool_names: None,
        }
    }
}

#[async_trait]
pub trait AgentTool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters(&self) -> serde_json::Value;
    fn requires_permission(&self) -> bool {
        false
    }
    async fn execute(
        &self,
        tool_call_id: &str,
        arguments: &serde_json::Value,
    ) -> crate::error::Result<AgentToolResult>;

    fn to_tool_def(&self) -> Tool {
        Tool {
            name: self.name().to_string(),
            description: self.description().to_string(),
            parameters: self.parameters(),
            constrained_sampling: None,
        }
    }
}
