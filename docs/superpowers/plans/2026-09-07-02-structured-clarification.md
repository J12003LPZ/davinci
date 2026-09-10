# Structured Clarification / Decision Interview Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Capture explicit user decisions for material ambiguity without mistaking a recommendation, timeout, or model response for consent.

**Architecture:** Add a typed decision store inside Living Plan metadata and a first-class ask_user_question tool that emits a host interaction. Reuse the approval modal transport but keep decision state and permission consent entirely separate.

**Tech Stack:** Rust 1.83.0; the existing Davinci runtime, serde/serde_json, SHA-256, Ratatui/Crossterm, and fixture-driven inline Rust tests. Additional backend decisions are stated explicitly below.

**Spec:** [Structured Clarification / Decision Interview — supplied source specification](2026-09-07-davinci-feature-specs/02-structured-clarification-decision-interview.md)

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

`living_plan.rs` contains `LivingPlan { revision, goal, evidence, assumptions, questions, steps, decisions, approved_revision, changes }`; current `decisions: BTreeMap<String, bool>` represents plan-step choices, not structured interviews. `LivingPlan::update` uses revision checking and denies unknown input fields, so the model cannot submit approval. `planning.rs::handle_plan_command` persists a proposed revision before changing live state; `restore_plan` clears blanket execution consent. Extend these boundaries rather than interpreting an existing Boolean as an interview answer.

## Scope, alternatives, and chosen approach

Choose additive structured decisions and a dedicated request tool. Free-form assistant questions lose state and provenance; overloading plan-step acceptance would let an answer masquerade as execution approval. Materiality is requested by the model but validated structurally by the host: a nonempty reason and relevant evidence references are mandatory. Do not claim a deterministic validator can prove that all possible investigation was exhausted. The planner prompt asks for investigation first, while the runtime enforces no implicit answer and bounded questioning.

**Dependencies and implementation order:** Feature 01 supplies the host interaction pattern. Feature 03 adds task prerequisite references; the core Living Plan decision store is independently testable first.

## Requirements traceability

| Requirement ID | Required result |
|---|---|
| F02-1 | Investigate before asking; require a materiality rationale and repository evidence references. |
| F02-2 | Provide 2–4 concrete choices where possible, an optional recommendation, and an appropriate custom-answer route. |
| F02-3 | Expose ask_user_question and persist explicit structured answer state in Living Plan. |
| F02-4 | Only an authenticated user selection creates AnsweredByUser; recommendation is not consent. |
| F02-5 | Preserve Open/AnsweredByUser/Deferred/Cancelled across resume and invalidate affected stale plan approvals. |

## Data and behavioral contracts

Add `DecisionQuestion { id, plan_revision, title, question, materiality, evidence_refs, options, recommended_option_id, allow_custom, state, answer }` and `DecisionAnswer { source: UserInteraction, choice_id: Option<String>, custom_text: Option<String>, answered_at_ms, host_event_id }`. States are exactly `Open`, `AnsweredByUser`, `Deferred`, `Cancelled`; old `LivingPlan.decisions` remains its existing step-approval map. `ask_user_question` accepts only question content, evidence references, and the expected plan revision; it cannot accept state, answer, actor identity, or approval fields.

Normal options number 2–4 and have stable IDs, concise explanations, and at most one recommendation. For genuinely non-enumerable choices, explicitly use a custom-answer-only question and document why options are unsuitable. Cap a question at 8 KiB, each custom answer at 8 KiB, and one outstanding material question per plan. A material answer revises the Living Plan and invalidates affected approvals; it never changes permission mode or task scope by itself. Deferral keeps affected work blocked when the decision is a prerequisite.

## User experience and mode behavior

The question card shows title, 2–4 explained choices when appropriate, a clearly nonbinding recommendation, custom entry, and why this is a material decision. The pending indicator names what work is blocked. Answering never toggles Plan Mode into execution.

## File ownership and task sequence

