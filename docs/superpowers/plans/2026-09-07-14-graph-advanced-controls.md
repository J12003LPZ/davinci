# /graph Diff, Fork, Rewind, Explain, Dry-run, Verify, Budget and Export Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Provide eight safe engineering controls around the native graph with immutable history, current-source proof, and cumulative resource accounting.

**Architecture:** Build typed operations on the saved-definition/compiler, immutable graph history, runtime checkpoint service, evidence store, and root budget ledger. Read-only operations inspect frozen snapshots; mutating operations require revisioned host authorization and safe execution boundaries.

**Tech Stack:** Rust 1.83.0; the existing Davinci runtime, serde/serde_json, SHA-256, Ratatui/Crossterm, and fixture-driven inline Rust tests. Additional backend decisions are stated explicitly below.

**Spec:** [/graph Diff, Fork, Rewind, Explain, Dry-run, Verify, Budget and Export — supplied source specification](2026-09-07-davinci-feature-specs/14-graph-advanced-controls.md)

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

GraphMutation::diff provides attributed review data, but graph baselines are not durable rewind history. Graph store state.json is a mutable projection and current retention can prune terminal runs. ReplayFingerprint currently hashes HEAD+porcelain status (or shallow non-Git names/lengths), so same-status dirty content changes can escape it. Existing --dry-run runs a simulated pipeline and persists artifacts; it is not the new zero-worker preflight. VerifyExec is injectable but its dry-run adapter can return exit zero, so simulation must remain disqualified as real proof. Existing resume accounting has a static double-charge risk when persisted counters and replayed usage are both added; exact nonzero tests are needed, not an assumption of reproduced failure.

## Scope, alternatives, and chosen approach

Choose one GraphCommand enum shared with12, and one GraphOperation service. Avoid separate command implementations with divergent policy. Diff/explain/preflight/export serialization do not use a model. Fork is a new lineage branch with a bounded strategy enum, not arbitrary orchestration code. Rewind delegates file restoration to04, not git reset. Verify-only cannot mutate through a writer or reuse stale proof. Budget edits act on09's root ledger and never erase spend. Keep the legacy --dry-run simulation explicitly labeled and separately routed for compatibility; `/graph dry-run [name]` is the no-worker preflight.

**Dependencies and implementation order:** Requires 12 saved definitions/native bindings,13 lifecycle/attempt control,04 rewind,05 scope,06 evidence, and 09 root budgets. Read-only diff/explain can develop after 12 schema freezes; mutating controls wait for all shared safety gates.

## Requirements traceability

| Requirement ID | Required result |
|---|---|
| F14-1 | Diff original/prior/current validated graph/state and separately present owned code changes. |
| F14-2 | Fork from a selected node with a different bounded strategy while preserving compatible earlier evidence. |
| F14-3 | Rewind to the task-owned checkpoint before a writer without overwriting dirty/manual edits. |
| F14-4 | Explain node purpose, dependencies, permissions, artifacts, and exit criteria deterministically. |
| F14-5 | Dry-run validates topology/budgets/scopes/roles/permissions without executing workers. |
| F14-6 | Verify-only runs current-source verification/review without implementation workers or simulated proof. |
| F14-7 | Inspect/adjust only authorized root resource ceilings without resetting cumulative usage. |
| F14-8 | Export the validated declarative definition for review/reuse, excluding private transient content. |
| F14-9 | All operations preserve immutable history, revision checks, exact replay identity, and mode-specific native safety gates. |

## Data and behavioral contracts

Commands and results:

| Command | Contract |
|---|---|
| `/graph diff [revision]` | Compare validated definition/state revisions; show owned code delta separately and label unattributed edits. |
| `/graph fork <node>` | New run ID, parent checkpoint, bounded strategy selection, inherited authorization/remaining root resources. |
| `/graph rewind <node>` | Preview checkpoint before selected writer; restore selected task-owned domains through04, with conflicts preserved. |
| `/graph explain` | Deterministic public purpose, dependencies, permissions, artifacts, blocked reasons, and exit criteria. |
| `/graph dry-run [name]` | Parse/compile/validate topology, bindings, scope, roles, budgets, and authorization requirements with zero workers/provider calls/verification commands. |
| `/graph verify` | Run only required verification/security/review stages on the current source; no writer execution. |
| `/graph budget [typed adjustments]` | Inspect or explicitly authorize root ceiling changes; current consumption/reservations remain. |
| `/graph export [name]` | Export validated redacted definition/config for review; no prompts, secrets, or implied execution consent. |

