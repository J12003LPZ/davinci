# Hooks and observability — implementation plan

Spec: `docs/superpowers/specs/2026-09-01-hooks-and-observability-design.md`.

- `/cost` `/status` wrap `session_stats_for_agent`.
- `hooks.json` load/merge; preTool blocks; postTool after the native hook;
  stop on `/quit`.
- Tests: untrusted project ignored; `PI_HOOKS_CONFIG`; corpus; parse.
