# pi Rust rewrite progress

**Complete: 72%**

Pinned spec: `vendor/pi` @ `853a80d26c90a14c1886f0ebb8ffaae133ca2185`.

## What landed

- Workspace on rustc **1.83.0**, edition 2021, pinned clap/uuid/tempfile/thiserror/ureq/url/rustls/webpki/zeroize.
- `vendor/pi` kept as the TypeScript behavioral spec (not deleted).
- **pi-protocol**: TS CBOR subset, 4-byte framing, hello/request/response, error strings locked to TS tests.
- **pi-session / pi-session-sqlite**: JSONL v4, v3→v4 migrate, cwd-encoded discovery (`~/.pi`, `--session-dir`), continue/resume/fork/clone, FTS, writer leases, `001_initial.sql`.
- **pi-ai**: all TS provider IDs + published 0.84.4 catalogs, auth.json / env / OAuth storage, fixture OAuth refresh + device-code poller, SSE stream lifecycle + usage/cost, OpenAI/Anthropic/Google tool-call parse, fixture replay only.
- **pi-agent**: TS agent loop events (`agent_start` … `tool_execution_end`), tool execution, retry, compaction, skills, prompt templates, AGENTS.md/CLAUDE.md, steer/follow-up queues, built-in read/write/edit/bash/grep/find/ls.
- **pi-tui**: `Component::render(width)`, ChatChrome transcript, fullscreen alt-buffer, editor, keybindings, markdown, fuzzy selectors, SGR mouse, themes, TuiBox, ScrollView.
- **pi-client / pi-server**: Unix + TCP + memory transports, handshake timeout, request correlation, leases via sqlite backend.
- **pi-telemetry / pi-evals**: TS contracts + fixture harness (no live models).
- **pi-coding-agent `pi`**: flags/subcommands from `args.ts` / `main.ts` (print, json, rpc, interactive, auth, install/remove/update/list/config, export), slash commands, RPC session/model commands, settings/trust, HTML export (`#app`/`#sidebar`/`#messages`), extension manifest discovery.
- **pi-parity**: required corpora (writer-leases, session entries, protocol hello/CBOR, assistant+usage, agent events, print/RPC) and optional `--parallel-run` / `--diff-jsonl`.

Gates on this slice: `cargo test --workspace`, `cargo fmt --check`, `clippy --workspace --all-targets -- -D warnings` green on 1.83.

## Remaining product gaps

- TypeScript **JS extension host** (event bus, UI primitives). Rust loads `pi.extension.json` / tool names; it does not execute TS/JS extension modules.
- **Browser OAuth login** for providers that require a live authorize redirect. Existing `auth.json` / env / API keys / fixture refresh work.
- **Per-API live streaming** for every TS provider (Bedrock converse-stream, Codex websocket, Mistral conversations, etc.). Complete+tool-call parse covers OpenAI/Anthropic/Google shapes; tests never call the network.
- Interactive **overlays / Kitty images / LaTeX** from `packages/tui` are not pixel-identical to TS InteractiveMode.
- **npm/bun self-update** for `pi update`. Local settings/extension catalog update only.
- HTML export is static `#app`/`#sidebar`/`#messages` (escaped). TS `template.js` sidebar interactivity is not ported.

## Next crate/module

`pi-ai` remaining provider stream/complete adapters from `vendor/pi/packages/ai/src/providers` (fixture SSE corpora), then deepen `pi-tui` InteractiveMode chrome.
