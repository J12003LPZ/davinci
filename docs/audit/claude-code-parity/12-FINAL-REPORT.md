# 12 — Final report: Claude Code 2.1.310 research → DaVinci audit

Status: final. One change implemented and tested (`G-01`); everything else is analysis or
plan. No commit, no push, no live model call, no benchmark.

| Document | Content |
|---|---|
| `00-RESEARCH-IMPORT-AND-EVIDENCE.md` | verified identity of the research corpus, evidence classes, what was recounted |
| `01-CLAUDE-DAVINCI-CAPABILITY-MATRIX.md` | 52 mechanism rows (11 MCP, 12 context, 10 orchestration, 10 session/permission, 9 coding-success), Claude Code behaviour vs DaVinci status |
| `02-MCP-REPLAY-AND-UNKNOWN-OUTCOMES.md` | the six conflated events, classification truth table, retry preconditions |
| `03-CONTEXT-CACHE-AND-COMPACTION.md` | prefix stability, compaction, cache accounting, what is not measured |
| `04-ORCHESTRATION-SUBAGENTS-WORKTREES.md` | spawn model, inheritance, admission, leaks, child-claimed success |
| `05-SESSIONS-PERMISSIONS-AND-IPC.md` | journal ordering, replay policy, permission order, IPC auth |
| `06-CODING-SUCCESS-AND-VERIFICATION.md` | completion gate, false-claim detection, the defective eval corpus |
| `07-GAP-MATRIX.md` | 18 gaps with P0–P3, evidence and the shape of a fix |
| `08-CODING-SUCCESS-AND-SUBSCRIPTION-METRICS.md` | two measurement planes, metric definitions, expected-vs-measured table |
| `09-ROADMAP-AND-ACCEPTANCE-CRITERIA.md` | five waves, each with command-level acceptance criteria |
| `10-IMPLEMENTATION-RECORD.md` | the fix: design, tests, changed expectations, risks, rollback |
| `11-VERIFICATION-RECORD.md` | exact commands, exit codes, counts, what was not run |

---

## Bottom line

DaVinci is **safer than Claude Code 2.1.310 on the specific hazard the research found
most dangerous** — duplicate side effects from replay. It has no call replay, no
reconnect, and a journal that never auto-replays a started effect. That safety is not a
side effect of care; it is a consequence of two independent mechanisms (`NeverAutoReplay`
policy plus an `EffectStatus::NotStarted` precondition).

The cost of that safety is **capability, not correctness**: a call that provably never
ran is also not retried, so the model pays a round-trip. That is a deliberate trade and
should stay until `_meta` invocation ids exist.

The one reachable defect was not a duplicate effect but a **misreport**: a `tools/call`
whose reply was lost was shown to the model as a definite failure and stored in the
compatibility ledger as a confirmed, retryable failure. That is fixed and tested here
(`G-01`), on the non-journal route; the journal route was already correct.

The largest remaining gap is **not** in the agent loop. It is that DaVinci has **no
valid way to demonstrate coding-task success**: the completion gate is advisory, a
subagent's claim is unverified, and the offline corpus cannot produce a verified success
at all (200/200 scenarios have empty verification commands; the shared fixture is a
21-byte `README.md`). Until that changes, no claim of improvement — including about this
audit's own fix — can be measured.

---

## A. What DaVinci does correctly

Each item is a mechanism that measurably prevents a failure mode the research documented
in Claude Code, and each has a test or a code path recorded in `01`.

