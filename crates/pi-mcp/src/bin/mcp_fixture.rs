//! In-tree stdio MCP server for tests. Speaks newline JSON-RPC on stdin.

use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

fn main() {
    let stdin = io::stdin();
    let mut stdout = io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        if msg.get("id").is_none() {
            continue;
        }
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        let result = match method {
            "initialize" => json!({
                "protocolVersion": "2025-03-26",
                "capabilities": { "tools": {}, "resources": {} },
                "serverInfo": { "name": "mcp-fixture", "version": "0" }
            }),
            "tools/list" => json!({
                "tools": [{
                    "name": "echo",
                    "description": "echo text",
                    "inputSchema": {
                        "type": "object",
                        "properties": { "text": { "type": "string" } }
                    },
                    "annotations": { "readOnlyHint": true }
                }]
            }),
            "tools/call" => {
                let text = msg
                    .pointer("/params/arguments/text")
                    .and_then(Value::as_str)
                    .unwrap_or("");
                json!({ "content": [{ "type": "text", "text": text }] })
            }
            "resources/list" => json!({
                "resources": [{ "uri": "fixture://note", "name": "note" }]
            }),
            "resources/read" => json!({
                "contents": [{ "uri": "fixture://note", "text": "a note" }]
            }),
            _ => json!({}),
        };
        let reply = json!({ "jsonrpc": "2.0", "id": id, "result": result });
        let _ = writeln!(stdout, "{reply}");
        let _ = stdout.flush();
    }
}
