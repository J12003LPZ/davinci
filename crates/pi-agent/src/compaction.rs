use crate::types::AgentMessage;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompactionSettings {
    pub max_context_tokens: usize,
    pub target_tokens: usize,
    pub min_cut_tokens: usize,
}

pub const DEFAULT_COMPACTION_SETTINGS: CompactionSettings = CompactionSettings {
    max_context_tokens: 100_000,
    target_tokens: 40_000,
    min_cut_tokens: 10_000,
};

pub fn should_compact(total_tokens: usize, settings: &CompactionSettings) -> bool {
    total_tokens > settings.max_context_tokens
}

pub fn summarize_conversation(messages: &[AgentMessage]) -> String {
    format!(
        "Summary of previous {} conversation messages.",
        messages.len()
    )
}
