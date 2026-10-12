# Matched trajectory analysis

Artifacts are relative to `luna-high-recovery-02/vm-off-cm-off+codex` under the
local benchmark root from the approved plan. `L` is a JSONL event line; OTEL
indexes are zero-based JSON pointers.

- D4: `davinci/t4-rename-r0.stdout.jsonl`
- C4: `codex/t4-rename-r0.stdout.jsonl`; O4: `codex/t4-rename-r0.otel.json`
- D1: `davinci/t1-intervals-r0.stdout.jsonl`
- C1: `codex/t1-intervals-r0.stdout.jsonl`; O1: `codex/t1-intervals-r0.otel.json`
- R: `results.jsonl`

Usage is **total input / cached input / output**. Each D4/D1 row has one
completed HTTP attempt, attempt ID 1, and `usage_complete=true`. Request
identity resides in its cited provider-observation event. No provider retries,
pruning or compaction occurred in D4.

## t4: every DaVinci request

| Request / observation | Usage | Tool/result locations | Classification and justification |
|---|---:|---|---|
| 1, D4:L24 | 10823 / 8704 / 70 | L27–37, batched search/discovery/listing succeeds | `necessary_discovery`, establishes callers/layout |
| 2, L74 | 11079 / 9728 / 147 | L77–99, six reads/discoveries | `necessary_read`, `necessary_discovery`; independent reads already batched |
| 3, L133 | 11475 / 9728 / 186 | L136–146, pricing/initializer read and repeated symbol search | `duplicate_read`, `duplicate_search`, `avoidable_sequential_read`, one `necessary_read`; pricing result L142 is already supplied verbatim at L9, search repeats L34 without edits; initializer inspection remains necessary |
| 4, L702 | 11888 / 8704 / 552 | L705–707, one patch updates five allowed files | `necessary_edit`, rename/behavior/regression coverage |
| 5, L731 | 12475 / 0 / 103 | L734–744, pytest collection fails, old-symbol check and diff succeed | `necessary_verification`; failure creates a real dependency |
| 6, L779 | 13613 / 9728 / 48 | L782–784, corrected Python module test entrypoint, four pass | `dependent_followup`, `necessary_verification`; not a provider retry |
| 7, L860 | 13687 / 10752 / 67 | L862–863, final answer then requirement reminder | `dependent_followup`, `harness_or_session_overhead`; requirement gate must not be bypassed |
| 8, L885 | 14221 / 11776 / 221 | L888–902, status/search/pricing/test rereads | `dependent_followup`, `necessary_read`, `duplicate_search`; rereads are valid after request-4 edits, search repeats L740 without edits |
| 9, L1119 | 14743 / 9728 / 325 | L1122–1124, strengthens test coverage | `necessary_edit`, invalidates previous test evidence |
| 10, L1148 | 15084 / 8704 / 125 | L1151–1161, six tests pass, rename/diff checks | `necessary_verification`, post-edit evidence |
| 11, L1242 | 15256 / 13824 / 141 | L1245–1247, bytecode cleanup/status/diff | `dependent_followup` cleanup; `redundant_validation` unchanged-source diff recheck |
| 12, L1447 | 15519 / 9728 / 261 | L1449–1451, requirement summary then verification reminder | `dependent_followup`, `harness_or_session_overhead`; no later source edit |
| 13, L1482 | 15813 / 9728 / 57 | L1485–1487, narrow pytest rerun, six pass | `redundant_validation`, `harness_or_session_overhead`; prompted by missing applicable receipt, do not simply remove verification |
| 14, L1519 | 15896 / 9728 / 23 | L1521–1524, final result still unverified at generation 2 | `dependent_followup`; final reporting required, discovery support limitation remains |

D4:L1525 records 27 tools, 11 batches, four parallel groups, maximum width six.
Its semantic-duplicate counters being zero is not evidence that the identified
duplicate searches/reads were necessary.

## t4: every Codex request, with missing correlation retained

For **all six** rows below, per-request usage/completeness and associations to
tools, edits, verification and continuation reason are **unknown**. Classifying
them beyond `unknown` would fabricate evidence. The sanitizer deliberately
discards content/general span names; request ancestry survives.

