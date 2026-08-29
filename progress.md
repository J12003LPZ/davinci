# Rust rewrite progress

**Complete: 88%**

TypeScript spec: `vendor/pi` @ `853a80d26c90a14c1886f0ebb8ffaae133ca2185` (`@earendil-works/pi-*` 0.84.4).
Toolchain: Rust **1.83.0**, no `edition2024`, no `unsafe`.

## What landed

- Vendored TypeScript `pi` (reference only). Workspace crates map every `vendor/pi/packages/*` package.
- **pi-session**: JSONL v4, v3→v4 migrate, `~/.pi` / `--session-dir`. Path encoding matches TypeScript `jsonlSessionDirectoryName` (`/tmp/work` → `--tmp-work--`) and `sessionFileName`.
- **Session tree API**: TypeScript `SessionState` / `Session` / `SessionRepo` (`createLane`, `appendEntry`/`appendRecord`, facts, stats, fork branch/tree/before/at). Memory and SQLite both run the shared conformance cases from `vendor/pi/packages/agent/src/harness/session/testing/conformance.ts`.
- **pi-session-sqlite**: TypeScript `001_initial.sql`, FTS5, writer leases, mutation log, same conformance suite.
- **pi-ai**: catalogs, auth.json, OAuth fixture parse, request bodies, SSE corpora for OpenAI / Anthropic / Google / Bedrock Converse / Mistral / Codex responses. Live `ureq` only when credentials + `allow_network`.
- **pi-tui**: `Component::render(width)`, wrap/ANSI, editor, keys, markdown, mouse/alt-buffer, themes, selectors, Container/Overlay/ChatView, `DiffScreen`, SGR 1006 mouse, stacks/scroll/input/settings list.
- **pi-agent / protocol / client / server / evals / coding-agent / parity**: tools, queues, CBOR, CLI flags, RPC command set, extension discovery + EventBus, `--parallel-run` / `--diff-jsonl`.

## What remains

- Interactive mode still uses a line reader rather than a raw-stdin key loop; many slash handlers besides quit/help/clone/trust/login are names only.
- RPC `prompt` queues a message; it does not yet run the agent or emit the full `{type:"response",command,success}` envelope set. `fork`/`clone`/`switch_session` operate on the in-memory message list.
- Remote `pi-server` implements list/create; remaining protocol commands return `not_implemented`.
- Live provider extras (Bedrock SigV4, Vertex ADC, Codex websocket-cached session) share the common HTTP/SSE path; catalogs, auth, and fixture parsers are present.

## Next crate/module

RPC envelopes + AgentSession wiring, then raw-stdin interactive TUI and remaining server commands.

## Gates

```bash
cargo test --workspace && cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings
```

Last run before rebase: green on 1.83 including `memory_matches_ts_conformance` and `sqlite_matches_ts_conformance`.
