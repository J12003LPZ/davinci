//! Agent runtime matching `@earendil-works/pi-agent-core`.

mod compaction;
mod context;
mod events;
mod queues;
mod skills;
mod templates;
mod tools;
mod turn;

pub use compaction::{
    build_history_prompt, calculate_context_tokens, compact_messages, compact_messages_with,
    compact_messages_with_options, compaction_context_message, compute_file_lists, convert_to_llm,
    env_summarizer, estimate_context_tokens, estimate_tokens, extract_file_ops, find_cut_point,
    format_file_operations, generate_summary_with_usage, get_summarization_failure,
    serialize_conversation, should_compact, CompactionDetails, CompactionResult,
    CompactionSettings, CutPointResult, FileOperations, SummarizeRequest, SummarizeResponse,
    Summarizer, BRANCH_SUMMARY_PREFIX, BRANCH_SUMMARY_SUFFIX, COMPACTION_SUMMARY_PREFIX,
    COMPACTION_SUMMARY_SUFFIX, DEFAULT_KEEP_RECENT_TOKENS, DEFAULT_RESERVE_TOKENS,
    SUMMARIZATION_PROMPT, SUMMARIZATION_SYSTEM_PROMPT, TURN_PREFIX_SUMMARIZATION_PROMPT,
    UPDATE_SUMMARIZATION_PROMPT,
};
pub use context::{load_context_files, ContextFile};
pub use events::AgentEvent;
pub use queues::{QueueMode, QueuedMessage, SteerFollowUpQueues};
pub use skills::{discover_skills, Skill};
pub use templates::{discover_prompt_templates, PromptTemplate};
pub use tools::{execute_tool, tool_specs, AgentTool, ToolError, ToolResult, BUILTIN_TOOLS};
pub use turn::retry_delay_ms;

use pi_ai::{content_text, ChatMessage, ThinkingBudgets};
use pi_protocol::ThinkingLevel;
use pi_session::{JsonlSession, SessionEntry};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;

type CustomToolFn = dyn Fn(&Path, &str, &Value) -> Result<ToolResult, ToolError> + Send + Sync;

/// Injected tool runner for JS/manifest tools so `pi-agent` stays independent of the coding-agent host.
#[derive(Clone)]
pub struct CustomToolExecutor {
    inner: Arc<CustomToolFn>,
}

impl std::fmt::Debug for CustomToolExecutor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("CustomToolExecutor")
    }
}

impl CustomToolExecutor {
    pub fn new<F>(f: F) -> Self
    where
        F: Fn(&Path, &str, &Value) -> Result<ToolResult, ToolError> + Send + Sync + 'static,
    {
        Self { inner: Arc::new(f) }
    }

