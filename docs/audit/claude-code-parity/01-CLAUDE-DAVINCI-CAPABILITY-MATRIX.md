# 01 — Claude Code 2.1.310 vs DaVinci: capability matrix

Status: audit result. One DaVinci change is recorded in `10-IMPLEMENTATION-RECORD.md`.
Audit date: 2026-10-27.
DaVinci revision audited: `J12003LPZ/cc-tui-parity`, HEAD `6912f940` (dirty files listed in §6).

## 1. Method and provenance

Every row compares a **research finding** (Claude Code 2.1.310, executable
`F9A39206…EBA61`) with **DaVinci source at the audited revision**.

Three kinds of evidence are labelled separately, because they are not the same
strength of claim:

| Tag | Meaning |
|---|---|
| `RUN` | Test executed during this audit session; result recorded in `11-VERIFICATION-RECORD.md` |
| `TEST` | Test source exists and asserts the property; not executed for that subsystem in this session |
| `CODE` | Read from source; no test found that pins the behaviour |
| `AUDIT` | Established by a read-only subsystem audit of the same revision, with `file:line`; spot-checked but not re-executed |

The MCP rows were re-read line by line for this audit (`crates/davinci-mcp/src/lib.rs`,
`stdio.rs`, `crates/davinci-agent/src/mcp.rs`, `turn.rs`, `tool_ledger.rs`) and are
covered by `RUN` tests. The context, orchestration, session and evaluation rows are
`TEST`/`CODE` and rest on the subsystem audits plus the cited files.

No live model, subscription or network call was made. No benchmark was executed.
Nothing in this document measures task success, token cost or subscription savings.

## 2. Status vocabulary

| Status | Definition |
|---|---|
| **IMPLEMENTED** | The mechanism exists, is reachable from the production path, and at least one test asserts it (`RUN` or `TEST`) |
| **WEAK** | The mechanism exists but a named guarantee is missing or conditional (the condition is stated in the row) |
| **PARTIAL** | Some of the mechanism exists; the row names exactly what is absent |
| **MISSING** | No implementation found |
| **DIFFERENT** | DaVinci solves the same problem by another mechanism; the difference is stated, not scored |
| **N/A** | The finding is a property of that product/provider and has no DaVinci requirement |

`DIFFERENT` is not a euphemism for missing. Where DaVinci's alternative is weaker
for a stated hazard, the row says **WEAK** or **PARTIAL** instead.

## 3. Matrix

### 3.1 MCP client: replay, identity and unknown outcomes

Research: `20-mcp-replay-safety.md`, `31-mcp-concurrent-recovery-and-collateral-replay.md`,
`42-mcp-cancellation-and-idle.md`; findings F-20-01…10, F-31-01…11, F-42-04…07, F-06-06…11.
Raw evidence: `re-claude-code/_artifacts/e2s4/`, `e2s5/`, `e2s6/`.

