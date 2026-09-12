use serde::{Deserialize, Serialize};

pub const GPT6_ASTRA_POLICY_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PromptModelPolicy {
    Default,
    Gpt6Astra,
}

impl PromptModelPolicy {
    pub fn id(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Gpt6Astra => "gpt6-astra",
        }
    }

    pub fn version(self) -> u32 {
        match self {
            Self::Default => 0,
            Self::Gpt6Astra => GPT6_ASTRA_POLICY_VERSION,
        }
    }
}

pub fn prompt_model_policy(provider: &str, model_id: &str) -> PromptModelPolicy {
    let provider = provider.to_ascii_lowercase();
    let model = model_id.to_ascii_lowercase();

    if matches!(provider.as_str(), "openai" | "openai-codex")
        && (model == "gpt-6-astra" || model.starts_with("gpt-6-astra-"))
    {
        PromptModelPolicy::Gpt6Astra
    } else {
        PromptModelPolicy::Default
    }
}

pub fn apply_model_policy(
    policy: PromptModelPolicy,
    profile: crate::prompt::PromptProfile,
    modules: Vec<crate::prompt::PromptModule>,
) -> Vec<crate::prompt::PromptModule> {
    match policy {
        PromptModelPolicy::Default => modules,
        PromptModelPolicy::Gpt6Astra => crate::prompt::astra::apply_astra_policy(profile, modules),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn selects_astra_for_codex_alias_and_snapshots() {
        assert_eq!(
            prompt_model_policy("openai-codex", "gpt-6-astra"),
            PromptModelPolicy::Gpt6Astra
        );
        assert_eq!(
            prompt_model_policy("openai-codex", "gpt-6-astra-2026-09-10"),
            PromptModelPolicy::Gpt6Astra
        );
    }

    #[test]
    fn does_not_apply_astra_policy_to_other_openai_models() {
        assert_eq!(
            prompt_model_policy("openai-codex", "gpt-5.6-sol"),
            PromptModelPolicy::Default
        );
        assert_eq!(
            prompt_model_policy("anthropic", "claude-opus-4-8"),
            PromptModelPolicy::Default
        );
    }
}
