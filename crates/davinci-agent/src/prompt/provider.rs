//! Minimal provider and model-family prompt adapters.

use crate::prompt::composer::{PromptCacheClass, PromptModule};
use serde::{Deserialize, Serialize};

pub const PROVIDER_ADAPTER_MAX_TOKENS: usize = 250;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PromptModelFamily {
    OpenAiReasoning,
    Anthropic,
    Gemini,
    Mistral,
    Generic,
}

impl PromptModelFamily {
    pub fn name(self) -> &'static str {
        match self {
            Self::OpenAiReasoning => "openai-reasoning",
            Self::Anthropic => "anthropic",
            Self::Gemini => "gemini",
            Self::Mistral => "mistral",
            Self::Generic => "generic",
        }
    }
}

pub fn prompt_model_family(provider: &str, model_id: &str) -> PromptModelFamily {
    let p = provider.to_lowercase();
    let m = model_id.to_lowercase();

    if p.contains("anthropic") || m.contains("claude") {
        PromptModelFamily::Anthropic
    } else if p.contains("google") || m.contains("gemini") {
        PromptModelFamily::Gemini
    } else if p.contains("mistral") || m.contains("codestral") {
        PromptModelFamily::Mistral
    } else if p.contains("openai")
        || m.starts_with("o1")
        || m.starts_with("o3")
        || m.contains("gpt-5")
        || m.contains("codex")
    {
        PromptModelFamily::OpenAiReasoning
    } else {
        PromptModelFamily::Generic
    }
}

pub fn provider_adapter(family: PromptModelFamily) -> Option<PromptModule> {
    match family {
        PromptModelFamily::OpenAiReasoning => None,
        PromptModelFamily::Anthropic => None,
        PromptModelFamily::Gemini => None,
        PromptModelFamily::Mistral => None,
        PromptModelFamily::Generic => None,
    }
}

pub fn create_family_adapter(family: PromptModelFamily, body: &str) -> PromptModule {
    PromptModule {
        id: format!("provider.{}", family.name()),
        version: 1,
        cache_class: PromptCacheClass::Stable,
        body: body.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::prompt::manifest::estimate_tokens_from_str;

    #[test]
    fn known_provider_models_map_to_stable_families() {
        assert_eq!(
            prompt_model_family("openai-codex", "gpt-5.6-luna"),
            PromptModelFamily::OpenAiReasoning
        );
        assert_eq!(
            prompt_model_family("anthropic", "claude-opus-4-5"),
            PromptModelFamily::Anthropic
        );
        assert_eq!(
            prompt_model_family("google", "gemini-2.0-flash"),
            PromptModelFamily::Gemini
        );
        assert_eq!(
            prompt_model_family("mistral", "codestral-2501"),
            PromptModelFamily::Mistral
        );
        assert_eq!(
            prompt_model_family("unknown", "custom-model"),
            PromptModelFamily::Generic
        );
    }

    #[test]
    fn adapter_budget_is_enforced() {
        let families = [
            PromptModelFamily::OpenAiReasoning,
            PromptModelFamily::Anthropic,
            PromptModelFamily::Gemini,
            PromptModelFamily::Mistral,
            PromptModelFamily::Generic,
        ];

        for fam in families {
            if let Some(module) = provider_adapter(fam) {
                assert!(
                    estimate_tokens_from_str(&module.body) <= PROVIDER_ADAPTER_MAX_TOKENS,
                    "Adapter for {:?} exceeds token budget: {}",
                    fam,
                    estimate_tokens_from_str(&module.body)
                );
            }
        }
    }
}