`GraphHistoryEntry { id, parent_id, run_id, definition_digest, task_checkpoint, attempt_refs, evidence_refs, root_budget_id, state_revision }` is immutable and reference-counted for retention. `OperationPreview { id, command, expected_revision, current_source_manifest, affected_nodes, effects, digest }` binds user confirmation. Fork strategies initially are RepairMinimal, AlternativeLocal, and ReplanWithinContract; every strategy stays in the same contract and bounded native graph language. New effective definitions are revalidated and frozen.

## User experience and mode behavior

All commands stay under /graph. Read-only commands show what is validated, estimated, unknown, stale, or approval-required. Mutating commands preview affected nodes/files/budgets and require a fresh confirmation. Fork strategy is bounded; export is local; no feature introduces a general workflow language.

## File ownership and task sequence

The task file lists are the implementation map. Register new agent modules in `crates/davinci-agent/src/runtime/mod.rs` only for runtime-owned functionality, new tools in the existing tool/capability registry, and new native TUI views in the existing view dispatcher. Do not introduce a second runtime, permission engine, or event loop. Treat all cross-feature edits to `turn.rs`, `runtime_host.rs`, `davinci_interactive.rs`, and TUI model/routing as sequential integration points, not parallel write scopes.

### Task 1: Immutable history, source identity, and retention

**Covers:** F14-2, F14-3, F14-9

**Test placement:** `crates/davinci-coding-agent/src/native_extensions/graph/history.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Create:** `crates/davinci-coding-agent/src/native_extensions/graph/history.rs` — immutable run/attempt/checkpoint lineage
- **Modify:** `crates/davinci-coding-agent/src/native_extensions/graph/store.rs` — durable history and retained references
- **Modify:** `crates/davinci-coding-agent/src/native_extensions/graph/replay.rs` — content-based exact fingerprints

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f14_replay_content_not_status() {
    assert!(!exact_replay("dirty", "dirty", "bytes-a", "bytes-b", true));
    assert!(exact_replay("dirty", "dirty", "bytes-a", "bytes-a", true));
    assert!(!exact_replay("clean", "clean", "bytes-a", "bytes-a", false));
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-coding-agent --offline f14_replay_content_not_status -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn exact_replay(_old_status: &str, _new_status: &str, old_content: &str,
    new_content: &str, other_bindings_match: bool) -> bool {
    old_content == new_content && other_bindings_match
}
```

Use06's complete content manifests and include definition/binding version, task contract, effective policy/model, upstream artifact digests, command profile, and simulation provenance. Persist history before switching active projection; a failed save cannot acknowledge a successful fork/rewind. Keep every prior attempt and nonzero usage receipt immutable. Pin ancestors and blobs referenced by active branches, exports, and pending previews against retention pruning. Missing or unknown fingerprints force reexecution/current verification rather than optimistic reuse. Do not rewrite old weak hashes into strong hashes without rerunning the associated work.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: same HEAD/status different bytes; same-size nested non-Git edit; missing prior fingerprint; corrupt history; parent retention; crash after history before projection; model/policy changed; simulated artifact not replayed as real.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-coding-agent/src/native_extensions/graph/history.rs" "crates/davinci-coding-agent/src/native_extensions/graph/store.rs" "crates/davinci-coding-agent/src/native_extensions/graph/replay.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: graph-advanced-controls task 1"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 2: Read-only diff and explain

**Covers:** F14-1, F14-4, F14-9

