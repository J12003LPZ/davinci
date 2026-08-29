# Pi (Rust)

A native Rust rewrite and product-equivalent replacement for TypeScript `pi` (`@earendil-works/pi`).

## Overview

Pi is a stateful coding agent harness featuring:
- Unified multi-provider LLM integration with cost and usage tracking (`pi-ai`)
- Agent loop with streaming tool execution, steering, and follow-up queues (`pi-agent`)
- Fast terminal user interface primitives (`pi-tui`)
- Robust session storage with JSONL and SQLite full-text search backend (`pi-session`, `pi-session-sqlite`)
- High-performance binary CBOR wire protocol and client/server architectures (`pi-protocol`, `pi-client`, `pi-server`)
- Telemetry interfaces and evaluations framework (`pi-telemetry`, `pi-evals`)
- Comprehensive coding agent CLI executable `pi` (`pi-coding-agent`)

## Toolchain & Building

Requirements:
- Rust 1.83.0

```bash
# Build the workspace
cargo build --workspace

# Run all tests
cargo test --workspace

# Check formatting and linting
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
```

## Running

```bash
# Print mode
cargo run --bin pi -- -p "Hello Pi"

# RPC mode
cargo run --bin pi -- --mode rpc
```
