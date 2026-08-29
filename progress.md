# pi Rust rewrite progress

**Complete: 82%**

Pinned spec: `vendor/pi` @ `853a80d26c90a14c1886f0ebb8ffaae133ca2185`.

## What landed

- Workspace on rustc **1.83.0**, edition 2021, pinned clap/uuid/tempfile/thiserror/ureq/url/rustls/webpki/zeroize (+ sha2 for PKCE).
- `vendor/pi` kept as the TypeScript behavioral spec (not deleted).
- **pi-protocol**: TS CBOR subset, 4-byte framing, hello/request/response, error strings locked to TS tests.
- **pi-session / pi-session-sqlite**: JSONL v4, v3→v4 migrate, cwd-encoded discovery (`~/.pi`, `--session-dir`), continue/resume/fork/clone, FTS, writer leases, `001_initial.sql`.
- **pi-ai**: all TS provider IDs + catalogs, all 10 `KnownApi` URLs/bodies, SSE + `live_stream`/`events_from_complete`, OpenAI/Anthropic/Google/Bedrock tool-call parse, OAuth authorize URLs (Anthropic/Codex/OpenRouter/xAI/Kimi/GitHub/Radius) + PKCE + `parse_authorization_input` + fixture exchange, device-code poller. Tests never call the network.
- **pi-agent**: TS agent loop events, tool execution, retry, compaction, skills, templates, context files, steer/follow-up queues, built-in tools.
- **pi-tui**: ChatChrome, overlays, SettingsList, fullscreen/mouse, editor, markdown, fuzzy, TuiBox, ScrollView, LaTeX symbols, Kitty/iTerm image wrappers.
- **pi-client / pi-server**: Unix + TCP + memory, handshake timeout, request correlation, leases.
- **pi-telemetry / pi-evals**: TS contracts + fixture harness.
- **pi-coding-agent `pi`**: flags/subcommands, slash (including `/login` authorize URL), RPC, settings/trust, **HTML export using TS `template.html`/`template.css`/`template.js` + marked/highlight**, extension manifest + event bus, `pi update` writes builtin catalogs to `~/.pi/agent/models`.
- **pi-parity**: six required corpora + optional `--parallel-run` / `--diff-jsonl`.

Gates on this slice: `cargo test --workspace`, `cargo fmt --check`, `clippy --workspace --all-targets -- -D warnings` green on 1.83.

## Remaining product gaps

- **JS extension module runtime**: event bus + JSON manifests exist; TypeScript/JavaScript extension modules are not executed.
- **Live OAuth callback HTTP server**: authorize URLs and fixture `pi-fixture-` / `PI_OAUTH_FIXTURE` exchange work; the Node `createServer` loopback is not bound in Rust.
- **Codex websocket / full per-event SSE adapters** for every TS stream quirk (tests stay fixture-only).
- **Full LaTeX parser** and **Kitty image decode** (symbol table + protocol wrappers only).
- **npm/bun self-update of the `pi` binary**. `pi update` refreshes local model catalogs.

## Next crate/module

OAuth loopback callback server (fixture-safe) and JS extension subprocess runner when Node is present; then Codex websocket fixture corpus.
