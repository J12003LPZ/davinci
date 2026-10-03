# Live Task and Agent Control Panel Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Expose every active worker's work, ownership, waits, usage, and intervention state, with steering and real owned-process cancellation.

**Architecture:** Project committed runtime registry/task/mailbox state into a bounded control view. Introduce host-authorized worker-control commands that connect to actual cancellation tokens and process handles, rather than only changing displayed registry state.

**Tech Stack:** Rust 1.83.0; the existing Davinci runtime, serde/serde_json, SHA-256, Ratatui/Crossterm, and fixture-driven inline Rust tests. Additional backend decisions are stated explicitly below.

**Spec:** [Live Task and Agent Control Panel — supplied source specification](2026-09-07-davinci-feature-specs/07-live-task-agent-control-panel.md)

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

The shared runtime already has `registry.rs`, `mailbox.rs`, `cancellation.rs`, `team.rs`, and `tools_agent.rs`. `RuntimeHandle::send_message` and drain_messages provide routing, and TeamManager has an interrupt path. Reconnaissance found that agent_stop_tool currently marks a registry state as Stopping without itself proving the process was stopped. `jobs.rs::JobBook` can terminate owned process groups/trees but needs task/agent provenance in registration. A reliable panel therefore needs control-path integration, not just a new rendering surface.

## Scope, alternatives, and chosen approach

Choose typed control commands plus acknowledgments from the executing worker. Optimistically showing Stopped on button press is rejected. Killing every process with a matching executable name is also rejected because it can terminate unrelated user work. A worker restart is an explicit new attempt; steering is delivery of a bounded user message to a safe boundary, with visible queued/delivered/applied status. Only the controller/owner may transfer a write lease, and a stopped old worker's lease must be invalidated before replacement.

**Dependencies and implementation order:** Requires 03 owner/revision service; uses 04 owned effects and 09 root budgets. Shares control acknowledgments with 13 graph monitoring. Read-only snapshots can land before mutating controls.

## Requirements traceability

| Requirement ID | Required result |
|---|---|
| F07-1 | Display current activity, owned paths, dependencies/waits, elapsed time, tool/usage counts for every active worker. |
| F07-2 | Steer one active worker without restarting the task and show redirect versus queued-follow-up semantics. |
| F07-3 | Stop one worker or its task-owned process tree with real execution acknowledgment. |
| F07-4 | Inspect recent tool calls and owned diffs; retry safely without resetting unrelated work. |
| F07-5 | Enforce write ownership and explicit handoff through the runtime rather than prompts. |

## Data and behavioral contracts

`WorkerControlCommand { id, root_run_id, agent_id, generation, task_id, expected_revision, action }`, where action is Inspect, Steer, Stop, Retry, or Diff. `SteeringReceipt { message_id, state: Queued | Delivered | Applied | Rejected, applies_after_boundary, reason }`. `ProcessLease { lease_id, task_id, agent_id, generation, os_handle_identity, created_at, child_tree }` is host-owned; a PID supplied by a model is not a lease.

Stopping transitions Requested → Stopping → Stopped only after actual cancellation and process exit acknowledgment; FailedToStop/Unknown are visible alternatives. Steer cancels/restarts no task automatically. A redirect applies before the next eligible model/tool boundary; a queued follow-up applies after the current turn. The choice is explicit to the user. Retry rechecks scope, evidence, worktree state, source fingerprint, and remaining root budget, then assigns a new generation. Keep recent tool events bounded to 100 rows per selected worker and support cursor-based artifact retrieval for older output.

## User experience and mode behavior

Enter inspects; s steers; x stops; r retries; d shows owned diff. An action receipt states queued, applied, stopping, stopped, rejected, or unknown. The user chooses redirect-now-at-next-boundary versus follow-up-after-turn instead of guessing how a message will behave.

## File ownership and task sequence

The task file lists are the implementation map. Register new agent modules in `crates/davinci-agent/src/runtime/mod.rs` only for runtime-owned functionality, new tools in the existing tool/capability registry, and new native TUI views in the existing view dispatcher. Do not introduce a second runtime, permission engine, or event loop. Treat all cross-feature edits to `turn.rs`, `runtime_host.rs`, `davinci_interactive.rs`, and TUI model/routing as sequential integration points, not parallel write scopes.

### Task 1: Revisioned control snapshots and command authority

**Covers:** F07-1, F07-4, F07-5

