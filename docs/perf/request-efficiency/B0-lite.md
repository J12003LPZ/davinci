# B0-lite — accepted measurement baseline

Accepted as the instrumented baseline after the owner-authorized CSV A/B and
the complete resumed screen. This is not an optimization or parity claim.
Earlier stop decisions below are retained as history and are superseded by
the final results in this section.

## Final screen and decision

Campaign: `C:\Users\sergi\davinci-bench-evidence\request-efficiency\campaigns\b0-screen-resumed-20260926`.
The runner exited zero; the manifest gate verified 48 unique pinned rows.
All 48 rows have complete request telemetry and usage. Both arms used
`gpt-6-luna`, medium effort, default service tier, and counterbalanced order.
No unrelated changes or transaction artifacts were observed. Grading is
diagnostic isolation only, not an enforced private-filesystem boundary.

| Metric, all 24 runs per harness | DaVinci B0 | Codex 0.157.1 |
|---|---:|---:|
| Passes | 24/24 | 22/24 |
| Median wall seconds | 35.807 | 30.051 |
| Mean wall seconds | 36.174 | 33.925 |
| p90 wall seconds | 43.888 | 46.248 |
| Total wall seconds | 868.181 | 814.197 |
| Logical requests | 175 | 157 |
| Provider attempts, including prewarm | 175 | 181 |
| Prewarm attempts | 0 | 24 |
| Gate reminders | 16 | unavailable |
| Auto-verification runs | 0 | unavailable |
| Requests after first reminder | 39 | unavailable |
| Mutations after reminder | 0 | unavailable |
| Model-visible tool calls | 211 | 135 |
| Executed leaf operations | 237 | unavailable |
| Uncached input tokens | 156351 | 356573 |
| Cache ratio | 0.918921 | 0.911676 |
| Output tokens, including reasoning | 25576 | 27988 |

Across the exact 24 pairs, the median DaVinci/Codex wall ratio is 1.1911,
with median paired delta +5.845 seconds. Across the 22 both-successful pairs,
the corresponding ratio is 1.1485 and delta +4.707 seconds. Failures remain
in the all-run metrics. Codex failed `t2-duration` repetitions 0 (14/17 tests)
and 2 (16/17); B0 passed both. Per-task summaries, paired token/request deltas
and metric availability are saved in `screen-analysis.json`.
DaVinci first/later request cache ratios are 0.917780/0.919084; Codex's
aggregate usage does not expose that split. Tool-call units differ by surface.

The contemporaneous parent comparison is limited to the separately authorized
CSV A/B: parent 9/10, B0 10/10, with no parent-pass/B0-fail pairs. The parent's
failure reproduced the region-key case-folding error. See `B0-csv-ab.md`.
There is no full eight-task same-campaign parent arm. Historical results are
not a causal comparison. Neither the CSV sample nor the component overhead
microbenchmarks proves negligible end-to-end instrumentation overhead.

**Accept B0-lite as measurement infrastructure and the parent for V1.**
The full screen, provider counters, isolated configuration, replay fidelity,
local tests and immutable binary are established. The user explicitly cleared
the CSV stop when the baseline reproduced the failure. No request/latency
improvement, default promotion, or general correctness advantage is claimed.
The next checkpoint is V1 (Tasks 2, 3 and 3.2a); B0-full grading isolation,
larger-task coverage and promotion statistics remain later requirements.

## Workspace and preserved evidence

- Branch: `perf/request-efficiency`.
- Worktree: `C:\Users\sergi\.claude-worktrees\davinci\request-efficiency-20260925`.
- Freshly fetched base: `eb542dd03ce034dbb8033a0d289baa2b78b8aa55`.
- Preserved historical campaign at `C:\Users\sergi\davinci-bench-evidence\2026-09-25-h2h`.
  Python SHA-256 comparison checked all 1,877 source files against the copy:
  zero missing or mismatched files. The source and copy remain read-only inputs.
  The first PowerShell hash attempt failed because Get-FileHash was unavailable;
  its apparent zero mismatch count is invalid and was superseded by Python.

## Current changes and verification

