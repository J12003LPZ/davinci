# Whole-task Budgets and Loop Detection Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bound the entire task's resources across every worker and retry while detecting repeated work without new evidence and preserving verification capacity.

**Architecture:** Attach a durable root ResourceLedger to RuntimeHandle and descendant processes. Reserve capacity atomically before dispatch, reconcile actual usage idempotently, and feed deterministic progress signals into a separate watchdog state machine.

**Tech Stack:** Rust 1.83.0; the existing Davinci runtime, serde/serde_json, SHA-256, Ratatui/Crossterm, and fixture-driven inline Rust tests. Additional backend decisions are stated explicitly below.

**Spec:** [Whole-task Budgets and Loop Detection — supplied source specification](2026-09-07-davinci-feature-specs/09-whole-task-budgets-loop-detection.md)

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

The runtime has capacity/cancellation primitives and per-run stats; graph config/resource snapshots and security-scan budgets track subsystem work. Provider retries already contribute actual additional-attempt statistics and abort-aware sleeps. These are useful adapters, but a graph-local or child-local cap is not a shared root budget. Token-governor dedupe optimizes repeated output and must not be mistaken for semantic loop detection; its retrieval/pruning guarantees remain unchanged.

## Scope, alternatives, and chosen approach

Choose one root ledger with bounded reservations and conservative reconciliation. Passing a fresh full budget to each child is rejected. A soft loop warning and a hard resource limit are different: Continue can dismiss a watchdog pause within remaining budget, not bypass a hard ceiling. Allow user-authorized ceiling changes through a revisioned host action, never from model text. Cost is unknown when rates or usage are unknown; zero is not a substitute. Estimates may guide reservations, but limits must clearly state measurement granularity and bounded unavoidable in-flight exposure.

**Dependencies and implementation order:** Requires 03 durable command infrastructure and 07 process cancellation integration. Lands before proof-backed completion 06 is enabled and before graph fork/retry controls 13–14. Watchdog observation can ship read-only before pause enforcement.

## Requirements traceability

| Requirement ID | Required result |
|---|---|
| F09-1 | Share token/time/concurrency/retry/cost accounting across parent, descendants, reviews, verification, and provider retries. |
| F09-2 | Show cost as unknown where pricing/usage is unavailable; do not reset budgets on recursion or retry. |
| F09-3 | Detect repeated unchanged reads/commands, oscillating edits, and failed tests without a new experiment. |
| F09-4 | Offer Continue within remaining budget, return to Plan Mode with evidence, or stop preserving a checkpoint. |
| F09-5 | Reserve verification and handoff capacity and enforce limits under concurrency, crash/replay, and unknown usage. |

## Data and behavioral contracts

`ResourceBudget { root_run_id, revision, token_ceiling, deadline, max_concurrency, retry_ceiling, cost_ceiling: Option<Money>, verification_reserve, handoff_reserve }`; `Reservation { id, attempt_id, purpose: Implementation | Verification | Handoff, max_input, max_output, remaining_time, worker_slot }`; `UsageReceipt { attempt_id, provider_usage, estimated_usage, cost: Known | Unknown, finished_at }`. Persist reservations before worker dispatch and reconcile once by attempt ID. Unknown usage keeps a conservative reserved charge until explicitly reconciled, never automatic zero/refund.

`ProgressObservation { action_fingerprint, input_manifest, output_digest, hypothesis_id, hypothesis_evidence_refs, edit_state_hash, test_failure_signature }`. Detect unchanged reads, repeated command/input/result triples, A→B→A edits, repeated test failure without a changed experiment, and child attempts missing the root ledger. Start with three equivalent attempts for warning, then a bounded pause before further repetition; thresholds are proposed and configurable, not proof of cognitive sameness. A changed label alone is not new evidence. Root elapsed time remains cumulative across pause/resume; restarting a process does not reset deadline or retry usage. Example mockup values (120k tokens, 15 minutes, four workers, five retries) are illustrative, not defaults.

## User experience and mode behavior

The budget view separates actual usage, outstanding reservations, protected reserves, and unknown cost. A progress warning names the repeated command/failure and offers Continue, Plan Mode, or Stop. These choices never conceal the remaining hard limits.

