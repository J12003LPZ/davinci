# Responses tool identity and terminal output

Freeform `apply_patch` is an opt-in experiment. `PI_CODEX_APPLY_PATCH=1`
requests the custom grammar only when the resolved model, provider, endpoint,
and authentication capabilities support it. An absent, invalid, or false-like
value retains the JSON function definition. Changing the preference affects
new tool declarations; it does not rewrite historical calls.

## Call and result contract

`function_call` and `custom_tool_call` both normalize into the existing
`ToolCall` execution shape. Custom patch input becomes `{ "input": raw_patch }`
without changing its contents. The provider item type, rather than the tool
name or argument shape, supplies the wire kind. A JSON function named
`apply_patch` can have the same `input` property as a custom call.

`AssistantMessage.extra.responsesToolWireKinds` records each call's kind,
keyed by its full call ID. Conversion to `ChatMessage` and session persistence
retain this metadata. Old sessions without the field remain readable;
retained native items supply their original kind when available, and legacy
calls without either representation keep the function form.

Response projection records the kind of each emitted call. Its result uses
that same kind even when the result's cached annotation is missing or wrong.
A native continuation seeds this map from the preserved prefix before
projecting a suffix that may begin with a result. Recovery and interrupted-turn
repair also derive result annotations from the originating assistant call.
The `call|item` identifier stays intact internally; provider output references
use its `call` portion.

## Completion contract

SSE, WebSocket, and non-streaming Responses JSON share `ResponsesDecoder`.
A nonempty terminal `output` array supplies the authoritative ordered output,
including items that had no earlier deltas and corrections to partial output.
Native replay retains the full provider items, including opaque reasoning and
unknown fields. Repeated terminal events do not create additional calls.

An incomplete, failed, cancelled, queued, or still-running response cannot
expose executable calls or establish a native continuation prefix. Partial
text can still be presented; token-limit termination keeps its `Length`
classification. An unterminated Responses stream closes as an error and
removes partial calls. The agent also refuses to execute tools on error,
abort, or length termination.

## Regression coverage

- `davinci-ai` unit tests compare SSE frames, fragmented WebSocket frames, and
  non-streaming JSON on identical mixed text/reasoning/function/custom output.
  They cover terminal-only output, terminal corrections, repeated terminals,
  incomplete statuses, call ID stability, and native result-only suffixes.
- `davinci-ai/tests/responses_freeform.rs` covers explicit opt-in and rollback,
  historical representation, and call authority over result annotations.
- Agent session persistence and operation publication tests cover reopened
  JSON/custom pairs, interrupted-turn repair, and durable recovered custom
  outputs before provider continuation.
- `davinci-agent/tests/freeform_patch_execution.rs` decodes actual function and
  custom provider items before comparing mutation targets and file effects.

These deterministic fixtures do not establish a live backend rollout or a
performance improvement. Freeform remains opt-in pending that evidence.
