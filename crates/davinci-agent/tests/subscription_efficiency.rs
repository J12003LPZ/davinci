use davinci_agent::prompt::{
    compose_profile_prompt, compose_with_mutations, PromptContext, PromptProfile,
};
use davinci_agent::{Agent, PermissionMode};

fn context(provider: &str) -> PromptContext<'_> {
    PromptContext {
        provider,
        model_id: "gpt-6-astra",
        permission_mode: PermissionMode::Auto,
        plan_active: false,
    }
}

#[test]
fn subscription_profiles_avoid_mandatory_meta_calls() {
    for profile in [PromptProfile::Stable, PromptProfile::Preview] {
        let prompt = compose_profile_prompt(profile, &context("openai-codex"));
        assert!(prompt.stable_text.contains("Subscription tool strategy"));
        assert!(prompt.stable_text.contains("Work solo on small tasks"));
        assert!(prompt
            .stable_text
            .contains("Preserve the chosen model and reasoning effort"));
        assert!(prompt.stable_text.contains("only selected skill bodies"));
        assert!(prompt.stable_text.contains("cannot approve permissions"));
        assert!(!prompt
            .stable_text
            .contains("Use subagents for bounded parallel research"));
    }
    let prompt = compose_with_mutations(&context("openai-codex"), &[]);
    assert!(prompt.stable_text.contains("Subscription tool strategy"));
}

#[test]
fn subscription_turns_keep_the_same_strategy_prefix() {
    let mut agent = Agent::new_builtin(PromptProfile::Stable);
    agent.provider = "openai-codex".into();
    agent.model_id = "gpt-6-astra".into();
    agent
        .prepare_builtin_prompt_for_user_turn("read src/lib.rs")
        .unwrap();
    let first = agent.system_prompt.clone();
    assert!(first.contains("Subscription tool strategy"));
    agent
        .prepare_builtin_prompt_for_user_turn("fix the frontend layout")
        .unwrap();
    assert_eq!(agent.system_prompt, first);
}

#[test]
fn api_and_other_routes_preserve_existing_strategy() {
    for provider in ["openai", "azure", "anthropic", "custom-openai-codex-proxy"] {
        let prompt = compose_profile_prompt(PromptProfile::Stable, &context(provider));
        let bundle = davinci_agent::prompt::bundle::stable_bundle();
        let policy =
            davinci_agent::prompt::model_policy::prompt_model_policy(provider, "gpt-6-astra");
        let modules = davinci_agent::prompt::model_policy::apply_model_policy(
            policy,
            PromptProfile::Stable,
            bundle.modules,
        );
        for module in modules {
            assert!(prompt.stable_text.contains(module.body.trim()));
        }
        assert!(!prompt.stable_text.contains("Subscription tool strategy"));
    }
    let legacy = compose_profile_prompt(PromptProfile::LegacyV1, &context("openai-codex"));
    assert_eq!(legacy.text, davinci_agent::default_system_prompt());
}
