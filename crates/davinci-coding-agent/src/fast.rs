//! The status label for the Codex service tier saved in settings
//! (`serviceTier`). There is no command to change it; see the README.
use davinci_ai::{CodexServiceTier, FastCapability};

pub fn speed_label(agent: &davinci_agent::Agent, capability: FastCapability) -> String {
    if agent.provider != "openai-codex"
        || crate::current_runtime_model(agent)
            .is_some_and(|model| model.api != "openai-codex-responses")
    {
        return String::new();
    }
    let label = agent.service_tier.label();
    match (agent.service_tier, capability) {
        (CodexServiceTier::Fast, FastCapability::Unknown) => {
            format!("{label} · capability unverified")
        }
        (CodexServiceTier::Fast, FastCapability::Unsupported) => {
            format!("{label} requested · blocked: not advertised")
        }
        _ => label.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn codex_agent(tier: CodexServiceTier) -> davinci_agent::Agent {
        let mut agent = davinci_agent::Agent::new("system");
        agent.provider = "openai-codex".into();
        agent.model_id = "fixture-model".into();
        agent.service_tier = tier;
        agent
    }

    #[test]
    fn a_saved_fast_tier_is_labeled_with_its_capability() {
        let fast = codex_agent(CodexServiceTier::Fast);
        assert!(speed_label(&fast, FastCapability::Unsupported).contains("blocked"));
        assert!(speed_label(&fast, FastCapability::Unknown).contains("unverified"));
        assert_eq!(
            speed_label(&fast, FastCapability::Supported),
            CodexServiceTier::Fast.label()
        );
        assert_eq!(
            speed_label(
                &codex_agent(CodexServiceTier::Standard),
                FastCapability::Supported
            ),
            CodexServiceTier::Standard.label()
        );
    }

    #[test]
    fn other_providers_show_no_speed() {
        for provider in ["anthropic", "google", "azure-openai", "openai", "ollama"] {
            let mut agent = codex_agent(CodexServiceTier::Fast);
            agent.provider = provider.into();
            assert!(speed_label(&agent, FastCapability::Supported).is_empty());
        }
    }
}
