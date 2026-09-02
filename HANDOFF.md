# HANDOFF — competitive harness, phases 1–5 landed

## Goal

Turn the Rust `pi` rewrite into a harness that competes with Claude Code and
Codex CLI on engineering and interface. The roadmap is
`docs/superpowers/specs/2026-09-01-competitive-harness-roadmap.md`. Phases 1–5
are done; phase 6 (hooks and observability) is not started.

## State (2026-09-02)

- Branch `rust-rewrite`. Latest: phase 5 (plan mode + `agent` tool).
- Phase 4: `eeba283`. `cargo fmt` / clippy `-D warnings` green on the
  crates touched.

## Phase 5

Spec `docs/superpowers/specs/2026-09-01-plan-and-subagents-design.md`.

- **`/plan` / `/act`**: `Agent.plan_mode` + `PermissionPolicy.plan_mode`.
  Mutations (not `Read`/`Network`) are refused with `plan mode: mutations
  are off until /act`. Deny rules still win. System prompt gains
  `PLAN_MODE_APPENDIX`. Header mode word is `plan` while the transcript
  is showing.
- **`agent` tool**: `{ prompt, tools?, description? }`. Worker tools are
  the request list (or the default read set) intersected with the parent,
  minus `bash`/`powershell`/`write`/`edit`/`notebook_edit`/`agent`.
  `SubagentRunner` is injected; tests install a canned one. The product
  runner honours `PI_SUBAGENT_FIXTURE` (file or literal) and otherwise
  echoes the prompt with the scoped tool list — wiring the parent's
  streaming completer into the worker is the next tightening.
- davinci: `verb_of("agent")` is `delegating`; target is `description` or
  a clipped prompt. Corpus rows `/plan` `/act`.

## Phase 4 (still in force)

Native MCP: `crates/pi-mcp`, `mcp__<server>__<tool>`, `mcp_read`, `/mcp`.
HTTP tests use `fixture:<path>` so they do not share `PI_MCP_FIXTURE`
across threads.

## Next

Phase 6: user hooks, `/cost` `/status`, session cost ledger, structured
logs. Phase-2 leftovers: `/permissions` sheet mockup; rule editing from
the panel; hooks answering permission (phase 6); ledger `✓` on denied
steps; live Codex check blocked by usage limit.