## File ownership and task sequence

The task file lists are the implementation map. Register new agent modules in `crates/davinci-agent/src/runtime/mod.rs` only for runtime-owned functionality, new tools in the existing tool/capability registry, and new native TUI views in the existing view dispatcher. Do not introduce a second runtime, permission engine, or event loop. Treat all cross-feature edits to `turn.rs`, `runtime_host.rs`, `davinci_interactive.rs`, and TUI model/routing as sequential integration points, not parallel write scopes.

### Task 1: Root ledger and reserve classes

**Covers:** F09-1, F09-2, F09-5

**Test placement:** `crates/davinci-agent/src/runtime/budget.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Create:** `crates/davinci-agent/src/runtime/budget.rs` — budget/ledger/reservation types
- **Modify:** `crates/davinci-agent/src/runtime/mod.rs` — shared root budget handle
- **Modify:** `crates/davinci-agent/src/runtime/capacity.rs` — atomic slot reservations

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f09_implementation_preserves_reserve() {
    assert_eq!(implementation_available(100, 50, 20, 10), Some(20));
    assert_eq!(implementation_available(100, 90, 20, 10), None);
    assert_eq!(implementation_available(100, u64::MAX, 20, 10), None);
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-agent --offline f09_implementation_preserves_reserve -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn implementation_available(total: u64, charged: u64, verify: u64, handoff: u64) -> Option<u64> {
    total.checked_sub(charged)?.checked_sub(verify)?.checked_sub(handoff)
}
```

Use checked integer arithmetic and explicit units; tokens and currency minor units must not be floats. Apply outstanding reservations as well as charged actual usage when deciding availability. The root controller owns the ledger and gives children bounded authenticated leases; no child may mint a new independent root for the same task. Define configurable initial reserves (proposed 15% verification and 5% handoff as a starting policy, adjusted by known required checks) and reject ceilings too small to satisfy required reserves. Slot acquisition/release and lease expiry must be atomic.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: concurrent reservation race; huge integer overflow; zero budget; verification larger than ceiling; recursion without root lease; slot release after failure; verification cannot spend handoff reserve silently.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-agent/src/runtime/budget.rs" "crates/davinci-agent/src/runtime/mod.rs" "crates/davinci-agent/src/runtime/capacity.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: whole-task-budgets-loop-detection task 1"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 2: Actual usage reconciliation and retry charging

**Covers:** F09-1, F09-2, F09-5

**Test placement:** `crates/davinci-agent/src/runtime/budget.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Extend prerequisite module:** `crates/davinci-agent/src/runtime/budget.rs` — idempotent settlement and unknown usage (introduced by F09 Task 1; do not recreate it)
- **Modify:** `crates/davinci-agent/src/turn.rs` — provider/tool attempt boundaries
- **Modify:** `crates/davinci-agent/src/stats.rs` — derive stats from receipts
- **Modify:** `crates/davinci-coding-agent/src/native_extensions/ecosystem/resource.rs` — graph child aggregation adapter

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f09_charge_once() {
    let mut seen = std::collections::BTreeSet::new();
    assert!(settle_once(&mut seen, "attempt-1"));
    assert!(!settle_once(&mut seen, "attempt-1"));
    assert!(settle_once(&mut seen, "attempt-2"));
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-agent --offline f09_charge_once -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn settle_once(seen: &mut std::collections::BTreeSet<String>, attempt: &str) -> bool {
    seen.insert(attempt.to_owned())
}
```

The durable implementation commits attempt receipt and resource delta in one ledger transaction; the set helper only defines idempotency semantics. Charge every actual provider retry, failed/cancelled attempt with measured usage, graph review, background learning work attributable to the task, and verification child. Separate cache-read/write tokens according to provider receipt without double-counting totals. When cancellation prevents exact final usage, keep an Unknown cost flag and a conservative reserved exposure. Release unused reservations only with proof of not-started or a reconciled terminal result. Monotonic elapsed time within a process plus persisted wall-clock deadline guards resume; clock rollback cannot extend a task indefinitely.