| # | Property | Why it matters | Evidence |
|---|---|---|---|
| A1 | **No automatic replay of a tool call.** One `Client` per server, single-flight, no retry loop | removes the class-E duplicate-mutation family (F-20-01…10, F-31-01…11) at the source | `01` M1/M2; `RUN` in `11` |
| A2 | **Unknown effect is a first-class journal state.** A failed completion records `EffectStatus::Unknown`; retry requires `NotStarted` | recovery can reconcile without guessing | `05` §3-4; `TEST` `agent_tool_resume_integration.rs` |
| A3 | **Crash honesty in the agent.** Interrupted effects bump the mutation generation, clear the verified generation and mark unknown paths before the next request | a half-finished edit can never be reported as verified | `06` §8; `TEST` `agent_crash_after_started_effect…` |
| A4 | **Deny precedes allow, including `AlwaysApprove`.** One dispatch choke point covers direct, batch, codemode, MCP and host calls | no permissive mode can override an explicit deny rule | `05` §8; `TEST` |
| A5 | **No approver means deny**, not silent allow | a headless run cannot self-approve | `05` §8 |
| A6 | **Session log durability**: `O_APPEND` + `sync_all` per line, torn tail backed up before truncation, first write takes an exclusive lock | the conversation survives a crash without losing the discarded bytes | `05` §6; `TEST` |
| A7 | **Load-time pairing repair**: unresolved calls are answered `unknown`/`not executed` with a machine-readable source; orphan results are refused | a reloaded session cannot silently lose or invent a tool result | `05` §7; `TEST` 8 tests |
| A8 | **Stable prompt prefix by construction**: stable text in `instructions`, per-turn state as `Dynamic`, tools sorted with a schema digest | cache-friendliness is a property of the code shape, not of luck | `03` §2-3; `TEST` |
| A9 | **Compaction commit ordering**: the summary entry is appended before live history is replaced, with rollback on failure; fold chunks checkpoint | a crash during compaction cannot lose the transcript | `03` §5.2; `TEST` |
| A10 | **Cancellation correctness**: flag-before-lock, `Weak` children, child-of-cancelled-parent race closed | no orphaned work after Esc or session switch on the oneshot path | `04` §9; `TEST` |
| A11 | **Duplicate-worker prevention**: durable logical id, task-board revision claim, one shared-writer lease, at-most-once mailbox | two workers cannot claim and edit the same task | `04` §7; `TEST` |
| A12 | **Worktree hygiene on the happy path**: deterministic path, dirty/failed leases preserved, unreadable state treated as dirty | cleanup can never silently discard work | `04` §6; `TEST` |
| A13 | **Unknown MCP outcome is now model-visible and durable** (this audit) | the model stops treating a possibly-executed call as a clean failure | `02`, `10`; `RUN` |

## B. What it does differently but correctly

These are places where the comparison is a design difference, not a deficiency. Each is
recorded as `DIFFERENT` in `01` with the reason, and none is scored as missing.

| # | Claude Code mechanism | DaVinci's alternative | Why it is not a defect |
|---|---|---|---|
| B1 | Numeric subagent depth limit (depth 3) | Depth 1 enforced **structurally**: `agent` is removed from every worker's tool set *and* nested admission is non-waiting | two independent mechanisms; a bug in one does not grant depth 2 |
| B2 | One shared agent client with session-level replay on teardown | One `Client` per MCP server behind a `Mutex`; a failure evicts only that server | blast radius is smaller, not larger; the peer still fails but is never re-sent |
| B3 | Session history is the durable record | Two stores: prose history in JSONL, operations in a SQLite journal with intent/effect latch | the split is what makes "unknown effect" representable at all (`05` §1) |
| B4 | `bypassImmune` permission class | `ReadOnly` hard-denies; any mode change re-binds a policy revision on the journal route | a strictly narrower surface for the same hazard |
| B5 | Approval caching with expiry | One-shot approvals only; no cache | strictly less authority retained |
| B6 | `idempotentHint` can authorise a retry | Annotations are parsed but never used to authorise replay | the hazard (at-most-once → at-least-once) cannot occur |
| B7 | Background reconnects to a dead MCP server | Failed server stays evicted for the session | fail-closed; costs a session restart instead of risking a duplicate |

## C. What is missing

Ordered by consequence. Full evidence and the shape of each fix are in `07`.

