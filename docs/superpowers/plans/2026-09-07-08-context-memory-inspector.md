# Context and Memory Inspector Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let the user inspect and correct the exact prepared context and memory provenance without disabling mandatory policy or silently changing in-flight requests.

**Architecture:** Add a final provider-view manifest and user-owned context overlay. Extend the existing ContextBroker for candidate selection reasons, then combine its packet with system/tool/history/plan context at final request preparation. Keep UI reads distinct from model context injection.

**Tech Stack:** Rust 1.83.0; the existing Davinci runtime, serde/serde_json, SHA-256, Ratatui/Crossterm, and fixture-driven inline Rust tests. Additional backend decisions are stated explicitly below.

**Spec:** [Context and Memory Inspector — supplied source specification](2026-09-07-davinci-feature-specs/08-context-memory-inspector.md)

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

`runtime/context.rs` already defines ContextRequest, ContextItem, ContextPacket, ContextSource, and a bounded deterministic ContextBroker. It currently returns selected items, not rejected-candidate reasons or a full request manifest. The actual provider context also includes prompts, tool schemas, history, pruning, and host overhead outside that broker. `native_extensions/ecosystem/context.rs`, vector_memory.rs, and token_governor.rs already supply provenance/retrieval behavior. An inspector that displays only ContextPacket would therefore falsely claim to show the whole prepared request.

## Scope, alternatives, and chosen approach

Choose immutable request snapshots plus versioned pin/exclude/correction overlays. Re-running retrieval whenever the user opens the inspector would show a different context than the pending request; editing provider messages in place would race active streams. The inspector reads the exact prepared snapshot, and changes apply only at the next request boundary with an explicit new revision. Token counts remain estimates unless a provider supplies exact accounting; do not describe character-count estimates as billed token totals.

**Dependencies and implementation order:** Uses 02 structured decisions,06 evidence manifests, and existing broker/governor/memory systems. Shared context-manifest work is independent of UI; integrate overlays after mandatory item classification is complete.

## Requirements traceability

| Requirement ID | Required result |
|---|---|
| F08-1 | Show the final prepared request context and why each item is present, including token breakdown. |
| F08-2 | Support inspect/pin/exclude/refresh with explicit next-boundary application and bounded context. |
| F08-3 | Distinguish user decision, repository fact, agent inference, and tool evidence provenance and freshness. |
| F08-4 | Allow explicit user correction/removal of memory without silent reinjection or inference promotion. |
| F08-5 | Mandatory system/permission/trust/safety context cannot be excluded. |

## Data and behavioral contracts

`PreparedContextManifest { request_id, root_run_id, source_revision, overlay_revision, entries, estimated_total_tokens, estimated_overhead_tokens, manifest_digest }`; `ContextManifestEntry { id, category, provenance_kind, source_ref, content_hash, token_estimate, selected, selection_reason, mandatory, freshness, inspect_ref }`. ProvenanceKind is UserDecision, RepositoryFact, AgentInference, ToolEvidence, or MandatoryPolicy. Stable IDs are derived from source identity/version, not list order.

`ContextOverlay { revision, pinned_ids, excluded_ids, memory_corrections, tombstones }` is user-owned. A pin raises optional selection priority but cannot exceed the hard context budget or turn untrusted data into instructions. Exclude cannot remove mandatory system/permission/trust/safety items. Repository facts include locations and current fingerprints; tool evidence carries command/result/freshness. Agent inference stays labeled as inference even if persisted for later review. User corrections/removals create provenance-preserving tombstones so retrieval/indexing cannot silently reintroduce the old fact. Display content is redacted; no hidden reasoning or credentials are exposed as an inspection feature.

## User experience and mode behavior

Each row answers what will be sent, why, from which source, with what freshness, and at what estimated token cost. User controls apply at a declared boundary. Mandatory policy is inspectable in an appropriate redacted summary but not removable.

## File ownership and task sequence

The task file lists are the implementation map. Register new agent modules in `crates/davinci-agent/src/runtime/mod.rs` only for runtime-owned functionality, new tools in the existing tool/capability registry, and new native TUI views in the existing view dispatcher. Do not introduce a second runtime, permission engine, or event loop. Treat all cross-feature edits to `turn.rs`, `runtime_host.rs`, `davinci_interactive.rs`, and TUI model/routing as sequential integration points, not parallel write scopes.