**Test placement:** `crates/davinci-agent/src/runtime/control.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Create:** `crates/davinci-agent/src/runtime/control.rs` — typed commands and bounded snapshots
- **Modify:** `crates/davinci-agent/src/runtime/registry.rs` — generation and control-state projection
- **Modify:** `crates/davinci-agent/src/runtime/events.rs` — control acknowledgments

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f07_stale_control_rejected() {
    assert!(control_current(4, 4, 2, 2, true));
    assert!(!control_current(4, 3, 2, 2, true));
    assert!(!control_current(4, 4, 2, 1, true));
    assert!(!control_current(4, 4, 2, 2, false));
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-agent --offline f07_stale_control_rejected -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn control_current(actual_revision: u64, expected_revision: u64,
    actual_generation: u64, expected_generation: u64, actor_authorized: bool) -> bool {
    actual_revision == expected_revision && actual_generation == expected_generation && actor_authorized
}
```

Build a read-only worker snapshot from registry/task/usage data using one consistent revision. Include last activity time and stale/disconnected markers; do not infer working from a task label. Authenticate controls through host identity or a validated controller capability. Deduplicate operation IDs and bind commands to root lineage and worker generation. UI selection uses stable agent IDs, not row indexes that may move as workers finish.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: worker exits between render and click; stale snapshot; renamed display label; command for another run; duplicate stop/retry; zero workers; missing usage shown unknown.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-agent/src/runtime/control.rs" "crates/davinci-agent/src/runtime/registry.rs" "crates/davinci-agent/src/runtime/events.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: live-task-agent-controls task 1"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 2: Steering mailbox receipts and safe boundaries

**Covers:** F07-2, F07-5

**Test placement:** `crates/davinci-agent/src/runtime/mailbox.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Modify:** `crates/davinci-agent/src/runtime/mailbox.rs` — delivery/application receipts
- **Modify:** `crates/davinci-agent/src/turn.rs` — apply steering at declared boundaries
- **Modify:** `crates/davinci-agent/src/runtime/tools_agent.rs` — scoped message adapter
- **Modify:** `crates/davinci-agent/src/queues.rs` — preserve established steering boundary semantics

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f07_steering_delivery() {
    assert_eq!(steering_state(false, false, false), "queued");
    assert_eq!(steering_state(true, false, false), "delivered");
    assert_eq!(steering_state(true, true, false), "applied");
    assert_eq!(steering_state(true, false, true), "rejected");
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-agent --offline f07_steering_delivery -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn steering_state(delivered: bool, applied: bool, rejected: bool) -> &'static str {
    if rejected { "rejected" } else if delivered && applied { "applied" }
    else if delivered { "delivered" } else { "queued" }
}
```

Maintain message IDs and exactly-once application per worker generation. A sender's success means queued, not applied. The executor acknowledges after incorporating the message at a safe boundary, outside an in-flight transactional mutation. Redirect updates the next continuation; follow-up remains queued until turn completion. Steering text is guidance and cannot change contract, owner, permission mode, or budget. Persist pending messages and reconcile delivery after restart without replaying already-applied instructions. Cancelled workers reject undelivered steering visibly.

Reuse `queues.rs::enqueue_steer`, `drain_steer`, and `turn.rs::inject_queued` for existing safe-boundary delivery. Add an explicit runtime-mailbox consumer/acknowledgment adapter; enqueueing alone is not delivery. The generic TeamManager APIs are not evidence that production hosts already wire this behavior.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: steer during shell mutation; duplicate delivery; worker retry generation changes; queue full; timeout; redirect vs follow-up; malicious instruction asks to ignore contract; UTF-8 message cap.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-agent/src/runtime/mailbox.rs" "crates/davinci-agent/src/turn.rs" "crates/davinci-agent/src/runtime/tools_agent.rs" "crates/davinci-agent/src/queues.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: live-task-agent-controls task 2"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 3: Owned process-tree cancellation

**Covers:** F07-3, F07-5

