# DaVinci versus Codex CLI benchmark

This harness produces **diagnostic screening evidence**. The eight legacy Python
tasks and four larger tasks have public generators containing their graders and
reference solutions. Calling those files `hidden` only describes when the local
grader is copied. It does not make an independent holdout.

Each run gets a fresh Git repository. Grading starts only after the native
process tree has been reaped or the named container has been removed.
A failed cleanup stops the campaign, retains an ungraded failed row, and never
copies the hidden grader into a workspace that may still have a live writer.

## Boundary and credential scope

`BENCH_DAVINCI_GRADING_ISOLATION=container` limits host filesystem mounts to the
public workspace, copied campaign agent directory, and executable. The root is
read-only, capabilities are dropped, and no-new-privileges is set. Docker
containers have unique names and a CID file outside the public workspace; the
runner removes the actual named container on timeout and on normal exit, then
checks that it is absent. Daemon errors do not establish successful cleanup.
Every measured container and container preflight has a CID file. Missing,
empty, or malformed creation acknowledgment blocks grading even if an abrupt
client exit is followed by a transient `no such container` response: the
daemon could still be processing the create request.

Native runs use a dedicated Linux child subreaper, which kills and reaps all
remaining descendants after the launcher exits, including detached or
double-forked writers. It confirms that it has no children before reporting
cleanup. `/proc` must expose the supervisor's own PID namespace so the owned
child IDs can be used safely. If that proof is unavailable, the native arm is
rejected before the benchmark command starts.

Native Windows runs use `windows_job.py`: the command starts suspended, is
assigned to a new Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, and only
then resumes, so every descendant belongs to the job. After exit or timeout the
runner terminates the whole job and reports cleanup only when the kernel shows
zero active processes in it. If the orchestrator dies, the job handle closes
and the kernel kills the tree. Output goes to temporary files rather than pipes,
so a descendant holding an inherited handle cannot stall the measured exit.
This makes a native Windows DaVinci-versus-Codex campaign possible again.
macOS native arms remain unsupported. This process supervision does not add a
security sandbox.

**This is not a credential or independent hidden-grader boundary.** DaVinci and
its tools can read the mounted `auth.json`. Bridge networking permits outbound
requests, including retrieval of public fixture generators. Read-only auth
mounts prevent modification, not reading or exfiltration. Native Codex also has
host and account access. Use a dedicated benchmark account and keep campaign
directories and transcripts private. The copied auth file uses mode 0600 and
its parent uses 0700 on systems that enforce POSIX modes; these permissions do
not separate an agent from its tools. Credentials are excluded from structured
identities, but an agent can put sensitive content in a transcript.

Every new manifest records `promotion_eligible: false`; every arm records
`grading_assurance: diagnostic-only`, credential exposure, and public fixture
secrecy. A promotion comparison fails the independent-boundary and untouched-
holdout rules. An independently protected evaluation needs new private fixtures,
a provider credential broker outside the tool process, and an enforced network
policy. Those facilities are not supplied by this harness. `host` networking
is rejected. Codex container mode is rejected until its authentication, workspace
paths, and telemetry transport have a supported container configuration.

## Prerequisites and offline checks

Use Python 3.11 or later for the runner. The grader's Python needs pytest.
Native Linux harnesses need `python`, `python3`, `pytest`, Git, and Bash on PATH,
plus the process ownership support described above. Native Windows harnesses
need `python` (with pytest importable), Git, and Bash on PATH; `python3` is not
required because standard Windows Python installs do not provide it.
Fixture repositories are committed with `--no-verify` and a repository-local
empty `core.hooksPath`, so operator-wide git hooks cannot alter the baseline;
any failing git setup command stops the run instead of being ignored. The preflight actually
imports pytest, executes the named commands, and records versions before any
model call. Container checks run inside the resolved image with networking off.
A missing Python/pytest command or failed cleanup rejects setup.

Build the supplied image before a live container run:

```sh
docker build -f scripts/bench/Dockerfile -t davinci-bench-tools .
```

The Dockerfile includes Python, pytest, Git, Bash, certificates, and ripgrep.
The runner resolves the built image to its immutable ID and launches that ID,
then rejects a changed tag during the campaign. The image and native tooling
versions remain separate in the manifest; do not infer tool parity from a
shared model name.

These commands are offline and do not consume model usage:

```sh
python -m unittest discover -s scripts/bench/tests -p 'test_*.py'
python scripts/bench/make_tasks.py
python scripts/bench/make_large_tasks.py
BENCH_RUNS=/absolute/new/fixture-validation python scripts/bench/bench.py validate --task-set all --large-manifest scripts/bench/large_manifest.json
```

