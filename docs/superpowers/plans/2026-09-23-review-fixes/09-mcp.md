# Phase 9: MCP Robustness Implementation Plan (`davinci-mcp`)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Real-world MCP servers (ones that log JSON to stdout, ping, keep SSE streams open, or chat a lot) stay connected, and no server name or tool name can break routing or the provider request.

**Architecture:** Changes stay inside `crates/davinci-mcp`. The stdio reader demultiplexes: responses go to the waiting call by id, server requests are answered, everything else is dropped. HTTP parses SSE incrementally. Tool names are routed through a lookup map, not by splitting the string.

**Tech Stack:** Rust 1.83, `ureq` 2.10.1.

Task 2.12 (environment allowlist) is in Phase 2. Independent of other phases.

---

## File map

| File | Tasks |
|---|---|
| `crates/davinci-mcp/src/stdio.rs:176-260` | 9.1, 9.2, 9.4 |
| `crates/davinci-mcp/src/jsonrpc.rs:45-53` | 9.1 |
| `crates/davinci-mcp/src/http.rs:110-125, 210-232, 280-300` | 9.2, 9.3 |
| `crates/davinci-mcp/src/lib.rs:380-400` (names), registry construction | 9.5 |
| `crates/davinci-mcp/src/config.rs` | 9.5 |

---

### Task 9.1: A JSON log line on stdout is skipped, not treated as a malformed reply

**Finding fixed:** davinci-mcp 14. `read_response` (`stdio.rs:212-224`) skips only non-object lines. An object without `method` must decode as a `Response` (`jsonrpc` is required, `jsonrpc.rs:45-53`) or the call fails with `Error::Protocol`, which drops the server (`lib.rs:57-60`). pino (a common Node logger) writes JSON to stdout by default, so a server using it is disconnected on its first log line, contrary to the comment "Servers that log to stdout do not fail the call".