**Test placement:** `crates/davinci-agent/src/jobs.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Modify:** `crates/davinci-agent/src/jobs.rs` — task/agent process leases
- **Modify:** `crates/davinci-agent/src/runtime/team.rs` — connect interrupt to worker handles
- **Modify:** `crates/davinci-agent/src/runtime/tools_agent.rs` — stop calls real controller
- **Extend prerequisite module:** `crates/davinci-agent/src/runtime/control.rs` — stop acknowledgment reducer (introduced by F07 Task 1; do not recreate it)

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f07_stop_requires_exit() {
    assert_eq!(stop_status(true, false, false), "stopping");
    assert_eq!(stop_status(true, true, false), "stopped");
    assert_eq!(stop_status(true, false, true), "failed_to_stop");
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-agent --offline f07_stop_requires_exit -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn stop_status(requested: bool, exited: bool, failed: bool) -> &'static str {
    if exited { "stopped" } else if failed { "failed_to_stop" }
    else if requested { "stopping" } else { "running" }
}
```

Bind process control to handles/leases captured at launch; verify generation and creation identity before signaling. Use Windows job-object membership where available and the existing tree termination adapter as a carefully scoped fallback; on Unix use the owned process group with PID/creation safeguards. Send cooperative cancellation first, drain output, then bounded escalation according to user/task stop policy. Reconcile descendants and actual exit before releasing the write lease. Failure to stop keeps ownership locked/quarantined; do not start a competing writer. Never kill the user's parent terminal, unrelated dev server, or sibling task.

`TeamManager::interrupt_teammate` currently marks Cancelled immediately after token cancellation; replace the production presentation with requested→draining→observed-exit. The agent JobBook has Windows tree-kill and Unix process-group plumbing, but graph/process.rs currently kills only the direct child on non-Windows. Converge the graph adapter on owned-group cleanup rather than claiming equivalent behavior already exists.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: worker with child and grandchild; PID reuse fixture; already exited; access denied; stubborn child; stop during patch journal transaction; unrelated same-name process unaffected; Windows and Unix fixture adapters.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-agent/src/jobs.rs" "crates/davinci-agent/src/runtime/team.rs" "crates/davinci-agent/src/runtime/tools_agent.rs" "crates/davinci-agent/src/runtime/control.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: live-task-agent-controls task 3"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 4: Retry, diff, and explicit handoff

**Covers:** F07-4, F07-5

**Test placement:** `crates/davinci-agent/src/runtime/control.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Extend prerequisite module:** `crates/davinci-agent/src/runtime/control.rs` — retry/handoff orchestration (introduced by F07 Task 1; do not recreate it)
- **Modify:** `crates/davinci-agent/src/runtime/tasks.rs` — attempt and owner-generation CAS
- **Modify:** `crates/davinci-coding-agent/src/native_extensions/graph/mutation.rs` — read-only owned-diff adapter

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f07_retry_lease() {
    assert!(!retry_allowed(false, true, true, true));
    assert!(!retry_allowed(true, false, true, true));
    assert!(!retry_allowed(true, true, false, true));
    assert!(retry_allowed(true, true, true, true));
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-agent --offline f07_retry_lease -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn retry_allowed(prior_quiescent: bool, ownership_valid: bool,
    budget_available: bool, source_reconciled: bool) -> bool {
    prior_quiescent && ownership_valid && budget_available && source_reconciled
}
```

Use feature03's explicit handoff transaction and feature04's effect journal to show task-owned diffs. A working-tree diff that includes prior dirty files is labeled unattributed; never present it as all worker-owned. Retry preserves completed independent tasks, creates a new attempt/generation, invalidates stale dependent evidence, and charges feature09's same root budget. Require current policy/contract checks and reconcile unfinished patch journals before dispatch. Do not automatically rewind manual edits as part of retry.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: old worker still alive; terminal task retry new attempt; owned diff excludes preexisting user work; task contract changed; root budget exhausted; pending journal; stale tool call from old generation denied.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-agent/src/runtime/control.rs" "crates/davinci-agent/src/runtime/tasks.rs" "crates/davinci-coding-agent/src/native_extensions/graph/mutation.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: live-task-agent-controls task 4"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 5: Interactive panel and noninteractive controls

**Covers:** F07-1, F07-2, F07-3, F07-4

**Test placement:** `crates/davinci-coding-agent/src/davinci_surfaces.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Create:** `crates/davinci-tui/src/davinci/views/agents.rs` — live worker panel
- **Modify:** `crates/davinci-coding-agent/src/davinci_surfaces.rs` — snapshot adapter
- **Modify:** `crates/davinci-coding-agent/src/davinci_interactive.rs` — inspect/steer/stop/retry/diff actions
- **Modify:** `crates/davinci-coding-agent/src/rpc.rs` — scoped control requests

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f07_panel_actions() {
    assert_eq!(worker_action("enter"), Some("inspect"));
    assert_eq!(worker_action("s"), Some("steer"));
    assert_eq!(worker_action("x"), Some("stop"));
    assert_eq!(worker_action("r"), Some("retry"));
    assert_eq!(worker_action("d"), Some("diff"));
    assert_eq!(worker_action("shift_tab"), None);
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-coding-agent --offline f07_panel_actions -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn worker_action(key: &str) -> Option<&'static str> {
    match key { "enter" => Some("inspect"), "s" => Some("steer"), "x" => Some("stop"),
        "r" => Some("retry"), "d" => Some("diff"), _ => None }
}
```

