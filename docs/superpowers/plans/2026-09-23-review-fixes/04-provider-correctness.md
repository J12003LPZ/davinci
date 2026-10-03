# Phase 4: Provider Correctness Implementation Plan (`davinci-ai`)

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Provider requests either complete correctly or fail with an error that says what happened: no silent half-answers, no hangs, no pointless retries, no lost tool calls, no rejected tool loops.

**Architecture:** One shared `ureq::Agent` with connect and idle timeouts replaces per-request `ureq::post`. Decoders mark a stream that ends without its terminal event as an error. Retry classification uses word boundaries and refuses context-overflow errors. Thinking signatures travel from the decoder through `ChatMessage` back into the next request.

**Tech Stack:** Rust 1.83, `ureq` 2.10.1 (pinned), `regex`.

Independent of Phases 2-3 except Task 4.1, which depends on the decision recorded in `00-index.md` (D1).

---

## File map

| File | Tasks |
|---|---|
| `crates/davinci-ai/src/stream.rs:74-87` (`ContentBlock`), `:463-485` (`assistant_to_chat`), `:829-836`, `:915-942`, `:1753-1810` (`anthropic_body`), `:1382` | 4.1, 4.2, 4.4 |
| `crates/davinci-ai/src/lib.rs:176-196` (`MessageContent`) | 4.2 |
| `crates/davinci-ai/src/stream_decoder_anthropic.rs:160-185, 262-272, 468-484` | 4.2, 4.3 |
| `crates/davinci-ai/src/stream_decoder.rs:873-889` | 4.3 |
| `crates/davinci-ai/src/stream_decoder_completions.rs:231-243, 451-467, 519-524` | 4.3, 4.6, 4.7 |
| `crates/davinci-ai/src/http.rs` (create) | 4.4 |
| `crates/davinci-ai/src/auth.rs:225, 650`, `oauth_providers.rs:425`, `catalog.rs:328`, `images.rs:155`, `stream.rs:1318` | 4.4 |
| `crates/davinci-ai/src/retry.rs:9-75` | 4.5 |
| `crates/davinci-ai/src/provider_retry.rs:50-62` | 4.5 |
| `crates/davinci-agent/src/turn.rs:1306-1320` (`prepare_tool_call_with_origin`) | 4.7 |
| `crates/davinci-ai/src/codex_ws.rs:85-93, 555-560, 838-875` | 4.8 |
| `crates/davinci-ai/src/stream.rs:1434-1442, 1486, 1507-1520, 1835, 1897-1905` | 4.9 |
| `crates/davinci-ai/src/stream.rs:1949-1955` | 4.10 |

---

### Task 4.1 (DECISION D1): Anthropic OAuth login stops sending the token as an API key; subscription login is not presented as supported

**Finding:** davinci-ai 1. For an OAuth credential, `ResolvedAuth` carries both `Authorization: Bearer <tok>` and `api_key = <tok>`; `collect_request_headers` then also sets `x-api-key: <oauth token>` (`stream.rs:829-836`). Every request after `/login anthropic` fails with 401.

**What this plan does not do, and why:** the TypeScript vendor code (`vendor/davinci/packages/ai/src/api/anthropic-messages.ts:76-106, 873-945, 1009-1016`) makes OAuth work by impersonating Claude Code: a `claude-cli/<version>` user agent, `x-app: cli`, the `claude-code-20250219` beta, the system line "You are Claude Code, Anthropic's official CLI for Claude.", and renaming tools to Claude Code's names ("stealth mode"). Its login also uses Claude Code's OAuth client id. That exists to get a Claude subscription token accepted by a client it was not issued for. This plan does not port it. Copying another product's identity to get past a provider's access check is not a fix this plan will specify.

**What the task does (recommended):**
1. Stop the header conflict: an OAuth credential never produces `x-api-key`.
2. Make `/login anthropic` say plainly that Anthropic login in davinci uses an API key (`/login anthropic sk-ant-...`), instead of starting a browser OAuth flow whose tokens are then rejected.
3. An existing stored Anthropic OAuth credential produces a clear error on first use ("Anthropic OAuth credentials are not supported; run `/login anthropic <api-key>`") instead of a raw 401.

If Julien decides otherwise under D1 (for example a separately registered davinci OAuth client from Anthropic, if one is ever issued), only item 2 changes.

**Files:**
- Modify: `crates/davinci-ai/src/stream.rs:821-836`
- Modify: `crates/davinci-ai/src/auth.rs:392-399` (where OAuth credentials become `ResolvedAuth`)
- Modify: `crates/davinci-ai/src/oauth_providers.rs:132-155` (`authorize_request`, `"anthropic"` arm)
- Modify: `crates/davinci-coding-agent/src/main.rs:6701-6740` (login flow message)

- [ ] **Step 1: Write the failing tests**

`stream.rs` tests:

```rust
    #[test]
    fn oauth_bearer_credentials_never_become_x_api_key() {
        let model = load_builtin_models()
            .into_iter()
            .find(|m| m.api == "anthropic-messages")
            .unwrap();
        let mut headers = std::collections::HashMap::new();
        headers.insert("Authorization".to_string(), "Bearer sk-ant-oat01-x".to_string());
        let auth = ResolvedAuth {
            api_key: Some("sk-ant-oat01-x".into()),
            headers,
            source: "oauth".into(),
        };
        let sent = collect_request_headers(&model, &auth, &StreamOptions::default());
        assert!(!sent.iter().any(|(name, _)| name.eq_ignore_ascii_case("x-api-key")));
    }
```

`auth.rs` tests:

```rust
    #[test]
    fn stored_anthropic_oauth_credential_is_refused_with_a_clear_message() {
        let dir = tempfile::tempdir().unwrap();
        let mut storage = AuthStorage::open(&dir.path().join("auth.json")).unwrap();
        storage
            .login_oauth("anthropic", "sk-ant-oat01-x".into(), None, Some(u64::MAX))
            .unwrap();
        let err = storage.resolve("anthropic").unwrap_err().to_string();
        assert!(err.contains("/login anthropic <api-key>"), "{err}");
    }
```

`oauth_providers.rs` tests:

```rust
    #[test]
    fn anthropic_has_no_browser_login() {
        let pkce = generate_pkce(b"0123456789abcdef0123456789abcdef");
        assert!(authorize_request("anthropic", &pkce, "s").is_none());
    }
```

