//! A conservative input ceiling and the matching provider output limit.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderContextBudget {
    pub window: u64,
    pub system: u64,
    pub tools: u64,
    pub reasoning_reserve: u64,
    pub output_reserve: u64,
    pub safety_margin: u64,
}

impl ProviderContextBudget {
    pub fn working_set_budget(self) -> u64 {
        self.window.saturating_sub(self.reserved())
    }

    pub fn output_limit(self) -> u64 {
        self.reasoning_reserve.saturating_add(self.output_reserve)
    }

    pub fn reserved(self) -> u64 {
        self.system
            .saturating_add(self.tools)
            .saturating_add(self.output_limit())
            .saturating_add(self.safety_margin)
    }
}

/// UTF-8 byte ceiling instead of bytes/4: code, non-ASCII and arbitrary tool
/// output must not silently consume the reserved output window. Framing is
/// charged separately for every message. Provider tokenizers may use less.
pub fn text_token_ceiling(text: &str) -> u64 {
    (text.len() as u64).saturating_add(32)
}

pub fn message_token_ceiling(message: &davinci_ai::ChatMessage) -> u64 {
    // Includes tool arguments/IDs and content framing, not just visible text.
    serde_json::to_vec(message).map_or(u64::MAX, |v| v.len() as u64 + 32)
}
