# H1 independent holdout: preregistration

Status: **prepared, not launched.** No run of H1 has happened. Nothing here authorizes a launch, a merge or a change of
any default. The evaluator is `scripts/bench/holdout_h1.py`; its tests are `scripts/bench/tests/test_holdout_h1.py`
and `test_holdout_h1_suite.py`; its mutation checks are `scripts/bench/mutation_check_holdout_h1.py`.

This file is frozen at the harness commit named in the pins file. A change after that commit is a new preregistration,
not an edit. Thresholds are never changed after results exist.

## Question

On six independently authored Python tasks, does DaVinci with the persistent WebSocket transport match Codex CLI in
verified success, and does it do so with fewer tokens and less wall time? And is the WebSocket arm at least as
successful as the same binary on HTTP, with a fallback behavior that a human can accept for production?

Scope of any conclusion: multi-file Python repositories tested with pytest, six tasks, one model (`gpt-6-luna`,
effort `high`, service tier `default`), three repetitions each. Repetitions of one task are not independent samples.
A result supports a statement about this suite and nothing wider. It is not evidence of superiority, of behavior in
other languages, or of behavior on other task shapes. The proposed success margin (two of eighteen) is an operational
tolerance. It is not a statistically established non-inferiority margin, and no p-value is used anywhere.

## Arms and pins

| Arm | System | Transport / settings |
|---|---|---|
| A | DaVinci, PR #193 binary | `transport: "auto"` (HTTP) |
| B | the same binary, same settings | `transport: "public-websocket"` |
| C | Codex CLI | `ignore_user_config`, effort `high`, service tier `default` |

A and B differ only in `transport`. The evaluator rejects a study in which their settings differ in any other field.

