# pi (Rust)

Product-equivalent Rust replacement for TypeScript [`pi`](https://github.com/earendil-works/pi) (vendor pin `853a80d26c90a14c1886f0ebb8ffaae133ca2185`).

A user who runs TypeScript `pi` can install this `pi` binary and keep the same flags, `~/.pi` sessions, provider credentials (`auth.json` / env vars), interactive TUI, `--print`, and `--mode rpc`.

TypeScript under `vendor/pi` is reference-only. Do not delete it.

## Crates

| Crate | TypeScript package |
| --- | --- |
| `pi-ai` | `@earendil-works/pi-ai` |
| `pi-agent` | `@earendil-works/pi-agent-core` |
| `pi-tui` | `@earendil-works/pi-tui` |
| `pi-session` / `pi-session-sqlite` | agent session JSONL + `@earendil-works/pi-session-backend-sqlite-node` |
| `pi-protocol` / `pi-client` / `pi-server` | `@earendil-works/pi-protocol`, `pi-client`, `pi-server` |
| `pi-telemetry` | `@earendil-works/pi-telemetry` |
| `pi-evals` | `@earendil-works/pi-evals` |
| `pi-coding-agent` (`pi`) | `@earendil-works/pi-coding-agent` |
| `pi-parity` | golden fixtures + optional `--parallel-run` / `--diff-jsonl` |

## Run locally

Requires Rust 1.83.0 (see `rust-toolchain.toml`).

```bash
cargo build --workspace
cargo test --workspace
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
./target/debug/pi --help
```

Print / RPC (no live provider calls required):

```bash
./target/debug/pi -p "List files in src/"
./target/debug/pi --mode rpc
```

Sessions live under `~/.pi/agent/sessions` (or `PI_CODING_AGENT_SESSION_DIR` / `--session-dir`), with cwd-encoded directories compatible with TypeScript `pi`.

Tests are fixture-only. They never call live LLM or network providers.

## License

MIT, matching upstream pi.
