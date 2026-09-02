# Competitive harness roadmap and phase 1 design

Date: 2026-09-01. Branch: `rust-rewrite`.

## Why

The goal moved. Until now the target was product equivalence with TypeScript
`pi`. The new target is a harness that competes with Claude Code and Codex CLI
on both the engineering underneath and the interface on top. This document
records what an audit of the current binary found, breaks the gap into
sub-projects, and specifies the first one in full.

## What the audit found (2026-09-01, debug build of `rust-rewrite` + working tree)

Verified by running the binary, not by reading code alone.

### Blocking defects

1. **Tool calls are dropped on every streaming provider.** A prompt that makes
   `openai-codex/gpt-5.6-luna` call `read` ends the turn with an empty
   assistant message (`content: []`, `stopReason: stop`) in print, JSON and
   interactive modes, with and without the headroom proxy. Root cause:
   `pi-ai/src/codex.rs::replay_codex_events` never assembles a `ToolCall`
   block from `response.output_item.added` / `function_call_arguments.done` /
   `output_item.done`, and `stream.rs::replay_sse_events` only reads
   `choices[0].delta.content` for chat completions, so `delta.tool_calls` is
   lost too. Every request goes through the streaming path
   (`live_complete_streaming_with` sets `stream: true` for all APIs), so this
   affects every provider. The interactive shell reports it as
   "the model returned no text".
2. **No Anthropic SSE decoder exists.** `anthropic-messages` bodies are sent
   with `stream: true`, and the reply is an SSE stream nobody parses, so
   Anthropic models return nothing. Google, Bedrock and Mistral streams are in
   the same position.
3. **Streaming is replay, not live.** `send_provider_body` calls
   `response.into_string()`, buffering the entire body before a single event
   is produced. The agent loop then re-emits the recorded events after the
   fact. The user sees a spinner for the whole response and then a wall of
   text. This is the single biggest "feels amateur" factor next to Claude
   Code and Codex, which paint tokens as they arrive.
4. **The davinci transcript ignores text until `MessageEnd`.** Even with a
   live stream, `davinci_session.rs::apply` only pushes prose when the
   message ends, and thinking deltas are discarded entirely.

### Interface gaps

5. **No markdown rendering.** `Entry::Prose` is wrapped plain text, so
   headings, emphasis, lists, inline code and fenced code blocks print as raw
   markdown syntax. The legacy chrome had a renderer; davinci does not.
6. **The startup emblem never shows.** The opening block pushes
   "loaded N context files" into the transcript, so the empty state that
   draws the Vitruvian mark (`1a`) is skipped on every launch.
7. **`/model` opens on a 1,327-row alphabetical list** starting at
   `amazon-bedrock/…` with no credential, and the row for the current model
   is only selected if it happens to sort first. Models the user can actually
   use are buried.
8. **Reasoning is invisible.** Codex reasoning summaries and Anthropic
   thinking arrive as `thinking_delta` events and are dropped; only a
   "thinking with minimal effort" label on the working line survives.

### Engineering gaps versus Claude Code and Codex CLI

Not defects, but absent capabilities that define the competitive set:

- Tool approval / permission modes (read-only, workspace-write, full-access;
  per-tool allow/deny rules; "always allow" memory).
- Plan mode and a task/todo ledger the model maintains.
- Native subagents (parallel workers with their own context).
- A native MCP client (today MCP arrives only through the JavaScript
  `pi-mcp-adapter` extension, which needs Node).
- Hooks the user configures (pre/post tool, on stop, notifications).
- Background shell jobs with notification on completion.
- Web fetch / web search tools.
- Diff review with syntax highlighting; collapsible tool output.
- Cost/usage reporting per session (`/cost`, `/status`) with cache figures.
- Extension host is rebuilt per message in the legacy path (performance).

## Sub-projects

Each gets its own spec, plan and implementation cycle. Order matters: nothing
later is worth building on top of a turn that drops tool calls.

