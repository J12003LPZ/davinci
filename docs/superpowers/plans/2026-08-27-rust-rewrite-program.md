# Pi Monorepo: TypeScript to Rust Migration Program

## Executive Summary

This program governs the complete, phase-gated migration of the `pi` AI agent monorepo from TypeScript to idiomatic Rust. The goal is to achieve production-grade performance, strict concurrency safety (e.g. SQLite writer leases, session isolation), robust protocol conformance (`pi-client`, `pi-server`, `pi-ai`, `pi-agent`, `pi-tui`), and differential fixture parity across all subsystems.

## Phase Architecture

- **Phase 0: Architecture, Tooling, and Workspace Setup**
  - Cargo workspace configuration (`crates/`)
  - TypeScript workspace (`packages/`)
  - Shared schemas and protocol definitions
- **Phase 1: Foundation and Shared Types (`pi-core`)**
  - Protocol message formats, event streams, RPC contracts
  - Serialization / deserialization parity with TypeScript
- **Phase 2: Storage & Session Engine (`pi-session-sqlite`)**
  - High-performance SQLite session storage
  - Writer-lease semantics (exclusive acquire, heartbeats, automatic lease expiry)
  - Differential snapshot and event playback conformance
- **Phase 3: AI Provider Subsystem (`pi-ai`)**
  - Multi-provider AI abstractions (OpenAI, Anthropic, Gemini, Ollama, custom LLMs)
  - Streaming completions, token counting, tool calling
- **Phase 4: Agent Core Execution Loop (`pi-agent`)**
  - State machine, thought-action-observation cycles, subagent delegation
  - Cancellation tokens, timeout management, middleware pipelines
- **Phase 5: Networking & Protocol Layer (`pi-client`, `pi-server`)**
  - JSON-RPC over WebSockets / HTTP / stdio
  - Real-time event streaming and duplex communication
- **Phase 6: Coding Agent & TUI (`pi-coding-agent`, `pi-tui`)**
  - Terminal User Interface with ratatui / crossterm
  - Interactive tool approval, diff viewer, multi-session management
- **Phase 7: Differential Conformance & Golden Fixture Validation**
  - Dual-run fixture evaluation verifying 100% output parity between TypeScript and Rust
- **Phase 8: Transition, Cutover, and Parity Sign-off**
  - Final phase gate validation: `cargo test`, `cargo clippy`, `cargo fmt` clean.

## Parity and Authority Rule
TypeScript remains authoritative for protocol schemas and golden fixtures until Phase 8 gate criteria are completely verified.
