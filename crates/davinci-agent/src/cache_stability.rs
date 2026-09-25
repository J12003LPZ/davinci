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
    fn a_tool_turn_with_a_verification_reminder_stays_append_only() {
        use davinci_ai::{AssistantMessage, ContentBlock, StopReason};
        use std::cell::RefCell;
        use std::rc::Rc;

        let dir = tempfile::tempdir().unwrap();
        let (mut agent, model) = appended_codex_agent();
        agent.cwd = dir.path().to_path_buf();
        agent.set_permission_mode(crate::PermissionMode::Ask);
        agent.approver = Some(crate::ToolApprover(std::sync::Arc::new(|_| {
            crate::ToolApprovalDecision::AllowOnce
        })));
        user_turn(
            &mut agent,
            "Create notes.txt containing hello, then read it back.",
            None,
        );

        let bodies: Rc<RefCell<Vec<Value>>> = Rc::default();
        let seen = Rc::clone(&bodies);
        let mut step = 0usize;
        agent
            .run_loop(move |current: &Agent| {
                seen.borrow_mut()
                    .push(wire_body_for_next_request(current, &model));
                step += 1;
                let call = |name: &str, arguments: Value| ContentBlock::ToolCall {
                    id: format!("call_{step}"),
                    name: name.into(),
                    arguments,
                };
                let (content, stop_reason) = match step {
                    1 => (
                        vec![call(
                            "write",
                            serde_json::json!({"path": "notes.txt", "content": "hello"}),
                        )],
                        StopReason::ToolUse,
                    ),
                    2 => (
                        vec![call("read", serde_json::json!({"path": "notes.txt"}))],
                        StopReason::ToolUse,
                    ),
                    _ => (
                        vec![ContentBlock::Text {
                            text: "done".into(),
                        }],
                        StopReason::Stop,
                    ),
                };
                Ok(AssistantMessage {
                    id: format!("a{step}"),
                    role: "assistant".into(),
                    content,
                    model: "fixture".into(),
                    usage: None,
                    stop_reason: Some(stop_reason),
                    error_message: None,
                })
            })
            .unwrap();

        let bodies = bodies.borrow();
        assert!(
            bodies.len() >= 4,
            "expected write, read, done, and a reminder follow-up; got {} requests",
            bodies.len()
        );
        for (index, pair) in bodies.windows(2).enumerate() {
            assert_eq!(
                first_prefix_break(&pair[0], &pair[1]),
                None,
                "request {} does not extend request {}:\nbefore={}\nafter={}",
                index + 2,
                index + 1,
                pair[0]["input"],
                pair[1]["input"]
            );
        }
    }
}
