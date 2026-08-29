# Rust rewrite progress

**Complete: 100%**

TypeScript spec: `vendor/pi` @ `853a80d26c90a14c1886f0ebb8ffaae133ca2185` (`@earendil-works/pi-*` 0.84.4).
Toolchain: Rust **1.83.0**, no `edition2024`, no `unsafe`. Remote rust-rewrite placeholders were merged (ours strategy) and harvested for retry/permission shapes; live HTTP is implemented with `ureq` rather than those stubs.

TypeScript remains in `vendor/pi` as the behavioral reference only. Rust under `crates/*` is the product.

## What landed

- Vendored TypeScript `pi` (reference only).
- Workspace crates mapping every `vendor/pi/packages/*` plus session-backends and Rust-only `pi-parity`: `pi-telemetry`, `pi-protocol`, `pi-session`, `pi-session-sqlite`, `pi-ai`, `pi-agent`, `pi-tui`, `pi-client`, `pi-server`, `pi-evals`, `pi-coding-agent` (`pi` binary), `pi-parity`. Each crate has TypeScript-fixture tests.
- **pi-ai**: 39 provider catalogs (1290 models), env-key map, `auth.json`, OAuth authorize URLs + token fixture parse + refresh (fixture or live token URL), request bodies for anthropic-messages / openai-completions / openai-responses / google-generative-ai / google-vertex / pi-messages / openrouter-images / bedrock-converse-stream / mistral-conversations, SSE corpora, usage/cost, retry classification, live `ureq` SSE when credentials + `allow_network` (tests stay fixture-only; `PI_DISABLE_NETWORK=1` blocks live calls). Error string `No API key for provider: {id}`.
- **pi-session / sqlite**: JSONL v4 header, v3→v4 migrate, continue/resume/fork/clone/fork-from-entry, entries/tree/stats/leaf, `~/.pi` + `--session-dir`. SQLite applies TypeScript `001_initial.sql` and implements sequences, stats, branch tips/entries, lanes + moves, records, facts, FTS5, fence leases, TS lease timing errors.
- **pi-agent**: compaction, skills, templates, context files, builtin tools read/write/edit/bash/ls/grep/find/powershell, Ask/Allow/Deny permission policies (stdin Ask), retry, steer/follow-up queue modes, thinking cycle, fixture-driven loop that executes tool calls.
- **pi-tui**: `Component::render(width)`, wrap, `wrapTextWithAnsi`, editor, keybindings, markdown, mouse/alt-buffer, raw `stty` input helpers, themes, selectors, Container/Overlay/ChatView, differential `DiffScreen` (DEC synchronized output), SGR 1006 mouse + overlay hit-test, Box/VStack/HStack/ScrollView/Input/SettingsList.
- **pi-protocol/client/server**: Unix+TCP+memory, handshake timeout, leases, request correlation, CBOR vectors. Server implements list/create/attach/detach/prompt/steer/abort/set_model/set_thinking.
- **pi-evals / pi-parity**: fixture evals + suite runner + required golden corpora. Optional `--parallel-run` / `--diff-jsonl`.
- **pi binary**: flags/subcommands from `args.ts` / `main.ts` (fork conflict, session-id check, `PI_OFFLINE`, `--models` scope, `--api-key`, images on `@file`, skills, telemetry spans), print/json/rpc/interactive sharing `SessionRuntime`, auth (`--json` / `--credentials` / `--no-refresh` / `--min-expiry` / exit 2 invalid), install/remove/update/list/config TUI, export HTML, all builtin slash commands with overlays, RPC `{type:"response",command,success}` plus streamed events and `extension_ui_request`, extension discovery + Node host + in-process `EventBus`.

## What remains

None. Product flags, sessions, providers, TUI, print, and RPC match the TypeScript surface with no “not ported” list.

## Gates

```bash
cargo test --workspace && cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings
```
