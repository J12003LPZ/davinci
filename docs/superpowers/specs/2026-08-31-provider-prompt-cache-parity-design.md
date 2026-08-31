# Provider prompt-cache parity

Date: 2026-08-31
Status: approved (design), pending implementation plan

## Problem

The Rust port dropped the prompt-cache harness that vendor TypeScript pi ships. Every
Anthropic request is sent without `cache_control` breakpoints, OpenAI requests carry no
`prompt_cache_key` or `prompt_cache_retention`, and the usage parser hardcodes
`cache_read: 0, cache_write: 0`. The result: full input-token price on every call, no
provider-side prefix reuse, and cost accounting that cannot see caching even when a
provider applies it implicitly.

The reference TypeScript implementation (pinned vendor commit) already has the full
harness in `vendor/pi/packages/ai/src/api/*`; this project's parity contract says we
mirror it, not improve on it. The OpenAI codex CLI demonstrates the same discipline
(stable prefix + per-conversation `prompt_cache_key` + cache-aware accounting) and is
the external benchmark the user pointed at.

Out of scope (explicitly decided): a GPTCache-style semantic response cache. It fits an
interactive agent loop badly (every turn's context differs; near-match answers are a
correctness risk) and has no Rust implementation. Provider prompt caching was chosen
instead.

## Goal

A user running the Rust `pi` binary gets the same provider-side prompt caching behavior
as TypeScript pi at the pinned vendor commit: identical request-body cache fields,
identical retention resolution, and cache read/write tokens flowing into `Usage` and
cost math.

## Design

### 1. New module `crates/pi-ai/src/cache.rs`

Mirrors `vendor/pi/packages/ai/src/api/openai-prompt-cache.ts` plus the per-API
retention helpers (each TS API file has a private `resolveCacheRetention`).

- `CacheRetention { Short, Long, None }`.
  Resolution order: explicit `StreamOptions.cache_retention` ("short" | "long" |
  "none") → env `PI_CACHE_RETENTION` ("long" only; any other value ignored) → `Short`.
- `anthropic_cache_control(model, retention) -> Option<serde_json::Value>`:
  `None` retention → no value; otherwise `{"type":"ephemeral"}` with `"ttl":"1h"` added
  when retention is `Long` and compat `supportsLongCacheRetention` is not `false`
  (default true).
- `clamp_openai_prompt_cache_key(key) -> String`: max 64 characters, counted as Unicode
  scalar values (TS `Array.from` semantics), matching
  `OPENAI_PROMPT_CACHE_KEY_MAX_LENGTH`.
- Compat readers over the raw `Model.compat` JSON value with TS defaults:
  `supports_cache_control_on_tools` (default true), `supports_long_cache_retention`
  (default true), `cache_control_format` (`Some("anthropic")` or `None`).

### 2. Anthropic body (`anthropic_body`, `crates/pi-ai/src/stream.rs`)

Mirror `buildParams` / `convertMessages` / `convertTools` in
`vendor/pi/packages/ai/src/api/anthropic-messages.ts`:

- System prompt becomes a block array `[{"type":"text","text":…,"cache_control":…}]`
  (marker omitted when retention is `None`). The OAuth path in TS prepends the Claude
  Code identity block, also cache-marked; during implementation confirm how the Rust
  OAuth request is assembled and mirror whichever shape applies.
- Tools: the last tool definition gets `cache_control` when
  `supportsCacheControlOnTools` (default true).
- Messages: the last message, when its role maps to `user` (plain user or tool-result
  turned user), gets `cache_control` on its final content block; string content is
  converted to a one-block array to carry the marker. This caches conversation history.
- No new beta headers: TS sends `ttl: "1h"` without an extended-cache beta header.

### 3. OpenAI bodies (`openai_responses_body`, `openai_body`, `bedrock_body`)

