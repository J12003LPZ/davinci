# Pi Monorepo: TypeScript to Rust High-Performance Agent Architecture

This monorepo contains both the TypeScript reference implementation (`packages/*`) and the high-performance idiomatic Rust implementation (`crates/*`) of the Pi AI Agent system.

## Architecture

### Rust Workspace (`crates/`)
- `pi-core`: Shared protocols, types, RPC envelopes, event schemas.
- `pi-session-sqlite`: SQLite session store with strict writer leases and TTL heartbeats.
- `pi-ai`: Multi-provider AI streaming, embeddings, and tool execution.
- `pi-agent`: Agent state machine, execution loop, subagent delegation.
- `pi-client`: Client SDK and WebSocket/stdio RPC transport.
- `pi-server`: Daemon server exposing agent services and sessions.
- `pi-coding-agent`: Specialized programming agent with AST and tool integration.
- `pi-tui`: Interactive terminal user interface built with Ratatui.
- `pi-conformance`: Differential test harness validating TS vs Rust parity.

### TypeScript Reference Packages (`packages/`)
- `@pi/core`: Core protocol definitions.
- `@pi/session-sqlite`: TypeScript SQLite storage with writer-lease semantics.
- `@pi/ai`: TypeScript AI provider integrations.
- `@pi/agent`: TypeScript agent execution loop.
- `@pi/client`: TypeScript client library.
- `@pi/server`: TypeScript server daemon.

## Building and Testing

```bash
# Build & test Rust crates
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --check

# Test Conformance
cargo test -p pi-conformance
```
