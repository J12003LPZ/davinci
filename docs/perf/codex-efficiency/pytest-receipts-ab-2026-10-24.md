# Live A/B of pytest receipts (PR #240), 2026-10-24

Owner-approved live A/B. Raw evidence stays outside the repository under
`C:\bench\pytestrx-20261024\` (`pytestrx.py` launcher, `ab_report.py`, results
under `runs2\`, report saved as `ab-report-runs2.txt`).

## Question and pre-registered rule

Does PR #240 cut completion-gate requests against the PR #239 head without
losing verified successes?

- Candidate: PR #240, commit `3d5cc288`, binary sha256
  `e9fd9c3c077c99622535b3368e3b9f61129470462981df65fceec96f3c3771ad`.
- Control: PR #239 head, commit `7e53c839`, binary sha256
  `17d244a72c793373dbef92cdc65245d329a6773f3d8b9bbfc7b186837dd9c67f`.
- Rule: promote only if verified successes do not drop and gate requests fall.

## Design

- Two frozen release binaries.
- Identical `settings.json`: contextVm off, codemode off, transport sse, retry
  disabled with provider maxRetries 0, requirementReview true, autoVerify true,
  toolSurface full, effortPolicy fixed, promptProfile stable.
- Model `gpt-6-luna`, thinking high, on the `openai-codex` subscription route.
- Legacy tasks `t1-intervals`, `t4-rename`, `t7-cli`, `t8-calc`.
- 3 reps per arm, 24 runs. Arm order alternated per task and rep.
- Every run used the same neutral workspace path (`runs\_active\<task>-r<rep>`)
  and was archived afterwards.

## Campaign history

- Attempt 1 stopped at run 5 when one provider request hit the 300 s global
  timeout (a route stop). Its 5 runs are kept in `runs\` and excluded.
- Attempt 2 stopped at run 7 on a provider `usage_limit` (Plus plan, 300-minute
  window). That one run was moved to `runs2\_discarded` by the launcher's
  `--resume` rule: only rows with a provider `stop_reason` are discarded. The
  campaign resumed at run 7 in the next window with the same binaries, settings
  and baseline hashes.
- Several runs had single provider requests of 160 to 650 s that returned HTTP
  200.

## Results

| arm | runs | verified | hidden tests passed | requests | uncached input | output | wall s |
|---|---|---|---|---|---|---|---|
| control | 12 | 11 | 110/111 | 147 | 128,019 | 28,499 | 1257.63 |
| candidate | 12 | 11 | 110/111 | 93 | 71,691 | 18,929 | 1559.65 |

Changes, candidate against control: requests -36.7%, uncached input -44.0%,
output -33.6%.

## Gates

Requests and uncached input are those made after the gate fired.

| arm | gate | reminders | requests after | uncached after |
|---|---|---|---|---|
| control | completion.requirements | 10 | 54 | 50,905 |
| control | verification_required | 10 | 22 | 17,941 |
| control | no gate | 0 | 71 | 59,173 |
| candidate | completion.requirements | 3 | 14 | 16,143 |
| candidate | verification_required | 3 | 8 | 3,796 |
| candidate | no gate | 0 | 71 | 51,752 |

Gate-attributed requests fell from 76 to 22 (-71.1%). Requests with no gate are
equal (71 each), which is consistent with the saving coming from the gates.

## Per task

Requests, control against candidate, reps 0/1/2:

| task | control | candidate |
|---|---|---|
| t1-intervals | 15/12/11 | 6/6/6 |
| t4-rename | 14/13/13 | 6/6/6 |
| t7-cli | 18/17/12 | 17/5/14 |
| t8-calc | 5/6/11 | 6/9/6 |

- `t1` and `t4` account for 42 of the 54 saved requests.
- The candidate used more requests than control in three pairs: t7 rep 2, t8
  rep 0 and t8 rep 1.

## Failures

- One failure per arm, the same hidden test
  `test_hidden_calc.py::test_errors` on `t8-calc` (control rep 1, candidate
  rep 2). No completion gate fired in either run.
- Both runs skipped the requirement review (`completionRequirementReviewsSkipped`
  = 1 in `harness_stats`). The control skip came from the credit that already
  existed in `7e53c839`. The candidate skip came from the new pytest credit.
- Across both arms, 2 of the 4 `t8-calc` runs that skipped the review failed,
  and both `t8-calc` runs that ran the review passed.
- That is too few runs to attribute a correctness cost to the change. Settling
  it needs checkpoint analysis: grade control rep 2 before and after its review,
  and replay candidate rep 2's completion state with the review on and off.

## Wall time

The candidate total is 24% higher. Provider latency outliers drive it: candidate
t8 rep 1 took 852 s, of which one request took 650 s, and candidate t7 rep 0
took 330 s. No speed claim either way.

## Verdict

The pre-registered rule passes. Verified successes are 11/12 in both arms, and
gate-attributed requests fell from 76 to 22.

Limits:

- Four legacy tasks with three reps cannot establish correctness noninferiority
  or generalization.
- The campaign spanned two quota windows.
- "Requests after a gate" counts downstream work, not proven waste.
- Token savings are not a measured subscription allowance saving.
- Bare unittest recognition in the diff was not exercised by these tasks.

Broader claims need a fresh holdout and the checkpoint analysis above.
