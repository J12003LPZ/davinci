//! Offline completion fixtures exercise the loop shared by every host mode.
use davinci_agent::{Agent, AgentEvent, PermissionMode, PromptProfile};
use davinci_ai::{AssistantMessage, ChatMessage, ContentBlock, StopReason};

const REMINDER: &str = "List each explicit requirement";

fn reply(_: &Agent) -> Result<AssistantMessage, String> {
    serde_json::from_value(serde_json::json!({
        "id": "answer", "role": "assistant", "model": "fixture",
        "content": [{"type":"text", "text":"Finished the requested change."}],
        "stopReason":"stop"
    }))
    .map_err(|error| error.to_string())
}

fn write_reply(agent: &Agent, writes: &[(&str, &str)]) -> Result<AssistantMessage, String> {
    let mut message = reply(agent)?;
    message.content = writes
        .iter()
        .enumerate()
        .map(|(index, (path, content))| ContentBlock::ToolCall {
            id: format!("write-{index}-{content}"),
            name: "write".into(),
            arguments: serde_json::json!({"path":path, "content":content}),
        })
        .collect();
    message.stop_reason = Some(StopReason::ToolUse);
    Ok(message)
}

fn run_with_writes(agent: &mut Agent, writes: &[(&str, &str)]) -> Vec<AgentEvent> {
    let mut wrote = false;
    agent
        .run_loop(|agent| {
            if !wrote {
                wrote = true;
                write_reply(agent, writes)
            } else {
                reply(agent)
            }
        })
        .unwrap()
}

fn reminders(events: &[AgentEvent]) -> usize {
    events
        .iter()
        .filter(|event| {
            let value = serde_json::to_value(event).unwrap();
            value["type"] == "completion_reminder"
                && value["reason_code"] == "completion.requirements"
        })
        .count()
}

fn agent(root: &std::path::Path, request: &str) -> Agent {
    let mut agent = Agent::new("fixture");
    agent.cwd = root.into();
    agent.auto_compaction = false;
    agent.auto_verify = false;
    agent.prompt(request);
    agent
}

#[test]
fn multi_file_requirement_review_runs_once_and_is_ephemeral() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.py"), "before").unwrap();
    let sessions = tempfile::tempdir().unwrap();
    let mut agent = Agent::new("fixture");
    agent.cwd = dir.path().into();
    agent.auto_compaction = false;
    agent.auto_verify = false;
    agent.session =
        Some(davinci_session::JsonlSession::create(sessions.path(), "completion", None).unwrap());
    agent.prompt("Update the implementation");
    let mut seen = 0;
    let mut wrote = false;
    let events = agent
        .run_loop(|agent| {
            if !wrote {
                wrote = true;
                return write_reply(agent, &[("a.py", "after"), ("test_a.py", "new")]);
            }
            let messages = agent.messages_for_provider();
            let text = serde_json::to_string(&messages).unwrap();
            if text.contains(REMINDER) {
                assert_eq!(text.matches(REMINDER).count(), 1);
                // Later verification turns retain the same reminder in their
                // history; only its introduction is the newest instruction.
                seen += usize::from(messages.last().is_some_and(|message| {
                    davinci_ai::content_text(&message.content).contains(REMINDER)
                }));
                assert!(text.contains("test_a.py"));
                assert!(text.contains("new"));
                assert!(text.contains("obsolete"));
                assert!(text.contains("existing test files"));
            }
            reply(agent)
        })
        .unwrap();
    assert_eq!(reminders(&events), 1);
    assert_eq!(seen, 1);
    assert_eq!(
        agent.last_assistant_text().as_deref(),
        Some("Finished the requested change.")
    );
    assert!(
        !std::fs::read_to_string(&agent.session.as_ref().unwrap().path)
            .unwrap()
            .contains(REMINDER)
    );
    assert!(!serde_json::to_string(&agent.messages_for_provider())
        .unwrap()
        .contains(REMINDER));
    let AgentEvent::AgentEnd { messages, .. } = events.last().unwrap() else {
        panic!("missing end")
    };
    assert!(!serde_json::to_string(messages).unwrap().contains(REMINDER));
}

