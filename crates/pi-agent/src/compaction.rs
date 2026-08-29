use pi_ai::{ChatMessage, MessageContent};
use serde::{Deserialize, Serialize};

pub const DEFAULT_RESERVE_TOKENS: u64 = 16_384;
pub const DEFAULT_KEEP_RECENT_TOKENS: u64 = 20_000;
const ESTIMATED_IMAGE_CHARS: usize = 4800;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompactionSettings {
    pub enabled: bool,
    pub reserve_tokens: u64,
    pub keep_recent_tokens: u64,
}

impl Default for CompactionSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            reserve_tokens: DEFAULT_RESERVE_TOKENS,
            keep_recent_tokens: DEFAULT_KEEP_RECENT_TOKENS,
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
}

pub const SUMMARIZATION_PROMPT: &str = "The messages above are a conversation to summarize. Create a structured context checkpoint summary that another LLM will use to continue the work.";

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
    let chars = match message.role.as_str() {
        "user" | "assistant" | "custom" | "toolResult" | "bashExecution" | "branchSummary"
        | "compactionSummary" => estimate_content_chars(&message.content),
        _ => estimate_content_chars(&message.content),
    };
    (chars as u64).div_ceil(4)
}

pub fn estimate_context_tokens(messages: &[ChatMessage]) -> u64 {
    messages.iter().map(estimate_tokens).sum()
}

/// TS `shouldCompact`: enabled and contextTokens > contextWindow - reserveTokens.
pub fn should_compact(
    context_tokens: u64,
    context_window: u64,
    settings: &CompactionSettings,
) -> bool {
    if !settings.enabled {
        return false;
    }
    (context_tokens as i128) > (context_window as i128) - (settings.reserve_tokens as i128)
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
    if messages.len() < 2 {
        return CompactionResult {
            summary: String::new(),
            messages: messages.to_vec(),
            compacted: false,
            details: CompactionDetails::default(),
        };
    }
    let cut = find_cut_point(messages, keep_recent_tokens);
    let mut cut_index = cut.first_kept_index;
    if cut_index == 0 {
        // Manual compact still summarizes a prefix when everything fits in keepRecentTokens.
        cut_index = messages.len().saturating_sub(1);
    }
    if cut_index == 0 {
        return CompactionResult {
            summary: String::new(),
            messages: messages.to_vec(),
            compacted: false,
            details: CompactionDetails::default(),
        };
    }

    let prefix = &messages[..cut_index];
    let file_ops = extract_file_ops(prefix);
    let details = compute_file_lists(&file_ops);
    let mut summary = serialize_conversation(prefix);
    if let Some(instructions) = custom_instructions {
        summary.push_str("\nInstructions: ");
        summary.push_str(instructions);
    } else {
        summary.push('\n');
        summary.push_str(SUMMARIZATION_PROMPT);
    }
    summary.push_str(&format_file_operations(
        &details.read_files,
        &details.modified_files,
    ));

    let mut compacted = vec![ChatMessage::text("user", summary.clone())];
    compacted.extend(messages[cut_index..].iter().cloned());
    CompactionResult {
        summary,
        messages: compacted,
        compacted: true,
        details,
    }
}

pub fn extract_file_ops(messages: &[ChatMessage]) -> FileOperations {
    let mut ops = FileOperations::default();
    for message in messages {
        if message.role != "assistant" {
            continue;
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
    ops.read.sort();
    ops.read.dedup();
    ops.written.sort();
    ops.written.dedup();
    ops.edited.sort();
    ops.edited.dedup();
    ops
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

fn serialize_conversation(messages: &[ChatMessage]) -> String {
    let mut parts = Vec::new();
    for message in messages {
        let text = message_text(message);
        if text.is_empty() {
            continue;
        }
        let label = match message.role.as_str() {
            "user" => "User",
            "assistant" => "Assistant",
            "toolResult" => "Tool",
            other => other,
        };
        parts.push(format!("[{label}]: {}", truncate(&text, 2000)));
    }
    parts.join("\n")
}

fn message_text(message: &ChatMessage) -> String {
    let mut text = String::new();
    for block in &message.content {
        match block {
            MessageContent::Text { text: value } => text.push_str(value),
            MessageContent::Thinking { thinking, .. } => text.push_str(thinking),
            MessageContent::ToolCall {
                name, arguments, ..
            } => {
                text.push_str(name);
                text.push(' ');
                text.push_str(&arguments.to_string());
            }
            MessageContent::Image { .. } => text.push_str("[image]"),
        }
    }
    text
}

fn truncate(text: &str, max: usize) -> String {
    if text.len() <= max {
        text.to_string()
    } else {
        format!("{}…", &text[..max])
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
        };
        assert!(should_compact(11, 20, &settings));
        assert!(!should_compact(10, 20, &settings));
        assert!(!should_compact(
            100,
            20,
            &CompactionSettings {
                enabled: false,
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
            }
        ));
        assert_eq!(calculate_context_tokens(12, 1, 2, 3, 4), 12);
        assert_eq!(calculate_context_tokens(0, 1, 2, 3, 4), 10);
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
            tool_call_id: None,
            tool_name: None,
            is_error: None,
        };
        let details = compute_file_lists(&extract_file_ops(&[assistant]));
        assert_eq!(details.read_files, vec!["b.rs"]);
        assert_eq!(details.modified_files, vec!["a.rs"]);
        let xml = format_file_operations(&details.read_files, &details.modified_files);
        assert!(xml.contains("<read-files>\nb.rs\n</read-files>"));
        assert!(xml.contains("<modified-files>\na.rs\n</modified-files>"));
    }
}
