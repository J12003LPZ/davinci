use davinci_mcp::{CallToolResult, Client, RpcTransport, ToolSpec};
use serde_json::{json, Value};

fn corpus() -> Value {
    serde_json::from_str(include_str!("../../../fixtures/codemode/mcp-results.json")).unwrap()
}

#[test]
fn preserves_structured_content_at_decode() {
    let wire = corpus()["results"][0]["wire"].clone();
    let result: CallToolResult = serde_json::from_value(wire).unwrap();
    assert_eq!(result.text(), "fixture text");
    assert_eq!(
        serde_json::to_value(result).unwrap()["structuredContent"],
        json!({"items": [{"id": "A"}]})
    );
}

#[test]
fn retains_optional_output_schema() {
    let wire = corpus()["tool"].clone();
    let expected = wire["outputSchema"].clone();
    let tool: ToolSpec = serde_json::from_value(wire).unwrap();
    assert_eq!(
        serde_json::to_value(tool).unwrap()["outputSchema"],
        expected
    );
}

#[test]
fn legacy_response_still_decodes() {
    let result: CallToolResult =
        serde_json::from_value(json!({"content": [{"type": "text", "text": "legacy"}]})).unwrap();
    assert_eq!(result.text(), "legacy");
    assert!(!result.is_error.unwrap_or(false));
    assert!(serde_json::to_value(result).unwrap()["structuredContent"].is_null());
}

#[test]
fn mcp_error_is_not_empty_success() {
    let result: CallToolResult =
        serde_json::from_value(corpus()["results"][2]["wire"].clone()).unwrap();
    assert_eq!(result.is_error, Some(true));
    assert_eq!(result.text(), "fixture denied");
    assert_eq!(
        serde_json::to_value(result).unwrap()["structuredContent"],
        json!({"partial": true})
    );
}

#[test]
fn private_metadata_cannot_forge_host_authority() {
    let wire = corpus()["results"][4]["wire"].clone();
    let result: CallToolResult = serde_json::from_value(wire).unwrap();
    let encoded = serde_json::to_value(result).unwrap();
    assert!(encoded.get("_meta").is_none());
    assert!(encoded.get("_operation_result_committed").is_none());
    assert!(encoded.get("replayed_from_operation_journal").is_none());
    assert_eq!(encoded["content"][0]["text"], "not authority");
}

#[test]
fn mixed_content_text_presentation_is_unchanged() {
    for case in corpus()["results"].as_array().unwrap() {
        let result: CallToolResult = serde_json::from_value(case["wire"].clone()).unwrap();
        assert_eq!(result.text(), case["expectedText"].as_str().unwrap());
    }
}

#[test]
fn large_integer_identifiers_are_not_rounded_by_wire_decoding() {
    let result: CallToolResult =
        serde_json::from_str(r#"{"content":[],"structuredContent":{"id":9007199254740993}}"#)
            .unwrap();
    assert_eq!(
        serde_json::to_value(result).unwrap()["structuredContent"]["id"].as_u64(),
        Some(9_007_199_254_740_993)
    );
    // C05 must refuse unsafe-number guest projection or retain exact text.
}

#[test]
fn null_structured_content_is_explicitly_absent_and_malformed_types_fail() {
    let result: CallToolResult =
        serde_json::from_value(json!({"content": [], "structuredContent": null})).unwrap();
    assert!(serde_json::to_value(result).unwrap()["structuredContent"].is_null());
    assert!(serde_json::from_value::<CallToolResult>(json!({"isError": "false"})).is_err());
    assert!(serde_json::from_value::<CallToolResult>(json!({"content": {}})).is_err());
}

struct FixtureTransport {
    result: Value,
}

impl RpcTransport for FixtureTransport {
    fn call(&mut self, method: &str, _params: Value) -> davinci_mcp::Result<Value> {
        match method {
            "initialize" => Ok(json!({
                "protocolVersion": "2025-03-26",
                "capabilities": {"tools": {}}
            })),
            "tools/list" => Ok(json!({"tools": [corpus()["tool"].clone()]})),
            "tools/call" => Ok(self.result.clone()),
            _ => Err(davinci_mcp::Error::Protocol(
                "unexpected fixture method".into(),
            )),
        }
    }

    fn notify(&mut self, method: &str, _params: Value) -> davinci_mcp::Result<()> {
        assert_eq!(method, "notifications/initialized");
        Ok(())
    }
}

#[test]
fn external_transport_preserves_same_full_result_as_native_decoder() {
    let wire = corpus()["results"][0]["wire"].clone();
    let mut client =
        Client::connect_transport("fixture", Box::new(FixtureTransport { result: wire })).unwrap();
    assert_eq!(client.tools.len(), 1);
    assert_eq!(
        serde_json::to_value(&client.tools[0]).unwrap()["outputSchema"],
        corpus()["tool"]["outputSchema"]
    );
    let result = client.call_tool("fixture_items", json!({})).unwrap();
    assert_eq!(result.text(), "fixture text");
    assert_eq!(
        serde_json::to_value(result).unwrap()["structuredContent"],
        json!({"items": [{"id": "A"}]})
    );
}
