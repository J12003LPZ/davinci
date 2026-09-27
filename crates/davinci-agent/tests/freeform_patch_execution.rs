use std::fs;

use davinci_agent::apply_patch::{self, FileAction};
use davinci_agent::runtime::contracts::extract_tool_targets;
use davinci_agent::tools::{execute_tool_with, tool_specs, ToolContext};
use serde_json::json;
use tempfile::tempdir;

#[test]
fn parser_accepts_supported_codex_forms_without_normalizing_content() {
    let patch = "*** Begin Patch\r\n\
*** Add File: empty.txt\r\n\
*** End of File\r\n\
*** Add File: literal.txt\r\n\
+*** Update File: this-is-content\r\n\
+*** End Patch is content\r\n\
*** Update File: unicode.txt\r\n\
@@ replace\r\n\
-old α\r\n\
+new \"β\" / slash\r\n\
*** End of File\r\n\
*** End Patch ***";

    let parsed = apply_patch::parse_codex_patch(patch).expect("valid Codex patch");
    assert_eq!(parsed.actions.len(), 3);
    assert!(matches!(
        &parsed.actions[0],
        FileAction::Add { path, content } if path == "empty.txt" && content.is_empty()
    ));
    assert!(matches!(
        &parsed.actions[1],
        FileAction::Add { path, content }
            if path == "literal.txt"
                && content.contains("*** Update File: this-is-content")
                && content.contains("*** End Patch is content")
    ));
    assert!(matches!(
        &parsed.actions[2],
        FileAction::Update { path, hunks }
            if path == "unicode.txt"
                && hunks.len() == 1
                && hunks[0].lines.len() == 2
    ));

    let no_final_lf = "*** Begin Patch\n*** Add File: no-final-lf.txt\n+data\n*** End Patch";
    assert!(apply_patch::parse_codex_patch(no_final_lf).is_ok());
}

#[test]
fn parser_rejects_malformed_or_unsupported_controls_before_execution() {
    let malformed = "*** Begin Patch\n*** Update File: target.txt\n-old\n+new\n*** End Patch";
    let error = apply_patch::parse_codex_patch(malformed).unwrap_err();
    assert!(error.contains("@@"), "{error}");

    let moved = "*** Begin Patch\n*** Move to: renamed.txt\n*** End Patch";
    let error = apply_patch::parse_codex_patch(moved).unwrap_err();
    assert!(error.contains("*** Move to:"), "{error}");
    assert!(
        error.to_ascii_lowercase().contains("delete plus an add"),
        "{error}"
    );

    let root = tempdir().unwrap();
    let result = execute_tool_with(
        root.path(),
        "apply_patch",
        &json!({"input": malformed}),
        &ToolContext::default(),
    )
    .unwrap();
    assert!(result.is_error);
    assert!(!root.path().join("target.txt").exists());
}

#[test]
fn rejected_terminal_patch_calls_never_reach_agent_execution() {
    let model = davinci_ai::load_builtin_models()
        .into_iter()
        .find(|model| model.api == "openai-codex-responses")
        .unwrap();
    let item = json!({"type":"custom_tool_call","id":"item","call_id":"call",
        "name":"apply_patch","input":"*** Begin Patch\n*** Add File: unsafe.txt\n+executed\n*** End Patch"});
    let mut incomplete_item = item.clone();
    incomplete_item["status"] = "incomplete".into();
    let mut cases = vec![
        vec![
            json!({"type":"response.output_item.added","output_index":0,"item":item}),
            json!({"type":"response.completed","response":{"status":"completed","output":[]}}),
        ],
        vec![
            json!({"type":"response.output_item.done","output_index":0,"item":item}),
            json!({"type":"response.completed","response":{"status":"completed","output":[]}}),
        ],
        vec![
            json!({"type":"response.incomplete","response":{"status":"completed",
            "incomplete_details":{"reason":"max_output_tokens"},"output":[item]}}),
        ],
        vec![
            json!({"type":"response.incomplete","response":{"status":null,
            "incomplete_details":{"reason":"max_output_tokens"},"output":[item]}}),
        ],
        vec![
            json!({"type":"response.completed","response":{"status":"completed",
            "output":[incomplete_item]}}),
        ],
    ];
    for status in ["incomplete", "in_progress", "failed", "completed"] {
        let mut open_item = item.clone();
        open_item["status"] = status.into();
        cases.push(vec![
            json!({"type":"response.output_item.added","output_index":0,"item":open_item}),
            json!({"type":"response.completed","response":{"status":"completed"}}),
        ]);
    }
    for frames in cases {
        let root = tempdir().unwrap();
        let corpus = frames
            .iter()
            .map(|frame| format!("data: {frame}\n\n"))
            .collect::<String>();
        let decoded = davinci_ai::fixture_complete(&model, &[], &corpus);
        let mut agent = davinci_agent::Agent::new("terminal safety regression");
        agent.cwd = root.path().to_path_buf();
        agent.set_permission_mode(davinci_agent::PermissionMode::AlwaysApprove);
        agent.prompt_user_with("Apply the requested patch", &[]);
        let mut requests = 0;
        let events = agent
            .run_loop(|_| {
                requests += 1;
                assert!(requests <= 2, "unexpected continuation: {corpus}");
                Ok(if requests == 1 {
                    decoded.clone()
                } else {
                    serde_json::from_value::<davinci_ai::AssistantMessage>(json!({
                        "id":"done","role":"assistant","model":"fixture",
                        "content":[{"type":"text","text":"done"}],"stopReason":"stop"
                    }))
                    .unwrap()
                })
            })
            .unwrap();
        assert!(!root.path().join("unsafe.txt").exists(), "{corpus}");
        assert!(
            !events
                .iter()
                .any(|event| matches!(event, davinci_agent::AgentEvent::ToolExecutionStart { .. })),
            "{corpus}"
        );
    }
}

