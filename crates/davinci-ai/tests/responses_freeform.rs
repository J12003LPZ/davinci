use davinci_ai::responses_ledger::{ResponsesItem, ResponsesLedger};
use davinci_ai::{
    assistant_to_chat, load_builtin_models, openai_responses_input_with, request_body_with,
    set_wire_kind, ChatMessage, ContentBlock, MessageContent, ResponsesInputOptions,
    ResponsesToolWireKind, StopReason, StreamDecoder, StreamOptions, ToolSpec,
    ResponsesDecoder,
};
use serde_json::json;

fn codex_model() -> davinci_ai::Model {
    let mut model = load_builtin_models()
        .into_iter()
        .find(|model| model.api == "openai-codex-responses")
        .expect("codex model");
    model.base_url = None;
    model
}

fn tools() -> Vec<ToolSpec> {
    vec![
        ToolSpec {
            name: "read".into(),
            description: "Read a file".into(),
            parameters: json!({
                "type": "object",
                "properties": {"path": {"type": "string"}},
                "required": ["path"]
            }),
            constrained_sampling: None,
        },
        ToolSpec {
            name: "apply_patch".into(),
            description: "Apply an exact patch".into(),
            parameters: json!({"type": "object"}),
            constrained_sampling: None,
        },
    ]
}

#[test]
fn supported_body_has_one_custom_patch_and_function_fallback_for_other_tools() {
    let model = codex_model();
    let body = request_body_with(
        &model,
        &[ChatMessage::text("user", "patch this")],
        None,
        &tools(),
        &StreamOptions {
            responses_is_oauth: Some(true),
            ..StreamOptions::default()
        },
    );
    let wire_tools = body["tools"].as_array().expect("tools");
    assert_eq!(
        wire_tools
            .iter()
            .filter(|tool| tool["name"] == "apply_patch")
            .count(),
        1
    );
    assert_eq!(
        wire_tools
            .iter()
            .find(|tool| tool["name"] == "apply_patch")
            .expect("patch tool")["type"],
        "custom"
    );
    assert_eq!(
        wire_tools
            .iter()
            .find(|tool| tool["name"] == "read")
            .expect("read tool")["type"],
        "function"
    );

    let proxy = {
        let mut model = model.clone();
        model.base_url = Some("https://proxy.example.test/v1".into());
        request_body_with(
            &model,
            &[],
            None,
            &tools(),
            &StreamOptions {
                responses_is_oauth: Some(true),
                ..StreamOptions::default()
            },
        )
    };
    assert_eq!(proxy["tools"][1]["type"], "function");
}

#[test]
fn custom_history_round_trips_lossless_input_and_output() {
    let raw_patch = "*** Begin Patch\n*** Update File: src/π.rs\n@@\n-\"old\"\\\\\n+\"new\"\n*** End Patch";
    let mut assistant = ChatMessage {
        role: "assistant".into(),
        content: vec![MessageContent::ToolCall {
            id: "call-1|item-1".into(),
            name: "apply_patch".into(),
            arguments: json!({"input": raw_patch}),
        }],
        ..ChatMessage::default()
    };
    set_wire_kind(
        &mut assistant.extra,
        "call-1|item-1",
        ResponsesToolWireKind::Custom,
    );
    let mut result = ChatMessage::tool_result("call-1|item-1", "apply_patch", "applied", false);
    set_wire_kind(
        &mut result.extra,
        "call-1|item-1",
        ResponsesToolWireKind::Custom,
    );
    let input = openai_responses_input_with(
        &[assistant.clone(), result.clone()],
        &ResponsesInputOptions {
            custom_tools: &["apply_patch"],
            ..ResponsesInputOptions::default()
        },
    );
    assert_eq!(input[0]["type"], "custom_tool_call");
    assert_eq!(input[0]["input"], raw_patch);
    assert_eq!(input[1]["type"], "custom_tool_call_output");
    assert_eq!(input[1]["call_id"], "call-1");

    let ledger = ResponsesLedger::from_messages("lineage", &[assistant, result]);
    assert!(matches!(ledger.items[0], ResponsesItem::CustomToolCall { .. }));
    assert!(matches!(ledger.items[1], ResponsesItem::CustomToolCallOutput { .. }));
}

