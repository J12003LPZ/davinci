# /graph Monitoring and Intervention Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make graph execution inspectable and safely controllable by run and node without restarting unrelated successful work.

**Architecture:** Extend existing ActiveRun snapshots and GraphRunSheet with stable node selection and typed controller commands. Add orthogonal lifecycle state, durable attempt history, safe-boundary pause/resume, and scoped stop/retry acknowledgments.

**Tech Stack:** Rust 1.83.0; the existing Davinci runtime, serde/serde_json, SHA-256, Ratatui/Crossterm, and fixture-driven inline Rust tests. Additional backend decisions are stated explicitly below.

**Spec:** [/graph Monitoring and Intervention — supplied source specification](2026-09-07-davinci-feature-specs/13-graph-monitoring-intervention.md)

## Global Constraints

- **Rust 1.83.0**, edition 2021; dependencies use exact `=x.y.z` pins. Prefer existing workspace dependencies. No toolchain, product-version, or provider wire-identity change is part of these features.
- Work only in active `crates/*`; `vendor/davinci` is reference-only and `packages/*` is not the implementation reference. Preserve upstream system, compaction, and branch prompt constants.
- Tests live in inline `#[cfg(test)] mod tests`; only `crates/davinci-parity/fixtures` may contain external fixtures. Tests are offline and fixture-controlled, with isolated temporary session/config roots. No provider calls, package downloads, browser downloads, or actual external actions in tests.
- Preserve Windows and Unix behavior, existing CLI aliases, JSON/print/RPC compatibility, JSONL sessions, legacy `.pi` discovery, and the five modes: Manual, Accept Edits, Plan Mode, Auto Mode, Always Approve.
- Deny rules, project trust, role allowlists, task scope, ownership, and platform restrictions are never relaxed by a UI choice. A prompt or permission mode is not an operating-system sandbox.
- Keep Living Plan, execution Tasks, user Decisions, and execution Evidence separate. **Claude-style dynamic workflows are intentionally excluded.** Existing workflow code is not a replacement for the graph controller.
- Preserve the graph's DAG, one mutation-capable writer at a time, bounded fan-out, mandatory real verification, applicable security, mode-specific review guarantees, and exact replay provenance. Do not turn disjoint scopes into permission for concurrent graph writers.
- Shared contracts and failure semantics in [the runtime contract](2026-09-07-00-shared-runtime-contracts.md) are part of this plan. Proposed bounds below are engineering decisions, not values mandated by the source mockups.
- Existing Simple graphs require verification and applicable security but do not currently require a Reviewer; Standard/Complex review guarantees must remain intact. Strengthening Simple review would be a separate explicit product decision.

---

## Execution notes

This is a **proposed implementation plan**, prepared against `main` at `9c42820`; it does not claim the feature is implemented or that the user has approved an execution design. Read the source spec, repository `AGENTS.md`/`CLAUDE.md`, and shared contract before execution. Re-resolve symbol anchors if HEAD changes. Existing paths below were found during reconnaissance; paths marked **Create** are proposals.

Each numbered task owns an independently reviewable deliverable. Its Rust snippets define a small executable contract/oracle, not the complete integrated feature; implement the surrounding adapters and all listed scenarios as part of that task. New symbols in snippets are proposed, not claimed existing APIs. Integrate `mod` declarations and imports before running the red test; a filter matching zero tests is **not** a pass. Merge snippets into one inline test module per source file. Each task's extra cases are required, not optional stretch goals.

Use an isolated execution worktree later, preserve unrelated user edits, and review each diff before its task commit. The documentation-generation session does not run the repository application test suite or change application code. Any separately checked standalone examples are documentation checks, not evidence that the feature works in Davinci.

## Repository fit and existing behavior

`graph/mod.rs::ActiveRun` already stores abort/snapshot/finished/thread state and excludes concurrent runs per workspace, including while stopping. `graph/controller.rs` checkpoints snapshots and has injected WorkerRunner/VerifyExec; research can run in parallel and automatic node attempts are bounded. GraphRunSheet/views/graph_run.rs already render a read-only graph view and the host refreshes it roughly once per second. There is no explicit Paused phase or selected-node retry API. Keep the existing extension-host behavior that drops the native mutex before running the graph.

