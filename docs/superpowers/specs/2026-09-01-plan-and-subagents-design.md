# Plan mode and native subagents — phase 5 design

Date: 2026-09-02. Branch: `rust-rewrite`. Roadmap:
`2026-09-01-competitive-harness-roadmap.md`, row 5. Builds on phase 2
(permission gate), phase 3 (todo ledger, plan sheet), phase 4 (MCP tools
stay available as reads).

## Why

Claude Code and Codex CLI both have a plan/act split and native subagents.
TypeScript `pi` has neither: the model edits as it thinks, and the only
parallel workers are the graph extension (out-of-process `pi` children).
A harness that cannot hold a plan still, or farm a research question to a
scoped worker, is not competitive on agency.

There is no TypeScript source to mirror. Documented divergence.

## What ships

| Piece | Where | Outcome |
|---|---|---|
| Plan mode | `pi-agent` + davinci | `/plan` freezes mutations; the model may read, search, fetch, keep the todo ledger. `/act` releases. Header names `plan` the way it names `auto`. |
| `agent` tool | `pi-agent` | `{ prompt, tools? }`. One nested `Agent`, depth 1, default tools `read grep find ls web_fetch web_search mcp_read`. Result is the worker's last assistant text. |
| Pane | davinci | A subagent is a tool line (`instrumenta`, verb `delegating`) whose output is the worker's reply. The 1c plan sheet still tracks the todo ledger. |
| Tests | fixtures | Plan mode refuses `write`/`bash`; `agent` runs a nested loop with a canned completer. Never the network. |

Out of scope: sampling/elicitation; spawning a `pi` child (that is graph);
a second transcript column; recursive subagents; a mockup screen id.

## Plan mode

`Agent.plan_mode: bool`, default `false`. `/plan` sets it, `/act` clears it,
`/plan` again is idempotent. The flag is session-only; it is not written to
settings.

While it is on:

- The permission gate treats every tool that is not `Read` or `Network` as
  denied, with `plan mode: mutations are off until /act`. `todo` stays `Read`
  so the ledger can be written. `mcp_read` stays `Read`. `mcp__*` follows
  `readOnlyHint` as in phase 4; a non-read MCP tool is refused.
- Deny rules still win.
- The system prompt gains a short appendix (a constant, part of the contract):
  the model is planning, must not edit, must keep the todo list current, and
  must wait for the user to `/act`.
- davinci header: the mode word is `plan` (copper) while the transcript is
  showing. The 1c sheet is still `/todo`'s surface, not a second plan mode.

`--permission-mode read-only` and plan mode can both be on; the stricter
refusal wins and the message still says plan mode when that flag is set.

## Subagents

Tool name `agent`. Parameters:

```
{ "prompt": string, "tools": string[]?, "description": string? }
```

`tools` is an allow-list intersected with the parent's active set, then with
the default read set if omitted. Unknown names are dropped. `bash` /
`powershell` / `write` / `edit` / `notebook_edit` are never granted to a
worker in this phase, even if listed — a subagent that can mutate is a later
widening, not the first ship.

Depth: a worker cannot call `agent`. The parent sets `plan_mode` on the
worker to false and strips `agent` from its tools.

Runner: `Agent.subagent_runner`, an injected `Arc<dyn Fn(SubagentRequest) ->
Result<String, String> + Send + Sync>`, same shape as `Summarizer`. The
coding-agent host installs a runner that clones cwd/provider/permissions
(read-only + network), builds a child `Agent`, and drives `run_loop` with the
same streaming completer as the parent. Tests install a canned runner.
Without a runner the tool returns `agent tool is not configured`.

Timeout 120s. One worker at a time in this phase (the parent's parallel
tool mode may still fire two `agent` calls; they serialise on a mutex).
Output is truncated to 50KB like a read.

Permission class: `Other` (asked in `ask`/`edits`, refused in `read-only`
and in plan mode). A grant for the session is `agent`.

## davinci

`instrument_of("agent")` is `instrumenta`. `state_of` is `Done` (or `Failed`).
`verb_of` is `delegating`. `target_of` is `agent` plus a clipped prompt or
the `description` if present. `/plan` and `/act` are corpus rows and
builtin slash commands. The header's mode word is `plan` when `plan_mode`
is on and the screen is `Agent`.

## Tests

- Plan mode: `write` denied with the plan-mode sentence; `read` and `todo`
  allowed; `/plan` `/act` parse; header word.
- Subagent: canned runner returns the prompt reversed; `agent` inside a
  worker is not in its tool list; mutation tools listed by the model are
  stripped; missing runner names the gap.
- Fixtures only. `PI_SUBAGENT_FIXTURE` may short-circuit the host runner.
