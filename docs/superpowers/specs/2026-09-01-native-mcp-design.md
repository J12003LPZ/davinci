# Native MCP client — phase 4 design

Date: 2026-09-02. Branch: `rust-rewrite`. Roadmap:
`2026-09-01-competitive-harness-roadmap.md`, row 4. Builds on phase 2 (the
permission gate) and phase 3 (tools that compete).

## Why

TypeScript `pi` has no MCP: "Build CLI tools with READMEs, or build an
extension that adds MCP support." Claude Code and Codex CLI both speak MCP
natively — stdio servers and streamable HTTP, tools merged into the model's
set, a `/mcp` status surface. A harness that needs Node and
`pi-mcp-adapter` to call a server is not competitive on tools.

There is no TypeScript source to mirror. Documented divergence (CLAUDE.md,
*Conventions*).

## What ships

| Piece | Where | Outcome |
|---|---|---|
| Protocol client | `crates/pi-mcp` | JSON-RPC 2.0 over stdio (newline JSON) and streamable HTTP (JSON POST). `initialize` / `initialized`, `tools/list` / `tools/call`, `resources/list` / `resources/read`. |
| Config | `~/.pi/agent/mcp.json`, project `.pi/mcp.json` when trusted | Named servers: `{ command, args, env }` or `{ url, headers }`. |
| Agent tools | `pi-agent` | Each MCP tool is an agent tool named `mcp__<server>__<tool>`. Calls go through the existing permission gate. |
| Resources | `mcp_read` | One built-in tool: `{ server, uri }`. Listed resources are named in its description. |
| `/mcp` | davinci sheet | Servers, transport, connected/error, tool counts. One panel. |
| Tests | fixture process | A tiny stdio server in-tree. `PI_MCP_FIXTURE` for HTTP. Never the network. |

No Node in the client. A server *may* be `npx …`; that is the server's
problem. No new workspace dependency: stdio is `Command`, HTTP is `ureq`.

## Config

`~/.pi/agent/mcp.json`:

```json
{
  "mcpServers": {
    "memory": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-memory"],
      "env": {}
    },
    "docs": {
      "url": "https://mcp.example.com/mcp",
      "headers": { "Authorization": "Bearer …" }
    }
  }
}
```

Project `.pi/mcp.json` is the same shape and is read only when the project is
trusted (same rule as `.pi/settings.json` permissions). User file first,
project adds or overrides by name. Unknown keys on a server object survive a
rewrite.

Disabled: `"disabled": true` skips spawn. Empty `mcpServers` is fine.

`PI_MCP_CONFIG` names a fixture file in tests so nothing touches `~/.pi`.

## Protocol

Client name `pi`, version the product version (`0.85.0`). Protocol version
`2025-03-26` (widely implemented); if the server answers with a different
supported version, believe it.

Handshake, then `tools/list`. Failures to spawn, handshake, or list are a
row on `/mcp` (`error · …`) and are not fatal to the session. A server that
dies mid-session is marked error; its tools vanish from the next turn's
active set.

`tools/call` timeout 60s (cap). Stdio: one JSON object per line on stdin,
one per line on stdout; stderr is kept as a 64 KB tail for the error row.
HTTP: `POST` the JSON-RPC body, `Accept: application/json, text/event-stream`;
if the response is SSE, take the last `data:` JSON-RPC payload. No OAuth
dance in this phase — `headers` carry a token if the user put one there.

Notifications from the server (`notifications/message`, progress) are
logged when `PI_AI_TRACE` is on and otherwise dropped. Sampling
(`/sampling/createMessage`) is refused: the host is not a nested model.

## Names and permissions

Tool name: `mcp__<server>__<tool>`. Server and tool names are `[A-Za-z0-9_-]+`;
anything else is skipped and named on `/mcp`. Permission rules match with the
existing glob (`mcp__memory__*`, `mcp__memory__create_entities`).

`ToolClass`: MCP `annotations.readOnlyHint == true` → `Read`; otherwise
`Other` (asked in `ask` and `edits`, refused in `read-only`, run in `auto`).
Deny rules still win. `mcp_read` is `Read`.

The model sees MCP tools in `tool_specs` next to built-ins. Description is
the server's, prefixed with `mcp:<server>.`. Schema is the server's
`inputSchema`.

## davinci

`instrument_of("mcp__…")` is `instrumenta`. `state_of` follows the class
(read vs other). `target_of` is `mcp <server> <tool>` plus a clipped first
argument. `/mcp` opens a sheet: one row per server, glyph for
connected/error/disabled, tool count, transport. Esc closes. No connect UI
in this phase — edit the JSON.

## Out of scope (later)

OAuth for remote servers; MCP prompts as slash commands; elicitation;
roots; sampling; a davinci add-server flow; Windows job objects for stdio
trees beyond `taskkill` of the child we spawned.
