# Promotion decision (Task 13, offline only)

Status: decision written 2026-10-22 from offline evidence. Nothing is deployed
and no default is changed by this file.

Plan: `docs/superpowers/plans/2026-10-22-codex-efficiency-implementation-plan.md`, Task 13.
Related: `docs/perf/codex-efficiency/2026-10-22-baseline.md`,
`docs/perf/codex-efficiency/requirement-review-decision.md`.

## Verdict

- **Efficiency policy: NO-GO.** No efficiency change to the completion review is
  proposed or promoted. The provider effect of any such change is unknown. There
  is no live A/B, Task 0 has not run, and the usage fields of the Codex backend
  route are unknown.
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

## GO criteria

`GO` requires all of the criteria in the plan's Task 13 Interfaces. Status for
the efficiency policy; none of them is met.

| Criterion | Status | Evidence |
| --- | --- | --- |
| Independently verified task success | not met | No run exists for `b191438b`. The October 17 grading was `diagnostic-only` (`C:\bench\pubcmp-20261016\rep0\campaign.json`, `grading_assurance`), and the public fixtures contain their graders and solutions (`fixture_secrecy`). |
| Complete attempt accounting | not met for this change | The October 17 audit has usage on 195 of 195 requests and no unknown-usage attempts (`turn-cache-summary.json` `requests_with_usage`; `C:\bench\pubcmp\report.txt` `unknown_usage` 0). That is a historical campaign on another build, not this change. |
| Unchanged permission, isolation and recovery | met offline for case (b), not live | After `b7c1fcc1`: `cargo test -p davinci-agent --lib` 1723 passed, `--test completion_requirements` 5 passed, `--test inline_verification` 10 passed 1 ignored, fmt and clippy `-D warnings` clean (all `--offline --locked`, today). The verification gate's own `verified` is unchanged. No live run. |
| Real endpoint and model consistency | not met | The October 17 campaign used the public `api.openai.com` route over `public-websocket`. No live DaVinci request has used the Codex backend route (`docs/cache/chatgpt-backend-route.md:3`). The default model name drifts between `gpt-5.6-luna` and `gpt-6-luna` (baseline file, "Drift"). |
| Credible improvement under predeclared thresholds | not met for efficiency; not applicable for case (b) | The predeclared rule is in Task 4 Step 4: candidate passes at least as many as control on the multi-requirement set, and requests per verified success at most 0.90x. No A/B ran. Case (b) claims no saving. |
| Independent reviewer signoff | not met | One cold automated review of `b191438b` found two holes (a lint pass followed by a partial test run; a stale test pass surviving a failure); both were fixed with regression tests before commit. That is not a human signoff; the PR review is. |

## NO-GO triggers from the plan, checked

| Trigger | Status |
| --- | --- |
| Missing evidence | present: no live A/B, Task 0 not run |
| Incomplete usage that affects comparisons | unknown for the backend route; none in the October 17 public-route logs |
| Crash or stream failure | none seen in the offline runs; no live run exists |
| Unidentified tool mutation | not assessed; no live run exists |
| Over-budget run | none; no live run exists |
| Unauthorized route | none used; the Codex backend route was not called |

## Rollback

Rollback target for the Task 5 change: revert `b7c1fcc1` and `b191438b`
together (reverting only `b191438b` leaves `b7c1fcc1` without its field).

There is no settings switch for the new rule. `requirementReview=false`
(`DAVINCI_REQUIREMENT_REVIEW=0`) disables the review entirely. That is the
existing baseline arm of the October 12 diagnosis, not a rollback of the new
rule, and it removes the review for every run.

## What ran and what did not

- No CI ran. The branch `perf/codex-efficiency-measurement` is not on the remote
  (`git ls-remote --heads origin` returned nothing) and no workflow run exists
  for it. Do not read any statement here as a CI result.
- No provider request was made.
- Full format, Clippy and locked test runs were not done for this document.
- No frozen production binary, process cleanup check or redaction check was done.

## Plan steps not done in this pass

- **Step 1, negative fixtures.** Not added. Open: no live approval, expired
  OAuth, API fallback temptation, unknown cached tokens, negative cache-write
  arithmetic, provider stream cut, aborted root budget reservation, all-skipped
  tests, worker from another session.
- **Step 2, confirm RED.** Not run. The existing subscription policy, root budget
  and bench tests were not run against those fixtures.
- **Step 3, decision report from Task 2.** Not generated. It needs trial
  outcomes from a live run, and none exists.
- **Step 4, verification.** Not done (see above).
- **Step 5, owner decision.** This file is the proposal. The owner decides.

## Next

1. Done 2026-10-22 with owner approval: Task 0 (`ROUTE_REPORTS_USAGE`) and one
   live run per `/config` mode. Results and decision (keep `contextVm` off):
   `live-2026-10-22.md`.
2. Check whether Context VM folds enlarge the context (`tokensAfterFold` above
   `tokensBeforeFold` in both fold runs) before any Context VM default change.
