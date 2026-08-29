# Pi monorepo (TypeScript + Rust port)

This repository vendors the TypeScript pi monorepo and a Rust workspace that ports it crate-by-crate.

**TypeScript is still the authoritative product.** The Rust port is complete through the Phase 8 shadow-complete gate defined in `docs/superpowers/plans/2026-08-27-rust-rewrite-program.md`. The default install path remains the TypeScript packages. Rust is a parity-tested port, not a cutover.

Upstream spec: `vendor/pi` at `853a80d26c90a14c1886f0ebb8ffaae133ca2185` (`earendil-works/pi`).

## Layout

| Tree | Role |
|------|------|
| `vendor/pi/packages/*` | TypeScript source of truth |
| `crates/*` | Rust ports |
| `docs/superpowers/plans/` | Program and per-phase plans |

| Crate | TypeScript package |
|-------|--------------------|
| `pi-core` | shared errors |
| `pi-session` / `pi-session-sqlite` | harness sessions + sqlite-node writer-leases |
| `pi-protocol` / `pi-client` / `pi-server` | framed CBOR protocol |
| `pi-ai` | stream contract + mock provider |
| `pi-agent` | agent loop, steer, follow-up, tools |
| `pi-tui` | component `render(width)` |
| `pi-coding-agent` | `pi` CLI (print mode, sessions, tools) |
| `pi-parity` | differential fixtures |

## Rust

Requires Rust 1.83+ (bundled `rusqlite`).

```bash
cargo test --workspace
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p pi-coding-agent -- -p "hello"
cargo run -p pi-coding-agent -- sessions --database sessions.sqlite
```

## TypeScript (authoritative runtime)

```bash
cd vendor/pi
npm install
npm test
```

Node 22.19+ is required for the TypeScript workspace. The Rust port does not replace `npm` install or the published `@earendil-works/*` packages.

## License

MIT. TypeScript sources retain the upstream copyright of Mario Zechner / Earendil Works.
