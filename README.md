# Pi

TypeScript-to-Rust migration of the Pi agent harness. TypeScript packages under `packages/` remain the schema and fixture authority until the Phase 8 gate. Rust crates under `crates/` are the port.

Program and phase plans: `docs/superpowers/plans/2026-08-27-rust-rewrite-*.md`.

## Rust workspace

| Crate | Phase | What it ports |
| --- | --- | --- |
| `pi-core` | 1 | Shared types, RFC 8949 CBOR subset, length-prefixed framing, protocol messages |
| `pi-session-sqlite` | 2 | Official SQLite schema, fenced writer leases, repository/entry/lane/fact storage |
| `pi-ai` | 3 | Stream protocol, faux/mock providers, tool-argument validation |
| `pi-agent` | 4 | Agent loop, tool dispatch, length-stop fail-all |
| `pi-client` / `pi-server` | 5 | Hello handshake, commands, exclusive/shared leases |
| `pi-coding-agent` / `pi-tui` | 6 | Print-mode CLI, built-in tools, differential renderer |
| `pi-conformance` | 7 | Golden CBOR, writer-lease, protocol, and loopback fixtures |

## Run

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check
cargo run -p pi-coding-agent -- -p "hello from the rust port"
```

TypeScript remains the shipping product path. The Rust `pi` binary is the print-mode port used for parity work.
