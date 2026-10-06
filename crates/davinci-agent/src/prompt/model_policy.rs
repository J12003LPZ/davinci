use serde::{Deserialize, Serialize};

/// 2: adds the shared GPT-6 tool, delegation and style modules.
pub const GPT6_ASTRA_POLICY_VERSION: u32 = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PromptModelPolicy {
    Default,
    Gpt6Astra,
    /// GPT-6 Sol and its point releases (`gpt-6.1-sol`).
    Gpt6Sol,
    Gpt6Luna,
}

impl PromptModelPolicy {
    pub fn id(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Gpt6Astra => "gpt6-astra",
            Self::Gpt6Sol => "gpt6-sol",
            Self::Gpt6Luna => "gpt6-luna",
        }
    }

    pub fn version(self) -> u32 {
        match self {
            Self::Default => 0,
            Self::Gpt6Astra => GPT6_ASTRA_POLICY_VERSION,
            Self::Gpt6Sol => crate::prompt::gpt6::GPT6_SOL_POLICY_VERSION,
            Self::Gpt6Luna => crate::prompt::gpt6::GPT6_LUNA_POLICY_VERSION,
        }
    }

    /// Whether this policy replaces the generic OpenAI provider adapter. The
    /// GPT-6 modules restate its parallel-call and reporting rules in the
    /// wording each variant needs, so keeping both would say it twice.
    /// Legacy prompts are left untransformed, so they keep the adapter.
    pub fn replaces_provider_adapter(self, profile: crate::prompt::PromptProfile) -> bool {
        !matches!(self, Self::Default) && profile != crate::prompt::PromptProfile::LegacyV1
    }

    /// The policy an eval names on the command line or in
    /// `DAVINCI_EVAL_PROMPT_MODEL_POLICY`.
    pub fn parse(id: &str) -> Option<Self> {
        match id.trim() {
            "default" => Some(Self::Default),
            "gpt6-astra" => Some(Self::Gpt6Astra),
            "gpt6-sol" => Some(Self::Gpt6Sol),
            "gpt6-luna" => Some(Self::Gpt6Luna),
            _ => None,
        }
    }
}

pub fn inferred_prompt_model_policy(provider: &str, model_id: &str) -> PromptModelPolicy {
    let provider = provider.to_ascii_lowercase();
    let model = model_id.to_ascii_lowercase();

    if !matches!(provider.as_str(), "openai" | "openai-codex") {
        return PromptModelPolicy::Default;
    }
    match gpt6_variant(&model) {
        Some("astra") => PromptModelPolicy::Gpt6Astra,
        Some("sol") => PromptModelPolicy::Gpt6Sol,
        Some("luna") => PromptModelPolicy::Gpt6Luna,
        _ => PromptModelPolicy::Default,
    }
}

/// The variant of a GPT-6 model id: `gpt-6-luna`, `gpt-6.1-sol` and dated
/// snapshots such as `gpt-6-astra-2026-09-10` name `luna`, `sol`, `astra`.
/// GPT-5.x models with the same variant names are not GPT-6.
fn gpt6_variant(model: &str) -> Option<&str> {
    let rest = model.strip_prefix("gpt-6")?;
    let rest = match rest.strip_prefix('.') {
        Some(point) => point.trim_start_matches(|ch: char| ch.is_ascii_digit()),
        None => rest,
    };
    let variant = rest.strip_prefix('-')?;
    let name = variant.split('-').next()?;
    let suffix = &variant[name.len()..];
    // Only a dated snapshot may follow the variant name.
    let dated = suffix.is_empty()
        || suffix
            .strip_prefix('-')
            .is_some_and(|date| date.chars().all(|ch| ch.is_ascii_digit() || ch == '-'));
    (dated && matches!(name, "astra" | "sol" | "luna")).then_some(name)
}