#[test]
fn explicit_function_metadata_survives_apply_patch_name() {
    let mut assistant = ChatMessage {
        role: "assistant".into(),
        content: vec![MessageContent::ToolCall {
            id: "function-call".into(),
            name: "apply_patch".into(),
            arguments: json!({"input": "legacy"}),
        }],
        ..ChatMessage::default()
    };
    set_wire_kind(
        &mut assistant.extra,
        "function-call",
        ResponsesToolWireKind::Function,
    );
    let input = openai_responses_input_with(
        &[assistant.clone()],
        &ResponsesInputOptions {
            custom_tools: &["apply_patch"],
            ..ResponsesInputOptions::default()
        },
    );
    assert_eq!(input[0]["type"], "function_call");
    let ledger = ResponsesLedger::from_messages("lineage", &[assistant]);
    assert!(matches!(ledger.items[0], ResponsesItem::FunctionCall { .. }));
}

#[test]
fn historical_function_defaults_are_not_rewritten_by_current_custom_visibility() {
    let assistant = ChatMessage {
        role: "assistant".into(),
        content: vec![MessageContent::ToolCall {
            id: "old-function".into(),
            name: "apply_patch".into(),
            arguments: json!({"input": "legacy patch"}),
        }],
        ..ChatMessage::default()
    };
    let result = ChatMessage::tool_result("old-function", "apply_patch", "legacy result", false);
    let input = openai_responses_input_with(
        &[assistant.clone(), result.clone()],
        &ResponsesInputOptions {
            custom_tools: &["apply_patch"],
            ..ResponsesInputOptions::default()
        },
    );
    assert_eq!(input[0]["type"], "function_call");
    assert_eq!(input[1]["type"], "function_call_output");
    let ledger = ResponsesLedger::from_messages("lineage", &[assistant, result]);
    assert!(matches!(ledger.items[0], ResponsesItem::FunctionCall { .. }));
    assert!(matches!(ledger.items[1], ResponsesItem::FunctionCallOutput { .. }));
}

#[test]
fn decoder_normalizes_custom_call_input_without_losing_unicode_or_order() {
    let model = codex_model();
    let raw_patch = "*** Begin Patch\n*** Add File: src/π.rs\n+quotes: \\\" / \\\\ \\n+*** End Patch";
    let events = [
        json!({
            "type": "response.output_item.added",
            "output_index": 0,
            "item": {"type": "custom_tool_call", "id": "item-custom", "call_id": "custom-1", "name": "apply_patch", "input": ""}
        }),
        json!({"type": "response.custom_tool_call_input.delta", "output_index": 0, "delta": raw_patch}),
        json!({"type": "response.custom_tool_call_input.done", "output_index": 0, "input": raw_patch}),
        json!({
            "type": "response.output_item.added",
            "output_index": 1,
            "item": {"type": "function_call", "id": "item-function", "call_id": "function-1", "name": "read", "arguments": ""}
        }),
        json!({"type": "response.function_call_arguments.delta", "output_index": 1, "delta": "{\"path\":\"Cargo.toml\"}"}),
        json!({
            "type": "response.output_item.done",
            "output_index": 0,
            "item": {"type": "custom_tool_call", "id": "item-custom", "call_id": "custom-1", "name": "apply_patch", "input": raw_patch}
        }),
        json!({
            "type": "response.output_item.done",
            "output_index": 1,
            "item": {"type": "function_call", "id": "item-function", "call_id": "function-1", "name": "read", "arguments": "{\"path\":\"Cargo.toml\"}"}
        }),
        json!({"type": "response.completed", "response": {"id": "resp-1", "status": "completed", "output": []}}),
    ];
    let mut decoder = ResponsesDecoder::new(&model);
    let mut stream_events = Vec::new();
    for event in &events {
        decoder.feed(event, &mut stream_events);
    }
    let message = decoder.finish(&mut stream_events);
    assert_eq!(message.stop_reason, Some(StopReason::ToolUse));
    assert_eq!(message.content.len(), 2);
    assert!(matches!(
        &message.content[0],
        ContentBlock::ToolCall { name, arguments, .. }
            if name == "apply_patch" && arguments["input"] == raw_patch
    ));
    assert!(matches!(
        &message.content[1],
        ContentBlock::ToolCall { name, arguments, .. }
            if name == "read" && arguments["path"] == "Cargo.toml"
    ));
    let chat = assistant_to_chat(&message);
    assert_eq!(chat.extra["responsesToolWireKinds"]["custom-1|item-custom"], "custom");
}
