# Rust rewrite progress

**Complete: 92%**

TypeScript spec: `vendor/pi` @ `853a80d26c90a14c1886f0ebb8ffaae133ca2185` (`@earendil-works/pi-*` 0.84.4).
Toolchain: Rust **1.83.0**, no `edition2024`, no `unsafe`. Remote rust-rewrite placeholders were merged (ours strategy) and harvested for retry/permission shapes; live HTTP is implemented with `ureq` rather than those stubs.

## What landed

- Vendored TypeScript `pi` (reference only).
- Workspace crates: `pi-telemetry`, `pi-protocol`, `pi-session`, `pi-session-sqlite`, `pi-ai`, `pi-agent`, `pi-tui`, `pi-client`, `pi-server`, `pi-evals`, `pi-coding-agent` (`pi` binary), `pi-parity`.
- **pi-ai**: 39 provider catalogs (1290 models), env-key map, `auth.json`, OAuth authorize URLs + token fixture parse, request bodies for anthropic-messages / openai-completions / openai-responses / google-generative-ai, SSE corpora for all three, usage/cost, retry classification, live `ureq` SSE when credentials + `allow_network` (tests stay fixture-only; `PI_DISABLE_NETWORK=1` blocks live calls). Error string `No API key for provider: {id}`.
- **pi-session / sqlite**: JSONL v4 header, v3→v4 migrate, continue/resume/fork/clone, `~/.pi` + `--session-dir`. SQLite applies TypeScript `001_initial.sql` (`migrations` id, sessions/entries/sequences/stats/branches/lanes/records/facts/writer_leases), FTS5, fence leases, TS lease timing errors.
- **pi-agent**: compaction, skills, templates, context files, builtin tools read/write/edit/bash/ls/grep/find, permission policy, retry, steer/follow-up, fixture-driven loop that executes tool calls.
- **pi-tui**: `Component::render(width)`, wrap, `wrapTextWithAnsi` snapshot from `wrap-ansi.test.ts`, editor, keybindings, markdown, mouse/alt-buffer, themes, selectors.
- **pi-protocol/client/server**: Unix+TCP+memory, handshake timeout, leases, request correlation, CBOR vectors.
- **pi-evals / pi-parity**: fixture evals + required golden corpora. Optional `--parallel-run` / `--diff-jsonl`.
- **pi binary**: flags/subcommands from `args.ts` / `main.ts`, print/json/rpc/interactive (interactive runs the agent), auth, install/remove/update/list/config, export, slash commands, RPC including `extension_tool`, extension discovery + Node host, settings/trust.

## What remains

- Interactive mode still uses a line-oriented prompt over alt-screen widgets rather than the full TypeScript overlay/chat differential renderer.
- Provider extras (Bedrock Converse signed headers, Vertex ADC, Codex websocket-cached, Cloudflare gateway binding) share the common HTTP/SSE path; catalogs and auth are present.
- Node extension host covers file discovery + `invoke`; not the full TS extension event-bus/UI API.

## Next crate/module

Interactive TUI overlay/chat renderer in `pi-coding-agent` / `pi-tui`.

## Gates

```bash
cargo test --workspace && cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings
```