| # | Sub-project | Outcome |
|---|---|---|
| 1 | **Turns that are real** (this document) | Tool calls parsed on every provider; tokens painted as they arrive; markdown transcript; thinking shown; startup and `/model` fixed. |
| 2 | **Trust and control** (`2026-09-01-trust-and-control-design.md`) | Permission modes, per-tool approval prompts in the davinci language, "always allow" persisted in `.pi/`, `--sandbox` presets. |
| 3 | **Tools that compete** (`2026-09-01-tools-that-compete-design.md`) | Background shell jobs, `web_fetch`/`web_search`, todo ledger, notebook-aware read/edit, collapsible tool output, syntax-highlighted diffs. Landed 2026-09-02 on `rust-rewrite`. |
| 4 | **Native MCP client** | stdio + streamable-HTTP MCP servers from `~/.pi/agent/mcp.json`, tools and resources exposed natively, `/mcp` sheet. |
| 5 | **Plan mode and subagents** | Plan/act toggle, native parallel subagents with scoped tools and their own transcript pane. |
| 6 | **Hooks and observability** | User hooks, `/cost` and `/status`, session cost ledger, structured logs. |

## Phase 1 design: turns that are real

### 1. Incremental provider decoding (`pi-ai`)

**Component: `stream_decoder` module.** One decoder per wire format, each
fed one parsed SSE payload at a time and producing `AssistantMessageEvent`s
as it goes. All three carry a partial `AssistantMessage` that grows with
every event, exactly as the TypeScript `processResponsesStream`,
`openai-completions.ts` and `anthropic-messages.ts` do.

- `ResponsesDecoder` — `openai-responses`, `azure-openai-responses`,
  `openai-codex-responses`. Slots keyed by `output_index` for `message`,
  `reasoning`, `function_call` and `custom_tool_call` items. Handles
  `response.created`, `output_item.added`, `output_text.delta`,
  `refusal.delta`, `reasoning_summary_text.delta`,
  `reasoning_summary_part.done`, `reasoning_text.delta`,
  `function_call_arguments.delta/.done`, `custom_tool_call_input.delta/.done`,
  `output_item.done`, `response.completed/.done/.incomplete` (usage, status
  to stop reason, `toolUse` when a tool call is present), `response.failed`
  and `error`. Tool call ids are `call_id|item_id` as in TS. Codex event
  normalisation (`mapCodexEvents`) is applied in front of it.
- `CompletionsDecoder` — `openai-completions` and every OpenAI-compatible
  provider. Reads `choices[0].delta.content`, `delta.reasoning_content` /
  `delta.reasoning`, `delta.tool_calls[]` accumulated by `index` (id, name,
  argument fragments), `finish_reason`, and a trailing `usage` chunk.
- `AnthropicDecoder` — `anthropic-messages`. `message_start` (usage,
  model), `content_block_start` (text / thinking / tool_use),
  `content_block_delta` (`text_delta`, `thinking_delta`,
  `input_json_delta`, `signature_delta`), `content_block_stop`,
  `message_delta` (stop reason, output usage), `message_stop`, `error`.
- Anything else (`google-*`, `bedrock-converse-stream`,
  `mistral-conversations`) is requested **without** `stream: true` and goes
  through the existing complete-response parser, with events synthesised by
  `events_from_complete`. Tool calls then work everywhere; streaming arrives
  for those APIs in a later sub-project.

`replay_sse_events` and `replay_codex_events` keep their signatures and
become thin wrappers that feed a decoder, so existing fixtures still pass.

**Component: live transport.** `live_complete_streaming_with` gains a sink:

```rust
pub fn live_complete_streaming_with_sink(
    model, messages, auth, system, tools, options,
    on_event: &mut dyn FnMut(&AssistantMessageEvent),
) -> Result<(AssistantMessage, Vec<AssistantMessageEvent>), String>
```

The old function calls it with a no-op sink. The SSE path reads
`response.into_reader()` through a `BufReader`, splits SSE frames on blank
lines, feeds the decoder, and calls the sink for every event. Between frames
it checks `StreamOptions::abort_signal`; when set it drops the connection and
finishes the message with `StopReason::Aborted`. HTTP-level failures still
surface from `send` and go through `retry_provider_request` unchanged; a
failure after the first byte is reported as `StopReason::Error` with the
provider text, never retried mid-stream (matches TS).

The Codex WebSocket path (`codex_ws.rs::read_codex_events`) feeds the same
decoder frame by frame and calls the sink, instead of collecting a corpus and
replaying it.

### 2. Live emission through the agent loop (`pi-agent`)

`CompleteOutput` gains `streamed_live: bool`. `Agent` gains
`emit_live(&self, AgentEvent)`, which forwards to `event_sink` only. The
provider closure in `complete_prompt_with_host` streams through
`live_complete_streaming_with_sink`; on the first event it emits
`MessageStart` with the partial message, then one `MessageUpdate` per event.
When the loop receives an output with `streamed_live == true` it records
`MessageStart` and the updates into the returned event list without touching
the sink, and emits `MessageEnd` normally. Ordering seen by a sink is
therefore `MessageStart, MessageUpdate…, MessageEnd`, the same as today but
live. Print, JSON and RPC modes inherit real streaming from this with no
change of their own.

