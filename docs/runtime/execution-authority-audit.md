# Execution authority audit

## Snapshot and scope

This is the Task 00 source audit for the unified execution and recovery plan.
The inspected checkout is `0d3163c30d528bcddaf9a2ee211a4ba052d7f420` on
`leonardojeziellopez/unified-execution-recovery-01a0ca95`, based on refreshed
`origin/main`. At the start of the audit, `git status --short` in this
worktree contained only the requested untracked plan file. The original shared
checkout had unrelated dirty and untracked work; it was left untouched.

`cargo metadata --no-deps --format-version 1` succeeded. It reports product
`davinci-coding-agent` version `1.0.71` and these 14 workspace packages:

`davinci-agent`, `davinci-ai`, `davinci-client`, `davinci-coding-agent`,
`davinci-evals`, `davinci-mcp`, `davinci-parity`, `davinci-protocol`,
`davinci-server`, `davinci-session`, `davinci-session-sqlite`,
`davinci-telemetry`, `davinci-tui`, and `davinci-voice`.

The `davinci` binary is declared by `crates/davinci-coding-agent/Cargo.toml`.
The workspace `Cargo.toml` package list and metadata agree with the plan's
crate list. HEAD is exactly the plan's source snapshot; there is no rebase
delta to reconcile.

## Corrected documentation finding

The initial plan text incorrectly said `AGENTS.md` describes inactive
`davinci-core`, `davinci-app`, and `davinci-cli` crates. The checked-out root
`AGENTS.md` is workflow guidance and contains none of those crate names. It
also does not provide a Rust module map. I verified this against the checked-out
file and Cargo metadata. I have corrected the plan's finding rather than
changing repository instructions or inventing a module map.

The active implementation surfaces are under `crates/davinci-agent` and
`crates/davinci-coding-agent`; the exact dispatch, adapter, persistence, and
recovery owners are indexed in [operation-coverage.md](operation-coverage.md).

## Existing authorities and current gaps

- `Agent::prepare_tool_call` and `Agent::run_prepared_call` in
  `crates/davinci-agent/src/turn.rs` apply the runtime decision hook, host
  pre-hook, tool allowlist, contract/capability checks, permission checks,
  dispatch approval, and then call the tool adapter. The direct and batch
  paths share this preparation/execution pair.
- `ToolCallLedger` in `crates/davinci-agent/src/tool_ledger.rs` stores call
  identity, normalized arguments/digest, effect class, replay policy, an
  attempt outcome, and a text result/error bit. It persists separately from
  conversation rows and does not retain the complete `ToolResult.details`.
- File writes, edits, notebook edits, and patch apply/rollback use
  `runtime/transactions::ToolTransaction` and its workspace/owner-bound
  transaction store. `observe_commit` reads Git state; it does not create a
  Git commit.
- The session layer (`davinci-session::JsonlSession` plus runtime log and
  `davinci-session-sqlite`) owns conversation persistence. The task journal in
  `runtime/task_store.rs` owns task receipts and recovery. These stores are
  separate from the tool ledger and transaction store.
- Foreground shell execution starts child processes from `tools.rs`.
  Background jobs and their child handles are held by `jobs::JobBook`; the
  separate supervisor and `ProcessManager` provide managed process ownership
  and controls. These do not create one cross-subsystem durable operation
  record.
- Workflow execution state is held in `WorkflowExecutor` maps; workflow
  artifacts are managed by `WorkflowStateStore`. Graph run state and attempts
  are persisted by the graph store, with worker bindings/leases, attempt
  records, and effect-report files. Graph failure classification/retry remains
  a separate orchestration decision.
- `RuntimeBus` separates decision hooks from observation subscribers. Tool
  hooks and native/JS extension callbacks also have their own host lifecycle.
  Their events/results are not one atomic commit with the tool ledger,
  transaction store, or session projection.

The planned operation journal is not implemented yet. All rows in the coverage
manifest are therefore `legacy` or `audit_required` at this checkpoint.

## CI configuration and remote check policy

All workflow job IDs found under `.github/workflows` are recorded below. These
are configured workflow job identifiers, not evidence that GitHub requires
them on a pull request:

| Workflow | Job IDs |
|---|---|
| `behavior-evals.yml` | `eval-matrix` |
| `behavior-live.yml` | `eval-matrix`, `coverage` |
| `ci.yml` | `test-impact-native`, `p12-live-browser-native`, `voice-native`, `quality`, `workspace-test-shards`, `workspace-tests` |
| `competitor-differential.yml` | `differential` |
| `graph-recovery.yml` | `graph` |
| `graph-verification.yml` | `regression` |
| `harness-optimization-executor.yml` | `execute` |
| `harness-optimization-learning.yml` | `execute` |
| `harness-optimization-learning-stage2.yml` | `execute` |
| `prompt-promotion.yml` | `certify` |
| `security-sarif.yml` | `schema` |
| `terminal-parity-workbench.yml` | `reference-inputs` |
| `terminal-reference.yml` | `reference` |
| `terminal-ui.yml` | `terminal` |
| `workflow-lint.yml` | `actionlint` |

Read-only GitHub API inspection of repository `J12003LPZ/pi-rust` reported
`main` as not branch-protected (required-status-check endpoint returned 404)
and no repository rulesets. No required CI context names were discoverable in
that repository policy snapshot. This does not establish organization-level
policy outside the repository API response.

## Audit boundary

The inventory opens the current entry points and their primary adapter/store
owners, including task transport and workflow, worktree, tool, jobs, graph
controller/process/worker/binding/session/control, browser, MCP, web, native
extension, and hook modules. It does not claim syscall-level observation or
prove every optional extension callback safe. Arbitrary shell programs,
custom extensions, hooks, remote MCP servers, browser navigation, and remote
HTTP endpoints remain opaque beyond their harness invocation and available
receipts. Platform-specific Windows, Unix, Linux, and macOS implementations
were located but are not all runtime-validated on this Windows host.
