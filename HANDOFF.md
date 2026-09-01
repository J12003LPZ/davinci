# HANDOFF — competitive harness, phase 1 landed

## Goal

Turn the Rust `pi` rewrite into a harness that competes with Claude Code and
Codex CLI on engineering and interface. The roadmap and the phase 1 design are
in `docs/superpowers/specs/2026-09-01-competitive-harness-roadmap.md`. Phase 1
("turns that are real") is implemented in the working tree; phases 2–6 are
listed there and not started.

## State (2026-09-01)

- Branch `rust-rewrite`, HEAD `fd349ef`. Everything since is uncommitted:
  the previous session's thinking-cycle / model-persistence / caret work and
  this session's phase 1. `cargo fmt`, `cargo clippy --workspace --all-targets
  -- -D warnings` and `cargo test --workspace` are green.
- Release binary copied to `~/.cargo/bin/pi.exe` and `~/.cargo/bin/davinci.exe`
  (see the memory note: `cargo install` does not work in this repo).

## What phase 1 changed

- `pi-ai/src/stream_decoder*.rs`: incremental decoders for the Responses API
  (Codex), chat completions and Anthropic messages, fed one SSE frame at a
  time; `replay_sse_events` / `replay_codex_events` are wrappers over them.
  Before this, every streaming provider dropped tool calls and Anthropic
  streams were not parsed at all.
- `pi-ai/src/stream.rs`: `live_complete_streaming_with_sink` reads the body
  on a reader thread and hands every event to the sink as it arrives; the
  abort flag (`StreamOptions::abort_signal`) is checked every 40ms. APIs
  without a decoder (Google, Bedrock, Mistral) are requested without
  `stream: true` and their events synthesised. Tool-call ids are persisted as
  `call_id|item_id` and only the `call_id` half is replayed (`responses_call_id`);
  the joined form was rejected by OpenAI as too long.
- `pi-ai/src/trace.rs`: `PI_AI_TRACE=1` (stderr) or `PI_AI_TRACE=<file>`
  logs every request, frame type and failure. Use it first when a turn
  misbehaves.
- `pi-agent`: `CompleteOutput::streamed_live` + `Agent::emit_live`; the loop
  records live-streamed `MessageStart`/`MessageUpdate` without resending.
  Non-retryable provider errors fail at once instead of being retried
  silently. Provider failures after a tool call are surfaced (they used to
  read as "the model returned no text").
- davinci: text streams into the transcript as it arrives; reasoning shows
  live then collapses to `⟐ reasoned 4s · first sentence`
  (`hideThinkingBlock` hides it); prose renders as markdown
  (`views/markdown.rs`); the startup emblem shows again (the "loaded …" line
  moved to `Startup::found`); `/model` opens on the current model with
  credentialed rows first.

## Verified

- Headless ConPTY harness (`drive.py`, `peek.py`, `real2.py`, `abort.py` in the
  session scratchpad, recipe in the memory note "driving the davinci TUI
  headlessly"): a real Codex turn reads `Cargo.toml`, streams a markdown
  reply, and `esc` interrupts within half a second with the partial reply kept.
- `pi -p "Read Cargo.toml …"` and `--mode json` complete the tool round trip.

## Next

Phase 2 in the roadmap (permission modes and tool approval) is the
recommended next sub-project; phases 3–6 follow. Commit the working tree in
two commits (previous session's TUI fixes, then phase 1) before starting.
