# Subagents, agent teams and workflows

Davinci delegates work the way Claude Code does: the model starts subagents on its own when a task needs them, a user can forbid that at any time, and every delegated worker is visible while it runs.

## Turning features on

One-shot and background subagents are always available. Agent teams and dynamic workflows are opt-in. Turn them on in `/config` (the behavior group) or in `~/.davinci/agent/settings.json`:

| Setting | `/config` row | Values | Default | Environment override |
|---|---|---|---|---|
| `agentTeams` | Agent teams | `true` / `false` | `false` | `DAVINCI_EXPERIMENTAL_AGENT_TEAMS` |
| `dynamicWorkflows` | Dynamic workflows | `true` / `false` | `false` | `DAVINCI_EXPERIMENTAL_WORKFLOWS` |
| `workflowSizeGuideline` | Dynamic workflow size | `unrestricted`, `small` (< 5 agents), `medium` (< 10), `large` (< 50) | `medium` | — |
| `workflowMaxConcurrentAgents` | Workflow concurrent agents | 1–256 (the sheet cycles 4, 8, 16, 32, 64) | `16` | `DAVINCI_WORKFLOW_MAX_CONCURRENT_AGENTS` |

An environment variable, when set, wins over the setting, as Claude Code's environment variables do. A change in `/config` applies to the running session: turning a feature on offers its tools (`agent_status`, `agent_message`, `agent_stop`, `task_*` for teams; `workflow_run`, `workflow_status` for workflows) to the lead at once, and turning it off withdraws them. Workers and explicit `--tools` lists keep exactly the tools they were given.

```json
{
  "agentTeams": true,
  "dynamicWorkflows": true,
  "workflowSizeGuideline": "small",
  "workflowMaxConcurrentAgents": 8
}
```

## When Davinci delegates, and how to stop it

The system prompt tells the model to use `agent` workers on its own initiative when a task needs broad searching across many files or several independent investigations that would flood its context, and not to delegate what a few direct calls finish.

Tell Davinci not to use subagents and it stops. Phrases such as "don't use subagents", "no agents please", "do it without subagents", "never spawn workers" or "don't delegate" forbid delegation for the rest of the conversation. The prompt carries the rule, and a deterministic check (`crates/davinci-agent/src/delegation.rs`) backs it: while forbidden, every `agent`, `workflow_run` and `graph_run` call is refused with a message telling the model to do the work itself, including launches inside a batch. Say "you can use subagents again" (or "feel free to spawn agents", "subagents are fine now") to lift it. Status and stop tools remain available.

- Only your own messages count. A message relayed from another agent cannot forbid or allow delegation.
- The latest directive wins, including within one message.
- Mentions are not directives: "read AGENTS.md", "don't modify the agents directory" and "no agent profile found" change nothing.
- The policy survives compaction and `/resume`: it is rebuilt from your original messages on the selected session branch, even when they are absent from the model's context. Summaries and abandoned branches cannot change it.
- Conversation rewind restores the policy at the selected checkpoint. Saved sessions use the original branch; unsaved conversations keep the policy alongside their in-memory conversation checkpoints. Code-only rewind leaves the current policy in place.
- To disable delegation permanently, add a deny rule: `"permissions": {"deny": ["agent", "workflow_run", "graph_run"]}`.

Offline regressions live in `crates/davinci-agent/tests/delegation_policy.rs` and
`crates/davinci-evals/tests/agent_orchestration.rs`. They exercise compacted resume,
branch selection, saved and unsaved rewind, both tool-dispatch paths, and a
24-case direct/batched launch matrix across Astra, generic OpenAI and Anthropic
profiles. Provider replies and graph executors are fixtures; these tests check
runtime enforcement, not live model quality or task-success rates. Stable prompt
text and provider tool schemas are unchanged by these enforcement fixes.

## Watching subagents work

A running `agent` call draws its workers under the call, as Claude Code does:

