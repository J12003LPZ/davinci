# Completion-reminder overhead in the Stage 1 DaVinci runs

Source: the retained event logs of the 12 DaVinci runs of campaign `v2-stage1-20261015-run2`
(local, never shared). Read-only. No agent command was rerun and no model was called. Labels are
heuristics from tool names, paths and ordering, not proven causes.

## What the reminder cycle is

`completion.requirements` fires exactly once in every DaVinci run (and `verification_required` in
11 of 12). Before it, DaVinci averages 8.6 requests per run, fewer than Codex's 12.8 in total. After
it, 9.8 more per run: 117 of 220 requests (53%), 61% of uncached input, 55% of provider time.

## Did the post-reminder work change correctness?

Each run's workspace at the moment the reminder fired was rebuilt by applying only the recorded
`edit` and `apply_patch` calls to the pristine fixture. The rebuild of the final state matched the
retained workspace exactly in all 12 runs, so the intermediate states are reliable. The pinned
hidden tests were then run on the pre-reminder state, offline.

* **12 of 12 runs already passed every hidden test when the reminder fired** (15/15, 15/15, 17/17,
  16/16). A pristine fixture fails 7 of 15 on m1, so the check discriminates.
* Post-reminder file changes: tests only in 10 runs (6 to 95 added lines in the public test file);
  tests plus a one-line behaviour-neutral refactor (`read_config = parse_config`) in m1 reps 1
  and 2. No post-reminder change flipped any hidden test.
* In m1 rep 0 the post-reminder work is what created the graded failure: the pre-reminder state
  passed 15 of 15, and the test the agent then wrote failed only under the mismatched grader
  environment (`v2-harness-changes.md`).

These four tasks have short, explicit requirements. The reminder is aimed at requests with many
requirements, which these runs do not exercise. Nothing here shows whether it helps there.

## Post-reminder requests by activity (117 requests, 794k uncached input, 1,026 s provider time)

| Category (heuristic) | Requests | Uncached input | Provider time |
|---|---|---|---|
| Verification: test authoring (edits to the public test file) | 23 (20%) | 158k (20%) | 344 s |
| Verification: first test or diff run after an edit | 17 (15%) | 135k (17%) | 106 s |
| Verification: review of an edited file | 2 (2%) | 18k (2%) | 15 s |
| Verification re-run with no edit call since the previous one (audited below) | 19 (16%) | 114k (14%) | 135 s |
| Housekeeping (`rm __pycache__`, `git status`, `git diff`; audited below) | 16 (14%) | 100k (13%) | 135 s |
| Necessary: final answer | 12 (10%) | 91k (11%) | 53 s |
| Unknown: plan bookkeeping (`todo`) | 12 (10%) | 70k (9%) | 97 s |
| Unknown: other no-tool replies | 9 (8%) | 57k (7%) | 84 s |
| New requirement coverage (the one-line alias refactor, m1 reps 1 and 2) | 3 (3%) | 15k (2%) | 22 s |
| Unknown: other shell, new exploration | 4 (3%) | 35k (4%) | 36 s |
| Genuine bug fixes (a change after a failing check) | 0 | 0 | 0 |

## Audit of the 35 "repeated or unnecessary" requests

The first pass labelled 35 post-reminder requests (19 verification re-runs, 16 housekeeping) as
repeated or unnecessary. That label came only from two rules: a verification-looking command with no
`edit` or `apply_patch` call since the previous one, and a shell command touching `__pycache__`,
`git status` or `git diff`. It did not compare command text, results or shell-made changes, so it
overstated what is redundant. A second pass checked each of the 19 re-runs against the previous
verification command in the same run (exit code, pass or fail count, command text, any write made
through the shell, and the harness mutation generation counter in `mutation_observation` events):

| Re-run verdict | Requests | Uncached input | Provider time |
|---|---|---|---|
| Identical command, workspace unchanged, previous run passed (only reads between) | 1 | 1.4k | 6 s |
| Justified: the previous run failed (exit 2) | 2 | 11.0k | 28 s |
| Justified: the workspace changed through the shell, which the first pass could not see | 1 | 6.4k | 5 s |
| Scope changed: a different command from the previous check | 15 | 95.3k | 97 s |

The 15 scope changes are not repeats. They pair `pytest` with `git diff --check`, `git status` or
`git diff --stat`, narrow a full run to one test file, or rerun `pytest` after a `__pycache__`
cleanup. None of them can be called redundant without knowing whether the added or dropped part was
what caught a problem, and no run recorded a hidden-test change either way. The earlier statement
that 7 of 20 command-level re-runs followed a gate note ("not counted as verification") stands for
that note, but it is a different measure from this audit and is not a count of justified repeats.

The 16 housekeeping requests, by what the command was:

| Command | Requests | Uncached input | Provider time | Next request was an edit |
|---|---|---|---|---|
| `git status` | 9 | 65.5k | 87 s | 4 of 9 |
| `rm` of `__pycache__` or `.pyc` | 6 | 30.9k | 38 s | 2 of 6 |
| `git diff` (first look at the edits) | 1 | 4.1k | 10 s | 1 of 1 |

`git status` is how a run confirms it left no stray file, and unrelated edits are a graded outcome,
so it is a check, not noise. Only the `__pycache__` cleanup has a mechanical argument against it:
the harness ignores `__pycache__` and `.pytest_cache` (`bench.py` `IGNORED`), so removing them cannot
change a graded result.

## What could be removed without weakening the correctness gate

| Candidate | Requests | Uncached input | Provider time | Share of all DaVinci (220 requests, 1.29M uncached, 1,958 s) | Basis |
|---|---|---|---|---|---|
| Identical repeat of a passing check, nothing changed | 1 | 1.4k | 6 s | under 0.5% / 0.1% / 0.3% | provable |
| `__pycache__` cleanup | 6 | 30.9k | 38 s | 2.7% / 2.4% / 1.9% | the harness ignores the artifact; the agent habit is the cost |
| **Upper bound with evidence behind it** | **7** | **32.3k** | **44 s** | **3% / 2.5% / 2.2%** | |
| Verification with changed scope, `git status` | 24 | about 161k | about 184 s | 11% / 12% / 9% | candidates for a live ablation only: nothing here shows they were unnecessary |
| Plan bookkeeping (`todo`) | 12 | 70k | 97 s | 5% / 5% / 5% | value unknown |

This replaces the earlier figure of 35 requests, about 215k uncached tokens and 270 s as "redundant
by construction". That figure was a count of requests matching two patterns, not of requests
shown to be unneeded. The evidence supports removing about 3% of requests, not 16%.

Not proposed for removal: test authoring (344 s, the largest block). It did not change any hidden
outcome here, but its value on tasks with many requirements is untested, and it is the work that
could catch a missed requirement. The completion reminder and requirement verification stay as they
are.

These savings are small next to the cache gap: the whole uncached gap to Codex is 916k tokens, and
the best-supported removable set is about 32k of DaVinci's 1.29M.

## Method limits

* Request categories come from tool names and paths; the first-pass "no edit since the last one"
  looked at edit and patch calls only. The audit above adds the shell-write and ledger checks, but its
  shell-write pattern is a regex, so a write it does not recognise would be missed.
* The audit scripts read the retained raw logs under `C:ench`, which are never shared, so they stay
  local. This document holds the aggregates only.
* The reconstruction applies recorded edits only. All 12 rebuilds matched the retained final
  state, which bounds the risk, but it is not a re-execution.
* Four tasks, three repetitions, one harness: direction only.