**Test placement:** `crates/davinci-coding-agent/src/native_extensions/graph/operations.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Create:** `crates/davinci-coding-agent/src/native_extensions/graph/operations.rs` — typed read-only operation service
- **Modify:** `crates/davinci-coding-agent/src/native_extensions/graph/render.rs` — diff/explain command parsing; bounded public explanations
- **Modify:** `crates/davinci-coding-agent/src/native_extensions/graph/mutation.rs` — owned/unattributed delta distinction

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f14_read_operations_do_not_dispatch() {
    assert!(!operation_needs_workers("diff"));
    assert!(!operation_needs_workers("explain"));
    assert!(!operation_needs_workers("dry-run"));
    assert!(operation_needs_workers("verify"));
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-coding-agent --offline f14_read_operations_do_not_dispatch -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn operation_needs_workers(command: &str) -> bool {
    matches!(command, "verify" | "run" | "fork")
}
```

Compare normalized definition fields separately from mutable run state, with explicit original/prior/current revision labels. Owned code diff uses effect provenance; whole working-tree changes are supplementary and labeled unattributed. Explain derives role, typed inputs/outputs, dependency conditions, allowed effects, permission requirements, verification/security/review gates, and current blocked reason from compiled contracts. It must not ask a model to invent a rationale or expose hidden reasoning. Paginate large diffs and redact private paths/values for exported summaries; UI inspection remains read-authorized.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: no prior revision; definition-only change; unrelated dirty file; binary delta; failed conditional edge; unknown permission effect marked unknown; executor spies remain zero; narrow rendering.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-coding-agent/src/native_extensions/graph/operations.rs" "crates/davinci-coding-agent/src/native_extensions/graph/render.rs" "crates/davinci-coding-agent/src/native_extensions/graph/mutation.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: graph-advanced-controls task 2"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 3: Zero-worker dry-run preflight

**Covers:** F14-5, F14-9

**Test placement:** `crates/davinci-coding-agent/src/native_extensions/graph/preflight.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Create:** `crates/davinci-coding-agent/src/native_extensions/graph/preflight.rs` — pure validation and authorization requirements
- **Modify:** `crates/davinci-coding-agent/src/native_extensions/graph/render.rs` — distinguish new subcommand from legacy simulation flag
- **Modify:** `crates/davinci-coding-agent/src/native_extensions/graph/mod.rs` — no-execution preflight dispatch

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f14_preflight_zero_effects() {
    assert!(preflight_is_read_only(0, 0, 0, 0));
    assert!(!preflight_is_read_only(1, 0, 0, 0));
    assert!(!preflight_is_read_only(0, 0, 1, 0));
    assert!(!preflight_is_read_only(0, 0, 0, 1));
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-coding-agent --offline f14_preflight_zero_effects -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn preflight_is_read_only(worker_spawns: usize, provider_calls: usize,
    verification_runs: usize, state_writes: usize) -> bool {
    worker_spawns == 0 && provider_calls == 0 && verification_runs == 0 && state_writes == 0
}
```

Compile the saved/current definition using read-only injected services. Check DAG/IDs/bindings, writer serialization, scopes, role/tool permissions, current trust, artifact schemas, verification profile availability, root budget validity, and mode-specific completion gates. Report RequiresApproval and Unknown separately from Permitted; do not open an approval modal or grant consent during preflight. Estimates name their basis and unknowns; no provider calls to estimate a graph. The new `/graph dry-run` writes no run state/artifacts and starts no worker/verification process. Preserve legacy `/graph --dry-run <goal>` simulation routing, label it simulated, and never promote its canned exit-zero results to verification evidence.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: spy every executor/store; untrusted named command; unknown shell effect; invalid budget; no current graph; saved name; legacy simulation regression; Simple/Standard/Complex gates; no model calls or sidecar writes.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-coding-agent/src/native_extensions/graph/preflight.rs" "crates/davinci-coding-agent/src/native_extensions/graph/render.rs" "crates/davinci-coding-agent/src/native_extensions/graph/mod.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: graph-advanced-controls task 3"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 4: Fork and rewind through shared task services

**Covers:** F14-2, F14-3, F14-9

