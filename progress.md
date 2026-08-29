# pi Rust rewrite progress

**Complete: 95%**

Pinned spec: `vendor/pi` @ `853a80d26c90a14c1886f0ebb8ffaae133ca2185`.

## What landed

- Workspace on rustc **1.83.0**, edition 2021, pinned clap/uuid/tempfile/thiserror/ureq/url/rustls/webpki/zeroize (+ sha2 for PKCE).
- `vendor/pi` kept as the TypeScript behavioral spec (not deleted).
- **pi-protocol**: TS CBOR subset, 4-byte framing, hello/request/response, error strings locked to TS tests.
- **pi-session / pi-session-sqlite**: JSONL v4, v3→v4 migrate, cwd-encoded discovery (`~/.pi`, `--session-dir`), continue/resume/fork/clone, FTS, writer leases, `001_initial.sql`.
- **pi-ai**: all TS provider IDs + catalogs, all 10 `KnownApi` URLs/bodies, SSE + `live_stream`/`events_from_complete`, OpenAI/Anthropic/Google/Bedrock tool-call parse, OAuth authorize URLs (Anthropic/Codex/OpenRouter/xAI/Kimi/GitHub/Radius) + PKCE + `parse_authorization_input` + fixture exchange, device-code poller, loopback callback, Codex websocket/SSE fixture corpus, **live token POST** matching TS bodies (Anthropic JSON, Codex/Radius form-urlencoded, OpenRouter JSON) and TS error strings. Tests never call live LLM or OAuth HTTP (`pi-fixture-` / `PI_OAUTH_FIXTURE`).
- **pi-agent**: TS agent loop events, tool execution, retry, compaction, skills, templates, context files, steer/follow-up queues, built-in tools.
- **pi-tui**: ChatChrome, overlays, SettingsList, fullscreen/mouse (`ENABLE_ALL_MOTION_MOUSE` / `DISABLE_MOUSE`), editor, markdown, fuzzy, TuiBox, ScrollView, LaTeX parser, Kitty CSI-u printable decode + graphics header parse, iTerm wrappers, **InteractiveSession** (alt screen, autowrap, bracketed paste `\x1b[?2004h`, Kitty keyboard query `\x1b[>7u\x1b[?u\x1b[c`, raw-mode key/mouse/paste, `/model` `/settings` `/tree` SelectList overlays, ctrl+p/ctrl+t/escape). Unit tests do not require a TTY.
- **pi-client / pi-server**: Unix + TCP + memory, handshake timeout, request correlation, leases.
- **pi-telemetry / pi-evals**: TS contracts + fixture harness.
- **pi-coding-agent `pi`**: flags/subcommands, slash (including `/login`, `/model` selector), RPC, settings/trust, HTML export using TS templates, extension manifest + event bus, **Node JS/TS extension runner** (jiti when present, `typescript.transpileModule` or import rewrite otherwise; virtual `@earendil-works/*` / `@mariozechner/*` / typebox files; factory `pi.on` / `registerTool` / `registerCommand`) plus manifest `command` tools, `pi update --self` copies the native binary or runs TS npm/pnpm/yarn/bun argv (`detectInstallMethod` / `getSelfUpdateCommandForMethod` / bun-binary download URL; `PI_SELF_UPDATE_DRY_RUN` prints without exec), `pi update --models` writes builtin catalogs.
- **pi-parity**: six required corpora + optional `--parallel-run` / `--diff-jsonl`.

Gates on this slice: `cargo test --workspace`, `cargo fmt --check`, `clippy --workspace --all-targets -- -D warnings` green on 1.83.

## Remaining product gaps

- **InteractiveMode chrome**: TS still has richer live tool cards, transcript Kitty-image placements, autocomplete, double-escape tree, and the full settings-selector value cycling UI. Rust covers raw-mode sequences, `/model` `/settings` `/tree` overlays, and keybindings; those remaining chrome surfaces are not product-equivalent yet.