### Task 1: Provider-view manifest and selection reasons

**Covers:** F08-1, F08-3, F08-5

**Test placement:** `crates/davinci-agent/src/runtime/context.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Modify:** `crates/davinci-agent/src/runtime/context.rs` — selection ledger and stable item IDs
- **Create:** `crates/davinci-agent/src/runtime/context_manifest.rs` — complete prepared manifest
- **Modify:** `crates/davinci-agent/src/lib.rs` — final provider-view capture
- **Modify:** `crates/davinci-coding-agent/src/runtime_host.rs` — host overhead provenance

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f08_total_includes_mandatory() {
    assert_eq!(context_total(&[3200, 1400, 2100, 700, 1900], 400), Some(9700));
    assert_eq!(context_total(&[u64::MAX], 1), None);
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-agent --offline f08_total_includes_mandatory -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn context_total(parts: &[u64], overhead: u64) -> Option<u64> {
    parts.iter().try_fold(overhead, |sum, value| sum.checked_add(*value))
}
```

Instrument the last stable preparation point after pruning/host injection and before provider serialization, not just broker collection. Track system/harness policy, tool schemas, Living Plan, history, memory, code, tool results, and wrapper overhead without double counting broker items re-injected elsewhere. Preserve candidate exclusion reasons such as stale, over budget, user excluded, scope denied, or duplicate. Record snapshot digest and request ID together; inspection must not alter cache affinity or call a provider. Respect existing provider-specific serialization differences and label estimates by the estimator used.

Capture `Agent::messages_for_provider` after plan/ephemeral context and history projection, plus system/tool-schema metadata. Keep runtime `ContextPacket` and ecosystem graph `ContextPacket` as separate source adapters. `estimated_context_tokens` is a heuristic, not an exact tokenizer or proven upper bound; distinguish estimate from provider-reported usage.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: broker empty but mandatory prompt nonempty; duplicate memory suppression; pruned large tool result; tool schema overhead; overflow; empty context source; same request inspected twice without new retrieval.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-agent/src/runtime/context.rs" "crates/davinci-agent/src/runtime/context_manifest.rs" "crates/davinci-agent/src/lib.rs" "crates/davinci-coding-agent/src/runtime_host.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: context-memory-inspector task 1"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 2: Mandatory-safe pin and exclude overlays

**Covers:** F08-2, F08-5

**Test placement:** `crates/davinci-agent/src/runtime/context_overlay.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Create:** `crates/davinci-agent/src/runtime/context_overlay.rs` — user-owned overlay reducer
- **Modify:** `crates/davinci-agent/src/runtime/context.rs` — apply optional selection preferences

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f08_cannot_exclude_policy() {
    assert!(!overlay_change_allowed(true, "exclude", true));
    assert!(overlay_change_allowed(false, "exclude", true));
    assert!(!overlay_change_allowed(false, "pin", false));
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-agent --offline f08_cannot_exclude_policy -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn overlay_change_allowed(mandatory: bool, action: &str, trusted_user: bool) -> bool {
    trusted_user && match action { "exclude" => !mandatory, "pin" => !mandatory, _ => false }
}
```

Resolve mandatory classification from trusted provider/host metadata, never from a model-supplied false value. Pins affect only optional ranking; if all pins cannot fit, show the omitted pins and ask the user to revise the selection without cutting safety instructions. Enforce permission/trust filters before optional inclusion. Update overlay with expected_revision and persist before acknowledging. Change only the next request, leaving an in-flight prepared manifest immutable. If a user excludes evidence essential to a task, show the resulting verification/context gap rather than allowing a contradictory completion claim.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: policy item renamed; mandatory flag spoofed; pins exceed cap; stale item pinned; scope-denied file requested; change during streaming; duplicate pin; corrupted overlay fails closed for mandatory data.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-agent/src/runtime/context_overlay.rs" "crates/davinci-agent/src/runtime/context.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: context-memory-inspector task 2"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 3: Provenance, corrections, and tombstones

**Covers:** F08-3, F08-4