| # | Finding | Claude Code behaviour | DaVinci status at `6912f940` | Evidence |
|---|---|---|---|---|
| M1 | F-20-01 | One automatic replay (two attempts) | **N/A / DIFFERENT** — DaVinci has no call replay at all | `crates/davinci-mcp/src/lib.rs` (no retry loop); `AUDIT` |
| M2 | F-31-03, F-31-05, F-31-11 | A peer call can be re-sent when a shared client is torn down; completed peer results are discarded | **DIFFERENT (safe side)** — one `Client` per server behind a `Mutex`; a transport error evicts that server and its routes; in-flight calls on the same client fail, but nothing is re-sent | `crates/davinci-agent/src/mcp.rs:487-500` `Inner::failed`; `RUN` `a_lost_mcp_reply_becomes_may_have_run_not_failed` |
| M3 | F-20-03, F-20-04 | stdio crash after commit and before commit both read as `Connection closed`; raw HTTP faults are never retried | **IMPLEMENTED (this audit)** — `tools/call` failures are classified: JSON-RPC error and provably-unqueued writes stay definite; everything else becomes `Ambiguous` | `crates/davinci-mcp/src/lib.rs:239-257`; `RUN` `a_lost_tool_reply_is_ambiguous_and_a_refused_call_is_not` |
| M4 | F-20-07 | Only one HTTP path produces a "may have run" warning | **IMPLEMENTED (this audit)** — the model sees an explicit unknown-outcome sentence and `details.outcome_unknown=true` | `crates/davinci-agent/src/tools.rs:365-403`; `RUN` `an_error_that_may_have_reached_the_tool_is_reported_as_unknown` |
| M5 | F-20-05, F-31-07 | HTTP 404 is treated as `mayHaveRun=false` and replayed, duplicating a mutation | **PREVENTED** — DaVinci never derives "did not run" from a status code; every non-RPC, non-refused failure is ambiguous | `crates/davinci-mcp/src/http.rs:127-145` → `Error::Transport` → `Ambiguous`; `RUN` M3 |
| M6 | F-20-08, F-20-09 | `idempotentHint`/`readOnlyHint` can convert at-most-once into at-least-once | **PREVENTED by absence** — annotations are parsed into tool metadata but are not used to authorise replay, because there is no replay | `crates/davinci-mcp/src/lib.rs` tool parsing; `CODE` |
| M7 | F-06-06 | Every `tools/call` carries `_meta` `toolUseId` and a progress token | **MISSING** — no `_meta` or `progressToken` is sent, so a compliant server cannot dedupe a repeated invocation | `rg "_meta|progressToken" crates/davinci-mcp/src` → only `notifications/cancelled`; `CODE` |
| M8 | F-06-08 | Bounded background reconnects | **MISSING (by design)** — a failed server stays evicted for the session; there is no reconnect or re-dial | `crates/davinci-agent/src/mcp.rs` `drop_server`; `CODE` |
| M9 | F-06-10 | An MCP crash after init is not retried within the crashing call | **MATCHES** — the call fails, the server is evicted, and the model is told the outcome is unknown when delivery was possible | `RUN` M3, M4 |
| M10 | F-42-01…03, F-42-07 | Cancellation on timeout sends `notifications/cancelled` | **PARTIAL** — `call_timeout` sends `notifications/cancelled` best-effort; server-side cancellation acknowledgement is not awaited and stdio cancellation during teardown is not tested | `crates/davinci-mcp/src/stdio.rs:274-281`; `TEST` `wor51_*` |
| M11 | F-20-10 | A model-initiated retry is a new invocation (`toolUseId`, args, handshake) | **MATCHES** — DaVinci model retries mint a new tool-call id and the ledger treats it as a new attempt | `crates/davinci-agent/src/tool_ledger.rs`; `TEST` |

**Net effect.** DaVinci is safer than Claude Code 2.1.310 on the class-E replay
hazards (M2, M5, M6) because it has no replay path at all. It pays for that with a
capability loss the research does not require: a call that provably never executed
(e.g. a refused write) is also not retried automatically, so the model must re-issue
it. M7 (no `_meta` invocation id) is the one place where a duplicate executed
*server-side* would be invisible to a cooperating server; DaVinci's report is honest
only because it never replays.

### 3.2 Context assembly, prompt cache and compaction

Research: `07-context-assembly.md`, `08-prompt-cache-and-tools.md`,
`09-session-persistence.md`, `35-auto-compact.md`, `41-token-cache-effectiveness.md`;
findings F-07-01…05, F-08-01…05, F-09-01…04, F-35-01…05, F-41-01…07.

