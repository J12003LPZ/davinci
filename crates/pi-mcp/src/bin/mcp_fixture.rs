//! In-tree stdio MCP server for tests. Speaks newline JSON-RPC on stdin.
//!
//! Flags select a misbehaviour the client must survive:
//! `--log-stdout` prints a plain log line before every reply,
//! `--sampling` sends a notification and a `sampling/createMessage` request
//! (with the caller's own id) before answering `tools/call` and echoes the
//! error code the client refused it with,
//! `--hang` never answers `tools/call` (and says so on stderr),
//! `--die` writes to stderr and exits on `tools/call`,
//! `--rpc-error` answers `tools/call` without `text` with `-32602`,
//! `--paged` splits both lists over two `nextCursor` pages and lists a
//! tool whose name is not an identifier,
//! `--list-error` answers `tools/list` with `-32603`.

use serde_json::{json, Value};
use std::io::{self, BufRead, Write};

struct Flags {
    log_stdout: bool,
    sampling: bool,
    hang: bool,
    die: bool,
    rpc_error: bool,
    paged: bool,
    list_error: bool,
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |flag: &str| args.iter().any(|arg| arg == flag);
    let flags = Flags {
        log_stdout: has("--log-stdout"),
        sampling: has("--sampling"),
        hang: has("--hang"),
        die: has("--die"),
        rpc_error: has("--rpc-error"),
        paged: has("--paged"),
        list_error: has("--list-error"),
    };
    let stdin = io::stdin();
    let mut lines = stdin.lock().lines();
    let mut stdout = io::stdout();
    while let Some(Ok(line)) = lines.next() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(trimmed) else {
            continue;
        };
        // Notifications and the client's replies to our own requests.
        if msg.get("id").is_none() || msg.get("method").is_none() {
            continue;
        }
        let id = msg.get("id").cloned().unwrap_or(Value::Null);
        let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
        if flags.log_stdout {
            let _ = writeln!(stdout, "fixture: handling {method}");
            let _ = stdout.flush();
        }
        let cursor = msg
            .pointer("/params/cursor")
            .and_then(Value::as_str)
            .map(str::to_string);
        let reply = match method {
            "initialize" => ok(
                &id,
                json!({
                    "protocolVersion": "2025-03-26",
                    "capabilities": { "tools": {}, "resources": {} },
                    "serverInfo": { "name": "mcp-fixture", "version": "0" }
                }),
            ),
            "tools/list" if flags.list_error => error(&id, -32603, "list broke"),
            "tools/list" => ok(&id, tools_page(&flags, cursor.as_deref())),
            "tools/call" if flags.hang => {
                eprintln!("hanging on purpose");
                continue;
            }
            "tools/call" if flags.die => {
                eprintln!("fatal: boom");
                std::process::exit(1);
            }
            "tools/call" => {
                let text = msg
                    .pointer("/params/arguments/text")
                    .and_then(Value::as_str);
                match text {
                    None if flags.rpc_error => error(&id, -32602, "text is required"),
                    _ => {
                        let mut text = text.unwrap_or("").to_string();
                        if flags.sampling {
                            let code = sample(&mut stdout, &mut lines, &id);
                            text = format!("{text} refused:{code}");
                        }
                        ok(
                            &id,
                            json!({ "content": [{ "type": "text", "text": text }] }),
                        )
                    }
                }
            }
            "resources/list" => ok(&id, resources_page(&flags, cursor.as_deref())),
            "resources/read" => ok(
                &id,
                json!({ "contents": [{ "uri": "fixture://note", "text": "a note" }] }),
            ),
            _ => ok(&id, json!({})),
        };
        let _ = writeln!(stdout, "{reply}");
        let _ = stdout.flush();
    }
}

fn ok(id: &Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn error(id: &Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

fn echo_tool() -> Value {
    json!({
        "name": "echo",
        "description": "echo text",
        "inputSchema": {
            "type": "object",
            "properties": { "text": { "type": "string" } }
        },
        "annotations": { "readOnlyHint": true }
    })
}

fn tools_page(flags: &Flags, cursor: Option<&str>) -> Value {
    if !flags.paged {
        return json!({ "tools": [echo_tool()] });
    }
    match cursor {
        None => json!({ "tools": [echo_tool()], "nextCursor": "page-2" }),
        Some(_) => json!({
            "tools": [
                { "name": "has space", "inputSchema": { "type": "object" } },
                { "name": "second" }
            ]
        }),
    }
}

fn resources_page(flags: &Flags, cursor: Option<&str>) -> Value {
    let note = json!({ "uri": "fixture://note", "name": "note" });
    if !flags.paged {
        return json!({ "resources": [note] });
    }
    match cursor {
        None => json!({ "resources": [note], "nextCursor": "page-2" }),
        Some(_) => json!({
            "resources": [{ "uri": "fixture://second", "name": "second", "mimeType": "text/plain" }]
        }),
    }
}

/// Send a notification and a server-to-client request that reuses the
/// caller's id, then read the client's reply and return its error code
/// (or `0` if the client answered with a result).
fn sample(stdout: &mut io::Stdout, lines: &mut io::Lines<io::StdinLock<'_>>, id: &Value) -> i64 {
    let note = json!({
        "jsonrpc": "2.0",
        "method": "notifications/message",
        "params": { "level": "info", "data": "about to sample" }
    });
    let request = json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "sampling/createMessage",
        "params": { "messages": [], "maxTokens": 1 }
    });
    let _ = writeln!(stdout, "{note}");
    let _ = writeln!(stdout, "{request}");
    let _ = stdout.flush();
    for line in lines.by_ref() {
        let Ok(line) = line else { break };
        let Ok(reply) = serde_json::from_str::<Value>(line.trim()) else {
            continue;
        };
        if reply.get("method").is_some() || reply.get("id") != Some(id) {
            continue;
        }
        return reply
            .pointer("/error/code")
            .and_then(Value::as_i64)
            .unwrap_or(0);
    }
    -1
}