#[test]
fn rejected_provider_calls_are_absent_from_saved_history_and_execution() {
    for status in ["length", "error", "aborted"] {
        let root = tempdir().unwrap();
        let mut agent = davinci_agent::Agent::new("provider boundary regression");
        agent.cwd = root.path().to_path_buf();
        agent.set_permission_mode(davinci_agent::PermissionMode::AlwaysApprove);
        agent.session =
            Some(davinci_session::JsonlSession::create(root.path(), status, None).unwrap());
        let session_path = agent.session.as_ref().unwrap().path.clone();
        agent.prompt_user_with("Write the requested file", &[]);
        let response: davinci_ai::AssistantMessage = serde_json::from_value(json!({
            "id":"partial","role":"assistant","model":"fixture","stopReason":status,
            "content":[{"type":"text","text":"Partial explanation"},
                {"type":"toolCall","id":"pending","name":"write",
                    "arguments":{"path":"unsafe.txt","content":"executed"}}],
            "responsesToolWireKinds":{"pending":"function"}
        }))
        .unwrap();
        let events = agent.run_loop(|_| Ok(response.clone())).unwrap();
        assert!(!root.path().join("unsafe.txt").exists());
        assert!(!events
            .iter()
            .any(|event| matches!(event, davinci_agent::AgentEvent::ToolExecutionStart { .. })));
        let mut reopened = davinci_agent::Agent::new("reopened");
        reopened
            .load_from_session(davinci_session::JsonlSession::open(&session_path).unwrap())
            .unwrap();
        for messages in [&agent.messages, &reopened.messages] {
            assert!(
                !messages
                    .iter()
                    .flat_map(|message| &message.content)
                    .any(|block| matches!(block, davinci_ai::MessageContent::ToolCall { .. })),
                "{status}"
            );
            assert!(!messages.iter().any(|message| message
                .extra
                .contains_key(davinci_ai::RESPONSES_TOOL_WIRE_KINDS_KEY)));
            assert!(messages.iter().any(|message| message.role == "assistant"
                && davinci_ai::content_text(&message.content).contains("Partial explanation")));
        }
    }
}

