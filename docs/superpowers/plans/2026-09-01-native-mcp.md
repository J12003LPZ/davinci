# Native MCP client — implementation plan

Spec: `docs/superpowers/specs/2026-09-01-native-mcp-design.md`.
Every task ends with `cargo test -p <crate>` green; the last with `make fmt`,
`make clippy`, `make test`.

## 1. Protocol crate (`pi-mcp`)

- Workspace member `crates/pi-mcp`. Types: JSON-RPC request/response/error,
  `initialize` params/result, tool and resource descriptors, `CallToolResult`.
- `stdio`: spawn `command args`, env merge, newline JSON, stderr tail.
- `http`: `ureq` POST, JSON or last SSE `data:` frame.
- `Client`: handshake, `tools/list`, `tools/call`, `resources/list`,
  `resources/read`. Drop kills the child.
- Tests: in-process stdio fixture (a tiny `--mcp-fixture` binary or a
  thread that speaks the protocol on pipes). List + call + bad JSON.
  HTTP via `PI_MCP_FIXTURE`.

## 2. Config and agent wiring

- `pi-coding-agent/src/mcp.rs`: load `mcp.json` from the agent dir and,
  when trusted, `.pi/mcp.json`. `PI_MCP_CONFIG` fixture.
- `Agent` holds `McpRegistry` (`Arc<Mutex<…>>`): connected clients, specs
  merged into `tool_specs` / `execute_tool_with` for names starting
  `mcp__`. `mcp_read` built-in.
- `permission.rs`: `mcp__*` class from `readOnlyHint`; `mcp_read` is Read.
- Tests: fixture server becomes one agent tool; deny rule wins; untrusted
  project file is ignored.

## 3. davinci `/mcp`

- Naming: `instrument_of` / `state_of` / `target_of` / `verb_of` /
  `summary_of` for `mcp__` and `mcp_read`.
- Command sheet: servers, status, counts. Corpus row `/mcp`.
- Tests: naming, sheet rows, corpus.

## 4. Finish

- fmt, clippy, tests; release copy to `~/.cargo/bin/{pi,davinci}.exe`.
- Roadmap row 4 cites the spec; CLAUDE.md; HANDOFF.md; memory note. Commit.
