# Evidence-backed Completion Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Produce honest per-dimension completion status backed by immutable execution evidence for the exact source that was verified.

**Architecture:** Introduce a shared EvidenceStore and source-manifest builder, with adapters for ordinary tools, graph verification, builds, installation checks, and interaction artifacts. Task completion consumes validated host-generated evidence rather than assistant prose.

**Tech Stack:** Rust 1.83.0; the existing Davinci runtime, serde/serde_json, SHA-256, Ratatui/Crossterm, and fixture-driven inline Rust tests. Additional backend decisions are stated explicitly below.

**Spec:** [Evidence-backed Completion — supplied source specification](2026-09-07-davinci-feature-specs/06-evidence-backed-completion.md)

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

`runtime/tasks.rs` has a completion decision hook but needs atomic revalidation. `native_extensions/graph/verify.rs` already rejects empty/all-skipped/interrupted verification, and `ecosystem/verification.rs::VerificationBundle` aggregates verification/security evidence. Its default Risk mode can permit unavailable optional security verification; this must not satisfy a contract-mandated check. `stats.rs` tracks attempts and time, while learning/evidence.rs extracts signals for learning; learning confidence is not proof of task completion.

`davinci-agent/src/evidence.rs::EvidenceStore` already stores overflow output, and `tool_ledger.rs` records tool outcomes. They are not a typed verification authority. Call the new receipt persistence facade `VerificationEvidenceStore` (in the proposed runtime module) and reference the existing overflow store for large outputs, rather than replacing it or conflating the two stores.

## Scope, alternatives, and chosen approach

Choose immutable evidence records and a deterministic evaluator. A single completed Boolean hides missing installation/UI checks; parsing an assistant's “tests passed” text is not trusted evidence. Hash complete relevant source/config/toolchain inputs, not only Git HEAD or status. A test is current only if pre-run and post-run input manifests match and the current manifest still matches. Verification scope must include transitive dependencies and relevant untracked/generated inputs; when that set cannot be proven, conservatively include the workspace or label freshness unproven.

**Dependencies and implementation order:** Requires 03 durable task state. Shares manifests with 04/05, reservations with 09, and artifact receipts with 11/14. Build the evidence core before wiring task completion or graph verify-only.

## Requirements traceability

| Requirement ID | Required result |
|---|---|
| F06-1 | Record command/tool provenance, execution outcome, relevant source fingerprint, useful runtime versions, and artifacts. |
| F06-2 | Mark evidence stale when relevant inputs change and reject prior-source success as current proof. |
| F06-3 | Distinguish implemented/tested/built/installed/live-verified and show remaining gaps. |
| F06-4 | Reserve verification/handoff resources and refuse proof-backed completion without required current evidence. |
| F06-5 | Use host-authenticated evidence with precise source coverage and no false success from skipped/unavailable checks. |

## Data and behavioral contracts

`EvidenceRecord { id, task_id, attempt, actor_id, operation_id, requirement_id, kind, argv, cwd, started_at_ms, finished_at_ms, exit: ExitOutcome, source_before, source_after, input_manifest, runtime_versions, artifact_refs, assertions, status }`. `ExitOutcome = Exited(i32) | Signalled | TimedOut | Cancelled | NotStarted`; none except a successful executed command can count as passed. `ArtifactRef { id, sha256, media_type, size, relative_store_path, redaction }`.

Completion dimensions are Implementation, TargetedTests, Build, InstalledApp, LiveUiCheck. Each is `NotRequired | NotPerformed | Running | PassedCurrent | Failed | Stale | Unproven`. A contract names required dimensions and requirement IDs. Installation proof compares the resolved executable's cryptographic hash to the verified build and records resolution path; a matching version string is insufficient. PTY/browser assertions count only for the environment actually tested; physical keyboard verification remains a separate explicit gap. No live UI check is inferred from compilation or screenshots alone.

## User experience and mode behavior

The completion sheet uses separate rows for implementation, targeted tests, build, installed app, and live UI. Every green row names its proof and current source. Missing physical interaction is a visible gap, not hidden behind a successful compilation.

## File ownership and task sequence

The task file lists are the implementation map. Register new agent modules in `crates/davinci-agent/src/runtime/mod.rs` only for runtime-owned functionality, new tools in the existing tool/capability registry, and new native TUI views in the existing view dispatcher. Do not introduce a second runtime, permission engine, or event loop. Treat all cross-feature edits to `turn.rs`, `runtime_host.rs`, `davinci_interactive.rs`, and TUI model/routing as sequential integration points, not parallel write scopes.

