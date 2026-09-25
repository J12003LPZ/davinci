//! Append-only request invariants for prompt-cache routes.

use serde_json::Value;

use crate::Agent;

/// Where a later provider request stops extending an earlier one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PrefixBreak {
    Instructions,
    Tools,
    Input { index: usize },
    Shorter,
}

/// The first place `next` does not extend `previous`, or `None` when every
/// cache-relevant part of `previous` is an exact prefix of `next`.
pub fn first_prefix_break(previous: &Value, next: &Value) -> Option<PrefixBreak> {
    if previous.get("instructions") != next.get("instructions") {
        return Some(PrefixBreak::Instructions);
    }
    if previous.get("tools") != next.get("tools") {
        return Some(PrefixBreak::Tools);
    }
    let empty = Vec::new();
    let before = previous
        .get("input")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    let after = next
        .get("input")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    if after.len() < before.len() {
        return Some(PrefixBreak::Shorter);
    }
    before
        .iter()
        .zip(after)
        .position(|(left, right)| left != right)
        .map(|index| PrefixBreak::Input { index })
}

/// The exact body the agent would send next, built offline.
pub fn wire_body_for_next_request(agent: &Agent, model: &davinci_ai::Model) -> Value {
    let system = agent.provider_system_prompt();
    let tools: Vec<davinci_ai::ToolSpec> = agent
        .provider_tool_specs()
        .into_iter()
        .map(|tool| davinci_ai::ToolSpec {
            name: tool.name,
            description: tool.description,
            parameters: tool.parameters,
            constrained_sampling: None,
        })
        .collect();
    davinci_ai::request_body_with(
        model,
        &agent.messages_for_provider(),
        Some(&system),
        &tools,
        &davinci_ai::StreamOptions {
            thinking_level: Some(agent.thinking_level),
            ..Default::default()
        },
    )
}

