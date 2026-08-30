//! Agent runtime matching `@earendil-works/pi-agent-core`.

mod branch;
mod compaction;
mod context;
mod edit_diff;
mod events;
mod file_mutation_queue;
mod images;
mod queues;
mod skills;
mod templates;
mod tools;
mod turn;

pub use branch::{
    build_branch_summary_prompt, collect_entries_for_branch_summary, generate_branch_summary,
    message_from_branch_entry, navigation_target, prepare_branch_entries, BranchPreparation,
    BranchSummaryResult, BRANCH_SUMMARY_PREAMBLE, BRANCH_SUMMARY_PROMPT,
};
pub use compaction::{
    branch_summary_context_message, build_history_prompt, calculate_context_tokens,
    compact_messages, compact_messages_with, compact_messages_with_options,
    compaction_context_message, compute_file_lists, convert_to_llm, env_summarizer,
    estimate_context_tokens, estimate_tokens, extract_file_ops, find_cut_point,
    format_file_operations, generate_summary_with_usage, get_summarization_failure,
    serialize_conversation, should_compact, CompactionDetails, CompactionResult,
    CompactionSettings, CompactionThreshold, CutPointResult, FileOperations, SummarizeRequest,
    SummarizeResponse, Summarizer, BRANCH_SUMMARY_PREFIX, BRANCH_SUMMARY_SUFFIX,
    COMPACTION_SUMMARY_PREFIX, COMPACTION_SUMMARY_SUFFIX, DEFAULT_KEEP_RECENT_TOKENS,
    DEFAULT_RESERVE_TOKENS, SUMMARIZATION_PROMPT, SUMMARIZATION_SYSTEM_PROMPT,
    TURN_PREFIX_SUMMARIZATION_PROMPT, UPDATE_SUMMARIZATION_PROMPT,
};
pub use context::{load_context_files, ContextFile};
pub use events::AgentEvent;
pub use file_mutation_queue::{mutation_queue_key, with_file_mutation_queue};
pub use images::{
    apply_block_images, convert_to_llm_for_provider, normalize_tool_result_images,
    parse_rpc_images, process_image_bytes, IMAGE_READING_DISABLED,
};
pub use queues::{QueueMode, QueuedMessage, SteerFollowUpQueues};
pub use skills::{discover_skills, expand_skill_command, expand_user_text, Skill};
pub use templates::{
    discover_prompt_templates, expand_prompt_template, parse_command_args, strip_frontmatter,
    substitute_args, PromptTemplate,
};
pub use tools::{execute_tool, tool_specs, AgentTool, ToolError, ToolResult, BUILTIN_TOOLS};
pub use turn::retry_delay_ms;

use pi_ai::{
    content_text, AssistantMessage, AssistantMessageEvent, ChatMessage, MessageContent,
    ThinkingBudgets,
};
use pi_protocol::ThinkingLevel;
use pi_session::{JsonlSession, SessionEntry};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use uuid::Uuid;

type CustomToolFn = dyn Fn(&Path, &str, &Value) -> Result<ToolResult, ToolError> + Send + Sync;
type PreToolFn = dyn Fn(&str, &Value) -> Option<String> + Send + Sync;
type PostToolFn = dyn Fn(&str, &Path, &str, &Value, ToolResult) -> ToolResult + Send + Sync;

/// Blocks a tool call when the hook returns a reason (TS `tool_call` `{ block: true }`).
#[derive(Clone)]
pub struct PreToolHook(pub Arc<PreToolFn>);

impl std::fmt::Debug for PreToolHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PreToolHook")
    }
}

/// Transforms a completed tool result before it is emitted and persisted.
/// Extensions use this for lossless output compression and other middleware
/// that must run for both built-in and custom tools.
#[derive(Clone)]
pub struct PostToolHook(pub Arc<PostToolFn>);

impl std::fmt::Debug for PostToolHook {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("PostToolHook")
    }
}

/// Live agent-loop subscriber matching TS `AgentSession.subscribe`.
#[derive(Clone)]
pub struct EventSink(pub Arc<dyn Fn(&AgentEvent) + Send + Sync>);

