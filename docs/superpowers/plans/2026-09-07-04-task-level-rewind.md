# Task-level Rewind with Dirty-file Preservation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Restore only a task's reversible changes and selected state while protecting preexisting and subsequent user edits.

**Architecture:** Add an immutable task-effect journal and content-addressed checkpoint store. Build and review an inverse-delta restore plan before using the existing transactional patch machinery; create new state-history branches instead of deleting audit history.

**Tech Stack:** Rust 1.83.0; the existing Davinci runtime, serde/serde_json, SHA-256, Ratatui/Crossterm, and fixture-driven inline Rust tests. Additional backend decisions are stated explicitly below.

**Spec:** [Task-level Rewind with Dirty-file Preservation — supplied source specification](2026-09-07-davinci-feature-specs/04-task-level-rewind.md)

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

`apply_patch.rs` already implements exclusive patch journals and explicit recovery. The reserved `.davinci_patch_journal.json` and legacy `.pi_patch_journal.json` paths must not be repurposed as user-supplied rewind authority. `native_extensions/graph/mutation.rs` captures graph baselines and deltas, but is not a safe task rewind service: a whole-file baseline cannot distinguish later manual edits and its retained text is bounded. `runtime/worktree.rs` supplies worktree leases, not a guarantee that arbitrary changes are task-owned. Reuse these primitives without treating either as a complete undo system.

## Scope, alternatives, and chosen approach

Choose operation-level ownership plus a previewed inverse patch. `git reset --hard`, repository-wide checkout, and blind snapshot replacement are rejected because they overwrite dirty work. Copying every repository file is expensive and still does not establish ownership. Before each task mutation, capture the relevant baseline and record the exact postimage and effect owner; after verified milestones, add named checkpoints. For text, use a three-way inverse merge; ambiguous or overlapping edits are conflicts. Binary replacement is allowed only when the current postimage hash exactly matches the owned version.

**Dependencies and implementation order:** Requires feature 03 owner/revision persistence. Share source manifests with 06 and stop/drain behavior with 07. Feature 14 calls this service for graph rewind; do not implement a separate graph-only reset.

## Requirements traceability

| Requirement ID | Required result |
|---|---|
| F04-1 | Checkpoint before substantial mutations and at verified milestones with task-owned effect provenance. |
| F04-2 | Protect preexisting dirty content and later manual changes; overlapping edits must conflict rather than overwrite. |
| F04-3 | Provide code-only, task/conversation-state-only, or combined restore selections. |
| F04-4 | Separate reversible local effects from irreversible publish/deploy or other external effects. |
| F04-5 | Make rewind previewable, revision checked, crash recoverable, and evidence invalidating without deleting history. |

## Data and behavioral contracts

Proposed modules `runtime/checkpoints.rs`, `runtime/rewind.rs`, and `runtime/effects.rs` define: `CheckpointRef { id, task_id, attempt, journal_sequence, source_manifest, task_revision, conversation_leaf }`; `OwnedFileEffect { operation_id, actor, owner_generation, path, kind, before_blob, after_blob, before_mode, after_mode, task_id }`; `RewindSelection { code: bool, task_state: bool, transcript: bool }`; `RewindPreview { checkpoint, current_manifest, inverse_effects, conflicts, irreversible_effects, digest }`.

Blob hashes cover bytes, not lossy UTF-8. Record relative path identity, file kind, symlink target, and executable bit where applicable. A task-owned edit must originate in the guarded mutation path; unattributed changes are protected. Proposed storage limits: 16 MiB per blob and 256 MiB per task, with no silent truncation. If a required checkpoint cannot be durable within the limit, block the mutation or obtain an explicit user-approved non-rewindable exception; never promise rewind for omitted content. Content storage remains private to the task store; external effects are recorded separately and are never replayed or reversed automatically.

## User experience and mode behavior

A checkpoint picker has independent Code, Task state, and Transcript selectors. Conflicts disable apply and show current/owned/baseline context for review. Every completed rewind says which domains changed and which external effects remain.

## File ownership and task sequence

The task file lists are the implementation map. Register new agent modules in `crates/davinci-agent/src/runtime/mod.rs` only for runtime-owned functionality, new tools in the existing tool/capability registry, and new native TUI views in the existing view dispatcher. Do not introduce a second runtime, permission engine, or event loop. Treat all cross-feature edits to `turn.rs`, `runtime_host.rs`, `davinci_interactive.rs`, and TUI model/routing as sequential integration points, not parallel write scopes.

