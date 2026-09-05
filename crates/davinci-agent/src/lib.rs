//! Agent runtime matching `@earendil-works/pi-agent-core`.

pub mod apply_patch;
mod batch;
mod branch;
mod compaction;
mod context;
mod edit_diff;
mod events;
mod evidence;
mod file_mutation_queue;
mod images;
pub mod jobs;
pub mod mcp;
pub mod notebook;
mod permission;
mod pruning;
mod queues;
mod scheduler;
mod skills;
mod stats;
mod subagent;
mod templates;
pub mod todo;
pub mod tool_ledger;
mod tools;
mod turn;
pub mod web;

pub use batch::{BATCH_MAX_OPERATIONS, VISIBLE_PER_OPERATION, VISIBLE_TOTAL};
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
pub use evidence::{EvidenceStore, EVIDENCE_TTL};
pub use file_mutation_queue::{mutation_queue_key, with_file_mutation_queue};
pub use images::{
    apply_block_images, convert_to_llm_for_provider, normalize_tool_result_images,
    parse_rpc_images, process_image_bytes, IMAGE_READING_DISABLED,
};
pub use jobs::{JobBook, JobNotice, JobStatus, JobSummary};
pub use mcp::{McpRegistry, McpServerRow};
pub use permission::{
    glob_matches, session_rule_for, subject_of, summary_of, tool_class, PermissionMode,
    PermissionPolicy, PermissionRule, PermissionVerdict, ToolApprovalDecision, ToolApprovalRequest,
    ToolApprover, ToolClass,
};
pub use pruning::PruneSettings;
pub use queues::{QueueMode, QueuedMessage, SteerFollowUpQueues};
pub use scheduler::{lane_for, ToolLane, MAX_TOOL_PARALLELISM};
pub use skills::{
    describe_skill, discover_skills, expand_skill_command, expand_user_text,
    expand_user_text_with_metadata, ExpandedUserText, Skill, SkillDescriptor,
};
pub use stats::{RunStats, SharedCounters};
pub use subagent::{
    scoped_tools, SubagentRequest, SubagentRunner, DEFAULT_SUBAGENT_TOOLS, PLAN_MODE_APPENDIX,
    PLAN_MODE_DENIAL,
};
pub use templates::{
    discover_prompt_templates, expand_prompt_template, parse_command_args, strip_frontmatter,
    substitute_args, PromptTemplate,
};
pub use todo::{TodoItem, TodoList, TodoStatus, TODO_ENTRY_TYPE};
pub use tool_ledger::{
    classify_side_effect, ToolCallLedger, ToolCallRecord, ToolExecutionStatus, ToolSideEffect,
};
pub use tools::{
    execute_tool, execute_tool_with, tool_specs, AgentTool, ToolContext, ToolError, ToolResult,
    BUILTIN_TOOLS, CODEX_HOT_TOOLS,
};
pub use turn::retry_delay_ms;