The unit suite generates its own temporary large fixtures and does not require
ignored `tasks/m*` directories or pytest. The separate fixture validator requires
pytest and checks that each broken starter fails and each reference passes.
Generators overwrite their own generated fixtures: run them before freezing a
campaign, never during one. `validate --task-set all` checks legacy and large
membership without looking up legacy entries in the large-only manifest.

Large manifests now declare `hash_order: relative-posix-codepoint-v1`. Public,
reference, and grader trees are hashed in that portable relative-path order.
The previous Windows Path sort order produced different hashes on Linux. Only
the hash algorithm tag and affected public-tree hashes changed; fixture contents
are unchanged. Older large manifests without the tag are explicitly rejected
for new runs. Regenerate and freeze a new manifest, and retain the original
manifest with its historical campaign rather than rewriting history.

## Clean executable provenance

New live campaigns require a clean committed runner checkout and schema 2
checkpoint identities. Old schema 1 sidecars, null dirty hashes, dirty builds,
missing source commits, and mismatched source trees or binary bytes are rejected.
Codex source provenance is unavailable; its executable hash and version are
recorded without inventing a source SHA.

Build a DaVinci checkpoint from an already committed clean checkout:

```sh
python scripts/bench/runner.py --repo /absolute/source-checkout --output /absolute/new/checkpoint
```

This helper runs `cargo build --locked --release` in a fresh temporary target
directory, verifies the source identity before and after the build, and copies
the executable and adjacent `.identity.json` outside the repository. The sidecar
records source commit/tree, the SHA-256 of empty diff bytes, `source_clean: true`,
actual binary hash, build command, and timestamp. The runner verifies that the
recorded commit/tree exists locally. This is reproducible source attribution,
not a claim of bit-for-bit reproducibility or an external signed build attestation.

Build the parent from the actual merge-base of the candidate and the declared
PR target, not an arbitrary old ancestor. The offline comparison command verifies
that relationship against Git objects using `--base-ref` and reports failure
when the purported parent is older. Retain the resolved target SHA in the
comparison output because a moving target branch may acquire a newer merge-base.
For a parent live run, supply `--parent-for <candidate-source-sha> --base-ref
<target-ref>` to perform the same check during setup before any model usage.

## Campaign setup

**Live runs spend account usage.** Use one measurement host/account at a time,
with no concurrent development, builds, or other model jobs. A kernel file lock
in the machine's system temporary directory excludes another cooperating
benchmark campaign across output directories and checkouts. It is held through
setup and every run. Before each process launch, it also writes and fsyncs a
durable ownership record. That record is cleared only after confirmed cleanup.
An orchestrator crash releases the kernel lock but leaves its outstanding
launch record, so a new campaign refuses to start over a surviving worker or
possibly pending container. Failed cleanup also blocks another launch in the
same campaign. A crashed owner also stays unresolved between timed launches,
when setup or grading subprocesses may still be active. Corrupt or older
unrecognized lock records fail closed.