```text
● Agent(map auth)
  ⎿  Search("session")
     Read(src/auth/login.rs)
     Read(src/auth/token.rs)
     +4 more tool uses (ctrl+t to expand)
```

When the worker finishes, the block collapses to one line:

```text
● Agent(map auth)
  ⎿  Done (7 tool uses · 23.4k tokens · 41s)
```

Several tasks in one call draw as a tree:

```text
● Agent(3 tasks)
   ├─ map auth · 4 tool uses · 12.1k tokens
   │  ⎿  Read(src/auth.rs)
   ├─ map db · 7 tool uses · 23.4k tokens
   │  ⎿  Done (7 tool uses · 23.4k tokens · 41s)
   └─ check tests
      ⎿  Initializing…
```

`ctrl+t` shows more recent calls and the worker's answer. A background worker shows `Running in background · /agents`. The status bar counts what still runs in the background: `1 job · 2 agents · 1 workflow`.

The progress travels as a Davinci-only `subagent_progress` agent event keyed by the lead's `agent` call. JSON and RPC hosts receive it too and may ignore it.

## Modes and lifecycle

The `agent` tool accepts `oneshot`, `background` and (with agent teams on) `teammate`. Unknown modes are errors. A call can carry up to eight tasks, four running at once. One-shot calls return their result directly. Background workers run once and report their result to the lead's mailbox. `--print` rejects `background` and `teammate`, because a print run cannot keep them alive.

Spawn a collaborator with `{"prompt":"Review the parser","mode":"teammate","name":"reviewer"}`. A teammate keeps its conversation between turns. It runs, reports, goes idle, and waits on its mailbox. `agent_message` or `/agents msg reviewer summarize your findings` wakes it. Reports are labeled with the sender:

```text
<agent-message from="reviewer" agent_id="…" kind="teammate">
status: completed

Findings and file references
</agent-message>
```

A failed turn reports `status: failed`. A teammate leaves after three failed turns in a row, on `agent_stop` or `/agents stop reviewer`, on a session switch, or after an idle timeout (`DAVINCI_TEAMMATE_IDLE_TIMEOUT_MS`, default 30 minutes). The lead then gets one `status: stopped` notice saying why it left.

An idle interactive lead with an empty composer starts a turn 750 ms after reports arrive. Typing or an open sheet holds it back. `DAVINCI_AGENT_TEAMS_AUTOWAKE=0` defers reports to the next prompt. RPC clients receive reports with their next prompt. Pressing Esc on the lead cancels its turn, not its background workers; stop those explicitly or switch sessions. A `--no-session` conversation keeps its team across prompts too.

## Controls and permissions

`/agents` opens the live agent panel. `/agents msg <name-or-id> <text>` sends user steering. `/agents stop <name-or-id>` stops a worker; the lead itself cannot be stopped this way. Names must match one live agent in the current run; use the ID when names collide.

Agent messages carry information, never user authorization: they cannot approve plans, permissions or destructive actions. Workers never exceed the lead's permission ceiling.

- Every worker reads and edits files. A worker that names no tools gets `read`, `grep`, `find`, `ls`, `web_fetch`, `web_search`, `mcp_read`, `write` and `edit` (those the lead has). Workers are read-only only when the lead is in Plan Mode.
- A worker in the shared workspace runs in Accept Edits mode with file tools (`write`, `edit`, `notebook_edit`, `apply_patch`) but no shell. A session-wide lease lets only one shared-workspace writer edit at a time: a worker takes it at its first mutating call and releases it when its turn ends, so read-only work still runs in parallel.
- A profile without `permission_mode` (or with `permission_mode: inherit`) follows the same rule. A profile that sets `read-only` or its own `tools` list keeps that restriction.
- A worker with `isolation: "worktree"` edits inside its own lease. It is read-only only when the lead is in Plan Mode. In the default Manual mode it may edit, because nothing reaches your tree until you merge its branch.
- Background workers in the shared workspace edit through the same lease; use `isolation: "worktree"` to keep their changes off your tree until you merge them.
- Clean leases are released. Dirty or failed leases are kept, and their path and branch are reported. Review and merge them yourself. Leases are rooted at the repository top even when Davinci starts in a subdirectory.