**Test placement:** `crates/davinci-coding-agent/src/native_extensions/graph/operations.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Extend prerequisite module:** `crates/davinci-coding-agent/src/native_extensions/graph/operations.rs` — previewed fork/rewind orchestration (introduced by F14 Task 2; do not recreate it)
- **Extend prerequisite module:** `crates/davinci-coding-agent/src/native_extensions/graph/history.rs` — branch and before-writer checkpoint refs (introduced by F14 Task 1; do not recreate it)
- **Modify:** `crates/davinci-coding-agent/src/native_extensions/graph/controller.rs` — bounded strategy and descendant reexecution

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f14_branch_keeps_spend() {
    assert_eq!(branch_remaining(120, 81, 10), Some(29));
    assert_eq!(branch_remaining(120, 121, 0), None);
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-coding-agent --offline f14_branch_keeps_spend -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn branch_remaining(ceiling: u64, spent: u64, outstanding: u64) -> Option<u64> {
    ceiling.checked_sub(spent)?.checked_sub(outstanding)
}
```

Fork requires a selected validated node and immutable checkpoint, creates a new run ID, and retains the parent's root resource ledger. Show a bounded strategy choice through the user decision seam; recompile/revalidate resulting native topology without allowing scripts or scope increases. Reuse compatible earlier artifacts only by exact fingerprints; invalidate descendant evidence including off-topology verification/review. Rewind selects the checkpoint before the writer and delegates preview/conflict/apply to04. Quiesce affected owned workers, require explicit host confirmation bound to current source/revision, and preserve manual conflicts. Neither fork nor rewind refunds resources, silently restores execution consent, or repeats external effects.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: fork prior parent pruned unless pinned; incompatible ancestor evidence; attempted fresh full budget; same-status source change; missing checkpoint; overlapping manual edit; code-only/state-only/combined restore; old worker generation blocked.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-coding-agent/src/native_extensions/graph/operations.rs" "crates/davinci-coding-agent/src/native_extensions/graph/history.rs" "crates/davinci-coding-agent/src/native_extensions/graph/controller.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: graph-advanced-controls task 4"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 5: Current-source verify-only execution

**Covers:** F14-6, F14-9

**Test placement:** `crates/davinci-coding-agent/src/native_extensions/graph/operations.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Extend prerequisite module:** `crates/davinci-coding-agent/src/native_extensions/graph/operations.rs` — verification-only stage selection (introduced by F14 Task 2; do not recreate it)
- **Modify:** `crates/davinci-coding-agent/src/native_extensions/graph/verify.rs` — simulation/real execution identity and root deadlines
- **Modify:** `crates/davinci-coding-agent/src/native_extensions/graph/review_coverage.rs` — current owned-chunk coverage
- **Modify:** `crates/davinci-coding-agent/src/native_extensions/graph/controller.rs` — fresh verification/security/review run without writer

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f14_verify_never_runs_writer() {
    assert!(!verify_stage_allowed("writer"));
    assert!(!verify_stage_allowed("planner"));
    assert!(verify_stage_allowed("verification"));
    assert!(verify_stage_allowed("security"));
    assert!(verify_stage_allowed("reviewer"));
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-coding-agent --offline f14_verify_never_runs_writer -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn verify_stage_allowed(stage: &str) -> bool {
    matches!(stage, "verification" | "security" | "reviewer")
}
```

Capture current source/owned mutation manifest, select required trusted verification profiles, and run only verification plus applicable security/review. A reviewer remains read-only and cannot fix code; failed checks produce evidence/gaps for a later task. Preserve Simple's native review policy while preventing Standard/Complex review bypass and self-approval. All verification commands receive the shorter of command timeout and root remaining deadline. Reject empty/all-skipped/simulated checks as required proof. Capture source again after stages and require unchanged relevant input; persist receipts and update completion projection through06's CAS gate. Use nonzero fake usage receipts to verify resumed totals are not charged twice.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: writer-runner spy zero; simulated exit0 rejected; unavailable mandatory security; changed source during review; omitted review chunk; no commands; failed test; root deadline; unrelated succeeded nodes untouched.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-coding-agent/src/native_extensions/graph/operations.rs" "crates/davinci-coding-agent/src/native_extensions/graph/verify.rs" "crates/davinci-coding-agent/src/native_extensions/graph/review_coverage.rs" "crates/davinci-coding-agent/src/native_extensions/graph/controller.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: graph-advanced-controls task 5"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 6: Authorized budget inspection and adjustment

