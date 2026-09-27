# DaVinci request-efficiency implementation — scoped final report

Status: **implementation, isolated grading support, two promotion windows, and
the fresh holdout execution are complete; the feature branch is ready for a
pull request.** Task 16 (installing or replacing the executable used by the
owner) was not authorized and was not performed.

The implementation is on branch perf/request-efficiency. It adds an explicit
grading-isolation contract to the benchmark runner, records the isolation and
executable identity in every campaign row, and rejects incomplete or mixed
campaign manifests. DaVinci promotion runs use the tested container boundary;
Codex remains diagnostic-only because the Windows Codex executable cannot be
placed inside the current Linux container. No raw prompts, hidden tests,
solutions, transcripts, credentials, binaries, or the downloaded plan are in
the repository.

## Frozen configuration

| Item | Candidate | Parent control |
| --- | --- | --- |
| Executable | C:\Users\sergi\davinci-bench-evidence\request-efficiency\checkpoints\f1-linux-promotion-20260927\davinci | C:\Users\sergi\davinci-bench-evidence\request-efficiency\checkpoints\parent-linux-boundary-20260927\davinci |
| SHA-256 | 4d06f7dd7f5d31f6d5f2d3e387d32e1d3a95d0aa917b658cbee55f08ecff7de4 | 47138c4701b8a294c632cf3e78ce164ea7ae782dd25641036bf9ecfe56f0475c |
| Version | 1.0.71 | 1.0.71 |
| Source identity | f8e10d5f4bf46a012788c632ebd762563491e600 | eb542dd03ce034dbb8033a0d289baa2b78b8aa55 |
| Dirty-tree identity | 121f753d6482e8f95ff8036e5569cd59ac5f9b985ae44d5eaec1d424a001b498 | clean |
| DaVinci grading | container | container |
| Codex grading | diagnostic-only | not run |

The frozen fixture manifest hash is
d8d0ecb6776b76b995c17472da9bca1883bd569b69db16f70fbe0fe66aa1b4fe and the
model catalog hash is
66da5cf76d3fe2e700e7ec64770091c8f500d9cd681cd93b31082e2679ec9e06. The
container image was rust:1.83-bookworm, resolved as
sha256:a45bf1f5d9af0a23b26703b3500d70af1abff7f984a7abef5a104b42c02a292b.
The independent boundary probe at
C:\Users\sergi\davinci-bench-evidence\request-efficiency\boundary-check-20260927.json
confirmed public-fixture-only read/write access, read-only agent and binary
mounts, a read-only root, dropped capabilities, no-new-privileges, and bridge
networking.

## Campaign evidence

All promotion rows used the frozen 12-task manifest and 10 repetitions. The
fresh holdout used the same frozen task families for three repetitions because
the plan contains no separate holdout manifest; it is a fresh execution, not
an independent task-family generalization claim.

| Campaign | Rows | Pass | Fail | Median wall | P90 wall | Nonzero/timeout | Unrelated | Transaction | Artifact |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Candidate promotion window 1 | 240 | 144 | 96 | 50.161 s | 90.349 s | 2 | 0 | 0 | 0 |
| Parent promotion window 1 | 120 | 74 | 46 | 66.618 s | 94.937 s | 3 | 0 | 0 | 0 |
| Candidate promotion window 2 | 240 | 141 | 99 | 51.479 s | 93.670 s | 1 | 1 | 0 | 0 |
| Parent promotion window 2 | 120 | 71 | 49 | 64.439 s | 102.538 s | 1 | 0 | 0 | 0 |
| Candidate fresh holdout | 72 | 44 | 28 | 53.735 s | 84.753 s | 0 | 0 | 0 | 0 |
| Parent fresh holdout | 36 | 23 | 13 | 66.937 s | 96.268 s | 0 | 0 | 0 | 0 |

The candidate promotion-window harness breakdown was:

| Window | Harness | Pass | Median wall | P90 wall | Uncached input | Cache ratio | Output |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 1 | DaVinci | 73/120 | 62.005 s | 108.324 s | 13,189,394 | 90.03% | 289,549 |
| 1 | Codex | 71/120 | 40.652 s | 76.362 s | 23,479,143 | 95.20% | 183,358 |
| 2 | DaVinci | 75/120 | 64.074 s | 99.954 s | 13,474,286 | 90.44% | 290,991 |
| 2 | Codex | 66/120 | 37.492 s | 71.531 s | 21,184,587 | 94.73% | 170,709 |

DaVinci logical/provider request telemetry was unavailable in these frozen
rows; it is reported as unavailable rather than inferred. Codex request
telemetry was recorded. The holdout had DaVinci 23/36 and Codex 21/36 for the
candidate, and DaVinci 23/36 for the parent; all holdout rows were clean on
process exit and boundary-leak checks.

## Discrepancy review

- Window 1 retained candidate t6-bookings exit 137 and t1-intervals timeout,
  plus parent t6-bookings exit 137, m2-public-api-migration timeout, and
  m3-state-persistence exit 143. The interrupted parent m3 row was repeated
  with identical settings; the recovery exited normally but still failed its
  grader, so the original row was not removed.
- Window 2 retained candidate and parent t6-bookings exit 137. Candidate
  Codex t7-cli repetition 7 exited normally and passed its public grader but
  reported the generated string Don't don't, WORD word... alpha! as an
  unrelated change, so it remains a failed row. There were no transaction or
  artifact leaks.
- The historical same-campaign t5-csv A/B remains visible in
  docs/perf/request-efficiency/B0-csv-ab.md: baseline 2027a2a5… passed 9/10
  with one failed repetition, while B0 f48278e9… passed 10/10. Per the
  requested rule, the baseline failure is recorded as model variance rather
  than attributed to B0.

These retained failures are why the report makes no general “DaVinci is ahead”
claim. The protected hidden-grader boundary is verified for DaVinci container
runs; Codex diagnostic rows are useful comparison evidence but are not an
independently protected Codex acceptance result.

## Validation performed

- python -m unittest discover -s scripts/bench/tests -p test_*.py: 65 passed.
- Legacy and large fixture validation: every broken start failed for the
  intended reason and every reference fixture passed its public and hidden
  checks.
- Candidate promotion completeness gate: 240 unique pinned rows.
- Candidate holdout completeness gate: 72 unique pinned rows.
- Direct manifest validation: zero errors for both promotion windows and both
  holdout directories.
- Independent container boundary smoke test: passed.
- Earlier affected Rust validation: davinci-agent 1,100 passed;
  davinci-ai 284 passed and 1 ignored; davinci-coding-agent 1,248 passed
  and 16 ignored; release build passed.
- git diff --check: passed. The repository-wide Rust format check remains
  non-green because of pre-existing unrelated formatting differences.

Raw campaign output and aggregate files remain outside git under
C:\Users\sergi\davinci-bench-evidence\request-efficiency\campaigns.
Hosted CI failures were ignored as requested.
