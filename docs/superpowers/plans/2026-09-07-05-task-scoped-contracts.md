# Task-scoped Execution Contracts Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Enforce accepted task scope across all execution paths, independently of permission mode and worker instructions.

**Architecture:** Compile a user-accepted plan scope into an immutable versioned TaskContract. A runtime guard intersects that contract with policy, ownership, trust, tool capabilities, and platform restrictions immediately before execution, including child workers and aliases.

**Tech Stack:** Rust 1.83.0; the existing Davinci runtime, serde/serde_json, SHA-256, Ratatui/Crossterm, and fixture-driven inline Rust tests. Additional backend decisions are stated explicitly below.

**Spec:** [Task-scoped Execution Contracts — supplied source specification](2026-09-07-davinci-feature-specs/05-task-scoped-execution-contracts.md)

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

`permission.rs` already checks deny rules, Plan Mode freeze, file boundaries, and protected paths before broad approval shortcuts. `turn.rs` prepares/runs/finalizes tools; scheduler lanes and batch members preserve individual gates. Graph `worker_hooks.rs` and role allowlists constrain worker capabilities. These boundaries do not yet constitute a specific accepted task contract. `planning.rs` is the genuine-user-only approval/handoff entry point and is the correct origin for a contract approval, not a model tool.

## Scope, alternatives, and chosen approach

Choose a separate contract guard rather than expanding PermissionMode into many task-specific variants. A prompt-only scope is advisory and rejected. A repository worktree isolates files but is not a security sandbox for shell/network side effects. The initial contract executor supports provable native file effects and explicit confined process profiles; unknown arbitrary shell/MCP mutation is blocked under a hard contract unless an enforceable backend and an explicit scope update cover its effects. This conservative limitation is preferable to falsely promising containment.

**Dependencies and implementation order:** Requires 03 task identity/owner CAS and 01 host-authenticated modal pattern. Integrates 06 evidence and 09 budgets; the deny path can land before either. Backend containment is a release gate, not an optional security claim.

## Requirements traceability

| Requirement ID | Required result |
|---|---|
| F05-1 | Bind writable/protected scope, dependency policy, external action policy, and verification requirements to an accepted task. |
| F05-2 | Enforce scope on every mutation including batch, aliases, shell, MCP/native extensions, and children. |
| F05-3 | Require an explicit contract revision for expansion, with approve/in-scope alternative/deny-with-instructions choices. |
| F05-4 | Always Approve never bypasses task ownership/contracts or hard filesystem/platform boundaries. |
| F05-5 | Provide auditable, race-safe enforcement and a clearly stated limitation for effects that cannot be contained. |

## Data and behavioral contracts

`TaskContract { id, revision, task_id, accepted_plan_revision, writable_paths, protected_paths, allowed_new_dependencies, external_effects, verification_requirements, artifact_write_roots, digest }`. Path entries are validated relative paths with explicit recursive-directory semantics, not arbitrary regexes. Protected scope wins. `PreparedAction` carries canonical tool/capability identity, resolved targets, declared effects, contract digest, actor/owner generation, and source manifest.

Authorization order: validate request and aliases → hard platform/secret/trust limits and explicit denies → actor/owner generation → task contract → permission-mode decision → resource reservation → final revision/target recheck → execute. A grant cannot skip earlier checks. A contract expansion proposal describes the old/new diff, rationale, affected verification, and exact resource/side-effect changes. Only a host-authenticated user decision creates the next contract revision; a model may propose but never approve. New revisions invalidate prepared calls and affected evidence. Verification writes such as `target/` or temporary reports need explicit artifact roots distinct from source scope; do not quietly allow writes everywhere because a command is called a test.

## User experience and mode behavior

The control panel shows accepted plan revision, writable/protected scopes, dependency policy, external action policy, and verification requirements. Scope expansion is a separate interaction from category permission. The user sees when a process capability cannot actually enforce a requested boundary.

## File ownership and task sequence

The task file lists are the implementation map. Register new agent modules in `crates/davinci-agent/src/runtime/mod.rs` only for runtime-owned functionality, new tools in the existing tool/capability registry, and new native TUI views in the existing view dispatcher. Do not introduce a second runtime, permission engine, or event loop. Treat all cross-feature edits to `turn.rs`, `runtime_host.rs`, `davinci_interactive.rs`, and TUI model/routing as sequential integration points, not parallel write scopes.

