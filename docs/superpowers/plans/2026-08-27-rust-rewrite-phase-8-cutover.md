# Phase 8: Cutover

## Decision

Rust is the default product. TypeScript remains vendored as `legacy-pi`.

## Checklist

1. `cargo test --workspace` passes.
2. `cargo clippy --workspace --all-targets -- -D warnings` is clean.
3. `cargo fmt --check` is clean.
4. Phase 7 fixtures are green.
5. Default install is `cargo install --path crates/pi-coding-agent` / `./scripts/install.sh` / `make install`.
6. Interactive TUI, print mode, and RPC JSONL are the CLI surfaces.
7. `vendor/pi` is documented as the behavioral reference, not the shipped binary.

## Follow-on (not blocking cutover)

- Same-process SQLite reopen reuse and lease heartbeat thread.
- JSONL v4 session repository.
- Unix-socket client/server transport and handshake timeout.
- Live OpenAI/Anthropic SSE adapters (fixture-tested; no CI network calls).
- Stateful `Agent` wrapper around `run_agent_loop`.