#[test]
fn decoded_freeform_and_json_apply_patch_share_targets_and_transaction_effects() {
    let patch = "*** Begin Patch\n*** Update File: a.txt\n@@\n-old\n+new\n*** Add File: b.txt\n+β\n*** End Patch";
    let decode = |custom: bool| {
        let model = davinci_ai::load_builtin_models()
            .into_iter()
            .find(|model| model.api == "openai-codex-responses")
            .unwrap();
        let item = if custom {
            json!({"type":"custom_tool_call", "id":"patch-item", "call_id":"patch-call", "name":"apply_patch", "input":patch})
        } else {
            json!({"type":"function_call", "id":"patch-item", "call_id":"patch-call", "name":"apply_patch", "arguments":json!({"input":patch}).to_string()})
        };
        let terminal = json!({"type":"response.completed", "response":{"status":"completed", "output":[item]}});
        let message = davinci_ai::fixture_complete(&model, &[], &format!("data: {terminal}\n\n"));
        let chat = davinci_ai::assistant_to_chat(&message);
        assert_eq!(
            chat.extra[davinci_ai::RESPONSES_TOOL_WIRE_KINDS_KEY]["patch-call|patch-item"],
            if custom { "custom" } else { "function" }
        );
        match &message.content[0] {
            davinci_ai::ContentBlock::ToolCall { arguments, .. } => arguments.clone(),
            other => panic!("expected decoded patch call, got {other:?}"),
        }
    };
    let json_input = decode(false);
    let targets = extract_tool_targets("apply_patch", &json_input);
    assert_eq!(targets, vec!["a.txt", "b.txt"]);

    let json_root = tempdir().unwrap();
    fs::write(json_root.path().join("a.txt"), "old\n").unwrap();
    let json_result = execute_tool_with(
        json_root.path(),
        "apply_patch",
        &json_input,
        &ToolContext::default(),
    )
    .unwrap();
    assert!(!json_result.is_error, "{}", json_result.content);

    let freeform_root = tempdir().unwrap();
    fs::write(freeform_root.path().join("a.txt"), "old\n").unwrap();
    let decoded_input = decode(true);
    assert_eq!(extract_tool_targets("apply_patch", &decoded_input), targets);
    let decoded_result = execute_tool_with(
        freeform_root.path(),
        "apply_patch",
        &decoded_input,
        &ToolContext::default(),
    )
    .unwrap();
    assert!(!decoded_result.is_error, "{}", decoded_result.content);

    assert_eq!(
        fs::read_to_string(json_root.path().join("a.txt")).unwrap(),
        fs::read_to_string(freeform_root.path().join("a.txt")).unwrap()
    );
    assert_eq!(
        fs::read_to_string(json_root.path().join("b.txt")).unwrap(),
        fs::read_to_string(freeform_root.path().join("b.txt")).unwrap()
    );
    let read = execute_tool_with(
        json_root.path(),
        "read",
        &json!({"path":"a.txt"}),
        &ToolContext::default(),
    )
    .unwrap();
    assert!(read.content.contains("new"));
}

#[test]
fn update_preserves_a_source_file_without_a_final_newline() {
    let root = tempdir().unwrap();
    fs::write(root.path().join("no-final-lf.txt"), "old").unwrap();
    let patch = "*** Begin Patch\n*** Update File: no-final-lf.txt\n@@\n-old\n+new\n*** End Patch";
    let result = execute_tool_with(
        root.path(),
        "apply_patch",
        &json!({"input": patch}),
        &ToolContext::default(),
    )
    .unwrap();
    assert!(!result.is_error, "{}", result.content);
    assert_eq!(
        fs::read(root.path().join("no-final-lf.txt")).unwrap(),
        b"new"
    );
}

#[test]
fn edit_schema_exposes_only_the_required_new_shape_while_legacy_parser_remains_usable() {
    let edit = tool_specs()
        .into_iter()
        .find(|tool| tool.name == "edit")
        .expect("edit tool");
    let required = edit.parameters["required"].as_array().unwrap();
    assert!(required.iter().any(|value| value == "path"));
    assert!(required.iter().any(|value| value == "edits"));
    assert!(edit.parameters["properties"].get("oldText").is_none());
    assert!(edit.parameters["properties"].get("newText").is_none());
    assert_eq!(edit.parameters["properties"]["edits"]["minItems"], 1);
    let item_required = edit.parameters["properties"]["edits"]["items"]["required"]
        .as_array()
        .unwrap();
    assert!(item_required.iter().any(|value| value == "oldText"));
    assert!(item_required.iter().any(|value| value == "newText"));

    let root = tempdir().unwrap();
    fs::write(root.path().join("legacy.txt"), "before\n").unwrap();
    let result = execute_tool_with(
        root.path(),
        "edit",
        &json!({"path":"legacy.txt", "oldText":"before", "newText":"after"}),
        &ToolContext::default(),
    )
    .unwrap();
    assert!(!result.is_error, "{}", result.content);
    assert_eq!(
        fs::read_to_string(root.path().join("legacy.txt")).unwrap(),
        "after\n"
    );
}
