use davinci_ai::{content_text, ChatMessage, MessageContent, StopReason};
use davinci_protocol::Usage;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

pub const DEFAULT_RESERVE_TOKENS: u64 = 16_384;
pub const DEFAULT_KEEP_RECENT_TOKENS: u64 = 20_000;
const ESTIMATED_IMAGE_CHARS: usize = 4800;
const TOOL_RESULT_MAX_CHARS: usize = 2000;

pub const SUMMARIZATION_SYSTEM_PROMPT: &str = "You are a context summarization assistant. Your task is to read a conversation between a user and an AI assistant, then produce a structured summary following the exact format specified.\n\nDo NOT continue the conversation. Do NOT respond to any questions in the conversation. ONLY output the structured summary.";

pub const SUMMARIZATION_PROMPT: &str = "The messages above are a conversation to summarize. Create a structured context checkpoint summary that another LLM will use to continue the work.\n\nUse this EXACT format:\n\n## Goal\n[What is the user trying to accomplish? Can be multiple items if the session covers different tasks.]\n\n## Constraints & Preferences\n- [Any constraints, preferences, or requirements mentioned by user]\n- [Or \"(none)\" if none were mentioned]\n\n## Progress\n### Done\n- [x] [Completed tasks/changes]\n\n### In Progress\n- [ ] [Current work]\n\n### Blocked\n- [Issues preventing progress, if any]\n\n## Key Decisions\n- **[Decision]**: [Brief rationale]\n\n## Next Steps\n1. [Ordered list of what should happen next]\n\n## Critical Context\n- [Any data, examples, or references needed to continue]\n- [Or \"(none)\" if not applicable]\n\nKeep each section concise. Preserve exact file paths, function names, and error messages.";

pub const UPDATE_SUMMARIZATION_PROMPT: &str = "The messages above are NEW conversation messages to incorporate into the existing summary provided in <previous-summary> tags.\n\nUpdate the existing structured summary with new information. RULES:\n- PRESERVE all existing information from the previous summary\n- ADD new progress, decisions, and context from the new messages\n- UPDATE the Progress section: move items from \"In Progress\" to \"Done\" when completed\n- UPDATE \"Next Steps\" based on what was accomplished\n- PRESERVE exact file paths, function names, and error messages\n- If something is no longer relevant, you may remove it\n\nUse this EXACT format:\n\n## Goal\n[Preserve existing goals, add new ones if the task expanded]\n\n## Constraints & Preferences\n- [Preserve existing, add new ones discovered]\n\n## Progress\n### Done\n- [x] [Include previously done items AND newly completed items]\n\n### In Progress\n- [ ] [Current work - update based on progress]\n\n### Blocked\n- [Current blockers - remove if resolved]\n\n## Key Decisions\n- **[Decision]**: [Brief rationale] (preserve all previous, add new)\n\n## Next Steps\n1. [Update based on current state]\n\n## Critical Context\n- [Preserve important context, add new if needed]\n\nKeep each section concise. Preserve exact file paths, function names, and error messages.";

pub const TURN_PREFIX_SUMMARIZATION_PROMPT: &str = "This is the PREFIX of a turn that was too large to keep. The SUFFIX (recent work) is retained.\n\nSummarize the prefix to provide context for the retained suffix:\n\n## Original Request\n[What did the user ask for in this turn?]\n\n## Early Progress\n- [Key decisions and work done in the prefix]\n\n## Context for Suffix\n- [Information needed to understand the retained recent work]\n\nBe concise. Focus on what's needed to understand the kept suffix.";

pub const COMPACTION_SUMMARY_PREFIX: &str =
    "The conversation history before this point was compacted into the following summary:\n\n<summary>\n";
pub const COMPACTION_SUMMARY_SUFFIX: &str = "\n</summary>";
pub const BRANCH_SUMMARY_PREFIX: &str =
    "The following is a summary of a branch that this conversation came back from:\n\n<summary>\n";