Use the real resolve entry point in `auth.rs` (the function containing lines 392-399) in place of `storage.resolve`.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-ai --lib oauth_bearer stored_anthropic_oauth anthropic_has_no_browser_login`
Expected: all three FAIL.

- [ ] **Step 3: Implement**

`stream.rs`, in `collect_request_headers`, compute once whether auth already carries a bearer header, and skip the key headers when it does:

```rust
    let has_bearer = auth
        .headers
        .keys()
        .any(|name| name.eq_ignore_ascii_case("authorization"));
    if let Some(key) = auth.api_key.as_ref().filter(|_| !has_bearer) {
        // ... the Task 2.10 match on model.api, unchanged ...
    }
```

`auth.rs`: in the OAuth branch of credential resolution, before building `ResolvedAuth`, add

```rust
        if provider == "anthropic" {
            return Err(AuthStorageError::Invalid(
                "Anthropic OAuth credentials are not supported by davinci; run `/login anthropic <api-key>`".into(),
            ));
        }
```

(use the error variant that surfaces as a user-facing message on this path).

`oauth_providers.rs`: delete the `"anthropic"` arm of `authorize_request` (it then falls through to `_ => None`), and remove Anthropic from `refresh_oauth_token`/`token_refresh_request`. Remove the Anthropic `CallbackProvider` variant only if nothing else uses it (`rg -n "CallbackProvider::Anthropic"`); otherwise leave it for Task 11.4 dead-code removal.

`main.rs`: where `fresh_authorize_request(provider)` returns `None` and the provider is `anthropic`, print `Anthropic login uses an API key: /login anthropic sk-ant-...` and return `Ok(false)`.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p davinci-ai` and `cargo test -p davinci-coding-agent --bin davinci login`
Expected: PASS. Update any existing test that exercised the Anthropic browser flow (`oauth_providers.rs:591`, `oauth_callback.rs` Anthropic tests) to use `openai-codex`, or delete it if it only covered the removed arm.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-ai crates/davinci-coding-agent/src/main.rs
git commit -m "fix(anthropic): never send OAuth tokens as x-api-key; require API keys for Anthropic"
```

---

### Task 4.2: Thinking signatures and redacted thinking survive the round trip

**Finding fixed:** davinci-ai 2. The decoder drops `signature_delta` (`stream_decoder_anthropic.rs:269-271`) and the payload of `redacted_thinking` (`:171-184`); `anthropic_body` drops `Thinking` blocks from assistant tool-use turns (`stream.rs:1785-1800`, `_ => None`). With extended thinking on, the second request of every tool loop is rejected with 400, because the final assistant turn must start with its signed thinking block.

**Files:**
- Modify: `crates/davinci-ai/src/stream.rs:78-80` (`ContentBlock::Thinking`), `:471-474` (`assistant_to_chat`), `:1382`, `:1785-1800`
- Modify: `crates/davinci-ai/src/lib.rs:180-184` (`MessageContent::Thinking`)
- Modify: `crates/davinci-ai/src/stream_decoder_anthropic.rs:161-184, 269-271`
- Modify: every `ContentBlock::Thinking { thinking }` pattern/constructor (`rg -n "ContentBlock::Thinking \{" crates`; about 20 sites) to add `..` in patterns and the new fields in constructors

**Interfaces:**
- Produces:
  - `ContentBlock::Thinking { thinking: String, signature: Option<String>, redacted: bool }`
  - `MessageContent::Thinking { thinking: String, redacted: Option<bool>, signature: Option<String> }`
  - Both new fields `#[serde(default, skip_serializing_if = ...)]`, so old sessions still load.

- [ ] **Step 1: Write the failing tests**

`stream_decoder_anthropic.rs` tests:

```rust
    #[test]
    fn thinking_signature_and_redacted_payload_are_kept() {
        let frames = [
            r#"{"type":"message_start","message":{"id":"m","model":"claude","usage":{}}}"#,
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"thinking","thinking":""}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"plan"}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"signature_delta","signature":"SIG"}}"#,
            r#"{"type":"content_block_stop","index":0}"#,
            r#"{"type":"content_block_start","index":1,"content_block":{"type":"redacted_thinking","data":"OPAQUE"}}"#,
            r#"{"type":"content_block_stop","index":1}"#,
            r#"{"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{}}"#,
            r#"{"type":"message_stop"}"#,
        ];
        let message = decode_frames(&frames);
        assert_eq!(
            message.content[0],
            ContentBlock::Thinking { thinking: "plan".into(), signature: Some("SIG".into()), redacted: false }
        );
        assert_eq!(
            message.content[1],
            ContentBlock::Thinking { thinking: String::new(), signature: Some("OPAQUE".into()), redacted: true }
        );
    }
```

`decode_frames` is the helper the existing tests in that file use to feed frames (search for how they drive the decoder and reuse it).

`stream.rs` tests:

```rust
    #[test]
    fn anthropic_body_replays_signed_thinking_first_in_tool_turns() {
        let model = load_builtin_models().into_iter().find(|m| m.api == "anthropic-messages").unwrap();
        let assistant = ChatMessage {
            role: "assistant".into(),
            content: vec![
                MessageContent::Thinking { thinking: "plan".into(), redacted: None, signature: Some("SIG".into()) },
                MessageContent::Thinking { thinking: String::new(), redacted: Some(true), signature: Some("OPAQUE".into()) },
                MessageContent::Thinking { thinking: "unsigned".into(), redacted: None, signature: None },
                MessageContent::ToolCall { id: "t1".into(), name: "read".into(), arguments: serde_json::json!({"path":"a"}) },
            ],
            ..ChatMessage::default()
        };
        let body = anthropic_body(&model, &[assistant], None, &[], &StreamOptions::default());
        let content = body["messages"][0]["content"].as_array().unwrap();
        assert_eq!(content[0], serde_json::json!({"type":"thinking","thinking":"plan","signature":"SIG"}));
        assert_eq!(content[1], serde_json::json!({"type":"redacted_thinking","data":"OPAQUE"}));
        assert_eq!(content[2]["type"], "tool_use");
        assert_eq!(content.len(), 3);
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-ai --lib thinking_signature anthropic_body_replays`
Expected: FAIL to compile (no `signature` field).

- [ ] **Step 3: Implement the types**

`stream.rs`:

```rust
    Thinking {
        thinking: String,
        /// Anthropic's signature (or, when `redacted`, the opaque `data`).
        /// Required to replay the block in the next request of a tool loop.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        redacted: bool,
    },
```

`lib.rs` `MessageContent::Thinking`: add

```rust
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
```

Fix every pattern (`ContentBlock::Thinking { thinking }` → `ContentBlock::Thinking { thinking, .. }`) and constructor (add `signature: None, redacted: false` / `signature: None`). `cargo build --workspace --all-targets` lists every site.

`assistant_to_chat`:

```rust
                ContentBlock::Thinking { thinking, signature, redacted } => MessageContent::Thinking {
                    thinking: thinking.clone(),
                    redacted: redacted.then_some(true),
                    signature: signature.clone(),
                },
```

- [ ] **Step 4: Implement the decoder**

`"thinking"` start: push `ContentBlock::Thinking { thinking: string_field(content_block, "thinking"), signature: None, redacted: false }`.

`"redacted_thinking"` start: push `ContentBlock::Thinking { thinking: String::new(), signature: Some(string_field(content_block, "data")), redacted: true }` and delete the comment about dropping it.

`signature_delta`:

```rust
            ("signature_delta", _) => {
                let piece = delta.get("signature").and_then(Value::as_str).unwrap_or("");
                if let Some(ContentBlock::Thinking { signature, .. }) =
                    self.message.content.get_mut(block.content_index)
                {
                    signature.get_or_insert_with(String::new).push_str(piece);
                }
            }
```

(match the local variable names in that `match`; `delta` is the delta object and `block` the open block).

- [ ] **Step 5: Implement the replay**

In `anthropic_body`'s assistant-with-tool-calls branch, build thinking blocks first, then the rest:

```rust
                let mut content: Vec<Value> = message
                    .content
                    .iter()
                    .filter_map(|block| match block {
                        // Unsigned thinking (another provider, or an old
                        // session) cannot be replayed; the API would reject it.
                        MessageContent::Thinking { signature: Some(signature), redacted, thinking } => {
                            Some(if redacted.unwrap_or(false) {
                                serde_json::json!({"type": "redacted_thinking", "data": signature})
                            } else {
                                serde_json::json!({"type": "thinking", "thinking": thinking, "signature": signature})
                            })
                        }
                        _ => None,
                    })
                    .collect();
                content.extend(message.content.iter().filter_map(|block| match block {
                    MessageContent::Text { text } if !text.is_empty() => {
                        Some(serde_json::json!({"type":"text","text": text}))
                    }
                    MessageContent::ToolCall { id, name, arguments } => Some(serde_json::json!({
                        "type": "tool_use", "id": id, "name": name, "input": arguments,
                    })),
                    _ => None,
                }));
```

- [ ] **Step 6: Run to verify pass**

Run: `cargo test --workspace`
Expected: PASS. (`davinci-agent/tests/context_vm_active.rs:111` and `context_vm_replay.rs:126` construct `MessageContent::Thinking`; add `signature: None` there.)

- [ ] **Step 7: Commit**

```bash
git add crates
git commit -m "fix(anthropic): keep thinking signatures and replay them in tool loops"
```

---

### Task 4.3: A stream that ends without its terminal event is an error, with the partial text kept

**Finding fixed:** davinci-ai 3. A clean EOF before `response.completed` (Responses, `stream_decoder.rs:873-889`), `message_stop` (Anthropic, `stream_decoder_anthropic.rs:468-483`) or `finish_reason` (Completions, `stream_decoder_completions.rs:451-467`) becomes `Stop` unless a tool call was open. A proxy that closes mid-answer produces a half-answer stored as complete; the Responses ledger records it (`stream.rs:436-441` only skips Error/Aborted). The comments say this is deliberate; it is unsafe.

**Behavior:** keep the received content, set `stop_reason = Error`, `error_message = "stream ended before a terminal response event"`. That text is already in the retry patterns (`retry.rs:~65`), so auto-retry can recover, and the Responses ledger skips it. Completions providers flagged by `expects_finish_reason() == false` keep today's behavior (they never send one).

**Files:**
- Modify: `crates/davinci-ai/src/stream_decoder.rs:873-889`
- Modify: `crates/davinci-ai/src/stream_decoder_anthropic.rs:468-484`
- Modify: `crates/davinci-ai/src/stream_decoder_completions.rs:451-467`

- [ ] **Step 1: Write the failing tests**

One per decoder, same shape. Anthropic:

```rust
    #[test]
    fn eof_before_message_stop_is_an_error_that_keeps_text() {
        let frames = [
            r#"{"type":"message_start","message":{"id":"m","model":"claude","usage":{}}}"#,
            r#"{"type":"content_block_start","index":0,"content_block":{"type":"text","text":""}}"#,
            r#"{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"half an ans"}}"#,
        ];
        let message = decode_frames(&frames);
        assert_eq!(message.stop_reason, Some(StopReason::Error));
        assert_eq!(message.error_message.as_deref(), Some("stream ended before a terminal response event"));
        assert!(matches!(&message.content[0], ContentBlock::Text { text } if text == "half an ans"));
        assert!(crate::is_retryable_assistant_error(&message));
    }
```

Responses (`stream_decoder.rs`): frames `response.created`, `response.output_item.added` (message), `response.output_text.delta` with `"half"`, then EOF. Completions: one chunk with `choices[0].delta.content = "half"` and no `finish_reason`, on a model where `expects_finish_reason()` is true; plus one test that a model with `expects_finish_reason() == false` still ends `Stop`. Build frames with the helpers each test module already uses.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-ai --lib eof_before`
Expected: FAIL (`Stop`, not `Error`).

- [ ] **Step 3: Implement**

Add to `stream_decoder.rs` (shared by all three):

```rust
/// Recorded when the connection closes before the provider's terminal
/// event. The received content is kept, but the turn is not complete. The
/// wording is matched by `retry.rs`.
pub(crate) const TRUNCATED_STREAM: &str = "stream ended before a terminal response event";
```

Responses `finish`:

```rust
    fn finish(&mut self, out: &mut Vec<AssistantMessageEvent>) -> AssistantMessage {
        if !self.done {
            let cut_tool_call = self.slots.values().any(|slot| slot.kind == SlotKind::ToolCall);
            if cut_tool_call {
                self.fail("Stream ended before the tool call was complete".into(), out);
            } else {
                self.fail(TRUNCATED_STREAM.into(), out);
            }
        }
        self.message.clone()
    }
```

Confirm `fail` keeps already-received content (it should only set stop reason and error message and emit the error event); if it clears content, add a `fail_keeping_content` variant used here.

Anthropic `finish`: same change (`self.finalize(out)` → `self.fail(TRUNCATED_STREAM.into(), out)`).

Completions `finish`: in the `!self.finish_reason_seen` branch:

```rust
                if !self.expects_finish_reason() {
                    self.message.stop_reason = Some(StopReason::Stop);
                } else if !self.tool_calls.is_empty() {
                    self.message.stop_reason = Some(StopReason::Error);
                    self.message.error_message =
                        Some("Stream ended before the tool call was complete".into());
                } else {
                    self.message.stop_reason = Some(StopReason::Error);
                    self.message.error_message = Some(TRUNCATED_STREAM.into());
                }