pub(crate) fn prompt_model_policy_with_eval_override(
    provider: &str,
    model_id: &str,
    eval_mode: bool,
    override_id: Option<&str>,
) -> PromptModelPolicy {
    if eval_mode {
        if let Some(policy) = override_id.and_then(PromptModelPolicy::parse) {
            return policy;
        }
    }
    inferred_prompt_model_policy(provider, model_id)
}

pub fn prompt_model_policy(provider: &str, model_id: &str) -> PromptModelPolicy {
    let eval_mode = std::env::var("DAVINCI_BEHAVIOR_EVAL").as_deref() == Ok("1");
    let override_id = std::env::var("DAVINCI_EVAL_PROMPT_MODEL_POLICY").ok();
    prompt_model_policy_with_eval_override(provider, model_id, eval_mode, override_id.as_deref())
}

pub fn apply_model_policy(
    policy: PromptModelPolicy,
    profile: crate::prompt::PromptProfile,
    modules: Vec<crate::prompt::PromptModule>,
) -> Vec<crate::prompt::PromptModule> {
    match policy {
        PromptModelPolicy::Default => modules,
        PromptModelPolicy::Gpt6Astra => crate::prompt::astra::apply_astra_policy(profile, modules),
        PromptModelPolicy::Gpt6Sol => crate::prompt::gpt6::apply_sol_policy(profile, modules),
        PromptModelPolicy::Gpt6Luna => crate::prompt::gpt6::apply_luna_policy(profile, modules),
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
    fn eval_override_is_guarded_and_can_force_default_for_astra() {
        assert_eq!(
            prompt_model_policy_with_eval_override(
                "openai-codex",
                "gpt-6-astra",
                true,
                Some("default"),
            ),
            PromptModelPolicy::Default
        );
        assert_eq!(
            prompt_model_policy_with_eval_override(
                "openai-codex",
                "gpt-6-astra",
                false,
                Some("default"),
            ),
            PromptModelPolicy::Gpt6Astra
        );
    }

    #[test]
    fn every_gpt6_variant_selects_its_own_policy() {
        for (model, policy) in [
            ("gpt-6-astra", PromptModelPolicy::Gpt6Astra),
            ("gpt-6-sol", PromptModelPolicy::Gpt6Sol),
            ("gpt-6.1-sol", PromptModelPolicy::Gpt6Sol),
            ("gpt-6-sol-2026-10-01", PromptModelPolicy::Gpt6Sol),
            ("GPT-6-Luna", PromptModelPolicy::Gpt6Luna),
            ("gpt-6.2-luna", PromptModelPolicy::Gpt6Luna),
        ] {
            assert_eq!(
                prompt_model_policy("openai-codex", model),
                policy,
                "{model}"
            );
            assert_eq!(prompt_model_policy("openai", model), policy, "{model}");
        }
        // GPT-5.x shares the variant names but not the policy, and look-alike
        // ids are not variants.
        for model in [
            "gpt-5.6-sol",
            "gpt-5.6-luna",
            "gpt-6",
            "gpt-6-solar",
            "gpt-6-sol-preview",
            "gpt-60-sol",
            "gpt-6-terra",
        ] {
            assert_eq!(
                prompt_model_policy("openai-codex", model),
                PromptModelPolicy::Default,
                "{model}"
            );
        }
        // Another provider serving the same name is not the OpenAI model.
        assert_eq!(
            prompt_model_policy("openrouter", "gpt-6-luna"),
            PromptModelPolicy::Default
        );
    }

    #[test]
    fn eval_override_names_every_policy() {
        for id in ["default", "gpt6-astra", "gpt6-sol", "gpt6-luna"] {
            let policy = PromptModelPolicy::parse(id).unwrap();
            assert_eq!(policy.id(), id);
            assert_eq!(
                prompt_model_policy_with_eval_override(
                    "openai-codex",
                    "gpt-6-luna",
                    true,
                    Some(id)
                ),
                policy
            );
        }
        assert_eq!(PromptModelPolicy::parse("gpt6-terra"), None);
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