| # | Finding | DaVinci status | Evidence |
|---|---|---|---|
| C1 | F-07-01, F-08-03 | **IMPLEMENTED** — stable prompt text stays in `instructions`; per-turn state (mode, plan revision, environment, date) is appended as `Dynamic` | `prompt/turn.rs:38-61`, `prompt/runtime_state.rs:37-114`, `lib.rs:2423-2437`; `TEST` `cache_stability.rs:138-274` |
| C2 | F-08-04 | **IMPLEMENTED** — Responses tools sorted by name with a schema digest; registry lists name-sorted; `serde_json` uses `BTreeMap` | `davinci-ai/src/responses_tools.rs:98-120`; `TEST` `request_shape.rs:90-152` |
| C3 | F-41-01 | **IMPLEMENTED** — the cache-sensitive split is gated to OpenAI reasoning routes; other routes keep state in the system prompt by design | `turn_context.rs:29-41`; `TEST` `cache_stability.rs:277-296` |
| C4 | F-08-05 (deferred tool schemas) | **WEAK** — progressive exposure exists but the default surface is `Full`; laziness is opt-in (`DAVINCI_TOOL_SURFACE=lean`) | `lib.rs:437-441`, `lib.rs:4343-4393`; `TEST` `lean_tool_surface.rs:35`, `mcp_progressive_disclosure.rs:29` |
| C5 | F-08-05 (skills) | **PARTIAL** — skill bodies are deferred from the prompt but all `SKILL.md` bodies are read at startup, so only context cost is deferred, not I/O | `skills.rs:131-158`, `:195`; `TEST` `skills.rs:569-582` |
| C6 | F-07-02 (child isolation) | **IMPLEMENTED** — workers start from a fresh agent, no parent transcript, no session | `davinci-coding-agent/src/main.rs:2514-2518`, `:2664-2713`; `TEST` `runtime/mod.rs:893` |
| C7 | F-35-01…04 (auto-compaction) | **IMPLEMENTED** — keep-recent+reserve floor, output threshold validation, provider-count trigger in hybrid, one overflow retry, backoff | `compaction.rs:216-307`, `:477-559`, `turn.rs:764-861`, `:967-985`; `TEST` `context_vm_hybrid.rs:634,656` |
| C8 | F-35-05 (compaction summary request) | **PARTIAL** — summary quality is not validated or measured; docs say so | `context-vm-measurements.md:138`, `:243-247` |
| C9 | F-07-03 (user restrictions survive compaction) | **WEAK** — strong grounding only on the Context VM path; the **default** mode is `Off`, where compaction is a plain summariser with a prompt section and no validation | `context_vm/types.rs:40`, `fold.rs:64`, `reducer.rs:333-338`; `TEST` `context_vm_integrity.rs:46-363` |
| C10 | F-09-04, F-36-08 (crash during compaction) | **IMPLEMENTED** — the session entry is committed before live history is truncated; fold chunks checkpoint; a failed write rolls back | `lib.rs:5441-5480`, `:4823-4871`; `TEST` `wor61_failed_checkpoint_persistence_rolls_the_fold_back` |
| C11 | F-41-03, F-41-06 (cache evidence) | **PARTIAL (accounting yes, measurement no)** — cached tokens are parsed, persisted and reported with an explicit unknown state; no live cache measurement exists | `stream_decoder.rs:927-975`, `turn.rs:5373-5439`, `cache_stats.rs:26-200`; `TEST` `cache_stats.rs:602-720` |
| C12 | F-41-06, F-41-07 | **NOT ESTABLISHED** — no billed-token or subscription saving is measured anywhere in this repository | `openai-efficiency.md:198,233`; `CODE` |

### 3.3 Orchestration: subagents, teams, workflows, worktrees

Research: `03-subagents.md`, `04-agent-teams.md`, `05-dynamic-workflows.md`,
`32-subagent-contract.md`, `33-team-protocol.md`, `34-workflow-durability.md`;
findings F-03-01…07, F-04-01…03, F-05-01…05, F-32-01…10, F-33-01…05, F-34-01…09.