#[cfg(test)]
pub(crate) fn codex_test_model() -> davinci_ai::Model {
    davinci_ai::load_builtin_models()
        .into_iter()
        .find(|model| model.provider == "openai-codex" && model.id == "gpt-5.6-luna")
        .expect("catalog has openai-codex/gpt-5.6-luna")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn appended_item_is_not_a_break() {
        let first = json!({"instructions": "a", "tools": [], "input": [{"n": 1}]});
        let second = json!({"instructions": "a", "tools": [], "input": [{"n": 1}, {"n": 2}]});
        assert_eq!(first_prefix_break(&first, &second), None);
    }

    #[test]
    fn each_kind_of_break_is_named() {
        let base = json!({"instructions": "a", "tools": [1], "input": [{"n": 1}]});
        assert_eq!(
            first_prefix_break(
                &base,
                &json!({"instructions": "b", "tools": [1], "input": [{"n": 1}]})
            ),
            Some(PrefixBreak::Instructions)
        );
        assert_eq!(
            first_prefix_break(
                &base,
                &json!({"instructions": "a", "tools": [2], "input": [{"n": 1}]})
            ),
            Some(PrefixBreak::Tools)
        );
        assert_eq!(
            first_prefix_break(
                &base,
                &json!({"instructions": "a", "tools": [1], "input": [{"n": 9}]})
            ),
            Some(PrefixBreak::Input { index: 0 })
        );
        assert_eq!(
            first_prefix_break(
                &base,
                &json!({"instructions": "a", "tools": [1], "input": []})
            ),
            Some(PrefixBreak::Shorter)
        );
    }

    fn appended_codex_agent() -> (Agent, davinci_ai::Model) {
        let mut agent = Agent::new_builtin(crate::prompt::PromptProfile::Stable);
        agent.provider = "openai-codex".into();
        agent.model_id = "gpt-5.6-luna".into();
        agent.turn_context_placement_override =
            Some(crate::turn_context::TurnContextPlacement::Appended);
        (agent, codex_test_model())
    }

    fn user_turn(agent: &mut Agent, text: &str, memory: Option<&str>) {
        agent.prompt(text);
        agent.commit_turn_context(memory.map(str::to_string));
    }

    #[test]
    fn new_capability_no_longer_rewrites_instructions() {
        let (mut agent, model) = appended_codex_agent();
        user_turn(&mut agent, "Add a verbose flag to the parser", None);
        let first = wire_body_for_next_request(&agent, &model);
        agent
            .messages
            .push(davinci_ai::ChatMessage::text("assistant", "Done."));
        user_turn(&mut agent, "Diagnose the root cause of this failure.", None);
        let second = wire_body_for_next_request(&agent, &model);
        assert_eq!(first_prefix_break(&first, &second), None);
        assert!(second.to_string().contains("<turn_context>"));
    }

    #[test]
    fn plan_mode_round_trip_keeps_prefix() {
        let (mut agent, model) = appended_codex_agent();
        user_turn(&mut agent, "Look at the parser", None);
        let first = wire_body_for_next_request(&agent, &model);
        agent
            .messages
            .push(davinci_ai::ChatMessage::text("assistant", "Read it."));
        agent.set_plan_mode(true);
        user_turn(&mut agent, "Plan the change", None);
        let second = wire_body_for_next_request(&agent, &model);
        assert_eq!(first_prefix_break(&first, &second), None);
        agent
            .messages
            .push(davinci_ai::ChatMessage::text("assistant", "Plan ready."));
        agent.set_plan_mode(false);
        user_turn(&mut agent, "Go ahead", None);
        let third = wire_body_for_next_request(&agent, &model);
        assert_eq!(first_prefix_break(&second, &third), None);
    }

    #[test]
    fn injected_memory_stays_where_it_was_injected() {
        let (mut agent, model) = appended_codex_agent();
        user_turn(
            &mut agent,
            "Fix the parser",
            Some("user prefers small diffs"),
        );
        let first = wire_body_for_next_request(&agent, &model);
        agent
            .messages
            .push(davinci_ai::ChatMessage::text("assistant", "Fixed."));
        user_turn(
            &mut agent,
            "Now the lexer",
            Some("lexer lives in src/lex.rs"),
        );
        let second = wire_body_for_next_request(&agent, &model);
        assert_eq!(first_prefix_break(&first, &second), None);
    }

    #[test]
    fn other_families_keep_per_turn_state_in_the_system_prompt() {
        let mut agent = Agent::new_builtin(crate::prompt::PromptProfile::Stable);
        agent.provider = "anthropic".into();
        agent.model_id = "claude-opus-4-5".into();

        agent.prompt("Diagnose the root cause of this failure.");

        assert_eq!(
            agent.turn_context_placement(),
            crate::turn_context::TurnContextPlacement::SystemPrompt
        );
        assert!(agent.system_prompt.contains("Permission mode:"));
    }

    #[test]
    fn tool_search_does_not_change_the_tool_list_on_cached_routes() {
        let (mut agent, model) = appended_codex_agent();
        agent.set_runtime(crate::RuntimeHandle::new(
            crate::RunId::new(),
            crate::AgentId::new(),
            crate::RuntimeBus::new(),
        ));
        agent.freeze_tools_for_cache();

        user_turn(&mut agent, "Search the web for the changelog", None);
        let first = wire_body_for_next_request(&agent, &model);
        assert!(
            first["tools"].to_string().contains("web_search"),
            "the frozen schema set must include authorized deferred tools"
        );

        crate::execute_tool_with(
            std::path::Path::new("."),
            "tool_search",
            &serde_json::json!({"query": "web_search"}),
            &agent.tool_context,
        )
        .unwrap();
        agent
            .messages
            .push(davinci_ai::ChatMessage::text("assistant", "Found it."));

        let second = wire_body_for_next_request(&agent, &model);
        assert_eq!(first_prefix_break(&first, &second), None);
    }

}
