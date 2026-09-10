# Execution Tasks / Live Task Board Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make work ownership, dependencies, progress, and evidence durable and inspectable without conflating execution tasks with the Living Plan.

**Architecture:** Extend the active TaskRegistry and runtime task tools. Introduce a serialized task command/commit service with compare-and-swap ownership and a durable authoritative journal; derive UI and observer events from committed records.

**Tech Stack:** Rust 1.83.0; the existing Davinci runtime, serde/serde_json, SHA-256, Ratatui/Crossterm, and fixture-driven inline Rust tests. Additional backend decisions are stated explicitly below.

**Spec:** [Execution Tasks / Live Task Board — supplied source specification](2026-09-07-davinci-feature-specs/03-execution-tasks-live-task-board.md)

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

`runtime/tasks.rs` already provides TaskRecord, dependency validation, and Pending/Ready/Running/Completed/Failed/Blocked/Cancelled states. `RuntimeHandle` shares the registry and rehydrates runtime events. Reconnaissance found unsafe extension points: assign_task overwrites an assignee without ownership CAS; completion can validate then commit against changed state; task_update does not derive caller authorization at its boundary. The runtime observer log is not authoritative persistence because append failures are ignored. Restore also needs run-lineage handling because task_list filters by current run while restored tasks may retain the old run ID.

## Scope, alternatives, and chosen approach

Use the existing registry as the read model, with one task commit coordinator as the writer. Merely adding fields to TodoList would still lose ownership and replay semantics. A new independent database/registry would duplicate working runtime infrastructure. Serialize mutating commands through a per-run coordinator, build the candidate state, persist the commit, publish it, then emit observations. Do not hold the registry lock while invoking completion hooks or UI. Introduce revision and owner-generation checks to detect races after external validation.

**Dependencies and implementation order:** Foundational for features 04–09. Implement ownership and durable command service first; UI can develop against fixtures in parallel with disjoint files. Completion adapter requires 06 before claiming proof-backed done.

## Requirements traceability

| Requirement ID | Required result |
|---|---|
| F03-1 | Persist id/title/status/owner/dependencies/parent plan step/evidence references/timestamps separately from the Living Plan. |
| F03-2 | Support pending/in_progress/completed/blocked/failed/cancelled in the UI without destroying internal readiness semantics. |
| F03-3 | Expose task_create/task_update/task_list/task_get and live board snapshots. |
| F03-4 | Only the owning worker or authorized controller can mutate; claims and handoffs are atomic and revision checked. |
| F03-5 | Resume dependencies, ownership history, and evidence references safely; completion must not be inferred from plan acceptance. |

## Data and behavioral contracts

Extend TaskRecord additively: `revision: u64`, `owner_generation: u64`, `parent_plan_step: Option<PlanStepRef>`, `evidence_refs: Vec<EvidenceId>`, `blocked_reasons: Vec<BlockReason>`, `attempt: u32`, and persisted lineage. Reuse TaskId/RunId/AgentId; do not replace UUIDs with unrelated string IDs. Existing `assigned_to` is the authoritative owner field; the UI calls it owner.

Public states map Pending and Ready to `pending` with a separate readiness reason; Running maps to `in_progress`; other terminal/blocked names stay exact. Expose `task_get` in addition to existing create/update/list. Tool input includes expected_revision and operation_id but no caller identity. Authenticated worker identity comes from RuntimeHandle. Controller authority is a scoped host capability, not agent name text. Claim is CAS from no owner; handoff requires explicit controller/owner authorization and increments owner_generation. Task completion is gated by feature 06; before that gate lands, show execution-finished as unverified, not proof-backed complete. Dependencies must belong to the same authorized lineage and form an acyclic graph.

## User experience and mode behavior

The board separates Plan intent from Execution progress. A pending task includes readiness/dependency detail. Completed tasks show evidence status, not just a checkmark. Worker identities are display labels backed by runtime IDs; the display label never grants ownership.

## File ownership and task sequence

The task file lists are the implementation map. Register new agent modules in `crates/davinci-agent/src/runtime/mod.rs` only for runtime-owned functionality, new tools in the existing tool/capability registry, and new native TUI views in the existing view dispatcher. Do not introduce a second runtime, permission engine, or event loop. Treat all cross-feature edits to `turn.rs`, `runtime_host.rs`, `davinci_interactive.rs`, and TUI model/routing as sequential integration points, not parallel write scopes.