Charge each actual provider attempt when it starts, including a real retry; a canceled retry backoff does not itself consume a new model-attempt charge. Denied tool actions and returned tool-ledger results do not create executed-tool usage. Retain committed and unknown usage from failed/superseded graph attempts and prove nonzero resume totals do not double count replayed artifacts.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: crash before dispatch/after dispatch/before receipt; duplicate callback; retry with no usage; cached tokens; review child; late usage after stop; missing pricing; provider overrun beyond reservation visibly blocks further work.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-agent/src/runtime/budget.rs" "crates/davinci-agent/src/turn.rs" "crates/davinci-agent/src/stats.rs" "crates/davinci-coding-agent/src/native_extensions/ecosystem/resource.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: whole-task-budgets-loop-detection task 2"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 3: Progress fingerprints and deterministic loop signals

**Covers:** F09-3

**Test placement:** `crates/davinci-agent/src/runtime/progress_watchdog.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Create:** `crates/davinci-agent/src/runtime/progress_watchdog.rs` — bounded observation window and loop classifier
- **Modify:** `crates/davinci-agent/src/turn.rs` — record finalized tool/experiment observations
- **Extend prerequisite module:** `crates/davinci-agent/src/runtime/evidence.rs` — new evidence linkage (introduced by F06 Task 1; do not recreate it)

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f09_repeated_work() {
    assert!(repeat_without_progress(&[("a", "i", "o"), ("a", "i", "o"), ("a", "i", "o")], false));
    assert!(!repeat_without_progress(&[("a", "i", "o"), ("a", "j", "o"), ("a", "i", "o")], false));
    assert!(!repeat_without_progress(&[("a", "i", "o"); 3], true));
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-agent --offline f09_repeated_work -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn repeat_without_progress(observations: &[(&str, &str, &str)], new_evidence: bool) -> bool {
    !new_evidence && observations.len() >= 3
        && observations[observations.len()-3..].windows(2).all(|pair| pair[0] == pair[1])
}
```

Normalize only non-semantic noise in command outputs using explicit profiles; do not erase changes that indicate a new failure. Include canonical tool/argv, actual input manifest, and result digest. Maintain separate detectors for repeated unchanged reads, equivalent test failures, and edit-state cycles; cap history at 128 observations per task. A new hypothesis is progress only with changed experiment inputs or linked new evidence, not a different model sentence. Exempt necessary recovery retrieval after pruning from being automatically treated as a stuck read loop, while still charging its resources. Watchdog warnings contain observed facts rather than hidden-reasoning analysis.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: oscillating A→B→A edits; new source despite same command; same failure new experiment; noisy timestamps; retrieval after pruning; legitimately polling job_output; long read-only research with new evidence.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-agent/src/runtime/progress_watchdog.rs" "crates/davinci-agent/src/turn.rs" "crates/davinci-agent/src/runtime/evidence.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: whole-task-budgets-loop-detection task 3"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 4: Hard limits, safe pause, and authorized choices

**Covers:** F09-4, F09-5

**Test placement:** `crates/davinci-agent/src/runtime/budget.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Extend prerequisite module:** `crates/davinci-agent/src/runtime/budget.rs` — dispatch gate and ceiling updates (introduced by F09 Task 1; do not recreate it)
- **Extend prerequisite module:** `crates/davinci-agent/src/runtime/progress_watchdog.rs` — pause/continue/plan/stop reducer (introduced by F09 Task 3; do not recreate it)
- **Modify:** `crates/davinci-coding-agent/src/davinci_interactive.rs` — watchdog interaction bridge

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f09_continue_cannot_bypass_ceiling() {
    assert_eq!(budget_decision(0, true, "continue"), "hard_stop");
    assert_eq!(budget_decision(10, true, "continue"), "continue");
    assert_eq!(budget_decision(10, true, "plan"), "return_to_plan");
    assert_eq!(budget_decision(10, true, "stop"), "stop_checkpoint");
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-agent --offline f09_continue_cannot_bypass_ceiling -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn budget_decision(remaining: u64, paused: bool, choice: &str) -> &'static str {
    if remaining == 0 { return "hard_stop"; }
    if !paused { return "running"; }
    match choice { "continue" => "continue", "plan" => "return_to_plan", "stop" => "stop_checkpoint", _ => "paused" }
}
```

