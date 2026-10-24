use super::*;
use crate::{
    CompletionEvidence, MutationVerificationState, VerificationCoverage, VerificationEvidence,
};
use davinci_session::JsonlSession;
use std::sync::Arc;

fn break_storage(path: &Path) {
    std::fs::rename(path, path.with_extension("backup")).unwrap();
    std::fs::create_dir(path).unwrap();
}

fn completion(tool: bool) -> AssistantMessage {
    AssistantMessage {
        extra: Default::default(),
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
        let call_ids: Vec<_> = agent
            .messages
            .iter()
            .filter(|message| message.role == "assistant")
            .flat_map(|message| message.content.iter())
            .filter_map(|content| match content {
                davinci_ai::MessageContent::ToolCall { id, .. } => Some(id.clone()),
                _ => None,
            })
            .collect();
        for id in call_ids {
            assert!(
                agent.messages.iter().any(|message| {
                    message.role == "toolResult" && message.tool_call_id.as_deref() == Some(&id)
                }),
                "{boundary}: dangling tool call {id}"
            );
        }
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

fn decoded_patch(custom: bool, status: &str) -> AssistantMessage {
    let model = davinci_ai::load_builtin_models()
        .into_iter()
        .find(|model| model.api == "openai-codex-responses")
        .unwrap();
    let patch = "*** Begin Patch\n*** Add File: unexpected.txt\n+effect\n*** End Patch";
    let item = if custom {
        serde_json::json!({"type":"custom_tool_call", "id":"item", "call_id":"patch", "name":"apply_patch", "input":patch})
    } else {
        serde_json::json!({"type":"function_call", "id":"item", "call_id":"patch", "name":"apply_patch", "arguments":serde_json::json!({"input":patch}).to_string()})
    };
    let event = serde_json::json!({"type":"response.completed", "response":{"status":status, "incomplete_details":{"reason":"max_output_tokens"}, "output":[item]}});
    davinci_ai::fixture_complete(&model, &[], &format!("data: {event}\n\n"))
}

#[test]
fn session_reopen_preserves_decoded_json_and_custom_patch_call_result_pairs() {
    for custom in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let session = JsonlSession::create(dir.path(), dir.path().to_str().unwrap(), None).unwrap();
        let path = session.path.clone();
        let mut agent = Agent::new("wire identity");
        agent.session = Some(session);
        let assistant = decoded_patch(custom, "completed");
        let chat = davinci_ai::assistant_to_chat(&assistant);
        agent.messages.push(chat.clone());
        agent.persist_assistant(&assistant, &chat, None);
        let mut result = ChatMessage::tool_result("patch|item", "apply_patch", "applied", false);
        agent.annotate_tool_result_wire_kind(&mut result);
        agent.persist_chat(&result).unwrap();
        drop(agent);
        let reopened = JsonlSession::open(&path).unwrap();
        let history = crate::messages_from_session(&reopened);
        assert_eq!(
            history[0].extra[davinci_ai::RESPONSES_TOOL_WIRE_KINDS_KEY],
            chat.extra[davinci_ai::RESPONSES_TOOL_WIRE_KINDS_KEY]
        );
        assert_eq!(history[1], result);
        let input = davinci_ai::openai_responses_input(&history);
        assert_eq!(
            input[0]["type"],
            if custom {
                "custom_tool_call"
            } else {
                "function_call"
            }
        );
        assert_eq!(
            input[1]["type"],
            if custom {
                "custom_tool_call_output"
            } else {
                "function_call_output"
            }
        );
    }
}

#[test]
fn interrupted_turn_repair_uses_the_originating_call_kind() {
    for custom in [false, true] {
        let mut agent = Agent::new("failed turn");
        agent
            .messages
            .push(davinci_ai::assistant_to_chat(&decoded_patch(
                custom,
                "completed",
            )));
        agent.fail_turn("storage failed");
        let result = agent.messages.last().unwrap();
        assert_eq!(
            result.extra[davinci_ai::RESPONSES_TOOL_WIRE_KIND_KEY],
            if custom { "custom" } else { "function" }
        );
        assert_eq!(
            davinci_ai::openai_responses_input(&agent.messages)[1]["type"],
            if custom {
                "custom_tool_call_output"
            } else {
                "function_call_output"
            }
        );
    }
}

#[test]
fn noncompleted_patch_turns_never_execute_or_continue() {
    for status in ["incomplete", "failed", "cancelled", "queued", "in_progress"] {
        let dir = tempfile::tempdir().unwrap();
        let mut agent = Agent::new("partial patch");
        agent.cwd = dir.path().into();
        agent.tools = vec!["apply_patch".into()];
        agent.set_permission_mode(crate::PermissionMode::AlwaysApprove);
        // Only an output-limit stop earns a bounded corrective turn; nothing
        // from a noncompleted response ever executes.
        let length_stop = decoded_patch(true, status).stop_reason == Some(StopReason::Length);
        let mut calls = 0;
        agent
            .run_loop(|_| {
                calls += 1;
                Ok(decoded_patch(true, status))
            })
            .unwrap();
        if length_stop {
            assert_eq!(calls, 3, "{status}: one request plus two bounded retries");
        } else {
            assert_eq!(calls, 1, "{status} must not execute tools and continue");
        }
        assert!(!dir.path().join("unexpected.txt").exists());
        assert_eq!(agent.run_stats().tool_calls, 0);
    }
}

#[test]
fn write_failure_surfaces_before_successful_turn_completion() {
    let dir = tempfile::tempdir().unwrap();
    let session = JsonlSession::create(dir.path(), dir.path().to_str().unwrap(), None).unwrap();
    let path = session.path.clone();
    let mut agent = Agent::new("write failure");
    agent.cwd = dir.path().to_path_buf();
    agent.session = Some(session);
    agent.prompt("remember this request");

    // The storage breaks while the model answers: the answer is not reported
    // as a finished turn, and the error carries the stable code.
    let mut calls = 0;
    let result = agent.run_loop(|_| {
        calls += 1;
        break_storage(&path);
        Ok(completion(false))
    });
    let error = result.expect_err("the turn finished as if it were saved");
    assert!(error.starts_with("PERSISTENCE_FAILURE"), "{error}");
    assert_eq!(calls, 1);

    // The session on disk is still readable and holds the saved request.
    std::fs::remove_dir(&path).unwrap();
    std::fs::rename(path.with_extension("backup"), &path).unwrap();
    let reopened = JsonlSession::open(&path).unwrap();
    let saved = crate::messages_from_session(&reopened);
    assert_eq!(saved.len(), 1, "{saved:?}");
    assert_eq!(saved[0].role, "user");
}

#[test]
fn a_dropped_write_outside_the_sticky_error_still_blocks_the_next_request() {
    let dir = tempfile::tempdir().unwrap();
    let session = JsonlSession::create(dir.path(), dir.path().to_str().unwrap(), None).unwrap();
    let path = session.path.clone();
    let mut agent = Agent::new("recorded failure");
    agent.session = Some(session);
    agent.persistence_failure = Some((path, "disk full".into()));
    let error = agent
        .run_loop(|_| -> Result<AssistantMessage, String> {
            panic!("a request was sent after a lost write")
        })
        .unwrap_err();
    assert!(error.starts_with("PERSISTENCE_FAILURE"), "{error}");
    assert!(error.contains("disk full"));
    // Another session is not blamed for the first one's failure.
    let other = JsonlSession::create(dir.path(), dir.path().to_str().unwrap(), None).unwrap();
    agent.session = Some(other);
    assert!(agent.ensure_session_persistence().is_ok());
}

#[test]
fn reopening_the_same_path_after_a_failed_write_lets_the_next_turn_run() {
    let dir = tempfile::tempdir().unwrap();
    let session = JsonlSession::create(dir.path(), dir.path().to_str().unwrap(), None).unwrap();
    let path = session.path.clone();
    let mut agent = Agent::new("same path");
    agent.session = Some(session);
    agent.prompt("first request");
    agent.persistence_failure = Some((path.clone(), "disk full".into()));
    assert!(agent
        .run_loop(|_| -> Result<AssistantMessage, String> { panic!("sent after a lost write") })
        .unwrap_err()
        .starts_with("PERSISTENCE_FAILURE"));

    // The disk recovered and the user resumes the very same file.
    agent
        .load_from_session(JsonlSession::open(&path).unwrap())
        .expect("the session reopens");
    assert!(agent.ensure_session_persistence().is_ok());
    agent.prompt("second request");
    agent
        .run_loop(|_| Ok(completion(false)))
        .expect("a reopened session runs a turn");
}

#[test]
fn a_host_side_write_failure_is_recorded_for_the_open_session() {
    let dir = tempfile::tempdir().unwrap();
    let session = JsonlSession::create(dir.path(), dir.path().to_str().unwrap(), None).unwrap();
    let mut agent = Agent::new("plugin write");
    agent.session = Some(session);
    agent.record_persistence_failure("plugin appendEntry: disk full");
    let error = agent.ensure_session_persistence().unwrap_err();
    assert!(error.starts_with("PERSISTENCE_FAILURE"), "{error}");
    assert!(error.contains("plugin appendEntry: disk full"), "{error}");
}

/// Preview, through the agent so the owner matches, a patch that rewrites
/// `a.txt` from "before" to "after". Returns the apply arguments.
fn previewed_patch(agent: &mut Agent, root: &Path) -> serde_json::Value {
    std::fs::write(
        root.join("a.txt"),
        "before
",
    )
    .unwrap();
    let preview = agent
        .execute_host_handoff_tool(
            root,
            "preview",
            "patch_preview",
            serde_json::json!({
                "input":"*** Begin Patch
*** Update File: a.txt
@@
-before
+after
*** End Patch"
            }),
        )
        .unwrap();
    assert_ne!(preview.is_error, Some(true), "{preview:?}");
    let id = preview.extra["details"]["transaction"]["id"].clone();
    serde_json::json!({"id": id, "paths": ["a.txt"]})
}

fn verified_agent(dir: &Path, session: JsonlSession) -> Agent {
    let mut agent = Agent::new("host effect fixture");
    agent.cwd = dir.to_path_buf();
    agent.session = Some(session);
    agent.set_permission_mode(crate::PermissionMode::AlwaysApprove);
    *agent.mutation_verification.lock().unwrap() = MutationVerificationState {
        mutation_generation: 2,
        verified_generation: Some(2),
        last_verification_succeeded: true,
        latest_evidence: Some(VerificationEvidence {
            generation: 2,
            command: "cargo test".into(),
            succeeded: true,
            mutation_paths: Vec::new(),
            verification_targets: Vec::new(),
            coverage: VerificationCoverage::Broad,
        }),
        ..MutationVerificationState::default()
    };
    agent.persist_mutation_verification();
    assert_eq!(agent.completion_evidence(), CompletionEvidence::Verified);
    agent
}

fn effect_ids(agent: &Agent, kind: &str) -> Vec<String> {
    agent
        .session
        .as_ref()
        .unwrap()
        .entries
        .iter()
        .filter(|entry| entry.custom_type.as_deref() == Some(kind))
        .map(|entry| entry.extra["data"]["id"].as_str().unwrap().to_owned())
        .collect()
}

#[test]
fn a_failed_pending_marker_write_stops_the_host_tool_before_it_runs() {
    let dir = tempfile::tempdir().unwrap();
    let session = JsonlSession::create(dir.path(), dir.path().to_str().unwrap(), None).unwrap();
    let path = session.path.clone();
    let mut agent = verified_agent(dir.path(), session);
    let args = previewed_patch(&mut agent, dir.path());
    break_storage(&path);
    let error = agent
        .execute_host_handoff_tool(dir.path(), "h1", "patch_apply", args)
        .unwrap_err();
    assert!(error.starts_with(crate::PERSISTENCE_FAILURE), "{error}");
    assert!(error.contains("was not run"), "{error}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "before\n",
        "the patch ran although its marker was not durable"
    );
}

#[test]
fn a_host_tool_leaves_a_resolved_marker_for_its_pending_marker() {
    let dir = tempfile::tempdir().unwrap();
    let session = JsonlSession::create(dir.path(), dir.path().to_str().unwrap(), None).unwrap();
    let mut agent = verified_agent(dir.path(), session);
    let args = previewed_patch(&mut agent, dir.path());
    let result = agent
        .execute_host_handoff_tool(dir.path(), "h1", "patch_apply", args)
        .unwrap();
    assert_ne!(result.is_error, Some(true), "{result:?}");
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.txt")).unwrap(),
        "after\n"
    );
    // The preview and the apply each leave a closed pair; only a few readers
    // are provably read-only, so every other tool is recorded.
    let pending = effect_ids(&agent, "mutation_effect_pending");
    assert_eq!(pending.len(), 2);
    assert_eq!(pending, effect_ids(&agent, "mutation_effect_resolved"));
    // The marker precedes the mutation state it vouches for.
    let position = |kind: &str| {
        agent
            .session
            .as_ref()
            .unwrap()
            .entries
            .iter()
            .rposition(|entry| entry.custom_type.as_deref() == Some(kind))
            .unwrap()
    };
    assert!(position("mutation_effect_pending") < position("mutation_effect_resolved"));
}

