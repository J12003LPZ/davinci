# Hooks and observability — phase 6 design

Date: 2026-09-02. Branch: `rust-rewrite`. Roadmap row 6.

## Why

Claude Code ships `/cost`, `/status`, user hooks, and a session ledger.
TypeScript `pi` has `/session` and `PI_AI_TRACE`. The competitive names and
a file the user can drop commands into are still missing.

No TypeScript counterpart for hooks. `/cost` and `/status` wrap the existing
session usage stats (`session_stats_for_agent`).

## What ships

| Piece | Outcome |
|---|---|
| `/cost` | Tokens (input/output/cache) and USD from the session ledger. |
| `/status` | Model, permission, plan mode, jobs, MCP servers, tokens. |
| `hooks.json` | User `~/.pi/agent/hooks.json`, trusted `.pi/hooks.json`. `preTool` / `postTool` / `stop` are argv arrays. Non-zero `preTool` blocks the call. `PI_HOOKS_CONFIG` fixture. |
| Session events | JSONL next to the session file (`<id>.events.jsonl`): `tool` / `turn` / `stop` rows. Tests use a temp session dir. |

Out of scope: a davinci cost sheet mockup; hooks answering the permission
panel (phase-2 leftover); OpenTelemetry exporters.
