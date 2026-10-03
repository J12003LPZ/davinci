# davinci-mcp

`davinci-mcp` is a native Rust client implementation of the [Model Context Protocol (MCP)](https://modelcontextprotocol.io/). It enables Davinci to connect to external tool and resource providers without needing Node.js or JavaScript wrappers.

---

## Key Capabilities

- **Transports (`transport.rs`)**:
  - **Stdio Transport**: Spawns local CLI tools or background servers as child processes and communicates over line-delimited JSON-RPC via standard input/output.
  - **HTTP / SSE Transport**: Connects to remote MCP servers over HTTP with Server-Sent Events (SSE) for streaming message streams.
- **MCP Feature Support**:
  - `tools/list` and `tools/call`: Auto-discovers external tools and mounts them into the agent tool suite as `mcp__<server>__<tool>`.
  - `resources/list` and `resources/read`: Reads external file trees, database schemas, and documentation resources via `mcp_read`.
  - Annotations support (`readOnlyHint`) automatically classifying tools for parallel scheduling.
- **In-Tree Test Fixture Server (`src/bin/mcp-fixture.rs`)**:
  - Provides a deterministic, offline MCP server binary used in automated tests across the workspace.

---

## Testing

```bash
# Test MCP client logic
cargo test -p davinci-mcp

# Run the bundled test fixture server
cargo run -p davinci-mcp --bin mcp-fixture
```