Stream parsing no longer labels Codex user turns or
completed DaVinci assistant messages as measured provider requests. Missing and
partially missing usage remains unavailable through aggregation and display.
The campaign gate now validates completeness against the pinned manifest;
passing it is not checkpoint acceptance. Parent/candidate comparison retains
failed rows, exact pair membership, unavailable metrics, and pass regressions.

The benchmark helpers now distinguish retries, logical requests, gate reminders,
and automatic verification calls. Campaign helpers cover frozen fixture hashes,
balanced execution order, composite task success, and campaign integrity.
Malformed row identities are rejected without crashing. The collector, campaign identity, timing, isolation, and failure preservation
are wired into the runner. Checkpoint source provenance is bound to copied
binary bytes through a required sidecar. File-inventory failures preserve an
unsuccessful row with unknown safety results before stopping.

Rust telemetry now observes provider dispatch and attempts, including HTTP
retries and WebSocket sends, and forwards observations through agent events.
The records exclude prompts and response text. Leaf-tool dispatches are counted
at the executor boundary, excluding batch wrappers, pre-dispatch denials, and
journal replay. Post-reminder mutations use completion-ledger generation
observations, not a general filesystem watcher. Unknown attempt outcomes remain
incomplete telemetry. Usage reports include reasoning when available and
first/later cache totals where the event stream supports them.

Validation recorded during this implementation:
- Python unittest discovery: 53 tests passed, including cache missingness,
  usage deduplication, paired metrics, immutable provenance, and failure rows.
- Codex OTLP logs and traces are wired into the runner. A live minimal-prompt
  smoke on Codex 0.157.1 returned exit 0, one logical request, two provider
  attempts (one prewarm), and zero collector errors. Model-catalog HTTP calls
  are excluded using a sanitized endpoint category. Raw prompts, headers,
  resource attributes, and span events are not retained.
  Evidence: `C:\Users\sergi\davinci-bench-evidence\request-efficiency\otel-smoke-c6c7c7eb9e42439c8a5da4d0c67c106e.json`.
- `davinci-ai` library suite: 279 passed, including HTTP retry observations;
  one explicit overhead diagnostic ignored in the normal suite and run
  separately in release mode (passed).
- `davinci-agent` library suite: 1,076 passed after dispatch-counter additions.
  Intermediate event-sink/list parity and start-order regressions were corrected
  and the full affected library suite rerun successfully.
- Baseline offline release build: passed, 201 crates. Rust 1.83.0 was verified.
- Final telemetry coding-agent offline release build: passed in 5m25s.
  Source identity was checked unchanged before copying the release binary.
- Replay example: six tests passed (early termination, reminder divergence,
  actual file mutation, tool-outcome divergence, and independent completion
  ordering, and keeping native caches outside the public repository).
  No provider is constructed.
- Frozen fixtures were absent from this worktree. The unchanged baseline
  generator created them. All eight starter versions failed and all eight
  reference solutions passed their original graders. Test counts by task:
  7, 17, 6, 6, 4, 8, 6, 18. The initial missing-fixture attempt is invalid.
  Evidence: `C:\Users\sergi\AppData\Local\Temp\davinci-fixture-validation-csaf5di1`.
- Initial 24-run offline replay matched 165 requests and the historical reminder
  counts, but four runs had extra tool errors caused by incomplete replay setup.
  Those runs are not fidelity evidence. Native tools, Windows cwd handling, and
  a tool-outcome divergence guard were added. The corrected 24-run replay
  completed with 165 requests, 15 verification-required reminders, and zero
  divergences. Per-run reminder and failure counts match all historical runs.
  Evidence: `C:\Users\sergi\davinci-bench-evidence\request-efficiency\replay\b0-corrected-20260926-05`.
  Immutable replay binary: `C:\Users\sergi\davinci-bench-evidence\request-efficiency\checkpoints\b0-replay-20260926\bench_replay.exe`.
  SHA-256: `5df3206529288a751d7c8158f046d7ee837683e47f395e8fa11c5b00008e9bf5`.
  This is a debug offline-replay binary, not the production B0 checkpoint.
  The guard compares tool IDs and success/failure, not exact output bytes;
  shell timing and path text differ across fresh repositories.
