use davinci_mcp::{parse_http_body, Client, Error, RpcTransport};
use serde_json::{json, Value};

struct ResourcesOnly;

impl RpcTransport for ResourcesOnly {
    fn call(&mut self, method: &str, _params: Value) -> davinci_mcp::Result<Value> {
        match method {
            "initialize" => Ok(json!({
                "protocolVersion": "2025-03-26",
                "capabilities": {"resources": {}},
                "serverInfo": {"name": "resource-fixture", "version": "1"}
            })),
            "resources/list" => Ok(json!({
                "resources": [{"uri": "fixture://document", "name": "document"}]
            })),
            "resources/read" => Ok(json!({
                "contents": [{"uri": "fixture://document", "text": "fixture document"}]
            })),
            _ => Err(Error::Rpc {
                code: -32601,
                message: format!("unsupported method: {method}"),
            }),
        }
    }

    fn notify(&mut self, method: &str, _params: Value) -> davinci_mcp::Result<()> {
        assert_eq!(method, "notifications/initialized");
        Ok(())
    }
}

#[test]
fn audit_resources_only_server_connects_without_tools_capability() {
    let connected = Client::connect_transport("resources", Box::new(ResourcesOnly));
    let mut client = match connected {
        Ok(client) => client,
        Err(error) => panic!("resource-only server was rejected: {error}"),
    };
    assert!(client.tools.is_empty());
    assert_eq!(client.resources.len(), 1);
    assert_eq!(
        client.read_resource("fixture://document").unwrap(),
        "fixture document"
    );
}

#[test]
fn audit_json_batch_preserves_correlated_response() {
    let response = json!({"jsonrpc": "2.0", "id": 7, "result": {"ok": true}});
    let batch = json!([
        {"jsonrpc": "2.0", "method": "notifications/progress", "params": {}},
        response.clone()
    ]);
    let parsed = parse_http_body("application/json", &batch.to_string(), Some(&json!(7)));
    assert_eq!(parsed.expect("valid 2025-03-26 JSON-RPC batch"), response);
}

#[test]
fn audit_sse_batch_preserves_correlated_response() {
    let response = json!({"jsonrpc": "2.0", "id": 7, "result": {"ok": true}});
    let body = format!("data: {}\n\n", json!([response.clone()]));
    let parsed = parse_http_body("text/event-stream", &body, Some(&json!(7)));
    assert_eq!(parsed.expect("valid 2025-03-26 SSE batch"), response);
}