**Test placement:** `crates/davinci-coding-agent/src/native_extensions/vector_memory.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Extend prerequisite module:** `crates/davinci-agent/src/runtime/context_overlay.rs` — correction and tombstone references (introduced by F08 Task 2; do not recreate it)
- **Modify:** `crates/davinci-coding-agent/src/native_extensions/vector_memory.rs` — filter superseded records
- **Modify:** `crates/davinci-coding-agent/src/native_extensions/learning/retrieval.rs` — retain exact version provenance

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f08_inference_not_fact() {
    assert_eq!(provenance_after_review("inference", false, false), "inference");
    assert_eq!(provenance_after_review("inference", true, false), "user_decision");
    assert_eq!(provenance_after_review("repository_fact", false, true), "removed");
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-coding-agent --offline f08_inference_not_fact -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn provenance_after_review(original: &str, explicit_user_confirmation: bool, tombstoned: bool) -> &str {
    if tombstoned { "removed" } else if explicit_user_confirmation { "user_decision" } else { original }
}
```

A confidence score or successful learning review must not relabel an inference as a user-approved fact. Add explicit provenance/supersession IDs to the existing memory records or a sidecar overlay, preserving existing versioned skill attribution. User edits create a new record referencing the prior record, not a history rewrite. Tombstones are scoped to the project/user decision as chosen, filter both sparse and dense retrieval, and survive reindexing. Removing a context item is not automatically deleting its original session history; explain the distinction. Refresh uses authorized source/tool retrieval and returns a new evidence record, not a fabricated fresh timestamp.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: reindex resurrects removed memory; same fact in sparse/dense index; inference high confidence remains inference; correction during background learning; source missing; global vs project scope; old memory without provenance shown unknown.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-agent/src/runtime/context_overlay.rs" "crates/davinci-coding-agent/src/native_extensions/vector_memory.rs" "crates/davinci-coding-agent/src/native_extensions/learning/retrieval.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: context-memory-inspector task 3"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 4: Snapshot isolation, refresh, and governor compatibility

**Covers:** F08-1, F08-2, F08-5

**Test placement:** `crates/davinci-agent/src/runtime/context_manifest.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Extend prerequisite module:** `crates/davinci-agent/src/runtime/context_manifest.rs` — request/snapshot identity checks (introduced by F08 Task 1; do not recreate it)
- **Modify:** `crates/davinci-agent/src/turn.rs` — boundary application of overlays
- **Modify:** `crates/davinci-coding-agent/src/native_extensions/token_governor.rs` — preserve retrieval and pruning resets

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f08_overlay_next_request() {
    assert_eq!(overlay_application(true, 3, 4), "next_request");
    assert_eq!(overlay_application(false, 3, 4), "reprepare");
    assert_eq!(overlay_application(false, 4, 4), "unchanged");
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-agent --offline f08_overlay_next_request -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn overlay_application(in_flight: bool, captured_revision: u64, current_revision: u64) -> &'static str {
    if captured_revision == current_revision { "unchanged" }
    else if in_flight { "next_request" } else { "reprepare" }
}
```

Capture the selected packet and final manifest as one prepared request unit. If an idle overlay changes, discard and reprepare before send; if already streaming, retain the original manifest and label pending changes. Do not treat inspector viewing as a model read that populates dedupe ledgers. Preserve native_context_pruned resets so excluded/pruned content can be retrieved again when legitimately needed. Keep retrieve_output available whenever compressed results are presented. Refresh failures retain stale evidence visibly and never reset the source fingerprint to the current time.

Keep pruning invariants in `Agent::prune_context`: removed output must remain retrievable, structured image blocks stay lossless, and native governor deduplication resets after context removal. Reading the inspector must not call memory retrieval again or change a graph packet already frozen for a node.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: pruning then refresh; repeated inspector opening; stream starts during overlay edit; missing retrieve_output; failed source read; context cache hit still shows correct provenance; mandatory content unavailable blocks request.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-agent/src/runtime/context_manifest.rs" "crates/davinci-agent/src/turn.rs" "crates/davinci-coding-agent/src/native_extensions/token_governor.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: context-memory-inspector task 4"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 5: Inspector view and user correction workflow

**Covers:** F08-1, F08-2, F08-3, F08-4, F08-5