Before new work, enforce remaining tokens, deadline, concurrency, retry count, and any enforceable known-cost ceiling. Interrupt in-flight calls at supported abort boundaries on deadline and label any unmeasured exposure. A watchdog pause stops further dispatch while preserving an in-flight transaction's recovery invariants; return-to-plan quiesces mutation workers before switching mode. Stop writes the latest available checkpoint/evidence summary within reserved handoff capacity. User ceiling increases require current revision and host authorization; model Continue never raises limits. Noninteractive hosts return budget_exhausted or progress_review_required rather than hanging.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: deadline during provider backoff; no checkpoint budget; pause while writer active; watchdog Continue with hard ceiling reached; user raises only tokens not time; unknown cost with hard monetary cap requires conservative refusal or explicit token-only policy.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-agent/src/runtime/budget.rs" "crates/davinci-agent/src/runtime/progress_watchdog.rs" "crates/davinci-coding-agent/src/davinci_interactive.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: whole-task-budgets-loop-detection task 4"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

### Task 5: Budget visibility, subsystem adapters, and regression metrics

**Covers:** F09-1, F09-2, F09-3, F09-4, F09-5

**Test placement:** `crates/davinci-coding-agent/src/davinci_surfaces.rs` — inline `#[cfg(test)]` module; production helper and adapters stay in the responsible modules listed below.

**Files and responsibilities:**
- **Modify:** `crates/davinci-coding-agent/src/davinci_surfaces.rs` — root ledger snapshot
- **Create:** `crates/davinci-tui/src/davinci/views/budget.rs` — usage/reserve/watchdog view
- **Modify:** `crates/davinci-coding-agent/src/native_extensions/graph/controller.rs` — inherit root reservations
- **Modify:** `crates/davinci-coding-agent/src/native_extensions/security_scan/budget.rs` — root-compatible child caps

**Interfaces:** Use only validated host-owned state and the feature contracts defined above; do not accept authority from model-generated JSON. The function in Step 3 is a new contract helper exported only as widely as its adapter needs; its exact signature is the producer contract for the test in Step 1.

- [ ] **Step 1 — Add the failing contract test** in the indicated source file's inline test module. Implement the named fixture cases below in the same task.

```rust
#[test]
fn f09_unknown_cost() {
    assert_eq!(cost_label(None), "unknown".to_string());
    assert_eq!(cost_label(Some(0)), "0 minor units".to_string());
    assert_eq!(cost_label(Some(12)), "12 minor units".to_string());
}
```

- [ ] **Step 2 — Prove the test is red.**

```text
rtk cargo test -p davinci-coding-agent --offline f09_unknown_cost -- --nocapture
```

Expected before implementation: a missing new symbol/contract or the stated assertion fails. A toolchain, dependency-cache, unrelated baseline failure, or zero selected tests does not establish the intended red state; record it separately and isolate the feature assertion.

- [ ] **Step 3 — Implement the contract and its production integration.**

```rust
pub fn cost_label(cost_minor_units: Option<u64>) -> String {
    cost_minor_units.map(|v| format!("{v} minor units")).unwrap_or_else(|| "unknown".to_owned())
}
```

Show consumed plus reserved tokens, root elapsed/deadline, active workers/capacity, attempts/retries, known or unknown cost, and protected verification/handoff reserves. Subsystem budgets remain tighter local caps but can never exceed the root lease. Compare totals with existing stats/resource snapshots using deterministic fixtures to avoid double counting. Add a regression corpus for repeated-work detection, including false-positive cases and no-new-evidence sequences. Report false positives and observed savings; do not promise a performance percentage without measurements. Update budget and control docs with the distinction between estimates, billed usage, and enforced caps.

RequestCapacity is an existing concurrency limiter, not the root budget ledger. Adapt graph milestone-scaled worker limits separately from global cost/deadline totals. Keep legacy zero-as-unlimited parsing at the configuration boundary and convert to typed unlimited values internally; never interpret an exhausted finite grant of zero as unlimited.

