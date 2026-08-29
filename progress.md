# pi Rust rewrite progress

**Complete: 98%**

Pinned spec: `vendor/pi` @ `853a80d26c90a14c1886f0ebb8ffaae133ca2185`.

## What landed

- Workspace on rustc **1.83.0**, edition 2021, pinned clap/uuid/tempfile/thiserror/ureq/url/rustls/webpki/zeroize (+ sha2 for PKCE).
- `vendor/pi` kept as the TypeScript behavioral spec (not deleted).
- **pi-protocol**: TS CBOR subset, 4-byte framing, hello/request/response, error strings locked to TS tests.
- **pi-session / pi-session-sqlite**: JSONL v4, v3→v4 migrate, cwd-encoded discovery (`~/.pi`, `--session-dir`), continue/resume/fork/clone, FTS, writer leases, `001_initial.sql`.
- **pi-ai**: all TS provider IDs + catalogs, all 10 `KnownApi` URLs/bodies, SSE + `live_stream`/`events_from_complete`, OpenAI/Anthropic/Google/Bedrock tool-call parse, OAuth authorize URLs + PKCE + fixture/live token exchange, device-code poller, loopback callback, Codex websocket/SSE fixture corpus. Tests never call live LLM or OAuth HTTP.
- **pi-agent**: TS agent loop events, tool execution, retry, compaction, skills, templates, context files, steer/follow-up queues, built-in tools.
- **pi-tui**: ChatChrome, overlays, SettingsList with value cycling, fullscreen/mouse, editor, markdown, fuzzy, TuiBox, ScrollView, LaTeX, Kitty CSI-u + **encodeKitty / deleteKittyImage** (`a=T,f=100,q=2`, 4096-byte chunks), iTerm wrappers, InteractiveSession (alt screen, bracketed paste, Kitty keyboard query, raw-mode keys/mouse/paste), **tool execution cards** (TS `formatToolExecution` + 10-line preview), **transcript Kitty-image placements**, **slash/path/`@` autocomplete** with Tab accept, **double-escape** tree/fork/none (500ms), `!` bash lines, Shift+Enter newline, `/model` `/settings` `/tree` overlays. Unit tests do not require a TTY.
- **pi-client / pi-server**: Unix + TCP + memory, handshake timeout, request correlation, leases.
- **pi-telemetry / pi-evals**: TS contracts + fixture harness.
- **pi-coding-agent `pi`**: flags/subcommands, slash (including `/login`, `/model`, `/settings` cycling), RPC, settings/trust (`double_escape_action`, `autocomplete_max_visible`), HTML export using TS templates, JS/TS extension runner with virtual `@earendil-works/*` packages, `pi update --self` native copy or npm/pnpm/yarn/bun argv, `pi update --models` catalogs.
- **pi-parity**: six required corpora + optional `--parallel-run` / `--diff-jsonl`.

Gates on this slice: `cargo test --workspace`, `cargo fmt --check`, `clippy --workspace --all-targets -- -D warnings` green on 1.83.

## Remaining product gaps

- **InteractiveMode extras**: first-time setup wizard, login-dialog TUI, `/tree` filter modes, scoped-models selector UI, and mermaid/custom-message widgets are still thinner than TypeScript. The previously listed chrome (tool cards, Kitty placements, autocomplete, double-escape, settings cycling) is ported.