### Task 1: Add task metadata and stable presentation

**Covers:** F03-1, F03-2

**Test placement:** `crates/davinci-agent/src/runtime/tasks.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Modify:** `crates/davinci-agent/src/runtime/tasks.rs` — additive metadata and state mapping
- **Modify:** `crates/davinci-agent/src/runtime/events.rs` — versioned task commit observations

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f03_public_status() {
    assert_eq!(public_task_status("ready"), "pending");
    assert_eq!(public_task_status("running"), "in_progress");
    assert_eq!(public_task_status("completed"), "completed");
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-agent --offline f03_public_status -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn public_task_status(internal: &str) -> &str {
    match internal { "ready" => "pending", "running" => "in_progress", other => other }
}
```

Use typed TaskState matching in production; this helper locks the external vocabulary. Add serde-default fields and explicit schema version to the commit envelope, not a new session wire format. PlanStepRef includes plan identity/revision/step ID so a renamed/revised step cannot silently attach to unrelated work. Preserve created_at; update updated_at only on committed mutations using an injected clock. Bound title to 512 bytes, description to 16 KiB, 64 dependencies and 128 evidence refs per task; reject oversized data before a write.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: old TaskRecord fixture; every state mapping; revision overflow; task with no plan; missing plan step displayed unresolved; repeated update does not reset created_at.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-agent/src/runtime/tasks.rs" "crates/davinci-agent/src/runtime/events.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: execution-tasks-live-board task 1"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 2: Owner CAS and explicit handoff

**Covers:** F03-4

**Test placement:** `crates/davinci-agent/src/runtime/tasks.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Modify:** `crates/davinci-agent/src/runtime/tasks.rs` — authorized mutation checks
- **Modify:** `crates/davinci-agent/src/runtime/team.rs` — replace check-then-assign claim path
- **Modify:** `crates/davinci-agent/src/runtime/tools_task.rs` — derive caller from runtime handle

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f03_claim_race() {
    let mut state = (0u64, None);
    assert!(claim_owner(&mut state, 0, 11));
    assert!(!claim_owner(&mut state, 0, 22));
    assert_eq!(state, (1, Some(11)));
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-agent --offline f03_claim_race -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn claim_owner(state: &mut (u64, Option<u64>), expected: u64, actor: u64) -> bool {
    if state.0 != expected || state.1.is_some() { return false; }
    let Some(next) = state.0.checked_add(1) else { return false; };
    *state = (next, Some(actor));
    true
}
```

The real method operates on TaskId/AgentId inside the single task commit transaction; primitive IDs here are a race oracle. Reject unrelated actors even when they know the latest revision. Handoff first quiesces the old owner's mutation lane, then changes owner and generation; all prepared calls recheck generation before dispatch. Retry creates a new attempt without making a terminal task mutable in place. Controller powers must be bound to run lineage and explicit action, not an unguarded Boolean exposed to model tools.

Fix `task_update_tool` as one candidate-state transaction: validate requested assignment, status, dependencies, expected revision, and actor before changing any field. The current reassign-before-status-validation sequence must not leave a partial update after an invalid status. Replace `TeamManager::claim_task` check-then-assign with the same authoritative CAS service.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: barrier-synchronized two-thread claim; stale owner after handoff; forged actor parameter; controller from another run; pending dependencies; duplicate operation ID; terminal task cannot be reassigned silently.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-agent/src/runtime/tasks.rs" "crates/davinci-agent/src/runtime/team.rs" "crates/davinci-agent/src/runtime/tools_task.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: execution-tasks-live-board task 2"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 3: Durable task commits and crash recovery

**Covers:** F03-1, F03-4, F03-5

**Test placement:** `crates/davinci-agent/src/runtime/task_store.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Create:** `crates/davinci-agent/src/runtime/task_store.rs` — bounded authoritative commit journal
- **Modify:** `crates/davinci-agent/src/runtime/tasks.rs` — single commit coordinator and read projection
- **Modify:** `crates/davinci-coding-agent/src/runtime_host.rs` — persist-before-publish wiring

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f03_no_publish_without_commit() {
    assert_eq!(commit_order(false), vec!["validate", "append", "fail"]);
    assert_eq!(commit_order(true), vec!["validate", "append", "sync", "publish", "observe"]);
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-agent --offline f03_no_publish_without_commit -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn commit_order(durable: bool) -> Vec<&'static str> {
    if durable { vec!["validate", "append", "sync", "publish", "observe"] }
    else { vec!["validate", "append", "fail"] }
}
```

