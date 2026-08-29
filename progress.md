# Progress: TypeScript → Rust Migration of `pi`

**Overall Completion: 25%**

## Current Status
- Initialized Rust workspace targeting Rust 1.83.0 toolchain.
- Vendor reference TypeScript repository locked at `853a80d26c90a14c1886f0ebb8ffaae133ca2185`.
- `pi-ai` slice completed with full model catalog, multi-provider interfaces (Anthropic, OpenAI, Google, OpenRouter, Faux), credential store, usage and cost calculation, context token estimation, and retry classification.
- All gates passing (`cargo test --workspace`, `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`).

## What Landed
- `pi-ai`:
  - Complete types: `Model`, `Context`, `Message`, `AssistantMessage`, `ToolCall`, `Usage`, `ModelCost`, `StopReason`, `AssistantMessageEvent`.
  - Providers: Anthropic, OpenAI, Google, OpenRouter, Faux with `stream_simple` dispatch.
  - Cost calculations: tiered pricing, cache read/write cost rates.
  - Context estimation: `estimate_context_tokens` with trailing tokens and assistant usage lookback.
  - Retry logic: regex patterns classifying retryable vs non-retryable provider errors.
  - Auth: `CredentialStore`, `InMemoryCredentialStore`, environment variable lookup.
  - Models store: `ModelsStore`, `InMemoryModelsStore`.
  - Builtin models catalog: Anthropic, OpenAI, Google, OpenRouter, Mistral, Groq, Cerebras, DeepSeek, xAI, Together, Fireworks.

## What Remains
1. `pi-agent`: Port core agent loop (`run_agent`), context compaction, skills, prompt templates, context files, extension tools, steer/follow-up queues.
2. `pi-tui`: Component render engine, fullscreen alternate buffer, editor, keybindings, markdown renderer, mouse support, themes, selectors.
3. `pi-session` & `pi-session-sqlite`: SQLite backend with FTS, schema v3->v4 migration, discovery, continue/resume/fork/clone logic.
4. `pi-protocol`, `pi-client`, `pi-server`: Transports (Unix sockets, TCP, in-memory), handshake timeouts, leases, request correlation, CBOR encoding.
5. `pi-telemetry` & `pi-evals`: Telemetry schemas/contracts and evals harness.
6. `pi-coding-agent`: Full `pi` CLI binary with all flags and subcommands, built-in tools (read, write, edit, bash), settings, trust, RPC server mode, HTML export.
7. `pi-parity`: Parity test suite with golden fixtures against vendor TypeScript reference.

## Next Step
Implement Slice 2: `pi-agent` crate with core agent loop, tool execution traits, permission policies, context compaction, skills, prompt templates, and steer/follow-up queues.

