//! Exercises the CLI's real executor attachment with deterministic provider replies.
use super::{attach_shared_tool_executor, attach_tool_executor, Agent, ExtensionHost};
use davinci_agent::{
    AgentEvent, PermissionMode, PermissionPolicy, PermissionRule, PermissionState,
};
use davinci_ai::{AssistantMessage, ContentBlock, StopReason};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};

#[path = "../tests/support/test_impact_fixture.rs"]
#[allow(dead_code)]
mod fixture;

fn call(agent: &mut Agent, name: &str, args: Value) -> (Value, Value, bool) {
    agent.prompt("Execute the fixture's next authorized step.");
    let call_id = format!("fixture-{}-{name}", agent.messages.len());
    let mut sent = false;
    let events = agent
        .run_loop(|_| {
            let content = if sent {
                vec![ContentBlock::Text {
                    text: "Fixture step complete.".into(),
                }]
            } else {
                vec![ContentBlock::ToolCall {
                    id: call_id.clone(),
                    name: name.into(),
                    arguments: args.clone(),
                }]
            };
            let stop_reason = Some(if sent {
                StopReason::Stop
            } else {
                StopReason::ToolUse
            });
            sent = true;
            Ok(AssistantMessage {
                id: format!("response-{name}"),
                role: "assistant".into(),
                content,
                model: "offline-fixture".into(),
                usage: None,
                stop_reason,
                error_message: None,
            })
        })
        .unwrap();
    events
        .into_iter()
        .find_map(|event| match event {
            AgentEvent::ToolExecutionEnd {
                tool_name,
                details,
                result,
                is_error,
                ..
            } if tool_name == name => Some((details.unwrap_or(Value::Null), result, is_error)),
            _ => None,
        })
        .expect("normal agent must execute one tool")
}

#[test]
fn normal_agent_discovers_edits_replans_and_runs_tests_without_graph() {
    for shared in [false, true] {
        let root = tempfile::tempdir().unwrap();
        let state = tempfile::tempdir().unwrap();
        fixture::populate(root.path());
        fixture::plant_failure(root.path());
        let host = ExtensionHost::load_with_cwd(state.path(), &[], root.path());
        let mut agent = Agent::new("Offline native test impact acceptance fixture.");
        agent.cwd = root.path().to_path_buf();
        agent.permissions = Arc::new(PermissionState::new(PermissionPolicy::new(
            PermissionMode::AlwaysApprove,
        )));
        let shell = if cfg!(windows) { "powershell" } else { "bash" };
        agent.tools = [
            "tool_search",
            "edit",
            "test_plan",
            "test_related",
            "test_impacted",
            "retrieve_output",
            shell,
        ]
        .into_iter()
        .map(String::from)
        .collect();
        agent
            .load_from_session(
                davinci_session::JsonlSession::create(state.path(), "impact-fixture", None)
                    .unwrap(),
            )
            .unwrap();
        host.register_with(&agent.runtime_for_session().unwrap().capability_registry);
        if shared {
            attach_shared_tool_executor(&mut agent, Arc::new(Mutex::new(host.clone())));
        } else {
            attach_tool_executor(&mut agent, &host);
        }
        assert!(!agent
            .provider_tool_specs()
            .iter()
            .any(|s| s.name == "test_plan"));
        let (discovered, _, error) = call(&mut agent, "tool_search", json!({"query":"test_plan"}));
        assert!(!error);
        assert_eq!(discovered["activated"], json!(["test_plan"]));
        assert!(agent
            .provider_tool_specs()
            .iter()
            .any(|s| s.name == "test_plan"));
        assert!(!agent
            .provider_tool_specs()
            .iter()
            .any(|s| s.name.starts_with("graph_")));
        let (before, output, error) =
            call(&mut agent, "test_plan", json!({"paths":[fixture::CHANGED]}));
        assert!(!error, "{output}");
        assert_eq!(before["total"], 3);
        let (_, output, error) = call(
            &mut agent,
            "edit",
            json!({"path":fixture::CHANGED,"oldText":"wrong:","newText":"token:"}),
        );
        assert!(!error, "{output}");
        let (after, output, error) = call(
            &mut agent,
            "test_plan",
            json!({"paths":[fixture::CHANGED],"refresh":true}),
        );
        assert!(!error, "{output}");
        assert_ne!(before["source_identity"], after["source_identity"]);
        assert_eq!(after["total"], 3);
        let paths: Vec<_> = after["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["path"].as_str().unwrap())
            .collect();
        // Only the fixed fixture's portable, non-option paths enter this shell command.
        assert!(paths.iter().all(|p| fixture::IMPACTED.contains(p)));
        let (_, output, error) = call(
            &mut agent,
            shell,
            json!({"command":format!("node --test --test-reporter=tap {}", paths.join(" ")),"timeout":30}),
        );
        assert!(!error, "{output}");
        let text = output.to_string();
        assert!(
            text.contains("# pass 3") && text.contains("# fail 0"),
            "{text}"
        );
        agent
            .permissions
            .lock()
            .unwrap()
            .deny
            .push(PermissionRule::subject("read", fixture::IMPACTED[2]));
        let (_, _, denied) = call(&mut agent, "test_plan", json!({"paths":[fixture::CHANGED]}));
        assert!(
            denied,
            "live revocation must reject previously cached evidence"
        );
    }
}
