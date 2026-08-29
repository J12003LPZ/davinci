# pi Rust rewrite progress

**Complete: 99%**

Pinned spec: `vendor/pi` @ `853a80d26c90a14c1886f0ebb8ffaae133ca2185`.

## What landed

- Workspace on rustc **1.83.0**, edition 2021, pinned clap/uuid/tempfile/thiserror/ureq/url/rustls/webpki/zeroize (+ sha2 for PKCE).
- `vendor/pi` kept as the TypeScript behavioral spec (not deleted).
- **pi-protocol**: TS CBOR subset, 4-byte framing, hello/request/response, error strings locked to TS tests.
- **pi-session / pi-session-sqlite**: JSONL v4, v3→v4 migrate, cwd-encoded discovery (`~/.pi`, `--session-dir`), continue/resume/fork/clone, FTS, writer leases, `001_initial.sql`.
- **pi-ai**: all TS provider IDs + catalogs, all 10 `KnownApi` URLs/bodies, SSE + `live_stream`/`events_from_complete`, OpenAI/Anthropic/Google/Bedrock tool-call parse, OAuth authorize URLs + PKCE + fixture/live token exchange, device-code poller, loopback callback, Codex websocket/SSE fixture corpus. Tests never call live LLM or OAuth HTTP.
- **pi-agent**: TS agent loop events, tool execution, retry, compaction, skills, templates, context files, steer/follow-up queues, built-in tools.
- **pi-tui**: ChatChrome, overlays, SettingsList with value cycling, fullscreen/mouse, editor, markdown, fuzzy, TuiBox, ScrollView, LaTeX, Kitty CSI-u + **encodeKitty / deleteKittyImage** (`a=T,f=100,q=2`, 4096-byte chunks), iTerm wrappers, InteractiveSession (alt screen, bracketed paste, Kitty keyboard query, raw-mode keys/mouse/paste), **tool execution cards** (TS `formatToolExecution` + 10-line preview), **transcript Kitty-image placements**, **slash/path/`@` autocomplete** with Tab accept, **double-escape** tree/fork/none (500ms), `!` bash lines, Shift+Enter newline, `/model` `/settings` `/tree` overlays, **first-time setup wizard** (logo, welcome copy, Dark/Light, analytics opt-in, ↑↓/j/k, enter continue/finish, escape skip), **login-dialog TUI** (OSC-8, Cmd/Ctrl+click, device code, manual input, waiting/progress/info, Escape → `Login cancelled`), **`/tree` filter modes** (`default` / `no-tools` / `user-only` / `labeled-only` / `all`, ctrl+d/t/u/l/a/o, `├─`/`└─` gutters, status labels, search), **scoped-models selector** (`EnabledIds` null=all, toggle/enableAll/clearAll/move/getSortedIds, `/scoped-models`, ctrl+s persist), **Mermaid transformer** (TS-locked flowchart Unicode art, warning `Mermaid diagram not rendered: …`, off/final/streaming + thinking skip), **custom-message** purple box with `[type]` label and expand/collapse. Unit tests do not require a TTY.
- **pi-client / pi-server**: Unix + TCP + memory, handshake timeout, request correlation, leases.
- **pi-telemetry / pi-evals**: TS contracts + fixture harness.
- **pi-coding-agent `pi`**: flags/subcommands, slash (including `/login` dialog, `/model`, `/settings` cycling, `/tree` filters, `/scoped-models`), RPC, settings/trust (`double_escape_action`, `autocomplete_max_visible`, `treeFilterMode`, `markdown.mermaid`, `enableAnalytics`/`trackingId`, `enabledModels`), HTML export using TS templates, JS/TS extension runner with virtual `@earendil-works/*` packages, `pi update --self` native copy or npm/pnpm/yarn/bun argv, `pi update --models` catalogs. First-time setup gate matches TS (`PI_EXPERIMENTAL=1`, no `PI_CODING_AGENT_DIR`, missing `settings.json`).
- **pi-parity**: six required corpora + optional `--parallel-run` / `--diff-jsonl`.

Gates on this slice: `cargo test --workspace`, `cargo fmt --check`, `clippy --workspace --all-targets -- -D warnings` green on 1.83.

## Remaining product gaps

- **Tree extras**: label edit, fold/unfold branch jumps, copy-selected, and horizontal viewport panning are still thinner than TypeScript `tree-selector.ts`.
- **Login / first-time polish**: no `openBrowser` spawn; first-time uses the current theme instead of a live OSC appearance probe.
- **Custom-message extensions**: default purple-box renderer is ported; extension-provided `MessageRenderer` components are not.
