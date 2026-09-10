//! Version definitions for prompt profiles and modules.

use serde::{Deserialize, Serialize};

pub const LEGACY_PROMPT_VERSION: u32 = 1;
pub const STABLE_PROMPT_VERSION: u32 = 2;
pub const PREVIEW_PROMPT_VERSION: u32 = 3;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PromptProfile {
    #[default]
    Stable,
    Preview,
    LegacyV1,
}

impl PromptProfile {
    pub const ALL: [PromptProfile; 3] = [
        PromptProfile::Stable,
        PromptProfile::Preview,
        PromptProfile::LegacyV1,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Preview => "preview",
            Self::LegacyV1 => "legacy-v1",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        let normalized = s.trim().to_lowercase();
        match normalized.as_str() {
            "stable" => Some(Self::Stable),
            "preview" => Some(Self::Preview),
            "legacy-v1" | "legacy" | "v1" => Some(Self::LegacyV1),
            _ => None,
        }
    }

    pub fn version(self) -> u32 {
        match self {
            Self::LegacyV1 => LEGACY_PROMPT_VERSION,
            Self::Stable => STABLE_PROMPT_VERSION,
            Self::Preview => PREVIEW_PROMPT_VERSION,
        }
    }
}

impl std::str::FromStr for PromptProfile {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s).ok_or_else(|| {
            format!("Invalid prompt profile '{s}'. Valid options: stable, preview, legacy-v1")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_and_id_profiles() {
        assert_eq!(PromptProfile::parse("stable"), Some(PromptProfile::Stable));
        assert_eq!(PromptProfile::parse("preview"), Some(PromptProfile::Preview));
        assert_eq!(PromptProfile::parse("legacy-v1"), Some(PromptProfile::LegacyV1));
        assert_eq!(PromptProfile::parse("legacy"), Some(PromptProfile::LegacyV1));
        assert_eq!(PromptProfile::parse("v1"), Some(PromptProfile::LegacyV1));
        assert_eq!(PromptProfile::parse("unknown"), None);

        assert_eq!(PromptProfile::Stable.id(), "stable");
        assert_eq!(PromptProfile::Preview.id(), "preview");
        assert_eq!(PromptProfile::LegacyV1.id(), "legacy-v1");
    }
}