impl std::fmt::Debug for EventSink {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("EventSink")
    }
}

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
    pub tool_registry: Vec<String>,
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
    pub pre_tool: Option<PreToolHook>,
    pub post_tool: Option<PostToolHook>,
    pub summarizer: Option<Summarizer>,
    pub block_images: bool,
    pub auto_resize_images: bool,
    pub retry_aborted: bool,
    pub transport: Option<String>,
    pub install_telemetry: bool,
    pub reload_count: u32,
    pub event_sink: Option<EventSink>,
    base_system_prompt: String,
    pending_bash_messages: Vec<ChatMessage>,
    pending_prompt_messages: Vec<ChatMessage>,
    /// Context supplied by extensions for the next provider request only.
    /// These messages never enter the persisted session history.
    ephemeral_context: Vec<ChatMessage>,
}

impl Agent {
    pub fn new(system_prompt: impl Into<String>) -> Self {
        let system_prompt = system_prompt.into();
        Self {
            system_prompt: system_prompt.clone(),
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
            tool_registry: BUILTIN_TOOLS.iter().map(|t| t.to_string()).collect(),
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
            pre_tool: None,
            post_tool: None,
            summarizer: None,
            block_images: false,
            auto_resize_images: true,
            retry_aborted: false,
            transport: None,
            install_telemetry: true,
            reload_count: 0,
            event_sink: None,
            base_system_prompt: system_prompt,
            pending_bash_messages: Vec::new(),
            pending_prompt_messages: Vec::new(),
            ephemeral_context: Vec::new(),
        }
    }

    /// Restore the base prompt before each extension-aware prompt turn.
    pub fn reset_system_prompt_to_base(&mut self) {
        self.system_prompt = self.base_system_prompt.clone();
    }

    /// Replace the ephemeral context used for the next provider request.
    /// Extension-provided context is inserted immediately before the latest
    /// user message so it supports, rather than follows, the active prompt.
    pub fn set_ephemeral_context(&mut self, messages: Vec<ChatMessage>) {
        self.ephemeral_context = messages;
    }

    /// Remove extension context after a prompt turn (or when a session is
    /// switched) without touching persisted conversation messages.
    pub fn clear_ephemeral_context(&mut self) {
        self.ephemeral_context.clear();
    }

    pub fn push_event(&self, events: &mut Vec<AgentEvent>, event: AgentEvent) {
        if let Some(sink) = &self.event_sink {
            (sink.0)(&event);
        }
        events.push(event);
    }

    /// TS `evalSession.reload()` — isolated evals have no extensions; count the step.
    pub fn reload(&mut self) {
        self.reload_count = self.reload_count.saturating_add(1);
        self.aborted = false;
    }

    pub fn prompt(&mut self, text: &str) -> ChatMessage {
        self.prompt_with(text, &[])
    }

    pub fn prompt_with(&mut self, text: &str, images: &[pi_ai::MessageContent]) -> ChatMessage {
        self.flush_pending_bash_messages();
        let mut content = vec![pi_ai::MessageContent::Text {
            text: text.to_string(),
        }];
        content.extend(images.iter().cloned());
        if self.auto_resize_images {
            content = crate::normalize_tool_result_images(&content, true);
        }
        let message = ChatMessage {
            role: "user".into(),
            content,
            ..ChatMessage::default()
        };
        self.messages.push(message.clone());
        if let Some(session) = &mut self.session {
            let _ = session.append_entry(SessionEntry::message(
                "user",
                serde_json::to_value(&message.content).unwrap_or(Value::Null),
            ));
        }
        self.pending_prompt_messages.push(message.clone());
        message
    }

    pub fn messages_for_provider(&self) -> Vec<ChatMessage> {
        if self.ephemeral_context.is_empty() {
            return convert_to_llm_for_provider(&self.messages, self.block_images);
        }
        let mut messages = self.messages.clone();
        let insertion = messages
            .iter()
            .rposition(|message| message.role == "user")
            .unwrap_or(messages.len());
        messages.splice(insertion..insertion, self.ephemeral_context.iter().cloned());
        convert_to_llm_for_provider(&messages, self.block_images)
    }

    fn persist_full_message(&mut self, message: &ChatMessage) {
        if let Some(session) = &mut self.session {
            let mut entry = SessionEntry::message(
                &message.role,
                serde_json::to_value(&message.content).unwrap_or(Value::Null),
            );
            entry.message = Some(serde_json::to_value(message).unwrap_or(Value::Null));
            let _ = session.append_entry(entry);
        }
    }

    fn commit_bash_message(&mut self, message: ChatMessage) {
        self.persist_full_message(&message);
        self.messages.push(message);
    }

