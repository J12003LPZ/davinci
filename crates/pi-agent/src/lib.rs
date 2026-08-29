pub mod agent;
pub mod compaction;
pub mod error;
pub mod events;
pub mod permission;
pub mod prompt_templates;
pub mod skills;
pub mod tools;

pub use agent::*;
pub use compaction::*;
pub use error::{Error, Result};
pub use events::*;
pub use permission::*;
pub use prompt_templates::*;
pub use skills::*;
pub use tools::*;

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use pi_ai::{Message, Model, ModelCost, UserContent, UserMessage};
    use std::sync::Arc;
    use tokio::sync::mpsc;

    struct DummyEchoTool;

    #[async_trait]
    impl AgentTool for DummyEchoTool {
        fn name(&self) -> &str {
            "echo"
        }
        fn description(&self) -> &str {
            "Echo tool"
        }
        fn parameters(&self) -> serde_json::Value {
            serde_json::json!({ "type": "object", "properties": { "msg": { "type": "string" } } })
        }
        async fn execute(
            &self,
            _tool_call_id: &str,
            arguments: &serde_json::Value,
        ) -> Result<AgentToolResult> {
            let msg = arguments.get("msg").and_then(|v| v.as_str()).unwrap_or("");
            Ok(AgentToolResult::text(format!("Echo: {}", msg)))
        }
    }

    #[tokio::test]
    async fn test_agent_run_faux() {
        let model = Model {
            id: "faux-1".to_string(),
            name: "Faux Model".to_string(),
            api: "faux".to_string(),
            provider: "faux".to_string(),
            base_url: "http://localhost:0".to_string(),
            reasoning: false,
            thinking_level_map: None,
            input: vec!["text".to_string()],
            cost: ModelCost::default(),
            context_window: 10000,
            max_tokens: 1000,
            sampling_params: None,
            headers: None,
            compat: None,
        };

        let config = AgentConfig::new(model).with_tools(vec![Arc::new(DummyEchoTool)]);
        let runtime = AgentRuntime::new(config);

        let (tx, mut rx) = mpsc::unbounded_channel();
        let initial_messages = vec![Message::User(UserMessage {
            role: "user".to_string(),
            content: UserContent::Text("Hello".to_string()),
            timestamp: 1000,
        })];

        let handle = tokio::spawn(async move { runtime.run(initial_messages, tx).await });

        let mut events = Vec::new();
        while let Some(evt) = rx.recv().await {
            events.push(evt);
        }

        let res = handle.await.unwrap();
        assert!(res.is_ok());
        assert!(!events.is_empty());
    }
}
