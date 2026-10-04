use davinci_agent::effort::EffortPolicy;
use davinci_agent::{Agent, PermissionMode};
use davinci_ai::{AssistantMessage, ChatMessage};
use davinci_protocol::ThinkingLevel;
use serde_json::json;

#[test]
fn fixed_effort_preserves_every_configured_level_after_tool_results() {
    use ThinkingLevel::*;
    for level in [Off, Minimal, Low, Medium, High, Xhigh, Max] {
        let mut agent = Agent::new("stable system");
        agent.thinking_level = level;
        agent.prompt("inspect the task");
        assert_eq!(agent.request_thinking_level(), level);
        for (id, tool, error) in [
            ("read", "read", false),
            ("write", "write", false),
            ("failure-1", "bash", true),
            ("failure-2", "bash", true),
        ] {
            agent
                .messages
                .push(ChatMessage::tool_result(id, tool, "fixture result", error));
            assert_eq!(agent.request_thinking_level(), level);
        }
        assert_eq!(agent.thinking_level, level);
        assert_eq!(agent.system_prompt, "stable system");
    }
}

#[test]
fn adaptive_effort_follows_current_turn_evidence_and_resets_on_new_task() {
    let mut agent = Agent::new("stable system");
    agent.thinking_level = ThinkingLevel::Medium;
    agent.effort_policy = EffortPolicy::Adaptive;
    agent.prompt("inspect the task");
    assert_eq!(agent.request_thinking_level(), ThinkingLevel::Low);

    for (id, tool, error, expected) in [
        ("read", "read", false, ThinkingLevel::Low),
        ("write", "write", false, ThinkingLevel::Medium),
        ("failure-1", "bash", true, ThinkingLevel::Medium),
        ("failure-2", "bash", true, ThinkingLevel::High),
        ("recovery", "read", false, ThinkingLevel::Medium),
    ] {
        agent
            .messages
            .push(ChatMessage::tool_result(id, tool, "fixture result", error));
        assert_eq!(agent.request_thinking_level(), expected);
        assert_eq!(agent.thinking_level, ThinkingLevel::Medium);
    }

    agent.prompt("a new task");
    assert_eq!(agent.request_thinking_level(), ThinkingLevel::Low);
    agent.thinking_level = ThinkingLevel::Off;
    for id in ["off-failure-1", "off-failure-2"] {
        agent.messages.push(ChatMessage::tool_result(
            id,
            "bash",
            "fixture failure",
            true,
        ));
    }
    assert_eq!(agent.request_thinking_level(), ThinkingLevel::Off);
    assert_eq!(agent.system_prompt, "stable system");
}

#[test]
fn provider_loop_keeps_deterministic_effort_across_request_boundaries() {
    for (policy, configured, expected) in [
        (
            EffortPolicy::Fixed,
            ThinkingLevel::Medium,
            ThinkingLevel::Medium,
        ),
        (
            EffortPolicy::Adaptive,
            ThinkingLevel::Medium,
            ThinkingLevel::Low,
        ),
        (
            EffortPolicy::Adaptive,
            ThinkingLevel::Off,
            ThinkingLevel::Off,
        ),
    ] {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(root.path().join("fixture.txt"), "public fixture").unwrap();
        let mut agent = Agent::new("stable system");
        agent.cwd = root.path().into();
        agent.auto_compaction = false;
        agent.auto_verify = false;
        agent.set_permission_mode(PermissionMode::AlwaysApprove);
        agent.thinking_level = configured;
        agent.effort_policy = policy;
        agent.prompt("Read fixture.txt");
        let mut requests = 0;

        agent
            .run_loop(|current| {
                requests += 1;
                assert_eq!(current.request_thinking_level(), expected);
                assert_eq!(current.thinking_level, configured);
                Ok(serde_json::from_value::<AssistantMessage>(json!({
                    "id": format!("response-{requests}"),
                    "role": "assistant",
                    "model": "fixture",
                    "content": if requests == 1 {
                        json!([{
                            "type": "toolCall",
                            "id": "read",
                            "name": "read",
                            "arguments": {"path": "fixture.txt"}
                        }])
                    } else {
                        json!([{"type": "text", "text": "done"}])
                    },
                    "stopReason": if requests == 1 { "toolUse" } else { "stop" }
                }))
                .unwrap())
            })
            .unwrap();

        assert_eq!(requests, 2);
        assert_eq!(agent.run_stats().model_turns, 2);
        assert!(agent.messages.iter().any(|message| {
            message.role == "toolResult"
                && message.tool_name.as_deref() == Some("read")
                && message.is_error == Some(false)
        }));
    }
}
