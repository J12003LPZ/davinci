# Progress: TypeScript → Rust Migration of `pi`

**Overall Completion: 10%**

## Current Status
- Initialized Rust workspace targeting Rust 1.83.0 toolchain.
- Vendor reference TypeScript repository locked at `853a80d26c90a14c1886f0ebb8ffaae133ca2185`.
- Pinned toolchain dependencies configured in root `Cargo.toml`.
- Crates skeleton created:
  - `crates/pi-ai`
  - `crates/pi-agent`
  - `crates/pi-tui`
  - `crates/pi-session`
  - `crates/pi-session-sqlite`
  - `crates/pi-protocol`
  - `crates/pi-client`
  - `crates/pi-server`
  - `crates/pi-telemetry`
  - `crates/pi-evals`
  - `crates/pi-coding-agent`
  - `crates/pi-parity`

## What Landed
- Setup root workspace configuration with exact dependency versions (clap 4.5.23, uuid 1.11.0, tempfile 3.14.0, thiserror 1, ureq 2.10.1, url 2.5.0, rustls 0.23.19, rustls-pki-types 1.10.1, webpki-roots 0.26.7, zeroize 1.8.1).
- Pinned rust-toolchain.toml to 1.83.0.

## What Remains
1. `pi-ai`: Port all providers (Anthropic, OpenAI, Google, Ollama, etc.), models catalog, auth storage, streaming event lifecycle, usage and cost estimation. SSE/HTTP fixture tests matching vendor TS.
2. `pi-agent`: Agent loop, context window compaction, skills, prompt templates, context files, extension tools, retry logic, steer/follow-up queues.
3. `pi-tui`: Component render engine, fullscreen alternate buffer, editor, keybindings, markdown renderer, mouse support, themes, selectors.
4. `pi-session` & `pi-session-sqlite`: SQLite backend with FTS, schema v3->v4 migration, discovery, continue/resume/fork/clone logic.
5. `pi-protocol`, `pi-client`, `pi-server`: Transports (Unix sockets, TCP, in-memory), handshake timeouts, leases, request correlation, CBOR encoding.
6. `pi-telemetry` & `pi-evals`: Telemetry schemas/contracts and evals harness.
7. `pi-coding-agent`: Full `pi` CLI binary with all flags and subcommands, built-in tools (read, write, edit, bash), settings, trust, RPC server mode, HTML export.
8. `pi-parity`: Parity test suite with golden fixtures against vendor TypeScript reference.

## Next Step
Implement Slice 1: `pi-ai` crate with full provider support, model catalog, auth storage, SSE streaming, and fixture tests.
