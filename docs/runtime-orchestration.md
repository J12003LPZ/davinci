# Runtime Orchestration and Routing Policy

This document establishes the architecture, boundaries, and routing policy for multi-agent and workflow execution modes in the Davinci agent runtime (`pi-rust`).

---

## 1. Orchestration Modes Overview

Davinci provides five distinct execution modes, each tailored to a specific operational scale, autonomy model, and reliability contract:

| Mode | Entry Point / Tool | Autonomy & Lifespan | Context Model | Isolation & Mutation Boundary |
| :--- | :--- | :--- | :--- | :--- |
| **Normal Agent** | CLI, `/act`, interactive turn | Synchronous foreground turn loop | Monolithic conversation history with Token Governor compression and turn compaction | Shared workspace, synchronous in-turn mutation barriers |
| **One-Shot Subagent** | `agent` tool (`mode: "oneshot"` or `"background"`) | Bounded ephemeral worker | Isolated throw-away context; returns compact summary string (capped at 50 KB) | Scoped tool allowlist; read-only default; worktree lease optional |
| **Agent Teams** | `agent` (`mode: "teammate"`), `/team`, `task_*`, `agent_message` | Multi-agent collaborative session | Per-agent scoped context, peer mailboxes, event-driven wakeup dispatch | Shared runtime task board; atomic task claiming; message passing |
| **Workflows** | `workflow_run`, `workflow_status`, `/workflow` | Multi-phase repeatable DAG pipeline | Phased execution; intermediate artifacts stored outside model context in Governor-backed store | Worktree isolation required for parallel writers; join policies (All, Any, Quorum); retry budgets |
| **Graph** | `graph_run`, `/graph <goal>` | Deterministic multi-stage engineering graph | Isolated worker child processes (`--no-session --no-extensions --no-skills`); bounded 2,500 token context packets | Rigid deterministic pipeline (`classify -> investigate -> plan -> implement -> verify -> review`), revision loops, security audits, review coverage |

---

## 2. Routing Decision Matrix

When an agent or operator determines how to approach a task, the runtime enforces the following routing criteria:

```text
                        ┌───────────────────────────────┐
                        │   Incoming Task or Goal       │
                        └───────────────┬───────────────┘
                                        │
                    Is it a direct user interaction or
                    standard interactive coding task?
                                       / \
                                Yes   /   \  No
                                     v     v
                         [ Normal Agent ]  Need automated multi-step
                                           coordination?
                                                / \
                                         Yes   /   \  No
                                              v     v
                  Is it bounded, read-only     [ One-Shot Subagent ]
                  information retrieval?
                                 / \
                          Yes   /   \  No
                               v     v
             Does it require code mutation with
             rigorous plan/verify/review gates?
                           / \
                    Yes   /   \  No
                         v     v
                  [ Graph ]    Is it an open-ended multi-agent
                               collaboration or repeatable DAG?
                                    / \
                     Open-ended    /   \  Repeatable DAG
                                  v     v
                          [ Teams ]   [ Workflow ]
```

### Policy Definitions:

1. **Normal Agent (Default)**:
   - **When to use**: Standard interactive user turns, simple bug fixes, explanatory answers, and direct file edits.
   - **Prohibited**: Do not spawn heavy background machinery or complex DAGs for tasks solvable in 1–3 direct tool calls.

2. **One-Shot Subagent (`agent`)**:
   - **When to use**: Bounded independent research, reading large files/documentation, or exploring codebases where broad searches would pollute the primary agent context.
   - **Invariants**: Cannot mutate the repository unless explicitly granted mutating tools and worktree isolation. Output is strictly bounded to 50 KB.

3. **Persistent Agent Teams (`mode: teammate`)**:
   - **When to use**: Multi-perspective interactive collaboration (e.g., Architect designing while Reviewer critiques and Tester writes specs), shared task assignment boards, and asynchronous agent-to-agent messaging.
   - **Invariants**: Agents communicate via typed `AgentMessage` mailboxes; lifecycle transitions publish to the runtime `EventBus`; tasks are tracked in `RuntimeRegistry`.

4. **Dynamic General Workflows (`workflow_run`, `/workflow`)**:
   - **When to use**: Repeatable, structured multi-phase orchestration pipelines with intermediate data handoffs (e.g., triage -> analyze -> synthesize report).
   - **Invariants**: Declared as deterministic DAGs (`WorkflowSpec`); validated prior to execution; supports fan-out/fan-in joins (`All`, `Any`, `Quorum`); intermediate artifacts are stored in Rust (`WorkflowStateStore`) and never dumped into model context.

5. **Deterministic Graph Engineering (`/graph`, `graph_run`)**:
   - **When to use**: Automated codebase modifications requiring strict structural guarantees, independent verification commands, reviewer coverage tracking, security scans, and deterministic revision loops.
   - **Invariants**: Controllers are pure Rust; model calls happen only inside isolated child processes; changes must pass security audit gates and unit tests before review.

---

## 3. Boundary & Isolation Invariants

### 3.1 Graph Internals Isolation from Workflows

Graph execution enforces rigorous internal invariants (least privilege per role, child worker hooks, baseline capture, and revision counters). Workflows and teams **must not** invoke Graph internal components directly:

1. **Prohibited Internal Tools**:
   - `graph_submit`: An internal exit door exclusive to Graph worker child processes. Workflows declaring `graph_submit` are rejected at validation time with `GraphInternalToolRejected`.
   - Direct execution of graph worker roles outside of the Graph controller is forbidden.

2. **Exposed Entry Point Boundary (`graph_run`)**:
   - A general workflow or agent may invoke Graph capabilities **only** as a complete, self-contained black box via `graph_run`, and only when `graph_run` is explicitly exposed in the permitted toolset.
   - When invoked, `graph_run` executes the full, immutable Graph lifecycle with its own isolated processes, verification tests, and review loops.
   - Because `graph_run` can mutate the repository, it is classified as a mutation tool (`ToolClass::Edit` / mutation tool), requiring worktree isolation if run concurrently with other workers.

### 3.2 Artifact Context Protection

A critical failure mode of multi-agent LLM systems is **context accumulation**, where output from earlier stages is concatenated into downstream prompts, causing prompt bloat, cache thrashing, and context window exhaustion.

- **Rust-Owned Artifact Store**: The workflow engine stores all intermediate phase outputs in `WorkflowStateStore`.
- **Digest Referencing**: Worker prompts receive references or digests (`governor://` or artifact URIs) rather than raw blobs.
- **Size Bounds**: Artifacts exceeding 64 KB are automatically spilled to Token Governor-backed overflow storage; a 1 MB artifact produces `< 200` bytes of reference metadata in worker prompts.

### 3.3 Mutation Ownership and Worktree Leases

To guarantee that parallel agents do not corrupt working trees or produce race conditions:

- **Single Writer Invariant**: In any shared workspace, at most one mutating agent may execute at any given time.
- **Worktree Leases**: Multiple parallel mutating workers (in workflows or teams) are permitted **only** if each worker acquires an isolated `WorktreeLease` (`isolation: "worktree"`).
- **Lease Cleanup**: Clean leases are released upon successful task completion; failed or dirty worktrees are preserved for operator inspection.

### 3.4 Permission Ceilings and Project Trust

- **No Privilege Escalation**: Subagents, team workers, and workflow phases inherit the parent session's `PermissionMode`. A session in `ReadOnly` mode rejects any workflow or agent task that requests mutating tools.
- **Untrusted Projects**: In an untrusted checkout, project-level workflows (`.davinci/workflows/*.json`) and custom agent profiles cannot be loaded or executed until the user explicitly runs `/trust`.
