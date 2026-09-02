# HANDOFF — competitive harness, phases 1–6 landed

## Goal

Turn the Rust `pi` rewrite into a harness that competes with Claude Code and
Codex CLI. The roadmap
`docs/superpowers/specs/2026-09-01-competitive-harness-roadmap.md` rows 1–6
are done on `rust-rewrite`.

## State (2026-09-02)

Phases 1–5 committed (`2d7162e`, `8442da4`, `6c45b51`, `eeba283`, `cb43f30`).
Phase 6 is this landing.

## Phase 6

- **`/cost` `/status`**: wrap `session_stats_for_agent`. Cost is tokens +
  USD; status is model, permission, plan/act, jobs, MCP count, then cost.
- **`hooks.json`**: user `~/.pi/agent/hooks.json`, trusted `.pi/hooks.json`.
  `preTool` / `postTool` / `stop` are argv arrays. Non-zero `preTool` blocks
  the call. `stop` runs on `/quit`. `PI_HOOKS_CONFIG` fixture;
  `PI_HOOKS_DRY_RUN` skips spawn.
- **Session events**: `<session>.events.jsonl` gets a `tool` row after each
  call.

## Phase-2 leftovers still open

`/permissions` sheet needs a mockup; rule editing from the panel; hooks
answering the permission panel; ledger `✓` on denied steps; js_host
autocomplete flake; live Codex check blocked by usage limit. The product
`agent` runner is still fixture-or-echo (phase 5).