| # | Finding | DaVinci status | Evidence |
|---|---|---|---|
| O1 | F-03-07, F-32-03, F-32-04 | **DIFFERENT (equivalent bound)** — no depth counter; depth 1 is enforced by removing `agent` from every worker and by non-waiting admission for a nested lease | `davinci-coding-agent/src/main.rs:2681`, `subagent.rs:319-323`, `worker_admission.rs:118-124`; `TEST` `subagent.rs:2014` |
| O2 | F-32-04 (admission) | **WEAK** — a memory reservation exists, but it is an estimate, unlimited when unconfigured, waits without timeout and has no queue cap | `worker_admission.rs:37-40`, `:97`, `:125-130`; `CODE` |
| O3 | F-03-01, F-33-01 | **DIFFERENT** — background children are detached threads owned by the process; Esc on the lead cancels oneshot children only; a session switch stops the roster | `subagent.rs:907-914`, `:1130-1138`, `team.rs:316-326`; `TEST` |
| O4 | F-03-02, F-32-06, F-33-05 | **PARTIAL** — a subagent report is journal-first (`AgentResultRecorded` then mailbox) and rehydratable; a **workflow** completion notice is mailbox-only and not rehydratable | `team.rs:577-603`, `mailbox.rs:296-309`, `executor.rs:1422-1429`; `TEST` `wor_mailbox_durability.rs` |
| O5 | F-34-05, F-34-08 | **PARTIAL** — artifacts and approvals are content-addressed and resume is serialised; execution state (`executions`, `specs`, `launches`) is in memory, so `workflow_status` is empty after restart and resume needs the caller to re-supply id and spec | `state.rs:228-414`, `executor.rs:548-621`, `tools.rs:296-305`; `TEST` `wor124`, `wor125` |
| O6 | F-32-08 (never publish an unwritten result path) | **PREVENTED** — a completed task always has its artifact on disk first; a failed save fails the workflow | `executor.rs:1800-1838`; `TEST` `wor132`, `wor133` |
| O7 | F-32-05, F-32-10 (child success) | **MISSING** — a worker is "successful" if it exits 0 **and** returns non-empty text; there is no diff, test or artifact check of the claim | `main.rs:2841-2854`; `CODE` |
| O8 | Worktree isolation (no research ID; hazard audited) | **WEAK** — leases are deterministic and preserved on failure, but teardown errors are discarded and there is no reaper, so a crash leaves a directory and branch invisible to the manager | `worktree.rs:474-491`, `:228-230`, `:519-523`; `CODE` |
| O9 | Duplicate workers (F-32-06 area) | **IMPLEMENTED** — durable child admission with stable logical id, task-board revision claims, one shared-writer lease, mailbox at-most-once per message id | `subagent.rs:1070-1109`, `tasks.rs:1005-1033`, `team.rs:328-409`, `mailbox.rs:334-342`; `TEST` `operation_agent_launch.rs` |
| O10 | Cancellation tree | **IMPLEMENTED** — flag-before-lock, `Weak` children, child-of-cancelled-parent closes the check/push race | `cancellation.rs:86-173`; `TEST` |

### 3.4 Session durability, permissions and local IPC

Research: `09-session-persistence.md`, `11-permission-modes.md`, `36-session-recovery.md`,
`37-permission-decisions.md`, `38-ide-ipc.md`; findings F-09-01…04, F-11-01…04,
F-36-01…08, F-37-01…06, F-38-01…12.