pub const BRANCH_SUMMARY_SUFFIX: &str = "</summary>";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CompactionThreshold {
    Tokens(u64),
    Percent(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompactionSettings {
    pub enabled: bool,
    pub reserve_tokens: u64,
    pub keep_recent_tokens: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub threshold: Option<CompactionThreshold>,
}

impl Default for CompactionSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            reserve_tokens: DEFAULT_RESERVE_TOKENS,
            keep_recent_tokens: DEFAULT_KEEP_RECENT_TOKENS,
            threshold: None,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct FileOperations {
    pub read: Vec<String>,
    pub written: Vec<String>,
    pub edited: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompactionDetails {
    #[serde(rename = "readFiles")]
    pub read_files: Vec<String>,
    #[serde(rename = "modifiedFiles")]
    pub modified_files: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionResult {
    pub summary: String,
    pub messages: Vec<ChatMessage>,
    pub compacted: bool,
    pub details: CompactionDetails,
    #[serde(rename = "firstKeptEntryId", default)]
    pub first_kept_entry_id: String,
    #[serde(rename = "tokensBefore", default)]
    pub tokens_before: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage: Option<Usage>,
}

#[derive(Debug, Clone)]
pub struct SummarizeRequest {
    pub system: String,
    pub prompt: String,
    pub max_tokens: u64,
    pub label: String,
    pub provider: String,
    pub model_id: String,
}

#[derive(Debug, Clone)]
pub struct SummarizeResponse {
    pub text: String,
    pub usage: Usage,
    pub stop_reason: Option<StopReason>,
    pub error_message: Option<String>,
    pub has_tool_call: bool,
}

type SummarizerFn = dyn Fn(&SummarizeRequest) -> Result<SummarizeResponse, String> + Send + Sync;

#[derive(Clone)]
pub struct Summarizer {
    inner: Arc<SummarizerFn>,
}

impl std::fmt::Debug for Summarizer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Summarizer")
    }
}

impl Summarizer {
    pub fn new<F>(f: F) -> Self
    where
        F: Fn(&SummarizeRequest) -> Result<SummarizeResponse, String> + Send + Sync + 'static,
    {
        Self { inner: Arc::new(f) }
    }

    pub fn summarize(&self, request: &SummarizeRequest) -> Result<SummarizeResponse, String> {
        (self.inner)(request)
    }
}

pub fn calculate_context_tokens(
    total_tokens: u64,
    input: u64,
    output: u64,
    cache_read: u64,
    cache_write: u64,
) -> u64 {
    if total_tokens > 0 {
        total_tokens
    } else {
        input + output + cache_read + cache_write
    }
}

pub fn estimate_content_chars(content: &[MessageContent]) -> usize {
    let mut chars = 0;
    for block in content {
        match block {
            MessageContent::Text { text } => chars += text.len(),
            MessageContent::Thinking { thinking, .. } => chars += thinking.len(),
            MessageContent::Image { .. } => chars += ESTIMATED_IMAGE_CHARS,
            MessageContent::ToolCall {
                name, arguments, ..
            } => {
                chars += name.len() + arguments.to_string().len();
            }
        }
    }
    chars
}

/// TS `estimateTokens`: ceil(chars / 4), including images at 4800 chars.
pub fn estimate_tokens(message: &ChatMessage) -> u64 {
    let chars = estimate_content_chars(&message.content);
    (chars as u64).div_ceil(4)
}

pub fn estimate_context_tokens(messages: &[ChatMessage]) -> u64 {
    messages.iter().map(estimate_tokens).sum()
}

/// TS `shouldCompact`: apply an optional absolute or percentage threshold while
/// retaining enough room for the recent-turn and reserved-token budgets.
pub fn should_compact(
    context_tokens: u64,
    context_window: u64,
    settings: &CompactionSettings,
) -> bool {
    if !settings.enabled {
        return false;
    }
    let capacity_threshold = context_window.saturating_sub(settings.reserve_tokens);
    let requested_threshold = match settings.threshold {
        Some(CompactionThreshold::Tokens(tokens)) => tokens,
        Some(CompactionThreshold::Percent(percent)) => {
            let scaled = (u128::from(context_window) * u128::from(percent)) / 100;
            scaled.min(u128::from(u64::MAX)) as u64
        }
        None => capacity_threshold,
    };
    let minimum_threshold = settings
        .keep_recent_tokens
        .saturating_add(settings.reserve_tokens);
    let effective_threshold = requested_threshold
        .max(minimum_threshold)
        .min(capacity_threshold);
    context_tokens > effective_threshold
}

fn is_cut_point_message(message: &ChatMessage) -> bool {
    !matches!(message.role.as_str(), "toolResult")
}

fn is_turn_start_message(message: &ChatMessage) -> bool {
    matches!(
        message.role.as_str(),
        "user" | "bashExecution" | "custom" | "branchSummary" | "compactionSummary"
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CutPointResult {
    pub first_kept_index: usize,
    pub turn_start_index: isize,
    pub is_split_turn: bool,
}

/// TS `findCutPoint` over chat messages (same walk-back + valid cut points).
pub fn find_cut_point(messages: &[ChatMessage], keep_recent_tokens: u64) -> CutPointResult {
    let cut_points: Vec<usize> = messages
        .iter()
        .enumerate()
        .filter(|(_, message)| is_cut_point_message(message))
        .map(|(index, _)| index)
        .collect();
    if cut_points.is_empty() {
        return CutPointResult {
            first_kept_index: 0,
            turn_start_index: -1,
            is_split_turn: false,
        };
    }

    let mut accumulated = 0_u64;
    let mut cut_index = cut_points[0];
    for (index, message) in messages.iter().enumerate().rev() {
        let tokens = estimate_tokens(message);
        if tokens == 0 {
            continue;
        }
        accumulated += tokens;
        if accumulated >= keep_recent_tokens {
            if let Some(&point) = cut_points.iter().find(|point| **point >= index) {
                cut_index = point;
            }
            break;
        }
    }

    let starts_turn = messages.get(cut_index).is_some_and(is_turn_start_message);
    let turn_start_index = if starts_turn {
        -1
    } else {
        messages[..cut_index]
            .iter()
            .enumerate()
            .rev()
            .find(|(_, message)| is_turn_start_message(message))
            .map(|(index, _)| index as isize)
            .unwrap_or(-1)
    };

    CutPointResult {
        first_kept_index: cut_index,
        turn_start_index,
        is_split_turn: !starts_turn && turn_start_index != -1,
    }
}

pub fn compact_messages(
    messages: &[ChatMessage],
    custom_instructions: Option<&str>,
) -> CompactionResult {
    compact_messages_with(messages, custom_instructions, DEFAULT_KEEP_RECENT_TOKENS)
}

pub fn compact_messages_with(
    messages: &[ChatMessage],
    custom_instructions: Option<&str>,
    keep_recent_tokens: u64,
) -> CompactionResult {
    compact_messages_with_options(
        messages,
        custom_instructions,
        keep_recent_tokens,
        DEFAULT_RESERVE_TOKENS,
        None,
        None,
    )
}

pub fn compact_messages_with_options(
    messages: &[ChatMessage],
    custom_instructions: Option<&str>,
    keep_recent_tokens: u64,
    reserve_tokens: u64,
    previous_summary: Option<&str>,
    summarizer: Option<&Summarizer>,
) -> CompactionResult {
    if messages.len() < 2 {
        return empty_result(messages);
    }
    let cut = find_cut_point(messages, keep_recent_tokens);
    let mut cut_index = cut.first_kept_index;
    if cut_index == 0 {
        // Manual compact still summarizes a prefix when everything fits in keepRecentTokens.
        cut_index = messages.len().saturating_sub(1);
    }
    if cut_index == 0 {
        return empty_result(messages);
    }

    let history_end = if cut.is_split_turn && cut.turn_start_index >= 0 {
        cut.turn_start_index as usize
    } else {
        cut_index
    };
    let history = &messages[..history_end.min(messages.len())];
    let turn_prefix = if cut.is_split_turn && cut.turn_start_index >= 0 {
        &messages[cut.turn_start_index as usize..cut_index]
    } else {
        &[]
    };
    let mut file_ops = extract_file_ops(history);
    merge_file_ops(&mut file_ops, &extract_file_ops(turn_prefix));
    let details = compute_file_lists(&file_ops);
    let tokens_before = estimate_context_tokens(messages);

    let env = env_summarizer();
    let (mut summary, usage) = if let Some(summarizer) = summarizer.or(env.as_ref()) {
        match generate_compaction_summary(
            history,
            turn_prefix,
            custom_instructions,
            previous_summary,
            reserve_tokens,
            summarizer,
        ) {
            Ok(result) => result,
            Err(error) => {
                return CompactionResult {
                    summary: error,
                    messages: messages.to_vec(),
                    compacted: false,
                    details,
                    first_kept_entry_id: String::new(),
                    tokens_before,
                    usage: None,
                };
            }
        }
    } else {
        (local_summary(history, custom_instructions), None)
    };
    summary.push_str(&format_file_operations(
        &details.read_files,
        &details.modified_files,
    ));

    let mut compacted = vec![compaction_context_message(&summary)];
    compacted.extend(messages[cut_index..].iter().cloned());
    CompactionResult {
        summary,
        messages: compacted,
        compacted: true,
        details,
        first_kept_entry_id: String::new(),
        tokens_before,
        usage,
    }
}

fn empty_result(messages: &[ChatMessage]) -> CompactionResult {
    CompactionResult {
        summary: String::new(),
        messages: messages.to_vec(),
        compacted: false,
        details: CompactionDetails::default(),
        first_kept_entry_id: String::new(),
        tokens_before: estimate_context_tokens(messages),
        usage: None,
    }
}

pub fn compaction_context_message(summary: &str) -> ChatMessage {
    ChatMessage::text(
        "user",
        format!("{COMPACTION_SUMMARY_PREFIX}{summary}{COMPACTION_SUMMARY_SUFFIX}"),
    )
}

pub fn branch_summary_context_message(summary: &str) -> ChatMessage {
    ChatMessage::text(
        "user",
        format!("{BRANCH_SUMMARY_PREFIX}{summary}{BRANCH_SUMMARY_SUFFIX}"),
    )
}

fn local_summary(messages: &[ChatMessage], custom_instructions: Option<&str>) -> String {
    let mut summary = serialize_conversation(messages);
    if let Some(instructions) = custom_instructions {
        summary.push_str("\nInstructions: ");
        summary.push_str(instructions);
    } else {
        summary.push('\n');
        summary.push_str(SUMMARIZATION_PROMPT);
    }
    summary
}

fn generate_compaction_summary(
    history: &[ChatMessage],
    turn_prefix: &[ChatMessage],
    custom_instructions: Option<&str>,
    previous_summary: Option<&str>,
    reserve_tokens: u64,
    summarizer: &Summarizer,
) -> Result<(String, Option<Usage>), String> {
    if !turn_prefix.is_empty() {
        let mut history_text = "No prior history.".to_string();
        let mut history_usage = None;
        if !history.is_empty() {
            let result = generate_summary_with_usage(
                history,
                reserve_tokens,
                custom_instructions,
                previous_summary,
                summarizer,
            )?;
            history_text = result.0;
            history_usage = Some(result.1);
        }
        let (prefix_text, prefix_usage) =
            generate_turn_prefix_summary(turn_prefix, reserve_tokens, summarizer)?;
        let summary =
            format!("{history_text}\n\n---\n\n**Turn Context (split turn):**\n\n{prefix_text}");
        let usage = Some(match history_usage {
            Some(first) => combine_usage(&first, &prefix_usage),
            None => prefix_usage,
        });
        return Ok((summary, usage));
    }
    let (text, usage) = generate_summary_with_usage(
        history,
        reserve_tokens,
        custom_instructions,
        previous_summary,
        summarizer,
    )?;
    Ok((text, Some(usage)))
}

pub fn generate_summary_with_usage(
    messages: &[ChatMessage],
    reserve_tokens: u64,
    custom_instructions: Option<&str>,
    previous_summary: Option<&str>,
    summarizer: &Summarizer,
) -> Result<(String, Usage), String> {
    let max_tokens = (reserve_tokens as f64 * 0.8).floor() as u64;
    let prompt = build_history_prompt(messages, previous_summary, custom_instructions);
    complete_summarization(&prompt, max_tokens.max(1), "Summarization", summarizer)
}

fn generate_turn_prefix_summary(
    messages: &[ChatMessage],
    reserve_tokens: u64,
    summarizer: &Summarizer,
) -> Result<(String, Usage), String> {
    let max_tokens = (reserve_tokens as f64 * 0.5).floor() as u64;
    let conversation = serialize_conversation(&convert_to_llm(messages));
    let prompt = format!(
        "<conversation>\n{conversation}\n</conversation>\n\n{TURN_PREFIX_SUMMARIZATION_PROMPT}"
    );
    complete_summarization(
        &prompt,
        max_tokens.max(1),
        "Turn prefix summarization",
        summarizer,
    )
}

pub fn build_history_prompt(
    messages: &[ChatMessage],
    previous_summary: Option<&str>,
    custom_instructions: Option<&str>,
) -> String {
    let mut base = if previous_summary.is_some() {
        UPDATE_SUMMARIZATION_PROMPT.to_string()
    } else {
        SUMMARIZATION_PROMPT.to_string()
    };
    if let Some(instructions) = custom_instructions {
        base.push_str("\n\nAdditional focus: ");
        base.push_str(instructions);
    }
    let conversation = serialize_conversation(&convert_to_llm(messages));
    let mut prompt = format!("<conversation>\n{conversation}\n</conversation>\n\n");
    if let Some(previous) = previous_summary {
        prompt.push_str("<previous-summary>\n");
        prompt.push_str(previous);
        prompt.push_str("\n</previous-summary>\n\n");
    }
    prompt.push_str(&base);
    prompt
}

fn complete_summarization(
    prompt: &str,
    max_tokens: u64,
    label: &str,
    summarizer: &Summarizer,
) -> Result<(String, Usage), String> {
    let response = summarizer.summarize(&SummarizeRequest {
        system: SUMMARIZATION_SYSTEM_PROMPT.to_string(),
        prompt: prompt.to_string(),
        max_tokens,
        label: label.to_string(),
        provider: String::new(),
        model_id: String::new(),
    })?;
    if let Some(failure) = get_summarization_failure(&response, label) {
        return Err(failure);
    }
    if response.has_tool_call {
        return Err(format!("{label} attempted to call a tool"));
    }
    Ok((response.text, response.usage))
}

pub fn get_summarization_failure(response: &SummarizeResponse, label: &str) -> Option<String> {
    match response.stop_reason {
        Some(StopReason::Error) => Some(format!(
            "{label} failed: {}",
            response.error_message.as_deref().unwrap_or("Unknown error")
        )),
        Some(StopReason::Length) => Some(format!(
            "{label} failed: generation hit the token cap and the summary is incomplete"
        )),
        _ => None,
    }
}

fn combine_usage(first: &Usage, second: &Usage) -> Usage {
    Usage {
        input: first.input + second.input,
        output: first.output + second.output,
        cache_read: first.cache_read + second.cache_read,
        cache_write: first.cache_write + second.cache_write,
        reasoning: match (first.reasoning, second.reasoning) {
            (None, None) => None,
            (a, b) => Some(a.unwrap_or(0) + b.unwrap_or(0)),
        },
        total_tokens: first.total_tokens + second.total_tokens,
        cost: davinci_protocol::UsageCost {
            input: first.cost.input + second.cost.input,
            output: first.cost.output + second.cost.output,
            cache_read: first.cost.cache_read + second.cost.cache_read,
            cache_write: first.cost.cache_write + second.cost.cache_write,
            total: first.cost.total + second.cost.total,
        },
    }
}

pub fn env_summarizer() -> Option<Summarizer> {
    if std::env::var("PI_COMPACTION_REPLY").is_err()
        && std::env::var("PI_COMPACTION_TURN_PREFIX_REPLY").is_err()
        && std::env::var("PI_BRANCH_SUMMARY_REPLY").is_err()
    {
        return None;
    }
    Some(Summarizer::new(summarize_from_env))
}

fn summarize_from_env(request: &SummarizeRequest) -> Result<SummarizeResponse, String> {
    let key = if request.label.contains("Turn prefix") {
        "PI_COMPACTION_TURN_PREFIX_REPLY"
    } else if request.label.contains("Branch") {
        "PI_BRANCH_SUMMARY_REPLY"
    } else {
        "PI_COMPACTION_REPLY"
    };
    let raw = std::env::var(key)
        .or_else(|_| std::env::var("PI_BRANCH_SUMMARY_REPLY"))
        .or_else(|_| std::env::var("PI_COMPACTION_REPLY"))
        .map_err(|_| format!("{} fixture missing", request.label))?;
    let text = if std::path::Path::new(&raw).is_file() {
        std::fs::read_to_string(&raw).map_err(|err| err.to_string())?
    } else {
        raw
    };
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) {
        let summary = value
            .get("text")
            .and_then(|v| v.as_str())
            .unwrap_or(&text)
            .to_string();
        let usage = value
            .get("usage")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        return Ok(SummarizeResponse {
            text: summary,
            usage,
            stop_reason: Some(StopReason::Stop),
            error_message: None,
            has_tool_call: false,
        });
    }
    Ok(SummarizeResponse {
        text,
        usage: Usage::default(),
        stop_reason: Some(StopReason::Stop),
        error_message: None,
        has_tool_call: false,
    })
}

pub fn extract_file_ops(messages: &[ChatMessage]) -> FileOperations {
    let mut ops = FileOperations::default();
    for message in messages {
        extract_file_ops_from_message(message, &mut ops);
    }
    ops.read.sort();
    ops.read.dedup();
    ops.written.sort();
    ops.written.dedup();
    ops.edited.sort();
    ops.edited.dedup();
    ops
}

fn extract_file_ops_from_message(message: &ChatMessage, ops: &mut FileOperations) {
    if message.role != "assistant" {
        return;
    }
    for block in &message.content {
        let MessageContent::ToolCall {
            name, arguments, ..
        } = block
        else {
            continue;
        };
        let Some(path) = arguments.get("path").and_then(|value| value.as_str()) else {
            continue;
        };
        match name.as_str() {
            "read" => ops.read.push(path.to_string()),
            "write" => ops.written.push(path.to_string()),
            "edit" => ops.edited.push(path.to_string()),
            _ => {}
        }
    }
}

fn merge_file_ops(into: &mut FileOperations, extra: &FileOperations) {
    into.read.extend(extra.read.iter().cloned());
    into.written.extend(extra.written.iter().cloned());
    into.edited.extend(extra.edited.iter().cloned());
    into.read.sort();
    into.read.dedup();
    into.written.sort();
    into.written.dedup();
    into.edited.sort();
    into.edited.dedup();
}

pub fn compute_file_lists(file_ops: &FileOperations) -> CompactionDetails {
    let mut modified: Vec<String> = file_ops
        .edited
        .iter()
        .chain(file_ops.written.iter())
        .cloned()
        .collect();
    modified.sort();
    modified.dedup();
    let read_files = file_ops
        .read
        .iter()
        .filter(|path| !modified.iter().any(|item| item == *path))
        .cloned()
        .collect();
    CompactionDetails {
        read_files,
        modified_files: modified,
    }
}

pub fn format_file_operations(read_files: &[String], modified_files: &[String]) -> String {
    let mut sections = Vec::new();
    if !read_files.is_empty() {
        sections.push(format!(
            "<read-files>\n{}\n</read-files>",
            read_files.join("\n")
        ));
    }
    if !modified_files.is_empty() {
        sections.push(format!(
            "<modified-files>\n{}\n</modified-files>",
            modified_files.join("\n")
        ));
    }
    if sections.is_empty() {
        String::new()
    } else {
        format!("\n\n{}", sections.join("\n\n"))
    }
}

/// TS `convertToLlm` for compaction serialization.
pub fn convert_to_llm(messages: &[ChatMessage]) -> Vec<ChatMessage> {
    messages
        .iter()
        .filter_map(|message| match message.role.as_str() {
            "bashExecution" => {
                if message.extra_bool("excludeFromContext") {
                    None
                } else {
                    Some(with_source_timestamp(
                        ChatMessage::text("user", bash_execution_to_text(message)),
                        message,
                    ))
                }
            }
            "custom" => Some(with_source_timestamp(
                ChatMessage {
                    role: "user".into(),
                    content: message.content.clone(),
                    ..ChatMessage::default()
                },
                message,
            )),
            "branchSummary" => {
                let summary = content_text(&message.content);
                Some(with_source_timestamp(
                    ChatMessage::text(
                        "user",
                        format!("{BRANCH_SUMMARY_PREFIX}{summary}{BRANCH_SUMMARY_SUFFIX}"),
                    ),
                    message,
                ))
            }
            "compactionSummary" => {
                let summary = content_text(&message.content);
                Some(with_source_timestamp(
                    ChatMessage::text(
                        "user",
                        format!("{COMPACTION_SUMMARY_PREFIX}{summary}{COMPACTION_SUMMARY_SUFFIX}"),
                    ),
                    message,
                ))
            }
            _ => Some(message.clone()),
        })
        .collect()
}

fn bash_execution_to_text(message: &ChatMessage) -> String {
    let command = message.extra_str("command").unwrap_or("");
    let output = message.extra_str("output").unwrap_or("");
    let mut text = format!("Ran `{command}`\n");
    if output.is_empty() {
        text.push_str("(no output)");
    } else {
        text.push_str("```\n");
        text.push_str(output);
        text.push_str("\n```");
    }
    if message.extra_bool("cancelled") {
        text.push_str("\n\n(command cancelled)");
    } else if let Some(code) = message
        .extra
        .get("exitCode")
        .and_then(serde_json::Value::as_i64)
    {
        if code != 0 {
            text.push_str(&format!("\n\nCommand exited with code {code}"));
        }
    }
    if message.extra_bool("truncated") {
        if let Some(path) = message.extra_str("fullOutputPath") {
            text.push_str(&format!("\n\n[Output truncated. Full output: {path}]"));
        }
    }
    text
}

fn with_source_timestamp(mut message: ChatMessage, source: &ChatMessage) -> ChatMessage {
    if let Some(timestamp) = source.extra.get("timestamp") {
        message.extra.insert("timestamp".into(), timestamp.clone());
    }
    message
}

/// TS `serializeConversation`.
pub fn serialize_conversation(messages: &[ChatMessage]) -> String {
    let mut parts = Vec::new();
    for message in messages {
        match message.role.as_str() {
            "user" => {
                let content = content_text(&message.content);
                if !content.is_empty() {
                    parts.push(format!("[User]: {content}"));
                }
            }
            "assistant" => {
                let mut thinking_parts = Vec::new();
                let mut tool_calls = Vec::new();
                for block in &message.content {
                    match block {
                        MessageContent::Thinking { thinking, .. } => {
                            thinking_parts.push(thinking.clone());
                        }
                        MessageContent::ToolCall {
                            name, arguments, ..
                        } => {
                            let args = match arguments {
                                serde_json::Value::Object(map) => map
                                    .iter()
                                    .map(|(key, value)| {
                                        format!(
                                            "{key}={}",
                                            serde_json::to_string(value)
                                                .unwrap_or_else(|_| value.to_string())
                                        )
                                    })
                                    .collect::<Vec<_>>()
                                    .join(", "),
                                other => other.to_string(),
                            };
                            tool_calls.push(format!("{name}({args})"));
                        }
                        _ => {}
                    }
                }
                if !thinking_parts.is_empty() {
                    parts.push(format!(
                        "[Assistant thinking]: {}",
                        thinking_parts.join("\n")
                    ));
                }
                if message
                    .content
                    .iter()
                    .any(|block| matches!(block, MessageContent::Text { .. }))
                {
                    parts.push(format!("[Assistant]: {}", content_text(&message.content)));
                }
                if !tool_calls.is_empty() {
                    parts.push(format!("[Assistant tool calls]: {}", tool_calls.join("; ")));
                }
            }
            "toolResult" => {
                let content = content_text(&message.content);
                if !content.is_empty() {
                    parts.push(format!(
                        "[Tool result]: {}",
                        truncate_for_summary(&content, TOOL_RESULT_MAX_CHARS)
                    ));
                }
            }
            _ => {}
        }
    }
    parts.join("\n\n")
}

fn truncate_for_summary(text: &str, max_chars: usize) -> String {
    if text.len() <= max_chars {
        text.to_string()
    } else {
        let truncated = text.len() - max_chars;
        format!(
            "{}\n\n[... {truncated} more characters truncated]",
            &text[..max_chars]
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_and_should_compact_match_ts() {
        let user = ChatMessage::text("user", "abcd");
        assert_eq!(estimate_tokens(&user), 1);
        let settings = CompactionSettings {
            enabled: true,
            reserve_tokens: 10,
            keep_recent_tokens: 1,
            threshold: None,
        };
        assert!(should_compact(11, 20, &settings));
        assert!(!should_compact(10, 20, &settings));
        assert!(!should_compact(
            100,
            20,
            &CompactionSettings {
                enabled: false,
                threshold: None,
                ..settings
            }
        ));
        assert!(should_compact(
            1,
            10,
            &CompactionSettings {
                enabled: true,
                reserve_tokens: 20,
                keep_recent_tokens: 1,
                threshold: None,
            }
        ));
        assert_eq!(calculate_context_tokens(12, 1, 2, 3, 4), 12);
        assert_eq!(calculate_context_tokens(0, 1, 2, 3, 4), 10);
    }

    #[test]
    fn explicit_compaction_threshold_supports_tokens_and_percent() {
        let token_threshold = CompactionSettings {
            enabled: true,
            reserve_tokens: 10,
            keep_recent_tokens: 5,
            threshold: Some(CompactionThreshold::Tokens(40)),
        };
        assert!(!should_compact(40, 100, &token_threshold));
        assert!(should_compact(41, 100, &token_threshold));

        let percent_threshold = CompactionSettings {
            enabled: true,
            reserve_tokens: 10,
            keep_recent_tokens: 5,
            threshold: Some(CompactionThreshold::Percent(25)),
        };
        assert!(!should_compact(25, 100, &percent_threshold));
        assert!(should_compact(26, 100, &percent_threshold));
    }

    #[test]
    fn find_cut_point_keeps_recent_tokens_and_skips_tool_results() {
        let messages = vec![
            ChatMessage::text("user", "one two three four five six seven eight"),
            ChatMessage::text("assistant", "reply one two three four"),
            ChatMessage::tool_result("c1", "read", "xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx", false),
            ChatMessage::text("user", "recent"),
        ];
        let cut = find_cut_point(&messages, 2);
        assert_eq!(cut.first_kept_index, 3);
        assert!(!cut.is_split_turn);
        let mid = find_cut_point(&messages, 8);
        assert_ne!(mid.first_kept_index, 2);
    }

    #[test]
    fn file_ops_and_summary_tags_match_ts() {
        let assistant = ChatMessage {
            role: "assistant".into(),
            content: vec![
                MessageContent::ToolCall {
                    id: "1".into(),
                    name: "read".into(),
                    arguments: serde_json::json!({"path": "a.rs"}),
                },
                MessageContent::ToolCall {
                    id: "2".into(),
                    name: "edit".into(),
                    arguments: serde_json::json!({"path": "a.rs"}),
                },
                MessageContent::ToolCall {
                    id: "3".into(),
                    name: "read".into(),
                    arguments: serde_json::json!({"path": "b.rs"}),
                },
            ],
            ..ChatMessage::default()
        };
        let details = compute_file_lists(&extract_file_ops(&[assistant]));
        assert_eq!(details.read_files, vec!["b.rs"]);
        assert_eq!(details.modified_files, vec!["a.rs"]);
        let xml = format_file_operations(&details.read_files, &details.modified_files);
        assert!(xml.contains("<read-files>\nb.rs\n</read-files>"));
        assert!(xml.contains("<modified-files>\na.rs\n</modified-files>"));
    }

    #[test]
    fn convert_to_llm_preserves_generated_metadata_and_truncation_details() {
        let mut bash = ChatMessage {
            role: "bashExecution".into(),
            ..ChatMessage::default()
        };
        bash.extra.insert("command".into(), serde_json::json!("ls"));
        bash.extra
            .insert("output".into(), serde_json::json!("listing"));
        bash.extra
            .insert("truncated".into(), serde_json::json!(true));
        bash.extra.insert(
            "fullOutputPath".into(),
            serde_json::json!("C:/tmp/full-output.txt"),
        );
        bash.extra.insert("timestamp".into(), serde_json::json!(42));

        let converted = convert_to_llm(&[bash]);
        assert_eq!(converted.len(), 1);
        assert_eq!(converted[0].role, "user");
        assert_eq!(
            converted[0].extra.get("timestamp"),
            Some(&serde_json::json!(42))
        );
        let text = content_text(&converted[0].content);
        assert!(text.contains("Ran `ls`"));
        assert!(text.contains("[Output truncated. Full output: C:/tmp/full-output.txt]"));
    }

    #[test]
    fn serialize_conversation_truncates_tool_results_like_ts() {
        let long = "x".repeat(5000);
        let messages = vec![ChatMessage::tool_result("tc1", "read", &long, false)];
        let result = serialize_conversation(&messages);
        assert!(result.contains("[Tool result]:"));
        assert!(result.contains("[... 3000 more characters truncated]"));
        assert!(result.contains(&"x".repeat(2000)));
        assert!(!result.contains(&"x".repeat(3000)));

        let short = "x".repeat(1500);
        assert_eq!(
            serialize_conversation(&[ChatMessage::tool_result("tc1", "read", &short, false)]),
            format!("[Tool result]: {short}")
        );

        let long_text = "y".repeat(5000);
        let both = serialize_conversation(&[
            ChatMessage::text("user", &long_text),
            ChatMessage::text("assistant", &long_text),
        ]);
        assert!(!both.contains("truncated"));
        assert!(both.contains(&long_text));
    }

    #[test]
    fn history_prompt_uses_ts_structured_checkpoint() {
        let prompt = build_history_prompt(
            &[ChatMessage::text("user", "ship rust pi")],
            None,
            Some("keep decisions"),
        );
        assert!(prompt.contains("<conversation>"));
        assert!(prompt.contains("[User]: ship rust pi"));
        assert!(prompt.contains("## Goal"));
        assert!(prompt.contains("Additional focus: keep decisions"));
        assert!(!prompt.contains(SUMMARIZATION_SYSTEM_PROMPT));
        assert!(prompt.contains(SUMMARIZATION_PROMPT));

        let update = build_history_prompt(
            &[ChatMessage::text("user", "next")],
            Some("old summary"),
            None,
        );
        assert!(update.contains("<previous-summary>\nold summary\n</previous-summary>"));
        assert!(update.contains(UPDATE_SUMMARIZATION_PROMPT));
    }

    #[test]
    fn llm_compaction_uses_complete_simple_fixture_and_split_turn() {
        let summarizer = Summarizer::new(|request| {
            let text = if request.prompt.contains("PREFIX of a turn") {
                "prefix-checkpoint".into()
            } else {
                assert_eq!(request.system, SUMMARIZATION_SYSTEM_PROMPT);
                assert!(request.prompt.contains("## Goal"));
                "## Goal\nship rust".into()
            };
            Ok(SummarizeResponse {
                text,
                usage: Usage {
                    input: 10,
                    output: 4,
                    total_tokens: 14,
                    ..Usage::default()
                },
                stop_reason: Some(StopReason::Stop),
                error_message: None,
                has_tool_call: false,
            })
        });
        let messages = vec![
            ChatMessage::text("user", "aaaaaaaaaa bbbbbbbbbb"),
            ChatMessage::text("assistant", "cccccccccc"),
            ChatMessage::text("user", "recent"),
        ];
        let result =
            compact_messages_with_options(&messages, None, 1, 100, None, Some(&summarizer));
        assert!(result.compacted);
        assert!(result.summary.starts_with("## Goal\nship rust"));
        assert_eq!(result.usage.as_ref().map(|u| u.total_tokens), Some(14));
        assert!(result.messages[0].content.iter().any(|block| matches!(
            block,
            MessageContent::Text { text } if text.contains(COMPACTION_SUMMARY_PREFIX)
        )));

        let split_messages = vec![
            ChatMessage::text("user", "original request ".repeat(20)),
            ChatMessage::text("assistant", "early work ".repeat(20)),
            ChatMessage::text("assistant", "recent suffix"),
        ];
        let split =
            compact_messages_with_options(&split_messages, None, 2, 100, None, Some(&summarizer));
        assert!(split.compacted);
        assert!(
            split.summary.contains("Turn Context (split turn)")
                || split.summary.contains("## Goal")
                || split.summary.contains("prefix-checkpoint")
        );
    }

    #[test]
    fn summarization_failure_matches_ts() {
        let error = SummarizeResponse {
            text: String::new(),
            usage: Usage::default(),
            stop_reason: Some(StopReason::Error),
            error_message: None,
            has_tool_call: false,
        };
        assert_eq!(
            get_summarization_failure(&error, "Summarization").as_deref(),
            Some("Summarization failed: Unknown error")
        );
        let length = SummarizeResponse {
            stop_reason: Some(StopReason::Length),
            ..error.clone()
        };
        assert_eq!(
            get_summarization_failure(&length, "Turn prefix summarization").as_deref(),
            Some("Turn prefix summarization failed: generation hit the token cap and the summary is incomplete")
        );
    }
}
