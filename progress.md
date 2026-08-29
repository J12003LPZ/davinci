# Rust rewrite progress

**Complete: 72%**

TypeScript spec: `vendor/pi` @ `853a80d26c90a14c1886f0ebb8ffaae133ca2185` (`@earendil-works/pi-*` 0.84.4).
Toolchain: Rust **1.83.0**, no `edition2024`, no `unsafe`. rust-rewrite-prior (`d5a35e0`) was not available in this empty repo; architecture follows the TypeScript package map and pinned crates (`clap`/`ureq`/`uuid`/…).

## What landed

- Vendored TypeScript `pi` (reference only).
- Workspace crates: `pi-telemetry`, `pi-protocol`, `pi-session`, `pi-session-sqlite`, `pi-ai`, `pi-agent`, `pi-tui`, `pi-client`, `pi-server`, `pi-evals`, `pi-coding-agent` (`pi` binary), `pi-parity`.
- **pi-protocol**: length-prefixed frames, strict RFC 8949 CBOR vectors, hello/request validation, error strings from TS tests.
- **pi-telemetry**: noop + in-memory spans matching TS contracts.
- **pi-ai**: 39 provider catalogs (1290 models) from published `@earendil-works/pi-ai` 0.84.4, env-key map, `auth.json` storage, stream/complete lifecycle from SSE/HTTP fixtures, usage/cost. No live provider calls in tests.
- **pi-session / sqlite**: JSONL v4 header, v3→v4 migrate, continue/resume/fork/clone, `~/.pi` + `--session-dir`, FTS search, writer leases.
- **pi-agent**: compaction, skills, prompt templates, context files, builtin tools, retry, steer/follow-up queues, fixture-driven loop events.
- **pi-tui**: `Component::render(width)`, editor, keybindings, markdown, mouse/alt-buffer, themes, selectors.
- **pi-protocol/client/server**: Unix+TCP+memory transports, handshake timeout, leases, request correlation.
- **pi-evals / pi-parity**: fixture evals + required golden corpora (writer-leases, session entries, protocol hello/CBOR, assistant+usage, agent events, print/RPC events). Optional `--parallel-run` / `--diff-jsonl`.
- **pi binary**: flags/subcommands from `args.ts` / `main.ts` (print, json, rpc, interactive, auth, install/remove/update/list/config, export), slash commands from `slash-commands.ts`, RPC types from `rpc-types.ts`.

## What remains

- Deeper provider request-body parity for every TS adapter (Bedrock/Vertex/Codex/Mistral/…) beyond fixture SSE parsing — catalogs + auth + event lifecycle are present.
- Interactive TUI polish vs `packages/tui` snapshot suite (layout widgets beyond editor/markdown/selectors).
- Full sqlite conformance vectors from `packages/session-backends/sqlite-node/test` (leases/FTS/migrate are implemented; remaining SQL edge cases).
- Extension JS host (TypeScript extensions run as files/settings entries; no JS runtime embed).

## Next crate/module

Keep tightening `pi-ai` provider adapters against `vendor/pi/packages/ai/test` fixture corpora, then `pi-tui` snapshots.

## Gates

Run after every slice:

```bash
cargo test --workspace && cargo fmt --check && cargo clippy --workspace --all-targets -- -D warnings
```