| Sampling span | Request record | WebSocket span | Classification |
|---|---|---|---|
| O4 `/spans/1489` | `/records/2` | `/spans/1415` | `unknown` |
| `/spans/2131` | `/records/3` | `/spans/1962` | `unknown` |
| `/spans/3689` | `/records/4` | `/spans/3662` | `unknown` |
| `/spans/4221` | `/records/5` | `/spans/4142` | `unknown` |
| `/spans/4897` | `/records/6` | `/spans/4824` | `unknown` |
| `/spans/5356` | `/records/7` | `/spans/5341` | `unknown` |

O4 `/records/1`, `/spans/909` is explicit warmup, classified
`harness_or_session_overhead`, usage unknown, not a seventh coding request.
`/records/0` is models lookup, not generation.

Unassigned action ledger: C4:L5 discovery; L7 batched reads; L9 symbol search;
L11 instructions/status; L14 five-file change; L17 diff; L18 failed pytest;
L20 rename check; L23 successful CLI smoke; L25 corrected test entrypoint,
five public tests pass; L26 final summary; L27 aggregate usage. Discovery,
reads, edit and post-edit checks are necessary; corrected invocation is a
dependent follow-up. Smoke output is not equivalent to independent assertions.

R:L7–8: both overall passes and six hidden tests. Codex has six logical
requests plus warmup, usage 120488 / 113152 / 1913, uncached 7336.
DaVinci has 14 requests, usage 191572 / 130560 / 2326, uncached 61012.

Codex's ten reported operations bundle leaf discoveries/reads in shell
commands. They are not ten actions equivalent to DaVinci's 27 tool calls.
Both package five file changes in one edit operation.

## Additional successful pair: t1

R:L1–2 records both passes and seven hidden tests each.

| DaVinci request / observation | Usage | Actions and classification |
|---|---:|---|
| 1, D1:L28 | 10828 / 8704 / 111 | Discovery L31–41, `necessary_discovery` |
| 2, L66 | 10995 / 8704 / 68 | Test/README reads L69–75, `necessary_read`; original source already supplied at L9 |
| 3, L255 | 11126 / 8704 / 322 | Source edit L258–260, `necessary_edit` |
| 4, L447 | 11461 / 8704 / 224 | Test edit L450–452, `necessary_edit` |
| 5, L472 | 11700 / 10752 / 74 | Post-edit reads/tests L475–485, two pass, `necessary_read`, `necessary_verification` |
| 6, L549 | 12037 / 10752 / 51 | Final/reminder L551–552, `dependent_followup`, `harness_or_session_overhead` |
| 7, L605 | 12495 / 8704 / 202 | Status/artifact discovery L608–610, `dependent_followup` cleanup evidence |
| 8, L718 | 12792 / 8704 / 176 | Cleanup/test/status L721–723, two pass; cleanup necessary, unchanged-source test `redundant_validation` |
| 9, L769 | 13087 / 10752 / 77 | Tests L772–774, two pass, `redundant_validation`, `harness_or_session_overhead` |
| 10, L918 | 13190 / 9728 / 210 | Summary L920, automatic test L921–925, reminder L929–930; `dependent_followup`, `harness_or_session_overhead` |
| 11, L971 | 13501 / 10752 / 51 | Final L973, `dependent_followup`; final reason `coverage_incomplete` at L976 `/messages/27` |

Every Codex logical request is classified `unknown` for action correlation and
usage: O1 `/records/2..8`, respectively sampling spans
1482, 1825, 2025, 3249, 3497, 3739, 4371 and WebSocket spans
1408, 1716, 1984, 3222, 3442, 3694, 4356.
`/records/1`, `/spans/913` is separate warmup.

C1 action ledger: L5 discovery, L7 batched source/test/README/instruction reads,
L9 status, L12 source/test edit, L14 four public tests pass, L16 diff, L17 final,
L18 aggregate usage 135296 / 122112 / 1249. DaVinci uncached input is 28252,
Codex 13184.

D1 has 14 actual tool starts: 13 model-issued plus one automatic check. The
benchmark intentionally reports those separately. This is not lost execution
accounting.

## Safe reduction candidates

Completion/verification follow-up overhead occurs in both pairs. The existing
engine rejects unsupported pytest discovery and some decorated commands for
broad coverage, as detailed in [architecture](architecture.md). A separate
environment-assignment discovery bypass was corrected; that safety correction
does not establish fewer turns.

The t4 request-3 duplicate supplied-file read/search and request-8 repeated
search can be removed only while retaining initializer/status/post-edit
evidence. Batching initializer inspection into request 2 is a future
behavior-evaluation candidate. No entire request removal is proven merely by
removing one duplicate component. No prompt/default change was made.
