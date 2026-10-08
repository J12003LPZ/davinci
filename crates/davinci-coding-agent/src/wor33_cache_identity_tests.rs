//! WOR-33: Auto/Plan switches must not churn the live provider cache key.

use super::*;
use davinci_agent::PermissionMode;

fn codex_model() -> davinci_ai::Model {
    davinci_ai::load_builtin_models()
        .into_iter()
        .find(|model| model.provider == "openai-codex" && model.id == "gpt-5.6-luna")
        .expect("catalog has openai-codex/gpt-5.6-luna")
}

fn auth() -> ResolvedAuth {
    ResolvedAuth {
        api_key: Some("test".into()),
        headers: Default::default(),
        source: "oauth".into(),
    }
}

fn key_for(agent: &Agent) -> String {
    live_cache_key(
        &codex_model(),
        &auth(),
        &agent.provider_system_prompt(),
        agent,
        &provider_tools(agent),
    )
}

fn appended_agent() -> Agent {
    let mut agent = Agent::new_builtin(davinci_agent::prompt::PromptProfile::Stable);
    agent.provider = "openai-codex".into();
    agent.model_id = "gpt-5.6-luna".into();
    agent.turn_context_placement_override =
        Some(davinci_agent::turn_context::TurnContextPlacement::Appended);
    agent
}

#[test]
fn auto_plan_auto_keeps_the_live_cache_key() {
    let mut agent = appended_agent();
    agent.set_permission_mode(PermissionMode::Auto);
    let auto = key_for(&agent);

    agent.set_plan_mode(true);
    assert!(agent.is_plan_mode());
    let plan = key_for(&agent);

    agent.set_plan_mode(false);
    agent.set_permission_mode(PermissionMode::Auto);
    let back = key_for(&agent);

    assert_eq!(auto, plan, "entering Plan Mode changed the cache key");
    assert_eq!(auto, back, "leaving Plan Mode changed the cache key");
}

#[test]
fn every_permission_mode_projects_the_same_prefix_inputs() {
    let mut agent = appended_agent();
    agent.set_permission_mode(PermissionMode::Ask);
    let tools = serde_json::to_string(&provider_tools(&agent)).unwrap();
    let system = agent.provider_system_prompt();
    let visible = agent.visible_tool_names();
    let key = key_for(&agent);

    for mode in [
        PermissionMode::Edits,
        PermissionMode::Auto,
        PermissionMode::ReadOnly,
        PermissionMode::Ask,
    ] {
        agent.set_permission_mode(mode);
        assert_eq!(
            serde_json::to_string(&provider_tools(&agent)).unwrap(),
            tools,
            "tool schema differs in {mode:?}"
        );
        assert_eq!(
            agent.provider_system_prompt(),
            system,
            "system differs in {mode:?}"
        );
        assert_eq!(
            agent.visible_tool_names(),
            visible,
            "visible tools differ in {mode:?}"
        );
        assert_eq!(key_for(&agent), key, "cache key differs in {mode:?}");
    }
}