| # | Finding | DaVinci status | Evidence |
|---|---|---|---|
| S1 | F-09-04 | **IMPLEMENTED** — intent is committed before the adapter runs; outcome, event, result, outbox and claim release are one transaction; `synchronous=FULL` + WAL | `runtime/operations/store.rs:99-160`, `:243-388`; `TEST` `operation_journal.rs:171,211` |
| S2 | F-36-08, F-20-03 area | **IMPLEMENTED** — a failed completion records `EffectStatus::Unknown`; an unreadable journal is treated as unknown (fail-safe) | `adapters/tools.rs:255-263`, `turn.rs:501-552`; `TEST` `agent_tool_resume_integration.rs:229` |
| S3 | F-37-01, F-37-02 | **IMPLEMENTED** — deny precedes allow, including `AlwaysApprove`; the single dispatch choke point is shared by direct, batch, codemode and MCP calls | `permission.rs:1079,1239-1338`, `turn.rs:2569-2652`; `TEST` (deny tests on prepare + engine) |
| S4 | F-11-02, F-11-03 | **DIFFERENT** — no `bypassImmune` removal class; `ReadOnly` hard-denies, and any mode change re-binds a policy revision on the journal route | `permission.rs:1290`, `adapters/tools.rs:159-178`; `CODE` |
| S5 | F-37-06 (approval caching/expiry) | **N/A (different product)** — one-shot approvals only; no approver means deny | `turn.rs:4448-4461`; `TEST` |
| S6 | Never-auto-replay | **IMPLEMENTED** — default policy `NeverAutoReplay`; the only re-executing recovery action requires safe class + quiescent owner + claimed-unstarted + `EffectStatus::NotStarted` | `runtime/capabilities.rs:60-64`, `recovery.rs:101-115`; `TEST` `operation_properties.rs:363` |
| S7 | F-09-01…03 | **DIFFERENT** — session history is JSONL with `O_APPEND` + `sync_all` and a torn-tail backup; operations live in a separate SQLite journal; a configured sqlite session backend is warned about and ignored | `davinci-session/src/lib.rs:469-474`, `:647-697`, `settings.rs:1316-1338`; `TEST` `operation_journal.rs`, `context_vm_durability.rs` |
| S8 | F-36-01…07 | **IMPLEMENTED** — unpaired calls are answered on load with `unknown` or `not executed`, orphans fail closed unless the journal knows them | `turn.rs:330-474`; `TEST` `tool_result_pairing_recovery.rs:195-392` |
| S9 | F-38-01…12 | **PARTIAL** — Unix-domain socket only, non-default feature, `0700` dir + `0600` socket, connection-level `AUTH` compared with `==`, **empty token disables auth**, no peer-credential check, no Windows named pipe | `davinci-server/src/unix.rs:16,161,210`, `lib.rs:1036-1046`, `experimental.rs:384`; `CODE` |
| S10 | F-11-04 (cleanup on signal) | **PARTIAL** — drop-based cleanup exists, but the SIGTERM/SIGHUP handler calls `process::exit`, skipping `Drop`; the foreground spawn job has no `KILL_ON_JOB_CLOSE` | `shutdown.rs:37-45`, `davinci-sys/src/process.rs:212-239`; `CODE` |

### 3.5 Coding-task success: completion, verification, measurement

Research: `40-effectiveness-and-quota.md`, `41-token-cache-effectiveness.md`,
`15-performance.md`, `16-token-and-cost.md`; findings F-40-01…07, F-41-06/07.