### Task 1: Canonical source manifests and evidence identity

**Covers:** F06-1, F06-2, F06-5

**Test placement:** `crates/davinci-agent/src/runtime/evidence.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Create:** `crates/davinci-agent/src/runtime/evidence.rs` — record and requirement types
- **Create:** `crates/davinci-agent/src/runtime/source_manifest.rs` — bounded content-based input manifest
- **Modify:** `crates/davinci-agent/src/runtime/tasks.rs` — evidence references

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f06_source_must_match() {
    assert!(evidence_current("a", "a", "a", true));
    assert!(!evidence_current("a", "b", "b", true));
    assert!(!evidence_current("a", "a", "b", true));
    assert!(!evidence_current("a", "a", "a", false));
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-agent --offline f06_source_must_match -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn evidence_current(before: &str, after: &str, current: &str, complete_coverage: bool) -> bool {
    complete_coverage && before == after && after == current
}
```

Hash sorted path identities, file kind/mode, raw content, dependency manifests/lockfiles, selected toolchain/config, and exact verification command profile into a length-delimited SHA-256 manifest. Track missing files explicitly so creation invalidates coverage. Never use modification time alone. Exclude irrelevant private secrets from content capture and prevent raw source blobs from leaking into public evidence exports; their inclusion in a proof must follow authorized read scope. If source changes while hashing, retry within a bound or mark unstable/unproven. Reuse the manifest with checkpoint/graph replay adapters.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: HEAD unchanged but bytes edited; untracked dependency created; file removed; lockfile changed; CRLF-only change; symlink target changed; incomplete scope; edit during manifest scan.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-agent/src/runtime/evidence.rs" "crates/davinci-agent/src/runtime/source_manifest.rs" "crates/davinci-agent/src/runtime/tasks.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: evidence-backed-completion task 1"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 2: Trusted execution receipts and artifacts

**Covers:** F06-1, F06-5

**Test placement:** `crates/davinci-agent/src/runtime/evidence_store.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Create:** `crates/davinci-agent/src/runtime/evidence_store.rs` — immutable receipts and artifact indexing
- **Modify:** `crates/davinci-agent/src/turn.rs` — finalize actual tool execution outcome
- **Modify:** `crates/davinci-agent/src/jobs.rs` — asynchronous exit/cancellation receipt
- **Modify:** `crates/davinci-agent/src/batch.rs` — inner-operation receipts through the shared finalizer
- **Modify:** `crates/davinci-agent/src/evidence.rs` — reference existing lossless output artifacts without replacing the overflow store

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f06_no_skipped_success() {
    assert!(execution_passed(true, Some(0), false, false));
    assert!(!execution_passed(false, Some(0), false, false));
    assert!(!execution_passed(true, Some(0), true, false));
    assert!(!execution_passed(true, None, false, true));
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-agent --offline f06_no_skipped_success -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn execution_passed(started: bool, exit_code: Option<i32>, timed_out: bool, cancelled: bool) -> bool {
    started && exit_code == Some(0) && !timed_out && !cancelled
}
```

Receipts are generated by the actual executor after process completion, never by a model tool accepting claimed exit codes. Include stdout/stderr artifact hashes, truncation metadata, assertion counts, and complete runtime versions when relevant. A long-running job is Running until its owned process exits; merely starting it is not success. Commit receipt metadata and artifact references atomically through the task store. Artifact retrieval validates path, size, media type, and hash; displays treat ANSI/HTML as untrusted. Record all failed attempts without double-counting repeated delivery of the same operation ID.

Capture trusted exit/status/artifact metadata before post-hook rewriting and governor compression. A later post-hook veto remains a separate negative completion condition. Route both ordinary calls and `batch.rs` inner operations through one exactly-once receipt finalizer; the current inner batch path bypasses `finalize_tool_call`. Preserve simulated/skipped/aborted/unknown provenance explicitly: `dry_run_verify_exec` can return exit 0 but must never produce real verification proof.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: permission denied before launch; process killed; background job not finished; zero tests selected; forged evidence JSON; duplicate exit notification; missing artifact blob; oversize output retains retrieval reference. Batch inner receipt once; successful raw exit followed by hook veto; simulated exit zero cannot satisfy real evidence.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-agent/src/runtime/evidence_store.rs" "crates/davinci-agent/src/turn.rs" "crates/davinci-agent/src/jobs.rs" "crates/davinci-agent/src/batch.rs" "crates/davinci-agent/src/evidence.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: evidence-backed-completion task 2"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 3: Deterministic completion dimensions and gates