**Files:**
- Modify: `crates/davinci-mcp/src/stdio.rs:212-235`

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn json_log_lines_are_skipped() {
        let lines = [
            r#"{"level":30,"time":1,"msg":"server started"}"#,
            r#"{"jsonrpc":"2.0","id":7,"result":{"ok":true}}"#,
        ];
        let mut transport = StdioTransport::for_test_lines(&lines);
        let result = transport.read_response(&serde_json::json!(7)).unwrap();
        assert_eq!(result, serde_json::json!({"ok": true}));
    }

    #[test]
    fn a_malformed_reply_to_our_id_is_still_an_error() {
        let lines = [r#"{"jsonrpc":"2.0","id":7,"result":1,"error":{"code":1}}"#];
        let mut transport = StdioTransport::for_test_lines(&lines);
        assert!(transport.read_response(&serde_json::json!(7)).is_err());
    }
```

`for_test_lines` builds a transport whose `lines` channel is pre-filled and whose stdin is a sink (`#[cfg(test)]` constructor).

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-mcp --lib json_log_lines_are_skipped`
Expected: FAIL with `Protocol("decode ...")`.

- [ ] **Step 3: Implement**

After the `method` check, only lines that carry **our** id and a `result` or `error` are decoded:

```rust
            if message.get("id") != Some(id) {
                // A log line, a notification, or a reply to another call.
                continue;
            }
            if !message.contains_key("result") && !message.contains_key("error") {
                continue;
            }
            let parsed: Response = serde_json::from_value(Value::Object(message))
                .map_err(|err| Error::Protocol(format!("decode `{trimmed}`: {err}")))?;
```

(Delete the later `if parsed.id != *id { continue; }`, now redundant.)

- [ ] **Step 4: Run and commit**

Run: `cargo test -p davinci-mcp` → PASS.

```bash
git add crates/davinci-mcp/src/stdio.rs
git commit -m "fix(mcp): skip JSON log lines on stdout instead of dropping the server"
```

---

### Task 9.2: `ping` is answered; server requests over HTTP get a reply; a timed-out call is cancelled

**Finding fixed:** davinci-mcp 16. `refuse_request` (`stdio.rs:238-246`) answers every server request, `ping` included, with `-32601`; servers that use ping as a keepalive treat the client as dead. Over HTTP (`http.rs:286-299`) server requests inside an SSE reply are ignored, so a server waiting for our answer to `ping`, sampling or elicitation stalls until the 60 s timeout and is then dropped. On a call timeout, no `notifications/cancelled` is sent, so the server keeps working on it.

**Files:**
- Modify: `crates/davinci-mcp/src/stdio.rs:212-246`, `crates/davinci-mcp/src/http.rs:280-300`

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn ping_is_answered_with_an_empty_result() {
        let lines = [
            r#"{"jsonrpc":"2.0","id":"s1","method":"ping"}"#,
            r#"{"jsonrpc":"2.0","id":7,"result":{}}"#,
        ];
        let mut transport = StdioTransport::for_test_lines(&lines);
        transport.read_response(&serde_json::json!(7)).unwrap();
        let written = transport.written_lines();
        assert!(written.iter().any(|line| line.contains(r#""id":"s1""#) && line.contains(r#""result":{}"#)));
    }

    #[test]
    fn a_timed_out_call_sends_notifications_cancelled() {
        let mut transport = StdioTransport::for_test_lines(&[]);
        transport.set_call_timeout(std::time::Duration::from_millis(50));
        assert!(transport.read_response(&serde_json::json!(9)).is_err());
        assert!(transport.written_lines().iter().any(|line| line.contains("notifications/cancelled") && line.contains("\"requestId\":9")));
    }
```

HTTP test (in `http.rs` tests, using the local test server helper those tests use): a POST whose SSE reply contains `{"jsonrpc":"2.0","id":"s1","method":"ping"}` before our result; assert the client POSTs back `{"jsonrpc":"2.0","id":"s1","result":{}}` and still returns our result.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-mcp`
Expected: the new tests FAIL.

- [ ] **Step 3: Implement**

`stdio.rs`:

```rust
    /// Answer a server-to-client request. `ping` gets `{}` (MCP basic
    /// utilities); this host has no nested model, so sampling, elicitation
    /// and roots are refused.
    fn answer_request(&mut self, id: Value, method: &str) -> Result<()> {
        let reply = if method == "ping" {
            json!({"jsonrpc": "2.0", "id": id, "result": {}})
        } else {
            json!({"jsonrpc": "2.0", "id": id, "error": {"code": -32601, "message": "method not supported"}})
        };
        self.write_line(&reply)
    }
```

Call it in place of `refuse_request(request_id)`, passing `message["method"]`. On the two timeout returns in `read_response`, first send

```rust
                let _ = self.write_line(&json!({
                    "jsonrpc": "2.0",
                    "method": "notifications/cancelled",
                    "params": {"requestId": id, "reason": "timeout"}
                }));
```

`http.rs`: while scanning SSE events, when an event has `method` and `id`, POST the same reply shape back to the endpoint (with the session header), then keep scanning.

- [ ] **Step 4: Run and commit**

Run: `cargo test -p davinci-mcp` → PASS.

```bash
git add crates/davinci-mcp
git commit -m "fix(mcp): answer ping, reply to server requests over HTTP, cancel timed-out calls"
```

---

### Task 9.3: HTTP parses SSE as it arrives and returns when our reply is seen

**Finding (SUSPECTED):** davinci-mcp 17. `http.rs:121` reads the whole SSE body (`read_response_body`) before parsing. The spec says the server SHOULD close the stream after the response; a server that keeps it open makes every call hit the 60 s timeout and then drop the server.

**Files:**
- Modify: `crates/davinci-mcp/src/http.rs:110-125, 210-232, 280-300`

- [ ] **Step 1: Write the failing test**

A local test server that sends headers `content-type: text/event-stream`, then `data: {"jsonrpc":"2.0","id":1,"result":{"ok":true}}\n\n`, then keeps the connection open for 30 s. Assert the call returns the result in under 2 s.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-mcp --lib sse_reply_returns_before_the_stream_closes`
Expected: FAIL (waits until timeout).

- [ ] **Step 3: Implement**

For `text/event-stream` responses, read line by line from `response.into_reader()` through a `BufReader`, assembling events (`data:` lines until a blank line), and handle each event as it completes: answer server requests (Task 9.2), return as soon as the event with our id arrives, and enforce the 16 MB cap on the total bytes read. JSON (non-SSE) responses keep `read_response_body`.

- [ ] **Step 4: Run and commit**

Run: `cargo test -p davinci-mcp` → PASS.

```bash
git add crates/davinci-mcp/src/http.rs
git commit -m "fix(mcp): parse SSE replies incrementally"
```

---

### Task 9.4: A chatty stdio server cannot deadlock the client

**Finding (SUSPECTED):** davinci-mcp 18. `write_all` to the server's stdin (`stdio.rs:176-184`) has no timeout; the stdout queue holds 4 lines (`stdio.rs:251-259`) and is not drained between calls. A server that emits many notifications fills the queue, our reader thread blocks, the server blocks writing stdout and stops reading stdin, and our next large request blocks forever.

**Files:**
- Modify: `crates/davinci-mcp/src/stdio.rs:~70-100 (reader thread), 176-184, 251-259`

- [ ] **Step 1: Write the failing test**

Use the MCP fixture server binary (`src/bin`) with a mode that emits 10,000 notifications after `initialize` and then echoes; send a 2 MB request after the burst and assert it completes within 5 s.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-mcp --test <fixture test> chatty_server`
Expected: hang (run under `timeout 60`).

- [ ] **Step 3: Implement**

Make the reader thread the demultiplexer: it parses each line and forwards only lines that carry an `id` (responses, and server requests which need an answer); notifications and log lines are dropped in the reader, so they never occupy the bounded queue. Server requests are answered from the reader thread through a shared, mutex-protected stdin writer (so a request that arrives while no call is waiting is still answered). Keep `MAX_QUEUED_STDOUT_LINES` for the forwarded lines.

- [ ] **Step 4: Run and commit**

Run: `cargo test -p davinci-mcp` → PASS.

```bash
git add crates/davinci-mcp
git commit -m "fix(mcp): drop notifications in the reader so chatty servers cannot deadlock"
```

---

### Task 9.5: Server and tool names route through a map and fit provider limits

**Finding fixed:** davinci-mcp 19. `is_ident` (`lib.rs:386-391`) accepts server names like `my__srv` or `a_`; `split_agent_tool_name` (`lib.rs:393-401`) then splits at the first `__` and reports "no MCP server named `my`". There is no 64-character check: a `mcp__<server>__<tool>` name over 64 characters makes OpenAI and Anthropic reject the whole request, so every turn fails while that server is connected.

**Files:**
- Modify: `crates/davinci-mcp/src/lib.rs:380-401` and the registry (where agent tool names are built and resolved)
- Modify: `crates/davinci-mcp/src/config.rs` (validate server names at load)

**Interfaces:**
- Produces: `pub fn agent_tool_name(server: &str, tool: &str) -> String` (≤ 64 chars), `McpRegistry::resolve_tool(&self, exposed: &str) -> Option<(&str, &str)>`

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn server_names_with_double_underscore_are_rejected() {
        assert!(validate_server_name("my__srv").is_err());
        assert!(validate_server_name("a_").is_err());
        assert!(validate_server_name("_a").is_err());
        assert!(validate_server_name("github").is_ok());
    }

    #[test]
    fn long_tool_names_are_shortened_deterministically() {
        let long_tool = "t".repeat(80);
        let name = agent_tool_name("server", &long_tool);
        assert!(name.len() <= 64, "{name}");
        assert_eq!(name, agent_tool_name("server", &long_tool));
        assert!(name.starts_with("mcp__server__"));
    }

    #[test]
    fn exposed_names_resolve_through_the_registry() {
        let registry = registry_with_tools(&[("srv", "list_files"), ("srv", &"x".repeat(80))]);
        assert_eq!(registry.resolve_tool("mcp__srv__list_files"), Some(("srv", "list_files")));
        let long = agent_tool_name("srv", &"x".repeat(80));
        assert_eq!(registry.resolve_tool(&long).map(|(_, t)| t.len()), Some(80));
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-mcp`
Expected: FAIL to compile.

- [ ] **Step 3: Implement**

```rust
const MAX_TOOL_NAME: usize = 64;

pub fn validate_server_name(name: &str) -> Result<(), String> {
    if !is_ident(name) || name.contains("__") || name.starts_with('_') || name.ends_with('_') {
        return Err(format!(
            "MCP server name `{name}` must use letters, digits, `-` and single `_`, not start or end with `_`"
        ));
    }
    Ok(())
}

/// `mcp__<server>__<tool>`, shortened to 64 characters with a stable hash
/// suffix when needed (OpenAI and Anthropic reject longer function names).
pub fn agent_tool_name(server: &str, tool: &str) -> String {
    let full = format!("mcp__{server}__{tool}");
    if full.len() <= MAX_TOOL_NAME {
        return full;
    }
    use sha2::{Digest, Sha256};
    let digest = format!("{:x}", Sha256::digest(full.as_bytes()));
    let keep = MAX_TOOL_NAME - 9; // "_" + 8 hex chars
    format!("{}_{}", &full[..keep], &digest[..8])
}
```

(All characters are ASCII after `is_ident`, so byte slicing is safe. Add `sha2.workspace = true` to `davinci-mcp` if it is not a dependency.) The registry keeps `HashMap<String, (String, String)>` from exposed name to `(server, tool)`, filled when tools are listed; every caller of `split_agent_tool_name` uses `resolve_tool` instead. Config load skips a server whose name fails validation and reports the reason in `/mcp`.

- [ ] **Step 4: Run and commit**

Run: `cargo test -p davinci-mcp -p davinci-agent -p davinci-coding-agent` → PASS.

```bash
git add crates/davinci-mcp crates/davinci-agent crates/davinci-coding-agent Cargo.lock
git commit -m "fix(mcp): validate server names, cap tool names at 64 chars, route through a map"
```

---

## Phase 9 exit check

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p davinci-mcp -p davinci-agent -p davinci-coding-agent
```

Then `00-index.md` → "Manual verification: MCP".