The task file lists are the implementation map. Register new agent modules in `crates/davinci-agent/src/runtime/mod.rs` only for runtime-owned functionality, new tools in the existing tool/capability registry, and new native TUI views in the existing view dispatcher. Do not introduce a second runtime, permission engine, or event loop. Treat all cross-feature edits to `turn.rs`, `runtime_host.rs`, `davinci_interactive.rs`, and TUI model/routing as sequential integration points, not parallel write scopes.

### Task 1: Typed question validation

**Covers:** F02-1, F02-2

**Test placement:** `crates/davinci-agent/src/decisions.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Create:** `crates/davinci-agent/src/decisions.rs` — decision types and schema validator
- **Modify:** `crates/davinci-agent/src/living_plan.rs` — add serde-defaulted structured decision field

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f02_question_bounds() {
    assert!(valid_question_shape(3, 1, true, true, false));
    assert!(!valid_question_shape(3, 2, true, true, false));
    assert!(!valid_question_shape(1, 0, true, true, false));
    assert!(valid_question_shape(0, 0, true, true, true));
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-agent --offline f02_question_bounds -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn valid_question_shape(options: usize, recommended: usize, material: bool,
    has_evidence: bool, custom_only: bool) -> bool {
    material && has_evidence && recommended <= 1
        && if custom_only { options == 0 && recommended == 0 } else { (2..=4).contains(&options) }
}
```

Use deny_unknown_fields on model input DTOs and unique option IDs. Resolve evidence IDs against the current plan/source store rather than accepting fabricated paths as verified facts. Store a question's evidence fingerprint and plan revision; stale evidence is displayed as stale and cannot justify silently applying a prior answer to new scope. Restrict question kinds to the spec's material choices, not routine low-risk implementation trivia.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: duplicate IDs; empty materiality; missing/stale evidence; fifth option; two recommendations; malformed custom-only exception; overlength Unicode input.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-agent/src/decisions.rs" "crates/davinci-agent/src/living_plan.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: structured-clarification task 1"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 2: Authenticated answer transitions

**Covers:** F02-3, F02-4, F02-5

**Test placement:** `crates/davinci-agent/src/decisions.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Extend prerequisite module:** `crates/davinci-agent/src/decisions.rs` — transition reducer and typed user answer entry point (introduced by F02 Task 1; do not recreate it)
- **Modify:** `crates/davinci-agent/src/planning.rs` — persist before state publication

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f02_recommendation_not_answer() {
    assert_eq!(decision_transition("Open", "recommend", false), Some("Open"));
    assert_eq!(decision_transition("Open", "answer", false), None);
    assert_eq!(decision_transition("Open", "answer", true), Some("AnsweredByUser"));
    assert_eq!(decision_transition("AnsweredByUser", "answer", true), None);
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-agent --offline f02_recommendation_not_answer -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn decision_transition(state: &str, event: &str, trusted_user: bool) -> Option<&'static str> {
    match (state, event, trusted_user) {
        ("Open", "recommend", _) => Some("Open"),
        ("Open", "answer", true) => Some("AnsweredByUser"),
        ("Open", "defer", true) => Some("Deferred"),
        ("Open", "cancel", _) => Some("Cancelled"),
        _ => None,
    }
}
```

The trusted_user Boolean in this reducer is supplied only after the host has authenticated the interaction; it is not a tool parameter. Use a private host capability for the real API. Validate choice membership or custom-answer eligibility, expected decision/plan revisions, and idempotency key. Persist the answer and new plan revision together before releasing waiting work. Correcting an answer creates a new revision and audit event rather than mutating history. Reopening Deferred requires an explicit host action and a fresh question revision.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: forged model answer; duplicate reply; persistence failure leaves Open; changed plan while user types; late answer after cancel; custom input cannot execute slash commands.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-agent/src/decisions.rs" "crates/davinci-agent/src/planning.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: structured-clarification task 2"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 3: Tool registration and bounded waiting

**Covers:** F02-1, F02-3, F02-4

