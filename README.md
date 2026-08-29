# Pi

Rust is the default product. The `pi` binary comes from `crates/pi-coding-agent`.

This repository ports the Earendil Works TypeScript monorepo crate-by-crate. TypeScript under `vendor/pi` is the behavioral reference (`legacy-pi`) at `853a80d26c90a14c1886f0ebb8ffaae133ca2185`. When Rust and TypeScript disagree, change Rust unless the TypeScript bug is independently confirmed.

## Install

Requires Rust 1.83+ (`rust-toolchain.toml` pins 1.83.0).

```bash
./scripts/install.sh
# or
make install
# or
cargo install --path crates/pi-coding-agent --force
```

`pi` is then on your PATH:

```bash
pi --help
pi --version
pi -p "summarize this repo"
pi --mode json -p "hello"
pi --mode rpc
pi sessions --database sessions.sqlite
```

Interactive mode is the default when stdin is a TTY and you do not pass `-p` / `--mode rpc`. Type a prompt, or `/help`. `/exit` leaves the session.

## Modes

| Invocation | Behavior |
|------------|----------|
| `pi` | Interactive TUI (Editor + agent loop + slash commands) |
| `pi -p "…"` / `--print` | One-shot print mode, then exit |
| `pi --mode json -p "…"` / `--json` | Print mode as JSON event lines |
| `pi --mode rpc` | JSONL RPC on stdin/stdout (`prompt`, `get_state`, `bash`, …) |
| `pi sessions` | List sessions in the SQLite database |

Print and RPC use the mock provider unless a live adapter is configured. CI never calls a network LLM.

## Layout

| Tree | Role |
|------|------|
| `crates/*` | Shipped Rust product |
| `vendor/pi/packages/*` | TypeScript reference (`legacy-pi`) |
| `docs/superpowers/plans/` | Rewrite program and per-phase plans |

| Crate | TypeScript package |
|-------|--------------------|
| `pi-core` | shared errors |
| `pi-session` / `pi-session-sqlite` | harness sessions + sqlite-node writer-leases |
| `pi-protocol` / `pi-client` / `pi-server` | framed CBOR protocol |
| `pi-ai` | stream contract, mock provider, fixture SSE adapters |
| `pi-agent` | agent loop, steer, follow-up, tools |
| `pi-tui` | component `render(width)` + interactive chat view |
| `pi-coding-agent` | `pi` CLI (interactive, print, RPC, sessions, tools) |
| `pi-parity` | differential fixtures (leases, protocol, entries, agent/CLI events) |

## Develop

```bash
make test
make fmt
make clippy
cargo run -p pi-coding-agent -- -p "hello"
cargo run -p pi-coding-agent -- sessions --database sessions.sqlite
```

## TypeScript reference

```bash
cd vendor/pi
npm install
npm test
```

Node 22.19+ is required only if you are comparing against the vendored TypeScript suite. It is not the install path for `pi`.

## License

MIT. Vendored TypeScript retains the upstream copyright of Mario Zechner / Earendil Works.