- A later instrumentation replay exposed one cache-file deletion race in
  `t6-bookings-r1`. Replay had placed native caches inside the public workspace,
  unlike the CLI's separate agent directory. That diagnostic remains at
  `replay\b0-observations-20260926-06`. A failing regression proved the setup
  issue; after correction all 24 runs again match the baseline's requests,
  reminders, tool failures, and auto-verification counts (165 requests,
  15 reminders, zero divergences).
  Final replay: `C:\Users\sergi\davinci-bench-evidence\request-efficiency\replay\b0-final-20260926-07`.
  Final replay binary SHA-256: `bdaae024b5dd13c7cef5ea2ec364bb2a1932e47f635a3d1fd558828e38607b9d`.
- Offline overhead: 21,000 metadata generations/serializations with a 64 KiB
  synthetic schema measured 0.055 ms median, 0.066 ms p95 per request. Two
  hundred local collector POSTs measured 0.59 ms median, 15.45 ms p95, zero
  errors. These are component measurements, not a live end-to-end overhead claim.
  Evidence: `C:\Users\sergi\davinci-bench-evidence\request-efficiency\telemetry-overhead-20260926.json`.

Immutable B0 release candidate:
`C:\Users\sergi\davinci-bench-evidence\request-efficiency\checkpoints\b0-lite-20260926\davinci.exe`.
SHA-256: `f48278e9e52586d9b75977881205adfdeb1b795190178b7f6eaacb6b2fe41dcc`.
Its adjacent identity sidecar records the verified source and dirty-diff hashes.
The later replay-cache correction affects only the offline example, not this
production executable.

Immutable baseline binary:
`C:\Users\sergi\davinci-bench-evidence\request-efficiency\checkpoints\baseline-eb542dd0-20260925\davinci.exe`

SHA-256: `2027a2a55f51391f9d9aee24b52fb5b280cd3f8fda1c12d13ae33b8ddcd9c831`.
This identifies the unchanged parent, not a completed B0 candidate.

## Earlier pending work (resolved)

- The model-selection mismatch was resolved by explicit catalog pinning.
  The final resumed screen and CSV A/B above supersede this pending item.

## Instruction reconciliation

The explicit execution request supersedes the external plan's planning-only
wording. The owner's no-subagent rule supersedes agent/tournament instructions;
all work is in one session. The specifically authorized branch is used instead
of the generic session-prefixed naming convention. Local commits are authorized
only for accepted checkpoints; pushing, PRs, merging and installation are not.
Task 10B and promotion-grade campaigns require the owner's decision. The
repository installation rule explicitly permits source-only delivery, and the
owner explicitly excludes Task 16. No dependency pins or toolchain were changed.
The referenced `docs/CODEX-NAVIGATION-GUIDE.md` does not exist at the base.

RTK is in use. The owner subsequently instructed this session to stop using
Headroom; no further Headroom calls are permitted for this task.

## Earlier resume checkpoint (historical)

Continue in this worktree with Task 1, not V1. The baseline build has completed.
The temporary sandbox write restriction was lifted and Python checks rerun.
Do not run a campaign until
B0-lite instrumentation and local checks are ready, and do not overlap work
with live campaign timing. Preserve the entire subsequent checkpoint sequence
from the external implementation plan; none has been completed or rejected.


## Interrupted live screen: model-selection blocker

On September 26, the fresh `b0-screen-20260926` campaign used the existing
legacy `.pi/agent` directory only as the credential-copy source. Owner
credentials and settings were not changed. The campaign requested
`gpt-6-luna`, medium effort, for both harnesses.

The first DaVinci row exited 1 before executing tools. Its actual provider
observations identify `openai-codex/gpt-5.3-codex-spark`, not the requested
model. The HTTP fallback returned 400: that model is not supported with this
ChatGPT account. There was one logical request and two provider attempts
(WebSocket followed by HTTP). The unchanged starter passed 5/7 hidden tests;
this is a failed run, not an implementation regression measurement.

The first Codex row completed successfully (7/7 hidden tests, 22.665 seconds,
5 logical requests, 6 provider attempts including 1 prewarm). The campaign
was then terminated, including its process tree. Only two complete result
rows were persisted; any subsequent interrupted work is not a result.

Evidence directory:
`C:\Users\sergi\davinci-bench-evidence\request-efficiency\campaigns\b0-screen-20260926`.