    /// TypeScript `AgentSession.recordBashResult`.
    pub fn record_bash_result(
        &mut self,
        command: &str,
        result: &Value,
        exclude_from_context: bool,
    ) {
        let output = result
            .get("output")
            .or_else(|| result.get("content"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let exit_code = result
            .get("exitCode")
            .cloned()
            .or_else(|| {
                result
                    .get("details")
                    .and_then(|value| value.get("exitCode"))
                    .cloned()
            })
            .unwrap_or(Value::Null);
        let cancelled = result
            .get("cancelled")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let truncated = result
            .get("truncated")
            .and_then(Value::as_bool)
            .or_else(|| {
                result
                    .get("details")
                    .and_then(|value| value.get("truncation"))
                    .map(|_| true)
            })
            .unwrap_or(false);
        let mut extra = serde_json::Map::new();
        extra.insert("command".into(), Value::String(command.to_string()));
        extra.insert("output".into(), Value::String(output));
        extra.insert("exitCode".into(), exit_code);
        extra.insert("cancelled".into(), Value::Bool(cancelled));
        extra.insert("truncated".into(), Value::Bool(truncated));
        extra.insert("timestamp".into(), serde_json::json!(pi_session::now_ms()));
        extra.insert(
            "excludeFromContext".into(),
            Value::Bool(exclude_from_context),
        );
        if let Some(path) = result.get("fullOutputPath").and_then(Value::as_str) {
            extra.insert("fullOutputPath".into(), Value::String(path.to_string()));
        }
        let message = ChatMessage {
            role: "bashExecution".into(),
            content: Vec::new(),
            extra,
            ..ChatMessage::default()
        };
        if self.is_streaming {
            self.pending_bash_messages.push(message);
        } else {
            self.commit_bash_message(message);
        }
    }

    pub fn flush_pending_bash_messages(&mut self) {
        let pending = std::mem::take(&mut self.pending_bash_messages);
        for message in pending {
            self.commit_bash_message(message);
        }
    }

    /// Persist and append a TypeScript extension `CustomMessage`.
    pub fn record_custom_message(&mut self, raw: &Value) -> ChatMessage {
        let content = match raw.get("content") {
            Some(Value::String(text)) => vec![pi_ai::MessageContent::Text { text: text.clone() }],
            Some(Value::Array(_)) => {
                serde_json::from_value(raw["content"].clone()).unwrap_or_default()
            }
            _ => Vec::new(),
        };
        let session_content = match raw.get("content") {
            Some(Value::String(text)) => Value::String(text.clone()),
            Some(Value::Array(value))
                if serde_json::from_value::<Vec<MessageContent>>(Value::Array(value.clone()))
                    .is_ok() =>
            {
                Value::Array(value.clone())
            }
            _ => serde_json::json!([]),
        };
        let timestamp = pi_session::now_ms();
        let mut extra = serde_json::Map::new();
        for key in ["customType", "display", "details"] {
            if let Some(value) = raw.get(key) {
                extra.insert(key.to_string(), value.clone());
            }
        }
        extra.insert("timestamp".into(), serde_json::json!(timestamp));
        let message = ChatMessage {
            role: "custom".into(),
            content,
            extra,
            ..ChatMessage::default()
        };
        if let Some(session) = &mut self.session {
            let mut extra = serde_json::Map::new();
            extra.insert("content".into(), session_content);
            for key in ["display", "details"] {
                if let Some(value) = raw.get(key) {
                    extra.insert(key.to_string(), value.clone());
                }
            }
            let _ = session.append_entry(SessionEntry {
                id: String::new(),
                entry_type: "custom_message".into(),
                parent_id: None,
                seq: 0,
                timestamp,
                message: None,
                custom_type: raw
                    .get("customType")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                extra,
            });
        }
        self.messages.push(message.clone());
        self.pending_prompt_messages.push(message.clone());
        message
    }

    pub fn abort_retry(&mut self) {
        self.retry_aborted = true;
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
            if !self.tool_registry.contains(name) {
                self.tool_registry.push(name.clone());
            }
            if !self.tools.contains(name) {
                self.tools.push(name.clone());
            }
        }
    }

    /// TS `setActiveToolsByName` — only registry names are enabled; unknown names ignored.
    pub fn set_active_tools_by_name(&mut self, names: &[String]) {
        self.tools = names
            .iter()
            .filter(|name| self.tool_registry.iter().any(|known| known == *name))
            .cloned()
            .collect();
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
        self.pending_prompt_messages.clear();
        self.session = Some(session);
    }

    /// Navigate the session tree. When `summarize` is true, generates a branch
    /// summary of the abandoned path and appends a `branch_summary` entry.
    pub fn navigate_tree_entry(
        &mut self,
        target_id: &str,
        summarize: bool,
        custom_instructions: Option<&str>,
        replace_instructions: bool,
        reserve_tokens: u64,
    ) -> Result<TreeNavigateResult, String> {
        let session = self
            .session
            .as_ref()
            .ok_or_else(|| "No session to navigate".to_string())?;
        let target = session
            .entries
            .iter()
            .find(|entry| entry.id == target_id)
            .cloned()
            .ok_or_else(|| format!("Entry {target_id} not found"))?;
        let old_leaf = session.leaf_id.clone();
        let (collected, _) =
            collect_entries_for_branch_summary(&session.entries, old_leaf.as_deref(), target_id);
        let (new_leaf, editor_text) = navigation_target(&target);
        let mut summary_text = None;
        if summarize && !collected.is_empty() {
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
            let result = generate_branch_summary(
                &collected,
                self.context_window,
                reserve_tokens,
                custom_instructions,
                replace_instructions,
                bound.as_ref(),
            );
            if result.aborted {
                return Ok(TreeNavigateResult {
                    cancelled: true,
                    editor_text: None,
                    summary: None,
                });
            }
            if let Some(error) = result.error {
                return Err(error);
            }
            if let Some(summary) = result.summary.clone() {
                let details = serde_json::to_value(&result.details).unwrap_or_default();
                let usage = result
                    .usage
                    .as_ref()
                    .and_then(|usage| serde_json::to_value(usage).ok());
                self.session
                    .as_mut()
                    .ok_or_else(|| "No session to navigate".to_string())?
                    .branch_with_summary(new_leaf.clone(), &summary, details, usage, false)
                    .map_err(|err| err.to_string())?;
                summary_text = Some(summary);
            }
        } else if let Some(session) = &mut self.session {
            session.set_leaf(new_leaf);
        }
        if let Some(session) = &self.session {
            self.messages = messages_from_session(session);
        }
        Ok(TreeNavigateResult {
            cancelled: false,
            editor_text,
            summary: summary_text,
        })
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

#[derive(Debug, Clone, Default)]
pub struct TreeNavigateResult {
    pub cancelled: bool,
    pub editor_text: Option<String>,
    pub summary: Option<String>,
}

pub(crate) fn custom_message_from_session_entry(entry: &SessionEntry) -> Option<ChatMessage> {
    let content = entry
        .extra
        .get("content")
        .cloned()
        .or_else(|| entry.message.clone())
        .unwrap_or_else(|| serde_json::json!([]));
    let content = match &content {
        Value::String(text) => vec![MessageContent::Text { text: text.clone() }],
        Value::Array(_) => serde_json::from_value(content).unwrap_or_default(),
        _ => Vec::new(),
    };
    let mut extra = serde_json::Map::new();
    if let Some(custom_type) = &entry.custom_type {
        extra.insert("customType".into(), Value::String(custom_type.clone()));
    }
    for key in ["display", "details"] {
        if let Some(value) = entry.extra.get(key) {
            extra.insert(key.to_string(), value.clone());
        }
    }
    extra.insert("timestamp".into(), serde_json::json!(entry.timestamp));
    Some(ChatMessage {
        role: "custom".into(),
        content,
        extra,
        ..ChatMessage::default()
    })
}

fn entry_to_chat(entry: &SessionEntry) -> Option<ChatMessage> {
    match entry.entry_type.as_str() {
        "compaction" => {
            let summary = entry.extra.get("summary")?.as_str()?;
            Some(compaction_context_message(summary))
        }
        "branch_summary" => {
            let summary = entry.extra.get("summary")?.as_str()?;
            Some(branch_summary_context_message(summary))
        }
        "custom_message" => custom_message_from_session_entry(entry),
        "message" => {
            let message = entry.message.as_ref()?;
            serde_json::from_value(message.clone()).ok()
        }
        _ => None,
    }
}

fn messages_from_session(session: &JsonlSession) -> Vec<ChatMessage> {
    pi_session::build_context_entries(&session.entries, session.leaf_id.as_deref())
        .into_iter()
        .filter_map(entry_to_chat)
        .collect()
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

/// Provider complete callback output. `From<AssistantMessage>` keeps existing closures working.
#[derive(Debug, Clone)]
pub struct CompleteOutput {
    pub message: AssistantMessage,
    pub stream_events: Option<Vec<AssistantMessageEvent>>,
}

impl From<AssistantMessage> for CompleteOutput {
    fn from(message: AssistantMessage) -> Self {
        Self {
            message,
            stream_events: None,
        }
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

    #[test]
    fn system_prompt_reset_restores_the_base_prompt() {
        let mut agent = Agent::new("base prompt");
        agent.system_prompt = "extension override".into();

        agent.reset_system_prompt_to_base();

        assert_eq!(agent.system_prompt, "base prompt");
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
        assert!(kinds.contains(&"message_update"));
        let update_types: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                AgentEvent::MessageUpdate {
                    assistant_message_event,
                    ..
                } => Some(match assistant_message_event {
                    pi_ai::AssistantMessageEvent::TextDelta { .. } => "text_delta",
                    pi_ai::AssistantMessageEvent::ThinkingDelta { .. } => "thinking_delta",
                    pi_ai::AssistantMessageEvent::ToolcallStart { .. } => "toolcall_start",
                    pi_ai::AssistantMessageEvent::ToolcallEnd { .. } => "toolcall_end",
                    _ => "other",
                }),
                _ => None,
            })
            .collect();
        assert!(update_types.contains(&"toolcall_start"));
        assert!(update_types.contains(&"text_delta"));
        assert_eq!(kinds.last().copied(), Some("agent_end"));
        assert_eq!(agent.last_assistant_text().as_deref(), Some("done"));
    }

    #[test]
    fn event_sink_receives_events_as_run_loop_emits_them() {
        use pi_ai::{AssistantMessage, ContentBlock, StopReason};

        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
        let sink_seen = seen.clone();
        let mut agent = Agent::new(default_system_prompt());
        agent.event_sink = Some(EventSink(std::sync::Arc::new(move |event| {
            sink_seen.lock().unwrap().push(event.kind().to_string());
        })));
        agent.prompt("hello");
        let events = agent
            .run_loop(|_| {
                Ok(AssistantMessage {
                    id: "a1".into(),
                    role: "assistant".into(),
                    content: vec![ContentBlock::Text { text: "ok".into() }],
                    model: "fixture".into(),
                    usage: None,
                    stop_reason: Some(StopReason::Stop),
                    error_message: None,
                })
            })
            .unwrap();
        let kinds = seen.lock().unwrap().clone();
        assert!(
            kinds.contains(&"agent_start".to_string()),
            "sink should observe agent_start before run_loop returns: {kinds:?}"
        );
        assert_eq!(
            kinds,
            events
                .iter()
                .map(|event| event.kind().to_string())
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn continue_loop_matches_ts_errors() {
        let mut agent = Agent::new("x");
        assert_eq!(
            agent
                .continue_loop::<_, AssistantMessage>(|_| unreachable!())
                .unwrap_err(),
            "Cannot continue: no messages in context"
        );
        agent.record_assistant("hi");
        assert_eq!(
            agent
                .continue_loop::<_, AssistantMessage>(|_| unreachable!())
                .unwrap_err(),
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
    fn block_images_and_abort_retry_match_ts() {
        use pi_ai::MessageContent;
        let mut agent = Agent::new("x");
        agent.block_images = true;
        agent.prompt_with(
            "hi",
            &[MessageContent::Image {
                data: "e30=".into(),
                mime_type: "image/png".into(),
            }],
        );
        let for_llm = agent.messages_for_provider();
        assert!(for_llm[0].content.iter().any(|block| matches!(
            block,
            MessageContent::Text { text } if text == IMAGE_READING_DISABLED
        )));
        agent.abort_retry();
        assert!(agent.retry_aborted);
    }

    #[test]
    fn ephemeral_context_precedes_latest_prompt_without_persistence() {
        let mut agent = Agent::new("x");
        agent.prompt("active question");
        agent.set_ephemeral_context(vec![ChatMessage::text("custom", "supporting memory")]);

        let provider_messages = agent.messages_for_provider();
        assert_eq!(provider_messages.len(), 2);
        assert_eq!(provider_messages[0].role, "user");
        assert_eq!(
            content_text(&provider_messages[0].content),
            "supporting memory"
        );
        assert_eq!(
            content_text(&provider_messages[1].content),
            "active question"
        );
        assert_eq!(agent.messages.len(), 1);

        agent.clear_ephemeral_context();
        assert_eq!(agent.messages_for_provider().len(), 1);
    }

    #[test]
    fn retry_delay_matches_ts_exponential_backoff() {
        assert_eq!(retry_delay_ms(2000, 0), 2000);
        assert_eq!(retry_delay_ms(2000, 1), 4000);
        assert_eq!(retry_delay_ms(2000, 2), 8000);
        assert_eq!(retry_delay_ms(1, 3), 8);
    }

    #[test]
    fn navigate_tree_appends_llm_branch_summary() {
        let dir = tempfile::tempdir().unwrap();
        let mut session = JsonlSession::create(dir.path(), "/tmp/project", Some("branch")).unwrap();
        session
            .append_entry(SessionEntry::message(
                "user",
                serde_json::json!([{"type":"text","text":"start"}]),
            ))
            .unwrap();
        session
            .append_entry(SessionEntry::message(
                "assistant",
                serde_json::json!([{"type":"text","text":"ok"}]),
            ))
            .unwrap();
        session
            .append_entry(SessionEntry::message(
                "user",
                serde_json::json!([{"type":"text","text":"abandoned"}]),
            ))
            .unwrap();
        let abandoned = session.leaf_id.clone().unwrap();
        session.set_leaf(Some(session.entries[1].id.clone()));
        session
            .append_entry(SessionEntry::message(
                "user",
                serde_json::json!([{"type":"text","text":"other"}]),
            ))
            .unwrap();

        let mut agent = Agent::new("x");
        agent.summarizer = Some(Summarizer::new(|request| {
            assert_eq!(request.system, SUMMARIZATION_SYSTEM_PROMPT);
            assert!(request.prompt.contains("## Goal"));
            assert_eq!(request.max_tokens, 2048);
            Ok(SummarizeResponse {
                text: "## Goal\nabandoned work".into(),
                usage: pi_protocol::Usage {
                    input: 1,
                    output: 2,
                    total_tokens: 3,
                    ..pi_protocol::Usage::default()
                },
                stop_reason: Some(pi_ai::StopReason::Stop),
                error_message: None,
                has_tool_call: false,
            })
        }));
        agent.load_from_session(session);
        let result = agent
            .navigate_tree_entry(&abandoned, true, None, false, 16_384)
            .unwrap();
        assert_eq!(result.editor_text.as_deref(), Some("abandoned"));
        assert!(result
            .summary
            .as_deref()
            .unwrap()
            .starts_with(BRANCH_SUMMARY_PREAMBLE));
        assert!(result
            .summary
            .as_deref()
            .unwrap()
            .contains("## Goal\nabandoned work"));
        let stored = agent.session.as_ref().unwrap();
        assert!(stored
            .entries
            .iter()
            .any(|entry| entry.entry_type == "branch_summary"
                && entry.extra.get("fromId").and_then(Value::as_str)
                    == stored
                        .entries
                        .iter()
                        .find(|e| {
                            e.entry_type == "message"
                                && e.message
                                    .as_ref()
                                    .and_then(|m| m.get("content"))
                                    .and_then(|c| c.as_array())
                                    .and_then(|items| items[0].get("text"))
                                    .and_then(Value::as_str)
                                    == Some("other")
                        })
                        .map(|e| e.id.as_str())));
        assert!(agent
            .messages
            .iter()
            .any(|message| content_text(&message.content).contains("abandoned work")));
        assert!(!agent
            .messages
            .iter()
            .any(|message| content_text(&message.content) == "other"));
    }

    #[test]
    fn pre_tool_hook_blocks_before_execution() {
        use pi_ai::{AssistantMessage, ContentBlock, StopReason};
        let mut agent = Agent::new("x");
        agent.pre_tool = Some(PreToolHook(Arc::new(|name, _| {
            if name == "bash" {
                Some("blocked by extension".into())
            } else {
                None
            }
        })));
        agent.prompt("run");
        let events = agent
            .run_loop(|current| {
                if current
                    .messages
                    .iter()
                    .any(|message| message.role == "toolResult")
                {
                    return Ok(AssistantMessage {
                        id: "a2".into(),
                        role: "assistant".into(),
                        content: vec![ContentBlock::Text {
                            text: "stopped".into(),
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
                        id: "c1".into(),
                        name: "bash".into(),
                        arguments: serde_json::json!({"command": "echo hi"}),
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
            .any(|event| matches!(event, AgentEvent::ToolExecutionEnd { is_error: true, .. })));
        let result = agent
            .messages
            .iter()
            .rev()
            .find(|message| message.role == "toolResult")
            .map(|message| content_text(&message.content))
            .unwrap_or_default();
        assert!(result.contains("blocked by extension"));
    }

    #[test]
    fn record_bash_result_persists_ts_message_and_excludes_double_bang_from_llm() {
        let dir = tempfile::tempdir().unwrap();
        let session = JsonlSession::create(dir.path(), "/tmp/project", Some("bash")).unwrap();
        let mut agent = Agent::new("x");
        agent.session = Some(session);

        agent.record_bash_result(
            "printf hi",
            &serde_json::json!({
                "output": "hi",
                "exitCode": 0,
                "cancelled": false,
                "truncated": false
            }),
            false,
        );

        let bash = agent.messages.last().expect("bash message");
        assert_eq!(bash.role, "bashExecution");
        assert_eq!(
            bash.extra.get("command"),
            Some(&serde_json::json!("printf hi"))
        );
        assert_eq!(bash.extra.get("output"), Some(&serde_json::json!("hi")));
        assert_eq!(bash.extra.get("exitCode"), Some(&serde_json::json!(0)));
        assert_eq!(bash.extra.get("cancelled"), Some(&serde_json::json!(false)));
        assert_eq!(bash.extra.get("truncated"), Some(&serde_json::json!(false)));

        let stored = agent.session.as_ref().unwrap().entries.last().unwrap();
        let stored_message = stored.message.as_ref().unwrap();
        assert_eq!(
            stored_message.get("role").and_then(Value::as_str),
            Some("bashExecution")
        );
        assert_eq!(
            stored_message.get("command").and_then(Value::as_str),
            Some("printf hi")
        );
        assert_eq!(
            stored_message.get("output").and_then(Value::as_str),
            Some("hi")
        );

        let llm = agent.messages_for_provider();
        assert_eq!(llm.last().unwrap().role, "user");
        assert_eq!(
            content_text(&llm.last().unwrap().content),
            "Ran `printf hi`\n```\nhi\n```"
        );

        agent.record_bash_result(
            "secret",
            &serde_json::json!({
                "output": "hidden",
                "exitCode": 0,
                "cancelled": false,
                "truncated": false
            }),
            true,
        );
        assert_eq!(
            agent
                .messages
                .last()
                .unwrap()
                .extra
                .get("excludeFromContext")
                .and_then(Value::as_bool),
            Some(true)
        );
        assert!(!agent
            .messages_for_provider()
            .iter()
            .any(|message| content_text(&message.content).contains("secret")));
    }

    #[test]
    fn pending_bash_results_flush_before_the_next_prompt() {
        let dir = tempfile::tempdir().unwrap();
        let session =
            JsonlSession::create(dir.path(), "/tmp/project", Some("bash-pending")).unwrap();
        let mut agent = Agent::new("x");
        agent.session = Some(session);
        agent.is_streaming = true;

        agent.record_bash_result(
            "echo queued",
            &serde_json::json!({
                "output": "queued",
                "exitCode": 0,
                "cancelled": false,
                "truncated": false
            }),
            false,
        );
        assert!(agent.messages.is_empty());
        assert!(agent.session.as_ref().unwrap().entries.is_empty());

        agent.is_streaming = false;
        agent.prompt("next");
        assert_eq!(agent.messages[0].role, "bashExecution");
        assert_eq!(agent.messages[1].role, "user");
        let entries = &agent.session.as_ref().unwrap().entries;
        assert_eq!(entries.len(), 2);
        assert_eq!(
            entries[0]
                .message
                .as_ref()
                .and_then(|message| message.get("role"))
                .and_then(Value::as_str),
            Some("bashExecution")
        );
    }

    #[test]
    fn record_custom_message_persists_before_agent_start_shape() {
        let dir = tempfile::tempdir().unwrap();
        let session = JsonlSession::create(dir.path(), "/tmp/project", Some("custom")).unwrap();
        let mut agent = Agent::new("x");
        agent.session = Some(session);

        agent.record_custom_message(&serde_json::json!({
            "customType": "hint",
            "content": [{"type":"text","text":"extension context"}],
            "display": false,
            "details": {"source":"before_agent_start"}
        }));

        let message = agent.messages.last().unwrap();
        assert_eq!(message.role, "custom");
        assert_eq!(content_text(&message.content), "extension context");
        assert_eq!(
            message.extra.get("customType").and_then(Value::as_str),
            Some("hint")
        );
        assert_eq!(
            message.extra.get("display").and_then(Value::as_bool),
            Some(false)
        );
        let stored = agent.session.as_ref().unwrap().entries.last().unwrap();
        assert_eq!(stored.entry_type, "custom_message");
        assert_eq!(stored.custom_type.as_deref(), Some("hint"));
        assert_eq!(
            stored.extra.get("content"),
            Some(&serde_json::json!([{"type":"text","text":"extension context"}]))
        );
        assert_eq!(stored.extra.get("display"), Some(&serde_json::json!(false)));
        assert_eq!(
            stored.extra.get("details"),
            Some(&serde_json::json!({"source":"before_agent_start"}))
        );
    }

    #[test]
    fn before_agent_start_custom_messages_are_emitted_in_current_turn_and_reload() {
        use pi_ai::{AssistantMessage, ContentBlock, StopReason};

        let dir = tempfile::tempdir().unwrap();
        let session = JsonlSession::create(dir.path(), "/tmp/project", Some("prompt")).unwrap();
        let mut agent = Agent::new("x");
        agent.session = Some(session);
        agent.prompt("hello");
        agent.record_custom_message(&serde_json::json!({
            "customType": "hint",
            "content": [
                {"type":"text","text":"extension context"},
                {"type":"image","data":"e30=","mimeType":"image/png"}
            ],
            "display": false,
            "details": {"source":"before_agent_start"}
        }));

        let events = agent
            .run_loop(|_| {
                Ok(AssistantMessage {
                    id: "assistant-1".into(),
                    role: "assistant".into(),
                    content: vec![ContentBlock::Text {
                        text: "done".into(),
                    }],
                    model: "fixture".into(),
                    usage: None,
                    stop_reason: Some(StopReason::Stop),
                    error_message: None,
                })
            })
            .unwrap();

        let message_starts: Vec<_> = events
            .iter()
            .filter_map(|event| match event {
                AgentEvent::MessageStart { message } => Some(message),
                _ => None,
            })
            .collect();
        assert_eq!(
            message_starts
                .iter()
                .map(|message| message.role.as_str())
                .collect::<Vec<_>>(),
            vec!["user", "custom", "assistant"]
        );
        assert_eq!(
            message_starts[1].extra.get("customType"),
            Some(&serde_json::json!("hint"))
        );
        assert!(matches!(
            message_starts[1].content.get(1),
            Some(pi_ai::MessageContent::Image { .. })
        ));

        let agent_end = events
            .iter()
            .find_map(|event| match event {
                AgentEvent::AgentEnd { messages, .. } => Some(messages),
                _ => None,
            })
            .unwrap();
        assert_eq!(
            agent_end
                .iter()
                .map(|message| message.role.as_str())
                .collect::<Vec<_>>(),
            vec!["user", "custom", "assistant"]
        );

        let session = agent.session.as_ref().unwrap();
        let custom_entry = session
            .entries
            .iter()
            .find(|entry| entry.entry_type == "custom_message")
            .unwrap();
        assert_eq!(custom_entry.custom_type.as_deref(), Some("hint"));
        assert_eq!(
            custom_entry.extra.get("content"),
            Some(&serde_json::json!([
                {"type":"text","text":"extension context"},
                {"type":"image","data":"e30=","mimeType":"image/png"}
            ]))
        );

        let reloaded = messages_from_session(session);
        let reloaded_custom = reloaded
            .iter()
            .find(|message| message.role == "custom")
            .unwrap();
        assert_eq!(content_text(&reloaded_custom.content), "extension context");
        assert_eq!(
            reloaded_custom.extra.get("customType"),
            Some(&serde_json::json!("hint"))
        );
        assert!(matches!(
            reloaded_custom.content.get(1),
            Some(pi_ai::MessageContent::Image { .. })
        ));
    }

    #[test]
    fn set_active_tools_by_name_ignores_unknown_and_rebuilds_active_set() {
        let mut agent = Agent::new("x");
        agent.apply_extension_tools(&["ticket".into()]);
        assert!(agent.tools.contains(&"ticket".into()));
        agent.set_active_tools_by_name(&["read".into(), "missing".into(), "ticket".into()]);
        assert_eq!(agent.tools, vec!["read".to_string(), "ticket".to_string()]);
        agent.set_active_tools_by_name(&["bash".into(), "read".into()]);
        assert_eq!(agent.tools, vec!["bash".to_string(), "read".to_string()]);
        assert!(agent.tool_registry.contains(&"ticket".into()));
    }
}
