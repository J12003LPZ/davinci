use crate::types::{Context, Message, UserContent};
use std::sync::atomic::{AtomicUsize, Ordering};

const CHARS_PER_TOKEN: usize = 4;
const ESTIMATED_IMAGE_CHARS: usize = 4800;

static GLOBAL_ID_COUNTER: AtomicUsize = AtomicUsize::new(1);

pub fn uuidv7() -> String {
    let count = GLOBAL_ID_COUNTER.fetch_add(1, Ordering::SeqCst);
    let now = chrono::Utc::now().timestamp_millis();
    format!("{}-{:08x}", now, count)
}

pub fn estimate_text_tokens(text: &str) -> usize {
    text.len().div_ceil(CHARS_PER_TOKEN)
}

pub fn estimate_content_tokens(content: &[UserContent]) -> usize {
    let mut chars = 0;
    for block in content {
        match block {
            UserContent::Text(t) => chars += t.text.len(),
            UserContent::Image(_) => chars += ESTIMATED_IMAGE_CHARS,
        }
    }
    chars.div_ceil(CHARS_PER_TOKEN)
}

pub fn estimate_message_tokens(message: &Message) -> usize {
    match message {
        Message::User(u) => estimate_content_tokens(&u.content),
        Message::ToolResult(t) => estimate_content_tokens(&t.content),
        Message::Assistant(a) => {
            let mut chars = 0;
            for block in &a.content {
                match block {
                    crate::types::AssistantContent::Text(t) => chars += t.text.len(),
                    crate::types::AssistantContent::Thinking(t) => chars += t.thinking.len(),
                    crate::types::AssistantContent::ToolCall(tc) => {
                        chars += tc.name.len() + tc.arguments.to_string().len()
                    }
                }
            }
            chars.div_ceil(CHARS_PER_TOKEN)
        }
    }
}

pub fn estimate_context_tokens(context: &Context) -> usize {
    let mut total = 0;
    if let Some(sys) = &context.system_prompt {
        total += estimate_text_tokens(sys);
    }
    for msg in &context.messages {
        total += estimate_message_tokens(msg);
    }
    total
}

pub fn content_text(content: &[crate::types::AssistantContent]) -> String {
    content
        .iter()
        .filter_map(|c| match c {
            crate::types::AssistantContent::Text(t) => Some(t.text.as_str()),
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("")
}