Reuse the existing shared sheet frame and exclusive input routing. Show phase/activity, owned scope, waiting reason, elapsed time, tool/usage counts, last acknowledgment, and remaining root resources. Stop/retry controls preview effects and demand explicit user confirmation when destructive. Keep selection stable through updates and do not inject large tool output into the main model prompt. RPC responses distinguish accepted command from completed action and support polling by operation ID; print-mode snapshots never block for interaction. Update help and retain normal composer/mode keys outside the panel.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: refresh while typing steer; narrow viewport; thousands of events bounded; no selection; delayed stop acknowledgment; diff of binary path; unknown cost; RPC command accepted but not yet stopped.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-tui/src/davinci/views/agents.rs" "crates/davinci-coding-agent/src/davinci_surfaces.rs" "crates/davinci-coding-agent/src/davinci_interactive.rs" "crates/davinci-coding-agent/src/rpc.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: live-task-agent-controls task 5"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

## Failure, security, and recovery matrix

| Failure | Behavior |
|---|---|
| Worker unreachable | Unknown/disconnected, not successful cancellation. |
| Process refuses termination | FailedToStop; keep mutation ownership quarantined. |
| Steering queue full | Backpressure and explicit rejection, not silent drop. |
| Retry with stale scope/source | Block until reconciled and reauthorized. |
| Registry/view lag | Show snapshot revision; never use row index as control identity. |

## Persistence, migration, and rollback

Add process ownership only at launch; existing unmanaged jobs remain inspectable but cannot be mass-killed through task controls. On restart, old process identities are unproven until reconciled. Persist messages/controls additively, and preserve old event histories. Rollback disables controls, not lease protections.

## End-to-end acceptance and release gates

Launch fixture workers with different owned paths and process trees. Steer one and prove its task/run IDs are unchanged, with a queued→applied receipt. Stop one and prove its descendants exit while a sibling remains active. Retry with a new generation, reject an old-generation write, and preserve unrelated completed work. No UI row may say Stopped solely because a stop command was accepted.

- [ ] Run the feature's listed inline tests with the offline fixture environment and prove each filter selects tests. Record test names, counts, exit codes, and source fingerprint.
- [ ] Run `rtk cargo fmt --check`, relevant crate tests, and then `rtk cargo test --workspace --offline` and `rtk cargo clippy --workspace --all-targets --offline -- -D warnings`. A pre-existing failure is reported, not silently waived or attributed to this feature.
- [ ] Test interactive Ratatui, legacy TUI, print/JSON, and RPC adapters where this plan adds interactions. Noninteractive uncertainty never becomes approval.
- [ ] Verify old session/config fixtures still load, source changes invalidate affected evidence, and cancellation leaves an explainable recoverable state.
- [ ] Update the user-facing docs for this feature and the relevant help/command discovery entries in the final integration task. Keep internal lifecycle operations internal.
- [ ] For later executable delivery, follow `CLAUDE.md`: resolve the actual launched executable, build offline, back up before replacing, compare build/installed hashes, and distinguish automated UI evidence from a physical keyboard check. Do not interrupt an active user's session to unlock a binary.

**Execution exit condition:** all source-spec requirements above have a passing test or a named, explicit manual verification gap; no missing required proof is described as complete. The roadmap's final integration gate applies even when this feature's own tests pass.

## References

Local code references above are anchored to repository HEAD `9c42820`. The supplied spec is preserved as Markdown in this bundle. [Repository audit](2026-09-07-00-repository-audit.md), [shared contracts](2026-09-07-00-shared-runtime-contracts.md), and [cross-feature validation matrix](2026-09-07-00-requirements-and-validation.md) explain the verified baseline and proposed additions.

Windows process-tree design should be checked against Microsoft Job Objects documentation; source listed in the shared contract references. No OS sandbox guarantee is inferred from process-tree ownership.