## Scope, alternatives, and chosen approach

Use a lifecycle field separate from Phase/TaskStatus to preserve old state compatibility. Pause means no new dispatch and acknowledgment after active work reaches safe boundaries; it is not OS process suspension or rollback of in-flight writes. Stop requests real owned-worker cancellation and keeps workspace exclusion until process exit/reconciliation. Retry targets one eligible failed/cancelled node with a new attempt, invalidating affected descendants and proof while preserving compatible independent successes. Prompt-contract inspection shows public role/input/output/policy contracts, not hidden reasoning.

**Dependencies and implementation order:** Requires 12 stable native bindings,03 owner/attempt identity,07 real process control,09 root budget, and 06 fresh evidence. Read-only view enhancements may land first.14 adds advanced owned diff/fork/rewind affordances.

## Requirements traceability

| Requirement ID | Required result |
|---|---|
| F13-1 | Show phases, worker states, time/usage, outputs, dependencies, and blocked reasons in a dedicated graph view. |
| F13-2 | Pause/resume at safe boundaries and never start new nodes after an acknowledged pause. |
| F13-3 | Stop/retry a selected node without restarting compatible unrelated successes. |
| F13-4 | Inspect public prompt contracts, tool activity, artifacts, verification, and owned diffs. |
| F13-5 | Preserve deterministic ownership, one writer, bounded fan-out, current evidence, and durable lifecycle controls. |

## Data and behavioral contracts

`GraphControl { operation_id, run_id, expected_run_revision, node_id: Option<String>, expected_attempt, action }`; lifecycle is Running, PauseRequested, Paused, StopRequested, Stopped, RecoveryRequired. `GraphControlReceipt { operation_id, accepted_revision, state, affected_nodes, reason }`. Node attempts are immutable records with owner generation, source/definition/input digests, exit outcome, artifacts, usage, and verification references.

Pause admission is serialized with node scheduling/resource reservation. A PauseRequested run can have active workers; Paused requires an empty active frontier and durable checkpoint. If a worker cannot safely settle, show the active blocker instead of falsely acknowledging pause. Node Stop cancels its owned subtree and marks dependents blocked/stale; graph Stop cancels all owned graph workers. Retry requires a current terminal attempt, reconciled writer effects, valid binding, dependency evidence, remaining retry/root budgets, and current authorization. It never makes an old terminal attempt mutable or blindly reuses downstream verification.

## User experience and mode behavior

The existing graph sheet becomes a focused control view with arrows/Enter/p/x/r/d. Control labels show requested versus applied. Node inspection exposes only public execution contracts and evidence, and selecting a node never starts or retries it.

## File ownership and task sequence

The task file lists are the implementation map. Register new agent modules in `crates/davinci-agent/src/runtime/mod.rs` only for runtime-owned functionality, new tools in the existing tool/capability registry, and new native TUI views in the existing view dispatcher. Do not introduce a second runtime, permission engine, or event loop. Treat all cross-feature edits to `turn.rs`, `runtime_host.rs`, `davinci_interactive.rs`, and TUI model/routing as sequential integration points, not parallel write scopes.

### Task 1: Typed lifecycle and control receipts

**Covers:** F13-1, F13-2, F13-5

**Test placement:** `crates/davinci-coding-agent/src/native_extensions/graph/control.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Create:** `crates/davinci-coding-agent/src/native_extensions/graph/control.rs` — versioned commands/lifecycle reducer
- **Modify:** `crates/davinci-coding-agent/src/native_extensions/graph/types.rs` — additive lifecycle/revision fields
- **Modify:** `crates/davinci-coding-agent/src/native_extensions/graph/mod.rs` — ActiveRun control channel

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f13_pause_ack_boundary() {
    assert_eq!(pause_state(true, 2, true), "pause_requested");
    assert_eq!(pause_state(true, 0, false), "recovery_required");
    assert_eq!(pause_state(true, 0, true), "paused");
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-coding-agent --offline f13_pause_ack_boundary -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn pause_state(requested: bool, active_workers: usize, checkpoint_durable: bool) -> &'static str {
    if !requested { "running" } else if active_workers > 0 { "pause_requested" }
    else if checkpoint_durable { "paused" } else { "recovery_required" }
}
```