**Covers:** F06-2, F06-3, F06-4, F06-5

**Test placement:** `crates/davinci-agent/src/runtime/completion.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Create:** `crates/davinci-agent/src/runtime/completion.rs` — requirement matching and completion evaluator
- **Modify:** `crates/davinci-agent/src/runtime/tasks.rs` — compare-and-swap commit after evidence validation
- **Modify:** `crates/davinci-coding-agent/src/native_extensions/ecosystem/verification.rs` — normalize graph/security receipts

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f06_required_dimensions() {
    assert!(completion_allowed(&[(true, "passed_current"), (false, "not_performed")]));
    assert!(!completion_allowed(&[(true, "stale")]));
    assert!(!completion_allowed(&[(true, "unavailable")]));
    assert!(!completion_allowed(&[]));
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-agent --offline f06_required_dimensions -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn completion_allowed(dimensions: &[(bool, &str)]) -> bool {
    !dimensions.is_empty() && dimensions.iter().all(|(required, state)| !required || *state == "passed_current")
}
```

Match evidence to exact requirement IDs, source manifest, task attempt, contract revision, and expected command/assertion profile. One unrelated passing command cannot satisfy all checks. Require explicit Implementation evidence even when tests are NotRequired; docs-only tasks can use a contract that requires document validation rather than a build. Verify mandatory security requirements fail closed regardless of optional Risk policy. After evaluation, reacquire the task commit coordinator and recheck revisions/source before durable completion; any changed input yields stale, not a race to completed.

The existing ecosystem Risk security mode can accept `SecurityVerification::Unavailable`; that legacy behavior is not proof of a required security check. An explicitly mandatory security requirement fails closed on unavailable evidence without silently redefining unrelated legacy policy defaults.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: all-skipped graph verification; required security unavailable; pass from old task attempt; wrong requirement ID; new source after evaluation; no dimensions; explicitly not-required build is displayed as such.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-agent/src/runtime/completion.rs" "crates/davinci-agent/src/runtime/tasks.rs" "crates/davinci-coding-agent/src/native_extensions/ecosystem/verification.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: evidence-backed-completion task 3"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 4: Budget reservation and installed/runtime proof

**Covers:** F06-3, F06-4

**Test placement:** `crates/davinci-coding-agent/src/runtime_host.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Extend prerequisite module:** `crates/davinci-agent/src/runtime/completion.rs` — verification reservation requirement (introduced by F06 Task 3; do not recreate it)
- **Modify:** `crates/davinci-coding-agent/src/runtime_host.rs` — budget and evidence adapter
- **Create:** `crates/davinci-coding-agent/src/completion_delivery.rs` — resolved executable/build hash inspection

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f06_installed_hash_match() {
    assert!(installed_matches(Some("sha-a"), Some("sha-a"), true));
    assert!(!installed_matches(Some("sha-a"), Some("sha-b"), true));
    assert!(!installed_matches(None, None, true));
    assert!(!installed_matches(Some("sha-a"), Some("sha-a"), false));
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-coding-agent --offline f06_installed_hash_match -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn installed_matches(build: Option<&str>, installed: Option<&str>, resolution_verified: bool) -> bool {
    resolution_verified && match (build, installed) { (Some(a), Some(b)) => !a.is_empty() && a == b, _ => false }
}
```

Use feature09's named Verification and Handoff reservation classes before implementation can consume the last resources. If resources are exhausted, stop and surface an incomplete evidence matrix rather than fabricate proof. The delivery inspector resolves aliases/wrappers/PATH to the actual executable and reads hashes only; replacement remains a separate authorized action following CLAUDE.md. Record build source and installed artifact chain. Existing sessions may still execute an older mapped binary; explicitly distinguish disk installation from an active process and require restart by the user where needed.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: unknown pricing; no verification reserve; PATH resolves old binary; same version different bytes; executable locked; wrapper targets another file; running process predates replacement; installation not required.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-agent/src/runtime/completion.rs" "crates/davinci-coding-agent/src/runtime_host.rs" "crates/davinci-coding-agent/src/completion_delivery.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: evidence-backed-completion task 4"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 5: Completion report, stale refresh, and evidence export

**Covers:** F06-1, F06-2, F06-3, F06-5

