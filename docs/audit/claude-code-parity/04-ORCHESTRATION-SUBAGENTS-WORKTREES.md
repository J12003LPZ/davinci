# 04 — Orchestration: subagents, teams, workflows, worktrees

Status: evidence record. No code changed for this subsystem in this audit.
Research basis: F-03-01…07, F-04-01…03, F-05-01…05, F-32-01…10, F-33-01…05, F-34-01…09.
Research reports: `03-subagents.md`, `04-agent-teams.md`, `05-dynamic-workflows.md`,
`32-subagent-contract.md`, `33-team-protocol.md`, `34-workflow-durability.md`.

## 1. Spawn model

`AgentSpawnMode` is `Oneshot | Background | Teammate` (`subagent.rs:75-80`).
Compile-time bounds: `SUBAGENT_OUTPUT_CAP = 50 KiB`, `MAX_PARALLEL_TASKS = 8`,
`MAX_TASK_CONCURRENCY = 4` (`subagent.rs:64-70`). Batch width is host-overridable
within 1..=8 (`:300-306`).

Three execution shapes:

1. a single oneshot runs inline and releases a clean worktree (`:1252-1277`);
2. any background/teammate child gets one detached `std::thread` with a start
   handshake and no retained join handle (`:1116-1172`);
3. a multi-oneshot batch runs in parallel lanes, one heading per task, and one
   failure does not abort the rest (`:1307-1414`).

Every child, including a nested one, must take a worker reservation
(`subagent.rs:318-332`) and is durably admitted as an operation with logical id
`subagent:{child_id}` before any thread starts (`:1070-1109`).

## 2. Inheritance (verified against `build_worker`, `main.rs:2593-2821`)

| Property | Behaviour |
|---|---|
| cwd | parent cwd, or the worktree path when isolated (`main.rs:2671`, `:2743-2745`) |
| tools | request/profile ∩ parent ceiling, `agent` always removed (`main.rs:2673-2711`; `subagent.rs:931-936`) |
| permission | parent mode, downgraded for a no-approver worker; `Ask` → `Edits` only with a worktree (`main.rs:2740-2748`, `:2826-2839`) |
| approver | none (`main.rs:2747-2748`) |
| model | explicit override by id, else parent provider/model (`:2772-2803`) |
| max_turns | caller value clamped to 1–60 (`:2667-2669`) |
| thinking level | **not** inherited: a new `Agent` starts at `ThinkingLevel::Off` (`lib.rs:795`) |
| project instruction files | **not** loaded: default disabled (`lib.rs:830`); only the lead enables them (`main.rs:10154`) |
| parent conversation | **not** inherited: `child.session = None` (`main.rs:2713`) |

The last three are documented nowhere in `docs/agent-teams.md`; see `01` §4.

## 3. Nesting

There is no numeric depth counter. Depth 1 is structural:

- `agent` is stripped from every worker (`main.rs:2681`; test `subagent.rs:2014`);
- a nested runner sees its parent's lease held and calls admission with
  `wait = false` (`subagent.rs:319-323`), which fails with
  `"nested worker reservation cannot fit while its parent holds local resources"`
  (`worker_admission.rs:118-124`).

Effective depth 1 is therefore enforced by capability removal **and** admission,
which survives a bug in either one.

## 4. Admission: the weak point

`WorkerAdmission` accounts an **estimate**, never measured RSS
(`worker_admission.rs:1`, `:50-54`, `:111`). Two consequences:

- unconfigured means unlimited: `new(None)` returns `Self::default()` and the
  accounting weight is forced to `0` (`:37-40`, `:97`);
- waiters spin on a condvar in 50 ms slices with **no timeout and no queue cap**
  (`:125-130`); only cancellation exits.

Workflow agent slots are separate: `DEFAULT_MAX_CONCURRENT_AGENTS = 16`, cap 256,
total cap 1 000 (`workflow/limits.rs:13-23`), one shared pool per run
(`executor.rs:813`, `:1900-1948`), and slot exhaustion blocks in 20 ms slices until
cancelled (`:1922-1937`).

## 5. Background lifetime and completion

- Async children are detached OS threads; the handle is dropped after a launch
  check (`subagent.rs:1130-1138`, `:1197-1203`). No supervisor, no reaper.
- Cancellation: a oneshot child is a child of the turn token (`:907-914`), so Esc
  cancels it. Background/teammate children are children of the session root via
  `TeamRoster::admit` (`team.rs:86-110`), so Esc does **not** cancel them; a session
  switch or `agent_stop` does (`team.rs:316-326`, `lib.rs:1032-1036`).
- A subagent result is durable **before** notification:
  `record_worker_result` appends `RuntimeEvent::AgentResultRecorded`
  (`team.rs:577-584`, `mailbox.rs:296-309`), then the mailbox message is queued. If
  the append fails, the worker is marked failed, never silently completed.
- An indeterminate notification is settled, never resent (`team.rs:590-596`).
- Results are cached in a bounded window (50 per worker, 64 workers,
  `team.rs:43-46`); `agent_status` retries pending results, then pages the window,
  then tells the caller that the authoritative copy is the runtime log
  (`tools_agent.rs:79-103`).
- **Workflow completion notices are weaker**: `report_completion` uses only
  `mailbox.send`, with no durable result record (`executor.rs:1422-1429`), and a
  background failure to notify is printed (`:511-517`). Such a notice is not
  rehydratable by `rehydrate_results`.

## 6. Worktrees

- Path is deterministic: `wt-{first8(run_id)}-{agent_key}`, created before the
  `WorktreeCreated` event (`worktree.rs:223-288`). If the path exists, creation fails
  hard (`:228-230`).
