# davinci-mcp

`davinci-mcp` is a native Rust client implementation of the [Model Context Protocol (MCP)](https://modelcontextprotocol.io/). It enables Davinci to connect to external tool and resource providers without needing Node.js or JavaScript wrappers.

---

## Key Capabilities

- **Transports (`stdio.rs`, `http.rs`)**:
  - **Stdio Transport**: Spawns local CLI tools or background servers as child processes and communicates over line-delimited JSON-RPC via standard input/output.
  - **HTTP / SSE Transport**: Connects to remote MCP servers over HTTP with Server-Sent Events (SSE) for streaming message streams.
- **MCP Feature Support**:
  - `tools/list` and `tools/call`: Auto-discovers external tools and mounts them into the agent tool suite as `mcp__<server>__<tool>`.
  - `resources/list` and `resources/read`: Reads external file trees, database schemas, and documentation resources via `mcp_read`.
  - Annotations support (`readOnlyHint`) with explicit local trust opt-in, never automatic remote authorization.
- **In-Tree Test Fixture Server (`src/bin/mcp-fixture.rs`)**:
  - Provides a deterministic, offline MCP server binary used in automated tests across the workspace.

---

## Per-server MCP call deadlines

An MCP server may take longer than the default 60-second deadline to
initialize, answer a tool call, or read a resource. Set `callTimeoutSeconds`
in that server's `mcp.json` entry when needed:

```json
{
  "mcpServers": {
    "reports": {
      "url": "https://example.com/mcp",
      "callTimeoutSeconds": 120
    }
  }
}
```

- **Default:** 60 seconds when omitted.
- **Valid range:** 1–1800 seconds (30 minutes). Zero and values outside the
  range fail the server connection with a visible error.
- **Scope:** Each server has its own deadline; changing it does not change
  other servers or the runtime's model-request timeout.
- **Safety:** A longer deadline is not an automatic retry. An MCP call whose
  result is unknown must never be replayed just because it timed out.
- **Supervised local servers:** The timeout is also set before the MCP
  initialization handshake for sandboxed stdio connections.

---

## Testing

```bash
# Test MCP client logic
cargo test -p davinci-mcp

# Run the bundled test fixture server
cargo run -p davinci-mcp --bin mcp-fixture
```