```

Replace the three "deliberate" comments with: `// The connection closed before the terminal event: keep what arrived, but the turn did not finish.`

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p davinci-ai`
Expected: PASS. Replay fixtures under `crates/davinci-ai/tests` or `fixtures/` that end without a terminal event now produce `Error`; for each, check whether the fixture is truncated by accident (fix the fixture by adding the terminal event) or on purpose (update the expected stop reason).

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-ai
git commit -m "fix(ai): treat streams that end without a terminal event as errors"
```

---

### Task 4.4: One HTTP agent with connect and idle timeouts for every provider call

**Finding fixed:** davinci-ai 4. `send_provider_request` uses `ureq::post(url)`: ureq's default agent has a 30 s connect timeout and **no read timeout**, and `provider_timeout_ms` defaults to `None` (`davinci-agent/src/lib.rs:489`). A half-open TCP connection hangs print/headless mode forever. Token exchange and refresh (`oauth_providers.rs:425`, `auth.rs:225`) have no timeout at all. When `timeout_ms` is set, `Request::timeout` is an overall deadline that also covers the streamed body, so a long answer is cut off and then retried.

**Behavior:** one `ureq::Agent` per distinct idle timeout, built once. `timeout_connect` 30 s, `timeout_write` 60 s, `timeout_read` = idle timeout (default 300 s, `options.timeout_ms` when set). `timeout_read` in ureq 2.x applies to each socket read, so it is an idle timeout, not an overall deadline. Non-streaming calls (token exchange, refresh, catalog) use a 60 s idle timeout.

**Files:**
- Create: `crates/davinci-ai/src/http.rs`
- Modify: `crates/davinci-ai/src/lib.rs` (`mod http;`)
- Modify: `stream.rs:915-942, 1318`, `auth.rs:225, 650`, `oauth_providers.rs:425`, `catalog.rs:328`, `images.rs:155`

**Interfaces:**
- Produces: `pub(crate) fn agent(idle: std::time::Duration) -> ureq::Agent`, `pub(crate) const PROVIDER_IDLE_TIMEOUT: Duration` (300 s), `pub(crate) const CONTROL_IDLE_TIMEOUT: Duration` (60 s)

- [ ] **Step 1: Write the failing test**

`http.rs` tests (a server that accepts, reads the request, then stays silent):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::time::{Duration, Instant};

    #[test]
    fn a_silent_server_hits_the_idle_timeout() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut buf = [0u8; 1024];
            let _ = stream.read(&mut buf);
            std::thread::sleep(Duration::from_secs(5));
        });
        let started = Instant::now();
        let result = agent(Duration::from_millis(300))
            .post(&format!("http://{addr}/v1/x"))
            .send_string("{}");
        assert!(result.is_err());
        assert!(started.elapsed() < Duration::from_secs(3));
        server.join().unwrap();
    }

    #[test]
    fn agents_are_reused_per_timeout() {
        let a = agent(Duration::from_secs(7));
        let b = agent(Duration::from_secs(7));
        // ureq::Agent is an Arc inside; clones of one agent share a pool.
        assert_eq!(format!("{a:?}"), format!("{b:?}"));
    }
}
```

If `ureq::Agent`'s `Debug` output does not distinguish instances, drop the second test; the first is the one that matters.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-ai --lib http::`
Expected: FAIL to compile.

- [ ] **Step 3: Implement**

```rust
//! The HTTP client every provider call uses. `ureq::post` builds a fresh
//! agent with no read timeout for each request; this module keeps one agent
//! per idle timeout so connections are pooled and a stalled server cannot
//! hang a turn.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

pub(crate) const PROVIDER_IDLE_TIMEOUT: Duration = Duration::from_secs(300);
pub(crate) const CONTROL_IDLE_TIMEOUT: Duration = Duration::from_secs(60);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(30);
const WRITE_TIMEOUT: Duration = Duration::from_secs(60);

/// `idle` bounds each socket read (ureq's `timeout_read`), so a long stream
/// that keeps producing bytes is never cut off, and a silent one is.
pub(crate) fn agent(idle: Duration) -> ureq::Agent {
    static AGENTS: OnceLock<Mutex<HashMap<u128, ureq::Agent>>> = OnceLock::new();
    let agents = AGENTS.get_or_init(Default::default);
    let mut agents = agents.lock().unwrap_or_else(|err| err.into_inner());
    agents
        .entry(idle.as_millis())
        .or_insert_with(|| {
            ureq::AgentBuilder::new()
                .timeout_connect(CONNECT_TIMEOUT)
                .timeout_read(idle)
                .timeout_write(WRITE_TIMEOUT)
                .build()
        })
        .clone()
}
```

`send_provider_request`:

```rust
    let idle = timeout_ms
        .map(std::time::Duration::from_millis)
        .unwrap_or(crate::http::PROVIDER_IDLE_TIMEOUT);
    let mut request = crate::http::agent(idle).post(url);
```

and delete the `request.timeout(...)` block. Replace `ureq::post(&url)` / `ureq::get(&url)` at the other listed sites with `crate::http::agent(crate::http::CONTROL_IDLE_TIMEOUT).post(&url)` / `.get(&url)`. The Codex WebSocket path is separate (Task 4.8).

Document the setting: `provider_timeout_ms` now means "idle timeout per read" in the settings docs/README where it is described (`rg -n "providerTimeout|provider_timeout" docs crates/davinci-coding-agent/src/settings.rs`).

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p davinci-ai`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-ai
git commit -m "fix(ai): pooled HTTP agent with connect and idle timeouts for every provider call"
```

---

### Task 4.5: Retry classification: status codes match as words, context overflow is never retried, no retry after the request reached the server

**Findings fixed:** davinci-ai 7 and 8.
- `retry.rs:36-41` matches `"429"`, `"500"`, … anywhere, so `prompt is too long: 205000 tokens` is "retryable". Context overflow can never succeed on retry; it burns attempts and time.
- `is_retryable_provider_error` (`provider_retry.rs:56-57`) retries every error with no status, including a read timeout after the model started generating, which bills a second generation.

**Files:**
- Modify: `crates/davinci-ai/src/retry.rs:9-75`
- Modify: `crates/davinci-ai/src/provider_retry.rs` (`ProviderError` gains `phase`; `provider_error_from_ureq` sets it)

**Interfaces:**
- Produces: `pub enum RequestPhase { Connect, Response }` and `ProviderError.phase: RequestPhase` (default `Response`)

- [ ] **Step 1: Write the failing tests**

`retry.rs` tests:

```rust
    #[test]
    fn numbers_inside_other_numbers_are_not_status_codes() {
        assert!(!is_retryable_error_text("prompt is too long: 205000 tokens > 200000 maximum"));
        assert!(!is_retryable_error_text("input has 135000 tokens"));
        assert!(is_retryable_error_text("HTTP 500 Internal Server Error"));
        assert!(is_retryable_error_text("status=429"));
    }

    #[test]
    fn context_overflow_is_never_retryable() {
        for text in [
            "prompt is too long: 205000 tokens > 200000 maximum",
            "This model's maximum context length is 128000 tokens",
            "context_length_exceeded",
            "input exceeds the context window",
            "Request too large: 500 server error while counting; prompt is too long",
        ] {
            assert!(!is_retryable_error_text(text), "{text}");
        }
    }
```

`provider_retry.rs` tests:

```rust
    #[test]
    fn only_connect_phase_errors_without_status_are_retried() {
        let connect = ProviderError::new(None, "connection refused").with_phase(RequestPhase::Connect);
        let midway = ProviderError::new(None, "timed out reading response");
        assert!(is_retryable_provider_error(&connect));
        assert!(!is_retryable_provider_error(&midway));
        assert!(is_retryable_provider_error(&ProviderError::new(Some(503), "unavailable")));
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-ai --lib retry`
Expected: FAIL.

- [ ] **Step 3: Implement `retry.rs`**

In `retryable_provider_pattern`, replace `"429", "500", "502", "503", "504", "524",` with the single entry `r"\b(?:429|5\d\d)\b",`.

Add to `non_retryable_limit_pattern`'s list:

```rust
            // Context overflow: retrying the same request cannot succeed.
            "prompt is too long",
            "context.?length.?exceeded",
            "maximum context length",
            "exceeds the context window",
            "input is too long",
            "too many (input )?tokens",
            "request too large",
```

Both predicates already check the non-retryable pattern first, so overflow wins even when "500" also appears.

- [ ] **Step 4: Implement the phase**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RequestPhase {
    /// DNS, TCP connect, TLS: the request never reached the provider.
    Connect,
    /// Anything after the request may have been received.
    #[default]
    Response,
}
```

Add `pub phase: RequestPhase` to `ProviderError` (initialised to `Response` in `new`) and

```rust
    pub fn with_phase(mut self, phase: RequestPhase) -> Self {
        self.phase = phase;
        self
    }
```

In `is_retryable_provider_error`:

```rust
    match error.status {
        None => error.phase == RequestPhase::Connect,
        Some(408 | 409 | 429) => true,
        Some(status) => status >= 500,
    }
```

In `provider_error_from_ureq`, for `ureq::Error::Transport(transport)`:

```rust
            let phase = match transport.kind() {
                // The request never left this machine or never reached the host.
                ureq::ErrorKind::Dns
                | ureq::ErrorKind::ConnectionFailed
                | ureq::ErrorKind::ProxyConnect => RequestPhase::Connect,
                // `Io` covers reads after the request was sent (timeouts, resets).
                _ => RequestPhase::Response,
            };