Implement this order with an injected journal interface and failure-injection tests, not a helper that merely returns labels. Each record has sequence, operation ID, prior revision, new record, schema version, and checksum. One authorized process holds the journal writer lease; child workers submit to the parent coordinator instead of concurrently appending. After sync succeeds, publication cannot invoke user hooks; observers are secondary and retryable. On crash after commit but before publication, replay reconstructs the new state exactly once. Treat a torn final record as quarantined tail; a checksum failure in the middle or unknown security-critical schema blocks mutations and reports recovery required.

Do not use `RuntimeLogSubscriber` as the authoritative commit store: it currently ignores append failures and `runtime_log::append` flushes without a durability guarantee. Persist a committed transaction record before acknowledging, with explicit platform sync/error behavior and idempotent recovery. Replay must restore cancellations, results, and dependency-ready cascades, not just creations and assignments.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: failure before write/during write/after sync/before publish; duplicate replay; disk full; second writer denied; corrupt middle record; fresh process resumes same task lineage; observer failure cannot roll back committed task.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-agent/src/runtime/task_store.rs" "crates/davinci-agent/src/runtime/tasks.rs" "crates/davinci-coding-agent/src/runtime_host.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: execution-tasks-live-board task 3"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 4: Dependency and completion transactions

**Covers:** F03-3, F03-4, F03-5

**Test placement:** `crates/davinci-agent/src/runtime/tasks.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Modify:** `crates/davinci-agent/src/runtime/tasks.rs` — dependency recomputation and completion recheck
- **Modify:** `crates/davinci-agent/src/runtime/tools_task.rs` — task_get and revisioned operations
- **Modify:** `crates/davinci-agent/src/runtime/capabilities.rs` — task_get schema/classification

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f03_dependency_gate() {
    assert!(dependencies_ready(&["completed", "completed"]));
    assert!(!dependencies_ready(&["completed", "failed"]));
    assert!(!dependencies_ready(&["in_progress"]));
    assert!(dependencies_ready(&[]));
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-agent --offline f03_dependency_gate -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn dependencies_ready(states: &[&str]) -> bool {
    states.iter().all(|state| *state == "completed")
}
```

Validate same-lineage dependencies, self edges, duplicate IDs, and cycles under the commit coordinator. Run any external evidence validation against a captured task/source revision, then recheck owner, revision, dependency states, and source fingerprint before durable completion. Never publish TaskCompleted as the input to a veto hook; use a completion proposal followed by a separate committed event. A failed prerequisite creates a specific blocked reason and a retry does not erase that evidence. Add task_get with bounded record/evidence summaries and pagination for list; no hidden ownership mutation in read tools.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: completion races cancellation/handoff; dependency cycle update; missing dependency; failed dependency retry; task_get denied across lineage; a required decision from feature02 remains Open; stale evidence from feature06.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-agent/src/runtime/tasks.rs" "crates/davinci-agent/src/runtime/tools_task.rs" "crates/davinci-agent/src/runtime/capabilities.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: execution-tasks-live-board task 4"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 5: Live task board, plan linkage, and resume

**Covers:** F03-1, F03-2, F03-3, F03-5

**Test placement:** `crates/davinci-coding-agent/src/davinci_surfaces.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Modify:** `crates/davinci-tui/src/davinci/model.rs` — task board projection
- **Create:** `crates/davinci-tui/src/davinci/views/task_board.rs` — status/dependency/evidence renderer
- **Modify:** `crates/davinci-coding-agent/src/davinci_surfaces.rs` — runtime task snapshot adapter
- **Modify:** `crates/davinci-coding-agent/src/main.rs` — restored lineage wiring; resume lineage selection and visible persistence errors

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f03_resume_lineage() {
    assert!(task_visible("root-a", "root-a", false));
    assert!(!task_visible("root-a", "root-b", false));
    assert!(task_visible("root-a", "root-a", true));
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-coding-agent --offline f03_resume_lineage -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn task_visible(task_root: &str, active_root: &str, _terminal: bool) -> bool {
    task_root == active_root
}
```

