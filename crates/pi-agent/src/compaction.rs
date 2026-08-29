use pi_ai::{estimate_context_tokens, Context, Message, UserMessage};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompactionSettings {
    pub max_context_tokens: usize,
    pub target_ratio: f64, // e.g., 0.5
}

impl Default for CompactionSettings {
    fn default() -> Self {
        Self {
            max_context_tokens: 100_000,
            target_ratio: 0.5,
        }
    }
}

pub fn should_compact(context: &Context, settings: &CompactionSettings) -> bool {
    let est = estimate_context_tokens(context);
    est.tokens > settings.max_context_tokens
}

pub fn compact_context(context: &Context, settings: &CompactionSettings) -> Context {
    if !should_compact(context, settings) || context.messages.len() <= 2 {
        return context.clone();
    }

    let cut_idx = context.messages.len() / 2;
    let retained_tail = &context.messages[cut_idx..];

    let summary = format!(
        "[Context summary: Earlier {} messages compacted to reduce token footprint]",
        cut_idx
    );

    let summary_msg = Message::User(UserMessage {
        role: "user".to_string(),
        content: pi_ai::UserContent::Text(summary),
        timestamp: pi_ai::now_ms(),
    });

    let mut new_messages = vec![summary_msg];
    new_messages.extend_from_slice(retained_tail);

    Context {
        system_prompt: context.system_prompt.clone(),
        messages: new_messages,
        tools: context.tools.clone(),
    }
}
