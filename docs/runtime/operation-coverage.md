# Operation entry-point coverage

Task 00 baseline inventory for the unified execution and recovery plan.

`legacy` means the entry point still relies on its existing, subsystem-local
ledger/store/recovery behavior. `audit_required` means an alternate or opaque
boundary needs explicit tracing before it can be migrated. Neither label means
the operation journal covers the path; it does not exist yet.

| Entry point / caller | Existing gate and adapter | Existing store, result publication, and recovery owner | Status and opaque boundary |
|---|---|---|---|
| Provider direct tool call; `Agent::execute_tool_batch` in `turn.rs` | `prepare_tool_call` applies runtime decision hooks, host pre-hook, tool allowlist, task contract, capability and permission checks; `run_prepared_call` consumes approval and invokes `execute_tool_with`. | `ToolCallLedger` persists start/outcome and text result; conversation session stores messages/results separately. Ledger restore classifies interrupted calls. | `legacy`; tool ledger does not persist full structured `ToolResult` details. |
| `batch` and each child tool | `BatchExecutor::run_batch` parses and sequences children through the same `prepare_tool_call` / `run_prepared_call`; existing lane and cancellation rules apply. | Child ledger records plus a parent batch result projected into the conversation. Recovery is per ledger call; parent/child publication is not one transaction. | `legacy`; preserve child order and parent-child idempotency mapping during migration. |
| Built-in file and notebook mutations: `write`, `edit`, `notebook_edit`, `apply_patch` | Tool dispatch, permission/capability checks, path boundary checks, per-file mutation queue, `ToolTransaction` preflight and owner authority. | Transaction record/preimages and active-transaction marker in `runtime/transactions`; factual response goes to tool ledger/session. `TransactionCoordinator` owns recovery/rollback. | `legacy`; operation intent and transaction phase are separate stores. |
| Explicit patch transactions: `patch_preview`, `patch_apply`, `patch_status`, `patch_rollback` | `runtime/transactions/api.rs` plus `ToolTransaction`; current permissions and source checks at mutation. `patch_status` may observe a Git commit. | Transaction store and verification/commit observations; `ToolCallLedger` and session retain separate tool outcomes. | `legacy`; `observe_commit` is an evidence probe, not a Git mutation. |
| Read-only file/search tools: `read`, `ls`, `grep`, `find`, code intelligence | Tool allowlist, path/secret filtering, and current runtime capabilities; `rg`/`fd` may be launched for managed search. | Tool ledger and session result only; no operation-specific journal. | `legacy`; process launches for helper binaries and filesystem reads are not unified. |
| Foreground command tools: `bash`, `powershell`, `exec_command` | Tool contract, capability, permission, approved command context, then shell/process adapter in `tools.rs`. | Foreground child status/output is returned through `ToolResult`, ledger, and session. No durable child owner/result record spans the external command. | `legacy`; arbitrary shell descendants may perform unobserved filesystem, Git, process, or network effects. |
| Long-running process controls: background shell, `write_stdin`, `job_output`, `job_kill` | Shell tool gate plus `jobs::JobBook`; managed subprocesses can use supervisor/`ProcessManager` leases and current process permission. | JobBook/child handles and bounded output are process-local; supervisor/process manager owns live-process observations. Tool/session rows store returned text separately. | `legacy`; process exit and kill requests must remain distinct facts, and JobBook state is not durable across host restart. |
| Git and worktree effects | `runtime/worktree::WorktreeManager` uses Git subprocesses for worktree lifecycle; graph controller also invokes Git for repository snapshots. Direct Git commands can also be run by shell tools under shell permission policy. | Worktree leases are in-memory; graph state/attempt store records graph-owned outcomes. Git mutations through arbitrary shell have no domain receipt. `transactions::commit::observe_commit` is read-only. | `audit_required`; direct Git via shell is opaque and Git mutation must not be inferred from commit observation. |
| Verification commands and evidence | Transaction verification/source observations and graph verification nodes; command execution uses child-process adapters. | Verification receipts, graph artifacts/attempt records, or shell tool result depending on the caller. | `audit_required`; a verification failure does not establish that the preceding mutation had no effect. |
| Task creation, claim, status, cancellation, and worker controls | Runtime task APIs preserve task generation/revision and `TaskOperationReceipt`; child task calls cross authenticated, parent-bound `TaskCoordinatorTransport`. `runtime/control.rs` handles cancellation. | Checksummed `TaskJournal`/session runtime log and in-memory registry; receipts are replayed by task recovery. Tool result is separately published to the session. | `legacy`; preserve the parent-bound coordinator and domain receipts while linking operation IDs. |
| Workflow save/run/resume/cancel | Workflow validation and capability checks; `WorkflowExecutor` invokes worker/subagent runner and supports background execution. | Execution/spec maps are in memory; `WorkflowStateStore` holds artifacts, while child tasks may use the task journal. Resume relies on saved artifacts and validated mutation fingerprints. | `legacy`; workflow lifecycle, child execution, and tool result are not one durable operation. |
| Graph run/submit, worker launch, and verification worker | Native graph host/controller validates graph policy, tool scope, budgets, bindings, and worker-session lease. Worker/verification process starts through `run_worker` and graph process supervision. | Atomic graph state files, per-task attempt records, artifact files, and worker effect-report files under graph run storage; graph controller owns retry classification and orchestration. | `legacy`; external process effects and report-to-operation identity are incomplete; retry must wait on operation recovery. |
| Graph/task control: pause, resume, stop, retry | Native graph commands and control reducer check run revision, action, and node/attempt identity; task controls keep their own generations. | Graph state and attempt history persist; `ControlTracker` receipt cache is in memory and graph state is saved separately. | `legacy`; duplicate control receipt durability and cross-linkage need migration. |
| Subagent and worker-session launch | Direct `agent` tool/subagent runner, workflow runner, and graph worker host apply existing child scope, budgets, and session binding. | Conversation/session storage, task journal/coordinator, worker binding and lease, or graph attempt store depending on launch path. | `audit_required`; enumerate direct library, CLI, JSON/RPC, server/client, and native-host entry points before closing bypasses. |
| Browser tools: `browser_open`, `browser_snapshot`, `browser_click`, `browser_type`, `browser_select`, `browser_console`, `browser_network`, `browser_accessibility`, `browser_screenshot`, `browser_close` | `BrowserController` checks configured backend, `ProcessManager` dev-server lease, process birth/listener ownership, permissions, browser/page identity, and response provenance. Graph workers receive a parent-owned scoped browser handler. | Browser process/session/verification artifacts and tool/session results; no operation journal. Remote page effects are not transactional with the local stores. | `legacy`; navigation, form submission, and remote state changes can be opaque after dispatch. |
| MCP tools and remote calls | `McpRegistry` advertises connected tool schemas/capabilities and dispatches namespaced `mcp__...` calls; transports are MCP stdio/HTTP. | MCP client/server process state plus tool/session result; no durable host operation result or endpoint receipt in the execution ledger. | `audit_required`; remote effect and response-loss boundary is opaque unless the endpoint has a verified idempotency/receipt contract. |
| Built-in web fetch/search | URL/DNS/private-address guards and HTTP client in `web.rs`; calls currently use HTTP GET. | Tool/session result and optional trace only; no operation-specific durable record. | `audit_required`; remote services can have side effects despite a GET method. |
| Native extensions, JS/manifest tools, and host callbacks | `ExtensionHost` dispatches native, JS, and manifest tools; runtime capability and tool gates vary by registered implementation. Native tools include `workspace_restore`, graph controls, and security-scan lifecycle tools. | Feature-local workspace, graph, security, or memory stores; final tool/session result. No common execution authority. | `audit_required`; unknown/custom tools must be treated as potentially mutating; callback behavior may cross process/network boundaries. |
| Pre/post/stop hooks | Host event policy and `HooksFile`; pre-tool hook can block, post-tool hook runs after the tool, hook program executes through supervised child process. | Hook configuration/traces and tool/session result are separate. Post-hook failure cannot prove the tool effect failed. | `audit_required`; configured hook commands are arbitrary external programs. |
| Alternate CLI, terminal, client/server, JSON/RPC entry points | Main runtime host and individual adapters resolve commands/tools; route varies by host and session mode. | Session/task/graph stores depend on host path. | `audit_required`; verify every host dispatch crosses the same coordinator before claiming universal coverage. |

