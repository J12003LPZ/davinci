# Unified execution and recovery verification

## Release context

- Plan: `docs/superpowers/plans/DaVinci_Unified_Execution_Recovery_Implementation_Plan.md`, Tasks 00-23.
- Branch: `leonardojeziellopez/unified-execution-recovery-01a0ca95`.
- Base: `origin/main` at `0d3163c30d528bcddaf9a2ee211a4ba052d7f420`.
- Task 23 starts at `15e8c394`; the final Task 23 commit is the commit that contains this report.
- Platform measured locally: Windows x86_64, Rust `1.83.0`, Cargo `1.83.0`.
- No push, merge, deployment, or remote branch-protection change was performed.

## Authority and capability result

The operation journal is authoritative for durable intent, logical operation and
attempt identity, owner generation, state transitions, recovery decisions,
result references, effect claims, and outbox delivery. ProcessManager and
supervisors own live process identity and exit evidence. Transaction
coordinators own file preimages and phases. Graph stores own topology and
worker bindings. Sessions own conversation lineage. Verification producers own
verdicts. TUI, telemetry, and runtime-bus observations remain rebuildable.

Direct and batch tools, task and worker control, shell and managed processes,
filesystem and Git observation, transactions, verification, graph/subagent
launches, browser lifetimes, external adapters, and CLI inspection now have
durable operation boundaries where their adapters can provide identity and
evidence. Existing ledgers and domain records remain evidence rather than a
second success authority.

Arbitrary shell descendants, custom extensions, hooks, remote MCP/HTTP effects,
browser-side effects, and older binaries remain conservative or unsupported
when no domain receipt or postcondition exists. Sessionless workers remain
ephemeral. No capability name was removed from the existing command surface.

## Schema, migration, retention, and privacy

- SQLite application ID: `0x44564f50`; journal schema: version 4.
- Journal path: `<root>/.davinci/operations/operations.sqlite3`.
- Schema v4 imports legacy tool-ledger and task-runtime records into
  append-only `legacy_observations` rows keyed by source digest and identity.
- Changed legacy bytes are rejected. Original files are not rewritten.
- Legacy `.pi` graph roots are checked beside `.davinci` roots; conflicting
  checkpoints fail closed. Startup performs no destructive repair.
- Bounds cover operation specs/results, attempt/event references, outbox
  payloads and batches, pending rows, snapshots, SQLite pages, task journals,
  receipts, checkpoint blobs, and inspector output.
- `OperationJournal::capacity()` reports occupancy without materializing all
  records. Graph pruning retains terminal runs whose artifacts still reference
  an operation, and retains unreadable or over-limit candidates.
- Retention does not remove unresolved attempts, effect claims, pending
  outbox, referenced artifacts, graph ancestors, task receipts, or idempotency
  tombstones. Payload and result access follows workspace/session permissions.

Migration fixtures and checks:
- `operation_migration`: 5 passed, including legacy import, source binding,
  changed-source rejection, corrupt legacy input, and no invented owner/effect.
- Runtime E2E reopens a real journal and a closed SQLite backup, then checks
  corruption and read-only stability.
- Graph legacy-root conflict and operation-reference retention tests passed.

## Focused deterministic evidence

The final focused gates include:

- `operation_properties`: 10 passed.
- `operation_crash`: 3 passed.
- `runtime_recovery_e2e`: 4 passed.
- `operation_capacity`: 5 passed; one explicit benchmark ignored by default.
- Capacity benchmark: 1 passed when run with `--ignored --nocapture`.
- Graph operation-reference retention: passed.
- Operation journal: 12 passed.
- Operation migration: 5 passed.
- Process operation: 6 passed.
- Process control: 5 passed.
- Process supervisor: 17 passed.
- Operation recovery: 8 passed.
- Operation publication: 8 passed.
- Operation verification: 4 passed.
- Operation transactions: 4 passed.
- Filesystem effect evidence: 4 passed.
- External adapter boundaries: 5 passed.
- Runtime recovery compatibility test after the Task 20 wording fix:
  `test_resume_reconciles_crashed_running_workers`: 1 passed.