**Covers:** F14-7, F14-9

**Test placement:** `crates/davinci-coding-agent/src/native_extensions/graph/operations.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Extend prerequisite module:** `crates/davinci-coding-agent/src/native_extensions/graph/operations.rs` — root budget view and adjustment adapter (introduced by F14 Task 2; do not recreate it)
- **Modify:** `crates/davinci-coding-agent/src/native_extensions/graph/render.rs` — typed budget arguments
- **Modify:** `crates/davinci-coding-agent/src/native_extensions/graph/config.rs` — legacy budget-unit conversion
- **Modify:** `crates/davinci-coding-agent/src/native_extensions/graph/controller.rs` — atomic dispatch with revised root ceilings

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f14_budget_floor() {
    assert!(budget_update_allowed(100, 70, 20, true));
    assert!(!budget_update_allowed(80, 70, 20, true));
    assert!(!budget_update_allowed(100, 70, 20, false));
    assert!(!budget_update_allowed(100, u64::MAX, 20, true));
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-coding-agent --offline f14_budget_floor -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn budget_update_allowed(new_ceiling: u64, spent: u64, reserved: u64, user_authorized: bool) -> bool {
    user_authorized && spent.checked_add(reserved).map(|used| new_ceiling >= used).unwrap_or(false)
}
```

Budget inspection is read-only. Adjustments require host-authenticated authorization, expected root revision, explicit units, and finite nonnegative values. Preserve old graph 0-as-unlimited semantics in legacy config conversion without using numeric zero as both exhausted and unlimited in the new typed ledger; represent unlimited explicitly. New ceilings cannot fall below spent+reserved usage or remove mandatory verification/handoff reserve. Raising one resource does not raise others or waive task scope. Serialize changes with dispatch, keep cumulative deadlines across pause/resume/fork, and show unknown cost rather than inventing a conversion rate.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: raise tokens only; lower below reservations; floating NaN/negative input; overflow; zero legacy unlimited; elapsed time preserved; unknown price; stale revision; model request cannot approve increase; original nonzero costs retained once.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-coding-agent/src/native_extensions/graph/operations.rs" "crates/davinci-coding-agent/src/native_extensions/graph/render.rs" "crates/davinci-coding-agent/src/native_extensions/graph/config.rs" "crates/davinci-coding-agent/src/native_extensions/graph/controller.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: graph-advanced-controls task 6"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 7: Redacted declarative export and complete command regression

**Covers:** F14-1, F14-4, F14-5, F14-6, F14-7, F14-8, F14-9

**Test placement:** `crates/davinci-coding-agent/src/native_extensions/graph/export.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Create:** `crates/davinci-coding-agent/src/native_extensions/graph/export.rs` — validated bounded export projection
- **Extend prerequisite module:** `crates/davinci-coding-agent/src/native_extensions/graph/operations.rs` — export operation and snapshot digest (introduced by F14 Task 2; do not recreate it)
- **Modify:** `crates/davinci-coding-agent/src/native_extensions/graph/mod.rs` — shared command dispatch
- **Modify:** `crates/davinci-coding-agent/src/slash.rs` — graph command documentation/discovery

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f14_export_allowlist() {
    assert!(export_field_allowed("topology"));
    assert!(export_field_allowed("verification_policy"));
    for field in ["chain_of_thought", "worker_prompt", "transcript", "api_key"] { assert!(!export_field_allowed(field)); }
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-coding-agent --offline f14_export_allowlist -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn export_field_allowed(field: &str) -> bool {
    matches!(field, "schema_version" | "name" | "description" | "topology" | "bindings"
        | "roles" | "artifact_contracts" | "budgets" | "verification_policy" | "parameters")
}
```

