# Requirement review decision (Task 4, Step 0 only)

Status: Step 0 done offline on 2026-10-22. Steps 1 to 4 not run. No policy change.

Plan: `docs/superpowers/plans/2026-10-22-codex-efficiency-implementation-plan.md`, Task 4.
Baseline: `docs/perf/codex-efficiency/2026-10-22-baseline.md`.

## Decision

The requirement-review requests are 20.0% of all requests in the October 17
campaign. That is at or above the 10% threshold in Task 4 Step 0, so Tasks 4 and
5 stay open. No policy is changed. `requirementReview` stays on by default.

## Step 0 command

Run on 2026-10-22 from the repository root. The output folder is outside the
repository and outside `C:\bench`.

```
python scripts/bench/turn_cache_audit.py C:\bench\pubcmp-20261016\rep0 C:\bench\pubcmp-20261016\rep1 C:\bench\pubcmp-20261016\rep2 --out <temp dir>
```

Inputs are historical logs from 2026-10-17 (DaVinci source `1628438c`, which
includes `8a87a98e`). The numbers below are today's output of the script at
`de17c967`, read from `<temp dir>\turn-cache-summary.json`. The script computes
them. None were estimated by hand.

## Result

| Measure | Value | JSON key |
| --- | --- | --- |
| Runs | 24 | `runs` |
| Requests | 195 | `requests_total` |
| Requests after the first gate | 58 (29.7%) | `requests_after_first_gate` |
| Runs where both gates fired | 9 | `runs_with_both_gates` |
| Uncached input, all requests | 323,968 | `uncached_input` |
| Gate order | `completion.requirements` first in 9 of 9, `verification_required` first in 0 | `gate_order` |

Per gate:

| Gate | Runs fired | Requests | Share of 195 | Uncached input | Share of 323,968 | Mutations afterwards | Gated runs that passed |
| --- | --- | --- | --- | --- | --- | --- | --- |
| `completion.requirements` | 9 | 39 | 20.0% | 44,270 | 13.7% | 3 | 8 of 9 |
| `verification_required` | 9 | 19 | 9.7% | 22,477 | 6.9% | 0 | 8 of 9 |

Source: `per_gate` in `turn-cache-summary.json`. Shares are computed from the
same file. 39 + 19 = 58 and 44,270 + 22,477 = 66,747, which equals
`uncached_input_from_first_completion_reminder`.

Other fields from the same run: transport is websocket for all 195 requests;
later-request cache hit 92.1%; every request has usage.

For comparison, the October 12 diagnosis attributed 47% of requests and 61% of
uncached input to the reminder before `8a87a98e`
(`docs/diagnostics/2026-10-efficiency-diagnosis.md:12`). The residual after that
fix is 20.0% and 13.7%. That is historical against today's re-read, on different
builds. It is not a controlled comparison.

## What the review costs, and what it skipped

`completionRequirementReviewsSkipped` is in the `harness_stats` record of every
`davinci/*.stdout.jsonl` log. Summed by script over the 24 runs:

| Quantity | Value |
| --- | --- |
| Runs with a skipped review | 15 (1 each) |
| Runs with a reminder | 9 (1 each) |
| Runs with both | 0 |
| Total `completionRequirementReviewsSkipped` | 15 |
| Total `completionRequirementReminders` | 9 |

So in 15 of 24 runs `8a87a98e` skipped the review, and in the other 9 the review
fired. The audit's own `per_run_mean` agrees (0.625 skips and 0.375 reminders
per run, times 24, gives 15 and 9). The sums above were computed by a separate
script over the raw logs.

## What Step 0 cannot show

- **Whether the review changed the outcome.** 8 of the 9 gated runs passed, but
  there is no run of the same task without the review. The logs have no
  counterfactual. Three of the nine edited after the review, so the review was
  followed by a mutation in a third of its runs. Whether those edits fixed
  something is not determinable from the logs.
- **Candidate (iii)**, skip the review when `verification_required` fires in the
  same finalization. In all 9 runs the review came first and
  `verification_required` followed it (`gate_order`). The two gates are
  sequential in these logs, not simultaneous. That does not show duplication. It
  shows only that on these 8 tasks the second gate came after the first. The
  data is limited to the 8 public tasks, which were used in tuning.