use davinci_ai::{
    content_text, AssistantMessage, AssistantMessageEvent, ChatMessage, MessageContent,
    ThinkingBudgets,
};
use davinci_protocol::ThinkingLevel;
use davinci_session::{JsonlSession, SessionEntry};
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
    /// Which tools may run without asking (`permission.rs`). Shared, because
    /// the gate reads it from `&self` on the tool thread while the host reads
    /// the mode for its chrome, and a granted rule is written back mid-turn.
    pub permissions: Arc<std::sync::Mutex<PermissionPolicy>>,
    /// Who answers when the policy says ask. `None` means the run cannot
    /// ask, and the call is refused with a message that says so.
    pub approver: Option<ToolApprover>,
    /// Background shell jobs (`jobs.rs`) and the model's todo ledger
    /// (`todo.rs`), shared with the tool thread and the shell.
    pub tool_context: ToolContext,
    pub summarizer: Option<Summarizer>,
    pub subagent_runner: Option<crate::subagent::SubagentRunner>,
    /// When true, mutations are refused until `/act`.
    pub plan_mode: bool,
    pub block_images: bool,
    pub auto_resize_images: bool,
    pub retry_aborted: bool,
    pub transport: Option<String>,
    pub install_telemetry: bool,
    pub reload_count: u32,
    pub event_sink: Option<EventSink>,
    /// Cross-thread interrupt: set from the UI thread while `run_loop` runs
    /// on a worker. Checked at every loop step alongside `aborted`.
    pub abort_signal: Option<std::sync::Arc<std::sync::atomic::AtomicBool>>,
    /// Counters for the run (`stats.rs`): turns, batch widths, wall time,
    /// peak context, prunings. Read them through `run_stats`, which also
    /// folds in the counters bumped from inside tool calls.
    pub stats: RunStats,
    pub counters: Arc<SharedCounters>,
    /// When old tool output leaves the provider's view (`pruning.rs`).
    pub prune_settings: PruneSettings,
    /// Where overflowing output is kept for a later `read` (`evidence.rs`).
    /// `None` means overflow is truncated with a note and nothing else.
    pub evidence: Option<EvidenceStore>,
    pub tool_ledger: Arc<std::sync::Mutex<ToolCallLedger>>,
    /// Tool-call ids whose results are pruned from the provider view. Only
    /// grows; the session file keeps every body.
    pruned_tool_results: std::collections::HashSet<String>,
    base_system_prompt: String,
    pending_bash_messages: Vec<ChatMessage>,
    pending_prompt_messages: Vec<ChatMessage>,
    /// Context supplied by extensions for the next provider request only.
    /// These messages never enter the persisted session history.
    ephemeral_context: Vec<ChatMessage>,
    /// Host-supplied schema/identity estimate, excluding `system_prompt` and messages.
    provider_context_overhead_tokens: Option<u64>,
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
            // TS `runtimeOptions.toolExecution ?? "parallel"`.
            tool_execution_mode: ToolExecutionMode::Parallel,
            custom_tool_executor: None,
            pre_tool: None,
            post_tool: None,
            permissions: Arc::new(std::sync::Mutex::new(PermissionPolicy::default())),
            approver: None,
            tool_context: ToolContext::default(),
            summarizer: None,
            subagent_runner: None,
            plan_mode: false,
            block_images: false,
            auto_resize_images: true,
            retry_aborted: false,
            transport: None,
            install_telemetry: true,
            reload_count: 0,
            event_sink: None,
            abort_signal: None,
            stats: RunStats::default(),
            counters: SharedCounters::shared(),
            prune_settings: PruneSettings::default(),
            evidence: None,
            tool_ledger: Arc::new(std::sync::Mutex::new(ToolCallLedger::default())),
            pruned_tool_results: std::collections::HashSet::new(),
            base_system_prompt: system_prompt,
            pending_bash_messages: Vec::new(),
            pending_prompt_messages: Vec::new(),
            ephemeral_context: Vec::new(),
            provider_context_overhead_tokens: None,
        }
    }

    /// Restore the base prompt before each extension-aware prompt turn.
    pub fn reset_system_prompt_to_base(&mut self) {
        self.system_prompt = self.base_system_prompt.clone();
        if self.plan_mode {
            self.system_prompt.push_str("\n\n");
            self.system_prompt.push_str(crate::PLAN_MODE_APPENDIX);
        }
    }

    pub fn set_plan_mode(&mut self, on: bool) {
        self.plan_mode = on;
        self.permissions
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .plan_mode = on;
        self.reset_system_prompt_to_base();
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

    /// Hand an event to the sink without recording it. A provider closure
    /// that streams live uses this for `MessageStart` and every
    /// `MessageUpdate`, and returns `CompleteOutput { streamed_live: true }`
    /// so the loop records them into the event list without sending them a
    /// second time.
    pub fn emit_live(&self, event: AgentEvent) {
        if let Some(sink) = &self.event_sink {
            (sink.0)(&event);
        }
    }

    /// TS `evalSession.reload()` — isolated evals have no extensions; count the step.
    pub fn reload(&mut self) {
        self.reload_count = self.reload_count.saturating_add(1);
        self.aborted = false;
    }

    pub fn prompt(&mut self, text: &str) -> ChatMessage {
        self.prompt_with(text, &[])
    }

    pub fn prompt_with(
        &mut self,
        text: &str,
        images: &[davinci_ai::MessageContent],
    ) -> ChatMessage {
        self.flush_pending_bash_messages();
        // A job that finished while the user was typing is in context
        // before what they typed, so the model reads the news first.
        for notice in self.job_notice_messages() {
            self.messages.push(notice.clone());
            if let Some(session) = &mut self.session {
                let _ = session.append_entry(chat_entry(
                    "user",
                    serde_json::to_value(&notice.content).unwrap_or(Value::Null),
                    &notice.extra,
                ));
            }
            self.pending_prompt_messages.push(notice);
        }
        let mut content = vec![davinci_ai::MessageContent::Text {
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
        if self.ephemeral_context.is_empty() && self.pruned_tool_results.is_empty() {
            return convert_to_llm_for_provider(&self.messages, self.block_images);
        }
        let mut messages = pruning::project(&self.messages, &self.pruned_tool_results);
        if self.ephemeral_context.is_empty() {
            return convert_to_llm_for_provider(&messages, self.block_images);
        }
        let insertion = messages
            .iter()
            .rposition(|message| message.role == "user")
            .unwrap_or(messages.len());
        messages.splice(insertion..insertion, self.ephemeral_context.iter().cloned());
        convert_to_llm_for_provider(&messages, self.block_images)
    }

    /// The run's counters, complete.
    pub fn run_stats(&self) -> RunStats {
        let mut stats = self.stats;
        self.counters.fold_into(&mut stats);
        stats
    }

    /// The token estimate for what the provider will actually be sent:
    /// pruned tool results count as their placeholder, and an extension's
    /// ephemeral context counts although it is not in `messages`. System and
    /// tool schemas count too. This is a byte heuristic, not a tokenizer or upper bound.
    pub fn estimated_context_tokens(&self) -> u64 {
        pruning::estimate_projected_tokens(&self.messages, &self.pruned_tool_results)
            + estimate_context_tokens(&self.ephemeral_context)
            + (self.system_prompt.len() as u64).div_ceil(4)
            + self.provider_context_overhead_tokens.unwrap_or_else(|| {
                let specs = self.builtin_and_mcp_specs();
                (serde_json::to_vec(&specs)
                    .expect("tool schemas are JSON")
                    .len() as u64)
                    .div_ceil(4)
            })
    }

    /// Set once per request configuration using the actual tool catalog and
    /// any host-added system suffix. `None` restores the builtin/MCP estimate.
    pub fn set_provider_context_overhead_tokens(&mut self, tokens: Option<u64>) {
        self.provider_context_overhead_tokens = tokens;
    }

    /// Prune old tool output from the provider view when the context has
    /// grown past the start line. Idempotent between prune passes, so the
    /// provider's prompt cache keeps its prefix until the next pass.
    pub fn prune_context(&mut self) {
        let tokens = self.estimated_context_tokens();
        let plan = pruning::plan_prune(
            &self.messages,
            &self.pruned_tool_results,
            tokens,
            self.context_window,
            &self.prune_settings,
        );
        if plan.is_empty() {
            return;
        }
        let ids: std::collections::HashSet<&String> = plan.iter().collect();
        let chars: usize = self
            .messages
            .iter()
            .filter(|message| {
                message.role == "toolResult"
                    && message
                        .tool_call_id
                        .as_ref()
                        .is_some_and(|id| ids.contains(id))
            })
            .map(|message| compaction::estimate_content_chars(&message.content))
            .sum();
        self.stats.pruned_results += plan.len() as u64;
        self.stats.pruned_chars += chars as u64;
        self.pruned_tool_results.extend(plan);
    }

    /// Ids of the tool results currently pruned from the provider view.
    pub fn pruned_tool_results(&self) -> &std::collections::HashSet<String> {
        &self.pruned_tool_results
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
        extra.insert(
            "timestamp".into(),
            serde_json::json!(davinci_session::now_ms()),
        );
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

    /// The finished background jobs the model has not heard about, as the
    /// user messages that tell it (`customType: backgroundJob`). Taking
    /// them marks them announced.
    pub fn job_notice_messages(&self) -> Vec<ChatMessage> {
        let notices = self
            .tool_context
            .jobs
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .take_unannounced();
        notices
            .iter()
            .map(|notice| {
                let mut extra = serde_json::Map::new();
                extra.insert(
                    "customType".into(),
                    Value::String(JOB_NOTICE_TYPE.to_string()),
                );
                extra.insert("jobId".into(), Value::from(notice.id));
                ChatMessage {
                    role: "user".into(),
                    content: vec![davinci_ai::MessageContent::Text {
                        text: notice.message_text(),
                    }],
                    extra,
                    ..ChatMessage::default()
                }
            })
            .collect()
    }

    /// Write the ledger to the session after the `todo` tool changed it, so
    /// a resumed session opens on the same plan.
    pub fn persist_todos(&mut self) {
        let value = self
            .tool_context
            .todos
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .to_value();
        if let Some(session) = &mut self.session {
            let mut extra = serde_json::Map::new();
            extra.insert("data".into(), value);
            let _ = session.append_entry(SessionEntry {
                id: String::new(),
                entry_type: "custom".into(),
                parent_id: None,
                seq: 0,
                timestamp: 0,
                message: None,
                custom_type: Some(TODO_ENTRY_TYPE.into()),
                extra,
            });
        }
    }

    /// The ledger the session last saved, if any.
    pub fn restore_todos(&mut self) -> bool {
        let Some(session) = &self.session else {
            return false;
        };
        let Some(list) = session
            .entries
            .iter()
            .rev()
            .find(|entry| entry.custom_type.as_deref() == Some(TODO_ENTRY_TYPE))
            .and_then(|entry| entry.extra.get("data"))
            .and_then(TodoList::from_value)
        else {
            return false;
        };
        *self
            .tool_context
            .todos
            .lock()
            .unwrap_or_else(|err| err.into_inner()) = list;
        true
    }

    /// Persist and append a TypeScript extension `CustomMessage`.
    pub fn record_custom_message(&mut self, raw: &Value) -> ChatMessage {
        let content = match raw.get("content") {
            Some(Value::String(text)) => {
                vec![davinci_ai::MessageContent::Text { text: text.clone() }]
            }
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
        let timestamp = davinci_session::now_ms();
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

    /// Merge connected MCP tools into the active set and the permission class map.
    pub fn attach_mcp(&mut self, registry: crate::mcp::McpRegistry) {
        let names = registry.tool_names();
        let read_only = registry.read_only_names();
        self.tool_context.mcp = registry;
        self.apply_extension_tools(&names);
        self.permissions
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .mcp_read_only = read_only;
    }

    /// Built-in specs plus live MCP tools, filtered by `self.tools`.
    pub fn builtin_and_mcp_specs(&self) -> Vec<AgentTool> {
        let mut specs: Vec<AgentTool> = tool_specs()
            .into_iter()
            .filter(|tool| self.tools.iter().any(|name| name == &tool.name))
            .collect();
        if let Some(spec) = specs.iter_mut().find(|tool| tool.name == "mcp_read") {
            spec.description = self.tool_context.mcp.mcp_read_description();
        }
        specs.extend(
            self.tool_context
                .mcp
                .specs()
                .into_iter()
                .filter(|tool| self.tools.iter().any(|name| name == &tool.name)),
        );
        specs
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

    /// True when either the local flag or the cross-thread signal fired.
    pub fn abort_requested(&self) -> bool {
        self.aborted
            || self
                .abort_signal
                .as_ref()
                .is_some_and(|signal| signal.load(std::sync::atomic::Ordering::Relaxed))
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

    pub fn entries_since(&self, since: Option<&str>) -> Vec<davinci_session::SessionEntry> {
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
    davinci_session::build_context_entries(&session.entries, session.leaf_id.as_deref())
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
    /// The closure already sent `MessageStart` and one `MessageUpdate` per
    /// stream event through `Agent::emit_live` while the provider was
    /// answering. The loop then only records them.
    pub streamed_live: bool,
}

impl From<AssistantMessage> for CompleteOutput {
    fn from(message: AssistantMessage) -> Self {
        Self {
            message,
            stream_events: None,
            streamed_live: false,
        }
    }
}

/// The `customType` of the user message that tells the model a background
/// job finished.
pub const JOB_NOTICE_TYPE: &str = "backgroundJob";

pub fn default_system_prompt() -> String {
    [
        "You are pi, a coding assistant with read, bash, edit, and write tools. Be concise and make precise edits.",
        "Keep a todo list with the todo tool on any task of three or more steps: send the whole list, mark the step you are on active, and mark steps done as you finish them.",
        "Run builds, test suites and anything that takes more than a few seconds with bash background: true; you will be told when the job finishes, and job_output reads what it printed meanwhile.",
        "Use web_search to find pages and web_fetch to read one before quoting it. Notebooks (.ipynb) read as numbered cells; edit matches inside a cell and notebook_edit changes whole cells.",
        TOOL_USE_STRATEGY,
    ]
    .join("\n")
}

/// The orchestration rules every pi prompt carries: they are what turns a
/// thirty-turn investigation into a six-turn one. Runtime and prompt have
/// to agree — the scheduler overlaps independent calls (`scheduler.rs`),
/// `batch` hides several operations behind one boundary, `agent` fans out
/// workers — and this is the prompt's half of that agreement.
pub const TOOL_USE_STRATEGY: &str = "\
Tool-use strategy — every model turn is expensive, every tool call is cheap:
- Minimize round trips. When the next several reads, searches or listings are already known, issue them all in one response, or put them in one batch call. Never do one search or read per turn when more are obviously coming.
- Independent read-only calls in the same response run concurrently; edits and shell commands run in order. Order calls the way you need their effects.
- Read with offset/limit around what you need instead of whole files, and do not re-read a file you already have unless it changed.
- Search before reading: one grep across the tree beats opening files one by one.
- Delegate research that would flood your context to agent workers (up to 8 concurrent tasks), each with a self-contained question and a request for a short answer with file paths; do not wait on them for anything you can do meanwhile.
- Keep tool output small: use grep limit/glob, ls limit, read ranges; ask for more only when needed.";

pub fn new_message_id() -> String {
    Uuid::new_v4().to_string()
}

/// A session entry for one chat message, carrying the message's `extra`
/// fields (`customType: backgroundJob`, `jobId`) so a resumed session reads
/// a job notice back as a job notice and not as something the user typed.
pub fn chat_entry(
    role: &str,
    content: Value,
    extra: &serde_json::Map<String, Value>,
) -> SessionEntry {
    let mut entry = SessionEntry::message(role, content);
    if let Some(Value::Object(message)) = &mut entry.message {
        for (key, value) in extra {
            message.entry(key.clone()).or_insert_with(|| value.clone());
        }
    }
    entry
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chat_entry_keeps_the_message_extras() {
        let mut extra = serde_json::Map::new();
        extra.insert("customType".into(), serde_json::json!(JOB_NOTICE_TYPE));
        extra.insert("jobId".into(), serde_json::json!(3));
        let entry = chat_entry("user", serde_json::json!("hi"), &extra);
        let message = entry.message.unwrap();
        assert_eq!(message["role"], "user");
        assert_eq!(message["content"], "hi");
        assert_eq!(message["customType"], JOB_NOTICE_TYPE);
        assert_eq!(message["jobId"], 3);
    }

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
    fn context_budget_counts_system_and_active_tool_schemas() {
        let mut agent = Agent::new("");
        agent.tools.clear();
        let empty = agent.estimated_context_tokens();
        agent.system_prompt = "x".repeat(4_000);
        assert!(agent.estimated_context_tokens() >= empty + 1_000);
        let without_tools = agent.estimated_context_tokens();
        agent.tools.push("read".into());
        assert!(agent.estimated_context_tokens() > without_tools);
        agent.tools.clear();
        assert_eq!(agent.estimated_context_tokens(), without_tools);
        agent.set_provider_context_overhead_tokens(Some(3_000));
        assert_eq!(agent.estimated_context_tokens(), 4_000);
        agent.set_provider_context_overhead_tokens(None);
        assert_eq!(agent.estimated_context_tokens(), without_tools);
    }

    #[test]
    fn context_overhead_triggers_pruning_before_the_provider_request() {
        let mut agent = Agent::new("x".repeat(24_000));
        agent.tools.clear();
        agent.context_window = 10_000;
        agent.prune_settings.keep_recent = 0;
        let mut result = ChatMessage::text("toolResult", "output".repeat(1_000));
        result.tool_call_id = Some("old-read".into());
        agent.messages.push(result);
        agent.prune_context();
        assert!(agent.pruned_tool_results().contains("old-read"));
        assert_eq!(agent.stats.pruned_results, 1);
    }

    #[test]
    fn retry_cancellation_during_backoff_prevents_another_request() {
        use davinci_ai::AssistantMessage;
        use std::sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        };
        let signal = Arc::new(AtomicBool::new(false));
        let mut agent = Agent::new("");
        agent.abort_signal = Some(signal.clone());
        agent.retry_attempts = 2;
        agent.retry_base_delay_ms = 5_000;
        agent.prompt("retry cancellation");
        let mut calls = 0;
        let mut interrupter = None;
        let started = std::time::Instant::now();
        agent
            .run_loop(|_| -> Result<AssistantMessage, String> {
                calls += 1;
                if calls == 1 {
                    let signal = signal.clone();
                    interrupter = Some(std::thread::spawn(move || {
                        std::thread::sleep(std::time::Duration::from_millis(50));
                        signal.store(true, Ordering::Relaxed);
                    }));
                }
                Err("overloaded_error".into())
            })
            .unwrap();
        interrupter.unwrap().join().unwrap();
        assert_eq!(calls, 1);
        assert_eq!(agent.stats.provider_retries, 0);
        assert!(started.elapsed() < std::time::Duration::from_secs(2));
    }

    #[test]
    fn retry_never_repeats_a_permanent_request_failure() {
        let mut agent = Agent::new("");
        agent.retry_base_delay_ms = 0;
        agent.prompt("bad request");
        let mut calls = 0;
        let result = agent.run_loop(|_| -> Result<davinci_ai::AssistantMessage, String> {
            calls += 1;
            Err("400 bad request".into())
        });
        assert!(result.is_err());
        assert_eq!(calls, 1);
        assert_eq!(agent.stats.provider_retries, 0);
    }

    #[test]
    fn retry_aborted_provider_response_is_not_reported_as_success() {
        let mut agent = Agent::new("");
        agent.retry_base_delay_ms = 0;
        agent.prompt("cancel recovery");
        let mut calls = 0;
        let events = agent
            .run_loop(|_| {
                calls += 1;
                if calls == 1 {
                    return Err("overloaded_error".into());
                }
                Ok(davinci_ai::AssistantMessage {
                    id: "abort".into(),
                    role: "assistant".into(),
                    content: Vec::new(),
                    model: "fixture".into(),
                    usage: None,
                    stop_reason: Some(davinci_ai::StopReason::Aborted),
                    error_message: Some("cancelled".into()),
                })
            })
            .unwrap();
        assert!(events
            .iter()
            .any(|event| matches!(event, AgentEvent::AutoRetryEnd { success: false, .. })));
        assert_eq!(agent.stats.provider_retries, 1);
    }

    #[test]
    fn auto_retry_emits_ts_session_events() {
        use davinci_ai::{AssistantMessage, ContentBlock, StopReason};

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
        assert_eq!(agent.stats.provider_retries, 1);
        let kinds: Vec<_> = events.iter().map(AgentEvent::kind).collect();
        assert!(kinds.contains(&"auto_retry_start"));
        assert!(kinds.contains(&"auto_retry_end"));
        assert_eq!(calls, 2);
        assert_eq!(agent.last_assistant_text().as_deref(), Some("recovered"));
    }

    #[test]
    fn a_closure_that_streamed_live_is_recorded_but_not_resent_to_the_sink() {
        use davinci_ai::{AssistantMessage, AssistantMessageEvent, ContentBlock, StopReason};
        use std::sync::{Arc, Mutex};

        let seen: Arc<Mutex<Vec<&'static str>>> = Arc::new(Mutex::new(Vec::new()));
        let sink_seen = seen.clone();
        let mut agent = Agent::new(default_system_prompt());
        agent.event_sink = Some(EventSink(Arc::new(move |event: &AgentEvent| {
            sink_seen.lock().unwrap().push(event.kind());
        })));
        agent.prompt("stream me");
        let events = agent
            .run_loop(|current| {
                let message = AssistantMessage {
                    id: "live".into(),
                    role: "assistant".into(),
                    content: vec![ContentBlock::Text {
                        text: "hi there".into(),
                    }],
                    model: "fixture".into(),
                    usage: None,
                    stop_reason: Some(StopReason::Stop),
                    error_message: None,
                };
                let stream_events = davinci_ai::events_from_complete(&message);
                let chat = std::sync::Arc::new(davinci_ai::assistant_to_chat(&message));
                current.emit_live(AgentEvent::MessageStart {
                    message: (*chat).clone(),
                });
                for event in &stream_events {
                    current.emit_live(AgentEvent::MessageUpdate {
                        message: chat.clone(),
                        assistant_message_event: event.clone(),
                    });
                }
                let _: &AssistantMessageEvent = &stream_events[0];
                Ok(CompleteOutput {
                    message,
                    stream_events: Some(stream_events),
                    streamed_live: true,
                })
            })
            .unwrap();
        let recorded: Vec<_> = events.iter().map(AgentEvent::kind).collect();
        let sunk = seen.lock().unwrap().clone();
        // The record and the sink saw the same sequence, once each: no
        // update was sent twice and none was dropped.
        assert_eq!(recorded, sunk);
        assert_eq!(
            recorded
                .iter()
                .filter(|kind| **kind == "message_start")
                .count(),
            2 // the user prompt and the assistant reply
        );
        assert_eq!(
            recorded
                .iter()
                .filter(|kind| **kind == "message_update")
                .count(),
            stream_events_len("hi there")
        );
        assert_eq!(agent.last_assistant_text().as_deref(), Some("hi there"));
    }

    fn stream_events_len(text: &str) -> usize {
        use davinci_ai::{AssistantMessage, ContentBlock, StopReason};
        davinci_ai::events_from_complete(&AssistantMessage {
            id: "n".into(),
            role: "assistant".into(),
            content: vec![ContentBlock::Text { text: text.into() }],
            model: "fixture".into(),
            usage: None,
            stop_reason: Some(StopReason::Stop),
            error_message: None,
        })
        .len()
    }

    #[test]
    fn agent_loop_emits_ts_event_names_and_runs_tools() {
        use davinci_ai::{AssistantMessage, ContentBlock, StopReason};

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
                    davinci_ai::AssistantMessageEvent::TextDelta { .. } => "text_delta",
                    davinci_ai::AssistantMessageEvent::ThinkingDelta { .. } => "thinking_delta",
                    davinci_ai::AssistantMessageEvent::ToolcallStart { .. } => "toolcall_start",
                    davinci_ai::AssistantMessageEvent::ToolcallEnd { .. } => "toolcall_end",
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

    /// A scripted model: one tool call per entry, then `done`.
    fn scripted_tool_calls(
        calls: Vec<(&'static str, serde_json::Value)>,
    ) -> impl FnMut(&Agent) -> Result<davinci_ai::AssistantMessage, String> {
        use davinci_ai::{AssistantMessage, ContentBlock, StopReason};
        let mut remaining = calls.into_iter();
        let mut index = 0;
        move |_current| {
            index += 1;
            match remaining.next() {
                Some((name, arguments)) => Ok(AssistantMessage {
                    id: format!("a{index}"),
                    role: "assistant".into(),
                    content: vec![ContentBlock::ToolCall {
                        id: format!("call_{index}"),
                        name: name.into(),
                        arguments,
                    }],
                    model: "fixture".into(),
                    usage: None,
                    stop_reason: Some(StopReason::ToolUse),
                    error_message: None,
                }),
                None => Ok(AssistantMessage {
                    id: format!("a{index}"),
                    role: "assistant".into(),
                    content: vec![ContentBlock::Text {
                        text: "done".into(),
                    }],
                    model: "fixture".into(),
                    usage: None,
                    stop_reason: Some(StopReason::Stop),
                    error_message: None,
                }),
            }
        }
    }

    fn tool_outcomes(events: &[AgentEvent]) -> Vec<(String, bool, String)> {
        events
            .iter()
            .filter_map(|event| match event {
                AgentEvent::ToolExecutionEnd {
                    tool_name,
                    is_error,
                    result,
                    ..
                } => Some((
                    tool_name.clone(),
                    *is_error,
                    result.as_str().unwrap_or_default().to_string(),
                )),
                _ => None,
            })
            .collect()
    }

    /// A scripted model that emits every call of one entry in a single
    /// assistant message, then `done`.
    fn scripted_batches(
        batches: Vec<Vec<(&'static str, serde_json::Value)>>,
    ) -> impl FnMut(&Agent) -> Result<davinci_ai::AssistantMessage, String> {
        use davinci_ai::{AssistantMessage, ContentBlock, StopReason};
        let mut remaining = batches.into_iter();
        let mut index = 0;
        move |_current| {
            index += 1;
            match remaining.next() {
                Some(calls) => Ok(AssistantMessage {
                    id: format!("a{index}"),
                    role: "assistant".into(),
                    content: calls
                        .into_iter()
                        .enumerate()
                        .map(|(n, (name, arguments))| ContentBlock::ToolCall {
                            id: format!("call_{index}_{n}"),
                            name: name.into(),
                            arguments,
                        })
                        .collect(),
                    model: "fixture".into(),
                    usage: None,
                    stop_reason: Some(StopReason::ToolUse),
                    error_message: None,
                }),
                None => Ok(AssistantMessage {
                    id: format!("a{index}"),
                    role: "assistant".into(),
                    content: vec![ContentBlock::Text {
                        text: "done".into(),
                    }],
                    model: "fixture".into(),
                    usage: None,
                    stop_reason: Some(StopReason::Stop),
                    error_message: None,
                }),
            }
        }
    }

    #[test]
    fn parallel_workers_in_one_message_overlap_and_answer_in_order() {
        use std::time::{Duration, Instant};
        let mut agent = Agent::new(default_system_prompt());
        agent.subagent_runner = Some(SubagentRunner::new(|req| {
            std::thread::sleep(Duration::from_millis(80));
            Ok(format!("answer:{}", req.prompt))
        }));
        agent.prompt("go");
        let start = Instant::now();
        let events = agent
            .run_loop(scripted_batches(vec![vec![
                ("agent", serde_json::json!({"prompt": "one"})),
                ("agent", serde_json::json!({"prompt": "two"})),
                ("agent", serde_json::json!({"prompt": "three"})),
            ]]))
            .unwrap();
        let elapsed = start.elapsed();
        assert!(
            elapsed < Duration::from_millis(200),
            "three 80 ms workers took {elapsed:?}: they ran one after another"
        );
        let results: Vec<String> = agent
            .messages
            .iter()
            .filter(|message| message.role == "toolResult")
            .map(|message| content_text(&message.content))
            .collect();
        assert_eq!(results, ["answer:one", "answer:two", "answer:three"]);
        // Every start precedes every end, and the ends are recorded in
        // source order even though the workers finished together.
        let kinds: Vec<(&str, String)> = events
            .iter()
            .filter_map(|event| match event {
                AgentEvent::ToolExecutionStart { tool_call_id, .. } => {
                    Some(("start", tool_call_id.clone()))
                }
                AgentEvent::ToolExecutionEnd { tool_call_id, .. } => {
                    Some(("end", tool_call_id.clone()))
                }
                _ => None,
            })
            .collect();
        assert_eq!(
            kinds,
            [
                ("start", "call_1_0".to_string()),
                ("start", "call_1_1".to_string()),
                ("start", "call_1_2".to_string()),
                ("end", "call_1_0".to_string()),
                ("end", "call_1_1".to_string()),
                ("end", "call_1_2".to_string()),
            ]
        );
        let stats = agent.run_stats();
        assert_eq!(stats.model_turns, 2);
        assert_eq!(stats.tool_batches, 1);
        assert_eq!(stats.tool_calls, 3);
        assert_eq!(stats.max_batch_width, 3);
        assert_eq!(stats.parallel_groups, 1);
        assert_eq!(stats.subagents, 3);
    }

    #[test]
    fn sequential_mode_runs_workers_one_at_a_time() {
        use std::time::{Duration, Instant};
        let mut agent = Agent::new(default_system_prompt());
        agent.tool_execution_mode = ToolExecutionMode::Sequential;
        agent.subagent_runner = Some(SubagentRunner::new(|_| {
            std::thread::sleep(Duration::from_millis(40));
            Ok("x".into())
        }));
        agent.prompt("go");
        let start = Instant::now();
        agent
            .run_loop(scripted_batches(vec![vec![
                ("agent", serde_json::json!({"prompt": "one"})),
                ("agent", serde_json::json!({"prompt": "two"})),
            ]]))
            .unwrap();
        assert!(start.elapsed() >= Duration::from_millis(80));
        assert_eq!(agent.run_stats().parallel_groups, 0);
    }

    #[test]
    fn an_edit_is_a_barrier_so_the_read_after_it_sees_the_change() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "before").unwrap();
        let mut agent = Agent::new(default_system_prompt());
        agent.cwd = dir.path().to_path_buf();
        agent.prompt("go");
        agent
            .run_loop(scripted_batches(vec![vec![
                ("read", serde_json::json!({"path": "a.txt"})),
                (
                    "edit",
                    serde_json::json!({"path": "a.txt", "oldText": "before", "newText": "after"}),
                ),
                ("read", serde_json::json!({"path": "a.txt"})),
            ]]))
            .unwrap();
        let results: Vec<String> = agent
            .messages
            .iter()
            .filter(|message| message.role == "toolResult")
            .map(|message| content_text(&message.content))
            .collect();
        assert_eq!(results.len(), 3, "{results:?}");
        assert!(results[0].contains("before"), "{results:?}");
        assert!(results[2].contains("after"), "{results:?}");
    }

    #[test]
    fn a_batch_runs_its_operations_behind_one_result() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.txt"), "alpha\nneedle\n").unwrap();
        std::fs::write(dir.path().join("b.txt"), "beta\n").unwrap();
        let mut agent = Agent::new(default_system_prompt());
        agent.cwd = dir.path().to_path_buf();
        agent.prompt("go");
        let events = agent
            .run_loop(scripted_batches(vec![vec![(
                "batch",
                serde_json::json!({"operations": [
                    {"tool": "read", "args": {"path": "a.txt"}},
                    {"tool": "grep", "args": {"pattern": "needle", "path": "."}},
                    {"tool": "ls", "args": {"path": "."}},
                    {"tool": "batch", "args": {"operations": []}},
                    {"tool": "nope", "args": {}},
                ]}),
            )]]))
            .unwrap();
        let outcomes = tool_outcomes(&events);
        assert_eq!(outcomes.len(), 1, "{outcomes:?}");
        let (name, is_error, text) = &outcomes[0];
        assert_eq!(name, "batch");
        assert!(!is_error);
        assert!(text.starts_with("batch: 5/5 operations ran"), "{text}");
        assert!(text.contains("[1] read path=\"a.txt\" → ok"), "{text}");
        assert!(text.contains("alpha"), "{text}");
        assert!(
            text.contains("[2] grep path=\".\" pattern=\"needle\""),
            "{text}"
        );
        assert!(text.contains("a.txt:2: needle"), "{text}");
        assert!(text.contains("[3] ls"), "{text}");
        assert!(text.contains("b.txt"), "{text}");
        assert!(text.contains("[4] batch operations=… → error"), "{text}");
        assert!(text.contains("cannot run inside a batch"), "{text}");
        assert!(text.contains("[5] nope → error"), "{text}");
        assert!(text.contains("Unknown tool: nope"), "{text}");
        let stats = agent.run_stats();
        assert_eq!(stats.tool_calls, 1);
        assert_eq!(stats.batch_operations, 5);
        // One model-visible tool result for five operations.
        assert_eq!(
            agent
                .messages
                .iter()
                .filter(|message| message.role == "toolResult")
                .count(),
            1
        );
    }

    #[test]
    fn a_batch_operation_is_gated_like_a_direct_call() {
        use std::sync::Arc;
        let dir = tempfile::tempdir().unwrap();
        let mut agent = Agent::new(default_system_prompt());
        agent.cwd = dir.path().to_path_buf();
        agent.permissions = Arc::new(std::sync::Mutex::new(PermissionPolicy::new(
            PermissionMode::ReadOnly,
        )));
        agent.prompt("go");
        let events = agent
            .run_loop(scripted_batches(vec![vec![(
                "batch",
                serde_json::json!({"operations": [
                    {"tool": "write", "args": {"path": "x.txt", "content": "no"}},
                    {"tool": "ls", "args": {}},
                ]}),
            )]]))
            .unwrap();
        let outcomes = tool_outcomes(&events);
        let text = &outcomes[0].2;
        assert!(
            text.contains("[1] write path=\"x.txt\" content=\"no\" → error"),
            "{text}"
        );
        assert!(text.contains("read-only"), "{text}");
        assert!(!dir.path().join("x.txt").exists());
        assert!(text.contains("[2] ls → ok"), "{text}");
    }

    #[test]
    fn overflowing_batch_output_goes_to_the_evidence_store() {
        let dir = tempfile::tempdir().unwrap();
        let big = "z".repeat(20 * 1024);
        std::fs::write(dir.path().join("big.txt"), &big).unwrap();
        let mut agent = Agent::new(default_system_prompt());
        agent.cwd = dir.path().to_path_buf();
        agent.evidence = Some(EvidenceStore::new(dir.path().join("evidence")));
        agent.prompt("go");
        let events = agent
            .run_loop(scripted_batches(vec![vec![(
                "batch",
                serde_json::json!({"operations": [
                    {"tool": "read", "args": {"path": "big.txt"}},
                ]}),
            )]]))
            .unwrap();
        let text = &tool_outcomes(&events)[0].2;
        assert!(
            text.len() < 14 * 1024,
            "visible result is {} bytes",
            text.len()
        );
        assert!(text.contains("full output saved to"), "{text}");
        let path = text
            .split("full output saved to ")
            .nth(1)
            .and_then(|rest| rest.split(" — ").next())
            .unwrap();
        let saved = std::fs::read_to_string(path.trim()).unwrap();
        assert!(saved.contains(&big));
        assert_eq!(agent.run_stats().evidence_files, 1);
    }

    #[test]
    fn old_tool_output_is_pruned_from_the_provider_view_but_kept_in_history() {
        let dir = tempfile::tempdir().unwrap();
        let body = "q".repeat(4_000);
        std::fs::write(dir.path().join("f.txt"), &body).unwrap();
        let mut agent = Agent::new(default_system_prompt());
        agent.cwd = dir.path().to_path_buf();
        agent.auto_compaction = false;
        // ~1000 tokens per read; a 6k window starts pruning past 3k.
        agent.context_window = 6_000;
        agent.prune_settings.keep_recent = 2;
        agent.prompt("go");
        let batches: Vec<Vec<(&'static str, serde_json::Value)>> = (0..6)
            .map(|_| vec![("read", serde_json::json!({"path": "f.txt"}))])
            .collect();
        agent.run_loop(scripted_batches(batches)).unwrap();
        let stats = agent.run_stats();
        assert!(stats.pruned_results >= 2, "{stats:?}");
        assert!(stats.pruned_chars >= 8_000, "{stats:?}");
        // History is whole.
        let full: Vec<&ChatMessage> = agent
            .messages
            .iter()
            .filter(|message| message.role == "toolResult")
            .collect();
        assert_eq!(full.len(), 6);
        assert!(full
            .iter()
            .all(|message| content_text(&message.content).contains(&body)));
        // The provider view is not.
        let projected = agent.messages_for_provider();
        let pruned = projected
            .iter()
            .filter(|message| message.role == "toolResult")
            .filter(|message| content_text(&message.content).contains("pruned to save context"))
            .count();
        assert_eq!(pruned as u64, stats.pruned_results);
        // The newest results are untouched.
        let last = projected
            .iter()
            .rev()
            .find(|message| message.role == "toolResult")
            .unwrap();
        assert!(content_text(&last.content).contains(&body));
        assert!(agent.estimated_context_tokens() < crate::estimate_context_tokens(&agent.messages));
    }

    #[test]
    fn ask_mode_asks_the_approver_for_edits_and_never_for_reads() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::Arc;

        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("note.txt"), "hello").unwrap();
        let asked = Arc::new(AtomicUsize::new(0));
        let seen = asked.clone();
        let mut agent = Agent::new(default_system_prompt());
        agent.cwd = dir.path().to_path_buf();
        agent.permissions = Arc::new(std::sync::Mutex::new(PermissionPolicy::new(
            PermissionMode::Ask,
        )));
        agent.approver = Some(ToolApprover(Arc::new(move |request| {
            seen.fetch_add(1, Ordering::SeqCst);
            assert_eq!(request.tool, "write");
            assert_eq!(request.summary, "write · out.txt");
            ToolApprovalDecision::AllowForSession
        })));
        agent.prompt("go");
        let events = agent
            .run_loop(scripted_tool_calls(vec![
                ("read", serde_json::json!({"path": "note.txt"})),
                (
                    "write",
                    serde_json::json!({"path": "out.txt", "content": "a"}),
                ),
                (
                    "write",
                    serde_json::json!({"path": "out.txt", "content": "b"}),
                ),
            ]))
            .unwrap();
        let outcomes = tool_outcomes(&events);
        assert_eq!(outcomes.len(), 3, "{outcomes:?}");
        assert!(
            outcomes.iter().all(|(_, is_error, _)| !is_error),
            "{outcomes:?}"
        );
        // The read never asked; the first write did and the second was
        // covered by the session rule the answer added.
        assert_eq!(asked.load(Ordering::SeqCst), 1);
        assert_eq!(
            std::fs::read_to_string(dir.path().join("out.txt")).unwrap(),
            "b"
        );
        assert_eq!(
            agent
                .permissions
                .lock()
                .unwrap()
                .session_allow
                .iter()
                .map(ToString::to_string)
                .collect::<Vec<_>>(),
            ["write"]
        );
    }

    #[test]
    fn read_only_refuses_an_edit_without_asking_and_the_loop_goes_on() {
        use std::sync::Arc;

        let dir = tempfile::tempdir().unwrap();
        let mut agent = Agent::new(default_system_prompt());
        agent.cwd = dir.path().to_path_buf();
        agent.permissions = Arc::new(std::sync::Mutex::new(PermissionPolicy::new(
            PermissionMode::ReadOnly,
        )));
        agent.approver = Some(ToolApprover(Arc::new(|_request| {
            panic!("read-only never asks")
        })));
        agent.prompt("go");
        let events = agent
            .run_loop(scripted_tool_calls(vec![(
                "write",
                serde_json::json!({"path": "out.txt", "content": "a"}),
            )]))
            .unwrap();
        let outcomes = tool_outcomes(&events);
        assert_eq!(outcomes.len(), 1);
        assert!(outcomes[0].1, "{outcomes:?}");
        assert!(
            outcomes[0]
                .2
                .contains("not allowed in permission mode `read-only`"),
            "{}",
            outcomes[0].2
        );
        assert!(!dir.path().join("out.txt").exists());
        assert_eq!(agent.last_assistant_text().as_deref(), Some("done"));
    }

    #[test]
    fn a_declined_call_tells_the_model_the_user_said_no() {
        use std::sync::Arc;

        let dir = tempfile::tempdir().unwrap();
        let mut agent = Agent::new(default_system_prompt());
        agent.cwd = dir.path().to_path_buf();
        agent.permissions = Arc::new(std::sync::Mutex::new(PermissionPolicy::new(
            PermissionMode::Ask,
        )));
        agent.approver = Some(ToolApprover(Arc::new(|_request| {
            ToolApprovalDecision::Deny
        })));
        agent.prompt("go");
        let events = agent
            .run_loop(scripted_tool_calls(vec![(
                "write",
                serde_json::json!({"path": "out.txt", "content": "a"}),
            )]))
            .unwrap();
        let outcomes = tool_outcomes(&events);
        assert_eq!(
            outcomes[0].2,
            "Permission denied: the user declined `write · out.txt`."
        );
        assert!(!dir.path().join("out.txt").exists());
    }

    #[test]
    fn without_an_approver_ask_mode_fails_closed_and_says_how_to_open_it() {
        use std::sync::Arc;

        let dir = tempfile::tempdir().unwrap();
        let mut agent = Agent::new(default_system_prompt());
        agent.cwd = dir.path().to_path_buf();
        agent.permissions = Arc::new(std::sync::Mutex::new(PermissionPolicy::new(
            PermissionMode::Ask,
        )));
        agent.prompt("go");
        let events = agent
            .run_loop(scripted_tool_calls(vec![(
                "bash",
                serde_json::json!({"command": "git status"}),
            )]))
            .unwrap();
        let outcomes = tool_outcomes(&events);
        assert!(outcomes[0].1);
        assert!(
            outcomes[0].2.starts_with(
                "Permission denied: `bash · git status` needs approval in permission mode `ask`, and this run cannot ask."
            ),
            "{}",
            outcomes[0].2
        );
        assert!(
            outcomes[0].2.contains("`bash(git status *)`"),
            "{}",
            outcomes[0].2
        );
    }

    #[test]
    fn the_library_default_runs_every_tool_as_vendor_pi_does() {
        let dir = tempfile::tempdir().unwrap();
        let mut agent = Agent::new(default_system_prompt());
        agent.cwd = dir.path().to_path_buf();
        agent.prompt("go");
        let events = agent
            .run_loop(scripted_tool_calls(vec![(
                "write",
                serde_json::json!({"path": "out.txt", "content": "a"}),
            )]))
            .unwrap();
        assert!(tool_outcomes(&events)
            .iter()
            .all(|(_, is_error, _)| !is_error));
        assert!(dir.path().join("out.txt").exists());
    }

    #[test]
    fn event_sink_receives_events_as_run_loop_emits_them() {
        use davinci_ai::{AssistantMessage, ContentBlock, StopReason};

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
        use davinci_ai::{AssistantMessage, ContentBlock, StopReason};

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
        use davinci_ai::MessageContent;
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
                usage: davinci_protocol::Usage {
                    input: 1,
                    output: 2,
                    total_tokens: 3,
                    ..davinci_protocol::Usage::default()
                },
                stop_reason: Some(davinci_ai::StopReason::Stop),
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
        use davinci_ai::{AssistantMessage, ContentBlock, StopReason};
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
        use davinci_ai::{AssistantMessage, ContentBlock, StopReason};

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
            Some(davinci_ai::MessageContent::Image { .. })
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
            Some(davinci_ai::MessageContent::Image { .. })
        ));
    }

    #[test]
    fn a_background_job_is_announced_to_the_model_before_its_next_completion() {
        let dir = tempfile::tempdir().unwrap();
        let mut agent = Agent::new(default_system_prompt());
        agent.cwd = dir.path().to_path_buf();
        agent.prompt("build it");
        let events = agent
            .run_loop(scripted_tool_calls(vec![
                (
                    "bash",
                    serde_json::json!({"command": "echo built", "background": true}),
                ),
                // Give the job time to exit; the next completion sees it.
                ("job_output", serde_json::json!({"jobId": 1, "wait": 10})),
            ]))
            .unwrap();
        let outcomes = tool_outcomes(&events);
        assert!(
            outcomes[0]
                .2
                .starts_with("Started background job 1: `echo built`"),
            "{:?}",
            outcomes[0]
        );
        assert!(outcomes[1].2.contains("built"), "{:?}", outcomes[1]);
        assert!(outcomes[1].2.contains("[job 1 exit 0"), "{:?}", outcomes[1]);
        // The notice is a user message with its custom type, injected once,
        // before the completion that followed the job's end.
        let notices: Vec<&ChatMessage> = agent
            .messages
            .iter()
            .filter(|message| {
                message.extra.get("customType") == Some(&serde_json::json!(JOB_NOTICE_TYPE))
            })
            .collect();
        assert_eq!(notices.len(), 1);
        let text = content_text(&notices[0].content);
        assert!(
            text.starts_with("[background job 1 finished · exit 0 · "),
            "{text}"
        );
        assert!(text.contains("] echo built\n    built"), "{text}");
        let notice_index = agent
            .messages
            .iter()
            .position(|message| message.extra.contains_key("customType"))
            .unwrap();
        let last_assistant = agent
            .messages
            .iter()
            .rposition(|message| message.role == "assistant")
            .unwrap();
        assert!(
            notice_index < last_assistant,
            "the notice came before the final reply"
        );
        // Nothing left to announce, and the events carried it as a message.
        assert!(agent.job_notice_messages().is_empty());
        assert!(events.iter().any(|event| matches!(
            event,
            AgentEvent::MessageStart { message } if message.extra.contains_key("customType")
        )));
    }

    #[test]
    fn a_job_that_ends_between_turns_is_prepended_to_the_next_prompt() {
        let dir = tempfile::tempdir().unwrap();
        let mut agent = Agent::new(default_system_prompt());
        agent.cwd = dir.path().to_path_buf();
        let result = execute_tool_with(
            dir.path(),
            "bash",
            &serde_json::json!({"command": "echo later", "background": true}),
            &agent.tool_context,
        )
        .unwrap();
        assert!(result.content.starts_with("Started background job 1"));
        execute_tool_with(
            dir.path(),
            "job_output",
            &serde_json::json!({"jobId": 1, "wait": 10}),
            &agent.tool_context,
        )
        .unwrap();
        agent.prompt("what happened?");
        let roles: Vec<(String, bool)> = agent
            .messages
            .iter()
            .map(|message| {
                (
                    message.role.clone(),
                    message.extra.contains_key("customType"),
                )
            })
            .collect();
        assert_eq!(
            roles,
            vec![("user".to_string(), true), ("user".to_string(), false)]
        );
        assert_eq!(agent.pending_prompt_messages.len(), 2);
    }

    #[test]
    fn the_todo_ledger_is_kept_on_the_agent_and_written_to_the_session() {
        let dir = tempfile::tempdir().unwrap();
        let session = JsonlSession::create(dir.path(), "/tmp/project", Some("todo")).unwrap();
        let mut agent = Agent::new(default_system_prompt());
        agent.session = Some(session);
        agent.cwd = dir.path().to_path_buf();
        agent.prompt("plan it");
        let events = agent
            .run_loop(scripted_tool_calls(vec![(
                "todo",
                serde_json::json!({"items": [
                    {"text": "read the parser", "status": "done"},
                    {"text": "add the branch", "status": "active"},
                    {"text": "run the tests", "status": "pending"}
                ]}),
            )]))
            .unwrap();
        let outcomes = tool_outcomes(&events);
        assert_eq!(
            outcomes[0].2,
            "3 items · 1 done · 1 active\n✓ read the parser\n◉ add the branch\n○ run the tests"
        );
        assert_eq!(
            agent.tool_context.todos.lock().unwrap().summary(),
            "1 of 3 done"
        );
        let path = agent.session.as_ref().unwrap().path.clone();
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("\"customType\":\"todo\""), "{raw}");

        // A fresh agent on the same session finds the ledger again.
        let reopened = JsonlSession::open(&path).unwrap();
        let mut again = Agent::new("x");
        again.session = Some(reopened);
        assert!(again.restore_todos());
        assert_eq!(again.tool_context.todos.lock().unwrap().items.len(), 3);
        let mut empty = Agent::new("x");
        assert!(!empty.restore_todos());
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

    #[test]
    fn batch_duplicate_mutating_calls_executes_once() {
        use davinci_ai::{AssistantMessage, ContentBlock, StopReason};
        use std::sync::atomic::{AtomicUsize, Ordering};

        let counter = Arc::new(AtomicUsize::new(0));
        let c = Arc::clone(&counter);
        let mut agent = Agent::new(default_system_prompt());
        agent.custom_tool_executor = Some(CustomToolExecutor::new(move |_cwd, _tool, _args| {
            c.fetch_add(1, Ordering::SeqCst);
            Ok(crate::ToolResult {
                content: "mutated".into(),
                is_error: false,
                details: None,
            })
        }));
        agent.tools.push("custom_mutating".into());
        agent.prompt("run duplicate batch");

        let mut turn = 0;
        let _ = agent.run_loop(|_| {
            turn += 1;
            if turn == 1 {
                Ok(AssistantMessage {
                    id: "a1".into(),
                    role: "assistant".into(),
                    content: vec![
                        ContentBlock::ToolCall {
                            id: "dup_call_1".into(),
                            name: "custom_mutating".into(),
                            arguments: serde_json::json!({"action": "write"}),
                        },
                        ContentBlock::ToolCall {
                            id: "dup_call_1".into(),
                            name: "custom_mutating".into(),
                            arguments: serde_json::json!({"action": "write"}),
                        },
                    ],
                    model: "fixture".into(),
                    usage: None,
                    stop_reason: Some(StopReason::ToolUse),
                    error_message: None,
                })
            } else {
                Ok(AssistantMessage {
                    id: "a2".into(),
                    role: "assistant".into(),
                    content: vec![ContentBlock::Text {
                        text: "done".into(),
                    }],
                    model: "fixture".into(),
                    usage: None,
                    stop_reason: Some(StopReason::Stop),
                    error_message: None,
                })
            }
        });

        // The mutating custom tool must have executed exactly once!
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn concurrent_identical_calls_followers_receive_terminal_result() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::time::Duration;

        let counter = Arc::new(AtomicUsize::new(0));
        let c = Arc::clone(&counter);
        let mut agent = Agent::new(default_system_prompt());
        agent.custom_tool_executor = Some(CustomToolExecutor::new(move |_cwd, _tool, _args| {
            c.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(50));
            Ok(crate::ToolResult {
                content: "finished_concurrent".into(),
                is_error: false,
                details: None,
            })
        }));
        agent.tools.push("custom_slow".into());
        let agent = Arc::new(agent);

        let mut handles = Vec::new();
        for _ in 0..4 {
            let a = Arc::clone(&agent);
            handles.push(std::thread::spawn(move || {
                let cwd = std::path::PathBuf::from(".");
                a.run_prepared_call(
                    &cwd,
                    "concurrent_call_id",
                    "custom_slow",
                    &serde_json::json!({"test": 1}),
                    0,
                )
            }));
        }

        for h in handles {
            let res = h.join().unwrap();
            assert_eq!(res.content, "finished_concurrent");
            assert!(!res.is_error);
        }

        // Executed exactly once!
        assert_eq!(counter.load(Ordering::SeqCst), 1);
    }
}