### 3. Live transcript in the davinci shell (`pi-tui`, `davinci_session.rs`)

- `Turn` tracks the index of the prose entry being streamed. `TextStart`
  pushes a gap and an empty `Entry::Prose`; `TextDelta` appends;
  `TextEnd` and `MessageEnd` set the final trimmed text. A message that was
  never streamed (offline stub, non-streaming API) still works through the
  `MessageEnd` path.
- New `Entry::Thinking { text, live }`. While live, the last three wrapped
  lines of the reasoning are drawn in muted under a `⟐ reasoning` row; once
  text starts or the message ends it collapses to one row:
  `⟐ reasoned for 4s · <first sentence, clipped>`. A `/settings` toggle
  (`Show reasoning`) keeps the full text expanded instead.
- `Entry::Prose` renders through a new `views/markdown.rs` built on
  `pulldown-cmark` (already a dependency): headings in bold text colour,
  emphasis, inline code in verdigris, fenced code blocks indented two
  columns behind a single border-colour rule with the language noted on the
  rule, bullet lists with `·`, ordered lists with their numbers, block
  quotes behind a muted rule, links as `text (url)`, thematic breaks as a
  hair rule, tables as aligned plain rows. Prose still wraps at the 74-column
  measure. Partial markdown (mid-stream) renders on every frame; the parser
  is tolerant of unclosed fences.
- The working line keeps its spinner and token meter; the token count now
  climbs from real deltas.

### 4. Startup and `/model`

- The opening block's "loaded …" line, the models-scoped note and the
  changelog pointer move into `Model::startup` as `found: Vec<String>` and
  are drawn by `views/startup.rs` under the emblem, so a fresh session opens
  on the `1a` screen. Warnings (untrusted project, models.json errors) stay
  in the transcript because they need the attention glyph.
- `open_models_sheet` orders rows: current model first, then models with a
  credential grouped by provider, then the rest; `catalog_index` points at
  the current model.

### Diagnostics

`PI_AI_TRACE=1` writes one line per request, frame and failure to stderr;
`PI_AI_TRACE=<path>` appends them to a file, which an interactive session
needs because its stderr is the screen. This is how the `call_id` defect
below was found and is the first tool to reach for when a turn misbehaves.

### Error handling

- Decoder receives malformed JSON: the frame is skipped; a stream that ends
  without a terminal event still yields `Done` with whatever was collected
  (matches today's replay behaviour).
- Provider `error` / `response.failed` frames become `StopReason::Error`
  with the provider's message, shown by the existing "the request failed"
  rows.
- Abort mid-stream: message closes with `StopReason::Aborted`; the
  recovery sheet already handles that reason.

### Testing

Fixture-only, no network, as the repository requires.

- Each decoder has unit tests fed from SSE corpora written from the
  TypeScript reference shapes: text only; text + one tool call; two tool
  calls; reasoning then text; refusal; usage and stop reason; `error`
  frame; truncated stream. Tool-call ids, argument JSON and stop reasons are
  asserted exactly.
- `replay_sse_events` / `replay_codex_events` existing tests keep passing.
- A `live_complete_streaming_with_sink` test against a loopback
  `std::net::TcpListener` that writes SSE frames with delays, asserting the
  sink receives the first `TextDelta` before the server has sent the last
  frame (proves incremental delivery) and that an abort flag stops the read.
- Agent loop test: a closure that calls `emit_live` and returns
  `streamed_live: true` produces exactly one `MessageStart`, N updates and
  one `MessageEnd` in the returned events and the sink sees no duplicates.
- Davinci: `apply` tests for `TextStart/Delta/End` building one prose entry;
  thinking entry collapse; markdown renderer snapshot tests (headings, lists,
  fenced code, inline code, unclosed fence); startup fixture `1a` unchanged;
  models sheet ordering test.
- End to end: the headless ConPTY harness in the session scratchpad runs a
  real turn and samples the grid every second; the check passes only if
  prose appears before the working line disappears.

### Out of scope for phase 1

Permission prompts, MCP, subagents, plan mode, hooks, syntax highlighting,
tool-output previews, Google/Bedrock/Mistral streaming decoders. Each is
listed above with the sub-project it belongs to.