```

and attach it with `.with_phase(phase)`. Check the variant names against ureq 2.10.1's `ErrorKind` (`cargo doc -p ureq --open` or the source in `~/.cargo/registry`); `ProxyConnect` exists only if the `proxy` feature is on, so drop that arm if it does not compile.

- [ ] **Step 5: Run to verify pass**

Run: `cargo test -p davinci-ai`
Expected: PASS. The agent-level retry (`davinci-agent/src/turn.rs:788, 835`) uses the same text predicates and benefits automatically; Task 5.9 fixes its attempt count.

- [ ] **Step 6: Commit**

```bash
git add crates/davinci-ai
git commit -m "fix(ai): word-bounded status matching, never retry context overflow or post-send failures"
```

---

### Task 4.6: Completions tool calls with a shared `index` but different ids stay separate

**Finding fixed:** davinci-ai 9. Slots are matched by `index` before `id` (`stream_decoder_completions.rs:231-243`). Providers that send every parallel call as `index: 0` with different ids (Gemini's OpenAI-compatible endpoint, some vLLM and Ollama builds) get arguments concatenated into `{..}{..}`, which `final_arguments` turns into `{}`, and the second call is lost.

**Files:**
- Modify: `crates/davinci-ai/src/stream_decoder_completions.rs:231-243`

- [ ] **Step 1: Write the failing test**

```rust
    #[test]
    fn same_index_different_ids_are_two_tool_calls() {
        let chunks = [
            r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"a","function":{"name":"read","arguments":"{\"path\":\"x\"}"}}]}}]}"#,
            r#"{"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"b","function":{"name":"read","arguments":"{\"path\":\"y\"}"}}]}}]}"#,
            r#"{"choices":[{"index":0,"delta":{},"finish_reason":"tool_calls"}]}"#,
        ];
        let message = decode_chunks(&chunks);
        let calls: Vec<_> = message
            .content
            .iter()
            .filter_map(|block| match block {
                ContentBlock::ToolCall { id, arguments, .. } => Some((id.clone(), arguments.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0], ("a".into(), serde_json::json!({"path":"x"})));
        assert_eq!(calls[1], ("b".into(), serde_json::json!({"path":"y"})));
    }
```

(`decode_chunks`: the helper the module's tests already use.)

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-ai --lib same_index_different_ids`
Expected: FAIL (one call).

- [ ] **Step 3: Implement**

```rust
        let by_index = stream_index.and_then(|index| {
            self.tool_calls
                .iter()
                .position(|slot| slot.stream_index == Some(index))
        });
        // A fragment that names a different non-empty id than the slot at its
        // index is a new call from a provider that reuses index 0.
        let by_index = by_index.filter(|position| match id {
            Some(id) if !id.is_empty() => {
                let slot_id = self.block_id(self.tool_calls[*position].content_index);
                slot_id.is_empty() || slot_id == id
            }
            _ => true,
        });
        let found = by_index.or_else(|| {
            id.filter(|id| !id.is_empty()).and_then(|id| {
                self.tool_calls
                    .iter()
                    .position(|slot| self.block_id(slot.content_index) == id)
            })
        });
```

When a new slot is created for a reused index, later fragments that carry only `index: 0` and no id belong to the **latest** slot with that index. Make the index lookup pick the last match: replace `.position(...)` in `by_index` with `.rposition(...)`.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p davinci-ai`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-ai/src/stream_decoder_completions.rs
git commit -m "fix(ai): split completions tool calls that reuse an index with new ids"
```

---

### Task 4.7: Unparseable tool arguments become a tool error, never a call with `{}`

**Finding fixed:** davinci-ai 10. `final_arguments` (`stream_decoder_completions.rs:519-524`), `parse_streaming_json` (`stream_decoder.rs:516-535`) and the Anthropic decoder (`:317`) turn invalid JSON, or arguments cut by `finish_reason: length`, into `{}` or the last partial that parsed. The tool then runs with empty or partial arguments.

**Files:**
- Modify: `crates/davinci-ai/src/lib.rs` (add `pub const INVALID_ARGUMENTS_KEY`)
- Modify: the three finalize sites above
- Modify: `crates/davinci-agent/src/turn.rs:1306-1320` (`prepare_tool_call_with_origin`)

**Interfaces:**
- Produces: `davinci_ai::INVALID_ARGUMENTS_KEY: &str = "__davinci_invalid_arguments"`; a finished tool call whose arguments did not parse has `arguments = {"__davinci_invalid_arguments": "<first 2000 chars of the raw buffer>"}`.

- [ ] **Step 1: Write the failing tests**

`stream_decoder_completions.rs`:

```rust
    #[test]
    fn invalid_final_arguments_are_marked_not_emptied() {
        let args = final_arguments("{\"path\": \"x\", ");
        assert!(args.get(crate::INVALID_ARGUMENTS_KEY).is_some());
        assert_eq!(final_arguments(""), serde_json::json!({}));
        assert_eq!(final_arguments("{\"a\":1}"), serde_json::json!({"a":1}));
    }
```

`davinci-agent/src/turn.rs` tests (use the fixture agent the other turn tests use):

```rust
    #[test]
    fn a_call_with_invalid_arguments_is_answered_with_an_error_and_not_run() {
        let agent = fixture_agent();
        let args = serde_json::json!({ davinci_ai::INVALID_ARGUMENTS_KEY: "{\"path\": " });
        match agent.prepare_tool_call(std::path::Path::new("."), "c1", "write", &args, 0) {
            Preparation::Immediate(result) => {
                assert!(result.is_error);
                assert!(result.content.contains("not valid JSON"), "{}", result.content);
            }
            _ => panic!("invalid arguments must not be scheduled"),
        }
    }
```

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-ai --lib invalid_final_arguments` and `cargo test -p davinci-agent --lib a_call_with_invalid_arguments`
Expected: FAIL.

- [ ] **Step 3: Implement**

`lib.rs`:

```rust
/// Set on a finished tool call whose arguments were not valid JSON. The
/// agent answers such a call with an error and never executes it.
pub const INVALID_ARGUMENTS_KEY: &str = "__davinci_invalid_arguments";

pub(crate) fn invalid_arguments(raw: &str) -> serde_json::Value {
    let shown: String = raw.chars().take(2000).collect();
    serde_json::json!({ INVALID_ARGUMENTS_KEY: shown })
}
```

`final_arguments`:

```rust
fn final_arguments(buffer: &str) -> Value {
    if buffer.trim().is_empty() {
        return Value::Object(Map::new());
    }
    match serde_json::from_str::<Value>(buffer) {
        Ok(value @ Value::Object(_)) => value,
        _ => crate::invalid_arguments(buffer),
    }
}
```

Apply the same rule at the Responses and Anthropic finalize points (where a tool call block is closed: `output_item.done` / `content_block_stop`): parse the full buffer; on failure store `crate::invalid_arguments(buffer)`. Streaming partials (`parse_streaming_json` during deltas) keep their current behavior; only the final value changes.

`turn.rs`, at the top of `prepare_tool_call_with_origin` (before `ensure_session_persistence`):

```rust
        if let Some(raw) = args.get(davinci_ai::INVALID_ARGUMENTS_KEY).and_then(Value::as_str) {
            return Preparation::Immediate(crate::ToolResult {
                content: format!(
                    "The arguments for `{name}` were not valid JSON, so the tool was not run. Send the call again with complete JSON arguments. Received: {raw}"
                ),
                is_error: true,
                details: Some(serde_json::json!({"invalidArguments": true})),
            });
        }
```

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p davinci-ai -p davinci-agent`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-ai crates/davinci-agent
git commit -m "fix(ai): refuse tool calls whose final arguments are not valid JSON"
```

---

### Task 4.8: Codex WebSocket: a real idle timeout after the handshake, and fragmented messages reassembled

**Findings fixed:** davinci-ai 12 and 13.
- When no idle timeout is configured, reads keep the 15 s connect timeout (`codex_ws.rs:555-560` vs `:89-93`), so more than 15 s of silent reasoning after `response.created` is a hard error with no SSE fallback (the attempt already started), reported as "connect timeout after 0ms".
- `read_frame` (`:838-875`) ignores the FIN bit and the caller drops continuation frames (opcode 0), so a large `response.completed` split across frames fails with "Invalid Codex WebSocket JSON".

**Files:**
- Modify: `crates/davinci-ai/src/codex_ws.rs:85-93, 757-780, 838-875`

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn fragmented_text_message_is_reassembled() {
        let text = br#"{"type":"response.completed","response":{"id":"r"}}"#;
        let (a, b) = text.split_at(10);
        let mut wire = Vec::new();
        wire.extend(frame(false, OPCODE_TEXT, a));
        wire.extend(frame(false, OPCODE_PING, b"")); // control frames may interleave
        wire.extend(frame(true, 0x0, b));
        let mut stream = WsStream::for_test(wire);
        let (opcode, payload) = read_message(&mut stream, Some(1000)).unwrap();
        assert_eq!(opcode, OPCODE_TEXT);
        assert_eq!(payload, text);
    }

    fn frame(fin: bool, opcode: u8, payload: &[u8]) -> Vec<u8> {
        let mut out = vec![(if fin { 0x80 } else { 0 }) | opcode, payload.len() as u8];
        out.extend_from_slice(payload);
        out
    }
```

`WsStream::for_test(bytes)` needs a test-only in-memory variant of `WsInner`; add `#[cfg(test)] Memory(std::io::Cursor<Vec<u8>>)` to `WsInner` and route `Read`/`Write` for it. If a ping must be answered with a pong during reassembly, give the memory variant a write sink and assert a pong was written.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-ai --lib fragmented_text_message`
Expected: FAIL to compile (`read_message` missing).

- [ ] **Step 3: Implement**

Change `read_frame` to return `(fin: bool, opcode: u8, payload: Vec<u8>)` (`let fin = header[0] & 0x80 != 0;`). Add:

```rust
/// One whole message: data frames joined until FIN. Control frames (ping,
/// pong, close) may arrive between fragments and are handled in place.
fn read_message(stream: &mut WsStream, idle_timeout_ms: Option<u64>) -> Result<(u8, Vec<u8>), String> {
    let mut opcode = None;
    let mut payload = Vec::new();
    loop {
        let (fin, frame_opcode, data) = read_frame(stream, idle_timeout_ms)?;
        match frame_opcode {
            OPCODE_PING => {
                write_frame(stream, OPCODE_PONG, &data)?;
                continue;
            }
            OPCODE_PONG => continue,
            OPCODE_CLOSE => return Ok((OPCODE_CLOSE, data)),
            0x0 if opcode.is_none() => return Err("WebSocket continuation without a start frame".into()),
            0x0 => {}
            start => {
                if opcode.is_some() {
                    return Err("WebSocket data frame inside a fragmented message".into());
                }
                opcode = Some(start);
            }
        }
        if payload.len() + data.len() > 8 * 1024 * 1024 {
            return Err("WebSocket message too big".into());
        }
        payload.extend_from_slice(&data);
        if fin {
            return Ok((opcode.unwrap_or(OPCODE_TEXT), payload));
        }
    }
}
```

Use the module's existing names for `OPCODE_PONG`, `OPCODE_CLOSE` and the frame writer (search for how the loop at `:757-825` answers pings today; move that handling into `read_message` and remove it from the caller). The caller at `:762` calls `read_message` instead of `read_frame`.

Idle timeout: after the handshake (line ~89), set the read timeout unconditionally:

```rust
    let idle = idle_timeout_ms
        .filter(|ms| *ms > 0)
        .unwrap_or(DEFAULT_WEBSOCKET_IDLE_TIMEOUT_MS);
    stream
        .set_read_timeout(Some(Duration::from_millis(idle)))
        .map_err(|err| format!("WebSocket timeout: {err}"))?;
```

with `const DEFAULT_WEBSOCKET_IDLE_TIMEOUT_MS: u64 = 300_000;` next to `DEFAULT_WEBSOCKET_CONNECT_TIMEOUT_MS`. Fix the timeout error text so it reports the idle value, not "connect timeout after 0ms" (find the formatter near `:892-898`).

Abort responsiveness (the review's second half of finding 12) stays as is in this task: abort is checked between frames, and with a 300 s idle timeout that can lag. It is listed as deferred item X3 in `00-index.md`.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p davinci-ai`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-ai/src/codex_ws.rs
git commit -m "fix(codex-ws): reassemble fragmented messages and use an idle timeout after the handshake"
```

---

### Task 4.9 (DECISION D2): Providers without a real wire format refuse tool use instead of corrupting it

**Finding:** davinci-ai 24. Google and Bedrock flatten tool calls and results into text (`stream.rs:1897-1905`, `:1434-1442`); Gemini never gets `functionResponse`; Bedrock gets consecutive user turns and blank assistant text (a `ValidationException`); there is no SigV4 signing, so Bedrock via `AWS_PROFILE` sends unsigned requests; the Vertex URL hard-codes `projects/default/locations/us-central1` (`:1511-1514`); Mistral posts a chat-completions body to `/v1/conversations` (`:1486, 1519`, SUSPECTED); Anthropic `max_tokens` is capped at 8192 (`:1835`); images are dropped everywhere that goes through `content_text`.

**Recommended scope for this plan:** fix the two cheap, certain bugs, and make the stub providers fail loudly for tool use. Building native Gemini, Bedrock (with SigV4) and Vertex clients is a feature project, not a fix, and is listed as deferred item X1.
1. Anthropic `max_tokens`: use the model's `max_tokens` from the catalog, not `8192`.
2. Vertex: build the URL from `GOOGLE_CLOUD_PROJECT` / `GOOGLE_CLOUD_LOCATION` (the names Google's own SDKs read), and error if the project is unset.
3. `google-generative-ai`, `bedrock-converse-stream`, `google-vertex` and Mistral: when the request has tools, return `Err("<provider> does not support tool use in davinci yet; pick a model from another provider for agent work")` before sending.

**Files:**
- Modify: `crates/davinci-ai/src/stream.rs:1486-1520, 1835` and the request entry (`live_complete_with` / `stream_request`) where tools are known

- [ ] **Step 1: Write the failing tests**

```rust
    #[test]
    fn anthropic_max_tokens_follows_the_model() {
        let mut model = load_builtin_models().into_iter().find(|m| m.api == "anthropic-messages").unwrap();
        model.max_tokens = 64_000;
        let body = anthropic_body(&model, &[ChatMessage::text("user", "hi")], None, &[], &StreamOptions::default());
        assert_eq!(body["max_tokens"], 64_000);
    }

    #[test]
    fn vertex_url_uses_the_configured_project_and_location() {
        let mut model = load_builtin_models().into_iter().find(|m| m.api == "google-vertex").unwrap();
        model.base_url = Some("https://us-east5-aiplatform.googleapis.com".into());
        let url = vertex_url(&model, Some("my-proj"), Some("us-east5")).unwrap();
        assert!(url.contains("/projects/my-proj/locations/us-east5/"), "{url}");
        assert!(vertex_url(&model, None, None).is_err());
    }

    #[test]
    fn stub_providers_refuse_tool_use() {
        for api in ["google-generative-ai", "google-vertex", "bedrock-converse-stream"] {
            assert!(refuse_unsupported_tools(api, 1).is_err(), "{api}");
            assert!(refuse_unsupported_tools(api, 0).is_ok(), "{api}");
        }
        assert!(refuse_unsupported_tools("anthropic-messages", 3).is_ok());
    }
```

If no built-in model has `api == "google-vertex"`, build one by cloning a Google model and setting `api`.

- [ ] **Step 2: Run to verify failure**

Run: `cargo test -p davinci-ai --lib anthropic_max_tokens vertex_url stub_providers`
Expected: FAIL.

- [ ] **Step 3: Implement**

At `stream.rs:1835` replace the `8192` cap with `model.max_tokens` (keep any `options.max_tokens` override that is already applied there).

```rust
fn vertex_url(model: &Model, project: Option<&str>, location: Option<&str>) -> Result<String, String> {
    let project = project.filter(|p| !p.is_empty()).ok_or(
        "google-vertex needs GOOGLE_CLOUD_PROJECT (and optionally GOOGLE_CLOUD_LOCATION)",
    )?;
    let location = location.filter(|l| !l.is_empty()).unwrap_or("us-central1");
    let base = model.base_url.as_deref().unwrap_or("https://aiplatform.googleapis.com");
    Ok(format!(
        "{}/v1/projects/{project}/locations/{location}/publishers/google/models/{}:generateContent",
        base.trim_end_matches('/'),
        model.id
    ))
}

/// Providers whose request builders flatten tool calls into text. Sending
/// tools to them produces wrong answers or provider validation errors.
fn refuse_unsupported_tools(api: &str, tool_count: usize) -> Result<(), String> {
    const STUBS: &[&str] = &["google-generative-ai", "google-vertex", "bedrock-converse-stream"];
    if tool_count > 0 && STUBS.contains(&api) {
        return Err(format!(
            "{api} does not support tool use in davinci yet; pick a model from another provider for agent work"
        ));
    }
    Ok(())
}
```

`request_url` for `google-vertex` calls `vertex_url(model, std::env::var("GOOGLE_CLOUD_PROJECT").ok().as_deref(), std::env::var("GOOGLE_CLOUD_LOCATION").ok().as_deref())`; since `request_url` returns `String`, change it to `Result<String, String>` and propagate (its callers already return `Result`). Call `refuse_unsupported_tools(&model.api, tools.len())?` at the start of the request entry that receives `tools`. Add Mistral's api name to `STUBS` only after confirming the `/v1/conversations` body mismatch against Mistral's API docs; if confirmed, also point it at `/v1/chat/completions` with the completions body, which is the documented OpenAI-compatible endpoint.

- [ ] **Step 4: Run to verify pass**

Run: `cargo test -p davinci-ai`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-ai/src/stream.rs
git commit -m "fix(ai): model max_tokens for Anthropic, configurable Vertex project, refuse tools on stub providers"
```

---

### Task 4.10: Verify, then fix, cache-read double counting from OpenAI-compatible proxies

**Finding (SUSPECTED):** davinci-ai 25. `stream.rs:1949-1955` may price `cache_read_input_tokens` twice when a proxy (LiteLLM) returns both `prompt_tokens` (which, in OpenAI semantics, already includes cached tokens) and `cache_read_input_tokens`.

- [ ] **Step 1: Confirm the semantics before changing code**

Read LiteLLM's usage documentation for `cache_read_input_tokens` and `prompt_tokens` (via the context7 docs tool or the LiteLLM docs site). Write down, in the commit message, whether `prompt_tokens` includes the cached count. If it does **not**, stop: there is no bug; add a test that pins today's behavior and commit that instead.

- [ ] **Step 2: Write the failing test (only if Step 1 confirmed double counting)**

```rust
    #[test]
    fn prompt_tokens_already_include_cache_reads() {
        let usage = parse_completions_usage(&serde_json::json!({
            "prompt_tokens": 1000,
            "completion_tokens": 10,
            "cache_read_input_tokens": 800
        }));
        assert_eq!(usage.input, 200);
        assert_eq!(usage.cache_read, 800);
    }
```

(use the real parsing function at `stream.rs:1949`.)

- [ ] **Step 3: Implement**

When `prompt_tokens` is present, `input = prompt_tokens - cache_read - cache_write` (saturating); when only Anthropic-style fields are present, keep today's mapping.

- [ ] **Step 4: Run and commit**

Run: `cargo test -p davinci-ai`
Expected: PASS.

```bash
git add crates/davinci-ai/src/stream.rs
git commit -m "fix(ai): do not double count cached prompt tokens from OpenAI-compatible proxies"
```

---

### Task 4.11: Stream decoding cost stops growing with the square of output length

**Finding:** davinci-ai 11 (performance). Every delta clones the whole `AssistantMessage` into `partial` (`stream_decoder.rs:389-435`, `stream_decoder_anthropic.rs:234-280`, `stream_decoder_completions.rs:290-298`), every event is kept in `events` and returned (`stream.rs:305, 444`), and tool-argument JSON is re-parsed on every delta. A write-tool call with ~100 KB of content costs hundreds of MB and a CPU spike.

**Budget (write it down before changing code, per the performance rule):** decoding a synthetic Anthropic stream of one `write` tool call whose `content` argument is 200 KB, delivered in 2,000 deltas, must allocate less than 20 MB in total and finish in under 200 ms in release mode on the CI Linux runner. Measure today's numbers first with the benchmark in Step 1 and record them in the commit message.

**Files:**
- Create: `crates/davinci-ai/benches/decode_large_tool_call.rs` (plain `#[test] #[ignore]` timing test if the crate has no bench harness; no new dependency)
- Modify: the three decoders' delta paths

- [ ] **Step 1: Write the measurement**

```rust
// crates/davinci-ai/tests/decode_large_tool_call.rs
#[test]
#[ignore = "performance budget; run with --release -- --ignored"]
fn large_tool_call_decodes_within_budget() {
    let frames = davinci_ai::test_support::anthropic_tool_call_frames("write", 200_000, 2_000);
    let started = std::time::Instant::now();
    let message = davinci_ai::test_support::decode_anthropic(&frames);
    let elapsed = started.elapsed();
    assert!(matches!(message.content.last(), Some(davinci_ai::ContentBlock::ToolCall { .. })));
    eprintln!("decode: {elapsed:?}");
    assert!(elapsed < std::time::Duration::from_millis(200), "{elapsed:?}");
}
```

`test_support` is a `#[doc(hidden)] pub mod` behind the `test-fixtures` feature from Task 2.15, holding a frame generator and a decode entry point that returns only the final message.

Run: `cargo test -p davinci-ai --release --test decode_large_tool_call -- --ignored --nocapture`
Expected today: FAIL on time; record the number.

- [ ] **Step 2: Implement**

- Tool-argument deltas append to the slot's `partial` buffer only; parse the buffer once at block end (Task 4.7's finalize point), not per delta. The streamed UI preview uses the raw partial string, which it already receives as `delta`.
- `partial: self.message.clone()` on every delta: change the event payload to `partial: Arc<AssistantMessage>` rebuilt at most every 50 ms or 4 KB of new content (whichever comes first), and always at block end. `AssistantMessageEvent` is consumed by the agent and the TUI; `Arc` derefs to the same type, so most consumers only need `&*partial`. `rg -n "partial" crates/davinci-agent/src crates/davinci-coding-agent/src crates/davinci-tui/src | rg -v "partial_json"` lists the consumers to adjust.
- `stream.rs:305, 444` keep every event in a `Vec` that the caller receives. Keep only the events the caller reads after the stream (check the callers of the returned events) and drop per-delta events once forwarded live.

- [ ] **Step 3: Run the budget and the suite**

Run: the Step 1 command (expected PASS), then `cargo test --workspace`.

- [ ] **Step 4: Commit**

```bash
git add crates
git commit -m "perf(ai): parse tool arguments once and throttle partial snapshots while streaming"
```

---

## Phase 4 exit check

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo test -p davinci-ai --release --test decode_large_tool_call -- --ignored
```

Then the manual check in `00-index.md` → "Manual verification: providers".
