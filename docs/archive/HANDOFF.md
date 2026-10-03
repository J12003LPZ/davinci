# HANDOFF — competitive harness, phases 1–6 landed

## Goal

Turn the Rust `pi` rewrite into a harness that competes with Claude Code and
Codex CLI. Roadmap rows 1–6 are done on `rust-rewrite`.

## Commits

- Phase 1 `2d7162e`, phase 2 `8442da4`, phase 3 `6c45b51`
- Phase 4 `eeba283`, phase 5 `cb43f30`, phase 6 `9958f5c`

## Phase-2 leftovers (this landing)

- `/permissions` is a sheet: modes selectable, rules listed, enter on a
  rule removes it (session / user / project file).
- Denied tool rows keep the `✓` glyph and summarise `denied`.
- `/quit` and the TUI quit chord run `stop` hooks.
- `agent` workers run a nested `complete_prompt` (still honour
  `PI_SUBAGENT_FIXTURE` first).
- Live JS autocomplete loads the extension before querying, and retries
  once if the first reply is empty.

## Still blocked

Live Codex provider check — usage limit on the account. Decoder coverage
is fixture-based in `pi-ai`.