#[test]
fn a_crash_after_a_host_mutation_marker_resumes_unverified() {
    let dir = tempfile::tempdir().unwrap();
    let session = JsonlSession::create(dir.path(), dir.path().to_str().unwrap(), None).unwrap();
    let path = session.path.clone();
    let mut agent = verified_agent(dir.path(), session);
    let args = previewed_patch(&mut agent, dir.path());
    agent
        .execute_host_handoff_tool(dir.path(), "h1", "patch_apply", args)
        .unwrap();
    // Cut the log right after the apply's pending marker: the patch landed, nothing
    // else was recorded.
    let log = std::fs::read_to_string(&path).unwrap();
    let lines: Vec<_> = log.lines().collect();
    let cut = lines
        .iter()
        .rposition(|line| line.contains("mutation_effect_pending"))
        .unwrap();
    let kept = &lines[..=cut];
    std::fs::write(&path, kept.join("\n") + "\n").unwrap();
    let mut resumed = Agent::new("host effect fixture");
    resumed
        .load_from_session(JsonlSession::open(&path).unwrap())
        .unwrap();
    assert_eq!(
        resumed.completion_evidence(),
        CompletionEvidence::Unverified
    );
}

#[test]
fn a_tool_result_is_never_durable_ahead_of_its_mutation_state() {
    let dir = tempfile::tempdir().unwrap();
    let session = JsonlSession::create(dir.path(), dir.path().to_str().unwrap(), None).unwrap();
    let path = session.path.clone();
    let mut agent = verified_agent(dir.path(), session);
    std::fs::write(dir.path().join("a.txt"), "one").unwrap();
    agent.record_successful_mutation_paths(vec![PathBuf::from("a.txt")]);
    agent
        .persist_chat(&ChatMessage::tool_result("c1", "write", "saved", false))
        .unwrap();
    let reopened = JsonlSession::open(&path).unwrap();
    let state_at = reopened
        .entries
        .iter()
        .rposition(|entry| entry.custom_type.as_deref() == Some("mutation_verification"))
        .unwrap();
    let result_at = reopened
        .entries
        .iter()
        .position(|entry| entry.entry_type == "message")
        .unwrap();
    assert!(
        state_at < result_at,
        "the result was written before its state"
    );
}
