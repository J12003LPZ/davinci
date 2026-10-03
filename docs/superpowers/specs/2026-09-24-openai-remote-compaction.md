# OpenAI remote compaction: source research

Research date: 2026-09-25. Research only; no implementation or backend request.
Codex source is pinned to revision
`d7b07d45517a793acfba4cbf8de697d723cceb46`. Backend behavior, availability and
entitlements are not established by this source inspection.

## Transport and request

The inspected remote-v2 path uses the ordinary Responses streaming client,
not a separate `/responses/compact` endpoint. The default ChatGPT provider
base is `https://chatgpt.com/backend-api/codex`; the HTTP Responses client
posts to `responses`, yielding `/backend-api/codex/responses`. The shared
client also supports WebSocket transport and its HTTP fallback.

The attempt clones the history, trims eligible recent tool outputs if needed,
collects model input with executed-tool metadata, and appends a typed
`ResponseItem::CompactionTrigger {}`. Its prompt includes the model-visible
tools, base instructions and parallel tool-call support, with no output schema.
The ordinary request builder supplies model, instructions, input, tools,
`tool_choice: auto`, reasoning, `store: false`, `stream: true`, encrypted
reasoning inclusion, service tier, prompt cache key and client metadata.
The optional responses-lite projection instead places instructions and tools
in a deterministic developer prefix; it is not evidence of a different
compaction endpoint.

Sources: [remote attempt](https://github.com/openai/codex/blob/d7b07d45517a793acfba4cbf8de697d723cceb46/codex-rs/core/src/compact_remote_v2_attempt.rs),
[request client](https://github.com/openai/codex/blob/d7b07d45517a793acfba4cbf8de697d723cceb46/codex-rs/core/src/client.rs),
[provider base](https://github.com/openai/codex/blob/d7b07d45517a793acfba4cbf8de697d723cceb46/codex-rs/model-provider-info/src/lib.rs),
[HTTP endpoint](https://github.com/openai/codex/blob/d7b07d45517a793acfba4cbf8de697d723cceb46/codex-rs/codex-api/src/endpoint/responses.rs).

## Response and history replacement

A successful attempt requires `response.completed` and exactly one typed
`ResponseItem::Compaction`. It returns that opaque item, response ID and
optional usage. Other response items do not become a conventional textual
summary in the replacement history.

History reconstruction retains real user/hook prompts and selected inter-agent
control messages; descendant progress/final-answer messages are excluded.
Selected inter-agent history has a 10,000-token budget. Client-authored
developer messages can also be retained. A newest-first 64,000-token retention
budget and feature-dependent image budget constrain the retained messages.
The opaque compaction item is appended, and initial context is inserted when
required for a mid-turn compaction. Successful replacement persists a checkpoint
with response/model/context metadata and recomputes token usage.

Oversized tool-output trimming works on a clone and only rewrites eligible
recent outputs while preserving identifiers and metadata. It stops at an
ineligible item. Failed attempts do not commit that trimmed clone as history.

Sources: [remote orchestration and replacement](https://github.com/openai/codex/blob/d7b07d45517a793acfba4cbf8de697d723cceb46/codex-rs/core/src/compact_remote_v2.rs),
[retained history](https://github.com/openai/codex/blob/d7b07d45517a793acfba4cbf8de697d723cceb46/codex-rs/core/src/compact_remote_history.rs).

## Triggers and fallback

Manual compaction can invoke remote-v2. Automatic pre-turn and mid/post-turn
paths compare usage with the configured auto-compaction budget or usable
context window. Model changes can require compaction when the compaction hash
changes, or when switching to a smaller context window whose budget is already
exceeded. There is no universal numerical trigger established by these paths.

The router selects remote-v2 when the model capability supports it; unsupported
models use the existing local summarizer. A remote-v2 request failure does not
automatically invoke that local summarizer in the inspected path. Stream retries
are bounded by the lesser of the provider limit and two, per transport; a
WebSocket-to-HTTP fallback can reset that retry count. The unlimited connection
retry behavior for sampling does not apply to remote-v2.

When attempting compaction under a previous model, the orchestration can retry
once under a supplied current-model context. Abort, interruption and session
budget errors bypass this model fallback. If the fallback also fails, the
original error is returned. History is replaced only after success.

The token-budget feature is a separate local context-window reset mechanism,
not a server-generated compaction summary or a remote failure fallback.

Sources: [turn routing](https://github.com/openai/codex/blob/d7b07d45517a793acfba4cbf8de697d723cceb46/codex-rs/core/src/session/turn.rs),
[retry policy](https://github.com/openai/codex/blob/d7b07d45517a793acfba4cbf8de697d723cceb46/codex-rs/core/src/responses_retry.rs),
[model fallback](https://github.com/openai/codex/blob/d7b07d45517a793acfba4cbf8de697d723cceb46/codex-rs/core/src/compact_model_fallback.rs),
[token budget](https://github.com/openai/codex/blob/d7b07d45517a793acfba4cbf8de697d723cceb46/codex-rs/core/src/compact_token_budget.rs).

## Possible davinci integration, requiring approval

An opt-in `compaction.remote` setting, default false and restricted to
`openai-codex`, could select a separate remote strategy. Leave the local
compaction prompts and default behavior untouched. Today,
`crates/davinci-agent/src/compaction.rs::compact_messages_with_options`
creates a summary context message followed by the retained recent messages;
on summarizer error it returns the original messages without compaction.

A future implementation would need:

- Typed opaque compaction items that survive serialization, persistence and
  request replay without conversion to ordinary chat text.
- Explicit model capability and authentication checks, with unsupported routes
  retaining local behavior. Backend rejection must not silently destroy history.
- Atomic durable replacement, preserving the previous history on transport,
  cancellation, malformed-response or persistence errors.
- Deliberate WebSocket continuation reset and cache-identity handling after
  replacement, plus observable usage and compaction outcomes.
- Tests for missing/multiple compaction items, disconnects, cancellation,
  replay, persistence failures and unsupported providers, and a genuine
  long-session quality evaluation against local compaction.

The short coding benchmark does not exercise compaction and cannot validate
this feature. No product code is proposed in this research commit.

## Unconfirmed and verification record

Source inspection cannot establish account entitlement, supported model rollout,
backend compatibility guarantees, opaque-item cryptographic semantics, latency,
pricing, cache reuse or answer quality. No live remote-compaction call was made.

Verification comprised three source passes: (1) reading the four compaction
modules named by the plan; (2) tracing transport, request construction, routing
and retry/model-fallback callers; (3) checking davinci's current compaction
replacement and failure behavior. Linked source files were fetched at the pinned
revision. These are source-derived observations, not backend acceptance results.
