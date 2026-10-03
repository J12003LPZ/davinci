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
    /// Unknown metadata is not a denial; explicit lack of support is.
    pub fn validate_capability(
        self,
        capability: crate::FastCapability,
        model_id: &str,
    ) -> Result<(), String> {
        if self == Self::Fast && capability == crate::FastCapability::Unsupported {
            return Err(format!(
                "Fast is not advertised for {model_id}. Select Standard with /fast or change the saved serviceTier setting; no request was sent."
            ));
        }
        Ok(())
    }

    /// Read only cached capability metadata. Never discover models, refresh
    /// credentials, substitute a model, or silently downgrade a request here.
    pub fn validate_for_model(self, model: &crate::Model) -> Result<(), String> {
        if self != Self::Fast || model.api != "openai-codex-responses" {
            return Ok(());
        }
        let auth_path = crate::default_auth_path();
        let capability = auth_path
            .parent()
            .map(|directory| crate::fast_capability_for_model(directory, &model.id))
            .unwrap_or(crate::FastCapability::Unknown);
        self.validate_capability(capability, &model.id)
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
    fn unsupported_fast_is_rejected_but_unknown_is_not_invented_support() {
        use crate::FastCapability::{Supported, Unknown, Unsupported};
        for capability in [Supported, Unknown] {
            assert!(CodexServiceTier::Fast
                .validate_capability(capability, "fixture")
                .is_ok());
        }
        let error = CodexServiceTier::Fast
            .validate_capability(Unsupported, "fixture")
            .unwrap_err();
        assert!(error.contains("no request was sent"));
        assert!(CodexServiceTier::Standard
            .validate_capability(Unsupported, "fixture")
            .is_ok());
        assert!(CodexServiceTier::Flex
            .validate_capability(Unsupported, "fixture")
            .is_ok());
    }

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
