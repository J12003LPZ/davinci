# Agent teams

Enable `DAVINCI_EXPERIMENTAL_AGENT_TEAMS=1` before starting Davinci to expose teammate mode and team tools. Use an interactive or RPC session: `--print` rejects `background` and `teammate` because that host cannot keep them alive. One-shot workers remain available without the flag.

## Modes and lifecycle

The `agent` tool accepts `oneshot`, `background`, and `teammate`. Unknown modes are errors. One-shot calls return their result directly; a call can contain up to eight tasks, with four running at once. Background workers run once and report their result to the lead's mailbox.

Spawn a collaborator with `agent` arguments such as `{"prompt":"Review the parser","mode":"teammate","name":"reviewer"}`. A teammate retains its conversation between turns. It runs, reports, becomes Idle, and waits on a mailbox condition variable. `agent_message` or `/agents msg reviewer summarize your findings` wakes it for another turn. Its report is labeled with its name, ID, and kind:

```text
<agent-message from="reviewer" agent_id="…" kind="teammate">
status: completed

Findings and file references
</agent-message>
```

Failed turns report `status: failed`. Three consecutive failed turns end the teammate. It also exits on `agent_stop`, `/agents stop reviewer`, a session switch, or an idle timeout. `DAVINCI_TEAMMATE_IDLE_TIMEOUT_MS` controls that timeout; the default is 30 minutes. Tests can shorten it and use `PI_SUBAGENT_FIXTURE` for deterministic responses.

An idle interactive lead with an empty composer starts a turn after a 750 ms debounce when reports arrive. Typing or an open sheet prevents this wakeup. Set `DAVINCI_AGENT_TEAMS_AUTOWAKE=0` to defer reports until the next prompt. RPC clients receive reports on their next prompt. Cancelling the lead's current turn does not cancel asynchronous workers; stop them explicitly or switch sessions.

## Controls and permissions

`/agents` opens the live agent panel. `/agents msg <name-or-id> <text>` sends user steering; `/agents stop <name-or-id>` cancels a worker. Names must resolve to one live agent in the current run; use an ID when names collide. The `agent_status`, `agent_message`, and `agent_stop` tools also accept names or IDs.

Agent messages carry information, never user authorization. They cannot approve plans, permissions, or destructive actions. `/agents msg` is explicit user steering. Workers cannot exceed their parent's permission ceiling. Shared workers receive read-only tools by default; an explicitly permitted profile may grant more. A session-wide mutex serializes turns of profile workers that can write the shared workspace. Background writers require `isolation: "worktree"`.

Teammates receive `agent_status`, `agent_message`, `task_list`, `task_get`, and `task_update` when those tools are available to the parent. Tool availability does not bypass permission checks: read-only workers can inspect the board, while messaging and task mutations require a permitting policy. The lead can create work with `task_create`; permitted teammates claim ready tasks with their agent ID and complete them through `task_update`, honoring the task board's revision checks. `/tasks` shows the board. Workers cannot spawn nested teams.

Worktree leases isolate writers. Successful clean leases are released. Dirty or failed leases are preserved, with their path and branch reported to the lead. Review and merge those branches manually.

## Workflows

`workflow_run` executes a validated phase DAG. Workers within a phase run concurrently up to `max_parallel_agents` (1–8). Parent permissions, model selection, allowed tools, and each worker's `max_turns` reach the host runner; turn limits are clamped to 1–60. Worktree isolation creates real leases. `any` and quorum joins cancel remaining workers, and `deadline_ms` cancels the run. Cancellation is cooperative: an in-flight provider or tool must honor its token.

`max_cost_usd` is rejected because cost enforcement is not implemented. Background completion is reported to the lead; print hosts execute requested background workflows synchronously and say so. Synchronous results include terminal-phase outputs, capped at 8 KiB each and 32 KiB total, with references for spilled artifacts.

`/workflow` (or `/workflow list`), `/workflow status <id>`, and `/workflow cancel <id>` inspect or cancel runs. The command is advertised only when workflow tools are enabled. Start workflows through `workflow_run`; the slash command does not launch them.

## Scope compared with Claude Code teams

| Capability | Davinci |
|---|---|
| Persistent teammate context and peer messages | Supported within the current session |
| Shared task board | `task_*` tools and `/tasks` |
| Team controls | `/agents` panel, `msg`, and `stop` |
| Claude Code panel UI and split panes | Not implemented; Davinci uses its own agent panel |
| Nested teams | Not supported |
| Teammates surviving `/resume` | Not supported; a session switch shuts down the previous team |

See [runtime orchestration](runtime-orchestration.md) for routing and artifact boundaries.