- [ ] **Step 4 — Verify the targeted deliverable.** Re-run Step 2 and require at least one selected test, then run the related module tests. Required additional scenarios: parent+twochildren+review+retry sums once; checkpoint rewind keeps cumulative use; multiple graph forks share root; cost unknown displayed; no pricing mutation; stale UI revision; token governor metrics not confused with budget consumption.

- [ ] **Step 5 — Review and checkpoint only this task.** Check `git diff` and the index for pre-existing hunks; stage only the feature changes, using patch staging for shared files. The following paths are the maximum staging scope, not permission to stage unrelated edits.

```text
rtk git add -- "crates/davinci-coding-agent/src/davinci_surfaces.rs" "crates/davinci-tui/src/davinci/views/budget.rs" "crates/davinci-coding-agent/src/native_extensions/graph/controller.rs" "crates/davinci-coding-agent/src/native_extensions/security_scan/budget.rs"
rtk git diff --cached --stat
rtk git commit -m "feat: whole-task-budgets-loop-detection task 5"
```

The task is complete only when its integration paths and required cases pass; a green pure helper alone is insufficient.

## Failure, security, and recovery matrix

| Failure | Result |
|---|---|
| Missing final provider usage | Conservative exposure retained, cost unknown. |
| Ledger append failure | No new dispatch; previously running work reconciled. |
| Child lacks root lease | Reject spawn or attach authenticated bounded lease. |
| Loop detector false positive | User can continue within remaining resources; evidence recorded. |
| Deadline/ceiling exhausted | Stop dispatch, cancel safely, preserve handoff; no silent reset on resume. |

## Persistence, migration, and rollback

Legacy sessions receive an explicit new root ledger only for future work; prior usage is unknown, not zero. Resumed current-ledger tasks preserve cumulative charges and original deadlines. Unknown schema blocks dispatch. Rollback may disable warnings but must not remove established hard budget enforcement.

## End-to-end acceptance and release gates

Use an offline fixture with a parent, two workers, a retry, a reviewer, and verification. All reservations and receipts must reconcile to one root total, including a crash/replay and duplicate receipt. Consume implementation allowance and prove verification/handoff still have protected capacity. Trigger each watchdog signal and a legitimate repeated-read recovery case; user Continue cannot exceed a hard ceiling.

- [ ] Run the feature's listed inline tests with the offline fixture environment and prove each filter selects tests. Record test names, counts, exit codes, and source fingerprint.
- [ ] Run `rtk cargo fmt --check`, relevant crate tests, and then `rtk cargo test --workspace --offline` and `rtk cargo clippy --workspace --all-targets --offline -- -D warnings`. A pre-existing failure is reported, not silently waived or attributed to this feature.
- [ ] Test interactive Ratatui, legacy TUI, print/JSON, and RPC adapters where this plan adds interactions. Noninteractive uncertainty never becomes approval.
- [ ] Verify old session/config fixtures still load, source changes invalidate affected evidence, and cancellation leaves an explainable recoverable state.
- [ ] Update the user-facing docs for this feature and the relevant help/command discovery entries in the final integration task. Keep internal lifecycle operations internal.
- [ ] For later executable delivery, follow `CLAUDE.md`: resolve the actual launched executable, build offline, back up before replacing, compare build/installed hashes, and distinguish automated UI evidence from a physical keyboard check. Do not interrupt an active user's session to unlock a binary.

**Execution exit condition:** all source-spec requirements above have a passing test or a named, explicit manual verification gap; no missing required proof is described as complete. The roadmap's final integration gate applies even when this feature's own tests pass.

## References

Local code references above are anchored to repository HEAD `9c42820`. The supplied spec is preserved as Markdown in this bundle. [Repository audit](2026-09-07-00-repository-audit.md), [shared contracts](2026-09-07-00-shared-runtime-contracts.md), and [cross-feature validation matrix](2026-09-07-00-requirements-and-validation.md) explain the verified baseline and proposed additions.
