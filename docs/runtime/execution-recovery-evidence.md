# Execution recovery evidence

## Baseline commands

All Rust checks below ran at `0d3163c30d528bcddaf9a2ee211a4ba052d7f420` in
the isolated worktree. The Cargo test and Clippy runs used `--offline --locked`
to avoid lockfile or dependency changes. This is a safe baseline variant of
the plan's commands; no CI workflow was run.

| Check | Result |
|---|---|
| `git rev-parse HEAD` | `0d3163c30d528bcddaf9a2ee211a4ba052d7f420` |
| `cargo metadata --no-deps --format-version 1` | Passed; 14 workspace packages and product version `1.0.71`. |
| `cargo fmt --check` | Failed on pre-existing formatting differences, including graph continuation/controller and worker-session code. No files were changed by the check. |
| `cargo clippy --workspace --all-targets --offline --locked -- -D warnings` | Failed on existing `clippy::needless_borrows_for_generic_args` at `crates/davinci-session/src/lib.rs:87` (`fs::create_dir_all(&dir)`). |
| `cargo test --workspace --offline --locked` | 26 suites: 1,348 passed, 1 failed, 4 ignored. Failure: `davinci-ai::stream::tests::a_live_stream_delivers_each_frame_before_the_next_is_sent`; first run received `Hel` but not the expected following `lo` frame. |
| `cargo test -p davinci-ai --lib stream::tests::a_live_stream_delivers_each_frame_before_the_next_is_sent --offline --locked` | Passed on immediate focused rerun: 1 passed, 227 filtered out. This is evidence of a timing-sensitive baseline failure, not a product fix. |

RTK retained the raw Clippy and workspace-test output in
`%LOCALAPPDATA%/rtk/tee/1790105820_cargo_clippy.log` and
`%LOCALAPPDATA%/rtk/tee/1790105971_cargo_test.log` on the audit host. RTK
reported that no automatic hook was installed; no token-savings claim is made.

## Task 02 — durable operation journal

Checks ran on Windows in the isolated task worktree, based on model commit
`01ed05cc`; the journal changes and this evidence entry are included in the
Task 02 commit `feat(runtime): add durable operation journal and schema guards`.

| Check | Result |
|---|---|
| `cargo test -p davinci-agent --test operation_journal --offline --locked` | Passed: 12 tests, exit 0. Includes child-process reopen, atomic result/event/outbox writes, injected SQLite write failure and poison recovery, corruption refusal/poisoning, bounds, schema/workspace/root guards, and backup checks. |
| `cargo test -p davinci-agent --test operation_journal --test operation_model --offline --locked` | Passed: 21 tests across both integration targets, exit 0. |
| `cargo test -p davinci-agent operations::transitions::tests --lib --offline --locked` | Passed: 1 test, 993 filtered out, exit 0. |
| Targeted `rustfmt --check` on changed Rust files | Passed, exit 0. |
| `git diff --check` | Passed, exit 0. |

The failed-write regression uses a SQLite trigger; physical disk-full and
power-loss behavior were not injected. The backup test checks that the closed
export has no WAL sidecar before opening it as a journal. All checks were local;
CI and non-Windows platform behavior were not run.

## Task 03 - idempotent admission, attempts, and owner fencing

Checks ran on Windows in the isolated task worktree. The red runs exposed the
missing admission/dispatch/owner APIs, the missing safe-recovery API, and a v2
schema path that accepted a cleared SQLite application ID; each regression
passed after its implementation fix.

| Check | Result |
|---|---|
| `cargo test -p davinci-agent --test operation_idempotency --offline --locked` | Passed: 4 tests, exit 0. Covers concurrent duplicate delivery, digest collisions, distinct operation kinds with identical arguments, and durable result replay. |
| `cargo test -p davinci-agent --test operation_ownership --offline --locked` | Passed: 8 tests, exit 0. Covers stale revisions/owners, claim and latch gating, quiescence, unstarted recovery, durable effect-latch recheck before retry, safe retry, and uncertain-effect blocking. |
| `cargo test -p davinci-agent --test operation_journal --test operation_model --offline --locked` | Passed: 21 tests across both integration targets, exit 0. |
| `cargo test -p davinci-agent --test operation_idempotency --test operation_ownership --test operation_journal --test operation_model --offline --locked` | Passed: 33 tests across four integration targets, exit 0. |
| `cargo test -p davinci-agent v1_upgrade_backfills_intent_owner_and_call_mapping_for_existing_operations --lib --offline --locked` | Passed: 1 test, exit 0. Verifies populated v1 backfill and rejection after clearing the v2 SQLite application ID. |
| `cargo test -p davinci-agent operations::transitions::tests --lib --offline --locked` | Passed: 1 test, 994 filtered out, exit 0. |
| Targeted `rustfmt --check` on changed Rust files | Passed, exit 0. |
| `git diff --check` | Passed, exit 0. |

The journal returns the complete stored result for a matching intent. This
lower-level API does not make a live user permission decision; the current
caller facade will enforce that when Task 05 routes tool admission through it.
All checks were local; CI and non-Windows behavior were not run.

## Task 05 - direct and batch admission through the operation facade