### Task 1: Contract schema and accepted-plan compilation

**Covers:** F05-1, F05-4

**Test placement:** `crates/davinci-agent/src/runtime/contracts.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Create:** `crates/davinci-agent/src/runtime/contracts.rs` — schema, canonical digest, validation
- **Modify:** `crates/davinci-agent/src/planning.rs` — user-only acceptance contract snapshot
- **Modify:** `crates/davinci-agent/src/runtime/tasks.rs` — immutable contract reference

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f05_protected_wins() {
    assert!(path_scope_allows("src/auth/reset.rs", &["src/auth/"], &["src/auth/private/"]));
    assert!(!path_scope_allows("src/auth/private/key.rs", &["src/auth/"], &["src/auth/private/"]));
    assert!(!path_scope_allows("src/authentic.rs", &["src/auth/"], &[]));
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-agent --offline f05_protected_wins -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn path_scope_allows(normalized: &str, writable: &[&str], protected: &[&str]) -> bool {
    let matches = |scope: &&str| normalized == scope.trim_end_matches('/')
        || (scope.ends_with('/') && normalized.starts_with(*scope));
    !protected.iter().any(matches) && writable.iter().any(matches)
}
```

This helper accepts already-validated canonical relative paths only; it is not a path security resolver. Validate absolute/UNC/device/drive-relative/parent-traversal/alternate-data-stream inputs before it. Resolve case behavior according to the actual filesystem, not unconditional lowercasing. Include accepted plan revision and verification definitions in a deterministic canonical digest. No dependency additions means both manifest and lockfile changes are checked semantically; do not allow a lockfile-only dependency insertion. Require explicit protected project metadata and credential boundaries even when writable scope is broad.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: path prefix collision; symlink/reparse escape; case-sensitive vs insensitive volume; Windows ADS; same file via alias; manifest+lockfile change; protected wins even if later writable entry matches.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-agent/src/runtime/contracts.rs" "crates/davinci-agent/src/planning.rs" "crates/davinci-agent/src/runtime/tasks.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: task-scoped-contracts task 1"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 2: Final execution gate and adapter parity

**Covers:** F05-2, F05-4, F05-5

**Test placement:** `crates/davinci-agent/src/runtime/contracts.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Extend prerequisite module:** `crates/davinci-agent/src/runtime/contracts.rs` — effect intersection check (introduced by F05 Task 1; do not recreate it)
- **Modify:** `crates/davinci-agent/src/turn.rs` — prepare and dispatch contract recheck
- **Modify:** `crates/davinci-agent/src/batch.rs` — member-level guard; shared final-admission checks for every inner operation
- **Modify:** `crates/davinci-agent/src/runtime/capabilities.rs` — declared effects

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f05_modes_do_not_bypass_scope() {
    for mode in ["ask", "edits", "read-only", "auto", "always-approve"] {
        assert!(!contract_gate(mode, false, true, true));
    }
    assert!(contract_gate("always-approve", true, true, true));
    assert!(!contract_gate("always-approve", true, false, true));
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-agent --offline f05_modes_do_not_bypass_scope -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn contract_gate(_mode: &str, in_scope: bool, owner_valid: bool, hard_limits_allow: bool) -> bool {
    in_scope && owner_valid && hard_limits_allow
}
```

Enforce the guard on canonical effects after native pre-tool transformations and again before dispatch, because extensions can change arguments and ownership/contracts may change during approval. Normalize exec_command/write_stdin/shell aliases and every batch member through the same path; a batch wrapper cannot grant scope. Propagate contract digest and owner generation to child worker startup and every mutation lease. A guard failure returns a structured scope violation with requested/allowed/protected effects, not a generic permission prompt that Always Approve can dismiss.

Cover `batch.rs` inner calls with their stable `{parent}#{index}` IDs. Those calls use prepare/run/post_tool directly and currently skip `finalize_tool_call`; an outer-only hook is insufficient. Cached ledger replay is labeled retrieval, not a new execution, and cannot mint fresh scope or evidence.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: alias bypass; batch with safe first member and forbidden second; transformed extension args; child loses scope env; unknown MCP mutation; owner handed off after preparation; poisoned mutex guard path.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-agent/src/runtime/contracts.rs" "crates/davinci-agent/src/turn.rs" "crates/davinci-agent/src/batch.rs" "crates/davinci-agent/src/runtime/capabilities.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: task-scoped-contracts task 2"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 3: Provable process and external-effect boundaries

