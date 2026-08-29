# Progress: TypeScript → Rust Migration of `pi`

**Overall Completion: 40%**

## Current Status
- Initialized Rust workspace targeting Rust 1.83.0 toolchain.
- Vendor reference TypeScript repository locked at `853a80d26c90a14c1886f0ebb8ffaae133ca2185`.
- `pi-ai` slice complete with multi-provider streaming, model catalog, and auth.
- `pi-agent` slice complete with streaming agent loop runtime, tool execution interfaces, permission policies, skills discovery and formatting, prompt template rendering, context window compaction.
- All gates passing (`cargo test --workspace`, `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`).

## What Landed
- `pi-agent`:
  - `AgentRuntime` & `AgentConfig`: multi-turn streaming agent loop with tool execution, prompt streaming, turn management, and error recovery.
  - `AgentTool` & `AgentToolResult`: typed tool execution trait with schemas and results (text, image, error, details, usage, added tool names).
  - `PermissionPolicy` & `PermissionDecision`: permission gating for tools requiring authorization.
  - `AgentEvent`: typed event stream (`AgentStart`, `TurnStart`, `MessageStart`, `TextDelta`, `ThinkingDelta`, `ToolCallStart/Delta/End`, `ToolExecutionStart/End`, `MessageEnd`, `TurnEnd`, `AgentEnd`).
  - `skills`: markdown skill loading, frontmatter parsing, formatting invocations `<skill name=... location=...>`.
  - `prompt_templates`: prompt template markdown loading, frontmatter parsing, `{{args}}` rendering.
  - `compaction`: `CompactionSettings`, `should_compact`, context summarization.

## What Remains
1. `pi-tui`: Component render engine, fullscreen alternate buffer, editor, keybindings, markdown renderer, mouse support, themes, selectors.
2. `pi-session` & `pi-session-sqlite`: SQLite backend with FTS, schema v3->v4 migration, discovery, continue/resume/fork/clone logic.
3. `pi-protocol`, `pi-client`, `pi-server`: Transports (Unix sockets, TCP, in-memory), handshake timeouts, leases, request correlation, CBOR encoding.
4. `pi-telemetry` & `pi-evals`: Telemetry schemas/contracts and evals harness.
5. `pi-coding-agent`: Full `pi` CLI binary with all flags and subcommands, built-in tools (read, write, edit, bash), settings, trust, RPC server mode, HTML export.
6. `pi-parity`: Parity test suite with golden fixtures against vendor TypeScript reference.

## Next Step
Implement Slice 3: `pi-tui` crate with component tree, differential rendering, alternate-screen mode, editor, markdown formatting, keybindings, and themes.


