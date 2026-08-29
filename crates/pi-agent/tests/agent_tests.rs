use pi_agent::agent::{Agent, AgentTool, AgentToolResult};
use pi_agent::compaction::{should_compact, summarize_conversation, DEFAULT_COMPACTION_SETTINGS};
use pi_ai::models::default_anthropic_model;
use pi_ai::types::UserContent;
use serde_json::json;
use std::sync::Arc;

struct EchoTool;

#[async_trait::async_trait]
impl AgentTool for EchoTool {
    fn name(&self) -> &str {
        "echo"
    }
    fn description(&self) -> &str {
        "Echo text"
    }
    fn parameters(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": { "text": { "type": "string" } } })
    }
    async fn execute(
        &self,
        _tool_call_id: &str,
        params: serde_json::Value,
    ) -> Result<AgentToolResult, String> {
        let text = params.get("text").and_then(|t| t.as_str()).unwrap_or("");
        Ok(AgentToolResult {
            content: vec![UserContent::Text(pi_ai::types::TextContent {
                content_type: "text".to_string(),
                text: text.to_string(),
                text_signature: None,
            })],
            details: json!({ "echo": text }),
            usage: None,
            added_tool_names: None,
            terminate: None,
        })
    }
}

#[tokio::test]
async fn test_agent_tool_execution() {
    let tool = EchoTool;
    let res = tool
        .execute("call-1", json!({ "text": "hello world" }))
        .await
        .expect("tool execution");
    assert_eq!(res.content.len(), 1);
}

#[test]
fn test_agent_prompt_and_queues() {
    let mut agent = Agent::new(default_anthropic_model());
    agent.add_tool(Arc::new(EchoTool));

    agent.prompt("Hello");
    agent.steer("Do this instead");
    agent.follow_up("Then do that");

    assert_eq!(agent.messages.len(), 1);
    assert_eq!(agent.steering_queue.len(), 1);
    assert_eq!(agent.follow_up_queue.len(), 1);
}

#[test]
fn test_agent_compaction_logic() {
    let settings = DEFAULT_COMPACTION_SETTINGS;
    assert!(!should_compact(50_000, &settings));
    assert!(should_compact(150_000, &settings));

    let summary = summarize_conversation(&[]);
    assert!(summary.contains("Summary"));
}