Use run/node/attempt IDs and expected revisions to reject stale controls; deduplicate operation IDs. Persist lifecycle and receipt before acknowledging a control as applied. Keep Phase available for progress labels and old serialization defaults. A view request cannot trigger resume as a side effect. Bare /graph reopens an explicitly Paused run without auto-resuming it; preserve the old stopped-run continuation behavior only where the stored lifecycle permits. Expose accepted versus applied receipts to RPC/host callers.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: pause while running; duplicate pause; stale run ID; persisted paused resume; old state-v1 defaults; disk failure; bare /graph does not unpause; unknown lifecycle fail-closed.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-coding-agent/src/native_extensions/graph/control.rs" "crates/davinci-coding-agent/src/native_extensions/graph/types.rs" "crates/davinci-coding-agent/src/native_extensions/graph/mod.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: graph-monitoring-intervention task 1"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 2: Safe scheduler boundaries and real stop/drain

**Covers:** F13-2, F13-3, F13-5

**Test placement:** `crates/davinci-coding-agent/src/native_extensions/graph/controller.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Modify:** `crates/davinci-coding-agent/src/native_extensions/graph/controller.rs` — dispatch admission/pause frontier/acknowledgments
- **Modify:** `crates/davinci-coding-agent/src/native_extensions/graph/process.rs` — owned process-tree cancellation adapter
- **Modify:** `crates/davinci-coding-agent/src/native_extensions/graph/worker.rs` — per-node cancellation token/lease

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f13_no_dispatch_when_paused() {
    assert!(graph_dispatch_allowed("running", true, true));
    assert!(!graph_dispatch_allowed("pause_requested", true, true));
    assert!(!graph_dispatch_allowed("paused", true, true));
    assert!(!graph_dispatch_allowed("running", false, true));
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-coding-agent --offline f13_no_dispatch_when_paused -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn graph_dispatch_allowed(lifecycle: &str, dependencies_ready: bool, lease_available: bool) -> bool {
    lifecycle == "running" && dependencies_ready && lease_available
}
```

Check lifecycle, frontier readiness, writer exclusivity, and budget lease in one admission critical section; do not race a pause against worker spawn. Let in-flight transactional work settle or cancel through its recovery path, while continuing to drain output and usage receipts. Integrate07's owned process-tree API: the existing graph Unix child kill alone is not sufficient to claim descendant termination. Preserve ActiveRun workspace exclusion until all children are reaped or explicitly quarantined. Global deadline remains enforced during pause/stop and verification subprocesses receive the minimum of node and root remaining deadlines.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: pause during parallel research; pause races spawn; stop selected child leaves siblings; deadline while paused; Windows grandchildren and Unix group; stubborn process keeps run exclusion; callbacks invoked outside locks.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-coding-agent/src/native_extensions/graph/controller.rs" "crates/davinci-coding-agent/src/native_extensions/graph/process.rs" "crates/davinci-coding-agent/src/native_extensions/graph/worker.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: graph-monitoring-intervention task 2"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 3: Node retry and descendant evidence invalidation

**Covers:** F13-3, F13-5

**Test placement:** `crates/davinci-coding-agent/src/native_extensions/graph/control.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Extend prerequisite module:** `crates/davinci-coding-agent/src/native_extensions/graph/control.rs` — eligible retry reducer (introduced by F13 Task 1; do not recreate it)
- **Modify:** `crates/davinci-coding-agent/src/native_extensions/graph/controller.rs` — new node attempts and bounded frontier restart
- **Modify:** `crates/davinci-coding-agent/src/native_extensions/graph/replay.rs` — attempt/source/definition checks
- **Modify:** `crates/davinci-coding-agent/src/native_extensions/graph/store.rs` — immutable attempt persistence

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f13_invalidate_descendants() {
    let edges = vec![("a", "b"), ("b", "c"), ("x", "y")];
    let affected = descendant_ids("a", &edges);
    assert_eq!(affected, std::collections::BTreeSet::from(["a", "b", "c"]));
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-coding-agent --offline f13_invalidate_descendants -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn descendant_ids<'a>(start: &'a str, edges: &[(&'a str, &'a str)]) -> std::collections::BTreeSet<&'a str> {
    let mut out = std::collections::BTreeSet::from([start]);
    loop {
        let before = out.len();
        for (from, to) in edges { if out.contains(from) { out.insert(*to); } }
        if out.len() == before { return out; }
    }
}
```

