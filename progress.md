# pi Rust rewrite progress

**Complete: 90%**

Pinned spec: `vendor/pi` @ `853a80d26c90a14c1886f0ebb8ffaae133ca2185`.

## What landed

- Workspace on rustc **1.83.0**, edition 2021, pinned clap/uuid/tempfile/thiserror/ureq/url/rustls/webpki/zeroize (+ sha2 for PKCE).
- `vendor/pi` kept as the TypeScript behavioral spec (not deleted).
- **pi-protocol**: TS CBOR subset, 4-byte framing, hello/request/response, error strings locked to TS tests.
- **pi-session / pi-session-sqlite**: JSONL v4, v3→v4 migrate, cwd-encoded discovery (`~/.pi`, `--session-dir`), continue/resume/fork/clone, FTS, writer leases, `001_initial.sql`.
- **pi-ai**: all TS provider IDs + catalogs, all 10 `KnownApi` URLs/bodies, SSE + `live_stream`/`events_from_complete`, OpenAI/Anthropic/Google/Bedrock tool-call parse, OAuth authorize URLs (Anthropic/Codex/OpenRouter/xAI/Kimi/GitHub/Radius) + PKCE + `parse_authorization_input` + fixture exchange, device-code poller, **loopback `createServer` callback** (Anthropic/Codex/OpenRouter/Radius paths, TS HTML titles/error strings, `127.0.0.1` bind; tests use port 0 and never hit live OAuth HTTP), **Codex websocket/SSE fixture corpus** (`websocket_connection_limit_reached` SSE fallback, `response.created` / `output_text.delta` / `completed` event names). Tests never call live LLM or OAuth providers.
- **pi-agent**: TS agent loop events, tool execution, retry, compaction, skills, templates, context files, steer/follow-up queues, built-in tools.
- **pi-tui**: ChatChrome, overlays, SettingsList, fullscreen/mouse, editor, markdown, fuzzy, TuiBox, ScrollView, **LaTeX parser** (TS symbol/script/frac/sqrt/matrix/cases/align tables, unsupported syntax returns `None`), **Kitty CSI-u printable decode** + graphics header parse, iTerm wrappers.
- **pi-client / pi-server**: Unix + TCP + memory, handshake timeout, request correlation, leases.
- **pi-telemetry / pi-evals**: TS contracts + fixture harness.
- **pi-coding-agent `pi`**: flags/subcommands, slash (including `/login` authorize URL, fixture-code exchange, optional `PI_OAUTH_WAIT` loopback), RPC, settings/trust, HTML export using TS templates, extension manifest + event bus, **Node subprocess JS extension runner** (factory `pi.on` / `registerTool` / `registerCommand`, `tool_call` block results) plus manifest `command` tools, `pi update --self` copies the binary to `~/.pi/bin/pi` with TS conflict strings, `pi update --models` writes builtin catalogs.
- **pi-parity**: six required corpora + optional `--parallel-run` / `--diff-jsonl`.

Gates on this slice: `cargo test --workspace`, `cargo fmt --check`, `clippy --workspace --all-targets -- -D warnings` green on 1.83.

## Remaining product gaps

- **TypeScript/jiti extension modules**: JS factories run via Node when present. TS sources and virtual `@earendil-works/*` packages from the jiti loader are not executed in-process.
- **npm/bun/pnpm/yarn global self-update**: Rust installs copy `pi` to `~/.pi/bin/pi`. The TS package-manager CLI that shells out to npm/pnpm/yarn/bun is not executed.
- **InteractiveMode raw-mode TUI**: interactive CLI is still a multi-turn readline loop, not the full TS overlay/Kitty-image session.
- **Live OAuth token HTTP**: loopback callback + `pi-fixture-` / `PI_OAUTH_FIXTURE` exchange work; live token POST remains fixture-gated.

## Next crate/module

In-process jiti-equivalent for TypeScript extensions (or a documented Node+jiti sidecar that resolves virtual `@earendil-works/*` packages), then InteractiveMode raw-mode overlays.