Teammates get `agent_status`, `agent_message`, `task_list`, `task_get` and `task_update` when the lead has them. The lead creates work with `task_create`; teammates claim ready tasks with their agent ID and complete them with `task_update`. `/tasks` shows the board. Workers cannot start nested teams.

## Workflows

`workflow_run` runs a validated phase DAG. Workers in a phase run in parallel, up to the spec's `max_parallel_agents` (1–256) and never more than `workflowMaxConcurrentAgents`. A run may schedule up to 1,000 agents. The size guideline is added to the tool's description, so the model aims for that many agents unless you ask for more. A run that schedules more than 25 agents (or more than your chosen guideline) carries a `Large workflow` warning; the warning never blocks the run.

- Parent permissions, model, allowed tools and each worker's `max_turns` (1–60) reach the workers.
- `isolation: "worktree"` creates real leases.
- `any` and quorum joins cancel the remaining workers.
- `deadline_ms` cancels the run.
- `max_cost_usd` is rejected, because cost enforcement is not implemented.

A background run belongs to the conversation, not to the turn that started it. Esc leaves it running, `/workflow cancel` and a session switch stop it, and its completion is reported to the lead. Print hosts run background workflows synchronously and say so.

| Command | What it does |
|---|---|
| `/workflow <saved-name>` | Runs `.davinci/workflows/<name>.json` (or `~/.davinci/agent/workflows/`) in the background. |
| `/workflow <goal>` | Asks the model to build and run a workflow for the goal. |
| `/workflow list`, `status <id>`, `cancel <id>` | Text answers. |
| `/workflows` (or bare `/workflow`) | Opens the workflows view. |
| `/workflow-stop <id>`, `/workflow-resume <id>` | Stop or resume a run. |

The workflows view follows Claude Code's progress view and refreshes every second:

- **Runs:** each run with its status, phases done, agent count, tokens and elapsed time.
- **Phases:** each phase with its status, `done/total agents`, tokens and elapsed time.
- **Agents:** each agent with its status, tool uses, tokens and elapsed time, and the latest call under the selected row.
- **Agent detail:** recent calls, and the result or error.

| Key | Action |
|---|---|
| `↑` / `↓` | Select a run, phase or agent |
| `enter` or `→` | Drill in; in the agent detail, expand the result and all calls |
| `esc` or `←` | Back out one level; `esc` at the run list closes the view |
| `j` / `k` | Scroll |
| `f` | Filter agents: all, running, completed, failed |
| `p` | Pause (no new agent starts; running ones finish) or resume |
| `x` | Stop the selected agent (it counts as failed), or the run above the agent level |

## Scope compared with Claude Code

| Capability | Davinci |
|---|---|
| Automatic delegation, and "don't use subagents" | Supported; a deterministic check backs the prompt |
| Live subagent progress under the call | Supported |
| Workflows progress view (`/workflows`) | Supported, with pause, stop and filter |
| Workflow size and concurrency settings | `workflowSizeGuideline`, `workflowMaxConcurrentAgents` |
| Persistent teammates and peer messages | Supported within the current session |
| Shared task board | `task_*` tools and `/tasks` |
| Saving a run as a command (`s`) | Not implemented; save specs with `workflow_run`'s `save_as` |
| JavaScript workflow scripts | Not implemented; workflows are JSON phase specs |
| Split panes, agent view across sessions | Not implemented |
| Nested teams, teammates surviving `/resume` | Not supported |

See [runtime orchestration](runtime-orchestration.md) for routing and artifact boundaries.