**Covers:** F05-1, F05-2, F05-4, F05-5

**Test placement:** `crates/davinci-agent/src/runtime/contract_executor.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Create:** `crates/davinci-agent/src/runtime/contract_executor.rs` — confined execution capability interface
- **Modify:** `crates/davinci-agent/src/shell_policy.rs` — classify declared effects without claiming sandboxing
- **Modify:** `crates/davinci-coding-agent/src/native_extensions/graph/worker_hooks.rs` — enforce inherited contract

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f05_unknown_effects_blocked() {
    assert!(!effect_profile_allows(false, true, true));
    assert!(!effect_profile_allows(true, false, true));
    assert!(!effect_profile_allows(true, true, false));
    assert!(effect_profile_allows(true, true, true));
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-agent --offline f05_unknown_effects_blocked -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn effect_profile_allows(effects_proven: bool, backend_enforces: bool, all_effects_authorized: bool) -> bool {
    effects_proven && backend_enforces && all_effects_authorized
}
```

Define `ContractExecutor::capabilities()` and `execute(prepared_action)` as host interfaces whose guarantees are explicit. A command allowlist is not enough to bound a build script, package lifecycle, shell substitution, or network access. Support a backend only when it can enforce the declared filesystem/network/process constraints; otherwise report execution_contract_unenforceable before spawning. For native file tools, resolve and validate targets with existing path rules and prevent link-swap races using platform-safe handles or a serialized trusted workspace boundary. For verification, list derived artifact roots separately. External publish/deploy requires a matching explicit effect grant and is recorded as irreversible.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: curl hidden in build script; shell substitution; test writes outside artifact roots; missing sandbox backend; network default deny; MCP readOnlyHint lies; inherited env cannot widen authority.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-agent/src/runtime/contract_executor.rs" "crates/davinci-agent/src/shell_policy.rs" "crates/davinci-coding-agent/src/native_extensions/graph/worker_hooks.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: task-scoped-contracts task 3"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 4: Explicit scope expansion decisions

**Covers:** F05-3, F05-4, F05-5

**Test placement:** `crates/davinci-agent/src/runtime/contracts.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Extend prerequisite module:** `crates/davinci-agent/src/runtime/contracts.rs` — proposal and revision transition (introduced by F05 Task 1; do not recreate it)
- **Modify:** `crates/davinci-coding-agent/src/davinci_interactive.rs` — scope expansion modal
- **Modify:** `crates/davinci-coding-agent/src/main.rs` — RPC/legacy host bridge

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f05_expansion_requires_user() {
    assert_eq!(expand_revision(4, 4, false, true), None);
    assert_eq!(expand_revision(4, 3, true, true), None);
    assert_eq!(expand_revision(4, 4, true, false), None);
    assert_eq!(expand_revision(4, 4, true, true), Some(5));
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-agent --offline f05_expansion_requires_user -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn expand_revision(current: u64, expected: u64, user_authorized: bool, durable: bool) -> Option<u64> {
    if current != expected || !user_authorized || !durable { return None; }
    current.checked_add(1)
}
```

Render a precise contract diff and the reason an out-of-scope action was stopped. Provide Approve expansion, Request in-scope approach, and Deny with instructions. Persist the new contract and audit event before updating active state; host authority is never taken from a model Boolean. Quiesce pending mutations and invalidate outstanding grants/prepared actions tied to the prior digest. An in-scope alternative becomes guidance with no contract change. In print/unsupported RPC, return scope_expansion_required with no automatic fallback grant.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: Always Approve expansion still needs user; concurrent contract update; storage failure; model fabricates approval; approved path differs from submitted preview; rejecting keeps current contract.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-agent/src/runtime/contracts.rs" "crates/davinci-coding-agent/src/davinci_interactive.rs" "crates/davinci-coding-agent/src/main.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: task-scoped-contracts task 4"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 5: Audit, migration, and verification integration

**Covers:** F05-1, F05-4, F05-5