**Test placement:** `crates/davinci-coding-agent/src/davinci_surfaces.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Create:** `crates/davinci-tui/src/davinci/views/context_inspector.rs` — manifest rows and provenance details
- **Modify:** `crates/davinci-coding-agent/src/davinci_surfaces.rs` — manifest and overlay adapter
- **Modify:** `crates/davinci-coding-agent/src/davinci_interactive.rs` — inspect/pin/exclude/refresh controls
- **Modify:** `crates/davinci-coding-agent/src/output.rs` — bounded JSON manifest summary

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f08_visible_freshness() {
    assert_eq!(freshness_label(false, false), "unproven");
    assert_eq!(freshness_label(true, false), "stale");
    assert_eq!(freshness_label(true, true), "fresh");
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-coding-agent --offline f08_visible_freshness -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn freshness_label(has_provenance: bool, fingerprint_matches: bool) -> &'static str {
    if !has_provenance { "unproven" } else if fingerprint_matches { "fresh" } else { "stale" }
}
```

Display category, estimated tokens, mandatory/pinned/selected status, provenance, freshness, source reference, and inclusion/exclusion reason. Enter opens a bounded redacted preview; p pins, x excludes optional content, and r refreshes authorized evidence. Memory edit/remove has an explicit confirmation showing scope and reinjection/tombstone behavior. The total is labeled estimated prepared tokens, not provider billing. Provide a clear Current request versus Pending next request switch. No automatic provider request or disk deletion occurs when opening the sheet.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: narrow terminal; huge source body paginated; secrets redacted; keyboard exclusion of mandatory item rejected in host; correction survives resume; JSON no private body by default; inspector absent before first request.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-tui/src/davinci/views/context_inspector.rs" "crates/davinci-coding-agent/src/davinci_surfaces.rs" "crates/davinci-coding-agent/src/davinci_interactive.rs" "crates/davinci-coding-agent/src/output.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: context-memory-inspector task 5"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

## Failure, security, and recovery matrix

| Failure | Result |
|---|---|
| Unknown/stale provenance | Label unproven/stale; no silent fact promotion. |
| Too many pinned items | Bounded selection with visible omitted pins; mandatory content retained. |
| Refresh fails | Preserve stale item and expose error. |
| Active request changes underneath UI | Display immutable captured revision; changes pending for next boundary. |
| Missing mandatory policy | Refuse request preparation, not empty safety context. |

## Persistence, migration, and rollback

Existing memory without provenance remains legacy/unproven; do not retroactively label it user-approved. Persist overlays/tombstones with version and scope. Old clients may ignore optional display metadata but may not disable policy. Rollback leaves corrections and tombstones effective in retrieval, even if the inspector UI is removed.

## End-to-end acceptance and release gates

Capture a canned final provider request and verify the inspector manifest accounts for system/tool/history/plan/memory/tool evidence without omissions or duplication. Exclude one optional fact and confirm only the next request changes. Attempt to exclude mandatory policy through UI and forged RPC/model input and require rejection. Correct an inference, reindex memory, and prove the original does not silently return as a durable fact.

- [ ] Run the feature's listed inline tests with the offline fixture environment and prove each filter selects tests. Record test names, counts, exit codes, and source fingerprint.
- [ ] Run `rtk cargo fmt --check`, relevant crate tests, and then `rtk cargo test --workspace --offline` and `rtk cargo clippy --workspace --all-targets --offline -- -D warnings`. A pre-existing failure is reported, not silently waived or attributed to this feature.
- [ ] Test interactive Ratatui, legacy TUI, print/JSON, and RPC adapters where this plan adds interactions. Noninteractive uncertainty never becomes approval.
- [ ] Verify old session/config fixtures still load, source changes invalidate affected evidence, and cancellation leaves an explainable recoverable state.
- [ ] Update the user-facing docs for this feature and the relevant help/command discovery entries in the final integration task. Keep internal lifecycle operations internal.
- [ ] For later executable delivery, follow `CLAUDE.md`: resolve the actual launched executable, build offline, back up before replacing, compare build/installed hashes, and distinguish automated UI evidence from a physical keyboard check. Do not interrupt an active user's session to unlock a binary.

**Execution exit condition:** all source-spec requirements above have a passing test or a named, explicit manual verification gap; no missing required proof is described as complete. The roadmap's final integration gate applies even when this feature's own tests pass.

## References

Local code references above are anchored to repository HEAD `9c42820`. The supplied spec is preserved as Markdown in this bundle. [Repository audit](2026-09-07-00-repository-audit.md), [shared contracts](2026-09-07-00-shared-runtime-contracts.md), and [cross-feature validation matrix](2026-09-07-00-requirements-and-validation.md) explain the verified baseline and proposed additions.


