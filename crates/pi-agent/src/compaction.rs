use pi_ai::{content_text, ChatMessage};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionResult {
    pub summary: String,
    pub messages: Vec<ChatMessage>,
    pub compacted: bool,
}

pub fn compact_messages(
    messages: &[ChatMessage],
    custom_instructions: Option<&str>,
) -> CompactionResult {
    if messages.len() < 2 {
        return CompactionResult {
            summary: String::new(),
            messages: messages.to_vec(),
            compacted: false,
        };
    }
    let mut summary = String::from("Compacted conversation:\n");
    for message in messages {
        let text = content_text(&message.content);
        if !text.is_empty() {
            summary.push_str(&format!("- {}: {}\n", message.role, truncate(&text, 240)));
        }
    }
    if let Some(instructions) = custom_instructions {
        summary.push_str("\nInstructions: ");
        summary.push_str(instructions);
    }
    let last = messages.last().cloned();
    let mut compacted = vec![ChatMessage::text("user", summary.clone())];
    if let Some(last) = last {
        if last.role == "assistant" {
            compacted.push(last);
        }
    }
    CompactionResult {
        summary,
        messages: compacted,
        compacted: true,
    }
}

fn truncate(text: &str, max: usize) -> String {
    if text.len() <= max {
        text.to_string()
    } else {
        format!("{}…", &text[..max])
    }
}