- `openai_responses_body` (openai-responses, openai-codex-responses,
  azure-openai-responses):
  - `prompt_cache_key` = clamped session id; omitted when retention is `None` or no
    session id (TS `openai-responses.ts:294`, `openai-codex-responses.ts:557`,
    `azure-openai-responses.ts:293` — azure sends the clamped key without the
    retention gate; mirror each file's exact rule).
  - `prompt_cache_retention: "24h"` when retention is `Long` and compat
    `supportsLongCacheRetention` (default true) (`openai-responses.ts:85`).
- `openai_body` (chat completions, `openai-completions.ts:805-810`):
  - `prompt_cache_key` when (base URL contains `api.openai.com` and retention ≠ `None`)
    or (retention is `Long` and compat `supportsLongCacheRetention`).
  - `prompt_cache_retention: "24h"` under the same Long rule.
  - When compat `cacheControlFormat == "anthropic"` (OpenRouter-style providers):
    apply Anthropic-style `cache_control` markers to the system prompt, the last tool
    definition, and the last user/assistant/tool-result text content, with 1h ttl on
    Long (`openai-completions.ts:1055` region).
- `bedrock_body` (`bedrock-converse-stream.ts:879, 1091`): `cachePoint` block after the
  system prompt and after the last message, `ttl: 1h` on Long.

### 4. Usage parsing (both parse sites in `stream.rs`)

The streaming SSE usage build (~line 236) and `parse_provider_response` (~line 1299)
stop hardcoding zeros:

- Anthropic: `cache_read_input_tokens` → `cache_read`,
  `cache_creation_input_tokens` → `cache_write`; `input_tokens` already excludes cached
  tokens, so `input` is taken as-is (`anthropic-messages.ts:604, 749-750`).
- OpenAI completions: `cache_read` =
  `prompt_tokens_details.cached_tokens ?? prompt_cache_hit_tokens ?? cached_tokens ?? 0`
  (OpenAI / DeepSeek / Kimi field variants), `cache_write` =
  `prompt_tokens_details.cache_write_tokens || 0`, and
  `input = max(0, prompt_tokens - cache_read - cache_write)`
  (`openai-completions.ts:1509-1531`).
- OpenAI responses: `cache_read` = `input_tokens_details.cached_tokens || 0`,
  `cache_write` = `input_tokens_details.cache_write_tokens || 0`,
  `input = max(0, input_tokens - cache_read - cache_write)`
  (`openai-responses-shared.ts:561-570`).
- Google: `usageMetadata.cachedContentTokenCount` → `cache_read`; TS subtracts it from
  `promptTokenCount` for `input` (`google-generative-ai.ts:227-231`).
- Bedrock: `cacheReadInputTokens` / `cacheWriteInputTokens`
  (`bedrock-converse-stream.ts:693`).

Cost math in `crates/pi-ai/src/lib.rs` (~line 233) already multiplies
`cache_read`/`cache_write` by the model's cache rates; it starts producing correct
numbers once the parser feeds it real values. Downstream display (TUI meters, `--print`
stats) consumes `Usage` unchanged.

### 5. Session-affinity headers (cache routing; last step, skippable)

- Anthropic: `x-session-affinity: <session id>` when compat
  `sendSessionAffinityHeaders` is true, a session id exists, and caching is enabled
  (`anthropic-messages.ts:950`).
- OpenAI-family: header set per compat `sessionAffinityFormat`
  (`openai` / `openai-nosession` / `openrouter`), mirroring the TS map
  (`openai-completions.ts:375`). Body `prompt_cache_key` stays governed by retention,
  not by this compat.

If this step turns out to interact badly with the existing attribution-header merge, it
can ship separately; steps 1-4 do not depend on it.

## Wiring already in place

- `StreamOptions.cache_retention` exists and is plumbed: the main loop passes `None`
  (defaults to Short) and compaction passes `"none"`
  (`crates/pi-coding-agent/src/main.rs:644, 1484`), matching TS
  `compaction.ts:591`.
- `Model.compat` is a raw `serde_json::Value`, so no catalog schema change is needed.
- No CLI flag is added: TS exposes only the env var and per-call options.

## Not in scope

- Semantic response caching (GPTCache-style embeddings + vector store).
- Restructuring `pi-ai` into per-API modules mirroring the TS file layout.
- `crates/pi-ai/src/request.rs` (the simple `http.rs` client path): patch only if
  pi-parity fixtures reference cache fields there; otherwise leave untouched.
- Google explicit `cachedContent` management (TS does none; implicit caching only).

## Testing

Fixture-only inline `#[cfg(test)]` tests, no network, per repo convention:

- Breakpoint placement per provider: system block, last tool, last user message
  (Anthropic); `prompt_cache_key` presence/absence and `prompt_cache_retention` per
  retention and per compat (OpenAI responses, completions, codex, azure); `cachePoint`
  placement (Bedrock); `cacheControlFormat: "anthropic"` markers (completions).
- Retention resolution: default Short, `PI_CACHE_RETENTION=long`, explicit `"none"`.
- Clamp: 64-character boundary with multibyte characters.
- Usage parsing: fixtures containing each provider's cache token fields, asserting
  `cache_read`/`cache_write` and adjusted `input`.
- Regression: compaction request body carries no cache fields (retention `"none"`).
- Existing tests asserting the old shapes (for example `body["system"] == "sys"`) are
  updated to the block-array shape where the builder changed.

## Risks

- OAuth Anthropic request shape in Rust may differ from the API-key shape; the identity
  block placement must be confirmed against the actual Rust OAuth path before marking
  the Anthropic step done.
- Providers that reject unknown fields: TS gates every field behind the same
  compat/URL checks being mirrored, so any breakage would equally affect TS pi; no
  extra gating is invented.
