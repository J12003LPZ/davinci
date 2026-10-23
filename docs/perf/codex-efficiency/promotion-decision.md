# Promotion decision (Task 13, offline only)

Status: decision written 2026-10-22 from offline evidence. Nothing is deployed
and no default is changed by this file.

Plan: `docs/superpowers/plans/2026-10-22-codex-efficiency-implementation-plan.md`, Task 13.
Related: `docs/perf/codex-efficiency/2026-10-22-baseline.md`,
`docs/perf/codex-efficiency/requirement-review-decision.md`.

## Verdict

- **Efficiency policy: NO-GO.** No efficiency change to the completion review is
  proposed or promoted. The provider effect of any such change is unknown. There
  is no live A/B. Task 0 has since run (2026-10-22, `ROUTE_REPORTS_USAGE`): the
  Codex backend route reports usage on every attempt, so usage is observed there
  and is no longer a blocker by itself. See "Evidence status" below.
- **Task 5 case (b): safety tightening, not an efficiency candidate.** Commits
  `b191438b` and `b7c1fcc1` stop a build, lint, format, type or syntax check
  from standing in for the requirement review. They add review requests where
  they apply. They are judged on correctness, not on savings.

## What is being decided

Change under decision: `b191438b` ("a check that ran no tests no longer skips
the requirement review") with its correction `b7c1fcc1` ("inline assertion
scripts count as tests for the review skip"). Files:
`crates/davinci-agent/src/completion.rs`, `lib.rs`, `shell_policy.rs`,
`docs/completion.md`.

Before the change, at `de17c967`, `docs/completion.md` says a workspace-wide
`cargo check` or `cargo clippy` counts as `verified` and suppresses the review
although no test ran. The change closes that gap. `b191438b` alone would also
have stopped inline Python assertion scripts from skipping the review (14 of the
15 skipping runs on October 17 ended with one); `b7c1fcc1` restores that, with
tests on the real classifier. With both, none of the 24 October 17 runs changes
on paper (`requirement-review-decision.md`).

## Evidence status (updated after the 2026-10-22 live runs)

Source: `live-2026-10-22.md`. None of this changes the NO-GO.

| Item | Status |
| --- | --- |
| Task 0, backend route usage | done: `ROUTE_REPORTS_USAGE`. Input, cache read, cache write, output and reasoning were present on every attempt (3 of 3 complete). |
| Usage on the backend route | observed. Stage B logged usage on every coding and compaction attempt. |
| A/B of the efficiency change | none. One rep per arm, no baseline-vs-candidate pair for `b191438b`. |
| Verified success | none. Every arm failed the same 1 of 6 hidden tests (5 of 6 passed), so no run is a fully verified success and pass rates cannot rank the arms. |
| Controlled comparison | no. The two `fold14k` arms used compaction threshold 14,000 (keep 3,000, reserve 3,000) and the other five arms 40,000 (keep 8,000, reserve 8,000). Their cost and latency differences mix the mode with the settings. |
| Context VM counters | `tokensBeforeFold` and `tokensAfterFold` are estimates of the checkpoint ledger before and after each fold, summed. They are not provider prompt sizes and say nothing about whether a prompt shrank. Prompt effects are measured with `hybridCompaction.contextTokensBefore/After` (message estimates of applied compactions) and `scripts/bench/fold_economics.py` (provider usage per summarizer request). |

## GO criteria

`GO` requires all of the criteria in the plan's Task 13 Interfaces. Status for
the efficiency policy; none of them is met.

| Criterion | Status | Evidence |
| --- | --- | --- |
| Independently verified task success | not met | No controlled efficiency A/B exists for `b191438b`; Stage B live mode runs are complete, but every arm failed one hidden test. The October 17 grading was `diagnostic-only` (`C:\bench\pubcmp-20261016\rep0\campaign.json`, `grading_assurance`), and the public fixtures contain their graders and solutions (`fixture_secrecy`). |
| Complete attempt accounting | not met for this change | The October 17 audit has usage on 195 of 195 requests and no unknown-usage attempts (`turn-cache-summary.json` `requests_with_usage`; `C:\bench\pubcmp\report.txt` `unknown_usage` 0). That is a historical campaign on another build, not this change. |
| Unchanged permission, isolation and recovery | met offline for case (b), not live | After `b7c1fcc1`: `cargo test -p davinci-agent --lib` 1723 passed, `--test completion_requirements` 5 passed, `--test inline_verification` 10 passed 1 ignored, fmt and clippy `-D warnings` clean (all `--offline --locked`, at the time). The verification gate's own `verified` is unchanged. Stage B live mode runs completed, but no controlled efficiency A/B ran. |
| Real endpoint and model consistency | not met | The October 17 campaign used the public `api.openai.com` route over `public-websocket`. Task 0 and Stage B used the Codex backend route and reported usage on every attempt (`live-2026-10-22.md`). The default model name drifts between `gpt-5.6-luna` and `gpt-6-luna` (baseline file, "Drift"). |
| Credible improvement under predeclared thresholds | not met for efficiency; not applicable for case (b) | The predeclared rule is in Task 4 Step 4: candidate passes at least as many as control on the multi-requirement set, and requests per verified success at most 0.90x. No A/B ran. Case (b) claims no saving. |
| Independent reviewer signoff | not met | One cold automated review of `b191438b` found two holes (a lint pass followed by a partial test run; a stale test pass surviving a failure); both were fixed with regression tests before commit. That is not a human signoff; the PR review is. |

## NO-GO triggers from the plan, checked

| Trigger | Status |
| --- | --- |
| Missing evidence | present: no live A/B, no fully verified success |
| Incomplete usage that affects comparisons | none seen: usage is present on every attempt on the backend route (Task 0, Stage B) and in the October 17 public-route logs |
| Crash or stream failure | none reported in the completed Stage B mode runs; no efficiency A/B |
| Unidentified tool mutation | not assessed for an efficiency A/B; Stage B ran |
| Over-budget run | none reported in Stage B; no efficiency A/B |
| Unauthorized route | none observed; Task 0 and Stage B used the Codex backend route |

## Rollback

Rollback target for the Task 5 change: revert `b7c1fcc1` and `b191438b`
together (reverting only `b191438b` leaves `b7c1fcc1` without its field).

There is no settings switch for the new rule. `requirementReview=false`
(`DAVINCI_REQUIREMENT_REVIEW=0`) disables the review entirely. That is the
existing baseline arm of the October 12 diagnosis, not a rollback of the new
rule, and it removes the review for every run.

## Historical offline-drafting snapshot (before the live runs)

The bullets in this section describe the initial offline draft, not the current
PR or CI status. Later owner-approved Task 0 and Stage B live runs are recorded
in `live-2026-10-22.md`; they still are not a controlled efficiency A/B.

- This draft itself made no provider requests. Task 0 and Stage B did later.
- Full format, Clippy and locked test runs were not done for this draft.
- No frozen production binary, process cleanup check or redaction check was
  done as part of this draft. Do not infer the current PR/CI status from it.

## Plan steps not done in that offline draft

- **Step 1, negative fixtures.** Not added. Open: no live approval, expired
  OAuth, API fallback temptation, unknown cached tokens, negative cache-write
  arithmetic, provider stream cut, aborted root budget reservation, all-skipped
  tests, worker from another session.
- **Step 2, confirm RED.** Not run. The existing subscription policy, root budget
  and bench tests were not run against those fixtures.
- **Step 3, decision report from Task 2.** Not generated in that offline draft.
  Task 0 and Stage B ran later, but no controlled efficiency A/B exists.
- **Step 4, verification.** Not done (see above).
- **Step 5, owner decision.** This file is the proposal. The owner decides.

## Next

1. Done 2026-10-22 with owner approval: Task 0 (`ROUTE_REPORTS_USAGE`) and one
   live run per `/config` mode. Results and decision (keep `contextVm` off):
   `live-2026-10-22.md`.
2. `tokensAfterFold` above `tokensBeforeFold` is ledger growth; it does not
   establish prompt growth or shrinkage (see "Evidence status").
   In the 14k hybrid log the coding prompt grew from 14,843 to 20,497 tokens
   because no compaction was ever applied, so the summarizer ran 13 times for
   nothing; `hybrid-fold-economics.md` has the fix and `fold_economics.py` the
   measurement. Re-run a controlled A/B (same compaction settings in every arm)
   before any Context VM default change.
