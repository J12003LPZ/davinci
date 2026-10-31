# 08 — Coding-success and subscription-efficiency metrics

Status: measurement plan. **No metric in this document has a value.** Nothing here was
measured in this audit; the only executed tests were unit/regression tests (`11`).
Research basis: F-40-01…07, F-41-01…07. Research reports:
`40-effectiveness-and-quota.md`, `41-token-cache-effectiveness.md`.

## 1. Two independent planes

The research corpus establishes neither plane for Claude Code, and this audit
establishes neither for DaVinci. They must never be merged into one score.

| Plane | Question | Allowed metrics | Must not be inferred from |
|---|---|---|---|
| **A. Task success** | did the work actually get done, verified independently? | verified success rate, false-claim rate, pass@k, time to first verified fix, revert rate | prompt size, cache reads, token counts, model self-report |
| **B. Resource use** | what did it cost to get there? | requests, uncached input tokens, cached input tokens, output tokens, wall time, tool calls, worker minutes | success rate, "looks efficient", cache hit ratio alone |

A change may improve plane B while degrading plane A. It may also improve plane A while
using more resources. A valid claim names both.

## 2. Subscription reality (openai-codex)

DaVinci is used with `openai-codex` subscription auth. That has three consequences for
measurement:

1. **There is no per-token price to report.** The unit that matters to the user is quota
   consumption and limit pressure, not USD. Subscription auth must be preserved; no
   API-key or API-only fallback may be introduced to make measurement easier.
2. **Cached input tokens are not savings by themselves.** A cache read is cheaper only
   if the provider's quota accounting treats it that way, and that mapping is not
   established (F-41-06/07). Report cached and uncached input tokens separately and
   never multiply by an assumed discount.
3. **No live subscription-consuming benchmark may run without explicit authorisation.**
   Offline mock and fixture measurements come first, and they measure plumbing only.

## 3. Metric definitions

### 3.1 Plane A

| Metric | Definition | Denominator | Anti-gaming rule |
|---|---|---|---|
| Verified success | scenario where every required verification command passed **and** the workspace diff matches the requirement, judged outside the model | all attempted scenarios | a scenario with no verification command is **excluded**, not counted as success |
| False-claim rate | runs where the final assistant text claims a verification that did not occur at or after the last mutation, divided by runs with any claim | runs with at least one claim | keyword detector plus manual audit of a sample; paraphrase is acknowledged as a detector limitation |
| Pass@k | fraction of scenarios where at least one of k attempts is verified | scenarios × k | attempts must be independent workspaces |
| Revert rate | verified successes that a later reviewer reverts | verified successes | requires a review step, not model judgement |
| Unverified completion rate | runs that end with mutations whose generation is not covered by a receipt | runs with mutations | read from the session, not from prose |

### 3.2 Plane B

| Metric | Definition | Notes |
|---|---|---|
| Requests per verified success | provider requests, including failed attempts | failed-attempt spend must be included (`docs/readiness/evaluation.md:110-122`) |
| Uncached input tokens per verified success | sum of uncached input over all attempts | from `usage`, persisted per turn (`turn.rs:5373-5439`) |
| Cached input tokens per verified success | same, cached bucket | a separate column, never summed into a discount |
| Output tokens per verified success | same | includes reasoning tokens where reported |
| Cache waste estimate | provider-reported cache misses weighted by prefix bytes | `cache_stats.rs` `compute_cache_waste`; an estimate, labelled as one |
| Tool calls / worker minutes | counters from `stats.rs` | descriptive, not a success measure |
| Cost per verified success | total resource use of all runs / verified successes | implemented for USD in `optimization.rs:34-90`; for subscription use, report the same ratio in requests and tokens |

## 4. How to run a campaign honestly

| Step | Requirement |
|---|---|
| 1. Corpus | every scenario has a **failing-before** reproducer and at least one real verification command; repo fixtures contain the files the requirements name |
| 2. Isolation | one workspace per attempt; baseline failure proven before the agent runs |
| 3. Oracle | verification executed by the harness, not by the model; assistant text never influences the verdict |
| 4. Budget | fixed attempts per scenario, recorded; failed attempts included in plane B |
| 5. Auth | the existing subscription path; no API-key fallback; no model switching |
| 6. Size | ≥150 private holdout tasks and 3 repetitions for any superiority claim (`evaluation.md:148-152`) |
| 7. Artifacts | campaign directory with binary hash, commit, corpus hash, per-run JSONL, summary |
| 8. Reporting | plane A and plane B side by side; every metric has its denominator and its unknown count |

Nothing in the current checkout satisfies steps 1, 6, 7 or 8 (`06` §5-6), so no campaign
can be reported today.

## 5. Expected vs measured

| Quantity | Expected (unverified) | Measured |
|---|---|---|
| daVinci verified success rate on a valid corpus | unknown | **not measured** |
| Change in verified success from any optimization in this audit | unknown | **not measured** |
| Uncached input tokens per task | unknown | **not measured** |
| Cached input share | unknown | **not measured** |
| Requests per verified success | unknown | **not measured** |
| Subscription quota consumed per task | unknown | **not measured** |
| False-claim rate | unknown | **not measured** |
| MCP duplicate-effect rate | not applicable (no replay path) | **not measured**; the absence of a replay path is structural, not a measurement |

"Expected" here means nothing. It is listed as unknown on purpose: a number invented
for this table would be indistinguishable from a measurement later.

## 6. What the G-01 fix does and does not buy

| Claim | Status |
|---|---|
| A lost MCP reply is now reported as an unknown outcome, not a failure | **measured structurally** — tests in `11` |
| The compatibility ledger no longer records such a call as a confirmed failure | **measured structurally** — test in `11` |
| Users will retry fewer times / make fewer duplicate mutations | **not measured**, and not measurable with an offline fixture alone |
| The fix reduces quota use | **not measured** and not implied |
| The fix improves task success | **not measured** and not implied |

## 7. Rules for any future number in this directory

1. State the plane (A or B) before the number.
2. State the denominator and the number of runs with unknown cost/usage.
3. Never convert a token reduction into a success claim, or a cache hit into a saving.
4. Never cite Claude Code's unpublished behaviour as a baseline; the corpus does not
   establish one (F-40-05…07, F-41-06/07).
5. A structural test proves the code path; it does not prove a user-visible improvement.
