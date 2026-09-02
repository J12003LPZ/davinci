# Plan mode and subagents — implementation plan

Spec: `docs/superpowers/specs/2026-09-01-plan-and-subagents-design.md`.
Every task ends with `cargo test -p <crate>` green; the last with `make fmt`,
`make clippy`, `make test`.

## 1. Plan mode

- `Agent.plan_mode`. Permission gate: if set, refuse anything that is not
  `Read` or `Network`, message `plan mode: mutations are off until /act`.
- System prompt appendix constant. Applied in `messages_for_provider` or
  `reset_system_prompt_to_base` so it is always on while the flag is.
- Tests: write denied, read/todo allowed, appendix present.

## 2. `agent` tool

- Builtin spec + execute. `SubagentRunner` on `Agent`, default None.
- Strip mutation tools and `agent` from the worker's list.
- Tests: canned runner; missing runner; stripped tools.

## 3. Host + davinci

- `build_agent` installs a runner that drives a child `Agent` with the
  parent's completer (`PI_SUBAGENT_FIXTURE` short-circuit).
- `/plan` `/act` slash + corpus + header mode word.
- Naming for `agent`. Tests: parse, corpus, naming.

## 4. Finish

- fmt, clippy, tests; roadmap row 5; CLAUDE.md; HANDOFF.md; memory. Commit.