- **Generality.** The 8 public tasks are short and single-purpose. They cannot
  show what the review is for on multi-requirement work.

## Task 5 case (b) ships independently

Task 5 RED case (b), `workspace_check_alone_does_not_suppress_review`, closes the
gap described in the pre-change `docs/completion.md`: a workspace-wide
`cargo check` or `cargo clippy` counted as `verified` and suppressed the review
although no test ran. Fixing it tightens the review. It can add requests on a run
that was verified only by a build or lint check. It is a safety change and does
not depend on this decision.

How many of the 24 runs it would have affected, from these logs:

- All 8 public tasks are Python. Across the 242 `tool_execution_start` events in
  the 24 DaVinci logs, none runs `cargo`, `tsc`, `tsgo`, `eslint`, `biome`,
  `go vet`, `make check|lint|fmt|clippy|build`, or `npm|yarn|pnpm run
  check|typecheck|lint|build`. This was checked with a script (today).
- So none of the 24 runs verified with a command in the new non-test list.
- That is not the end of it. In all 15 runs that skipped the review, the last
  recorded shell command is an inline Python check (`python - <<'PY'` or
  `python -c`), not `pytest` or `unittest`. In `t6-bookings-r0` the check is
  inside a `batch` operation. Checked with a script over the logs (today).
- `b191438b` judged "ran tests" from the command against the shell test-runner
  patterns only, so these inline checks (classified `TargetedScript` by
  `crates/davinci-agent/src/verification.rs`) would no longer have skipped the
  review: up to 15 of 24 runs reviewed again, at the observed 39 requests per 9
  fired runs.
- `b7c1fcc1` fixes it. "Ran tests" now comes from the verification assessment:
  `TargetedScript` counts, a `Suite` counts when its command runs tests,
  `SyntaxOnly` and `Unknown` never count. The tests
  `inline_assertion_script_skips_review` and `syntax_only_check_still_reviews`
  (`crates/davinci-agent/src/completion.rs`) run the real classifier on the
  heredoc and `python -c` forms taken from these logs; the first failed before
  the fix. With both commits, none of the 24 runs changes outcome on paper: no
  run verified with a build or lint check, and the 15 skipping runs still skip.
  This is a reading of the logs against unit tests, not a re-run.

## Steps not run

| Step | Why not run |
| --- | --- |
| 1, freeze multi-requirement task contracts | Needs tasks authored independently. The H1 holdout framework is on `origin/bench/holdout-h1-framework` and needs a human task author. |
| 2, offline RED and controls | Not run in this pass. Overlaps Task 5 Step 0 and Step 1. |
| 3, candidate policies (i), (ii), (iii) | Each is one arm. None is built. |
| 4, decision gate | Needs an approved live A/B with the predeclared rule (candidate passes at least as many as control on the multi-requirement set, requests per verified success at most 0.90x). Not approved. |

## Next

1. Get a task author for Step 1.
2. Ask the owner whether a live A/B is approved (Step 4).

## Follow-up 2026-10-23: why passing pytest earned no credit

Cause: a pytest run was recognized as a test runner, but its receipt carried no
assertion counts, so it was no evidence. `observe_shell_verification` dropped it
and a passing `pytest -q` earned neither verification nor test credit. Pytest now
has its own runner id and its counts come from pytest's final summary line.

In 7 of the 9 gated runs a passing pytest run preceded the review. The recorded
post-gate tails that this fix could reach total at most 46 requests and 56,296
uncached input tokens, out of 58 requests and 66,747 uncached input tokens in the
tails of all 9 gated runs. That total is the sum of two parts: pytest receipts in
the 7 runs with a preceding passing pytest account for 43 requests and 54,070
uncached tokens, and the unittest default-discovery change accounts for 3
requests and 2,226 uncached tokens (the verification_required tail of t7-cli
rep 2).

This is a counterfactual from recorded logs, not a measured saving. A live A/B is
required before any saving is claimed.

Trade-off: on the rename task the review led to added boundary tests. With the
fix that review would be skipped when `pytest -q` passed. No hidden-test grade
changed in that campaign, so this cost is not observed, only possible.
