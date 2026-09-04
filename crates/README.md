# Davinci Workspace Crates (`crates/`)

This directory contains the 13 active Rust crates that comprise the **Davinci** agentic coding harness, along with 1 legacy archive crate.

---

## Workspace Architecture

Dependency flow is strictly layered from bottom to top:

```
┌────────────────────────────────────────────────────────────────────────┐
│                   davinci-coding-agent (bin: davinci)                   │
│         CLI Entrypoint · REPL / Interactive Loop · Native Extensions   │
└───────────────┬──────────────────────────────────────┬─────────────────┘
                │                                      │
┌───────────────▼────────────────┐   ┌─────────────────▼─────────────────┐
│          davinci-tui           │   │           davinci-agent           │
│  Ratatui UI · Command Sheets   │   │  Agent Loop · Tool Execution Engine│
│  Dual Canvases · Meters & ANSI │   │  Permissions · Lanes · Subagents  │
└────────────────────────────────┘   └───────┬──────────────┬────────────┘
                                             │              │
                     ┌───────────────────────▼──┐   ┌───────▼───────────┐
                     │        davinci-ai        │   │    davinci-mcp    │
                     │  LLMs · SSE Streaming    │   │  Native MCP Client│
                     │  Prompt Caching · OAuth  │   │  stdio & SSE Trans│
                     └──────────────────────────┘   └───────────────────┘
                                             │
┌────────────────────────────────────────────▼───────────────────────────┐
│                     davinci-session / davinci-session-sqlite           │
│            JSONL Session Storage · Branch Cache · Context Limits       │
└────────────────────────────────────────────┬───────────────────────────┘
                                             │
┌────────────────────────────────────────────▼───────────────────────────┐
│                          davinci-protocol                              │
│                Length-Prefixed CBOR Framing · RPC Wire Format          │
└───────────────────────┬──────────────────────────────────┬─────────────┘
                        │                                  │
┌───────────────────────▼────────┐        ┌────────────────▼─────────────┐
│         davinci-client         │        │        davinci-server        │
│   Client SDK for Daemon IPC    │        │  Background RPC Agent Daemon │
└────────────────────────────────┘        └──────────────────────────────┘

Supporting Infrastructure:
• davinci-telemetry: Structured logging, OpenTelemetry, Codex telemetry tracing
• davinci-evals: Autonomous benchmark runner & SWE-bench harness
• davinci-parity: Golden fixture suites validating parity against TypeScript reference
```

---

## Crate Directory

| Crate | Purpose | Primary Modules | Tests |
| :--- | :--- | :--- | :--- |
| [`davinci-coding-agent`](davinci-coding-agent/) | Main executable CLI (`davinci`), flags, interactive shell, native extensions | `main.rs`, `davinci_interactive.rs`, `native_extensions/`, `args.rs` | `cargo test -p davinci-coding-agent` |
| [`davinci-agent`](davinci-agent/) | Execution engine, tool runner, tool scheduler, permissions, subagents | `agent.rs`, `tools/`, `scheduler.rs`, `permission.rs`, `subagent.rs` | `cargo test -p davinci-agent` |
| [`davinci-ai`](davinci-ai/) | LLM providers (Anthropic, OpenAI, Codex, Bedrock, Ollama, Gemini), streaming | `stream.rs`, `codex.rs`, `cache.rs`, `oauth.rs`, `catalog.rs` | `cargo test -p davinci-ai` |
| [`davinci-tui`](davinci-tui/) | Terminal UI using Ratatui, transcript mockups (`1a`–`2c`), command sheets (`3a`–`6d`) | `davinci/`, `tui_runtime.rs`, `theme.rs`, `autocomplete.rs` | `cargo test -p davinci-tui` |
| [`davinci-session`](davinci-session/) | Session storage, JSONL serialization, discovery, compaction tracking | `repo.rs`, `jsonl_repo.rs`, `discovery.rs`, `types.rs` | `cargo test -p davinci-session` |
| [`davinci-session-sqlite`](davinci-session-sqlite/) | High-performance SQLite session backend and branch snapshot cache | `branch_cache.rs`, `lib.rs` | `cargo test -p davinci-session-sqlite` |
| [`davinci-mcp`](davinci-mcp/) | Native Model Context Protocol (MCP) client & stdio/SSE transports | `client.rs`, `transport.rs`, `protocol.rs`, `mcp_fixture` | `cargo test -p davinci-mcp` |
| [`davinci-protocol`](davinci-protocol/) | CBOR wire framing, message encoding, RPC requests/responses | `framing.rs`, `cbor.rs`, `protocol.rs`, `types.rs` | `cargo test -p davinci-protocol` |
| [`davinci-client`](davinci-client/) | Client SDK for talking to running davinci daemons | `client.rs`, `lib.rs` | `cargo test -p davinci-client` |
| [`davinci-server`](davinci-server/) | RPC server daemon hosting agents over Unix domain sockets or TCP | `server.rs`, `lib.rs` | `cargo test -p davinci-server` |
| [`davinci-telemetry`](davinci-telemetry/) | Observability, structured events, and Codex telemetry tracing | `lib.rs`, `events.rs` | `cargo test -p davinci-telemetry` |
| [`davinci-evals`](davinci-evals/) | Benchmarking suite, evaluation harness, SWE-bench runner | `harness.rs`, `reporter.rs`, `artifacts.rs` | `cargo test -p davinci-evals` |
| [`davinci-parity`](davinci-parity/) | Differential golden-fixture testing against the reference implementation | `runner.rs`, `fixtures/` | `cargo test -p davinci-parity` |
| [`davinci-core`](davinci-core/) | *Legacy*: Early port stub (superseded by `davinci-protocol`; uncompiled) | `cbor.rs`, `framing.rs` | N/A |

---

## Workspace Conventions

1. **Exact Pinning**: All dependencies in `Cargo.toml` use exact versions (`=x.y.z`). Keep this convention when updating or introducing crates.
2. **Deterministic & Offline Tests**: Tests use fixtures (`PI_*` environment flags) and never hit external network endpoints.
3. **No Unsafe Code**: The codebase relies on standard memory-safe Rust with thin LTO enabled for release builds.
