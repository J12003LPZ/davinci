# 07 — Gap matrix and priorities

Status: audit result. One gap (G-01) is closed by this audit; the rest are open.
Evidence prefixes: `MM` = `01-CLAUDE-DAVINCI-CAPABILITY-MATRIX.md` row, `F-xx-yy` = research
finding.

## 1. How priorities were assigned

| Priority | Test used |
|---|---|
| **P0** | Reachable now, causes a false statement about the world (silent duplicate effect, work reported done that was not, auth that is not auth) **and** has a reproducible fixture |
| **P1** | Reachable now, degrades trust, durability or recoverability; a user or agent can act on wrong information, or state is lost/leaked |
| **P2** | Real weakness with bounded impact, or a precondition for measuring anything |
| **P3** | Hygiene, documentation, or a future feature gate |

No DaVinci path in the audited revision met the P0 test. The audit confirmed that the
journal route already reports unknown effects and never auto-replays a started call; the
duplicate-effect and false-success hazards were therefore not reachable **on that
route**. The reachable defect was the non-journal route's *misreporting* (plain failure
plus a retryable ledger entry), which is why G-01 was the first fix, not a P0 cleanup.

## 2. Closed in this audit

| ID | Gap | Fix | Evidence |
|---|---|---|---|
| G-01 | A `tools/call` whose reply was lost was reported as a definite failure, and the compatibility ledger stored a confirmed failure (retryable). Violates the Phase D guarantee "unknown is not failure" (F-20-03, F-20-07). | `Error::{Ambiguous,NotDelivered}`, `ToolError::MayHaveRun`, `into_tool_result`, `record_unknown_outcome`; writer distinguishes incomplete from unflushed. | `10-IMPLEMENTATION-RECORD.md`, `11-VERIFICATION-RECORD.md` |

## 3. Open gaps

