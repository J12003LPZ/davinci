//! User-owned Codex routing policy, independent of model and reasoning effort.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CodexServiceTier {
    #[default]
    #[serde(alias = "default")]
    Standard,
    #[serde(alias = "priority")]
    Fast,
    Flex,
}

impl CodexServiceTier {
    /// Unknown capability remains an explicit user choice; only a known denial
    /// blocks Fast. Never silently change the selected model, effort, or tier.
    pub fn validate_capability(
        self,
        model: &str,
        capability: crate::codex_models::FastCapability,
    ) -> Result<(), String> {
        if self == Self::Fast && capability == crate::codex_models::FastCapability::Unsupported {
            return Err(format!("Fast is not advertised for {model}"));
        }
        Ok(())
    }

    pub fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "standard" | "default" => Some(Self::Standard),
            "fast" | "priority" => Some(Self::Fast),
            "flex" => Some(Self::Flex),
            _ => None,
        }
    }

    pub const fn request_value(self) -> Option<&'static str> {
        match self {
            Self::Standard => None,
            Self::Fast => Some("priority"),
            Self::Flex => Some("flex"),
        }
    }

    pub const fn settings_value(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Fast => "fast",
            Self::Flex => "flex",
        }
    }

    pub const fn label(self) -> &'static str {
        match self {
            Self::Standard => "Standard",
            Self::Fast => "Fast",
            Self::Flex => "Flex",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalization_and_wire_values() {
        for (value, tier, wire) in [
            ("fast", CodexServiceTier::Fast, Some("priority")),
            (" PRIORITY ", CodexServiceTier::Fast, Some("priority")),
            ("standard", CodexServiceTier::Standard, None),
            ("default", CodexServiceTier::Standard, None),
            ("Flex", CodexServiceTier::Flex, Some("flex")),
        ] {
            assert_eq!(CodexServiceTier::parse(value), Some(tier));
            assert_eq!(tier.request_value(), wire);
        }
        assert_eq!(CodexServiceTier::parse("turbo"), None);
        assert_eq!(CodexServiceTier::Fast.settings_value(), "fast");
        assert_eq!(CodexServiceTier::default(), CodexServiceTier::Standard);
    }

    #[test]
    fn model_capability_distinguishes_missing_from_explicit_catalog() {
        use crate::codex_models::{parse_codex_models, FastCapability};
        let models = parse_codex_models(&serde_json::json!({"models": [
            {"slug":"fast", "service_tiers":[{"id":"priority", "name":"Fast"}]},
            {"slug":"standard", "service_tiers":[{"id":"default", "name":"Standard"}]},
            {"slug":"unknown"},
            {"slug":"empty", "service_tiers":[]}
        ]}));
        assert_eq!(models[0].fast_capability(), FastCapability::Supported);
        assert_eq!(models[1].fast_capability(), FastCapability::Unsupported);
        assert_eq!(models[2].fast_capability(), FastCapability::Unknown);
        assert_eq!(models[3].fast_capability(), FastCapability::Unsupported);
        let cached = serde_json::to_value(&models).unwrap();
        assert_eq!(parse_codex_models(&cached), models);
    }
}

#[cfg(test)]
mod review_tests {
    use super::*;
    use crate::codex_models::FastCapability;
    #[test]
    fn review_fast_capability_policy_is_consistent_and_never_downgrades() {
        assert!(CodexServiceTier::Fast
            .validate_capability("fixture", FastCapability::Unsupported)
            .is_err());
        for tier in [
            CodexServiceTier::Standard,
            CodexServiceTier::Fast,
            CodexServiceTier::Flex,
        ] {
            for capability in [
                FastCapability::Unknown,
                FastCapability::Supported,
                FastCapability::Unsupported,
            ] {
                if tier != CodexServiceTier::Fast || capability != FastCapability::Unsupported {
                    assert!(tier.validate_capability("fixture", capability).is_ok());
                }
            }
        }
    }
}
