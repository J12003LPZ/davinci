# Provider Prompt-Cache Parity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Port the vendor TypeScript prompt-cache harness into `pi-ai`: Anthropic `cache_control` breakpoints, OpenAI `prompt_cache_key` / `prompt_cache_retention`, Bedrock `cachePoint`, Mistral `prompt_cache_key`, cache-aware usage parsing, and session-affinity headers.

**Architecture:** A new `crates/pi-ai/src/cache.rs` module holds retention resolution, cache-control builders, the prompt-cache-key clamp, and compat readers. The existing request builders in `crates/pi-ai/src/stream.rs` call into it. Usage parsing in `stream.rs` gains a shared helper that reads every provider's cache-token fields and computes cost. Spec: `docs/superpowers/specs/2026-08-31-provider-prompt-cache-parity-design.md`.

**Tech Stack:** Rust 1.83.0, serde_json, existing pi-ai crate. No new dependencies.

## Global Constraints

- Mirror vendor TS exactly; cite the TS file in doc comments (repo convention). Reference files: `vendor/pi/packages/ai/src/api/{anthropic-messages,openai-responses,openai-completions,openai-codex-responses,azure-openai-responses,bedrock-converse-stream,mistral-conversations,openai-prompt-cache}.ts`, `openai-responses-shared.ts`.
- Tests are fixture-only, never touch the network, and live in inline `#[cfg(test)] mod tests` blocks.
- Retention values on the wire/options are the strings `"short"`, `"long"`, `"none"`; default is short; env `PI_CACHE_RETENTION=long` upgrades the default. Tests must not mutate process env — use the pure resolver.
- Every commit: `make fmt` (`cargo fmt --check` must pass — run `cargo fmt` first) and `cargo clippy -p pi-ai --all-targets -- -D warnings` clean.
- Commit messages end with:
  `Co-Authored-By: Claude Fable 5 <noreply@anthropic.com>`
  `Claude-Session: https://claude.ai/code/session_013bAm6x66eMjw92DtkcdYmh`

---

### Task 1: `cache.rs` module

**Files:**
- Create: `crates/pi-ai/src/cache.rs`
- Modify: `crates/pi-ai/src/lib.rs` (add `pub mod cache;` next to the other module declarations)

**Interfaces:**
- Consumes: `crate::catalog::Model` (fields `compat: serde_json::Value`, `base_url: Option<String>`, `provider: String`, `id: String`, `name: String`), `crate::stream::StreamOptions` (field `cache_retention: Option<String>`).
- Produces (later tasks call these exactly):
  - `pub enum CacheRetention { Short, Long, None }` (`Copy`, `PartialEq`, `Debug`, `Clone`, `Eq`)
  - `pub fn resolve_cache_retention(explicit: Option<&str>, env: Option<&str>) -> CacheRetention`
  - `pub fn cache_retention_from_options(options: &crate::stream::StreamOptions) -> CacheRetention`
  - `pub const OPENAI_PROMPT_CACHE_KEY_MAX_LENGTH: usize`
  - `pub fn clamp_openai_prompt_cache_key(key: &str) -> String`
  - `pub fn anthropic_cache_control(compat: &serde_json::Value, retention: CacheRetention) -> Option<serde_json::Value>`
  - `pub fn supports_cache_control_on_tools(compat: &serde_json::Value) -> bool`
  - `pub fn supports_long_cache_retention(compat: &serde_json::Value) -> bool`
  - `pub fn completions_cache_control_format(model: &crate::catalog::Model) -> Option<String>`
  - `pub fn completions_supports_long_cache_retention(model: &crate::catalog::Model) -> bool`
  - `pub fn bedrock_supports_prompt_caching(model: &crate::catalog::Model, force_env: Option<&str>) -> bool`
  - `pub fn bedrock_cache_point(retention: CacheRetention) -> Option<serde_json::Value>`

- [ ] **Step 1: Write the failing tests**

Create `crates/pi-ai/src/cache.rs` containing only the test module for now (plus a `use` line so it compiles once implemented — start with the tests referencing the not-yet-written functions):

```rust
//! Prompt-cache helpers matching vendor/pi/packages/ai/src/api/openai-prompt-cache.ts
//! and the per-API cache retention helpers in anthropic-messages.ts,
//! openai-responses.ts, openai-completions.ts, and bedrock-converse-stream.ts.

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{Model, ModelCost};
    use serde_json::{json, Value};

    fn model(provider: &str, id: &str, base_url: &str, compat: Value) -> Model {
        Model {
            id: id.into(),
            name: id.into(),
            api: "openai-completions".into(),
            provider: provider.into(),
            base_url: Some(base_url.into()),
            reasoning: false,
            input: vec!["text".into()],
            cost: ModelCost {
                input: 0.0,
                output: 0.0,
                cache_read: 0.0,
                cache_write: 0.0,
            },
            context_window: 1,
            max_tokens: 1,
            compat,
            headers: Default::default(),
            thinking_level_map: Default::default(),
        }
    }

    #[test]
    fn resolves_retention_with_ts_precedence() {
        assert_eq!(
            resolve_cache_retention(None, None),
            CacheRetention::Short
        );
        assert_eq!(
            resolve_cache_retention(None, Some("long")),
            CacheRetention::Long
        );
        // Only the exact value "long" upgrades; anything else is ignored.
        assert_eq!(
            resolve_cache_retention(None, Some("none")),
            CacheRetention::Short
        );
        assert_eq!(
            resolve_cache_retention(Some("none"), Some("long")),
            CacheRetention::None
        );
        assert_eq!(
            resolve_cache_retention(Some("long"), None),
            CacheRetention::Long
        );
        assert_eq!(
            resolve_cache_retention(Some("short"), Some("long")),
            CacheRetention::Short
        );
        // Unknown explicit values fall back to the env/default chain.
        assert_eq!(
            resolve_cache_retention(Some("bogus"), None),
            CacheRetention::Short
        );
    }

    #[test]
    fn clamps_prompt_cache_key_at_64_chars() {
        let short = "s".repeat(64);
        assert_eq!(clamp_openai_prompt_cache_key(&short), short);
        let long = "x".repeat(70);
        assert_eq!(clamp_openai_prompt_cache_key(&long).chars().count(), 64);
        // Char-count semantics (TS Array.from), not bytes.
        let emoji = "🦀".repeat(70);
        assert_eq!(clamp_openai_prompt_cache_key(&emoji).chars().count(), 64);
    }

    #[test]
    fn builds_anthropic_cache_control_per_retention_and_compat() {
        assert_eq!(
            anthropic_cache_control(&Value::Null, CacheRetention::None),
            None
        );
        assert_eq!(
            anthropic_cache_control(&Value::Null, CacheRetention::Short),
            Some(json!({"type": "ephemeral"}))
        );
        assert_eq!(
            anthropic_cache_control(&Value::Null, CacheRetention::Long),
            Some(json!({"type": "ephemeral", "ttl": "1h"}))
        );
        assert_eq!(
            anthropic_cache_control(
                &json!({"supportsLongCacheRetention": false}),
                CacheRetention::Long
            ),
            Some(json!({"type": "ephemeral"}))
        );
    }

    #[test]
    fn compat_flags_default_true() {
        assert!(supports_cache_control_on_tools(&Value::Null));
        assert!(!supports_cache_control_on_tools(
            &json!({"supportsCacheControlOnTools": false})
        ));
        assert!(supports_long_cache_retention(&Value::Null));
        assert!(!supports_long_cache_retention(
            &json!({"supportsLongCacheRetention": false})
        ));
    }

    #[test]
    fn detects_completions_cache_control_format() {
        // Explicit compat wins.
        let explicit = model(
            "custom",
            "anything",
            "https://example.com/v1",
            json!({"cacheControlFormat": "anthropic"}),
        );
        assert_eq!(
            completions_cache_control_format(&explicit).as_deref(),
            Some("anthropic")
        );
        // OpenRouter + anthropic/ model id is auto-detected (openai-completions.ts:1621).
        let openrouter = model(
            "openrouter",
            "anthropic/claude-sonnet-5",
            "https://openrouter.ai/api/v1",
            Value::Null,
        );
        assert_eq!(
            completions_cache_control_format(&openrouter).as_deref(),
            Some("anthropic")
        );
        let openrouter_gpt = model(
            "openrouter",
            "openai/gpt-5.5",
            "https://openrouter.ai/api/v1",
            Value::Null,
        );
        assert_eq!(completions_cache_control_format(&openrouter_gpt), None);
        let plain = model("openai", "gpt-5.5", "https://api.openai.com/v1", Value::Null);
        assert_eq!(completions_cache_control_format(&plain), None);
    }

    #[test]
    fn completions_long_retention_excludes_known_hosts() {
        let openai = model("openai", "gpt-5.5", "https://api.openai.com/v1", Value::Null);
        assert!(completions_supports_long_cache_retention(&openai));
        let together = model(
            "together",
            "meta-llama",
            "https://api.together.xyz/v1",
            Value::Null,
        );
        assert!(!completions_supports_long_cache_retention(&together));
        let nvidia = model(
            "nvidia",
            "nim",
            "https://integrate.api.nvidia.com/v1",
            Value::Null,
        );
        assert!(!completions_supports_long_cache_retention(&nvidia));
        let cloudflare = model(
            "cloudflare-workers-ai",
            "wf",
            "https://api.cloudflare.com/client/v4",
            Value::Null,
        );
        assert!(!completions_supports_long_cache_retention(&cloudflare));
        let ant_ling = model("ant-ling", "ling", "https://example.com/v1", Value::Null);
        assert!(!completions_supports_long_cache_retention(&ant_ling));
        // Explicit compat overrides detection.
        let forced = model(
            "together",
            "meta-llama",
            "https://api.together.xyz/v1",
            json!({"supportsLongCacheRetention": true}),
        );
        assert!(completions_supports_long_cache_retention(&forced));
    }

    #[test]
    fn bedrock_caching_gated_to_claude_models() {
        let fable = model("amazon-bedrock", "anthropic.claude-fable-5", "https://bedrock-runtime.us-east-1.amazonaws.com", Value::Null);
        assert!(bedrock_supports_prompt_caching(&fable, None));
        let sonnet4 = model("amazon-bedrock", "anthropic.claude-sonnet-4-20250514-v1:0", "https://bedrock-runtime.us-east-1.amazonaws.com", Value::Null);
        assert!(bedrock_supports_prompt_caching(&sonnet4, None));
        let sonnet37 = model("amazon-bedrock", "anthropic.claude-3-7-sonnet-20250219-v1:0", "https://bedrock-runtime.us-east-1.amazonaws.com", Value::Null);
        assert!(bedrock_supports_prompt_caching(&sonnet37, None));
        let haiku35 = model("amazon-bedrock", "anthropic.claude-3-5-haiku-20241022-v1:0", "https://bedrock-runtime.us-east-1.amazonaws.com", Value::Null);
        assert!(bedrock_supports_prompt_caching(&haiku35, None));
        let nova = model("amazon-bedrock", "amazon.nova-pro-v1:0", "https://bedrock-runtime.us-east-1.amazonaws.com", Value::Null);
        assert!(!bedrock_supports_prompt_caching(&nova, None));
        // Inference profiles without "claude" in the ARN can be forced via env.
        let profile = model("amazon-bedrock", "arn:aws:bedrock:us-east-1:123:application-inference-profile/abc", "https://bedrock-runtime.us-east-1.amazonaws.com", Value::Null);
        assert!(!bedrock_supports_prompt_caching(&profile, None));
        assert!(bedrock_supports_prompt_caching(&profile, Some("1")));
        // Claude 3.5 Sonnet is NOT in the allow list.
        let sonnet35 = model("amazon-bedrock", "anthropic.claude-3-5-sonnet-20241022-v2:0", "https://bedrock-runtime.us-east-1.amazonaws.com", Value::Null);
        assert!(!bedrock_supports_prompt_caching(&sonnet35, None));
    }

    #[test]
    fn builds_bedrock_cache_point() {
        assert_eq!(bedrock_cache_point(CacheRetention::None), None);
        assert_eq!(
            bedrock_cache_point(CacheRetention::Short),
            Some(json!({"cachePoint": {"type": "default"}}))
        );
        assert_eq!(
            bedrock_cache_point(CacheRetention::Long),
            Some(json!({"cachePoint": {"type": "default", "ttl": "1h"}}))
        );
    }
}
```