**Decision: B0 is not accepted.** This partial campaign cannot establish
same-model parity, an efficiency improvement, or parent/candidate regressions.
Do not resume or append to it. No checkpoint has been committed. Model
resolution under isolated settings must be resolved before another live
campaign; changing the benchmark model silently is prohibited by the plan.
The model-selection cause has not yet been established. No claim of expired
credentials or exhausted usage limits is supported by the observed error.


### Model-selection diagnosis and setup correction

The CLI resolves an unknown explicit model into a temporary custom record, but
`complete_prompt_with_host` selects again from the installed catalog and falls
back to the first provider model when the requested ID is absent. The isolated
campaign directory omitted the remote catalog. The current owner catalog at
`.davinci/agent/models-store.json` contains the requested `gpt-6-luna` record;
the legacy `.pi` catalog does not. Credentials and catalog therefore have
separate sources for the next campaign.

The benchmark now supports an explicit `--model-store` input. It copies only
the exact requested public Codex model record into campaign isolation, pins
its hash in the manifest, and checks that hash before each DaVinci run. It
rejects records with credential/header fields or a different provider route.
A observed coding-model mismatch stops further scheduling and preserves the
failed row. Production model-resolution behavior was not changed.

Two regression tests failed before implementation and passed afterward;
all 53 Python benchmark tests now pass. A fresh live screen is still required.


## Stop-rule result: pinned-catalog screen rejected

The isolated CLI was verified offline to list `openai-codex/gpt-6-luna` with
272K context and 128K maximum output after installing the pinned record and
copied credentials. The fresh `b0-screen-pinned-20260926` screen then made
requests successfully on the requested model. Model/catalog checks passed.
Production executable bytes remained unchanged.

The screen was terminated after the second DaVinci CSV run failed all four
hidden tests. The supplied parent passed all four for `t5-csv`, repetition 1
(and both other CSV repetitions). A local regrade with tracebacks suppressed
reproduced four assertion failures. The generated implementation case-folds
returned region keys. This is an incorrect task result, with no demonstrated
infrastructure explanation. No hidden tests, reference solutions, or task
fixtures were edited or supplied to the coding agents.

The runner had completed one additional bookings row by the time termination
reached the process tree. There are 27 complete rows (14 DaVinci, 13 Codex);
the subsequent interrupted run is excluded, not counted as a success. The
process handle returned exit 1 after termination. No campaign remains running.

**Reject B0 for acceptance; stop before V1 under the owner's regression rule.**
This historical parent comparison is not contemporaneous causal evidence:
configuration isolation and Codex version differ from the old campaign. It
establishes an observed task-pass loss, not that instrumentation caused it.
Do not waive the loss as model noise. No accepted or committed checkpoints.

Partial matched metrics below cover exactly 13 completed task/repetition pairs,
including failures; the unpaired bookings result is excluded. They are not a
48-run screen and cannot support an efficiency or parity conclusion.

| Metric | DaVinci | Codex |
|---|---:|---:|
| Passes | 12 | 10 |
| Median wall seconds | 38.1667 | 33.6603 |
| p90 wall seconds | 49.5786 | 57.3262 |
| Total wall seconds | 489.1337 | 466.4113 |
| Logical requests | 101 | 85 |
| Gate reminders | 9 | unavailable |
| Tool calls | 136 | 70 |
| Uncached input tokens | 97652 | 154304 |
| Cache ratio | 0.9121 | 0.9297 |
| Output tokens | 13646 | 14255 |

Paired deltas/ratios, unmatched totals, and the expected incomplete-membership
errors are saved in `interrupted-analysis.json` inside the campaign directory.
Same-campaign parent metrics remain unavailable. The next action requires owner
direction because the explicit execution request requires stopping on a
non-infrastructure hidden-pass loss.


## Owner-authorized resolution of the CSV stop

The same-campaign `t5-csv` A/B completed 10 runs per binary: baseline 9/10,
B0 10/10. Baseline repetition 8 failed all four hidden tests. Under the owner's
explicit rule, this demonstrates that the observed failure can occur without
B0 and allows resuming the plan as model variance. See `B0-csv-ab.md` for
identities, validation, results and limitations. The earlier stop decision is
superseded; B0 still needs its complete paired screen before acceptance.
