# Rust rewrite progress

**Complete: 86%**

TypeScript spec: `vendor/pi` @ `853a80d26c90a14c1886f0ebb8ffaae133ca2185` (`@earendil-works/pi-*` 0.84.4).
Toolchain: Rust **1.83.0**, no `edition2024`, no `unsafe`. Remote rust-rewrite placeholders were merged (ours strategy) and harvested for retry/permission shapes; live HTTP is implemented with `ureq` rather than those stubs.

## What landed

- Vendored TypeScript `pi` (reference only).
- Workspace crates: `pi-telemetry`, `pi-protocol`, `pi-session`, `pi-session-sqlite`, `pi-ai`, `pi-agent`, `pi-tui`, `pi-client`, `pi-server`, `pi-evals`, `pi-coding-agent` (`pi` binary), `pi-parity`.
- **pi-ai**: 39 provider catalogs (1290 models), env-key map, `auth.json`, OAuth authorize URLs + token fixture parse, request bodies for anthropic-messages / openai-completions / openai-responses / google-generative-ai, SSE corpora for all three, usage/cost, retry classification, live `ureq` SSE when credentials + `allow_network` (tests stay fixture-only; `PI_DISABLE_NETWORK=1` blocks live calls). Error string `No API key for provider: {id}`.
- **pi-session / sqlite**: JSONL v4 header, v3→v4 migrate, continue/resume/fork/clone, `~/.pi` + `--session-dir`, FTS5 MATCH + LIKE fallback, writer leases.
- **pi-agent**: compaction, skills, templates, context files, builtin tools read/write/edit/bash/ls/grep/find, permission policy, retry, steer/follow-up, fixture-driven loop that executes tool calls.
- **pi-tui**: `Component::render(width)`, wrap, editor, keybindings, markdown, mouse/alt-buffer, themes, selectors; wrap/keys/editor snapshots.
- **pi-protocol/client/server**: Unix+TCP+memory, handshake timeout, leases, request correlation, CBOR vectors.
- **pi-evals / pi-parity**: fixture evals + required golden corpora. Optional `--parallel-run` / `--diff-jsonl`.
- **pi binary**: flags/subcommands from `args.ts` / `main.ts`, print/json/rpc/interactive (interactive runs the agent), auth, install/remove/update/list/config, export, slash commands, RPC including `extension_tool`, extension discovery + Node host, settings/trust.

## What remains

- Provider-specific request extras still generic for Bedrock Converse, Vertex ADC headers, Codex websocket-cached, Mistral conversations, Cloudflare gateway binding (catalog + openai/anthropic/google bodies + live HTTP are present).
- Interactive mode is a line-oriented TUI with alt-screen/editor/markdown/selectors, not the full TypeScript differential overlay/chat widget suite.
- SQLite SQL edge cases from `sqlite-node/test` beyond FTS/leases/migrate (schema is in place).
- Extension host invokes Node modules; it does not embed the full TypeScript extension UI/event-bus API.

## Next crate/module

Tighten `pi-tui` interactive rendering against `vendor/pi/packages/tui` snapshots, then remaining sqlite-node conformance vectors.

## Gates

```bash
cargo test --workspace && cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings
```