**Test placement:** `crates/davinci-agent/src/tools.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Modify:** `crates/davinci-agent/src/tools.rs` — ask_user_question registration and execution adapter
- **Modify:** `crates/davinci-agent/src/runtime/capabilities.rs` — declare interaction capability
- **Modify:** `crates/davinci-agent/src/turn.rs` — suspend only dependent continuation
- **Modify:** `crates/davinci-agent/src/scheduler.rs` — serial host-interaction lane for clarification

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f02_waiting_mode() {
    assert_eq!(decision_wait(false, false, false), "decision_required");
    assert_eq!(decision_wait(true, false, false), "wait_for_user");
    assert_eq!(decision_wait(true, true, false), "deferred");
    assert_eq!(decision_wait(true, false, true), "cancelled");
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-agent --offline f02_waiting_mode -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn decision_wait(interactive: bool, deferred: bool, cancelled: bool) -> &'static str {
    if cancelled { "cancelled" } else if deferred { "deferred" }
    else if interactive { "wait_for_user" } else { "decision_required" }
}
```

Create a tool schema with question content only. Waiting must not hold a native mutex, worker slot that prevents the controlling UI from running, or an open transaction. Return explicit pending/deferred/cancelled outcomes rather than synthesizing a selected option. Under Plan Mode the question interaction is legal but any resulting effect still passes normal gates. Budget elapsed time remains visible; a timeout cannot answer. Parent cancellation resolves the outstanding interaction and preserves its state.

Explicitly put `ask_user_question` in the SERIAL scheduler lane, alongside other host-state interactions; classifying it as Read without this exception would make it eligible for a parallel worker thread. The RPC waiter is thread-local. The native extension UI queue is currently drained after a turn, so it cannot serve as a synchronous mid-turn question bridge. Use the existing host channel pattern with a distinct question payload and never wait on child `--print` stdin.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: parallel workers ask same prerequisite; no UI available; timeout; parent stop; one-question capacity; Plan Mode ask causes no mutation of repository or global permission settings. A question executes on the host interaction lane; a print child returns unavailable without waiting.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-agent/src/tools.rs" "crates/davinci-agent/src/runtime/capabilities.rs" "crates/davinci-agent/src/turn.rs" "crates/davinci-agent/src/scheduler.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: structured-clarification task 3"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 4: Decision modal and RPC adapters

**Covers:** F02-2, F02-3, F02-4

**Test placement:** `crates/davinci-tui/src/davinci/views/decision_modal.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Create:** `crates/davinci-tui/src/davinci/views/decision_modal.rs` — choice/custom editor UI
- **Modify:** `crates/davinci-coding-agent/src/davinci_interactive.rs` — typed decision event handling
- **Modify:** `crates/davinci-coding-agent/src/rpc.rs` — decision request/reply adapter

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f02_enter_required() {
    assert!(!commits_decision("highlight", true));
    assert!(!commits_decision("enter", false));
    assert!(commits_decision("enter", true));
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-tui --offline f02_enter_required -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn commits_decision(event: &str, answer_valid: bool) -> bool {
    event == "enter" && answer_valid
}
```

Reuse feature 01's exclusive input ownership, not its grant issuance. Render materiality and evidence behind an inspect action, mark recommendation in text, and make custom entry editable before confirmation. Distinguish defer from cancel. Do not pre-submit a recommended choice when a modal first opens or when a key is repeated. RPC answers require the issued request ID; older clients can use sequential select/input requests with identical semantics. Print mode emits decision_required with structured question data.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: arrow and digit selection not answer; resize preserves custom draft; Enter repeats once; Esc cancel; defer blocks prerequisites; forged RPC request ID; legacy TUI typed custom response.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-tui/src/davinci/views/decision_modal.rs" "crates/davinci-coding-agent/src/davinci_interactive.rs" "crates/davinci-coding-agent/src/rpc.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: structured-clarification task 4"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 5: Plan revisions, resume, and task prerequisites

**Covers:** F02-3, F02-4, F02-5

**Test placement:** `crates/davinci-agent/src/living_plan.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Modify:** `crates/davinci-agent/src/living_plan.rs` — answer impact and approval invalidation
- **Modify:** `crates/davinci-agent/src/planning.rs` — branch-aware restore
- **Modify:** `crates/davinci-agent/src/runtime/tasks.rs` — decision prerequisite references

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f02_answer_invalidates_approval() {
    assert_eq!(approval_after_decision(Some(7), true), None);
    assert_eq!(approval_after_decision(Some(7), false), Some(7));
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-agent --offline f02_answer_invalidates_approval -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn approval_after_decision(approved_revision: Option<u64>, material_change: bool) -> Option<u64> {
    if material_change { None } else { approved_revision }
}
```