### Task 1: Task-owned checkpoints and byte-preserving effects

**Covers:** F04-1, F04-2

**Test placement:** `crates/davinci-agent/src/runtime/checkpoints.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Create:** `crates/davinci-agent/src/runtime/checkpoints.rs` — checkpoint metadata and blob store
- **Create:** `crates/davinci-agent/src/runtime/effects.rs` — effect identity and attribution
- **Modify:** `crates/davinci-agent/src/turn.rs` — capture before/after guarded mutations

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f04_checkpoint_required() {
    assert!(may_mutate_with_checkpoint(true, true, false));
    assert!(!may_mutate_with_checkpoint(true, false, false));
    assert!(may_mutate_with_checkpoint(true, false, true));
    assert!(!may_mutate_with_checkpoint(false, true, true));
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-agent --offline f04_checkpoint_required -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn may_mutate_with_checkpoint(authorized: bool, durable_checkpoint: bool,
    explicit_nonrewindable_exception: bool) -> bool {
    authorized && (durable_checkpoint || explicit_nonrewindable_exception)
}
```

Place checkpoint capture after authorization and before the mutation starts, under the task's mutation lease. Record the actual preimage, not Git HEAD, so existing dirty bytes remain protected baseline. Capture newly created files as absent preimages and deleted files as absent postimages. After tool completion, compare actual changed paths against declared effects; unknown shell effects become unattributed and cannot be claimed safely. Flush blob and journal references before reporting a rewindable checkpoint. Use controlled fault injection rather than modifying real user repositories.

Graph baseline/delta attribution includes any manual changes made after the baseline and stores text only up to 512 KiB. Therefore it is not a source of authoritative ownership or lossless rollback bytes. The new task ledger captures each admitted native mutation at its actual boundary, with unknown effects represented explicitly, rather than attributing every later workspace difference to a task.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: CRLF; invalid UTF-8 binary; empty file; rename; chmod; symlink; preexisting untracked file; blob cap; disk full; guard denial creates no checkpoint-backed mutation.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-agent/src/runtime/checkpoints.rs" "crates/davinci-agent/src/runtime/effects.rs" "crates/davinci-agent/src/turn.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: task-level-rewind task 1"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 2: Inverse-delta planning and manual-edit conflicts

**Covers:** F04-1, F04-2

**Test placement:** `crates/davinci-agent/src/runtime/rewind.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Create:** `crates/davinci-agent/src/runtime/rewind.rs` — three-way inverse planning and conflict report
- **Modify:** `crates/davinci-agent/src/apply_patch.rs` — reuse parse/validation primitives without journal bypass

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f04_restore_classification() {
    assert_eq!(restore_kind("before", "after", "after", false), "inverse");
    assert_eq!(restore_kind("before", "after", "before", false), "already_restored");
    assert_eq!(restore_kind("before", "after", "manual", true), "conflict");
    assert_eq!(restore_kind("before", "after", "manual", false), "three_way_preview");
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-agent --offline f04_restore_classification -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn restore_kind(before: &str, after: &str, current: &str, overlap: bool) -> &'static str {
    if current == before { "already_restored" }
    else if current == after { "inverse" }
    else if overlap { "conflict" }
    else { "three_way_preview" }
}
```

Walk owned effects backward from current attempt to the selected checkpoint, coalescing only effects with proven lineage. For each text path, use postimage as merge base, preimage as the desired inverse, and current bytes as the user's version; calculate disjoint changes with the existing similar diff dependency. Preserve current nonoverlapping insertions/deletions. Require unambiguous anchors and file identity; repeated identical spans, conflicting renames, or any overlap produce a conflict. File deletion is safe only when the created postimage remains exact and no user changes/dependents would be removed. Do not turn a failed merge into whole-file restore.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: preexisting dirty line survives; nonoverlapping later manual line survives; overlapping manual line conflicts; repeated identical lines; rename destination occupied; binary changed after task; user already restored; Windows case-only rename.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-agent/src/runtime/rewind.rs" "crates/davinci-agent/src/apply_patch.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: task-level-rewind task 2"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 3: Atomic apply, stale previews, and recovery

**Covers:** F04-2, F04-5

