# Progress

Last updated: 2026-08-29  
Default product: Rust `pi` (`crates/pi-coding-agent` 0.84.4)  
TypeScript reference: `vendor/pi` at `853a80d26c90a14c1886f0ebb8ffaae133ca2185` (`legacy-pi`)  
GitHub: `J12003LPZ/pi-rust` branch `rust-rewrite`

## Status

**Cutover is done.** Rust is the shipped install path. TypeScript is the behavioral reference, not the product binary.

A 1:1 port of the TypeScript coding-agent (auth, skills, extensions, full-screen TUI, full model catalog) is **not** done.

## Phase map

| Phase | Crate | Status |
|-------|--------|--------|
| 1 | `pi-core` | Done — shared errors, IDs, JSON helpers |
| 2 | `pi-session`, `pi-session-sqlite` | Done — writer-leases, conformance, same-process reopen reuse, heartbeat, JSONL v4 |
| 3 | `pi-protocol`, `pi-client`, `pi-server` | Done — RFC 8949 CBOR, framing, in-memory loopback, Unix sockets + handshake timeout |
| 4 | `pi-ai` | Done — stream contract, `MockProvider`, OpenAI/Anthropic SSE adapters (fixture-tested) |
| 5 | `pi-agent` | Done — agent loop, steer/follow-up, sequential/parallel tools, stateful `Agent` |
| 6 | `pi-tui` | Done — `Component::render(width)`, Editor, SelectList, ChatView, slash commands |
| 7 | `pi-coding-agent` | Partial — `pi` CLI: print, `--mode json`, `--mode rpc`, interactive REPL, `sessions`, four tools |
| 8 | `pi-parity` | Done — differential fixtures + quality gates; cutover performed |

## Sibling agents folded into this branch

The parallel New Project agents (three Cursor Grok 4.6 High, four Gemini 3.7 Flash High) all committed onto the shared `main` history. Their combined tree — vendored TypeScript under `vendor/pi` plus the Rust workspace — is what `rust-rewrite` ships.

Additional implementer branches merged on top:

| Agent | Branch | What landed |
|-------|--------|-------------|
| Session reopen and heartbeat | `cursor/session-sqlite-reopen-heartbeat-c2fa` | `LeaseHeartbeat`, JSONL v4 repo |
| HTTP providers and Agent | `cursor/pi-ai-http-agent-79fe` | SSE adapters + stateful `Agent` |
| Unix protocol transport | `cursor/unix-socket-transport-d59c` | Unix sockets + handshake timeout |

A deeper, unrelated port that was previously on GitHub `rust-rewrite` (edition 2024 / rustc 1.98, `pi-telemetry`, fixture corpora) is preserved as `rust-rewrite-prior` at `d5a35e0` so that history is not discarded.

## Shipped on `rust-rewrite` / `main`

- Install: `./scripts/install.sh`, `make install`, `cargo install --path crates/pi-coding-agent --force`
- Toolchain pin: Rust 1.83.0 (`rust-toolchain.toml`)
- Live HTTPS on 1.83 via pinned `ureq` 2.10.1 + `rustls` 0.23.19 (newer url/idna/zeroize need edition2024)
- SQLite writer-lease SQL/fence/error strings match TypeScript
- Session conformance (memory, SQLite, JSONL)
- Unix transport: stale-socket rebind, `PiClient::connect_unix`, handshake timeout `"Handshake timeout"`
- Interactive: line-oriented Editor + agent loop + `/help` `/exit` `/clear` `/sessions`
- RPC JSONL: `prompt`, `steer`, `follow_up`, `get_state`, `bash`, `shutdown`, …
- Built-in tools: `read`, `write`, `edit`, `bash` (cwd-scoped)
- CI: fixtures only — no live LLM calls

## Quality gates (last known green)

```bash
cargo test --workspace
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```

## Not ported yet

- `--continue` / `--resume` / `--session` / `--session-id` / `--fork` / `--list-models`
- `auth`, settings, extensions, skills, themes, AGENTS.md / CLAUDE.md discovery
- OAuth and the rest of the live model catalog (~40 providers)
- Full-screen alt-buffer TUI (`--tui-mode fullscreen`)
- Compaction, package-manager CLI (`install` / `update` / `config`), HTML export
- Telemetry / evals
- Line-by-line parity with `vendor/pi/packages/coding-agent` (hundreds of TS files)

## How to run

```bash
cargo run -p pi-coding-agent -- -p "hello"
cargo run -p pi-coding-agent -- --mode rpc
cargo run -p pi-coding-agent -- sessions --database sessions.sqlite
```

See `README.md` and `docs/superpowers/plans/2026-08-27-rust-rewrite-program.md`.