    pub fn execute(&self, cwd: &Path, name: &str, args: &Value) -> Result<ToolResult, ToolError> {
        (self.inner)(cwd, name, args)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolExecutionMode {
    Sequential,
    Parallel,
}

#[derive(Debug, Clone)]
pub struct Agent {
    pub system_prompt: String,
    pub messages: Vec<ChatMessage>,
    pub thinking_level: ThinkingLevel,
    pub auto_compaction: bool,
    pub compaction: CompactionSettings,
    pub auto_retry: bool,
    pub retry_attempts: u32,
    pub retry_base_delay_ms: u64,
    pub provider_timeout_ms: Option<u64>,
    pub provider_max_retries: Option<u32>,
    pub provider_max_retry_delay_ms: u64,
    pub thinking_budgets: Option<ThinkingBudgets>,
    pub context_window: u64,
    pub queues: SteerFollowUpQueues,
    pub tools: Vec<String>,
    pub skills: Vec<Skill>,
    pub templates: Vec<PromptTemplate>,
    pub context_files: Vec<ContextFile>,
    pub session: Option<JsonlSession>,
    pub cwd: PathBuf,
    pub aborted: bool,
    pub is_streaming: bool,
    pub is_compacting: bool,
    pub provider: String,
    pub model_id: String,
    pub tool_execution_mode: ToolExecutionMode,
    pub custom_tool_executor: Option<CustomToolExecutor>,
    pub summarizer: Option<Summarizer>,
}

impl Agent {
    pub fn new(system_prompt: impl Into<String>) -> Self {
        Self {
            system_prompt: system_prompt.into(),
            messages: Vec::new(),
            thinking_level: ThinkingLevel::Off,
            auto_compaction: true,
            compaction: CompactionSettings::default(),
            auto_retry: true,
            retry_attempts: 3,
            retry_base_delay_ms: 2_000,
            provider_timeout_ms: None,
            provider_max_retries: None,
            provider_max_retry_delay_ms: 60_000,
            thinking_budgets: None,
            context_window: 200_000,
            queues: SteerFollowUpQueues::default(),
            tools: BUILTIN_TOOLS.iter().map(|t| t.to_string()).collect(),
            skills: Vec::new(),
            templates: Vec::new(),
            context_files: Vec::new(),
            session: None,
            cwd: std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
            aborted: false,
            is_streaming: false,
            is_compacting: false,
            provider: "google".into(),
            model_id: String::new(),
            tool_execution_mode: ToolExecutionMode::Sequential,
            custom_tool_executor: None,
            summarizer: None,
        }
    }

    pub fn prompt(&mut self, text: &str) -> ChatMessage {
        let message = ChatMessage::text("user", text);
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
        let message = ChatMessage::text("assistant", text);
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
        self.is_compacting = true;
        let previous_summary = self.session.as_ref().and_then(|session| {
            session
                .entries
                .iter()
                .rev()
                .find(|entry| entry.entry_type == "compaction")
                .and_then(|entry| {
                    entry
                        .extra
                        .get("summary")
                        .and_then(|value| value.as_str())
                        .map(str::to_string)
                })
        });
        let provider = self.provider.clone();
        let model_id = self.model_id.clone();
        let bound = self.summarizer.clone().map(|inner| {
            Summarizer::new(move |request| {
                let mut request = request.clone();
                if request.provider.is_empty() {
                    request.provider = provider.clone();
                }
                if request.model_id.is_empty() {
                    request.model_id = model_id.clone();
                }
                inner.summarize(&request)
            })
        });
        let mut result = compact_messages_with_options(
            &self.messages,
            custom_instructions,
            self.compaction.keep_recent_tokens,
            self.compaction.reserve_tokens,
            previous_summary.as_deref(),
            bound.as_ref(),
        );
        if result.compacted {
            if let Some(session) = &mut self.session {
                let first_kept = first_kept_entry_id(session, &self.messages, &result.messages);
                result.first_kept_entry_id = first_kept.clone();
                let mut extra = serde_json::Map::new();
                extra.insert("summary".into(), serde_json::json!(result.summary));
                extra.insert("firstKeptEntryId".into(), serde_json::json!(first_kept));
                extra.insert(
                    "details".into(),
                    serde_json::to_value(&result.details).unwrap_or_default(),
                );
                extra.insert("fromHook".into(), serde_json::json!(false));
                if let Some(usage) = &result.usage {
                    extra.insert(
                        "usage".into(),
                        serde_json::to_value(usage).unwrap_or_default(),
                    );
                }
                let _ = session.append_entry(SessionEntry {
                    id: String::new(),
                    entry_type: "compaction".into(),
                    parent_id: session.leaf_id.clone(),
                    seq: 0,
                    timestamp: 0,
                    message: None,
                    custom_type: None,
                    extra,
                });
            }
        }
        if result.compacted {
            self.messages = result.messages.clone();
        }
        self.is_compacting = false;
        result
    }

    pub fn abort(&mut self) {
        self.aborted = true;
    }

    pub fn load_from_session(&mut self, session: JsonlSession) {
        self.messages = messages_from_session(&session);
        self.session = Some(session);
    }

    pub fn session_tree(&self) -> Vec<serde_json::Value> {
        self.session
            .as_ref()
            .map(|session| {
                session
                    .entries
                    .iter()
                    .map(|entry| {
                        serde_json::json!({
                            "id": entry.id,
                            "parentId": entry.parent_id,
                            "type": entry.entry_type,
                        })
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn entries_since(&self, since: Option<&str>) -> Vec<pi_session::SessionEntry> {
        let Some(session) = &self.session else {
            return Vec::new();
        };
        match since {
            Some(id) => session
                .entries
                .iter()
                .skip_while(|entry| entry.id != id)
                .skip(1)
                .cloned()
                .collect(),
            None => session.entries.clone(),
        }
    }
}

fn entry_to_chat(entry: &SessionEntry) -> Option<ChatMessage> {
    if entry.entry_type == "compaction" {
        let summary = entry.extra.get("summary")?.as_str()?;
        return Some(compaction_context_message(summary));
    }
    let message = entry.message.as_ref()?;
    serde_json::from_value(message.clone()).ok()
}

fn messages_from_session(session: &JsonlSession) -> Vec<ChatMessage> {
    let Some(index) = session
        .entries
        .iter()
        .rposition(|entry| entry.entry_type == "compaction")
    else {
        return session.entries.iter().filter_map(entry_to_chat).collect();
    };
    let first_kept_id = session.entries[index]
        .extra
        .get("firstKeptEntryId")
        .and_then(Value::as_str)
        .unwrap_or("");
    let first_kept_index = session
        .entries
        .iter()
        .position(|entry| entry.id == first_kept_id)
        .unwrap_or(index + 1);
    let mut messages = Vec::new();
    if let Some(summary) = session.entries[index]
        .extra
        .get("summary")
        .and_then(Value::as_str)
    {
        messages.push(compaction_context_message(summary));
    }
    for entry in session.entries.iter().skip(first_kept_index) {
        if entry.entry_type == "compaction" {
            continue;
        }
        if let Some(message) = entry_to_chat(entry) {
            messages.push(message);
        }
    }
    messages
}

fn first_kept_entry_id(
    session: &JsonlSession,
    before: &[ChatMessage],
    after: &[ChatMessage],
) -> String {
    let kept = after.len().saturating_sub(1);
    let first_kept_index = before.len().saturating_sub(kept);
    let mut message_index = 0usize;
    for entry in &session.entries {
        if entry.entry_type == "compaction" {
            continue;
        }
        if entry_to_chat(entry).is_none() {
            continue;
        }
        if message_index == first_kept_index {
            return entry.id.clone();
        }
        message_index += 1;
    }
    session
        .entries
        .last()
        .map(|entry| entry.id.clone())
        .unwrap_or_default()
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

    #[test]
    fn auto_retry_emits_ts_session_events() {
        use pi_ai::{AssistantMessage, ContentBlock, StopReason};

        let mut agent = Agent::new(default_system_prompt());
        agent.retry_attempts = 2;
        agent.retry_base_delay_ms = 0;
        agent.prompt("retry me");
        let mut calls = 0;
        let events = agent
            .run_loop(|_| {
                calls += 1;
                if calls == 1 {
                    return Ok(AssistantMessage {
                        id: "e1".into(),
                        role: "assistant".into(),
                        content: Vec::new(),
                        model: "fixture".into(),
                        usage: None,
                        stop_reason: Some(StopReason::Error),
                        error_message: Some("overloaded_error".into()),
                    });
                }
                Ok(AssistantMessage {
                    id: "ok".into(),
                    role: "assistant".into(),
                    content: vec![ContentBlock::Text {
                        text: "recovered".into(),
                    }],
                    model: "fixture".into(),
                    usage: None,
                    stop_reason: Some(StopReason::Stop),
                    error_message: None,
                })
            })
            .unwrap();
        let kinds: Vec<_> = events.iter().map(AgentEvent::kind).collect();
        assert!(kinds.contains(&"auto_retry_start"));
        assert!(kinds.contains(&"auto_retry_end"));
        assert_eq!(calls, 2);
        assert_eq!(agent.last_assistant_text().as_deref(), Some("recovered"));
    }

    #[test]
    fn agent_loop_emits_ts_event_names_and_runs_tools() {
        use pi_ai::{AssistantMessage, ContentBlock, StopReason};

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("note.txt"), "hello").unwrap();
        let mut agent = Agent::new(default_system_prompt());
        agent.cwd = dir.path().to_path_buf();
        agent.prompt("read the note");
        let events = agent
            .run_loop(|current| {
                if current.messages.iter().any(|m| m.role == "toolResult") {
                    return Ok(AssistantMessage {
                        id: "a2".into(),
                        role: "assistant".into(),
                        content: vec![ContentBlock::Text {
                            text: "done".into(),
                        }],
                        model: "fixture".into(),
                        usage: None,
                        stop_reason: Some(StopReason::Stop),
                        error_message: None,
                    });
                }
                Ok(AssistantMessage {
                    id: "a1".into(),
                    role: "assistant".into(),
                    content: vec![ContentBlock::ToolCall {
                        id: "call_1".into(),
                        name: "read".into(),
                        arguments: serde_json::json!({"path": "note.txt"}),
                    }],
                    model: "fixture".into(),
                    usage: None,
                    stop_reason: Some(StopReason::ToolUse),
                    error_message: None,
                })
            })
            .unwrap();
        let kinds: Vec<_> = events.iter().map(AgentEvent::kind).collect();
        assert_eq!(kinds.first().copied(), Some("agent_start"));
        assert!(kinds.contains(&"tool_execution_start"));
        assert!(kinds.contains(&"tool_execution_end"));
        assert_eq!(kinds.last().copied(), Some("agent_end"));
        assert_eq!(agent.last_assistant_text().as_deref(), Some("done"));
    }

    #[test]
    fn continue_loop_matches_ts_errors() {
        let mut agent = Agent::new("x");
        assert_eq!(
            agent.continue_loop(|_| unreachable!()).unwrap_err(),
            "Cannot continue: no messages in context"
        );
        agent.record_assistant("hi");
        assert_eq!(
            agent.continue_loop(|_| unreachable!()).unwrap_err(),
            "Cannot continue from message role: assistant"
        );
    }

    #[test]
    fn custom_tool_executor_runs_unknown_builtin_names() {
        use pi_ai::{AssistantMessage, ContentBlock, StopReason};

        let mut agent = Agent::new(default_system_prompt());
        agent.tools.push("ticket".into());
        agent.custom_tool_executor = Some(CustomToolExecutor::new(|_cwd, name, args| {
            Ok(ToolResult {
                content: format!("{name}:{}", args["id"].as_str().unwrap_or("")),
                is_error: false,
                details: None,
            })
        }));
        agent.prompt("lookup");
        let events = agent
            .run_loop(|current| {
                if current.messages.iter().any(|m| m.role == "toolResult") {
                    return Ok(AssistantMessage {
                        id: "a2".into(),
                        role: "assistant".into(),
                        content: vec![ContentBlock::Text {
                            text: "done".into(),
                        }],
                        model: "fixture".into(),
                        usage: None,
                        stop_reason: Some(StopReason::Stop),
                        error_message: None,
                    });
                }
                Ok(AssistantMessage {
                    id: "a1".into(),
                    role: "assistant".into(),
                    content: vec![ContentBlock::ToolCall {
                        id: "call_1".into(),
                        name: "ticket".into(),
                        arguments: serde_json::json!({"id": "42"}),
                    }],
                    model: "fixture".into(),
                    usage: None,
                    stop_reason: Some(StopReason::ToolUse),
                    error_message: None,
                })
            })
            .unwrap();
        assert!(events
            .iter()
            .any(|event| event.kind() == "tool_execution_end"));
        assert_eq!(agent.last_assistant_text().as_deref(), Some("done"));
    }

    #[test]
    fn retry_delay_matches_ts_exponential_backoff() {
        assert_eq!(retry_delay_ms(2000, 0), 2000);
        assert_eq!(retry_delay_ms(2000, 1), 4000);
        assert_eq!(retry_delay_ms(2000, 2), 8000);
        assert_eq!(retry_delay_ms(1, 3), 8);
    }
}
