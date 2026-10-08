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

/// What one image can cost, whatever its size. Providers scale an image down
/// before counting it: OpenAI's tile and patch counts top out near 1.5k-2.5k
/// tokens, Anthropic's near 1.6k. Its base64 bytes are not tokens.
pub const IMAGE_TOKEN_CEILING: u64 = 3_000;

pub fn message_token_ceiling(message: &davinci_ai::ChatMessage) -> u64 {
    // Includes tool arguments/IDs and content framing, not just visible text.
    let images = message
        .content
        .iter()
        .filter(|content| matches!(content, davinci_ai::MessageContent::Image { .. }))
        .count() as u64;
    if images == 0 {
        return serde_json::to_vec(message).map_or(u64::MAX, |v| v.len() as u64 + 32);
    }
    let mut framing = message.clone();
    for content in &mut framing.content {
        if let davinci_ai::MessageContent::Image { data, .. } = content {
            data.clear();
        }
    }
    serde_json::to_vec(&framing).map_or(u64::MAX, |v| {
        (v.len() as u64 + 32).saturating_add(images.saturating_mul(IMAGE_TOKEN_CEILING))
    })
}

#[cfg(test)]
mod image_ceiling_tests {
    use super::*;

    #[test]
    fn an_image_costs_its_ceiling_not_its_base64_bytes() {
        let text = davinci_ai::ChatMessage::text("user", "look at this");
        let plain = message_token_ceiling(&text);
        let mut screenshot = text.clone();
        screenshot.content.push(davinci_ai::MessageContent::Image {
            data: "A".repeat(1_400_000),
            mime_type: "image/png".into(),
        });
        let with_image = message_token_ceiling(&screenshot);
        assert!(with_image > plain + IMAGE_TOKEN_CEILING - 1, "{with_image}");
        assert!(
            with_image < plain + IMAGE_TOKEN_CEILING + 200,
            "{with_image}"
        );
        // Two images, two ceilings; text still counts byte for byte.
        screenshot.content.push(davinci_ai::MessageContent::Image {
            data: "B".repeat(10),
            mime_type: "image/png".into(),
        });
        assert!(message_token_ceiling(&screenshot) >= plain + 2 * IMAGE_TOKEN_CEILING);
        let long = davinci_ai::ChatMessage::text("user", "x".repeat(10_000));
        assert!(message_token_ceiling(&long) >= 10_000);
    }
}
