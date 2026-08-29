# Pi TypeScript → Rust rewrite program

Date: 2026-08-27
Status: cutover complete
Default product: Rust `pi` (`crates/pi-coding-agent`)
TypeScript reference: `vendor/pi` at `853a80d26c90a14c1886f0ebb8ffaae133ca2185` (`earendil-works/pi` main), also called `legacy-pi`

## Objective

Port the pi monorepo crate-by-crate from TypeScript to Rust. Rust is the shipped product. TypeScript remains the executable specification for fixtures and error strings.

## Authority

- Rust under `crates/*` is the default install and binary.
- TypeScript under `vendor/pi/packages/*` is the behavioral reference.
- When behavior disagrees, change Rust, not TypeScript, unless a TypeScript bug is independently confirmed.
- Do not delete the TypeScript tree. It is no longer the product install path.

## Crate map

| Phase | TypeScript package | Rust crate | Gate |
|-------|--------------------|------------|------|
| 1 | shared types / errors | `pi-core` | workspace builds |
| 2 | `packages/agent` session types + `packages/session-backends/sqlite-node` | `pi-session`, `pi-session-sqlite` | writer-leases + session conformance |
| 3 | `packages/protocol`, `packages/client`, `packages/server` | `pi-protocol`, `pi-client`, `pi-server` | CBOR/framing vectors + loopback |
| 4 | `packages/ai` | `pi-ai` | stream contract + usage + tool-call fixtures |
| 5 | `packages/agent` | `pi-agent` | agent-loop steer/follow-up/tools |
| 6 | `packages/tui` | `pi-tui` | component render snapshots |
| 7 | `packages/coding-agent` | `pi-coding-agent` | CLI interactive/print/RPC/session + built-in tools |
| 8 | cross-package | `pi-parity` | differential fixtures + quality gates + cutover |

## End state (Phase 8 gate)

The rewrite is complete when **all** of the following are true:

1. Every crate in the map exists, is wired into the Cargo workspace, and is covered by tests.
2. SQLite writer-lease SQL, fence takeover, error strings, and `list()` non-mutation match TypeScript.
3. Session backend conformance covers entries/lanes, records/log, queries/facts, validation, repository/forks.
4. Protocol CBOR RFC 8949 known vectors and framing fragmentation tests pass.
5. Client/server handshake, request correlation, and session leases work over an in-memory byte channel.
6. `pi-ai` exposes `stream` / `complete` with the TypeScript event lifecycle and usage accounting.
7. `pi-agent` implements the agent loop, sequential/parallel tools, steer, and follow-up.
8. `pi-tui` implements the `Component::render(width)` contract and snapshot-tested widgets.
9. `pi` CLI supports `--help`, `--version`, interactive mode, print mode, RPC JSONL, session list, and the four built-in tools.
10. `pi-parity` fixtures lock JSON shapes shared with TypeScript.
11. `cargo test --workspace`, `cargo fmt --check`, and `cargo clippy --workspace --all-targets -- -D warnings` are green.
12. The default product install is Rust (`cargo install --path crates/pi-coding-agent` / `scripts/install.sh`). TypeScript under `vendor/pi` remains as `legacy-pi` reference.

Cutover **has been performed**. Remaining ports (heartbeat, Unix sockets, live HTTP providers) land as follow-on crate work against this default.

## Quality bar

- Idiomatic Rust. No `unsafe`.
- Exact TypeScript error strings where tests assert them.
- Fixtures, not live provider calls, for CI.
- Keep plans in `docs/superpowers/plans/2026-08-27-rust-rewrite-phase-*.md` current with the work.

## Per-phase plans

See `2026-08-27-rust-rewrite-phase-1.md` through `2026-08-27-rust-rewrite-phase-8.md` and `2026-08-27-rust-rewrite-phase-8-cutover.md`.
