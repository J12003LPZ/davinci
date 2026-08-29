# pi (Rust)

Product-equivalent Rust rewrite of TypeScript [`pi`](https://github.com/earendil-works/pi)
(`@earendil-works/pi-coding-agent` 0.84.4). TypeScript under `vendor/pi` at
`853a80d26c90a14c1886f0ebb8ffaae133ca2185` is the behavioral spec. Rust under
`crates/*` is the product.

A user who today runs TypeScript `pi` can install this `pi` binary and use the
same flags, `~/.pi` sessions, provider credentials, interactive TUI, print
mode, and RPC.

## Requirements

- Rust **1.83.0** (pinned in `rust-toolchain.toml`)
- No `edition2024`, no `unsafe`

## Build and run

```bash
cargo build --release -p pi-coding-agent
./target/release/pi --help
```

Common invocations match TypeScript:

```bash
pi                              # interactive TUI
pi -p "List files in src/"      # print mode
pi --mode json -p "Explain this"
pi --mode rpc                   # JSONL RPC on stdin/stdout
pi --continue
pi --resume
pi auth check --provider anthropic
pi install <source>             # local path, npm:, git:, or URL
pi config                       # enable/disable package resources
pi update --extensions          # refresh installed package trees
pi --export session.jsonl out.html
```

`pi install npm:…` / `git:…` materializes TypeScript's managed trees
(`~/.pi/agent/npm/node_modules/<name>` and `~/.pi/agent/git/<host>/<path>`).
Tests and offline runs copy from `PI_PACKAGE_FIXTURE` instead of the network.
`PI_DISABLE_NETWORK=1` blocks live `npm`/`git` spawns. `pi update` without a
target is self-update only (same as TypeScript); cargo/source installs print
the TypeScript unavailable instruction.

Sessions default to `~/.pi/agent/sessions/`, overridable with `--session-dir`
or `PI_CODING_AGENT_SESSION_DIR`. Credentials use the same `auth.json` layout
as TypeScript.

## Workspace crates

| Crate | TypeScript package |
| --- | --- |
| `pi-ai` | `@earendil-works/pi-ai` |
| `pi-agent` | `@earendil-works/pi-agent-core` |
| `pi-tui` | `@earendil-works/pi-tui` |
| `pi-session` / `pi-session-sqlite` | session JSONL + `@earendil-works` sqlite backend |
| `pi-protocol` / `pi-client` / `pi-server` | protocol / client / server |
| `pi-telemetry` | `@earendil-works/pi-telemetry` |
| `pi-evals` | `@earendil-works/pi-evals` |
| `pi-coding-agent` | `@earendil-works/pi-coding-agent` (`pi` binary) |
| `pi-parity` | golden fixtures + optional `--parallel-run` / `--diff-jsonl` |

## Tests

Tests are fixture-only. They never call live LLM or network providers.

```bash
cargo test --workspace
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```

`pi-parity` can optionally compare against a TypeScript `pi` on `PATH`
(`--parallel-run`, `--diff-jsonl`) when Node is present.

## Progress

See `progress.md`.