## Coverage rule for later tasks

For every migrated row, record the exact adapter and resource boundary that
consumes its permit, the durable intent/result/evidence IDs, its current
recovery owner, and the regression test proving that a second entry point
cannot bypass admission. Shell, custom extensions, hooks, browsers, and remote
servers remain explicitly opaque outside the harness-controlled invocation.

## Task 20 migration status

The operation journal is now schema version 4. Legacy stores are imported as
append-only `legacy_observations`, keyed by source digest and record identity.
An observation retains the original source identity and evidence paths; it does
not invent an owner, start time, effect latch, completion result, or retry
authority. Repeating the same import is idempotent, while a changed record
under the same source key is rejected without rewriting the prior observation.

The migrated entry points have these boundaries:

| Legacy source | Migration projection | Recovery authority after migration |
|---|---|---|
| `ToolCallLedger` | One source-bound observation per ordered record, read before compatibility reconciliation can rewrite the file. | The operation journal when configured; an existing legacy row blocks authoritative-to-legacy fallback until reconciliation. |
| Session/task runtime log | State and result digests become observations before the durable task journal projects orphaned `running` tasks to failure. | Durable task receipts and operation reconciliation; a legacy `running` value is not a live owner or retry permit. |
| `.pi` graph run roots | Legacy runs remain discoverable beside `.davinci`; duplicate run IDs with different checkpoint bytes fail closed. | Graph continuation and operation recovery; the legacy retry projection is observation-only. |
| Legacy transaction records | Source kind is reserved in the observation schema; no missing effect or owner facts are synthesized. | Transaction and operation adapters must provide authoritative phase/effect evidence before retry. |

The existing `--no-session` worker path remains ephemeral and is covered by
its current worker tests. No durable sessionless transcript or durable
memory-only embedding claim was enabled by this migration; embedding callers
must report that crash recovery is unavailable when they do not provide a
durable session root. Downgrade support is not claimed: the current reader
rejects conflicting modern and legacy roots, and no assertion is made that an
older binary understands the new markers.