| ID | P | Missing capability | Consequence |
|---|---|---|---|
| C1 (G-02) | P1 | A subagent's claimed success is never checked against the repository | the lead can be told "tests pass" when they were never run; the preserved worktree is the only evidence, and nobody looks at it |
| C2 (G-06) | P1 | A corpus that can produce a verified success | every offline "verified success" would mean "`git diff --check` passed"; no campaign can support any claim |
| C3 (G-07) | P1 | A completion gate that refuses, or a recorded `unverified` terminal state | the model can finish once with unverified mutations; the user sees a transient notice |
| C4 (G-05) | P1 | Durable workflow execution state and notices | after restart, `workflow_status` is `[]` and resume needs the caller's own copy of the spec; a workflow completion notice is mailbox-only |
| C5 (G-03) | P1 | Worktree teardown verification and a reaper | directory + branch leaks accumulate; a same-run-id retry fails hard on `PathAlreadyExists` |
| C6 (G-04) | P1 | Constraint preservation in the **default** compaction mode | the strongest validation exists only on the opt-in Context VM path; default is unvalidated summarisation |
| C7 (G-12) | P2 | `_meta` invocation id on `tools/call` | a cooperating server cannot dedupe; blocks any future retry or reconnect |
| C8 (G-11) | P2 | Bounded memory admission | unconfigured means unlimited; waiters have no timeout and there is no queue cap |
| C9 (G-09) | P2 | Real local-IPC authentication | empty token disables auth; `==` comparison; no peer credentials; Unix only |
| C10 (G-10) | P2 | Cleanup on signal | SIGTERM/SIGHUP calls `process::exit`, skipping `Drop`; foreground jobs are not kill-on-close |
| C11 (G-13) | P2 | Policy revision re-binding on the compatibility route | a policy change between prepare and dispatch can be missed on the legacy path |
| C12 (G-08) | P2 | Lazy tool surface by default | `Full` is the default; every capability enlarges the prefix |
| C13 (G-14) | P2 | A test for cancellation-after-commit | the report stays conservative either way, but the contract is unverified |
| C14 (G-17) | P3 | Lazy `SKILL.md` body reads | startup I/O and memory grow with the plugin set |
| C15 (G-18) | P3 | Workflow spend cap | `max_cost_usd` is rejected rather than enforced |

## D. What is incorrect

"incorrect" here means a reachable false statement about the world, or a doc claim the
code contradicts.

| # | Defect | Status |
|---|---|---|
| D1 | A `tools/call` whose reply was lost was reported as a definite failure, and the compatibility ledger stored a confirmed (retryable) failure | **fixed and tested** — `G-01`, `02`, `10`, `11` |
| D2 | `docs/runtime-orchestration.md:16` says oneshot subagents are read-only by default; `subagent.rs:375-376` grants `FileEdits` under `Auto`/`AlwaysApprove` | open — `01` §4 |
| D3 | `docs/runtime-orchestration.md:18` says worktree isolation is required for parallel writers; only >1 shared-cwd writer **in one batch** is rejected, others are serialised | open — doc overstates the guarantee |
| D4 | `docs/agent-teams.md:128` says results survive recovery; true for `AgentResultRecorded`, false for workflow completion notices | open |
| D5 | `docs/agent-teams.md:157` says the parent model, permissions and tools reach workers; workers start at `ThinkingLevel::Off` and load no project instruction files | open — real behavioural difference |
| D6 | Workflow docs describe `resume` as if state were durable; it is in-memory | open |
| D7 | `execution_authority(id, journal_configured, journal_configured)` passes the same flag twice | open (G-15) — dead distinction, future bug surface |
| D8 | The compatibility ledger's dedupe window is 256 records, so a very old call id can be re-admitted | open (G-16) — bounded, not documented |

No path produced a **silent duplicate side effect**. That is the single most important
negative result of this audit, and it rests on the absence of a replay path plus the
journal's `NotStarted` precondition, not on a test of every error shape.

## E. What must not be copied from Claude Code

These are properties of that product, or behaviours the research itself shows to be
defects. Copying them would make DaVinci worse.

