use super::*;
use davinci_session::JsonlSession;
use std::sync::Arc;

fn break_storage(path: &Path) {
    std::fs::rename(path, path.with_extension("backup")).unwrap();
    std::fs::create_dir(path).unwrap();
}

fn completion(tool: bool) -> AssistantMessage {
    AssistantMessage {
        id: "fixture".into(),
        role: "assistant".into(),
        content: if tool {
            vec![ContentBlock::ToolCall {
                id: "durable-write".into(),
                name: "write".into(),
                arguments: serde_json::json!({"path":"result.txt", "content":"once"}),
            }]
        } else {
            vec![ContentBlock::Text {
                text: "done".into(),
            }]
        },
        model: "fixture".into(),
        usage: None,
        stop_reason: Some(if tool {
            StopReason::ToolUse
        } else {
            StopReason::Stop
        }),
        error_message: None,
    }
}

#[test]
fn session_persistence_failure_blocks_provider_and_mutation_boundaries() {
    for boundary in ["prompt", "assistant", "result"] {
        let dir = tempfile::tempdir().unwrap();
        let session = JsonlSession::create(dir.path(), dir.path().to_str().unwrap(), None).unwrap();
        let path = session.path.clone();
        let mut agent = Agent::new("storage fixture");
        agent.cwd = dir.path().to_path_buf();
        agent.set_permission_mode(crate::PermissionMode::AlwaysApprove);
        agent.session = Some(session);
        if boundary == "prompt" {
            break_storage(&path);
        }
        agent.prompt("write once");
        if boundary == "result" {
            let path = path.clone();
            agent.post_tool = Some(crate::PostToolHook(Arc::new(move |_, _, _, _, result| {
                break_storage(&path);
                result
            })));
        }
        let mut calls = 0;
        let result = agent.run_loop(|_| {
            calls += 1;
            if calls == 1 && boundary == "assistant" {
                break_storage(&path);
            }
            Ok(completion(calls == 1))
        });
        assert!(result.is_err(), "{boundary}: continued with lost history");
        assert!(result.unwrap_err().contains("Session recovery required"));
        assert_eq!(calls, usize::from(boundary != "prompt"), "{boundary}");
        assert_eq!(dir.path().join("result.txt").exists(), boundary == "result");
        assert!(!agent.is_streaming);
        let result = agent.run_loop(|_| -> Result<AssistantMessage, String> {
            panic!("a failed session must be reopened before another request")
        });
        assert!(result.is_err());
    }
}

#[test]
fn session_persistence_preserves_tool_result_identity() {
    let dir = tempfile::tempdir().unwrap();
    let session = JsonlSession::create(dir.path(), dir.path().to_str().unwrap(), None).unwrap();
    let path = session.path.clone();
    let mut agent = Agent::new("result identity");
    agent.session = Some(session);
    let mut message = ChatMessage::tool_result("call-7", "write", "saved", false);
    message
        .extra
        .insert("fixture".into(), serde_json::json!(true));
    agent.persist_chat(&message).unwrap();
    let reopened = JsonlSession::open(&path).unwrap();
    let messages = crate::messages_from_session(&reopened);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0], message);
}

#[test]
fn session_persistence_prompt_identity_reports_write_failure() {
    let dir = tempfile::tempdir().unwrap();
    let session = JsonlSession::create(dir.path(), dir.path().to_str().unwrap(), None).unwrap();
    break_storage(&session.path);
    let mut agent = Agent::new_builtin(crate::prompt::PromptProfile::Stable);
    agent.session = Some(session);
    assert!(agent.persist_prompt_session().is_err());
}
