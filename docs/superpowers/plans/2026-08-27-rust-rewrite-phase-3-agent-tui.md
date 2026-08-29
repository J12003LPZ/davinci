# Phase 3: AI, Agent, Client, Server, and TUI Migration

## Overview
Phases 3 through 6 cover:
- `pi-ai`: Multi-provider streaming and tool calling engine.
- `pi-agent`: Autonomous agent execution loop, prompt generation, tool dispatch.
- `pi-client` / `pi-server`: JSON-RPC protocol transport and session multiplexing.
- `pi-coding-agent` & `pi-tui`: Interactive terminal user interface and developer workflows.

## Validation Gates
- Zero warnings under `cargo clippy --workspace --all-targets -- -D warnings`.
- Formatting clean under `cargo fmt --check`.
- All unit, integration, and differential fixture tests passing under `cargo test --workspace`.
