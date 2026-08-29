use crate::types::{ContentBlock, Context, Message, StopReason, Usage};

pub const CHARS_PER_TOKEN: usize = 4;
pub const ESTIMATED_IMAGE_CHARS: usize = 4800;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ContextUsageEstimate {
    pub tokens: usize,
    pub usage_tokens: usize,
    pub trailing_tokens: usize,
    pub last_usage_index: Option<usize>,
}

pub fn calculate_context_tokens(usage: &Usage) -> u64 {
    if usage.total_tokens > 0 {
        usage.total_tokens
    } else {
        usage.input + usage.output + usage.cache_read + usage.cache_write
    }
}

pub fn estimate_text_tokens(text: &str) -> usize {
    text.len().div_ceil(CHARS_PER_TOKEN)
}

pub fn estimate_content_block_tokens(block: &ContentBlock) -> usize {
    match block {
        ContentBlock::Text(t) => estimate_text_tokens(&t.text),
        ContentBlock::Thinking(th) => estimate_text_tokens(&th.thinking),
        ContentBlock::Image(_) => ESTIMATED_IMAGE_CHARS.div_ceil(CHARS_PER_TOKEN),
        ContentBlock::ToolCall(tc) => {
            let args_str = tc.arguments.to_string();
            (tc.name.len() + args_str.len()).div_ceil(CHARS_PER_TOKEN)
        }
    }
}

pub fn estimate_message_tokens(message: &Message) -> usize {
    match message {
        Message::User(u) => match &u.content {
            crate::types::UserContent::Text(text) => estimate_text_tokens(text),
            crate::types::UserContent::Blocks(blocks) => {
                blocks.iter().map(estimate_content_block_tokens).sum()
            }
        },
        Message::Assistant(a) => a.content.iter().map(estimate_content_block_tokens).sum(),
        Message::ToolResult(tr) => tr.content.iter().map(estimate_content_block_tokens).sum(),
    }
}

fn get_last_assistant_usage_info(messages: &[Message]) -> Option<(Usage, usize)> {
    let mut latest_prefix_timestamp = i64::MIN;
    let mut usage_info = None;

    for (i, msg) in messages.iter().enumerate() {
        if let Message::Assistant(assistant) = msg {
            let usage_applies = assistant.timestamp >= latest_prefix_timestamp;
            if usage_applies
                && assistant.stop_reason != StopReason::Aborted
                && assistant.stop_reason != StopReason::Error
                && calculate_context_tokens(&assistant.usage) > 0
            {
                usage_info = Some((assistant.usage.clone(), i));
            }
        }
        latest_prefix_timestamp = latest_prefix_timestamp.max(msg.timestamp());
    }

    usage_info
}

pub fn estimate_context_tokens(context: &Context) -> ContextUsageEstimate {
    let messages = &context.messages;
    if let Some((usage, idx)) = get_last_assistant_usage_info(messages) {
        let usage_tokens = calculate_context_tokens(&usage) as usize;
        let mut trailing_tokens = 0;
        for msg in &messages[idx + 1..] {
            trailing_tokens += estimate_message_tokens(msg);
        }
        ContextUsageEstimate {
            tokens: usage_tokens + trailing_tokens,
            usage_tokens,
            trailing_tokens,
            last_usage_index: Some(idx),
        }
    } else {
        let mut tokens = 0;
        for msg in messages {
            tokens += estimate_message_tokens(msg);
        }
        ContextUsageEstimate {
            tokens,
            usage_tokens: 0,
            trailing_tokens: tokens,
            last_usage_index: None,
        }
    }
}