Use an authorized persisted lineage ID to select resumed tasks instead of blindly replacing their original run IDs. Retain provenance of prior attempts and mark workers from a previous process as orphaned/not running until reconciled. The task board reads committed registry snapshots; LivingPlan acceptance only creates/link tasks explicitly and does not mark them done. Preserve the lightweight existing todo compatibility projection where needed but do not make it the task authority. Display owner, current activity, dependencies, verification label, timestamp, and blocked reason. Update live rows incrementally without moving selection or composer state.

Reconcile the saved run/session lineage before constructing the visible task query. The existing main startup creates a new RunId before replay while `task_list_tool` filters by current RunId; preserved old tasks can disappear from the view. Surface read/replay corruption instead of ignoring errors. Historical Running→Failed/process_terminated behavior is the migration baseline; any new orphaned presentation is additive and cannot imply a surviving process.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: resume after main runtime gets a new RunId; old running worker orphaned; no tasks; thousands of tasks paginated; terminal narrow; branch-specific tasks not leaked; acceptance of plan does not complete tasks.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-tui/src/davinci/model.rs" "crates/davinci-tui/src/davinci/views/task_board.rs" "crates/davinci-coding-agent/src/davinci_surfaces.rs" "crates/davinci-coding-agent/src/main.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: execution-tasks-live-board task 5"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

## Failure, security, and recovery matrix

| Failure | Result |
|---|---|
| Wrong owner, stale revision/generation | Conflict; no event or mutation committed. |
| Log append or sync error | Read-only recovery state; never a successful update. |
| Process restart | Reconstruct committed tasks; running workers become orphaned until reconciled. |
| Unknown dependency/cycle | Reject entire command, preserving prior task graph. |
| Observer/UI lag | Show snapshot revision and catch up; observer cannot authorize state. |

## Persistence, migration, and rollback

Read old task events into a versioned initial snapshot without inventing evidence or owner consent. Persist original run IDs and an authorized resume-lineage mapping. No wholesale rewriting of legacy JSONL sessions. Downgrade preserves sidecar journal and reports unsupported task-control state instead of allowing unguarded updates. Rollback of UI is safe; rollback of ownership checks is not.

## End-to-end acceptance and release gates

Demonstrate two competing workers claiming the same task: exactly one owns it and the other receives a conflict. Kill the fixture process after durable commit and resume; the board, dependencies, timestamps, and owner history must match the committed journal. Accepting a Living Plan must never turn task rows green. Complete a task only with current required evidence after feature06 is integrated.

- [ ] Run the feature's listed inline tests with the offline fixture environment and prove each filter selects tests. Record test names, counts, exit codes, and source fingerprint.
- [ ] Run `rtk cargo fmt --check`, relevant crate tests, and then `rtk cargo test --workspace --offline` and `rtk cargo clippy --workspace --all-targets --offline -- -D warnings`. A pre-existing failure is reported, not silently waived or attributed to this feature.
- [ ] Test interactive Ratatui, legacy TUI, print/JSON, and RPC adapters where this plan adds interactions. Noninteractive uncertainty never becomes approval.
- [ ] Verify old session/config fixtures still load, source changes invalidate affected evidence, and cancellation leaves an explainable recoverable state.
- [ ] Update the user-facing docs for this feature and the relevant help/command discovery entries in the final integration task. Keep internal lifecycle operations internal.
- [ ] For later executable delivery, follow `CLAUDE.md`: resolve the actual launched executable, build offline, back up before replacing, compare build/installed hashes, and distinguish automated UI evidence from a physical keyboard check. Do not interrupt an active user's session to unlock a binary.

**Execution exit condition:** all source-spec requirements above have a passing test or a named, explicit manual verification gap; no missing required proof is described as complete. The roadmap's final integration gate applies even when this feature's own tests pass.

## References

Local code references above are anchored to repository HEAD `9c42820`. The supplied spec is preserved as Markdown in this bundle. [Repository audit](2026-09-07-00-repository-audit.md), [shared contracts](2026-09-07-00-shared-runtime-contracts.md), and [cross-feature validation matrix](2026-09-07-00-requirements-and-validation.md) explain the verified baseline and proposed additions.