**Test placement:** `crates/davinci-agent/src/runtime/rewind.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Extend prerequisite module:** `crates/davinci-agent/src/runtime/rewind.rs` — preview digest and restore transaction (introduced by F04 Task 2; do not recreate it)
- **Modify:** `crates/davinci-agent/src/apply_patch.rs` — journal-backed multi-path restore adapter

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f04_stale_preview() {
    assert!(can_apply_rewind("p1", "p1", 0, true));
    assert!(!can_apply_rewind("p1", "p2", 0, true));
    assert!(!can_apply_rewind("p1", "p1", 1, true));
    assert!(!can_apply_rewind("p1", "p1", 0, false));
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-agent --offline f04_stale_preview -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn can_apply_rewind(preview_digest: &str, current_digest: &str,
    conflict_count: usize, mutation_lane_quiescent: bool) -> bool {
    preview_digest == current_digest && conflict_count == 0 && mutation_lane_quiescent
}
```

Acquire the task/run mutation barrier and stop/drain selected owned processes before restore. Re-read every target's content/kind and compare the complete preview manifest. Validate all paths and conflicts before writing any file. Use a host-created exclusive restore transaction journal with planned inverse/preimage refs; never trust an arbitrary repository journal. Apply all eligible paths atomically at the logical transaction level, and retain recovery data on failure. Recovery validates every target before rollback or roll-forward, reports unresolved conflicts, and blocks new mutations until reconciled. No automatic force option overwrites manual conflicts.

`apply_patch::recover_incomplete_journal_if_any` is existing patch-crash recovery, not a safe task-rewind API: its `restore_entry` can overwrite the current post-image without a compare check. Add a guarded inverse transaction adapter with expected post-images and explicit conflict results. Do not call legacy recovery blindly to implement rewind. The existing file mutation queue serializes only this process; process-local locking is not a guarantee against external writers.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: edit between preview and confirm; crash after first restored path; journal already exists; malicious traversal path; symlink swapped during restore; parent kill; recovery failure preserves journal and all evidence.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-agent/src/runtime/rewind.rs" "crates/davinci-agent/src/apply_patch.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: task-level-rewind task 3"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 4: Task/transcript branch restoration and evidence invalidation

**Covers:** F04-3, F04-5

**Test placement:** `crates/davinci-agent/src/runtime/rewind.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Extend prerequisite module:** `crates/davinci-agent/src/runtime/rewind.rs` — state-only and combined restore reducer (introduced by F04 Task 2; do not recreate it)
- **Modify:** `crates/davinci-agent/src/runtime/tasks.rs` — new attempt/branch and dependent invalidation
- **Modify:** `crates/davinci-agent/src/planning.rs` — conversation/plan reference restoration

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f04_restore_selection() {
    assert_eq!(restore_domains(true, false, false), vec!["code"]);
    assert_eq!(restore_domains(false, true, true), vec!["tasks", "transcript"]);
    assert_eq!(restore_domains(true, true, true), vec!["code", "tasks", "transcript"]);
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-agent --offline f04_restore_selection -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn restore_domains(code: bool, tasks: bool, transcript: bool) -> Vec<&'static str> {
    let mut out = Vec::new();
    if code { out.push("code"); } if tasks { out.push("tasks"); }
    if transcript { out.push("transcript"); } out
}
```

Reject an empty selection. Task-state rewind appends a new branch/attempt referencing the checkpoint; it does not rewrite terminal state history in place. Transcript rewind changes the active conversation leaf while retaining original entries and runtime audit. Code-only restore marks affected evidence and downstream tasks stale even when task history is not selected. State-only restore warns that the code is unchanged and verification may not match. Combined restore publishes task/branch changes only after the code transaction commits. Rewinding does not refund token/cost usage, reinstate execution consent, or restart workers automatically.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: all seven nonempty selections; code failure prevents combined state publication; active plan approval cleared; ancestor transcript retained; dependent evidence stale; budgets remain cumulative.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-agent/src/runtime/rewind.rs" "crates/davinci-agent/src/runtime/tasks.rs" "crates/davinci-agent/src/planning.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: task-level-rewind task 4"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 5: Irreversible effects and confirmation UI

**Covers:** F04-3, F04-4, F04-5