#[test]
fn read_only_plan_and_single_trivial_edits_do_not_review_requirements() {
    for (mutate, mode, request) in [
        (
            false,
            PermissionMode::Auto,
            "Change parsing, add coverage; rerun the checks.",
        ),
        (
            true,
            PermissionMode::ReadOnly,
            "Change parsing, add coverage; rerun the checks.",
        ),
        (true, PermissionMode::Auto, "Fix the typo"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let mut agent = agent(dir.path(), request);
        agent.set_permission_mode(mode);
        let events = if mutate {
            run_with_writes(&mut agent, &[("app.py", "after")])
        } else {
            agent.run_loop(reply).unwrap()
        };
        assert_eq!(reminders(&events), 0, "{request}, {mode:?}");
        assert_eq!(
            dir.path().join("app.py").exists(),
            mutate && mode == PermissionMode::Auto
        );
    }
}

#[test]
fn separate_requirements_trigger_for_one_file_and_new_prompt_resets() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = agent(
        dir.path(),
        "Handle normal input, reject invalid input; preserve the old format.",
    );
    assert_eq!(
        reminders(&run_with_writes(&mut agent, &[("app.py", "first")])),
        1
    );
    agent.prompt_with(
        "Harness context with several clauses, commas; and sentences.",
        &[],
    );
    assert_eq!(
        reminders(&run_with_writes(&mut agent, &[("app.py", "second")])),
        0
    );
    agent.prompt("Handle normal input. Reject invalid input. Preserve the old format.");
    assert_eq!(
        reminders(&run_with_writes(&mut agent, &[("app.py", "third")])),
        1
    );
}

#[test]
fn completion_keeps_stable_prompt_and_frozen_tool_schema() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = Agent::new_builtin(PromptProfile::Stable);
    agent.cwd = dir.path().into();
    agent.auto_compaction = false;
    agent.prompt("Update the implementation");
    let hash = agent
        .prompt_manifest
        .as_ref()
        .unwrap()
        .stable_sha256
        .clone();
    let schema = agent.provider_tool_schema_identity();
    let system = agent.system_prompt.clone();
    let events = run_with_writes(&mut agent, &[("app.py", "after"), ("test_app.py", "after")]);
    assert_eq!(reminders(&events), 1);
    assert_eq!(agent.prompt_manifest.as_ref().unwrap().stable_sha256, hash);
    assert_eq!(agent.provider_tool_schema_identity(), schema);
    assert_eq!(agent.system_prompt, system);
    assert!(agent
        .messages
        .iter()
        .all(
            |message: &ChatMessage| !davinci_ai::content_text(&message.content).contains(REMINDER)
        ));
}

#[test]
fn disabled_requirement_review_is_a_baseline_arm_without_prompt_changes() {
    let dir = tempfile::tempdir().unwrap();
    let mut agent = agent(
        dir.path(),
        "Handle normal input, reject invalid input; preserve the format.",
    );
    let system = agent.system_prompt.clone();
    let schema = agent.provider_tool_schema_identity();
    agent.requirement_review_enabled = false;
    assert_eq!(
        reminders(&run_with_writes(
            &mut agent,
            &[("app.py", "after"), ("test_app.py", "after")]
        )),
        0
    );
    assert_eq!(agent.provider_tool_schema_identity(), schema);
    assert_eq!(agent.system_prompt, system);
}

#[cfg(unix)]
#[test]
fn a_symlink_alias_cannot_bypass_a_denied_completion_read() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("private.py"), "before").unwrap();
    std::os::unix::fs::symlink("private.py", dir.path().join("alias.py")).unwrap();
    let mut agent = Agent::new("fixture");
    agent.cwd = dir.path().into();
    agent.auto_compaction = false;
    agent
        .permissions
        .lock()
        .unwrap()
        .deny
        .push(davinci_agent::PermissionRule::parse("read(private.py)").unwrap());
    agent.prompt("Handle normal input, reject invalid input; preserve the format.");
    assert_eq!(
        reminders(&run_with_writes(&mut agent, &[("private.py", "after")])),
        0
    );
}
