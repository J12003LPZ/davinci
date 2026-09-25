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

const OPENAI_REASONING_ADAPTER: &str = "\
OpenAI model guidance:
- Before each tool call, work out what else you already know you will need. Send every independent read, search and listing in the same response as parallel calls, or put them in one batch call. Chain related shell commands into one command.
- For a small, clear task (one or two files), read the relevant code, make the change, run the narrowest test, and finish. Do not create a task list for it.
- Use apply_patch for multi-hunk or multi-file edits and keep patches minimal with enough context to match once.
- After a change, report what changed and how it was verified in a few sentences, without repeating whole files or the plan.
- Stop when the requested task is complete and verified; do not start unrelated work.";

pub fn provider_adapter(family: PromptModelFamily) -> Option<PromptModule> {
    match family {
        PromptModelFamily::OpenAiReasoning => {
            Some(create_family_adapter(family, OPENAI_REASONING_ADAPTER))
        }
        PromptModelFamily::Anthropic
        | PromptModelFamily::Gemini
        | PromptModelFamily::Mistral
        | PromptModelFamily::Generic => None,
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
    #[test]
    fn openai_adapter_asks_for_fewer_round_trips() {
        let body = provider_adapter(PromptModelFamily::OpenAiReasoning)
            .expect("OpenAI adapter")
            .body;
        assert!(body.contains("one batch call"), "{body}");
        assert!(body.contains("Chain related shell commands"), "{body}");
        assert!(body.contains("Do not create a task list"), "{body}");
        assert!(estimate_tokens_from_str(&body) <= PROVIDER_ADAPTER_MAX_TOKENS);
    }

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
    fn openai_reasoning_models_get_a_stable_adapter() {
        let module = provider_adapter(PromptModelFamily::OpenAiReasoning).expect("OpenAI adapter");
        assert_eq!(module.id, "provider.openai-reasoning");
        assert_eq!(module.cache_class, PromptCacheClass::Stable);
        assert!(module.body.contains("parallel"));
        assert!(estimate_tokens_from_str(&module.body) <= PROVIDER_ADAPTER_MAX_TOKENS);
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
