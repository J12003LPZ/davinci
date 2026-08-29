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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionResult {
    pub summary: String,
    pub messages: Vec<ChatMessage>,
    pub compacted: bool,
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
        };
    }

    let mut summary = String::from("Compacted conversation:\n");
    for message in &messages[..cut_index] {
        let text = message_text(message);
        if !text.is_empty() {
            summary.push_str(&format!("- {}: {}\n", message.role, truncate(&text, 240)));
        }
    }
    if let Some(instructions) = custom_instructions {
        summary.push_str("\nInstructions: ");
        summary.push_str(instructions);
    }

    let mut compacted = vec![ChatMessage::text("user", summary.clone())];
    compacted.extend(messages[cut_index..].iter().cloned());
    CompactionResult {
        summary,
        messages: compacted,
        compacted: true,
    }
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
}