| Pin | Value |
|---|---|
| DaVinci source | `1628438c4ebfcb1e23aee6c381533da48362eec1` (PR #193 head, CI run 38947104356 green) |
| DaVinci binary SHA-256 | `364b229275f1a7114ff36fa074bd53c9c7da3b22992b6567af921faef7e3a55b` |
| Codex CLI version | `codex-cli 0.162.1` |
| Codex binary SHA-256 | `dd13bdb162a184fa04039488b234a8258ef9b76f84e926c505d5742b19249a6a` |
| Codex code-mode host SHA-256 | `9f865a748c984492ca9b69aac75680e4954f31311c43ec33ddbdae0ed77bd4bd` |
| Model, effort, tier | `gpt-6-luna`, `high`, `default` |
| Suite digest, task ids, harness commit | in the pins file, written by `holdout_h1.py make-pins` after the suite is frozen |

Both DaVinci arms use one immutable copy of the binary outside any repository. The Codex binary and its host are
immutable copies too. The launcher refuses to start if any observed hash differs from its pin.

## Design

Six tasks, three repetitions, three arms: **54 runs, no warm-ups**. Every run is measured; nothing is discarded.

The private-suite loader runs a whole split per campaign, so the unit of launch is one campaign: one arm, one
repetition, all six tasks. A stage is one repetition (three campaigns, 18 runs) and uses one sign-in.

| Stage | Campaign order |
|---|---|
| 1 | `h1-01-A-r0`, `h1-02-B-r0`, `h1-03-C-r0` |
| 2 | `h1-04-B-r1`, `h1-05-C-r1`, `h1-06-A-r1` |
| 3 | `h1-07-C-r2`, `h1-08-A-r2`, `h1-09-B-r2` |

The rotation gives each arm each position (first, second, third) once. It balances drift across the three
repetitions. It does not balance drift inside one repetition: an arm that runs first in a stage runs first on all six
tasks. The report states this limit and shows results per repetition so a drift pattern is visible.

Task order inside a campaign is fixed by the harness. Task ids are opaque (`h1-01` to `h1-06`).

## Measures

Per run, from the harness row: verified success (`pass`: exit 0, hidden grader passed, no unrelated edit, clean
cleanup, no transaction or artifact leak), wall time, requests, uncached input (input minus cached), output tokens
(reasoning included for every arm, checked per row), provider attempts, transports used, attempts without usage.

An **unrelated edit** is any changed file outside the files the reference commit touches. It fails the run. The
prompts name the allowed files (authoring brief, section 4).

Failed runs stay in every denominator. Unknown usage stays unknown and is never imputed.

## Decision rules (fixed before any run)

Each DaVinci arm is compared with Codex over the 18 (task, repetition) blocks where both finished.

**1. Success.** Let `d` and `c` be verified passes of the DaVinci arm and Codex, out of 18.

- `AHEAD`: `d > c` and no task has `c_task - d_task > 1`.
- `MATCHES`: `d >= c - 2` and no task has `c_task - d_task > 1`.
- `NO_SIGNAL`: both pass nothing (`d = c = 0`). Equal zeros are not a match. Efficiency is not scored.
- `BEHIND`: otherwise. Efficiency is then not scored.

**2. Efficiency**, scored only for `MATCHES` or `AHEAD`. All three must hold:

- (uncached input + output) per verified success, DaVinci / Codex, is at most 0.90.
- The median over tasks of each task's median-wall-time ratio, DaVinci / Codex, is at most 1.00.
- DaVinci is faster on at least 3 of the 6 tasks (task median wall time).

Outcome labels: `MATCHES_AND_EFFICIENT`, `MATCHES_NOT_EFFICIENT`, `AHEAD_AND_EFFICIENT`,
`AHEAD_NOT_EFFICIENT`, `BEHIND`, `NO_SIGNAL`. If Codex has no verified success, tokens per success has no Codex value
and the label is `AHEAD_EFFICIENCY_UNDEFINED` (or `MATCHES_EFFICIENCY_UNDEFINED`): no efficiency claim is made.

**3. B against A (default gate).** Gate `PASS` if B has at least as many verified passes as A over the paired blocks
and the fallback evaluation (rule 4) is `NO_FALLBACK_OBSERVED` or `ACCEPTABLE_PENDING_REVIEW`. Gate `FAIL` if B has
fewer passes or the fallback evaluation is `NOT_ACCEPTABLE`. Gate `UNVERIFIED` if the fallback evaluation is
`UNVERIFIED` and neither of the `FAIL` conditions holds (`FAIL` takes precedence). The evaluator reports gate evidence only. It never makes the production decision: the result field
`decision` is always `HUMAN_REVIEW_REQUIRED`.

**4. Fallback evaluation (arm B).** A fallback event is a B run with any request labeled `http`. For each event the
report lists the run, the HTTP and WebSocket request counts, failed attempts, logical requests against provider
attempts, whether the run passed, and a classification:

- `LOST_OR_DUPLICATED`: a tool call that started twice, ended twice, never ended, or ended without starting, in the
  transcript; or executed operations different from tool calls.
- `UNVERIFIED`: the transcript is missing, holds no tool events, or its tool-call starts do not equal the row's tool
  call count; or the operation counts are not available.
- `RECOVERABLE`: none of the above, and the run passed.

The verdict is `NOT_ACCEPTABLE` if any event is `LOST_OR_DUPLICATED` or any fallback run failed; `UNVERIFIED` if any
event is unverified; otherwise `ACCEPTABLE_PENDING_REVIEW`, and a human reads every event. A fallback rate below 10%
is not the criterion and is not reported as one. With zero events the verdict is `NO_FALLBACK_OBSERVED`, reported with
the rule-of-three bound (3/18, about 17% per run, 95% upper) and the statement that zero events do not show an
acceptable production rate.

Also reported without thresholds: request ratio, wall ratio, passes per task and per repetition, and unrelated-edit run
counts.

## Stops and invalid states

A stop ends the study. No run is repeated, replaced or stitched in. The evaluator replays the stop rules over the
rows in launch order, so a run that slipped past a live stop still produces a `STOPPED` verdict.

- Any harness `stop_reason` that is not null (including a credential or refresh failure and a cleanup failure).
- An identity mismatch: binary, source, model, effort, tier, suite digest, task id, split, language, variant,
  repetition, transport label for the arm, A/B settings difference, fixture change, Codex version or settings.
- A transport count that is not a non-negative integer, that does not add up to the row's provider attempts, or an arm
  B run with no WebSocket request.
- A `pass` flag that is not a boolean, or is true while the exit code is not 0, unrelated files changed or cleanup
  failed.
- A DaVinci attempts-without-usage value that is missing or not a non-negative integer. More than 1 DaVinci attempt
  without usage in a run, or more than 3 in total (Codex rows do not count); a failed DaVinci run with any attempt
  without usage; any B run with an attempt without usage or without input tokens.
- A Codex run without complete request telemetry.
- Missing output tokens.
- Safety: B is 2 or more verified passes behind A over the paired blocks finished so far.

Result states: `INVALID` (bad pins, unreadable or malformed result rows), `CONTAMINATION_REVIEW`, `STOPPED`,
`NEEDS_LEAK_SCAN`, `INCOMPLETE` (missing or unscorable runs; no verdict, no dropped blocks), `COMPLETE`.

A run is unscorable, never zero, when its wall time is missing or not above 0, its requests, token totals or unrelated
list are missing or malformed, or cached tokens exceed input tokens.

The numbers stay unread until the leak scan has run. The scan reports one of `CLEAN`, `LEAK`, `NOT_SCANNED` (nothing
was scanned, or the suite cannot be read), `INCOMPLETE_SCAN` (fewer stdout transcripts than runs) or `WRONG_SUITE`
(the suite digest differs from the pin). Only `CLEAN` lets the numbers through. Any other status gives
`CONTAMINATION_REVIEW` and no use of the result until a human has reviewed it, even when a stop also occurred. A
stop reason on its own is operational information and is shown at once; the safety stop names how many passes B
trails A by.

## Privacy safeguards

- The suite lives outside the harness checkout, the results root, the system temp directory, the working directory
  and the agent configuration directories (`.pi`, `.codex`, `.claude`, `.davinci`). `check-location` refuses an
  overlap in either direction, and the launcher runs it before every stage.
- The suite digest and the opaque task ids are the only suite facts in the pins file and the reports.
- After the runs, the scanner searches agent transcripts, stderr and telemetry (never credentials or binaries) for the
  suite path (any case, Windows, POSIX and MSYS spellings) and for every hidden-test and reference-solution line
  of 60 characters or more that is absent from the starter and the prompt. It compares the file as written and the strings a
  JSON reader decodes from it, so escaped quotes, tabs and non-ASCII characters do not hide a line. The authoring brief
  requires a canary line in each hidden test file. The report carries counts per campaign and no matched text.
- Limit of the scan: it matches whole lines, case-insensitively. A transcript that quotes only part of a line, or
  rewrites it, is not caught. The canary, the path check and the storage location are the other safeguards.
- Reports are built from numbers and ids only. A test checks that no hidden line, prompt text, path or test output
  appears in a complete report.
- Limit that no safeguard removes: the harness grades with `diagnostic-only` isolation, and an agent process can read
  the filesystem and its own credentials. Secrecy depends on where the suite is stored and on the scan. The harness
  states that private inputs do not establish an independent grading boundary; this study does not claim one.

## Allowance (estimates, not measurements)

DaVinci used about 0.23 points of the 5-hour window per run in the 39-run confirmation study on public tasks. Codex
per-run cost has not been measured. Independently authored tasks may be larger than the public ones.

| Item | Estimate |
|---|---|
| Runs | 54 (36 DaVinci, 18 Codex) |
| 5-hour window | 12 to 22 points in total, 4 to 8 per stage |
| Weekly allowance | 4 to 8 points |
| Wall time | 2.5 to 3.3 hours of runs, plus three sign-ins and checks |

Start conditions for each stage: 5-hour window below 60% used, weekly below 80% used, a fresh sign-in, no stop in an
earlier stage. If the window cannot cover a stage, the stage waits. It is not shortened.

## What this cannot show

- Anything about languages other than Python, or repositories without pytest.
- Anything about Claude Code. That comparison (H2) is a separate, unapproved study and needs its own adapter.
- A production fallback rate. Eighteen B runs bound it loosely at best.
- Independence of grading. See the privacy section.

## Launch readiness

The frozen suite must have exactly six Python tasks, all in the holdout split, all marked large, each with its own
repository id and an opaque id. `make-pins` refuses anything else.

Not ready. Open items, none of which this document can close:

1. The independent author and the six tasks (authoring brief), delivered, validated, frozen and digest-pinned.
2. A pins file written from the frozen suite, and the harness commit named in it.
3. A fresh sign-in at the start of each stage.
4. The suite stored outside every forbidden location, confirmed by `check-location`.
5. Codex authentication path for the Codex arm checked in a no-model dry run.
6. Explicit approval to launch H1.
7. PR #193 is not merged. The study uses the PR head binary by hash; merging is a separate decision.