| ID | P | Gap | Evidence | Impact if unaddressed | Shape of fix |
|---|---|---|---|---|---|
| G-02 | P1 | A subagent's claimed success is not verified: exit 0 + non-empty text counts as success | `main.rs:2841-2854`; `executor.rs:1800-1834`; `team.rs:709-712`; MM O7 | the lead (and the user) can be told work is done when the diff/test says otherwise; the worktree is preserved but never checked | require a per-worker verification receipt (command + exit + artifact digest) before `finish_agent` reports success; keep "unverified" as a distinct state |
| G-03 | P1 | Worktree teardown errors are discarded; no reaper; crash orphans are invisible | `worktree.rs:474-491`, `:228-230`, `:519-523`; MM O8 | disk/branch leaks grow monotonically; a retry with the same run id fails hard | check teardown results, record them, add a startup scan that lists unknown `wt-*` directories and branches and refuses to reuse a path |
| G-04 | P1 | Default compaction mode `Off` has no validation that user constraints survive | `context_vm/types.rs:40`; `compaction.rs:13-15`; `context-vm-measurements.md:59`; MM C9 | a long session can silently drop a user restriction, and the model acts on the summary | either make `Hybrid`/`Active` the tested default, or add a post-summary constraint check to `Off` |
| G-05 | P1 | Workflow execution state is in memory; workflow completion notices are not durable | `tools.rs:296-305`; `executor.rs:554-560`, `:1422-1429`, `:511-517`; MM O4/O5 | after a restart the operator cannot list or resume without their own record; a completion can be lost with no record | persist `spec`/`launch`/status; use `record_worker_result` for workflow notices |
| G-06 | P1 | The offline evaluation corpus cannot produce a verified success | `scenario.rs:108-120`; `core-200.json` (200/200 empty commands, 121/121 `core-repo`); `fixtures/repos/core-repo` (README only); MM K2 | every "verified success" number is a whitespace check; no campaign can support a claim | write per-scenario verification commands and real repo fixtures, or mark the corpus as structural-only and stop treating it as an oracle |
| G-07 | P1 | The interactive completion gate is advisory (one reminder per mutation generation) | `turn.rs:1169-1233`; `lib.rs:1760-1785`; MM K5 | the model can end a turn with unverified changes and the user sees only a notice | make the gate a refusal with an explicit override path, or record `unverified` as the turn's terminal state in the session and report it |
| G-08 | P2 | Default tool surface is `Full`; laziness is opt-in | `lib.rs:437-441`; MM C4 | every capability enlarges the prefix and the blast radius of a tool change | make `lean` the default after an A/B on prefix bytes and task success, not before |
| G-09 | P2 | Empty IPC token disables auth; comparison is not constant-time; no peer credentials; Unix-only | `davinci-server/src/lib.rs:1036-1046`; `unix.rs:16`; MM S9 | a misconfigured token silently exposes a local control channel to any local process that can open the socket (mode is the only remaining barrier) | refuse to serve with an empty token, compare in constant time, add peer-credential checks; state that Windows is unsupported |
| G-10 | P2 | Signal path skips destructors; foreground spawn jobs are not kill-on-close | `shutdown.rs:37-45`; `davinci-sys/src/process.rs:212-239`; MM S10 | orphan processes and socket files after SIGTERM/SIGHUP | explicit cleanup in the signal path; `KILL_ON_JOB_CLOSE` for the foreground job |
| G-11 | P2 | Memory admission is estimate-only, unlimited when unconfigured, no timeout, no queue cap | `worker_admission.rs:37-40`, `:97`, `:125-130`; MM O2 | a fan-out can exhaust memory on a host with no configured budget | refuse fan-out when the budget is unconfigured above a small default, or bound the wait |
| G-12 | P2 | No `_meta` invocation id on `tools/call` | MM M7 | a cooperating server cannot dedupe; blocks any future retry/reconnect work | send a stable invocation id and a progress token |
| G-13 | P2 | Compatibility route does not re-bind the policy revision at dispatch; no end-to-end deny test through batch/codemode children | `turn.rs:2569-2652` vs `adapters/tools.rs:159-178`; MM S3 | a policy change between prepare and dispatch can be missed on the legacy route | bind the revision on both routes; add two end-to-end denial tests |
| G-14 | P2 | Cancellation during MCP teardown is untested; the research did not establish it (F-42-07) | MM M10 | an unknown-outcome report is still correct, but the cancellation contract is unverified | fixture: server commits after `notifications/cancelled`; assert the report stays "unknown" |
| G-15 | P3 | `execution_authority(id, journal_configured, journal_configured)` collapses two parameters | `turn.rs:2327` | dead distinction; future bug surface | pass the real capability flag or remove the parameter |
| G-16 | P3 | Compatibility ledger dedupe window is 256 terminal records | `tool_ledger.rs:17`, `:623-642` | an old call id can be re-admitted on the legacy path | document the bound; make it configurable if a long session needs it |
| G-17 | P3 | All `SKILL.md` bodies are read at startup | `skills.rs:131-158`; MM C5 | startup I/O and memory grow with the plugin set | lazily read bodies on invocation |
| G-18 | P3 | Workflow `max_cost_usd` is rejected, not enforced | `executor.rs:446-450`, `:487-490` | no spend cap in workflow mode | enforce or document loudly |

## 4. Why G-01 first

Three candidate "first fixes" were considered against the Phase A→L rules:

| Candidate | Why not first |
|---|---|
| G-02 (child success verified) | Requires designing a receipt and deciding what "verified" means for a worktree handoff; cannot be a small reversible change and cannot be tested without a real worker run |
| G-07 (completion gate refusal) | Changes user-visible turn semantics; needs a product decision on the override path |
| G-01 (unknown outcome) | A named class-E defect with a fixture, a fail-safe default, no new product semantics, and a test that can be written before the fix |

G-01 also had a second property that made it the right first target: the fix is
*conservative*. It never turns a known success into a failure; it only stops claiming
"did not run" when the client cannot prove it.

## 5. Explicitly not planned

- A retry or reconnect layer for MCP. It cannot be written safely before G-12 and a
  proven non-delivery signal; `02` §8 lists the preconditions.
- Copying Claude Code's constants (depth 3, 20 concurrent agents, 10 concurrent calls).
  They are properties of that product (MM O1, O2).
- Any claim of parity, superiority, token saving or subscription saving. See `08`.
- Making the eval corpus "look" valid by adding `git diff --check`-style commands to
  more scenarios. That would convert a visible structural gap into an invisible one.