**Test placement:** `crates/davinci-agent/src/runtime/contracts.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Extend prerequisite module:** `crates/davinci-agent/src/runtime/contracts.rs` — auditable outcome summaries (introduced by F05 Task 1; do not recreate it)
- **Modify:** `crates/davinci-agent/src/runtime/events.rs` — contract decision event
- **Modify:** `crates/davinci-coding-agent/src/native_extensions/ecosystem/verification.rs` — mandatory requirement bridge
- **Modify:** `crates/davinci-coding-agent/src/davinci_surfaces.rs` — scope status

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f05_requirement_not_skippable() {
    assert!(!required_check_satisfied(true, "unavailable"));
    assert!(!required_check_satisfied(true, "skipped"));
    assert!(required_check_satisfied(true, "passed"));
    assert!(required_check_satisfied(false, "unavailable"));
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-agent --offline f05_requirement_not_skippable -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn required_check_satisfied(required: bool, state: &str) -> bool {
    !required || state == "passed"
}
```

Connect contract verification requirements to feature06's current-source evidence gate. Required security verification cannot inherit the older risk-policy behavior that permits unavailable optional security evidence. Record the canonical denied effect, actor/task, contract revision, and reason with secrets redacted. Existing unconstrained legacy tasks remain visibly legacy/uncontracted; do not claim their prior work was scoped. A user must explicitly accept a contract before the new guarantee is displayed. Update scope docs with enforceable backend limitations and platform matrix.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: old task without contract; invalid unknown contract schema; missing required security scanner; extra unrelated passed tests cannot satisfy named requirement; correct contract revision displayed after resume.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-agent/src/runtime/contracts.rs" "crates/davinci-agent/src/runtime/events.rs" "crates/davinci-coding-agent/src/native_extensions/ecosystem/verification.rs" "crates/davinci-coding-agent/src/davinci_surfaces.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: task-scoped-contracts task 5"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

## Failure, security, and recovery matrix

| Failure | Behavior |
|---|---|
| Unknown/unprovable process effects | Refuse under hard contract; explain absent enforcement capability. |
| Scope changed after approval | Reject stale prepared action and re-evaluate. |
| User denies expansion | Retain contract and request alternative or stop. |
| Link/path identity changes | Refuse, never resolve outside authorized root. |
| Required verification unavailable | Completion blocked; record explicit gap. |

## Persistence, migration, and rollback

Contract references are additive and versioned. Legacy sessions stay readable but cannot be displayed as contract-enforced without explicit acceptance. Unknown future contract version is fail-closed for mutations. Rollback retains the latest contract and denies unsupported actions rather than reverting to unrestricted execution.

## End-to-end acceptance and release gates

Run the same forbidden migration write through write/edit/apply_patch/bash/exec_command/batch/native-extension/MCP/child-worker paths and require no side effect, including in Always Approve. Approve only the named migration path, then prove its sibling remains protected. Run a malicious fixture build script and either show actual backend confinement or the explicit unenforceable refusal; parsing its command text alone is not a passing containment test.

- [ ] Run the feature's listed inline tests with the offline fixture environment and prove each filter selects tests. Record test names, counts, exit codes, and source fingerprint.
- [ ] Run `rtk cargo fmt --check`, relevant crate tests, and then `rtk cargo test --workspace --offline` and `rtk cargo clippy --workspace --all-targets --offline -- -D warnings`. A pre-existing failure is reported, not silently waived or attributed to this feature.
- [ ] Test interactive Ratatui, legacy TUI, print/JSON, and RPC adapters where this plan adds interactions. Noninteractive uncertainty never becomes approval.
- [ ] Verify old session/config fixtures still load, source changes invalidate affected evidence, and cancellation leaves an explainable recoverable state.
- [ ] Update the user-facing docs for this feature and the relevant help/command discovery entries in the final integration task. Keep internal lifecycle operations internal.
- [ ] For later executable delivery, follow `CLAUDE.md`: resolve the actual launched executable, build offline, back up before replacing, compare build/installed hashes, and distinguish automated UI evidence from a physical keyboard check. Do not interrupt an active user's session to unlock a binary.

**Execution exit condition:** all source-spec requirements above have a passing test or a named, explicit manual verification gap; no missing required proof is described as complete. The roadmap's final integration gate applies even when this feature's own tests pass.

## References

Local code references above are anchored to repository HEAD `9c42820`. The supplied spec is preserved as Markdown in this bundle. [Repository audit](2026-09-07-00-repository-audit.md), [shared contracts](2026-09-07-00-shared-runtime-contracts.md), and [cross-feature validation matrix](2026-09-07-00-requirements-and-validation.md) explain the verified baseline and proposed additions.