Build exports from an explicit typed allowlist, not by serializing GraphRun and deleting a few sensitive fields. Revalidate the frozen definition and safe parameters after redaction; mark simulated/incomplete source-run provenance separately without treating export as completed save. Default export is a bounded review document/result; named project output uses12's safe path and atomic store rules with explicit overwrite confirmation. No remote upload/publish is part of export. Test all eight subcommands through one parser/operation dispatcher, preserving bare/goal/flag semantics and internal lifecycle compatibility. Include usage, migration, errors, and a complete serializer-generated example in final user docs.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: secret in description/parameter; prompt field denied; trace/private path excluded; export checksum; name traversal; interrupted write; missing prior revision; all eight commands parse; unsupported command cannot become surprise worker goal when reserved.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-coding-agent/src/native_extensions/graph/export.rs" "crates/davinci-coding-agent/src/native_extensions/graph/operations.rs" "crates/davinci-coding-agent/src/native_extensions/graph/mod.rs" "crates/davinci-coding-agent/src/slash.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: graph-advanced-controls task 7"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

## Failure, security, and recovery matrix

| Failure | Result |
|---|---|
| Weak/missing replay hash | Reexecute or require current evidence. |
| Rewind checkpoint missing/conflicted | Refuse file restore; keep history and bytes. |
| Budget adjustment unauthorized/too low | Conflict/rejection, no changed ceiling. |
| Simulated verification or source drift | No current completion proof. |
| Export private/unknown data | Omit/reject explicitly; never wholesale dump GraphRun. |
| History persistence error | No success acknowledgment; retain prior active projection. |

## Persistence, migration, and rollback

Introduce immutable history alongside state-v1 projections. Old snapshots remain inspectable but cannot be safely rewound without owned checkpoints, and old weak replay fingerprints require reexecution. Preserve legacy simulated --dry-run routing and label it distinctly. Retain referenced parents across GC. Rollback keeps history, proof, and budgets authoritative and refuses unsupported mutating commands.

## End-to-end acceptance and release gates

Run all eight commands against an injected native graph fixture with immutable history, dirty files, nonzero usage, and current verification. Diff/explain/preflight/export must start no workers or provider calls; preflight also writes no state. Fork/rewind preserve prior history and total resources; overlap produces no overwrite. Verify runs no writer and rejects simulated/stale proof. Budget adjustments respect authorization and reserved resources. Export contains only the validated declarative configuration.

- [ ] Run the feature's listed inline tests with the offline fixture environment and prove each filter selects tests. Record test names, counts, exit codes, and source fingerprint.
- [ ] Run `rtk cargo fmt --check`, relevant crate tests, and then `rtk cargo test --workspace --offline` and `rtk cargo clippy --workspace --all-targets --offline -- -D warnings`. A pre-existing failure is reported, not silently waived or attributed to this feature.
- [ ] Test interactive Ratatui, legacy TUI, print/JSON, and RPC adapters where this plan adds interactions. Noninteractive uncertainty never becomes approval.
- [ ] Verify old session/config fixtures still load, source changes invalidate affected evidence, and cancellation leaves an explainable recoverable state.
- [ ] Update the user-facing docs for this feature and the relevant help/command discovery entries in the final integration task. Keep internal lifecycle operations internal.
- [ ] For later executable delivery, follow `CLAUDE.md`: resolve the actual launched executable, build offline, back up before replacing, compare build/installed hashes, and distinguish automated UI evidence from a physical keyboard check. Do not interrupt an active user's session to unlock a binary.

**Execution exit condition:** all source-spec requirements above have a passing test or a named, explicit manual verification gap; no missing required proof is described as complete. The roadmap's final integration gate applies even when this feature's own tests pass.

## References

Local code references above are anchored to repository HEAD `9c42820`. The supplied spec is preserved as Markdown in this bundle. [Repository audit](2026-09-07-00-repository-audit.md), [shared contracts](2026-09-07-00-shared-runtime-contracts.md), and [cross-feature validation matrix](2026-09-07-00-requirements-and-validation.md) explain the verified baseline and proposed additions.