A record left by a dead orchestrator is reconciled automatically only when its
work is proven gone: the owner PID must no longer exist, every in-flight
container must be force-removed and confirmed absent, and every in-flight native
Windows run is covered by its kill-on-close Job Object (the kernel killed the
tree when the dead owner's handle closed). A live owner, a Linux native run, or
an unreadable record still fails closed; then an operator must reconcile prior
workers before resetting ownership while holding the kernel lock. If prior work
cannot be accounted for, use a clean measurement host. The record is rewritten
in place and shortened only afterwards, so a crash mid-write leaves invalid
JSON (which fails closed), never an empty file that would read as clean. On
Windows the kernel lock covers a byte far past the record, so the record stays
readable while a campaign runs. Do not delete the lock file while a process may own it; doing so can split
ownership across different inodes. This lock cannot control unrelated programs
or use of the same account on another machine. The comparison checks the full
campaign timestamp envelope, including all Codex control rows, for overlap;
DaVinci performance metrics remain separate from the control arm.

1. Commit the runner changes and build the immutable DaVinci checkpoint above.
   Set `BENCH_DAVINCI` to the copied native executable, or select container mode
   with `BENCH_DAVINCI_CONTAINER_BINARY` and
   `BENCH_CONTAINER_IMAGE=davinci-bench-tools`. Container mode uses a copied Linux
   executable. `BENCH_DAVINCI` is not used for that arm.
2. Set `BENCH_RUNS` to a new, nonexistent output directory outside Git. Reuse is
   rejected. Optionally set `BENCH_CODEX`, `BENCH_MODEL` (default `gpt-6-luna`),
   `BENCH_EFFORT` (default `medium`), `BENCH_TIMEOUT`, and `BENCH_SERVICE_TIER`.
3. Use `run --harness davinci codex --task-set legacy --reps 3 --order
   counterbalanced --order-seed 0 --variant B0`. This requests 48 rows. The full
   frozen set uses `--task-set all --large-manifest scripts/bench/large_manifest.json`
   and requests 72 rows at three repetitions. Each campaign saves the full
   schedule, fixtures, identities, effective settings, tooling preflight, lock
   scope, and boundary limitations before execution.
4. Run `report` and `gate` with the same `BENCH_RUNS`. `report` saves separate
   arm, stratum, and per-task summaries. `gate` checks only completeness and
   pinned row consistency, retaining failures. Its successful exit is explicitly
   **not checkpoint acceptance**.
5. Compare fresh parent and candidate campaigns offline:

```sh
python scripts/bench/bench.py compare --baseline /absolute/parent-campaign --candidate /absolute/candidate-campaign --base-ref origin/main
```

`compare` prints per-rule pass/fail/unavailable states and exits nonzero unless
all promotion rules pass. With the supplied diagnostic boundary and public
fixtures it cannot certify promotion. One invocation evaluates one paired window;
it marks two independent windows, deterministic code validation, and any missing
telemetry as unestablished. A parent-only DaVinci campaign is valid: its absent
Codex control is unavailable rather than an error or a fabricated comparison.

DaVinci uses copied credentials and explicit settings. `--settings` selects an
experimental configuration; defaults disable decision intelligence, use fixed
effort, stable prompt profile, full tools, and automatic verification. For a
model outside the binary's catalog, `--model-store` pins only the exact public
Codex model record and rechecks its hash. Coding-model mismatch stops a run.
Inherited product/provider/OTEL overrides are stripped. Codex uses
`--ignore-user-config` and a local sanitized OTLP collector. Source cleanliness,
binary bytes, fixture hashes, and the pinned model catalog are rechecked during
the campaign.

Both harnesses receive the same execution-environment preamble followed by the
unchanged task text. The runner has already prepared an isolated repository;
the preamble directs edits and verification to that directory and excludes
repository worktree/session bootstrap and remote shipping rituals. Repository
coding/testing guidance and task scope still apply. Instruction files are not
rewritten. This preamble is part of the runner revision and must remain fixed
within a matched comparison.

Grading saves complete local `.regression.json` and `.grader.json` receipts
beside each task workspace, outside the graded tree. They include command,
working directory, timeout, exit, cleanup status and both output streams.
Result rows contain receipt paths and SHA-256 hashes; their compact summaries
are not the full failure evidence. The visible regression receipt is saved
before hidden tests are overlaid. Fixture validation uses separate
`.starter-grader.json`, `.reference-grader.json` and
`.reference-regression.json` files. These receipts can contain private source
paths and test output; keep them with the local campaign evidence and review
them before sharing. A receipt does not establish independent grader isolation.

## Metric definitions and acceptance

### Codex subscription acceptance

Use `--subscription-policy` with an external JSON policy containing exactly
`schema_version: 1`, `billing: "subscription-only"`, `model: "gpt-6-luna"`,
`effort: "high"`, and positive integer `max_requests`, `max_tasks`, and
`max_wall_seconds`. Select only the DaVinci arm, set `BENCH_EFFORT=high`, and
provide settings with `transport: "sse"`, `effortPolicy: "fixed"`,
`retry.enabled: false`, and `retry.provider.maxRetries: 0`. Policy admission
rejects other models, efforts, billing routes, extra fields, or excessive task
schedules before launching a model process.

`--grading-timeout` freezes a per-grader and regression-process bound between
1 and 3600 seconds (default 120); select a sufficient bound for cold Rust builds
during `validate` and `run`. Subscription campaigns also clamp each grading
process to the remaining campaign wall-clock allowance.

No API spend authorized. Subscription-only campaign. Budget by subscription usage allowance, request count, task count, and wall-clock time; API-equivalent dollars are reporting-only.

This path uses existing Codex OAuth credentials and the ChatGPT-backed
`openai-codex-responses` route. It creates a shared durable request ledger and
absolute deadline, requires fresh committed receipts for every task, and stops
on quota exhaustion, failed transport, unknown response identity or accounting,
or exhausted caps. It cannot silently switch billing routes. Token usage remains
unknown where the provider omits it. Authorized API spending is zero; actual
billing is unmeasured, not a manufactured zero. Public Responses cache/output
limits are not assumed for the OAuth route.

The current adapter cannot enforce a reliable subscription allowance percentage.
Freeze conservative request/task/time caps instead and retain any available
before/after subscription-window snapshots. Those snapshots cover the account,
including other sessions. The current native supervisor owns and reaps the
process tree; it does not provide OS filesystem/network isolation or certify
hidden-grader secrecy. Independent acceptance remains a separate gate.

- **Wall time:** monotonic launch through process reaping and container cleanup.
  Grading/parsing are excluded. All failures/timeouts stay in distributions;
  paired both-successful rows are reported separately so an early failure is
  visible rather than silently rewarded as a speedup.
- **Startup state:** readiness reports retain all rows and separately summarize
  an optional workload-recorded `startup_state` of `cold` or `warm`. Missing or
  unrecognized values are `unknown`; provider cache hits and repetition order
  never establish process warmth. `attempt_outcomes` counts process completion,
  failure, timeout, abort, and unknown exit status separately from composite
  task success. Missing token usage stays unavailable in each group.
- **Task success:** integer exit zero, grader success, confirmed cleanup, no
  unrelated changes, no transaction leak, and no detected forbidden artifact.
  Missing evidence cannot pass. Cleanup and inspection failures are written as
  failed rows before the campaign stops.
- **Artifact versus transaction:** the transaction flag inspects actual
  `.davinci-transactions` paths. The independent artifact scan checks reserved
  hidden-grader names, credential files, harness artifact directories, and
  symlinks before grading, including Git-ignored paths. It records paths and its
  scan scope. False means no match in this bounded scan; it cannot establish no
  network retrieval, memory contamination, or concealed content leak.
- **Requests:** coding logical requests and actual provider attempts remain
  distinct; retries share logical identity. Codex user turns are not request
  counts. Prewarm and Jev are separate. No Jev observations means unavailable,
  even if coding telemetry exists. Reports show available and complete row
  denominators; incomplete request telemetry cannot pass request-reduction rules.
- **Tokens:** input is normalized total input including cached input. Uncached
  input is computed per row as `input_tokens - cached_tokens`; missing or invalid
  operands remain unavailable. Cache ratio is summed cached divided by summed
  total input. Output includes Responses reasoning; reasoning is separately
  available only when reported. First/later cache ratios use deduplicated
  terminal usage where available.
- **Tools and reminders:** top-level calls, reported batch children, and actual
  leaf dispatches remain separate. Reminder reasons, harness verification runs,
  requests after the first reminder, and completion-ledger mutation generations
  are reported; generations are not a census of filesystem writes.
- **Pairs and uncertainty:** candidate divided by parent uses exact unique
  task/repetition pairs. Bootstrap resamples whole task clusters, retaining all
  repetitions together, with 2,000 deterministic resamples and a 95% percentile
  interval. Reports distinguish ratio of medians, median paired ratio/delta,
  and p90 ratio. A single task cannot establish cluster uncertainty. Twelve
  public task families still support a narrow diagnostic conclusion.

The per-window promotion report checks clean source/actual merge-base,
sequential execution, usable comparable tools, boundary and holdout evidence,
frozen legacy/large coverage, ten repetitions per task, independent windows,
deterministic tests, aggregate correctness, every unreviewed parent-pass/candidate-
fail pair, at least 10% lower median, at most 10% p90 regression, latency
uncertainty, logical-request and reminder-continuation reductions, and per-task
uncached/output token regressions. A missing rule is unavailable and blocks
acceptance. Aggregate gains do not erase individual correctness regressions.

## Offline replay and cache diagnosis

The `davinci-coding-agent` example `bench_replay` drives the actual agent loop
with recorded assistant responses and real tools in a fresh public repository,
without a live provider. Build it and select an immutable copy with
`replay.py --replay-binary`; also supply `--recordings`, `--original-runs` (the
historical path prefix to remap), and a fresh `--runs`. Replay stops on divergent
reminders, tool IDs/outcomes, or early completion. It evaluates harness behavior,
not how a live model would react to changed prompts.

`cache_report.py` reads an existing `DAVINCI_WIRE_DUMP` directory for continuation,
prefix, and cache ratios. Missing usage remains unavailable. Wire dumps contain
full conversations and file contents; keep them private and outside Git. Timed
campaigns deliberately remove inherited wire-dump settings, so collect those
separately from timing evidence.
