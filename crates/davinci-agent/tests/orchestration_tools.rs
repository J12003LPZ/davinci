//! The lead only sees the agent-team and workflow tools when those features
//! are on, and then sees them without a `tool_search` round trip.
//!
//! Its own test binary: it flips the process-wide feature switches.

use davinci_agent::{Agent, AgentId, PromptProfile, RunId, RuntimeBus, RuntimeHandle};

fn provider_names(agent: &Agent) -> Vec<String> {
    agent
        .provider_tool_specs()
        .into_iter()
        .map(|tool| tool.name)
        .collect()
}

fn lead() -> Agent {
    let mut agent = Agent::new_builtin(PromptProfile::Stable);
    agent.set_runtime(RuntimeHandle::new(
        RunId::new(),
        AgentId::new(),
        RuntimeBus::new(),
    ));
    agent
}

#[test]
fn orchestration_tools_follow_the_feature_settings() {
    std::env::remove_var("DAVINCI_EXPERIMENTAL_WORKFLOWS");
    std::env::remove_var("DAVINCI_RUNTIME_WORKFLOWS");
    std::env::remove_var("DAVINCI_EXPERIMENTAL_AGENT_TEAMS");

    // Off (the default): nothing extra is offered.
    davinci_agent::tools::set_orchestration_settings(None, None);
    let mut agent = lead();
    agent.sync_orchestration_tools();
    let names = provider_names(&agent);
    for hidden in ["workflow_run", "agent_message", "task_create"] {
        assert!(!names.iter().any(|name| name == hidden), "{hidden} leaked");
    }

    // Settings on: the lead is offered both families at once.
    davinci_agent::tools::set_orchestration_settings(Some(true), Some(true));
    agent.sync_orchestration_tools();
    let names = provider_names(&agent);
    for tool in davinci_agent::TEAM_TOOLS
        .iter()
        .chain(davinci_agent::WORKFLOW_TOOLS)
    {
        assert!(
            names.iter().any(|name| name == tool),
            "{tool} missing: {names:?}"
        );
    }

    // Turning one family off again removes only that family.
    davinci_agent::tools::set_orchestration_settings(Some(true), Some(false));
    agent.sync_orchestration_tools();
    let names = provider_names(&agent);
    assert!(!names.iter().any(|name| name == "workflow_run"));
    assert!(names.iter().any(|name| name == "agent_message"));

    // An explicit environment value wins over the setting.
    std::env::set_var("DAVINCI_EXPERIMENTAL_AGENT_TEAMS", "0");
    assert!(!davinci_agent::tools::team_tools_enabled());
    std::env::remove_var("DAVINCI_EXPERIMENTAL_AGENT_TEAMS");
    assert!(davinci_agent::tools::team_tools_enabled());

    // A worker (no `agent` tool) keeps exactly the tools it was given.
    let mut worker = lead();
    worker.tools = vec!["read".into(), "grep".into()];
    worker.sync_orchestration_tools();
    assert_eq!(worker.tools, vec!["read".to_string(), "grep".to_string()]);

    davinci_agent::tools::set_orchestration_settings(None, None);
}