| # | Finding | DaVinci status | Evidence |
|---|---|---|---|
| K1 | F-40-05 (verified success) | **PARTIAL (harness exists, no results)** — oracle-driven scenario runner with baseline-must-fail, real product process, independent re-verification; no campaign artifacts exist | `davinci-evals/src/behavior/executor.rs:116-277`; `docs/readiness/evaluation.md:100-106` |
| K2 | F-40-05 | **MISSING (results)** — the offline corpus cannot produce a meaningful verified success: all 200 `core-200.json` scenarios have empty `verification_commands` and fall back to `git diff --check`; 121 use a repo fixture containing only `README.md` | `scenario.rs:108-120`, `fixtures/behavior/core-200.json`, `fixtures/repos/core-repo/`; `CODE` |
| K3 | F-40-06 | **NOT ESTABLISHED** — no client-vs-client comparison has been run | `docs/readiness/evaluation.md:148-152`; `CODE` |
| K4 | F-40-07 | **PARTIAL (metric, no values)** — `cost_per_verified_success` exists in Rust and Python, includes failed-attempt spend, returns `None` on unknown cost; no campaign populates it | `davinci-evals/src/optimization.rs:34-90`, `scripts/bench/readiness_metrics.py:135-138`; `TEST` (unit) |
| K5 | Completion gate | **WEAK** — real generation-scoped evidence model, but the gate is advisory: at most one extra turn per mutation generation, then the answer stands with a host notice; no persistent refusal | `turn.rs:1131-1242`, `lib.rs:1754-1789`, `verification_notice.rs:6-27`; `TEST` |
| K6 | False-claim detection | **PARTIAL (eval-only)** — keyword claim detector and `NoUnverifiedSuccessClaim` scoring, CI gate 0.005; nothing parses model prose at runtime | `behavior/trace.rs:336-394`, `scorer.rs:208-219`, `gate.rs:19,72-78`; `TEST` |
| K7 | Structured criteria | **PARTIAL (separate subsystem)** — runtime-task contracts and acceptance packs check per-requirement receipts; the interactive agent keeps prose only and asks the model to enumerate requirements | `runtime/completion.rs:12-17,147-261`, `completion.rs:30-43,604-624`; `CODE` |
| K8 | Crash honesty | **IMPLEMENTED** — interrupted effects invalidate the verified generation before any request | `lib.rs:1478-1591`, `:1747-1750`; `TEST` `agent_crash_after_started_effect…`, `operation_properties.rs:363` |
| K9 | Dependency-aware verification | **PARTIAL** — path-level coverage is conservative and tested; dependency tools are model-invoked and never enforced at completion | `verification.rs:397-437`, `tests/inline_verification.rs:63-86`; `TEST` |

## 4. Documentation-vs-code divergences found during the audit

These are documentation defects in the audited revision, independent of Claude Code.

| Doc claim | Code | Consequence |
|---|---|---|
| `docs/runtime-orchestration.md:16` "oneshot subagents are read-only default" | `subagent.rs:375-376` — `Auto`/`AlwaysApprove` get `FileEdits` | The doc is right for `Ask`/`None`, wrong for the permissive modes |
| `docs/runtime-orchestration.md:18` "worktree isolation required for parallel writers" | Only >1 shared-cwd writer **in one batch** is rejected (`subagent.rs:744-753`); other writers are serialised by the shared-write lease (`team.rs:328-367`) | The guarantee is weaker than stated |
| `docs/agent-teams.md:128` results survive recovery | True for `AgentResultRecorded`; false for workflow completion notices (`executor.rs:1422-1429`) | Overstated for workflow runs |
| `docs/agent-teams.md:157` "parent model, permissions, tools reach workers" | True, but workers start at `ThinkingLevel::Off` (`lib.rs:795`) and load **no** project instruction files (`lib.rs:830`; only `main.rs:10154` enables them for the lead) | Worker behaviour can differ from the lead in ways the doc omits |
| Workflow docs describe `resume` | Execution state is in memory; `workflow_status` is empty after restart and resume needs id + spec from the caller (`tools.rs:296-305`, `executor.rs:554-560`) | Operators cannot resume from a cold start without their own record |

## 5. What this matrix does not prove

- `TEST` means a test asserts the property **in source**. Only the rows marked `RUN`
  were executed here.
- No row measures coding-task success, pass@k, false-claim rate, tokens per task,
  provider cache hits or subscription savings. Those are `NOT ESTABLISHED` and are
  tracked in `08-CODING-SUCCESS-AND-SUBSCRIPTION-METRICS.md`.
- "IMPLEMENTED" is a statement about this revision, not a forward guarantee. The
  rows with `WEAK`/`PARTIAL` name the missing condition so a later change can be
  judged against a specific sentence.

## 6. Repository state used for this audit

```
branch  J12003LPZ/cc-tui-parity
HEAD    6912f940
pre-existing dirty files (left untouched):
  AGENTS.md
  crates/davinci-coding-agent/src/browser_integration_tests.rs
  CLAUDE.md (untracked)
```