Compute invalidation over both dependency edges and proof/input references, including host verification/security/review records not represented as ordinary topology nodes. Preserve unaffected ancestors/independent nodes only when exact source/input/contract fingerprints still match. A writer with unknown partial outcome cannot retry until effects are reconciled through04; never apply another writer on top of an unowned ambiguous delta. Increment attempt and owner generation, keep all spent usage, and count manual retry against bounded retry policy. Validate every generated attempt binding through12's compiled native plan.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: retry failed research preserves independent success; writer unknown outcome refuses; stale review/security invalidated; cancelled ancestor; max retries; same-status source edit; repeated control ID creates one attempt.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-coding-agent/src/native_extensions/graph/control.rs" "crates/davinci-coding-agent/src/native_extensions/graph/controller.rs" "crates/davinci-coding-agent/src/native_extensions/graph/replay.rs" "crates/davinci-coding-agent/src/native_extensions/graph/store.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: graph-monitoring-intervention task 3"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 4: Node focus, public contract inspection, and controls

**Covers:** F13-1, F13-2, F13-3, F13-4

**Test placement:** `crates/davinci-tui/src/davinci/model.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Modify:** `crates/davinci-tui/src/davinci/model.rs` — GraphRunSheet selection and control status
- **Modify:** `crates/davinci-tui/src/davinci/views/graph_run.rs` — node-focused view and key hints
- **Modify:** `crates/davinci-tui/src/davinci/app.rs` — typed graph actions before composer
- **Modify:** `crates/davinci-coding-agent/src/davinci_interactive.rs` — stable polling and control bridge

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f13_graph_keys() {
    assert_eq!(graph_view_action("p"), Some("pause_resume"));
    assert_eq!(graph_view_action("x"), Some("stop"));
    assert_eq!(graph_view_action("r"), Some("retry"));
    assert_eq!(graph_view_action("d"), Some("diff"));
    assert_eq!(graph_view_action("enter"), Some("inspect"));
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-tui --offline f13_graph_keys -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn graph_view_action(key: &str) -> Option<&'static str> {
    match key { "p" => Some("pause_resume"), "x" => Some("stop"), "r" => Some("retry"),
        "d" => Some("diff"), "enter" => Some("inspect"), _ => None }
}
```

Arrows select a stable node ID rather than only scrolling a text buffer; retain selection when snapshots refresh. Show phase completion counts, actual or unknown token/time data, current root budget, waiting dependencies, owner, and control acknowledgment. Enter opens public role/input/output/permission/exit criteria, recent bounded tool activity, artifacts, and current/stale proof. d uses owned diff from04/14, with unsupported controls visibly disabled until integrated. p toggles explicit pause/resume; x previews node versus entire-graph stop; r previews invalidated descendants. Modal input ownership protects the composer and mode keys.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: 1-second refresh while selected node finishes; 40-column screen; long artifacts; no node selected; stale action after retry; keyboard does not edit draft; pause label Requested until ack; hidden reasoning never displayed.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-tui/src/davinci/model.rs" "crates/davinci-tui/src/davinci/views/graph_run.rs" "crates/davinci-tui/src/davinci/app.rs" "crates/davinci-coding-agent/src/davinci_interactive.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: graph-monitoring-intervention task 4"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 5: Persistence, noninteractive parity, and control acceptance

**Covers:** F13-1, F13-2, F13-3, F13-4, F13-5

**Test placement:** `crates/davinci-coding-agent/src/native_extensions/graph/store.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Modify:** `crates/davinci-coding-agent/src/native_extensions/graph/store.rs` — durable checkpoint errors and history references
- **Modify:** `crates/davinci-coding-agent/src/native_extensions/graph/mod.rs` — resume/control lifecycle reconciliation
- **Modify:** `crates/davinci-coding-agent/src/rpc.rs` — typed graph control receipts
- **Modify:** `crates/davinci-coding-agent/src/output.rs` — structured graph status

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f13_restart_never_resurrects_worker() {
    assert_eq!(restored_worker_state("running", false), "reconciliation_required");
    assert_eq!(restored_worker_state("succeeded", false), "succeeded");
    assert_eq!(restored_worker_state("running", true), "running");
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-coding-agent --offline f13_restart_never_resurrects_worker -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn restored_worker_state(recorded: &str, live_identity_verified: bool) -> &str {
    if recorded == "running" && !live_identity_verified { "reconciliation_required" } else { recorded }
}
```