**Test placement:** `crates/davinci-tui/src/davinci/views/rewind.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Create:** `crates/davinci-tui/src/davinci/views/rewind.rs` — checkpoint selection and conflict preview
- **Modify:** `crates/davinci-coding-agent/src/davinci_interactive.rs` — host-owned rewind confirmation
- **Extend prerequisite module:** `crates/davinci-agent/src/runtime/effects.rs` — external effect receipts (introduced by F04 Task 1; do not recreate it)

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f04_external_effect_not_undo() {
    assert_eq!(effect_rewind_action("local_file", true), "restore");
    assert_eq!(effect_rewind_action("publish", true), "disclose_only");
    assert_eq!(effect_rewind_action("deploy", false), "disclose_only");
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-tui --offline f04_external_effect_not_undo -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn effect_rewind_action(kind: &str, owned: bool) -> &'static str {
    if kind == "local_file" && owned { "restore" } else { "disclose_only" }
}
```

Display checkpoint name/time, selected restore domains, exact task-owned file summary, conflict count, and irreversible effect receipts. A publish/deploy receipt includes an external operation ID and outcome, not credentials. The user may request a separate compensating action, but it must become a new independently authorized task. Confirm the exact preview digest through the host interaction seam; model tools may request a preview but cannot confirm destructive restore. RPC requires explicit confirmation and print emits rewind_confirmation_required. Update rewind documentation and demonstrate that conflict markers are not silently written into user files.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: external publish remains performed after code rewind; cancelled confirm; task with only external effects; secret redaction; narrow terminal; user selection preserves unrelated composer text.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-tui/src/davinci/views/rewind.rs" "crates/davinci-coding-agent/src/davinci_interactive.rs" "crates/davinci-agent/src/runtime/effects.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: task-level-rewind task 5"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

## Failure, security, and recovery matrix

| Situation | Required result |
|---|---|
| Missing/oversized preimage | Mark not rewindable before mutation; never silently truncate. |
| Unknown ownership or manual overlap | Conflict; preserve all current bytes. |
| External action | Disclose immutable receipt; no false undo claim. |
| Stale preview | Reject and regenerate before confirmation. |
| Crash/recovery ambiguity | Block new mutations and retain journal/blobs; no guessed clean state. |

## Persistence, migration, and rollback

Old tasks without effect journals are explicitly not rewindable. Existing graph baselines may seed review information but cannot be converted into proof of safe ownership. Keep checkpoints until every referenced branch and evidence record releases them. GC is explicit and bounded; rollback hides controls and leaves data intact. Never delete patch recovery files to make a failed restore appear complete.

## End-to-end acceptance and release gates

Use a temporary Git fixture with an already-dirty file, let the task change a separate span, then make a later manual edit. Rewind must remove only the task span. Repeat with overlap and require a conflict with zero writes. Crash midway through a multi-file restore and demonstrate explicit recovery. State-only, code-only, and combined paths must have distinct tested results, with original history and cumulative resource usage intact.

- [ ] Run the feature's listed inline tests with the offline fixture environment and prove each filter selects tests. Record test names, counts, exit codes, and source fingerprint.
- [ ] Run `rtk cargo fmt --check`, relevant crate tests, and then `rtk cargo test --workspace --offline` and `rtk cargo clippy --workspace --all-targets --offline -- -D warnings`. A pre-existing failure is reported, not silently waived or attributed to this feature.
- [ ] Test interactive Ratatui, legacy TUI, print/JSON, and RPC adapters where this plan adds interactions. Noninteractive uncertainty never becomes approval.
- [ ] Verify old session/config fixtures still load, source changes invalidate affected evidence, and cancellation leaves an explainable recoverable state.
- [ ] Update the user-facing docs for this feature and the relevant help/command discovery entries in the final integration task. Keep internal lifecycle operations internal.
- [ ] For later executable delivery, follow `CLAUDE.md`: resolve the actual launched executable, build offline, back up before replacing, compare build/installed hashes, and distinguish automated UI evidence from a physical keyboard check. Do not interrupt an active user's session to unlock a binary.

**Execution exit condition:** all source-spec requirements above have a passing test or a named, explicit manual verification gap; no missing required proof is described as complete. The roadmap's final integration gate applies even when this feature's own tests pass.

## References

Local code references above are anchored to repository HEAD `9c42820`. The supplied spec is preserved as Markdown in this bundle. [Repository audit](2026-09-07-00-repository-audit.md), [shared contracts](2026-09-07-00-shared-runtime-contracts.md), and [cross-feature validation matrix](2026-09-07-00-requirements-and-validation.md) explain the verified baseline and proposed additions.