| # | Must not copy | Why |
|---|---|---|
| E1 | The two-attempt replay on a shared-client teardown (F-31-03/05/11) | it is the documented source of duplicate mutations and discarded peer results |
| E2 | Deriving "did not run" from an HTTP status, e.g. 404 → `mayHaveRun=false` (F-20-05) | the same inference turns a 404 into a duplicated mutation |
| E3 | Using `idempotentHint`/`readOnlyHint` to authorise a replay (F-20-08/09) | a server hint is not a client guarantee; it converts at-most-once into at-least-once |
| E4 | Reporting a crash after commit and before commit with the same message (F-20-03) | it is exactly the defect `G-01` removes |
| E5 | Its numeric constants — depth 3, 20 concurrent agents, 10 concurrent calls | they encode that product's resource model; DaVinci's bounds are different and documented |
| E6 | Its specific prompt text, tool descriptions or wording | proprietary; and the research does not establish that the wording causes the behaviour |
| E7 | A "parity score" or a token/cache number presented as a success measure | the research establishes neither (`08` §1) |

## F. First tasks

Ordered so that **measurement exists before optimisation**. Each has command-level
acceptance criteria in `09`.

| Order | Task | Why first |
|---|---|---|
| 1 | **Repair the evaluation corpus** (G-06): real verification commands, real repo fixtures, delete the dead fixture directories | without it, nothing downstream can be validated; today "verified" means whitespace |
| 2 | **Make the completion gate refuse or record** (G-07) | the cheapest change that makes "done" mean something to the user |
| 3 | **Verify a worker's claim** (G-02): receipt = command + exit + diff/artifact digest; `Unverified` is a state, not a success | the largest trust gap in orchestration, and it inherits the corpus from task 1 |
| 4 | **Workflow state + notice durability** (G-05) | recovery currently depends on the caller remembering what it launched |
| 5 | **Worktree teardown + reaper** (G-03) | silent, unbounded disk/branch leak with no user-visible cause |
| 6 | **Constraint survival in the default compaction mode** (G-04) | the failure is invisible until the model acts on a dropped instruction |
| 7 | **IPC auth hardening** (G-09) | security-relevant, bounded, independently testable |
| 8 | **`_meta` invocation id** (G-12) | precondition for any future retry/reconnect; do it before, not after |

Tasks 4–8 are independent and can be reordered by whatever the team can measure first.
Task 1 is not optional and cannot be replaced by a benchmark against Claude Code: the
research does not supply a Claude Code baseline (`08` §2).

## G. Implemented and tested

`G-01` — unknown outcome for a lost MCP reply.

**Change** (7 files, +412 −26 including tests):

1. `davinci-mcp/src/lib.rs` — `Error::{Ambiguous, NotDelivered}` plus `may_have_run()` /
   `definitely_not_delivered()`; classification at the single `Client::call_tool`
   boundary; fail-safe default (anything not provably undelivered and not an RPC reply is
   ambiguous).
2. `davinci-mcp/src/stdio.rs` — the writer reports *why* a line did not complete:
   queue-full and incomplete lines are `NotDelivered`; a complete line whose flush failed
   and a writer that stopped after accepting a job are `Ambiguous`.
3. `davinci-agent/src/mcp.rs` — an ambiguous transport error becomes
   `ToolError::MayHaveRun` after the server is evicted.
4. `davinci-agent/src/tools.rs` — `into_tool_result` renders "Outcome unknown … It was not
   retried. Check the server or target state before calling it again." and sets
   `details.outcome_unknown` / `details.tool`.
5. `davinci-agent/src/turn.rs` — the catch-all uses `into_tool_result`; the non-journal
   ledger branch reads `outcome_unknown`.
6. `davinci-agent/src/tool_ledger.rs` — `record_unknown_outcome` writes terminal
   `status=Failed` with `outcome=StartedUnknown`, which recovery maps to
   `Stop`/`ReplayBlocked`, never `Retry`/`Replay`.
7. `davinci-agent/tests/common/codemode.rs` — fixture expectation updated to the new
   contract (effect count still 1).

**Verification** (`11`): 1971 cases executed, 0 failed.

```
cargo test -p davinci-mcp                           52 lib + 15 integration   exit 0
cargo test -p davinci-agent --lib                   1863 passed               exit 0
8 targeted integration suites                        41 passed                 exit 0
cargo fmt --check, cargo clippy --all-targets        clean                     exit 0
```