- Retention is deliberate: keep on error or dirty, release only on success and clean
  (`subagent.rs:1142-1153`, `:1266-1277`); a preserved lease is reported as a note.
- `is_dirty` treats unreadable state as dirty (`worktree.rs:398-401`), so cleanup
  cannot silently discard work.

Leak paths:

| Path | Evidence | Consequence |
|---|---|---|
| Teardown errors discarded (`git worktree remove`, `remove_dir_all`, `branch -D`) | `worktree.rs:474-491` | directory and branch survive while the in-memory lease is removed |
| Crash/kill mid-run | lease registry is an in-memory map (`:273`, `:519-523`) | orphan directory + branch, invisible after restart; a retry with the same run id fails `PathAlreadyExists` |
| Preserved dirty/failed leases | by design (`docs/agent-teams.md:149`) | unbounded growth over time |

There is no `git worktree prune`, no startup scan, no age-based cleanup, and no test
for the crash path.

## 7. Duplicate workers and double claims

Prevented by five independent mechanisms:

1. durable child admission with stable logical id (`subagent.rs:1070-1109`);
2. task-board claim with revision check and durable receipt (`tasks.rs:1005-1033`);
3. at most one non-worktree writer per batch (`subagent.rs:714-753`);
4. one shared-write lease for all shared-workspace writers, fail-fast, not queued
   (`team.rs:328-409`);
5. idempotent roster admission and at-most-once mailbox send (`team.rs:95-110`,
   `mailbox.rs:334-342`).

Residual: nothing prevents two separate sessions/processes from launching workers in
the same repository; the only arbiter would be git itself. No cross-process lock was
found.

## 8. Workflow durability

Durable: authority, artifacts (manifest + overflow + SHA-256), plan/approval grants
(`state.rs:228-414`), mutation fingerprints spent before a rerun (`executor.rs:548-553`).
Resume reconstructs completed phases from stored artifacts and runs only missing
workers (`:603-621`; test `wor124`).

Not durable: `executions`, `specs`, `launches`, `tokens` are in memory. After a
restart `workflow_status` returns `[]` when no executor is active (`tools.rs:296-305`)
and `resume_execution_with` requires the caller to supply both the id and the spec
again (`executor.rs:554-560`). `max_cost_usd` is rejected, not checkpointed
(`:446-450`, `:487-490`).

Ordering: artifact first, task completion second (`executor.rs:1800-1814`); a journal
failure fails the workflow rather than being swallowed (`:1815-1838`; tests `wor132`,
`wor133`).

## 9. Cancellation

`CancellationToken::cancel` sets the flag before locking, fires callbacks, then
recursively cancels children (`cancellation.rs:86-135`). `child_token` reads the flag
while holding the children lock, closing the check/push race (`:146-159`); children
are `Weak`, pruned amortised (`:152-158`); `detached_child` inherits an ancestor, not
the immediate parent (`:164-173`).

Races handled and tested: cancel between flag set and registration; admit racing
`shutdown_all`; cancellation observed in every bounded wait; a child cancelled before
its operation starts is not executed (`subagent.rs:1500-1513`); teammate stop is
re-checked around each turn (`team.rs:706-784`).

Early join/cancel settles stragglers as `Cancelled` and discards their results after a
5 s grace, but keeps waiting for a started mutating worker with no result
(`executor.rs:1540-1549`, `:1729-1739`).

## 10. Child-claimed success is unverified

A worker is successful if it exits 0 **and** returns non-empty text:

```rust
let (code, error) = print_text_exit(&events);
if code != 0 { return Err(error.unwrap_or_else(|| "subagent failed".into())); }
if text.trim().is_empty() { Err("subagent returned no text".into()) } else { Ok(text) }
```
`main.rs:2841-2854`

No diff inspection, no test run, no artifact existence check, no cross-check that the
files the child names actually changed. A workflow worker is successful if the runner
returned `Ok` **and** its artifact was written (`executor.rs:1800-1834`); the artifact
is whatever text the model produced. The worktree handoff (`snapshot_code` /
`integrate_code`) lets a consumer take the worker's tree; it does not check that the
worker's *report* is true. Teammates post every turn verbatim (`team.rs:709-712`).

This is the single largest orchestration gap and is carried as `ORC-1` in
`07-GAP-MATRIX.md`.

## 11. Test inventory: real state vs fixtures

Real filesystem/journal assertions: `worktree_lease_concurrency.rs`,
`worker_workspace_parity.rs`, `wor_worker_control.rs` (`wor100` torn ledger),
`wor_mailbox_durability.rs`, `operation_agent_launch.rs`, `operation_capacity.rs`,
`agent_tool_resume_integration.rs`, `executor.rs` `wor124/125/132/133/137/138`,
`state.rs` `wor102/103/104/148`, and the four worktree tests in `subagent.rs`
(`:2868`, `:2922`, `:2959`, `:3010`).

In-memory fixtures only: most of `subagent.rs` `mod tests`, `team.rs` tests,
`mailbox.rs` unit tests, `registry.rs`, `cancellation.rs`, `worker_admission.rs`,
`delegation.rs` regex tests, `orchestration_tools.rs`, `delegation_policy.rs`.

Missing coverage, by search:

- parent-process kill with a live background worker, then inspection of the orphaned
  worktree/branch;
- two concurrent `agent` batches from two sessions in one repo;
- memory admission under real processes;
- any assertion that a child's claimed success matches repository state;
- restart of `workflow_status`/resume without a caller-supplied spec.

## 12. Non-claims

No orchestration benchmark was run. The Claude Code constants (depth 3, 20
concurrent agents) are not evidence for DaVinci's numbers; DaVinci's bounds are
different and, in the case of worker admission, weaker in a specific, stated way.