- [ ] **Step 2: Add `pub mod cache;` to `crates/pi-ai/src/lib.rs` and run tests to verify they fail**

Run: `cargo test -p pi-ai cache 2>&1 | tail -20`
Expected: COMPILE ERROR — `resolve_cache_retention` etc. not found.

- [ ] **Step 3: Write the implementation** (above the tests module in `cache.rs`)

```rust
use serde_json::{json, Value};

use crate::catalog::Model;
use crate::stream::StreamOptions;

/// TS `CacheRetention`: `"short" | "long" | "none"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheRetention {
    Short,
    Long,
    None,
}

/// TS `resolveCacheRetention` (identical in each API file): explicit value
/// first, then `PI_CACHE_RETENTION=long`, defaulting to short.
pub fn resolve_cache_retention(explicit: Option<&str>, env: Option<&str>) -> CacheRetention {
    match explicit {
        Some("short") => return CacheRetention::Short,
        Some("long") => return CacheRetention::Long,
        Some("none") => return CacheRetention::None,
        _ => {}
    }
    if env == Some("long") {
        return CacheRetention::Long;
    }
    CacheRetention::Short
}

/// Resolve retention from stream options plus the real process environment.
pub fn cache_retention_from_options(options: &StreamOptions) -> CacheRetention {
    resolve_cache_retention(
        options.cache_retention.as_deref(),
        std::env::var("PI_CACHE_RETENTION").ok().as_deref(),
    )
}

/// TS `OPENAI_PROMPT_CACHE_KEY_MAX_LENGTH` (openai-prompt-cache.ts).
pub const OPENAI_PROMPT_CACHE_KEY_MAX_LENGTH: usize = 64;

/// TS `clampOpenAIPromptCacheKey`: `Array.from` semantics — count Unicode
/// scalar values, not bytes.
pub fn clamp_openai_prompt_cache_key(key: &str) -> String {
    key.chars().take(OPENAI_PROMPT_CACHE_KEY_MAX_LENGTH).collect()
}

fn compat_bool(compat: &Value, key: &str, default: bool) -> bool {
    compat.get(key).and_then(Value::as_bool).unwrap_or(default)
}

/// TS anthropic-messages.ts `getCacheControl`: no marker for retention none;
/// `ttl: "1h"` only for long retention on models that support it (default true).
pub fn anthropic_cache_control(compat: &Value, retention: CacheRetention) -> Option<Value> {
    match retention {
        CacheRetention::None => None,
        CacheRetention::Short => Some(json!({"type": "ephemeral"})),
        CacheRetention::Long => {
            if compat_bool(compat, "supportsLongCacheRetention", true) {
                Some(json!({"type": "ephemeral", "ttl": "1h"}))
            } else {
                Some(json!({"type": "ephemeral"}))
            }
        }
    }
}

/// TS `AnthropicMessagesCompat.supportsCacheControlOnTools`. Default: true.
pub fn supports_cache_control_on_tools(compat: &Value) -> bool {
    compat_bool(compat, "supportsCacheControlOnTools", true)
}

/// TS `supportsLongCacheRetention` (all compat interfaces). Default: true.
pub fn supports_long_cache_retention(compat: &Value) -> bool {
    compat_bool(compat, "supportsLongCacheRetention", true)
}

/// TS openai-completions.ts:1621 — explicit compat, else auto-detected for
/// OpenRouter Anthropic models.
pub fn completions_cache_control_format(model: &Model) -> Option<String> {
    if let Some(format) = model
        .compat
        .get("cacheControlFormat")
        .and_then(Value::as_str)
    {
        return Some(format.to_string());
    }
    if model.provider == "openrouter" && model.id.starts_with("anthropic/") {
        return Some("anthropic".to_string());
    }
    None
}

/// TS openai-completions.ts detected `supportsLongCacheRetention`: false for
/// Together, Cloudflare Workers AI / AI Gateway, NVIDIA NIM, and Ant Ling.
pub fn completions_supports_long_cache_retention(model: &Model) -> bool {
    if let Some(explicit) = model
        .compat
        .get("supportsLongCacheRetention")
        .and_then(Value::as_bool)
    {
        return explicit;
    }
    let base = model.base_url.as_deref().unwrap_or("");
    let blocked = base.contains("api.together.xyz")
        || base.contains("api.cloudflare.com")
        || base.contains("gateway.ai.cloudflare.com")
        || base.contains("integrate.api.nvidia.com")
        || model.provider == "together"
        || model.provider == "cloudflare-workers-ai"
        || model.provider == "cloudflare-ai-gateway"
        || model.provider == "nvidia"
        || model.provider == "ant-ling";
    !blocked
}

/// TS bedrock-converse-stream.ts `supportsPromptCaching`: Claude 5 / 4.x /
/// 3.7 Sonnet / 3.5 Haiku by id or name; `AWS_BEDROCK_FORCE_CACHE=1` forces
/// caching for inference profiles whose ARN hides the model name.
/// `force_env` is the value of `AWS_BEDROCK_FORCE_CACHE` (injected for tests).
pub fn bedrock_supports_prompt_caching(model: &Model, force_env: Option<&str>) -> bool {
    let candidates = [model.id.to_lowercase(), model.name.to_lowercase()];
    let has_claude = candidates.iter().any(|s| s.contains("claude"));
    if !has_claude {
        return force_env == Some("1");
    }
    candidates.iter().any(|s| {
        s.contains("fable-5")
            || s.contains("opus-5")
            || s.contains("sonnet-5")
            || s.contains("-4-")
            || s.contains("claude-3-7-sonnet")
            || s.contains("claude-3-5-haiku")
    })
}