Surface graph checkpoint save errors that are currently ignored; durable controls must not rely on a best-effort snapshot write. Store immutable attempt/lifecycle records and a latest-state projection. On restart reconcile running workers and partial writer effects instead of resurrecting them from status text. Retention pins ancestors/checkpoints referenced by active control/history operations so the existing terminal-run pruning does not remove required evidence. Test through injected blocking WorkerRunner and VerifyExec closures with atomic probes and temporary directories; no assumed magic environment fixture flag. Update graph monitoring and RPC docs with accepted/applied/unknown semantics.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: crash before/after pause commit; stop receipt after restart; corrupt state; terminal retention with referenced ancestor; RPC disconnect; nonzero usage retained exactly; legacy stopped-run continuation regression.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-coding-agent/src/native_extensions/graph/store.rs" "crates/davinci-coding-agent/src/native_extensions/graph/mod.rs" "crates/davinci-coding-agent/src/rpc.rs" "crates/davinci-coding-agent/src/output.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: graph-monitoring-intervention task 5"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

## Failure, security, and recovery matrix

| Failure | Required behavior |
|---|---|
| Pause cannot reach safe boundary | PauseRequested with blocker, not false Paused. |
| Stop cannot reap child tree | RecoveryRequired/FailedToStop; workspace remains exclusive. |
| Retry writer outcome uncertain | Refuse until effect reconciliation. |
| Stale node/attempt command | Conflict; no action on replacement node. |
| Persistence fails | No applied acknowledgment; retain recovery state. |

## Persistence, migration, and rollback

Add lifecycle/revision/attempt metadata with safe defaults for old graph state. Keep phase/status wire values and internal lifecycle commands compatible. Paused is never inferred as ordinary cancelled/stopped auto-resume. Downgrade refuses mutation of unsupported controlled states and preserves history.

## End-to-end acceptance and release gates

Use injected parallel research workers and pause while they are active: no new node may start after pause admission, and Paused appears only after a durable safe checkpoint. Stop one selected node and preserve sibling work. Retry a failed node, invalidate dependent verification/review, and prove independent success and cumulative resources remain. Repeat controls across crash/resume and narrow-view refresh without mis-targeting another node.

- [ ] Run the feature's listed inline tests with the offline fixture environment and prove each filter selects tests. Record test names, counts, exit codes, and source fingerprint.
- [ ] Run `rtk cargo fmt --check`, relevant crate tests, and then `rtk cargo test --workspace --offline` and `rtk cargo clippy --workspace --all-targets --offline -- -D warnings`. A pre-existing failure is reported, not silently waived or attributed to this feature.
- [ ] Test interactive Ratatui, legacy TUI, print/JSON, and RPC adapters where this plan adds interactions. Noninteractive uncertainty never becomes approval.
- [ ] Verify old session/config fixtures still load, source changes invalidate affected evidence, and cancellation leaves an explainable recoverable state.
- [ ] Update the user-facing docs for this feature and the relevant help/command discovery entries in the final integration task. Keep internal lifecycle operations internal.
- [ ] For later executable delivery, follow `CLAUDE.md`: resolve the actual launched executable, build offline, back up before replacing, compare build/installed hashes, and distinguish automated UI evidence from a physical keyboard check. Do not interrupt an active user's session to unlock a binary.

**Execution exit condition:** all source-spec requirements above have a passing test or a named, explicit manual verification gap; no missing required proof is described as complete. The roadmap's final integration gate applies even when this feature's own tests pass.

## References

Local code references above are anchored to repository HEAD `9c42820`. The supplied spec is preserved as Markdown in this bundle. [Repository audit](2026-09-07-00-repository-audit.md), [shared contracts](2026-09-07-00-shared-runtime-contracts.md), and [cross-feature validation matrix](2026-09-07-00-requirements-and-validation.md) explain the verified baseline and proposed additions.
