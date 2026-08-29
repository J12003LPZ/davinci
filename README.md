# pi (Rust)

A high-performance, product-equivalent Rust implementation of the `pi` coding agent harness.

## Crates
- `pi-ai`: Unified multi-provider LLM API (Anthropic Messages, OpenAI Chat Completions, Google Generative AI, Ollama, OpenRouter, Groq, etc.) with streaming SSE, usage/cost tracking, and credential/auth management.
- `pi-agent`: Agent runtime, streaming agent loop, compaction, skills, prompt templates, tools, retry policies, and steer/follow-up queues.
- `pi-tui`: Terminal UI framework with differential rendering, alternate-screen viewport, syntax highlighting, themes, selectors, and synchronized output.
- `pi-session`: Session management, state tree, fork/resume/continue/clone workflows, JSONL persistence.
- `pi-session-sqlite`: SQLite backend for sessions with full-text search (FTS5) and schema migrations.
- `pi-protocol`: Strongly-typed protocol types, CBOR and JSON encoders, frame parsing, leases, and request correlation.
- `pi-client`: Client SDK for connecting to Pi server backends over Unix sockets, TCP, and memory channels.
- `pi-server`: Daemon server hosting sessions and agents over IPC/network transports.
- `pi-telemetry`: Telemetry schemas, span collection, and metrics export.
- `pi-evals`: Agent and model evaluation harness.
- `pi-coding-agent`: Interactive CLI binary `pi`, supporting interactive REPL, print mode (`-p`), JSON output, RPC daemon mode, auth management, slash commands, extensions, and themes.
- `pi-parity`: Comprehensive test suite verifying product parity and fixture compatibility against TypeScript reference implementation.

## Building & Running

```bash
cargo build --release
./target/release/pi --help
```