Migrate old questions as unstructured Open entries only when there is an unambiguous stable mapping; never infer answers from assistant prose or Boolean step approvals. Resume historical explicit answers with provenance, while preserving existing restore_plan behavior that clears blanket execution consent. Project task prerequisites refer to decision ID and revision, not array index. Changed source that changes the decision's assumptions marks its applicability stale and asks for review; it does not rewrite the user answer.

Keep existing Boolean `LivingPlan.decisions` as per-step accept/reject only. Apply answer-linked plan revisions at a supported safe host boundary; do not bypass `handle_plan_command` busy-runtime checks. On resume, answers may survive but blanket execution approval remains cleared by `restore_plan`.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: old session without decision field; branch fork; compacted history; answer after plan edit; deferred required decision keeps task blocked; user correction invalidates only dependent plan decisions.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-agent/src/living_plan.rs" "crates/davinci-agent/src/planning.rs" "crates/davinci-agent/src/runtime/tasks.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: structured-clarification task 5"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

## Failure, security, and recovery matrix

| Failure | Result |
|---|---|
| Unsupported interaction in print/RPC | Structured decision_required, not automatic recommended option. |
| Stale question or unknown evidence | Reject answer application; retain text for user review without authorization. |
| Duplicate/replayed reply | Idempotent same result or explicit conflict; no extra plan revision. |
| Disk error | Keep prior persisted state; task remains blocked. |
| Prompt injection in custom answer | Treat as user data in this decision field, not a new command or permission grant. |

## Persistence, migration, and rollback

Add serde defaults and keep existing step decisions intact. Unknown future decision schema must not become answered. Rollback can retain structured entries as opaque session metadata, but must not run tasks whose required decisions cannot be interpreted. Do not delete user answer history.

## End-to-end acceptance and release gates

Run a canned planning interview that selects SQLite, persists the explicit answer, resumes the session, and shows both the answer and its evidence. A recommendation-only transcript, a model tool payload containing answer fields, and a timeout must all fail to create AnsweredByUser. Correct the answer and prove dependent approval/evidence applicability is invalidated without deleting the prior audit record.

- [ ] Run the feature's listed inline tests with the offline fixture environment and prove each filter selects tests. Record test names, counts, exit codes, and source fingerprint.
- [ ] Run `rtk cargo fmt --check`, relevant crate tests, and then `rtk cargo test --workspace --offline` and `rtk cargo clippy --workspace --all-targets --offline -- -D warnings`. A pre-existing failure is reported, not silently waived or attributed to this feature.
- [ ] Test interactive Ratatui, legacy TUI, print/JSON, and RPC adapters where this plan adds interactions. Noninteractive uncertainty never becomes approval.
- [ ] Verify old session/config fixtures still load, source changes invalidate affected evidence, and cancellation leaves an explainable recoverable state.
- [ ] Update the user-facing docs for this feature and the relevant help/command discovery entries in the final integration task. Keep internal lifecycle operations internal.
- [ ] For later executable delivery, follow `CLAUDE.md`: resolve the actual launched executable, build offline, back up before replacing, compare build/installed hashes, and distinguish automated UI evidence from a physical keyboard check. Do not interrupt an active user's session to unlock a binary.

**Execution exit condition:** all source-spec requirements above have a passing test or a named, explicit manual verification gap; no missing required proof is described as complete. The roadmap's final integration gate applies even when this feature's own tests pass.

## References

Local code references above are anchored to repository HEAD `9c42820`. The supplied spec is preserved as Markdown in this bundle. [Repository audit](2026-09-07-00-repository-audit.md), [shared contracts](2026-09-07-00-shared-runtime-contracts.md), and [cross-feature validation matrix](2026-09-07-00-requirements-and-validation.md) explain the verified baseline and proposed additions.