/// TS bedrock-converse-stream.ts cachePoint blocks (lines 879, 1091).
pub fn bedrock_cache_point(retention: CacheRetention) -> Option<Value> {
    match retention {
        CacheRetention::None => None,
        CacheRetention::Short => Some(json!({"cachePoint": {"type": "default"}})),
        CacheRetention::Long => {
            Some(json!({"cachePoint": {"type": "default", "ttl": "1h"}}))
        }
    }
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p pi-ai cache 2>&1 | tail -10`
Expected: all Task 1 tests PASS.

- [ ] **Step 5: fmt, clippy, commit**

```bash
cargo fmt
cargo clippy -p pi-ai --all-targets -- -D warnings
git add crates/pi-ai/src/cache.rs crates/pi-ai/src/lib.rs
git commit -m "feat(ai): prompt-cache helpers module mirroring the TS harness"
```

---

### Task 2: Anthropic `cache_control` breakpoints

**Files:**
- Modify: `crates/pi-ai/src/stream.rs` — `anthropic_body` (~line 1029) and its callers' expectations; add tests to the existing `#[cfg(test)] mod tests`.

**Interfaces:**
- Consumes (Task 1): `crate::cache::{cache_retention_from_options, anthropic_cache_control, supports_cache_control_on_tools, CacheRetention}`.
- Produces: `anthropic_body` emits `system` as a block array `[{"type":"text","text":…,"cache_control"?}]`, marks the last tool and the last user message. Signature unchanged.

- [ ] **Step 1: Write the failing test** (append inside `mod tests` in `stream.rs`)

```rust
#[test]
fn anthropic_body_places_cache_control_breakpoints() {
    let anthropic = load_builtin_models()
        .into_iter()
        .find(|m| m.api == "anthropic-messages")
        .expect("anthropic");
    let messages = vec![
        ChatMessage::text("user", "first"),
        ChatMessage {
            role: "assistant".into(),
            content: vec![MessageContent::Text {
                text: "reply".into(),
            }],
            ..ChatMessage::default()
        },
        ChatMessage::text("user", "second"),
    ];
    let tools = vec![
        ToolSpec {
            name: "read".into(),
            description: "read".into(),
            parameters: serde_json::json!({"type":"object"}),
            constrained_sampling: None,
        },
        ToolSpec {
            name: "write".into(),
            description: "write".into(),
            parameters: serde_json::json!({"type":"object"}),
            constrained_sampling: None,
        },
    ];
    let options = StreamOptions {
        cache_retention: Some("short".into()),
        ..StreamOptions::default()
    };
    let body = request_body_with(&anthropic, &messages, Some("sys"), &tools, &options);

    // System prompt becomes a cache-marked block array (anthropic-messages.ts:1027-1034).
    assert_eq!(body["system"][0]["type"], "text");
    assert_eq!(body["system"][0]["text"], "sys");
    assert_eq!(body["system"][0]["cache_control"]["type"], "ephemeral");
    // Last tool only (anthropic-messages.ts:1360).
    assert!(body["tools"][0].get("cache_control").is_none());
    assert_eq!(body["tools"][1]["cache_control"]["type"], "ephemeral");
    // Last user message: string content wrapped into a marked block
    // (anthropic-messages.ts:1296-1317).
    let last = body["messages"].as_array().unwrap().last().unwrap();
    assert_eq!(last["role"], "user");
    assert_eq!(last["content"][0]["text"], "second");
    assert_eq!(last["content"][0]["cache_control"]["type"], "ephemeral");
    // Earlier messages untouched.
    assert_eq!(body["messages"][0]["content"], "first");

    // Long retention adds ttl 1h.
    let long = request_body_with(
        &anthropic,
        &messages,
        Some("sys"),
        &tools,
        &StreamOptions {
            cache_retention: Some("long".into()),
            ..StreamOptions::default()
        },
    );
    assert_eq!(long["system"][0]["cache_control"]["ttl"], "1h");

    // Retention none: block-array system, no markers anywhere.
    let none = request_body_with(
        &anthropic,
        &messages,
        Some("sys"),
        &tools,
        &StreamOptions {
            cache_retention: Some("none".into()),
            ..StreamOptions::default()
        },
    );
    assert_eq!(none["system"][0]["text"], "sys");
    assert!(none["system"][0].get("cache_control").is_none());
    assert!(none["tools"][1].get("cache_control").is_none());
    let none_last = none["messages"].as_array().unwrap().last().unwrap();
    assert_eq!(none_last["content"], "second");
}

#[test]
fn anthropic_body_marks_tool_result_content_block() {
    let anthropic = load_builtin_models()
        .into_iter()
        .find(|m| m.api == "anthropic-messages")
        .expect("anthropic");
    let messages = vec![ChatMessage {
        role: "toolResult".into(),
        tool_call_id: Some("t1".into()),
        content: vec![MessageContent::Text { text: "out".into() }],
        ..ChatMessage::default()
    }];
    let body = request_body_with(
        &anthropic,
        &messages,
        None,
        &[],
        &StreamOptions {
            cache_retention: Some("short".into()),
            ..StreamOptions::default()
        },
    );
    let last = body["messages"].as_array().unwrap().last().unwrap();
    assert_eq!(last["role"], "user");
    assert_eq!(last["content"][0]["type"], "tool_result");
    assert_eq!(last["content"][0]["cache_control"]["type"], "ephemeral");
}

#[test]
fn anthropic_body_respects_tool_cache_compat() {
    let mut anthropic = load_builtin_models()
        .into_iter()
        .find(|m| m.api == "anthropic-messages")
        .expect("anthropic");
    anthropic.compat = serde_json::json!({"supportsCacheControlOnTools": false});
    let tools = vec![ToolSpec {
        name: "read".into(),
        description: "read".into(),
        parameters: serde_json::json!({"type":"object"}),
        constrained_sampling: None,
    }];
    let body = request_body_with(
        &anthropic,
        &[],
        None,
        &tools,
        &StreamOptions {
            cache_retention: Some("short".into()),
            ..StreamOptions::default()
        },
    );
    assert!(body["tools"][0].get("cache_control").is_none());
}
```

Note: check `ToolSpec`'s actual field list in `crates/pi-ai/src/types.rs` before writing — if it has more fields than shown, use `..Default::default()` or fill them; if `ChatMessage::text` doesn't exist with that signature, construct the struct literally the way neighboring tests do.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p pi-ai anthropic_body 2>&1 | tail -20`
Expected: FAIL — system is a plain string, no `cache_control` fields.

- [ ] **Step 3: Implement in `anthropic_body`**

At the top of `anthropic_body` resolve the marker once:

```rust
let cache_control = crate::cache::anthropic_cache_control(
    &model.compat,
    crate::cache::cache_retention_from_options(options),
);
```

Change the `converted` binding to `let mut converted: Vec<Value> = …` and, after it is built, mark the last user message (TS `convertMessages` lines 1296-1317):

```rust
if let Some(cache_control) = cache_control.as_ref() {
    if let Some(last) = converted.last_mut() {
        if last.get("role").and_then(Value::as_str) == Some("user") {
            match last.get_mut("content") {
                Some(Value::Array(blocks)) => {
                    if let Some(block) = blocks.last_mut() {
                        block["cache_control"] = cache_control.clone();
                    }
                }
                Some(content) if content.is_string() => {
                    let text = content.as_str().unwrap_or_default().to_string();
                    *content = serde_json::json!([{
                        "type": "text",
                        "text": text,
                        "cache_control": cache_control,
                    }]);
                }
                _ => {}
            }
        }
    }
}
```

Replace the system assignment (TS `buildParams` lines 1025-1034):

```rust
if let Some(system) = system {
    let mut block = serde_json::json!({"type": "text", "text": system});
    if let Some(cache_control) = cache_control.as_ref() {
        block["cache_control"] = cache_control.clone();
    }
    body["system"] = Value::Array(vec![block]);
}
```

After the tools array is built, mark the last tool (TS `convertTools` line 1360):

```rust
if let Some(cache_control) = cache_control
    .as_ref()
    .filter(|_| crate::cache::supports_cache_control_on_tools(&model.compat))
{
    if let Some(tools_array) = body.get_mut("tools").and_then(Value::as_array_mut) {
        if let Some(last) = tools_array.last_mut() {
            last["cache_control"] = cache_control.clone();
        }
    }
}
```

- [ ] **Step 4: Run the pi-ai suite**

Run: `cargo test -p pi-ai 2>&1 | tail -10`
Expected: new tests PASS; if any existing test asserted `body["system"]` as a string for anthropic, update it to the block-array shape (`body["system"][0]["text"]`). Do not touch `crates/pi-ai/src/request.rs` — that is the separate `http.rs` client path, out of scope.

- [ ] **Step 5: fmt, clippy, commit**

```bash
cargo fmt
cargo clippy -p pi-ai --all-targets -- -D warnings
git add crates/pi-ai/src/stream.rs
git commit -m "feat(ai): anthropic cache_control breakpoints on system, tools, and history"
```

---

### Task 3: OpenAI Responses `prompt_cache_key` (responses / codex / azure)

**Files:**
- Modify: `crates/pi-ai/src/stream.rs` — `openai_responses_body` (~line 577); tests in the same file.

**Interfaces:**
- Consumes (Task 1): `crate::cache::{cache_retention_from_options, clamp_openai_prompt_cache_key, supports_long_cache_retention, CacheRetention}`.
- Produces: `openai_responses_body` emits `prompt_cache_key` / `prompt_cache_retention` / `prompt_cache_options` per API flavor. Signature unchanged.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn responses_bodies_carry_prompt_cache_key() {
    let mut model = load_builtin_models()
        .into_iter()
        .find(|m| m.api == "openai-responses")
        .expect("openai responses model");
    let session = StreamOptions {
        session_id: Some("sess-1234".into()),
        ..StreamOptions::default()
    };
    // openai-responses: key present, no retention field on short.
    let body = request_body_with(&model, &[], None, &[], &session);
    assert_eq!(body["prompt_cache_key"], "sess-1234");
    assert!(body.get("prompt_cache_retention").is_none());
    assert!(body.get("prompt_cache_options").is_none());

    // Long retention adds prompt_cache_retention 24h (openai-responses.ts:85).
    let long = request_body_with(
        &model,
        &[],
        None,
        &[],
        &StreamOptions {
            session_id: Some("sess-1234".into()),
            cache_retention: Some("long".into()),
            ..StreamOptions::default()
        },
    );
    assert_eq!(long["prompt_cache_retention"], "24h");

    // Retention none: no key; explicit opt-out only with compat flag
    // (openai-responses.ts:289-296).
    let none = request_body_with(
        &model,
        &[],
        None,
        &[],
        &StreamOptions {
            session_id: Some("sess-1234".into()),
            cache_retention: Some("none".into()),
            ..StreamOptions::default()
        },
    );
    assert!(none.get("prompt_cache_key").is_none());
    assert!(none.get("prompt_cache_options").is_none());
    model.compat = serde_json::json!({"supportsExplicitPromptCacheMode": true});
    let explicit = request_body_with(
        &model,
        &[],
        None,
        &[],
        &StreamOptions {
            session_id: Some("sess-1234".into()),
            cache_retention: Some("none".into()),
            ..StreamOptions::default()
        },
    );
    assert_eq!(explicit["prompt_cache_options"]["mode"], "explicit");

    // Key is clamped to 64 chars.
    let clamped = request_body_with(
        &model,
        &[],
        None,
        &[],
        &StreamOptions {
            session_id: Some("k".repeat(80)),
            ..StreamOptions::default()
        },
    );
    assert_eq!(
        clamped["prompt_cache_key"].as_str().unwrap().chars().count(),
        64
    );

    // No session id: no key.
    let anonymous = request_body_with(&model, &[], None, &[], &StreamOptions::default());
    assert!(anonymous.get("prompt_cache_key").is_none());
}

#[test]
fn codex_and_azure_bodies_carry_prompt_cache_key() {
    let codex = load_builtin_models()
        .into_iter()
        .find(|m| m.api == "openai-codex-responses")
        .expect("codex model");
    let body = request_body_with(
        &codex,
        &[],
        None,
        &[],
        &StreamOptions {
            session_id: Some("sess-codex".into()),
            ..StreamOptions::default()
        },
    );
    // openai-codex-responses.ts:557 — key from the cache session id.
    assert_eq!(body["prompt_cache_key"], "sess-codex");
    assert!(body.get("prompt_cache_retention").is_none());
    let none = request_body_with(
        &codex,
        &[],
        None,
        &[],
        &StreamOptions {
            session_id: Some("sess-codex".into()),
            cache_retention: Some("none".into()),
            ..StreamOptions::default()
        },
    );
    assert!(none.get("prompt_cache_key").is_none());

    // azure-openai-responses.ts:293 — always sends the clamped key, no
    // retention gate, no retention field.
    let mut azure = load_builtin_models()
        .into_iter()
        .find(|m| m.api == "openai-codex-responses")
        .expect("base model to clone");
    azure.api = "azure-openai-responses".into();
    let azure_body = request_body_with(
        &azure,
        &[],
        None,
        &[],
        &StreamOptions {
            session_id: Some("sess-azure".into()),
            cache_retention: Some("none".into()),
            ..StreamOptions::default()
        },
    );
    assert_eq!(azure_body["prompt_cache_key"], "sess-azure");
    assert!(azure_body.get("prompt_cache_retention").is_none());
}
```

If the built-in catalog has a real `azure-openai-responses` model, find it directly instead of mutating a codex model's `api`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p pi-ai prompt_cache_key 2>&1 | tail -20`
Expected: FAIL — no `prompt_cache_key` field.

- [ ] **Step 3: Implement in `openai_responses_body`**

After the `if codex { … }` block and before the tools handling, add:

```rust
let retention = crate::cache::cache_retention_from_options(options);
let session_key = options
    .session_id
    .as_deref()
    .filter(|id| !id.is_empty())
    .map(crate::cache::clamp_openai_prompt_cache_key);
match model.api.as_str() {
    // azure-openai-responses.ts:293 — clamped key, no retention gate.
    "azure-openai-responses" => {
        if let Some(key) = session_key {
            body["prompt_cache_key"] = Value::String(key);
        }
    }
    // openai-codex-responses.ts:267-268, 557 — key unless retention none.
    "openai-codex-responses" => {
        if retention != crate::cache::CacheRetention::None {
            if let Some(key) = session_key {
                body["prompt_cache_key"] = Value::String(key);
            }
        }
    }
    // openai-responses.ts:288-296.
    _ => {
        if retention != crate::cache::CacheRetention::None {
            if let Some(key) = session_key {
                body["prompt_cache_key"] = Value::String(key);
            }
        }
        if retention == crate::cache::CacheRetention::Long
            && crate::cache::supports_long_cache_retention(&model.compat)
        {
            body["prompt_cache_retention"] = Value::String("24h".into());
        }
        let explicit_mode = model
            .compat
            .get("supportsExplicitPromptCacheMode")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        if retention == crate::cache::CacheRetention::None && explicit_mode {
            body["prompt_cache_options"] = serde_json::json!({"mode": "explicit"});
        }
    }
}
```

(`body` is created earlier in the function; insert this after `body` exists and after the `if codex { … }` mutations.)

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p pi-ai 2>&1 | tail -10`
Expected: PASS, including the codex websocket tests in `codex.rs` (the body now carries `prompt_cache_key`; if a codex test asserts the exact body, update it).

- [ ] **Step 5: fmt, clippy, commit**

```bash
cargo fmt
cargo clippy -p pi-ai --all-targets -- -D warnings
git add crates/pi-ai/src/stream.rs
git commit -m "feat(ai): prompt_cache_key and retention for OpenAI Responses APIs"
```

---

### Task 4: OpenAI Completions + Mistral cache fields

**Files:**
- Modify: `crates/pi-ai/src/stream.rs` — `openai_body` (~line 890), `mistral_body` (~line 853), `request_body_with` (~line 548); tests in the same file.

**Interfaces:**
- Consumes (Task 1): `crate::cache::{cache_retention_from_options, clamp_openai_prompt_cache_key, completions_cache_control_format, completions_supports_long_cache_retention, anthropic_cache_control, CacheRetention}`.
- Produces: `openai_body` emits `prompt_cache_key` / `prompt_cache_retention` and, for `cacheControlFormat: "anthropic"` providers, Anthropic-style `cache_control` markers. `mistral_body(model, messages, system, tools, options)` gains the options parameter and emits `prompt_cache_key`. A new private helper `apply_anthropic_cache_control_to_completions(body: &mut Value, cache_control: &Value)` exists in `stream.rs`.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn completions_body_carries_prompt_cache_key_for_openai() {
    let model = load_builtin_models()
        .into_iter()
        .find(|m| m.api == "openai-completions" && m.provider == "openai")
        .expect("openai completions model");
    // api.openai.com + retention short → key (openai-completions.ts:805-809).
    let body = request_body_with(
        &model,
        &[],
        None,
        &[],
        &StreamOptions {
            session_id: Some("sess-c".into()),
            ..StreamOptions::default()
        },
    );
    assert_eq!(body["prompt_cache_key"], "sess-c");
    assert!(body.get("prompt_cache_retention").is_none());
    // Long adds 24h retention.
    let long = request_body_with(
        &model,
        &[],
        None,
        &[],
        &StreamOptions {
            session_id: Some("sess-c".into()),
            cache_retention: Some("long".into()),
            ..StreamOptions::default()
        },
    );
    assert_eq!(long["prompt_cache_retention"], "24h");
    // Retention none: neither field.
    let none = request_body_with(
        &model,
        &[],
        None,
        &[],
        &StreamOptions {
            session_id: Some("sess-c".into()),
            cache_retention: Some("none".into()),
            ..StreamOptions::default()
        },
    );
    assert!(none.get("prompt_cache_key").is_none());
    assert!(none.get("prompt_cache_retention").is_none());
}

#[test]
fn completions_body_skips_key_for_non_openai_hosts_on_short() {
    let mut model = load_builtin_models()
        .into_iter()
        .find(|m| m.api == "openai-completions")
        .expect("completions model");
    model.base_url = Some("https://api.example.com/v1".into());
    model.compat = serde_json::Value::Null;
    let short = request_body_with(
        &model,
        &[],
        None,
        &[],
        &StreamOptions {
            session_id: Some("sess-x".into()),
            ..StreamOptions::default()
        },
    );
    assert!(short.get("prompt_cache_key").is_none());
    // But long retention sends it when the host supports long retention.
    let long = request_body_with(
        &model,
        &[],
        None,
        &[],
        &StreamOptions {
            session_id: Some("sess-x".into()),
            cache_retention: Some("long".into()),
            ..StreamOptions::default()
        },
    );
    assert_eq!(long["prompt_cache_key"], "sess-x");
    assert_eq!(long["prompt_cache_retention"], "24h");
}

#[test]
fn completions_body_applies_anthropic_markers_for_openrouter_claude() {
    let mut model = load_builtin_models()
        .into_iter()
        .find(|m| m.api == "openai-completions")
        .expect("completions model");
    model.provider = "openrouter".into();
    model.id = "anthropic/claude-sonnet-5".into();
    model.base_url = Some("https://openrouter.ai/api/v1".into());
    model.compat = serde_json::Value::Null;
    let tools = vec![ToolSpec {
        name: "read".into(),
        description: "read".into(),
        parameters: serde_json::json!({"type":"object"}),
        constrained_sampling: None,
    }];
    let messages = vec![ChatMessage::text("user", "hello")];
    let body = request_body_with(
        &model,
        &messages,
        Some("sys"),
        &tools,
        &StreamOptions {
            session_id: Some("sess-or".into()),
            ..StreamOptions::default()
        },
    );
    // System message content converted to a marked block array
    // (openai-completions.ts:1140-1151).
    let system = &body["messages"][0];
    assert_eq!(system["role"], "system");
    assert_eq!(system["content"][0]["text"], "sys");
    assert_eq!(system["content"][0]["cache_control"]["type"], "ephemeral");
    // Last tool marked (openai-completions.ts:1101-1111).
    assert_eq!(
        body["tools"][0]["cache_control"]["type"],
        "ephemeral"
    );
    // Last conversation message marked (openai-completions.ts:1087-1099).
    let last = body["messages"].as_array().unwrap().last().unwrap();
    assert_eq!(last["content"][0]["cache_control"]["type"], "ephemeral");
}

#[test]
fn mistral_body_carries_prompt_cache_key() {
    let mut model = load_builtin_models()
        .into_iter()
        .find(|m| m.api == "openai-completions")
        .expect("base model");
    model.api = "mistral-conversations".into();
    let body = request_body_with(
        &model,
        &[],
        None,
        &[],
        &StreamOptions {
            session_id: Some("sess-m".into()),
            ..StreamOptions::default()
        },
    );
    // mistral-conversations.ts:521 — unclamped session id, gated only by
    // retention != none plus a present session id.
    assert_eq!(body["prompt_cache_key"], "sess-m");
    let none = request_body_with(
        &model,
        &[],
        None,
        &[],
        &StreamOptions {
            session_id: Some("sess-m".into()),
            cache_retention: Some("none".into()),
            ..StreamOptions::default()
        },
    );
    assert!(none.get("prompt_cache_key").is_none());
}
```

If the catalog has a real `mistral-conversations` model, use it instead of mutating `api`.

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p pi-ai completions_body 2>&1 | tail -20` and `cargo test -p pi-ai mistral_body 2>&1 | tail -10`
Expected: FAIL — fields absent.

- [ ] **Step 3: Implement**

In `openai_body`, after `apply_openai_thinking(&mut body, model, options);` and before `body` is returned:

```rust
// openai-completions.ts:805-810.
let retention = crate::cache::cache_retention_from_options(options);
let long_supported = retention == crate::cache::CacheRetention::Long
    && crate::cache::completions_supports_long_cache_retention(model);
let is_openai_host = model
    .base_url
    .as_deref()
    .unwrap_or("https://api.openai.com/v1")
    .contains("api.openai.com");
if (is_openai_host && retention != crate::cache::CacheRetention::None) || long_supported {
    if let Some(key) = options
        .session_id
        .as_deref()
        .filter(|id| !id.is_empty())
        .map(crate::cache::clamp_openai_prompt_cache_key)
    {
        body["prompt_cache_key"] = Value::String(key);
    }
}
if long_supported {
    body["prompt_cache_retention"] = Value::String("24h".into());
}
// openai-completions.ts getCompatCacheControl + applyAnthropicCacheControl.
if crate::cache::completions_cache_control_format(model).as_deref() == Some("anthropic") {
    if let Some(cache_control) = crate::cache::anthropic_cache_control(
        &serde_json::json!({
            "supportsLongCacheRetention":
                crate::cache::completions_supports_long_cache_retention(model),
        }),
        retention,
    ) {
        apply_anthropic_cache_control_to_completions(&mut body, &cache_control);
    }
}
```

Add the helper below `openai_body` (mirrors `applyAnthropicCacheControl`, openai-completions.ts:1065-1167):

```rust
/// TS openai-completions.ts `applyAnthropicCacheControl`: mark the system
/// prompt, the last tool definition, and the last user/assistant/tool message
/// with Anthropic-style `cache_control` for `cacheControlFormat: "anthropic"`
/// providers.
fn apply_anthropic_cache_control_to_completions(body: &mut Value, cache_control: &Value) {
    fn mark_text_content(message: &mut Value, cache_control: &Value) -> bool {
        match message.get_mut("content") {
            Some(content) if content.is_string() => {
                let text = content.as_str().unwrap_or_default().to_string();
                if text.is_empty() {
                    return false;
                }
                *content = serde_json::json!([{
                    "type": "text",
                    "text": text,
                    "cache_control": cache_control,
                }]);
                true
            }
            Some(Value::Array(parts)) => {
                for part in parts.iter_mut().rev() {
                    if part.get("type").and_then(Value::as_str) == Some("text") {
                        part["cache_control"] = cache_control.clone();
                        return true;
                    }
                }
                false
            }
            _ => false,
        }
    }

    if let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) {
        // First system/developer message.
        for message in messages.iter_mut() {
            let role = message.get("role").and_then(Value::as_str);
            if role == Some("system") || role == Some("developer") {
                mark_text_content(message, cache_control);
                break;
            }
        }
        // Last user/assistant/tool message with markable text.
        for message in messages.iter_mut().rev() {
            let role = message.get("role").and_then(Value::as_str);
            if role == Some("user") || role == Some("assistant") || role == Some("tool") {
                if mark_text_content(message, cache_control) {
                    break;
                }
            }
        }
    }
    if let Some(tools) = body.get_mut("tools").and_then(Value::as_array_mut) {
        if let Some(last) = tools.last_mut() {
            last["cache_control"] = cache_control.clone();
        }
    }
}
```

Change `mistral_body` to take and use options (update the call in `request_body_with` from `mistral_body(model, messages, system, tools)` to pass `options`):

```rust
fn mistral_body(
    model: &Model,
    messages: &[ChatMessage],
    system: Option<&str>,
    tools: &[ToolSpec],
    options: &StreamOptions,
) -> Value {
    // Build on the completions shape without inheriting its OpenAI cache
    // fields; mistral-conversations.ts has its own gate.
    let mut body = openai_body(model, messages, system, tools, &StreamOptions::default());
    body["stream"] = Value::Bool(false);
    // mistral-conversations.ts:533-535 `shouldUsePromptCaching`.
    let retention = crate::cache::cache_retention_from_options(options);
    if retention != crate::cache::CacheRetention::None {
        if let Some(session_id) = options.session_id.as_deref().filter(|id| !id.is_empty()) {
            body["prompt_cache_key"] = Value::String(session_id.to_string());
        }
    }
    body
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p pi-ai 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 5: fmt, clippy, commit**

```bash
cargo fmt
cargo clippy -p pi-ai --all-targets -- -D warnings
git add crates/pi-ai/src/stream.rs
git commit -m "feat(ai): completions and mistral prompt-cache fields with anthropic-format markers"
```

---

### Task 5: Bedrock `cachePoint` blocks

**Files:**
- Modify: `crates/pi-ai/src/stream.rs` — `bedrock_body` (~line 817) and its call in `request_body_with` (~line 562); tests in the same file.

**Interfaces:**
- Consumes (Task 1): `crate::cache::{cache_retention_from_options, bedrock_supports_prompt_caching, bedrock_cache_point, CacheRetention}`.
- Produces: `bedrock_body(model, messages, system, tools, options)` gains the options parameter; system block list and last user message content gain a `cachePoint` entry for supported Claude models.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn bedrock_body_adds_cache_points_for_claude() {
    let mut model = load_builtin_models()
        .into_iter()
        .find(|m| m.api == "bedrock-converse-stream")
        .expect("bedrock model");
    model.id = "anthropic.claude-fable-5".into();
    let messages = vec![ChatMessage::text("user", "hello")];
    let body = request_body_with(
        &model,
        &messages,
        Some("sys"),
        &[],
        &StreamOptions {
            cache_retention: Some("short".into()),
            ..StreamOptions::default()
        },
    );
    // bedrock-converse-stream.ts:879 — cachePoint appended to system blocks.
    let system = body["system"].as_array().unwrap();
    assert_eq!(system[0]["text"], "sys");
    assert_eq!(system[1]["cachePoint"]["type"], "default");
    // bedrock-converse-stream.ts:1086-1094 — cachePoint on last user message.
    let last = body["messages"].as_array().unwrap().last().unwrap();
    let content = last["content"].as_array().unwrap();
    assert_eq!(content.last().unwrap()["cachePoint"]["type"], "default");

    // Long retention carries ttl.
    let long = request_body_with(
        &model,
        &messages,
        Some("sys"),
        &[],
        &StreamOptions {
            cache_retention: Some("long".into()),
            ..StreamOptions::default()
        },
    );
    assert_eq!(long["system"][1]["cachePoint"]["ttl"], "1h");

    // Non-Claude bedrock models get no cache points.
    model.id = "amazon.nova-pro-v1:0".into();
    let nova = request_body_with(
        &model,
        &messages,
        Some("sys"),
        &[],
        &StreamOptions {
            cache_retention: Some("short".into()),
            ..StreamOptions::default()
        },
    );
    assert_eq!(nova["system"].as_array().unwrap().len(), 1);
    let nova_last = nova["messages"].as_array().unwrap().last().unwrap();
    assert!(nova_last["content"]
        .as_array()
        .unwrap()
        .iter()
        .all(|block| block.get("cachePoint").is_none()));

    // Retention none: no cache points even for Claude.
    model.id = "anthropic.claude-fable-5".into();
    let none = request_body_with(
        &model,
        &messages,
        Some("sys"),
        &[],
        &StreamOptions {
            cache_retention: Some("none".into()),
            ..StreamOptions::default()
        },
    );
    assert_eq!(none["system"].as_array().unwrap().len(), 1);
}
```

Note: `model.name` may still say e.g. "Nova Pro" after overwriting `model.id` — set `model.name = model.id.clone()` in the test when switching ids so the gate sees consistent candidates.

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p pi-ai bedrock_body 2>&1 | tail -20`
Expected: FAIL — `system` has one block, no `cachePoint`.

- [ ] **Step 3: Implement**

Change the signature and the `request_body_with` call site:

```rust
"bedrock-converse-stream" => bedrock_body(model, messages, system, tools, options),
```

In `bedrock_body`:

```rust
fn bedrock_body(
    model: &Model,
    messages: &[ChatMessage],
    system: Option<&str>,
    tools: &[ToolSpec],
    options: &StreamOptions,
) -> Value {
    let retention = crate::cache::cache_retention_from_options(options);
    let cache_point = if crate::cache::bedrock_supports_prompt_caching(
        model,
        std::env::var("AWS_BEDROCK_FORCE_CACHE").ok().as_deref(),
    ) {
        crate::cache::bedrock_cache_point(retention)
    } else {
        None
    };
    let mut converted: Vec<Value> = messages
        .iter()
        .map(|message| {
            serde_json::json!({
                "role": if message.role == "assistant" { "assistant" } else { "user" },
                "content": [{"text": content_text(&message.content)}],
            })
        })
        .collect();
    // bedrock-converse-stream.ts:1086-1094 — cachePoint on the last user message.
    if let Some(cache_point) = cache_point.as_ref() {
        if let Some(last) = converted.last_mut() {
            if last.get("role").and_then(Value::as_str) == Some("user") {
                if let Some(content) = last.get_mut("content").and_then(Value::as_array_mut) {
                    content.push(cache_point.clone());
                }
            }
        }
    }
    let mut body = serde_json::json!({
        "modelId": model.id,
        "messages": converted,
    });
    if let Some(system) = system {
        // bedrock-converse-stream.ts:879 — cachePoint after the system text block.
        let mut blocks = vec![serde_json::json!({"text": system})];
        if let Some(cache_point) = cache_point.as_ref() {
            blocks.push(cache_point.clone());
        }
        body["system"] = Value::Array(blocks);
    }
    // … existing toolConfig handling unchanged …
    body
}
```

- [ ] **Step 4: Run tests to verify they pass**

Run: `cargo test -p pi-ai 2>&1 | tail -10`
Expected: PASS.

- [ ] **Step 5: fmt, clippy, commit**

```bash
cargo fmt
cargo clippy -p pi-ai --all-targets -- -D warnings
git add crates/pi-ai/src/stream.rs
git commit -m "feat(ai): bedrock cachePoint blocks for supported Claude models"
```

---

### Task 6: Cache-aware usage parsing and cost

**Files:**
- Modify: `crates/pi-ai/src/stream.rs` — the streaming usage build (~line 236) and the `parse_provider_response` usage build (~line 1299); tests in the same file.

**Interfaces:**
- Consumes: `crate::calculate_usage(model, input, output, cache_read, cache_write) -> Usage` (exists, `crates/pi-ai/src/lib.rs:229`).
- Produces: `fn usage_from_value(model: &Model, usage: &Value) -> Usage` and `fn usage_from_google_metadata(model: &Model, metadata: &Value) -> Usage`, both private to `stream.rs`, used by both parse sites.

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn usage_parses_cache_tokens_per_provider() {
    let model = load_builtin_models()
        .into_iter()
        .find(|m| m.api == "anthropic-messages")
        .expect("model");
    // Anthropic: input_tokens already excludes cached tokens
    // (anthropic-messages.ts:604, 749-750).
    let anthropic = usage_from_value(
        &model,
        &serde_json::json!({
            "input_tokens": 10,
            "output_tokens": 5,
            "cache_read_input_tokens": 900,
            "cache_creation_input_tokens": 100,
        }),
    );
    assert_eq!(anthropic.input, 10);
    assert_eq!(anthropic.cache_read, 900);
    assert_eq!(anthropic.cache_write, 100);
    assert!(anthropic.cost.total > 0.0);

    // OpenAI completions: cached and written tokens are inside prompt_tokens,
    // so they are subtracted (openai-completions.ts:1509-1531).
    let completions = usage_from_value(
        &model,
        &serde_json::json!({
            "prompt_tokens": 1000,
            "completion_tokens": 20,
            "prompt_tokens_details": {"cached_tokens": 700, "cache_write_tokens": 100},
        }),
    );
    assert_eq!(completions.input, 200);
    assert_eq!(completions.cache_read, 700);
    assert_eq!(completions.cache_write, 100);

    // DeepSeek / Kimi field variants.
    let deepseek = usage_from_value(
        &model,
        &serde_json::json!({"prompt_tokens": 100, "completion_tokens": 1, "prompt_cache_hit_tokens": 40}),
    );
    assert_eq!(deepseek.cache_read, 40);
    assert_eq!(deepseek.input, 60);
    let kimi = usage_from_value(
        &model,
        &serde_json::json!({"prompt_tokens": 100, "completion_tokens": 1, "cached_tokens": 30}),
    );
    assert_eq!(kimi.cache_read, 30);
    assert_eq!(kimi.input, 70);

    // OpenAI responses shape (openai-responses-shared.ts:561-570).
    let responses = usage_from_value(
        &model,
        &serde_json::json!({
            "input_tokens": 500,
            "output_tokens": 10,
            "input_tokens_details": {"cached_tokens": 450},
            "total_tokens": 510,
        }),
    );
    assert_eq!(responses.input, 50);
    assert_eq!(responses.cache_read, 450);
    assert_eq!(responses.total_tokens, 510);

    // Bedrock camelCase (bedrock-converse-stream.ts:693).
    let bedrock = usage_from_value(
        &model,
        &serde_json::json!({
            "inputTokens": 25,
            "outputTokens": 5,
            "cacheReadInputTokens": 300,
            "cacheWriteInputTokens": 50,
        }),
    );
    assert_eq!(bedrock.input, 25);
    assert_eq!(bedrock.cache_read, 300);
    assert_eq!(bedrock.cache_write, 50);

    // Never negative.
    let odd = usage_from_value(
        &model,
        &serde_json::json!({"prompt_tokens": 10, "prompt_tokens_details": {"cached_tokens": 50}}),
    );
    assert_eq!(odd.input, 0);
}

#[test]
fn usage_parses_google_metadata() {
    let model = load_builtin_models()
        .into_iter()
        .find(|m| m.api == "google-generative-ai")
        .expect("google model");
    // google-generative-ai.ts:227-231 — cached tokens subtracted from prompt count.
    let usage = usage_from_google_metadata(
        &model,
        &serde_json::json!({
            "promptTokenCount": 1000,
            "candidatesTokenCount": 30,
            "totalTokenCount": 1030,
            "cachedContentTokenCount": 800,
        }),
    );
    assert_eq!(usage.input, 200);
    assert_eq!(usage.cache_read, 800);
    assert_eq!(usage.output, 30);
    assert_eq!(usage.total_tokens, 1030);
}

#[test]
fn parse_provider_response_reads_cache_usage() {
    let model = load_builtin_models()
        .into_iter()
        .find(|m| m.api == "anthropic-messages")
        .expect("model");
    let parsed = parse_provider_response(
        &model,
        r#"{"content":[{"type":"text","text":"hi"}],"usage":{"input_tokens":3,"output_tokens":2,"cache_read_input_tokens":70,"cache_creation_input_tokens":7}}"#,
    );
    let usage = parsed.usage.expect("usage");
    assert_eq!(usage.cache_read, 70);
    assert_eq!(usage.cache_write, 7);
}

#[test]
fn parse_provider_response_reads_google_usage_metadata() {
    let model = load_builtin_models()
        .into_iter()
        .find(|m| m.api == "google-generative-ai")
        .expect("google model");
    let parsed = parse_provider_response(
        &model,
        r#"{"candidates":[{"content":{"parts":[{"text":"hi"}]}}],"usageMetadata":{"promptTokenCount":100,"candidatesTokenCount":5,"totalTokenCount":105,"cachedContentTokenCount":60}}"#,
    );
    let usage = parsed.usage.expect("usage");
    assert_eq!(usage.cache_read, 60);
    assert_eq!(usage.input, 40);
}
```

- [ ] **Step 2: Run tests to verify they fail**

Run: `cargo test -p pi-ai usage_parses 2>&1 | tail -20`
Expected: COMPILE ERROR — `usage_from_value` not found.

- [ ] **Step 3: Implement the helpers** (near `parse_provider_response` in `stream.rs`)

```rust
/// Cache-aware usage extraction across provider shapes. Mirrors:
/// - anthropic-messages.ts:604, 749-750 (input excludes cached; own keys)
/// - openai-completions.ts:1509-1531 (subtract cached + written from prompt)
/// - openai-responses-shared.ts:561-570 (subtract details from input_tokens)
/// - bedrock-converse-stream.ts:693 (camelCase converse keys)
fn usage_from_value(model: &Model, usage: &Value) -> Usage {
    let get = |key: &str| usage.get(key).and_then(Value::as_u64);
    let base_input = get("prompt_tokens")
        .or_else(|| get("input_tokens"))
        .or_else(|| get("inputTokens"))
        .unwrap_or(0);
    let output = get("completion_tokens")
        .or_else(|| get("output_tokens"))
        .or_else(|| get("outputTokens"))
        .unwrap_or(0);
    let anthropic_read = get("cache_read_input_tokens").or_else(|| get("cacheReadInputTokens"));
    let anthropic_write =
        get("cache_creation_input_tokens").or_else(|| get("cacheWriteInputTokens"));
    let (input, cache_read, cache_write) = if anthropic_read.is_some() || anthropic_write.is_some()
    {
        // Anthropic / Bedrock: input token counts already exclude cached tokens.
        (
            base_input,
            anthropic_read.unwrap_or(0),
            anthropic_write.unwrap_or(0),
        )
    } else {
        let read = usage
            .pointer("/prompt_tokens_details/cached_tokens")
            .or_else(|| usage.pointer("/input_tokens_details/cached_tokens"))
            .and_then(Value::as_u64)
            .or_else(|| get("prompt_cache_hit_tokens"))
            .or_else(|| get("cached_tokens"))
            .unwrap_or(0);
        let write = usage
            .pointer("/prompt_tokens_details/cache_write_tokens")
            .or_else(|| usage.pointer("/input_tokens_details/cache_write_tokens"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        (
            base_input.saturating_sub(read).saturating_sub(write),
            read,
            write,
        )
    };
    let mut computed = crate::calculate_usage(model, input, output, cache_read, cache_write);
    if let Some(total) = get("total_tokens").or_else(|| get("totalTokens")) {
        computed.total_tokens = total;
    }
    computed
}

/// google-generative-ai.ts:225-236 — usageMetadata mapping with cached tokens
/// subtracted from the prompt count.
fn usage_from_google_metadata(model: &Model, metadata: &Value) -> Usage {
    let get = |key: &str| metadata.get(key).and_then(Value::as_u64);
    let cached = get("cachedContentTokenCount").unwrap_or(0);
    let input = get("promptTokenCount").unwrap_or(0).saturating_sub(cached);
    let output = get("candidatesTokenCount").unwrap_or(0);
    let mut computed = crate::calculate_usage(model, input, output, cached, 0);
    if let Some(total) = get("totalTokenCount") {
        computed.total_tokens = total;
    }
    computed
}
```

Replace the streaming usage build (~line 236):

```rust
if let Some(usage) = value.get("usage") {
    message.usage = Some(usage_from_value(model, usage));
}
```

(Confirm `model` is in scope in `replay_sse_events`; it is a parameter.)

Replace the `parse_provider_response` usage build (~line 1299):

```rust
let usage = value
    .get("usage")
    .map(|usage| usage_from_value(model, usage))
    .or_else(|| {
        value
            .get("usageMetadata")
            .map(|metadata| usage_from_google_metadata(model, metadata))
    });
```

Check whether `Usage`'s `cost` field type matches what `calculate_usage` returns (it does — both are the `pi_protocol` `Usage`); the previous literal-struct construction is deleted.

- [ ] **Step 4: Run the full crate suite**

Run: `cargo test -p pi-ai 2>&1 | tail -15`
Expected: PASS. Existing tests asserting `usage.input` for plain fixtures still pass because subtraction of zero is identity; if a test asserted `cost == Default::default()`, update it — cost is now computed.

- [ ] **Step 5: fmt, clippy, commit**

```bash
cargo fmt
cargo clippy -p pi-ai --all-targets -- -D warnings
git add crates/pi-ai/src/stream.rs
git commit -m "feat(ai): parse provider cache tokens into usage and compute cost"
```

---

### Task 7: Session-affinity headers

**Files:**
- Modify: `crates/pi-ai/src/stream.rs` — `collect_request_headers` (~line 443) and its two call sites (`live_complete_with` ~line 364, `live_complete_streaming_with` ~line 418); tests in the same file.

**Interfaces:**
- Consumes (Task 1): `crate::cache::{cache_retention_from_options, CacheRetention}`.
- Produces: `collect_request_headers(model, auth, options)` — the third parameter changes from `Option<&str>` (session id) to `&StreamOptions`; read `options.session_id` inside. Emits affinity headers per API.

- [ ] **Step 1: Read `collect_request_headers` first** (~line 443) to see its current shape and how the session id parameter is used today; adapt the diff below to it.

- [ ] **Step 2: Write the failing test**

```rust
#[test]
fn affinity_headers_follow_ts_rules() {
    let session = StreamOptions {
        session_id: Some("sess-aff".into()),
        ..StreamOptions::default()
    };
    let auth = crate::auth::ResolvedAuth {
        api_key: Some("k".into()),
        ..Default::default()
    };
    let header = |headers: &[(String, String)], name: &str| -> Option<String> {
        headers
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.clone())
    };

    // Anthropic: x-session-affinity only with compat opt-in
    // (anthropic-messages.ts:950).
    let mut anthropic = load_builtin_models()
        .into_iter()
        .find(|m| m.api == "anthropic-messages")
        .expect("anthropic");
    let plain = collect_request_headers(&anthropic, &auth, &session);
    assert_eq!(header(&plain, "x-session-affinity"), None);
    anthropic.compat = serde_json::json!({"sendSessionAffinityHeaders": true});
    let opted = collect_request_headers(&anthropic, &auth, &session);
    assert_eq!(
        header(&opted, "x-session-affinity"),
        Some("sess-aff".into())
    );
    // Retention none disables it.
    let none = collect_request_headers(
        &anthropic,
        &auth,
        &StreamOptions {
            session_id: Some("sess-aff".into()),
            cache_retention: Some("none".into()),
            ..StreamOptions::default()
        },
    );
    assert_eq!(header(&none, "x-session-affinity"), None);

    // openai-responses: unconditional when a session id exists
    // (openai-responses.ts:237-246); openai format also sends session_id.
    let responses = load_builtin_models()
        .into_iter()
        .find(|m| m.api == "openai-responses")
        .expect("responses model");
    let resp_headers = collect_request_headers(&responses, &auth, &session);
    assert_eq!(
        header(&resp_headers, "x-client-request-id"),
        Some("sess-aff".into())
    );
    assert_eq!(header(&resp_headers, "session_id"), Some("sess-aff".into()));

    // openrouter format sends x-session-id instead.
    let mut openrouter = responses.clone();
    openrouter.provider = "openrouter".into();
    openrouter.base_url = Some("https://openrouter.ai/api/v1".into());
    let or_headers = collect_request_headers(&openrouter, &auth, &session);
    assert_eq!(header(&or_headers, "x-session-id"), Some("sess-aff".into()));
    assert_eq!(header(&or_headers, "x-client-request-id"), None);

    // openai-completions: gated by sendSessionAffinityHeaders (default false)
    // (openai-completions.ts:761-770).
    let completions = load_builtin_models()
        .into_iter()
        .find(|m| m.api == "openai-completions" && m.provider == "openai")
        .expect("completions model");
    let comp_plain = collect_request_headers(&completions, &auth, &session);
    assert_eq!(header(&comp_plain, "x-session-affinity"), None);
    let mut comp_opted_model = completions.clone();
    comp_opted_model.compat = serde_json::json!({"sendSessionAffinityHeaders": true});
    let comp_opted = collect_request_headers(&comp_opted_model, &auth, &session);
    assert_eq!(
        header(&comp_opted, "x-session-affinity"),
        Some("sess-aff".into())
    );
    assert_eq!(
        header(&comp_opted, "x-client-request-id"),
        Some("sess-aff".into())
    );
    assert_eq!(header(&comp_opted, "session_id"), Some("sess-aff".into()));

    // mistral: x-affinity when caching enabled (mistral-conversations.ts:343-345).
    let mut mistral = completions.clone();
    mistral.api = "mistral-conversations".into();
    mistral.compat = serde_json::Value::Null;
    let mistral_headers = collect_request_headers(&mistral, &auth, &session);
    assert_eq!(
        header(&mistral_headers, "x-affinity"),
        Some("sess-aff".into())
    );
}
```

Adjust `ResolvedAuth` construction to its real field set (read `crates/pi-ai/src/auth.rs` if the `..Default::default()` form does not compile).

- [ ] **Step 3: Run test to verify it fails**

Run: `cargo test -p pi-ai affinity_headers 2>&1 | tail -20`
Expected: COMPILE ERROR (signature) or FAIL (headers absent).

- [ ] **Step 4: Implement**

Change the signature to `fn collect_request_headers(model: &Model, auth: &ResolvedAuth, options: &StreamOptions) -> Vec<(String, String)>`, derive `let session_id = options.session_id.as_deref().filter(|id| !id.is_empty());` and keep every existing use of the old parameter working off that binding. Update both call sites to pass `options` instead of `options.session_id.as_deref()`.

Append before returning the headers:

```rust
if let Some(session_id) = session_id {
    let retention = crate::cache::cache_retention_from_options(options);
    let affinity_format = model
        .compat
        .get("sessionAffinityFormat")
        .and_then(Value::as_str)
        .map(str::to_string)
        .unwrap_or_else(|| {
            // openai-responses.ts:50-52 detectSessionAffinityFormat.
            let openrouter = model.provider == "openrouter"
                || model
                    .base_url
                    .as_deref()
                    .is_some_and(|url| url.contains("openrouter.ai"));
            if openrouter { "openrouter".into() } else { "openai".into() }
        });
    let opted_in = model
        .compat
        .get("sendSessionAffinityHeaders")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    match model.api.as_str() {
        // anthropic-messages.ts:950 — compat opt-in plus caching enabled.
        "anthropic-messages" | "pi-messages" => {
            if opted_in && retention != crate::cache::CacheRetention::None {
                headers.push(("x-session-affinity".into(), session_id.to_string()));
            }
        }
        // openai-responses.ts:237-246 — unconditional.
        "openai-responses" | "azure-openai-responses" => {
            if affinity_format == "openrouter" {
                headers.push(("x-session-id".into(), session_id.to_string()));
            } else {
                if affinity_format == "openai" {
                    headers.push(("session_id".into(), session_id.to_string()));
                }
                headers.push(("x-client-request-id".into(), session_id.to_string()));
            }
        }
        // mistral-conversations.ts:343-345.
        "mistral-conversations" => {
            if retention != crate::cache::CacheRetention::None {
                headers.push(("x-affinity".into(), session_id.to_string()));
            }
        }
        // openai-codex-responses handles its own headers in codex.rs.
        "openai-codex-responses" => {}
        // openai-completions.ts:761-770 — compat opt-in.
        _ => {
            if opted_in {
                if affinity_format == "openrouter" {
                    headers.push(("x-session-id".into(), session_id.to_string()));
                } else {
                    if affinity_format == "openai" {
                        headers.push(("session_id".into(), session_id.to_string()));
                    }
                    headers.push(("x-client-request-id".into(), session_id.to_string()));
                    headers.push(("x-session-affinity".into(), session_id.to_string()));
                }
            }
        }
    }
}
```

If `collect_request_headers` already emits one of these headers for some API, keep the existing emission and skip the duplicate — dedupe by header name before returning.

- [ ] **Step 5: Run tests, fmt, clippy, commit**

Run: `cargo test -p pi-ai 2>&1 | tail -10`
Expected: PASS.

```bash
cargo fmt
cargo clippy -p pi-ai --all-targets -- -D warnings
git add crates/pi-ai/src/stream.rs
git commit -m "feat(ai): session-affinity headers for prompt-cache routing"
```

---

### Task 8: Workspace verification and compaction regression

**Files:**
- Modify: none expected; `crates/pi-coding-agent/src/main.rs` only if the compaction regression fails.

- [ ] **Step 1: Add the compaction regression test** (in `stream.rs` tests — verifies retention "none" produces a clean body end-to-end)

```rust
#[test]
fn compaction_retention_none_strips_all_cache_fields() {
    let anthropic = load_builtin_models()
        .into_iter()
        .find(|m| m.api == "anthropic-messages")
        .expect("anthropic");
    // main.rs:644 passes cache_retention "none" for compaction requests,
    // matching TS compaction.ts:591.
    let options = StreamOptions {
        session_id: Some("sess-compact".into()),
        cache_retention: Some("none".into()),
        ..StreamOptions::default()
    };
    let body = request_body_with(
        &anthropic,
        &[ChatMessage::text("user", "summarize")],
        Some("sys"),
        &[],
        &options,
    );
    let raw = serde_json::to_string(&body).unwrap();
    assert!(!raw.contains("cache_control"));
    assert!(!raw.contains("prompt_cache_key"));
}
```

- [ ] **Step 2: Run the full workspace suite**

Run: `cargo test --workspace 2>&1 | tail -15`
Expected: PASS. Fix any fallout in dependent crates (`pi-agent`, `pi-coding-agent` fixtures that assert request bodies).

- [ ] **Step 3: Final gates**

```bash
cargo fmt
make clippy
```
Expected: both clean (`make clippy` runs the whole workspace with `-D warnings`).

- [ ] **Step 4: Commit**

```bash
git add -A crates
git commit -m "test(ai): compaction retention none regression for cache fields"
```

---

## Self-Review Notes

- Spec coverage: spec §1 → Task 1; §2 → Task 2; §3 → Tasks 3-5; §4 → Task 6; §5 → Task 7; testing section → per-task tests plus Task 8. The spec's OAuth-identity-block risk is Task 2 territory: the Rust builder has no OAuth-specific system path today (no Claude Code identity block exists anywhere in `crates/`), so there is nothing to mirror yet; if one is added later it must receive the same `cache_control` treatment.
- Struct-literal fields in tests (`ToolSpec`, `ChatMessage`, `ResolvedAuth`, `Model`) must be checked against the real definitions before use; the plan flags this at each site.
- `pi-messages` API shares `anthropic_body`, so it inherits the breakpoints — intended (TS `pi-messages.ts` wraps the Anthropic client).
