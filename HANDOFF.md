# HANDOFF — competitive harness, phases 1–4 landed

## Goal

Turn the Rust `pi` rewrite into a harness that competes with Claude Code and
Codex CLI on engineering and interface. The roadmap is
`docs/superpowers/specs/2026-09-01-competitive-harness-roadmap.md`. Phases 1
("turns that are real", `2d7162e`), 2 ("trust and control", `8442da4`), 3
("tools that compete", `6c45b51`) and 4 ("native MCP client", spec
`docs/superpowers/specs/2026-09-01-native-mcp-design.md`) are done; phases 5–6
are listed in the roadmap and not started.

## State (2026-09-02)

- Branch `rust-rewrite`. `cargo fmt`, `cargo clippy --workspace --all-targets
  -- -D warnings` and the crates touched by phase 4 (`pi-mcp`, `pi-agent`,
  `pi-tui`, `pi-coding-agent`) are green.
- Release binary: copy `target/release/pi.exe` to `~/.cargo/bin/{pi,davinci}.exe`
  (`cargo install` is broken in this repo; see memory).

## What phase 4 changed

Documented divergence from TypeScript `pi` (vendor has no MCP).

- **`crates/pi-mcp`**: JSON-RPC 2.0 over stdio (newline JSON) and streamable
  HTTP (`ureq` POST, last SSE `data:` frame). Handshake `initialize` /
  `notifications/initialized`, then `tools/list` + `resources/list`. Protocol
  `2025-03-26`. Client info `pi` / crate version. `tools/call` timeout 60s.
  In-tree `mcp-fixture` binary for stdio tests. HTTP tests use `PI_MCP_FIXTURE`
  or a `fixture:<path>` URL so they never hit the network.
- **Config**: `~/.pi/agent/mcp.json`, then trusted `.pi/mcp.json` (later name
  wins). `{ command, args, env }` or `{ url, headers }`; `"disabled": true`
  skips spawn. `PI_MCP_CONFIG` is a fixture path. Untrusted project files are
  ignored.
- **Agent tools**: each MCP tool is `mcp__<server>__<tool>` (names
  `[A-Za-z0-9_-]+`). Description prefixed `mcp:<server>.`. Schema is the
  server's `inputSchema`. `mcp_read { server, uri }` reads a listed resource.
  `Agent.attach_mcp` merges names into `tools` / `tool_registry` and fills
  `PermissionPolicy.mcp_read_only` from `readOnlyHint`. `builtin_and_mcp_specs`
  is what the provider sees. A transport failure marks the server error and
  drops its tools from the next turn's spec set.
- **Permissions**: `mcp_read` is `Read`. `mcp__*` with `readOnlyHint` is `Read`;
  otherwise `Other` (asked in `ask`/`edits`, refused in `read-only`). Deny
  rules still win; tool-name globs such as `mcp__memory__*` match.
- **davinci `/mcp`**: one row per server (glyph, name, transport, tool count
  or status). Naming: `instrumenta`; `mcp_read` is Read/studying;
  `target_of` is `mcp <server> <tool>` plus a clipped first argument. Corpus
  row `/mcp`. Legacy chrome prints the same rows as text.

## Verified

- `pi-mcp`: names, stdio fixture list+call+read, HTTP fixture, config
  parse/merge.
- `pi-agent`: fixture server becomes `mcp__memory__echo` and `mcp_read`;
  disabled servers are listed not spawned; deny glob wins over read-only hint.
- `pi-tui`: `/mcp` sheet draws connected + error rows.
- `pi-coding-agent`: `PI_MCP_CONFIG` wins; untrusted project file ignored;
  naming, corpus, `/mcp` parse.

## Next

Phase 5 in the roadmap ("plan mode and subagents": plan/act toggle, native
parallel workers, scoped tools, own transcript pane). Phase-2 leftovers still
open unless absorbed: `/permissions` sheet needs a mockup; rule editing from
the panel; hooks answering permission questions (phase 6); ledger `✓` on
denied steps (cosmetic); live Codex check blocked by usage limit.