The property and crash suites cover duplicate delivery, sibling isolation,
owner fencing, effect uncertainty, corruption, unknown schemas, result and
outbox publication, and child-process crash boundaries. The stable bounded
mutation harness is in `operation_properties`; optional longer fuzzing is
documented in `fuzz/README.md`.

## Task 22 local performance evidence

The measured Windows local-tempdir reference workload was:

```
benchmark=os-boundary os=windows arch=x86_64 profile=debug fs=local-tempdir
scenario=baseline-encode count=32 p50_us=1 p95_us=2 throughput_ops_s=286481.65
scenario=journaled-admission count=32 p50_us=781 p95_us=903 throughput_ops_s=1258.90
scenario=journaled-read count=32 p50_us=25 p95_us=36 throughput_ops_s=30683.67
scenario=journaled-mutation count=32 p50_us=1404 p95_us=1697 throughput_ops_s=663.02
scenario=journaled-batch count=4 p50_us=5872 p95_us=6205 throughput_ops_s=158.69
scenario=journaled-worker-read count=32 p50_us=79460 p95_us=140427 throughput_ops_s=12.49
```

These numbers are local reference measurements, not provider latency or a
cross-platform release benchmark.

## Full release gates run on the final branch

Commands were run through RTK's PowerShell proxy from the worktree.

| Command | Result | Evidence |
| --- | --- | --- |
| `cargo fmt --check` | FAIL | Existing formatting drift across graph/runtime and test files, including `operation_control.rs`, `operation_git.rs`, `operation_transactions.rs`, browser process code, and graph continuation/controller/worker-session files. |
| `cargo clippy --workspace --all-targets -- -D warnings` | FAIL | Existing lint violations in `davinci-session/src/lib.rs` and `davinci-ai/src/responses_request.rs` and `stream.rs`; no Task 23 Rust source was added. |
| `cargo test --workspace` | FAIL with baseline | Final rerun: 1,154 passed, 12 ignored, 1 failed. The only failure is `native_extensions::graph::control::tests::test_repeated_control_id_creates_one_attempt`, unchanged from `origin/main`. The Task 20 recovery-message regression was fixed and its targeted test passes. |

The full workspace test did run all workspace packages before reporting the one
baseline failure. The repository is not claimed fully green while these
pre-existing fmt, clippy, and graph-test failures remain.

## CI and required-check integration

The new workflow is `.github/workflows/runtime-recovery.yml`. It runs on
push, pull request, manual dispatch, and a weekly schedule. Ubuntu, Windows,
and macOS matrices run the deterministic operation-property/crash,
migration/capacity, CLI recovery, graph retention, and process ownership
targets. Manual and scheduled runs also execute the bounded mutation harness.

The existing CI workflows were inspected before adding this workflow. A
read-only GitHub check found:

- branch protection required-status-check endpoint: HTTP 404, branch not
  protected;
- repository rulesets: 0.

No remote required-check policy was changed. Local actionlint was unavailable,
and the pinned Docker actionlint image was absent, so workflow linting still
needs to run in CI or on a machine with actionlint. GitHub matrix execution was
not run from this local worktree.

## Inspector and incident evidence

Read-only commands are:

```
davinci inspect operation <operation-uuid> --json
davinci inspect run <run-uuid> --json
davinci inspect session <session-id> --json
davinci doctor runtime --json
```

Healthy reports exit 0. Missing, corrupt, unsupported, inconsistent, or
bounded-out reports exit 3. Exit 3 is a diagnosis and never grants repair
authority. `docs/runtime/recovery-playbook.md` covers failed result
persistence after an effect, unknown process ownership, stale worker binding,
incomplete transactions, missing evidence, corrupt journals, duplicate
collisions, legacy-root/downgrade conflicts, and storage pressure.

## Remaining unsupported guarantees

Physical disk-full and power-loss behavior beyond the exercised transactional
boundaries still requires release/CI fault-injection evidence. The batch API
keeps per-operation durable intent acknowledgements rather than claiming one
shared transaction across all members. Remote providers, arbitrary child
processes, browser-side effects, custom extensions, and older binaries cannot
be made replay-safe without a domain-owned receipt or postcondition. These
cases remain blocked or evidence-only by design.
