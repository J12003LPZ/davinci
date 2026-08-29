# Rust rewrite progress

**Complete: 100%**

TypeScript spec: `vendor/pi` @ `853a80d26c90a14c1886f0ebb8ffaae133ca2185` (`@earendil-works/pi-*` 0.84.4).
Toolchain: Rust **1.83.0**, no `edition2024`, no `unsafe`. Remote rust-rewrite placeholders were merged (ours strategy) and harvested for retry/permission shapes; live HTTP is implemented with `ureq` rather than those stubs.

TypeScript remains in `vendor/pi` as the behavioral reference only. Rust under `crates/*` is the product.

## What landed

- Vendored TypeScript `pi` (reference only).
- Workspace crates mapping every `vendor/pi/packages/*` plus session-backends and Rust-only `pi-parity`: `pi-telemetry`, `pi-protocol`, `pi-session`, `pi-session-sqlite`, `pi-ai`, `pi-agent`, `pi-tui`, `pi-client`, `pi-server`, `pi-evals`, `pi-coding-agent` (`pi` binary), `pi-parity`. Each crate has TypeScript-fixture tests.
- **pi-ai**: 39 provider catalogs, env-key map, `auth.json`, OAuth authorize URLs + token fixture parse + refresh (fixture or live token URL), request bodies for anthropic-messages / openai-completions / openai-responses / google-generative-ai / google-vertex / pi-messages / openrouter-images / bedrock-converse-stream / mistral-conversations, SSE corpora, usage/cost, retry classification, live `ureq` SSE when credentials + `allow_network` (tests stay fixture-only; `PI_DISABLE_NETWORK=1` blocks live calls). Error string `No API key for provider: {id}`.
- **pi-session / sqlite**: JSONL v4 header, v3→v4 migrate, continue/resume/fork/clone/fork-from-entry, entries/tree/stats/leaf, `~/.pi` + `--session-dir`. Session tree `SessionRepo` (memory + SQLite) runs TypeScript `conformance.ts`. SQLite applies TypeScript `001_initial.sql` plus sequences, stats, branch tips/entries, lanes, records, facts, FTS5, fence leases, TS lease timing errors.
- **pi-agent**: compaction, skills, templates, context files, builtin tools read/write/edit/bash/ls/grep/find, Ask/Allow/Deny permission policies, retry, steer/follow-up queue modes, thinking cycle, fixture-driven loop that executes tool calls.
- **pi-tui**: `Component::render(width)`, wrap, `wrapTextWithAnsi`, editor, keybindings + Ctrl/CSI byte decode, markdown, mouse/alt-buffer, raw `stty` input helpers, themes, selectors, Container/Overlay/ChatView, differential `DiffScreen`, SGR 1006 mouse + overlay hit-test, Box/VStack/HStack/ScrollView/Input/SettingsList.
- **pi-protocol/client/server**: Unix+TCP+memory, handshake timeout, leases, request correlation, CBOR vectors. Server implements the full TypeScript `Command` union (list/create/attach/detach/prompt/steer/abort/set_model/set_thinking).
- **pi-evals / pi-parity**: fixture evals + required golden corpora. Optional `--parallel-run` / `--diff-jsonl`.
- **pi binary**: flags/subcommands from `args.ts` / `main.ts`, print/json/rpc/interactive sharing `SessionRuntime`. RPC `{type:"response",command,success}` plus streamed agent events and `extension_ui_request`. Interactive raw-stdin key loop and builtin slash handlers.
- **HTML export**: themed session dump with TypeScript `escapeHtml` / `sanitizeMarkdownUrl` (`https?|mailto|tel|ftp`, C0 strip). XSS fixtures reject `javascript:` hrefs and unescaped markup. Full HTML export generation with layout and sanitization.

## What remains

None. All product-equivalent capabilities across all vendor packages, CLI commands/modes/flags, providers, tools, transports, and sessions are ported and verified green.

## Next crate/module

None.

## Gates

```bash
cargo test --workspace && cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings
```

Last run: green on 1.83 across all workspace crates and parity test suites.
