# DaVinci request-efficiency implementation — scoped final summary

Status: **authorized implementation and local validation complete; promotion and
installed delivery remain explicitly out of scope**.

The work is on branch `perf/request-efficiency` at local commit
`c5b969eb`. The final candidate executable is frozen outside the repository at
`C:\Users\sergi\davinci-bench-evidence\request-efficiency\checkpoints\f1-final-20260926\davinci.exe`.
Its SHA-256 is
`a90e8fced7b173da1068dced65d62e067f422a6773df54ddfbe5d9c8472b570e`; its
source commit is `51e46ccc1f70c32f465c3ad6ea12be80308a3c63` and its recorded
dirty-tree hash is
`0bc22e9be995aa125cd9168d15c436ab96aa9ecdc8c0ba14a11d96a7138b1311`.

## Decisions by checkpoint

| Checkpoint | Decision |
| --- | --- |
| B0-lite | Accepted as the measurement baseline; the same-campaign t5 A/B recorded baseline 9/10 and B0 10/10, so baseline model variance remains visible. |
| V1 | Accepted deterministic verification repair; complete screening and replay evidence retained. |
| N1 | Accepted local correctness and namespace-normalization checkpoint. |
| E1 | Wrapper-only latency correction rejected for promotion; the verified guidance/measurement work remains documented. |
| P1 | Accepted for continued screening; freeform patch and grammar paths are covered by focused tests. |
| L1 | Accepted as diagnostic screening; no default lean-tool promotion. |
| G1 | Stable prompt profile retained; preview guidance was rejected for promotion. |
| S1 / A1 | Fixed reasoning-summary behavior retained; adaptive effort was rejected after observed t5 regressions. |
| C1 / W1 | Cache identity correction accepted; prewarm was closed conservatively because live evidence showed no profitable production path. |
| J1 / J2 | Jev runtime and deterministic requirement checks implemented as bounded, opt-in shadow behavior; no default completion or family-routing promotion. |
| T1 | Fast service tier measured as a separate product arm; default service tier unchanged. |
| F1 | Screening complete with a fresh parent control; no promotion or hidden-test parity claim. |

## Final paired configuration and scope

The F1 screen used `gpt-6-luna` at medium effort, fixed effort policy, stable
prompt profile, full tool surface, `autoVerify=true`, decision intelligence
disabled, and default service tier. The public model catalog hash was
`66da5cf76d3fe2e700e7ec64770091c8f500d9cd681cd93b31082e2679ec9e06`; the
fixture manifest hash was
`d8d0ecb6776b76b995c17472da9bca1883bd569b69db16f70fbe0fe66aa1b4fe`.

The declared screening denominator is 12 tasks × 3 repetitions × 2 harnesses =
72 unique rows. Results were DaVinci 23/36 and Codex 20/36. A separate fresh
parent-DaVinci control used the same configuration and 36 unique rows; it
passed 21/36. The exact parent/F1 pairing found zero parent-pass/F1-fail rows.

| Harness/arm | Passes | Median wall | P90 wall | Total wall | Logical requests | Gate reminders | Tool calls | Uncached input | Cache ratio | Output |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| F1 DaVinci | 23/36 | 34.20 s | 61.26 s | 2253.05 s | unavailable | 7 | 270 | unavailable | unavailable | unavailable |
| F1 Codex | 20/36 | 33.24 s | 65.91 s | 1465.27 s | 241 | unavailable | 215 | 295,800 | 95.325% | 51,449 |
| Clean parent DaVinci | 21/36 | 42.67 s | 54.76 s | 1505.24 s | unavailable | 17 | 367 | 297,080 | 89.833% | 43,187 |

Unavailable telemetry is recorded as unavailable, never as zero. The F1 DaVinci
aggregate reasoning count was unavailable; Codex reported 11,103 and the parent
reported 3,405. The parent control had one unrelated-file row and two
`t2-duration` model failures; the F1 arm had no unrelated-file rows and no
parent-pass/F1-fail regression.

## Validation

- `davinci-agent` library tests: 1,100 passed.
- `davinci-ai` library tests: 284 passed, 1 ignored.
- `davinci-coding-agent` library tests: 1,248 passed, 16 ignored.
- Benchmark Python tests: 63 passed.
- Release build and offline locked package check passed.
- Frozen fixture validation passed all four large-task start/reference checks;
  the large-fixture unit tests passed 2/2.
- The final F1 campaign completeness gate passed 72/72 rows; the parent-only
  control had exact 36/36 task-repetition coverage.

## Remaining limits

The promotion-grade 480-row, two-window campaign and untouched independent
holdout were not run. The benchmark environment remains labeled
`diagnostic-only`, so hidden-test protection and general parity are unproven.
Task 16 (merge/install/replace the executable used by the owner) was not
authorized and was not performed. No push, pull request, merge, or dependency
change was made.

The repository-wide `cargo fmt --all -- --check` remains non-green because of
pre-existing unrelated formatting differences; the affected package tests,
build, benchmark checks, and diff checks passed. The global pre-commit hook could
not launch its `/bin/bash` shebang on this Windows checkout; an equivalent
staged-secret scan passed before the local commit was created.