**Test placement:** `crates/davinci-coding-agent/src/davinci_surfaces.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Create:** `crates/davinci-tui/src/davinci/views/completion.rs` — per-dimension report
- **Modify:** `crates/davinci-coding-agent/src/davinci_surfaces.rs` — evidence summaries and gaps
- **Modify:** `crates/davinci-coding-agent/src/output.rs` — structured print/JSON completion summary

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f06_report_gap() {
    assert_eq!(evidence_label(false, false, false), "not performed");
    assert_eq!(evidence_label(true, true, false), "stale");
    assert_eq!(evidence_label(true, true, true), "passed on current source");
    assert_eq!(evidence_label(true, false, true), "failed");
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-coding-agent --offline f06_report_gap -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn evidence_label(performed: bool, passed: bool, current: bool) -> &'static str {
    if !performed { "not performed" } else if !passed { "failed" }
    else if !current { "stale" } else { "passed on current source" }
}
```

Show source fingerprint, last verified time, each dimension, commands/artifact links, and named gaps. Refresh re-runs a selected authorized verification profile on the current tree; it does not mutate old evidence records. Plain assistant output receives a bounded evidence summary so it cannot honestly describe missing dimensions as passed. Export only redacted public evidence and stable artifact refs, not credentials, hidden reasoning, or raw browser traces without review. Attach feature11 interaction evidence while labeling virtual terminal/browser environment separately from physical keyboard checks.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: live UI not performed after build; invalidated tests render stale; same-source rerun creates new receipt; redacted export includes truncation notice; JSON stdout valid; terminal width40; no artifact fetch outside scope.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-tui/src/davinci/views/completion.rs" "crates/davinci-coding-agent/src/davinci_surfaces.rs" "crates/davinci-coding-agent/src/output.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: evidence-backed-completion task 5"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

## Failure, security, and recovery matrix

| Failure | Result |
|---|---|
| Source changed during/after verification | Stale/unproven, never current pass. |
| Missing exit code or command never started | NotPerformed/Cancelled/Unproven, never zero by default. |
| Artifact missing or hash mismatch | Invalid evidence; required completion blocked. |
| Required check unavailable | Explicit gap, even if optional risk policy allows graph continuation. |
| Budget exhausted | Preserve evidence and handoff; no generic done claim. |

## Persistence, migration, and rollback

Old success records without sufficient provenance remain historical/unproven; do not invent input manifests retrospectively. Version evidence DTOs with additive fields and explicit unknown states. Rollback keeps artifacts and prevents the old simple task completion path from overriding mandatory evidence requirements.

## End-to-end acceptance and release gates

Complete a fixture task with current tests and build evidence, then modify one relevant source byte without changing HEAD. The task/report must become stale or require re-verification before another completed claim. Demonstrate separate installed and live-UI statuses, a skipped verification that cannot pass, and a required security check that is unavailable. A same-version/different-hash installed executable must not match the verified build.

- [ ] Run the feature's listed inline tests with the offline fixture environment and prove each filter selects tests. Record test names, counts, exit codes, and source fingerprint.
- [ ] Run `rtk cargo fmt --check`, relevant crate tests, and then `rtk cargo test --workspace --offline` and `rtk cargo clippy --workspace --all-targets --offline -- -D warnings`. A pre-existing failure is reported, not silently waived or attributed to this feature.
- [ ] Test interactive Ratatui, legacy TUI, print/JSON, and RPC adapters where this plan adds interactions. Noninteractive uncertainty never becomes approval.
- [ ] Verify old session/config fixtures still load, source changes invalidate affected evidence, and cancellation leaves an explainable recoverable state.
- [ ] Update the user-facing docs for this feature and the relevant help/command discovery entries in the final integration task. Keep internal lifecycle operations internal.
- [ ] For later executable delivery, follow `CLAUDE.md`: resolve the actual launched executable, build offline, back up before replacing, compare build/installed hashes, and distinguish automated UI evidence from a physical keyboard check. Do not interrupt an active user's session to unlock a binary.

**Execution exit condition:** all source-spec requirements above have a passing test or a named, explicit manual verification gap; no missing required proof is described as complete. The roadmap's final integration gate applies even when this feature's own tests pass.

## References

Local code references above are anchored to repository HEAD `9c42820`. The supplied spec is preserved as Markdown in this bundle. [Repository audit](2026-09-07-00-repository-audit.md), [shared contracts](2026-09-07-00-shared-runtime-contracts.md), and [cross-feature validation matrix](2026-09-07-00-requirements-and-validation.md) explain the verified baseline and proposed additions.