**Explicitly not done:** no retry, no reconnect, no `_meta` id, no change to journal
replay policy, permissions, IPC, worktrees, evals, model selection or auth.

## H. What remains unverified

| # | Unverified | Why it matters |
|---|---|---|
| H1 | Any change in user-visible reliability from `G-01` (fewer duplicates, fewer retries) | the fix makes the report honest; whether behaviour improves needs a live run, which was not authorised |
| H2 | Real third-party MCP servers behave like the fixtures | the fixtures implement the surface DaVinci uses, not a real server's edge behaviour |
| H3 | Cancellation-after-commit (`notifications/cancelled` then the server commits) | the research did not establish it (F-42-07); no test exists |
| H4 | `Incomplete` → `NotDelivered` for a non-conforming server that acts on partial lines | documented assumption (`02` §7.2) |
| H5 | Any coding-task success rate, false-claim rate, pass@k, token cost, cache hit or subscription saving | `08` — no values exist, none were produced |
| H6 | Journal durability under power loss or injected write faults | only process-crash + reopen is tested (`05` §2) |
| H7 | Crash-orphaned worktrees and the restart behaviour of workflow state | no test, and the code paths are acknowledged gaps (G-03, G-05) |
| H8 | `davinci-coding-agent`, `davinci-session`, `davinci-evals`, `davinci-server` suites | not executed this session; their rows in `01` are `CODE`/`TEST` |
| H9 | The 256-record ledger dedupe bound under a long session | not exercised |
| H10 | Two concurrent sessions launching workers in one repository | no cross-process lock was found and none was tested |

## I. Expected vs measured

**Expected** here means "believed, unverified". It is not a forecast.

| Quantity | Expected (unverified) | Measured |
|---|---|---|
| Duplicate MCP mutations caused by the client | zero, by construction (no replay path) | **no measurement**; the absence of a replay path is structural |
| Model-visible misreport of a lost reply | zero after `G-01` | **structural only** — 8 regression tests, no live run |
| DaVinci verified success rate on a valid corpus | unknown | **not measured** — and not measurable until G-06 |
| Effect of any optimisation in `09` on verified success | unknown | **not measured** |
| Uncached / cached input tokens per task | unknown | **not measured** |
| Requests per verified success | unknown | **not measured** |
| Subscription quota consumed per task | unknown | **not measured** |
| False-claim rate | unknown | **not measured** |
| Wall time of the executed suites | — | 46.99 s (`davinci-agent --lib`), 6.70 s (`davinci-mcp --lib`), single debug run; **not a benchmark** |
| Test cases executed | — | **1971, 0 failed** (`11` §2) |

Rules applied to this table (`08` §7): success and resource use are separate planes; a
token or cache number is never converted into a success claim; a structural test is never
reported as a user-visible improvement.

## J. Next wave

Sequence and acceptance criteria: `09`. In one paragraph:

The next wave must make success measurable before it tries to improve it. Wave 1 repairs
the corpus, turns the completion gate into a refusal or a recorded `unverified` state,
and requires a receipt before a worker's claim is reported as success — in that order,
because each depends on the oracle the previous one validates. Wave 2 closes the silent
loss paths that have nothing to do with model behaviour: worktree teardown and reaping,
and durable workflow state and notices. Wave 3 is the first wave that may touch prompt
shape (default compaction validation, then a measured `Full` vs `lean` comparison, then
lazy skill bodies); it must not be credited with a task-success effect, because task
success is what Wave 1 measures. Wave 4 is security and cleanup with no success story
attached: IPC auth, signal cleanup, and one policy revision on both dispatch routes.
Wave 5 is capacity policy and the precondition for any future retry: memory admission
bounds, a `_meta` invocation id, a cancellation-after-commit fixture, and a decision on
the workflow spend cap.

**Do not start Wave 3 or 5 before Wave 1 is finished.** Without a corpus that can fail,
every later comparison is a comparison of unverified numbers, and this audit's own fix
would be claimed as an improvement it has not demonstrated.