Checks ran on Windows in the isolated implementation worktree. The direct and
batch paths now share journal admission for built-in capabilities, while
existing ledger rows and non-built-in tool families retain the compatibility
ledger route. A focused runtime test also verifies operation context survives
session continuation and is scoped to a child worker.

| Check | Result |
|---|---|
| `rtk cargo test -p davinci-agent --test operation_dispatch --offline --locked` | Passed: 5 tests. Covers replay-stable child keys, planning without adapter effects, conservative unknown/custom-tool defaults, intent-persistence failure, and a non-executed post-admission denial. |
| `rtk cargo test -p davinci-agent operation_dispatch_tests --lib --offline --locked` | Passed: 2 tests. Covers direct and batch journal admission/lineage, scheduler-skipped cancellation before effects, and permission-revision denial after admission. |
| `rtk cargo test -p davinci-agent --lib operation_context_survives_continuation_and_is_scoped_to_child_workers --offline --locked` | Passed: 1 test. |
| `rtk cargo test -p davinci-agent tool_ledger::tests --lib --offline --locked` | Passed: 14 tests. |
| `rtk cargo test -p davinci-agent scheduler::tests --lib --offline --locked` | Passed: 6 tests. |
| `rtk cargo test -p davinci-agent permission_state::tests --lib --offline --locked` | Passed: 2 tests. |
| `rtk cargo test -p davinci-agent approval::tests --lib --offline --locked` | Passed: 10 tests. |
| `rtk cargo test -p davinci-agent batch::tests --lib --offline --locked` | Passed: 4 tests. |
| `rtk cargo test -p davinci-agent f05_ --lib --offline --locked` | Passed: 44 tests. |
| `rtk cargo test -p davinci-agent --lib f01_cancelled_mixed_plan_batch_finishes_every_call --offline --locked` | Passed: 1 test. |
| `rtk cargo test -p davinci-agent --lib f03_batch_observes_runtime_and_ui_cancellation --offline --locked` | Passed: 1 test. |
| `rtk cargo test -p davinci-agent --lib a_batch_runs_its_operations_behind_one_result --offline --locked` | Passed: 1 test. |
| `rtk cargo fmt -p davinci-agent -- --check` | Passed, exit 0. |
| `git diff --check` | Passed, exit 0. |

No full crate/workspace suite, CI run, or non-Windows validation was performed.
RTK reported no automatic hook installed, so no token-savings claim is made.

## Ignored and platform-specific cases

The four ignored tests reported by the workspace run were:

- `immutable_reuse_restart_and_large_workspace_edit` — explicit cache acceptance benchmark.
- `context_image_cold_and_warm_performance` — paired context latency/memory measurement.
- `process_manager_existing_jobbook_baseline` — explicit local Node process lifecycle evaluation.
- `transaction_before_measurement` — explicit pre-P4 transaction behavior measurement.

The inspected process, job-supervisor, worker-session, transaction, and private
directory code contains Windows/Unix branches; socket ownership also has
Linux/macOS/Windows implementations, and transaction metadata/ACL handling
has platform-specific implementations. This audit ran on Windows only, so it
does not validate those other target implementations. Browser tests that need
a configured trusted Node/Playwright installation and graph worker tests that
need `PI_GRAPH_WORKER_EXECUTABLE` require their explicit local fixtures and
were not used as evidence for this baseline.

## Existing regressions to preserve

The following are existing test areas discovered at source entry points and
must be selected again when their owning adapters change:

- `ToolCallLedger`: duplicate/collision reservations, persisted `StartedUnknown`,
  fail-closed persistence and interrupted-call recovery (`tool_ledger.rs`).
- Tool dispatch: contract/capability/permission rechecks, cancellation, mutation
  transaction and batch child behavior (`turn.rs`, `batch.rs`, `tools.rs`).
- Task ownership/transport: parent-bound worker coordinator, framed request
  bounds, command receipts, session rehydration, and corrupt journal refusal
  (`runtime/task_transport.rs`, `runtime/task_store.rs`, `runtime/session.rs`).
- Transactions/processes: source conflict, apply/rollback recovery, file identity,
  process birth/listener ownership, and termination status (`runtime/transactions/`,
  `process_manager/`, `jobs/`).
- Graph: worker binding/lease, effect report, artifact validation, retry
  classification, and successful sibling preservation (`graph/worker.rs`,
  `graph/worker_sessions.rs`, `graph/controller_attempts.rs`, `graph/recovery.rs`).
- Browser/MCP/extensions/hooks: browser lifetime and receipt provenance, MCP
  connection and response-loss fixtures, native/custom dispatch, and hook
  denial/error behavior (`browser.rs`, `mcp.rs`, `extension_host.rs`, `hooks.rs`).
- Provider/cache identity: keep operation IDs and timestamps out of stable
  provider prompt/cache identity; retain the existing cache and wire fixtures
  when result publication is migrated.

These are source-discovered selection areas, not a claim that every listed
regression was run during Task 00. The workspace baseline and one focused
stream rerun are the tests actually run here.

## CI check policy

The configured workflow/job IDs are listed in
[execution-authority-audit.md](execution-authority-audit.md). GitHub's
read-only required-status-check endpoint for repository `main` returned 404
(`Branch not protected`), and its repository ruleset listing was empty. Thus
there were no repository-configured required status check names to record at
the time of this audit. This records policy discovery; it is not a CI run.
