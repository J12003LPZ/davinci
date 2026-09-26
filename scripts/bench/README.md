# DaVinci versus Codex CLI benchmark

The legacy set is frozen to eight Python tasks, `t1-intervals` through
`t8-calc`. Runs use fresh Git repositories. The agent receives the public
starter and prompt; grading runs after process exit. Current execution is
**diagnostic isolation only**: the host filesystem does not prevent an agent
from finding private fixtures. These results cannot establish independent
hidden-test acceptance or general parity.

## Campaign setup

Live runs spend account usage. Run one campaign at a time, with no concurrent
development, builds, or other model jobs on the measurement host. On Windows,
set `PYTHONUTF8=1` before invoking Python.

1. Build the desired revision and retain its source SHA and dirty-diff hash.
   Copy `davinci.exe` to a unique absolute directory outside the repository.
   `BENCH_DAVINCI` must select that copy, not a mutable build output.
2. Write an adjacent `davinci.exe.identity.json` containing `schema_version: 1`,
   `binary_sha256`, `source_sha`, and `dirty_diff_hash`. Hash the copied bytes;
   source identity must come from the verified build, not a later checkout.
   The runner checks this sidecar and rechecks binary bytes before each run.
   `runner.source_identity` hashes tracked changes and untracked implementation
   files. Codex source provenance is explicitly unavailable; its executable
   hash and version are retained.
3. Set `BENCH_RUNS` to a new, nonexistent directory. The runner refuses reuse.
   Optionally select `BENCH_CODEX`, `BENCH_MODEL` (default `gpt-6-luna`), and
   `BENCH_EFFORT` (default `medium`). Each campaign saves its frozen fixture
   manifest, schedule, identities, and effective settings before execution.
4. Use the `run` command with `--harness davinci codex --task-set legacy
   --reps 3 --order counterbalanced --order-seed 0 --variant B0`. This requests
   48 rows. `--tasks` can select a subset for diagnosis; it does not change the
   frozen membership. `large` and `all` require `--large-manifest`.
5. Run `report` and `gate` against that `BENCH_RUNS`. The gate checks campaign
   completeness and consistency, including failed rows. Passing it is **not
   checkpoint acceptance**. Use `compare --baseline <parent-directory>
   --candidate <candidate-directory>` for paired metrics and pass regressions.

The unchanged `make_tasks.py` creates fixtures when absent; do not regenerate
or edit fixtures during a campaign. `validate` checks starter failure and
reference success offline. Keep its output directory separate from live runs.
`--help` lists the command options. Offline regression tests run through
`python -m unittest discover -s scripts/bench/tests`.

The larger stratum is generated separately so the legacy membership stays
frozen. Run `python scripts/bench/make_large_tasks.py` once before the first
model-behavior arm, review and retain `scripts/bench/large_manifest.json`, then
validate it with:

```text
python scripts/bench/bench.py validate --task-set large --large-manifest scripts/bench/large_manifest.json
```

The manifest records each public allowlist, public verification command, and
the hashes of the public starter, reference solution, and hidden grader. The
offline discovery, LSP, and browser fixtures under
`scripts/bench/specialist_fixtures/` describe availability and authorization;
they do not make a specialist-tool preference a functional requirement.

DaVinci uses a per-campaign agent directory containing only copied credentials
and explicit settings. For a model absent from the executable's built-in catalog,
pass `--model-store <existing-models-store.json>`. The runner pins only the exact
requested public Codex model record, records its hash, and rechecks it before
each DaVinci run. Provider observations stop the campaign on a coding-model
mismatch. Credentials are never included in manifests. Inherited
product, provider, and OTEL overrides are stripped. `--settings` can supply an
explicit experimental settings file. Codex uses `--ignore-user-config` and
local, sanitized OTLP logs and traces configured through CLI overrides.

## Metric definitions

- Wall time is monotonic process launch through exit, excluding grading and
  subsequent parsing. Failed and timed-out runs remain in distributions.
- Success requires exit zero, grader success, no unrelated changes, and no
  transaction artifacts. Missing file-safety evidence cannot pass; inspection
  failures are saved before the campaign stops.
- Logical requests and provider attempts are distinct. Retries share logical
  identity. Codex user turns are not request counts. Prewarm is reported
  separately; missing or incomplete telemetry is flagged, never counted as zero.
- Top-level tool calls, reported batch children, and actual leaf-tool dispatches
  are separate. Leaf dispatches include failures and exclude batch wrappers,
  pre-dispatch denials, and journal replay. They are not subprocess counts.
- Post-reminder mutations count changes to DaVinci's completion-ledger generation,
  not all filesystem writes. Gate reminders are grouped by reason; requests
  after the first reminder and harness verification runs are also reported.
- Input is provider-normalized total input, including cached input. Cache ratios
  divide summed cached tokens by summed total tokens. First/later request groups
  use deduplicated DaVinci terminal usage; mismatched request counts make them
  unavailable. Codex's aggregate turn usage cannot supply those groups.
- Reasoning counts are retained when reported and are included in the Responses
  output total. Missing usage stays unavailable through paired reports.
- Paired ratios mean candidate divided by baseline on matching task/repetition
  rows. Missing pairs are rejected; missing metric values remain unavailable.
  Screening reports do not yet provide promotion-grade confidence intervals.

## Offline replay

The `davinci-coding-agent` example `bench_replay` drives the actual agent loop
with recorded assistant responses and real tools in a fresh public repository.
It constructs no live provider. Build that example and select an immutable copy
with `replay.py --replay-binary`. Its other required options are `--recordings`,
`--original-runs` (the historical path prefix to remap), and a fresh `--runs`.

Replay checks cumulative reminders and tool IDs/error outcomes before supplying
each response. It stops when the candidate diverges or finishes early. Tool
output bytes are not required to match because paths and timing vary. Results
measure harness decisions, not how a model would react to changed prompts.
The runner never loads hidden graders or reference solutions.

## Opt-in cache diagnosis

`cache_report.py` accepts an existing `DAVINCI_WIRE_DUMP` directory. It reports
continuation and prefix changes plus first/later/all cache ratios. Missing usage
is unavailable. Wire dumps contain full conversations and file contents; keep
them outside Git. The controlled campaign environment deliberately removes
inherited wire-dump settings, so collect diagnostic dumps in a separate direct
DaVinci invocation, outside timed campaigns.
