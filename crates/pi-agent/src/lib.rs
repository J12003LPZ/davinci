//! Agent runtime matching `@earendil-works/pi-agent-core`.

mod compaction;
mod context;
mod queues;
mod skills;
mod templates;
mod tools;

pub use compaction::{compact_messages, CompactionResult};
pub use context::{load_context_files, ContextFile};
pub use queues::{QueueMode, QueuedMessage, SteerFollowUpQueues};
pub use skills::{discover_skills, Skill};
pub use templates::{discover_prompt_templates, PromptTemplate};
pub use tools::{execute_tool, tool_specs, AgentTool, ToolError, ToolResult, BUILTIN_TOOLS};

use pi_ai::{content_text, ChatMessage, MessageContent};
use pi_protocol::ThinkingLevel;
use pi_session::{JsonlSession, SessionEntry};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolExecutionMode {
    Sequential,
    Parallel,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentEvent {
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<ChatMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct Agent {
    pub system_prompt: String,
    pub messages: Vec<ChatMessage>,
    pub thinking_level: ThinkingLevel,
    pub auto_compaction: bool,
    pub auto_retry: bool,
    pub retry_attempts: u32,
    pub queues: SteerFollowUpQueues,
    pub tools: Vec<String>,
    pub skills: Vec<Skill>,
    pub templates: Vec<PromptTemplate>,
    pub context_files: Vec<ContextFile>,
    pub session: Option<JsonlSession>,
}

impl Agent {
    pub fn new(system_prompt: impl Into<String>) -> Self {
        Self {
            system_prompt: system_prompt.into(),
            messages: Vec::new(),
            thinking_level: ThinkingLevel::Off,
            auto_compaction: true,
            auto_retry: true,
            retry_attempts: 3,
            queues: SteerFollowUpQueues::default(),
            tools: BUILTIN_TOOLS.iter().map(|t| t.to_string()).collect(),
            skills: Vec::new(),
            templates: Vec::new(),
            context_files: Vec::new(),
            session: None,
        }
    }

    pub fn prompt(&mut self, text: &str) -> ChatMessage {
        let message = ChatMessage {
            role: "user".into(),
            content: vec![MessageContent::Text {
                text: text.to_string(),
            }],
            tool_call_id: None,
        };
        self.messages.push(message.clone());
        if let Some(session) = &mut self.session {
            let _ = session.append_entry(SessionEntry::message(
                "user",
                serde_json::json!([{"type":"text","text": text}]),
            ));
        }
        message
    }

    pub fn record_assistant(&mut self, text: &str) {
        let message = ChatMessage {
            role: "assistant".into(),
            content: vec![MessageContent::Text {
                text: text.to_string(),
            }],
            tool_call_id: None,
        };
        self.messages.push(message);
        if let Some(session) = &mut self.session {
            let _ = session.append_entry(SessionEntry::message(
                "assistant",
                serde_json::json!([{"type":"text","text": text}]),
            ));
        }
    }

    pub fn last_assistant_text(&self) -> Option<String> {
        self.messages.iter().rev().find_map(|message| {
            if message.role == "assistant" {
                Some(content_text(&message.content))
            } else {
                None
            }
        })
    }

    pub fn apply_extension_tools(&mut self, names: &[String]) {
        for name in names {
            if !self.tools.contains(name) {
                self.tools.push(name.clone());
            }
        }
    }

    pub fn compact(&mut self, custom_instructions: Option<&str>) -> CompactionResult {
        let result = compact_messages(&self.messages, custom_instructions);
        self.messages = result.messages.clone();
        result
    }
}

pub fn default_system_prompt() -> String {
    "You are pi, a coding assistant with read, bash, edit, and write tools. Be concise and make precise edits.".into()
}

pub fn new_message_id() -> String {
    Uuid::new_v4().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queues_and_compaction_match_ts_modes() {
        let mut agent = Agent::new(default_system_prompt());
        agent.prompt("aaaaaaaaaa bbbbbbbbbb");
        agent.record_assistant("cccccccccc");
        agent.queues.enqueue_steer("steer me");
        agent.queues.enqueue_follow_up("follow");
        assert_eq!(agent.queues.steer.len(), 1);
        let drained = agent.queues.drain_steer(QueueMode::OneAtATime);
        assert_eq!(drained.len(), 1);
        let compacted = agent.compact(Some("keep decisions"));
        assert!(compacted.summary.contains("keep decisions"));
        assert!(!agent.messages.is_empty());
    }
}
