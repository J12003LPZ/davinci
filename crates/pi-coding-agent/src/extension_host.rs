use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Extension event bus matching `vendor/pi/packages/coding-agent/src/core/extensions`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ExtensionEvent {
    #[serde(rename = "before_agent_start")]
    BeforeAgentStart,
    #[serde(rename = "agent_end")]
    AgentEnd,
    #[serde(rename = "tool_call")]
    ToolCall {
        #[serde(rename = "toolName")]
        tool_name: String,
        args: Value,
    },
    #[serde(rename = "tool_result")]
    ToolResult {
        #[serde(rename = "toolName")]
        tool_name: String,
        #[serde(rename = "isError")]
        is_error: bool,
    },
    #[serde(rename = "session_before_compact")]
    SessionBeforeCompact,
    #[serde(rename = "session_shutdown")]
    SessionShutdown,
}

#[derive(Debug, Default)]
pub struct ExtensionHost {
    pub events: Vec<ExtensionEvent>,
}

impl ExtensionHost {
    pub fn emit(&mut self, event: ExtensionEvent) {
        self.events.push(event);
    }

    pub fn kinds(&self) -> Vec<&'static str> {
        self.events
            .iter()
            .map(|event| match event {
                ExtensionEvent::BeforeAgentStart => "before_agent_start",
                ExtensionEvent::AgentEnd => "agent_end",
                ExtensionEvent::ToolCall { .. } => "tool_call",
                ExtensionEvent::ToolResult { .. } => "tool_result",
                ExtensionEvent::SessionBeforeCompact => "session_before_compact",
                ExtensionEvent::SessionShutdown => "session_shutdown",
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_names_match_ts() {
        let mut host = ExtensionHost::default();
        host.emit(ExtensionEvent::BeforeAgentStart);
        host.emit(ExtensionEvent::ToolCall {
            tool_name: "read".into(),
            args: serde_json::json!({}),
        });
        assert_eq!(host.kinds(), ["before_agent_start", "tool_call"]);
    }
}
