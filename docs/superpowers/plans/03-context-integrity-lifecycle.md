# Context Integrity and Session Lifecycle Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Prevent silent context loss, authority confusion, stale instructions, broken recall and wrong session state across turns, compaction, branches, resume and shutdown.

**Architecture:** Instrument the effective provider projection first, reproduce each suspected defect against the current branch, then fix provenance, overlay admission, scoping, source recovery, compaction and persistence with mode-parity regression gates.

**Tech Stack:** Rust agent/context VM, JSONL session journal, provider compaction, source hashes, deterministic eval fixtures.

**Spec:** [`../specs/2026-10-17-davinci-reliability-and-zero-maintenance-design.md`](../specs/2026-10-17-davinci-reliability-and-zero-maintenance-design.md)

**Dependencies / execution order:** Can start in parallel with Plan 02, but scoped project instructions and session hooks must honor Plan 01 trust contracts.

## Global Constraints

**Path shorthand:** Any unqualified `src/...` or `tests/...` in this plan is relative to `crates/davinci-agent/`. Explicit `crates/...` paths are repo-root-relative. For the Design Studio plan, `design-ui/...` is relative to `crates/davinci-coding-agent/`.

- This is a plan, not an implementation. Start from a clean isolated worktree of the exact target `main` commit. Confirm historical audit findings still reproduce, especially after recent compaction changes.
- Rust workspace/pinned toolchains and existing feature flags, permission policy, sandbox, provider identity, durable stores and cancellation must stay intact. No live package installs, cloud requests or data mutations without explicit operator approval.
- Local JSONL remains authoritative. Preserve disabled-mode parity, existing slash commands, recovery safety and provenance; no mandatory Docker/Qdrant/SQLite.
- New tasks must use red-green TDD, targeted verification, one purpose per commit, reviewer sign-off and rollback/feature gate before default enablement.

## Review Focus

- Empty summarizer output must not delete history — Task 2.
- Internal user-role reminder must not become user authority — Task 4.
- Corrected fact and excluded memory must not leak into next provider request — Task 5.
- Old screenshots and huge single-line sources remain recoverable — Task 9.
- Session switch must never keep previous project/session rules — Task 8/11.

---

### Task 1: Establish provider-request context trace and lifecycle harness

**Files:** Modify: `crates/davinci-agent/src/prepared_context.rs`, `src/lib.rs`; Create: `crates/davinci-agent/tests/context_integrity_contract.rs`

**Interfaces:** Produces `ContextTrace`, `ContextItemReceipt`; consumed by tests and `/context` debugging.

- [ ] **Step 1 — Write the failing regression** `context_trace_reports_selected_origin_and_omissions`. Assert: A diagnostic trace binds request/branch identity and every selected/omitted source to trusted origin, mandatory status, freshness, budget reason and recovery reference; `/context` reports deduplicated instruction-file contributions matching the real provider prompt; no raw secrets dumped..
- [ ] **Step 2 — Prove red on baseline.** Run `cargo test -p davinci-agent context_trace_reports_selected_origin_and_omissions -- --nocapture`. Expected: test fails for the identified behavior (or mark as already fixed and report evidence; never make an unnecessary fix).
- [ ] **Step 3 — Implement one targeted change.** Add debug-only `ContextTrace` DTO behind explicit opt-in; capture exact final provider projection, not speculative intermediate state. Align `context_usage.rs` memory-file accounting with the system prompt's deduplicated inclusion and add a zero-raw-source diagnostic when appropriate. Use fixtures in off/hybrid/active/shadow.
- [ ] **Step 4 — Verify green and regression boundaries.** Re-run `cargo test -p davinci-agent context_trace_reports_selected_origin_and_omissions -- --nocapture`; expected: pass. Also run the module-specific suite, `cargo fmt --all -- --check`, and relevant Clippy targets. Save test transcript/receipt and verify no new permissions or default behavior changed.
- [ ] **Step 5 — Review and commit.** Reviewer inspects exact code and config/data compatibility; commit as `feat(context): trace effective provider projection`. Do not proceed when the gate is red.

### Task 2: Reject empty compaction summaries and no-op budget regressions

**Files:** Modify: `crates/davinci-agent/src/compaction.rs`, `src/lib.rs`; Test: `crates/davinci-agent/tests/context_integrity_contract.rs`

**Interfaces:** Produces `CompactionValidation { fits, reduces, has_summary }` and explicit recovery result.

- [ ] **Step 1 — Write the failing regression** `compact_empty_or_larger_summary_never_commits`. Assert: A success-shaped but blank summarizer reply, non-finite budget or summary larger than original context cannot replace history; original messages and branch journal unchanged..
- [ ] **Step 2 — Prove red on baseline.** Run `cargo test -p davinci-agent compact_empty_or_larger_summary_never_commits -- --nocapture`. Expected: test fails for the identified behavior (or mark as already fixed and report evidence; never make an unnecessary fix).
- [ ] **Step 3 — Implement one targeted change.** Reproduce first against current `main` (which recently changed compaction). Validate meaningful nonempty content and post-compact token admission/reduction at transaction boundary; no-op has explicit non-destructive result.
- [ ] **Step 4 — Verify green and regression boundaries.** Re-run `cargo test -p davinci-agent compact_empty_or_larger_summary_never_commits -- --nocapture`; expected: pass. Also run the module-specific suite, `cargo fmt --all -- --check`, and relevant Clippy targets. Save test transcript/receipt and verify no new permissions or default behavior changed.
- [ ] **Step 5 — Review and commit.** Reviewer inspects exact code and config/data compatibility; commit as `fix(context): guard compaction commit`. Do not proceed when the gate is red.

### Task 3: Bound summarization input and auto-compaction fallback

**Files:** Modify: `crates/davinci-agent/src/compaction.rs`, `src/turn.rs`; Test: `crates/davinci-agent/tests/context_integrity_contract.rs`

**Interfaces:** Produces `build_bounded_summary_request(...) -> Result<..., ContextBudgetError>` and `admit_after_compaction(...)`.

- [ ] **Step 1 — Write the failing regression** `oversized_history_compacts_or_errors_without_dispatch`. Assert: A full summarizer prompt including custom messages, tool arguments and task context fits provider budget or fails recoverably; after auto-compact failure no oversized ordinary model request is emitted..
- [ ] **Step 2 — Prove red on baseline.** Run `cargo test -p davinci-agent oversized_history_compacts_or_errors_without_dispatch -- --nocapture`. Expected: test fails for the identified behavior (or mark as already fixed and report evidence; never make an unnecessary fix).
- [ ] **Step 3 — Implement one targeted change.** Calculate full request admission; use the provider-visible pruned view rather than silently reintroducing raw pruned tool output, with explicit source references to allow recall. Implement hierarchical/bounded source-aware summarization where required; capped emergency truncation only for eligible evidence, never authoritative user requirements.
- [ ] **Step 4 — Verify green and regression boundaries.** Re-run `cargo test -p davinci-agent oversized_history_compacts_or_errors_without_dispatch -- --nocapture`; expected: pass. Also run the module-specific suite, `cargo fmt --all -- --check`, and relevant Clippy targets. Save test transcript/receipt and verify no new permissions or default behavior changed.
- [ ] **Step 5 — Review and commit.** Reviewer inspects exact code and config/data compatibility; commit as `fix(context): enforce summary and dispatch budgets`. Do not proceed when the gate is red.

### Task 4: Preserve provenance for synthetic user-role and custom events

**Files:** Modify: `crates/davinci-agent/src/runtime/context_vm/events.rs`, `src/compaction.rs`; Test: `crates/davinci-agent/tests/context_integrity_contract.rs`

**Interfaces:** Produces `classify_context_origin(&ChatMessage) -> ProvenanceKind` shared by fold/serializer.

- [ ] **Step 1 — Write the failing regression** `internal_notice_is_not_user_decision`. Assert: Background notices, capability reminders and custom messages are categorized as internal/tool evidence, not UserDecision, even if role=user in provider transcript; true user requests remain authoritative..
- [ ] **Step 2 — Prove red on baseline.** Run `cargo test -p davinci-agent internal_notice_is_not_user_decision -- --nocapture`. Expected: test fails for the identified behavior (or mark as already fixed and report evidence; never make an unnecessary fix).
- [ ] **Step 3 — Implement one targeted change.** Classify by verified origin markers/custom type before role; preserve typed provenance in legacy compaction serializing, and treat unrecognized messages as non-authoritative.
- [ ] **Step 4 — Verify green and regression boundaries.** Re-run `cargo test -p davinci-agent internal_notice_is_not_user_decision -- --nocapture`; expected: pass. Also run the module-specific suite, `cargo fmt --all -- --check`, and relevant Clippy targets. Save test transcript/receipt and verify no new permissions or default behavior changed.
- [ ] **Step 5 — Review and commit.** Reviewer inspects exact code and config/data compatibility; commit as `fix(context): honor true message origin`. Do not proceed when the gate is red.

### Task 5: Apply correction, pin and exclusion overlays to active projection

**Files:** Modify: `crates/davinci-agent/src/lib.rs`, `src/runtime/context.rs`, `src/runtime/context_overlay.rs`, `src/prepared_context.rs`; Test: `crates/davinci-agent/tests/context_integrity_contract.rs`

**Interfaces:** Consumes `ContextOverlay`; produces `build_context_with_ledger_and_overlay(...)` and revision identity included in cache keys.

- [ ] **Step 1 — Write the failing regression** `active_context_honors_overlay_and_revision`. Assert: Corrected old fact not selected; explicitly excluded optional item omitted; pinned item prioritized; a changed overlay invalidates prepared request; mandatory policy cannot be excluded..
- [ ] **Step 2 — Prove red on baseline.** Run `cargo test -p davinci-agent active_context_honors_overlay_and_revision -- --nocapture`. Expected: test fails for the identified behavior (or mark as already fixed and report evidence; never make an unnecessary fix).
- [ ] **Step 3 — Implement one targeted change.** Pass an authorized overlay through `build_context_with_overlay` in the actual active context assembly; enforce non-user-modifiable mandatory items; align ID and correction maps with stable source refs.
- [ ] **Step 4 — Verify green and regression boundaries.** Re-run `cargo test -p davinci-agent active_context_honors_overlay_and_revision -- --nocapture`; expected: pass. Also run the module-specific suite, `cargo fmt --all -- --check`, and relevant Clippy targets. Save test transcript/receipt and verify no new permissions or default behavior changed.
- [ ] **Step 5 — Review and commit.** Reviewer inspects exact code and config/data compatibility; commit as `fix(context): apply overlays to real request`. Do not proceed when the gate is red.

### Task 6: Correct mandatory-source authority inference and untrusted markup

**Files:** Modify: `crates/davinci-agent/src/runtime/context.rs`; Test: `crates/davinci-agent/tests/context_integrity_contract.rs`

**Interfaces:** Produces `ContextAuthority::{Mandatory,User,Tool,Optional}` host-assigned, not source-declared.

- [ ] **Step 1 — Write the failing regression** `optional_source_named_mandatory_is_not_policy`. Assert: Optional `source="mandatory_looking_name"` cannot force mandatory admission; forged context tags remain escaped/source-labeled, never promoted into trusted policy..
- [ ] **Step 2 — Prove red on baseline.** Run `cargo test -p davinci-agent optional_source_named_mandatory_is_not_policy -- --nocapture`. Expected: test fails for the identified behavior (or mark as already fixed and report evidence; never make an unnecessary fix).
- [ ] **Step 3 — Implement one targeted change.** Replace substring-based mandatory logic with host-trusted authority provenance; escape/safely serialize source attributes and untrusted body. Recheck provenance at broker boundary.
- [ ] **Step 4 — Verify green and regression boundaries.** Re-run `cargo test -p davinci-agent optional_source_named_mandatory_is_not_policy -- --nocapture`; expected: pass. Also run the module-specific suite, `cargo fmt --all -- --check`, and relevant Clippy targets. Save test transcript/receipt and verify no new permissions or default behavior changed.
- [ ] **Step 5 — Review and commit.** Reviewer inspects exact code and config/data compatibility; commit as `fix(context): enforce trustworthy context authority`. Do not proceed when the gate is red.

### Task 7: Scope nested AGENTS.md to target file and refresh modified rules

**Files:** Modify: `crates/davinci-agent/src/context.rs`; integration: `crates/davinci-coding-agent/src/main.rs`, tool read/edit/write preparation; Test: `crates/davinci-agent/tests/context_integrity_contract.rs`

**Interfaces:** Produces `effective_project_instructions(cwd, targets, trust) -> ScopedInstructions` and cache revision.

- [ ] **Step 1 — Write the failing regression** `nested_target_rules_load_and_refresh_after_change`. Assert: When editing `crates/api/src/x.rs` the applicable `crates/api/AGENTS.md` is included; unrelated sibling instructions are not; changing applicable instruction file invalidates old provider prompt before next affected tool call..
- [ ] **Step 2 — Prove red on baseline.** Run `cargo test -p davinci-agent nested_target_rules_load_and_refresh_after_change -- --nocapture`. Expected: test fails for the identified behavior (or mark as already fixed and report evidence; never make an unnecessary fix).
- [ ] **Step 3 — Implement one targeted change.** Wire existing `load_context_files_for_targets` to allowed tool target paths and add path-hash-based invalidation. Never execute project resource until trust accepted.
- [ ] **Step 4 — Verify green and regression boundaries.** Re-run `cargo test -p davinci-agent nested_target_rules_load_and_refresh_after_change -- --nocapture`; expected: pass. Also run the module-specific suite, `cargo fmt --all -- --check`, and relevant Clippy targets. Save test transcript/receipt and verify no new permissions or default behavior changed.
- [ ] **Step 5 — Review and commit.** Reviewer inspects exact code and config/data compatibility; commit as `fix(context): honor scoped and fresh project rules`. Do not proceed when the gate is red.

### Task 8: Reset session-scoped plugin context on new/resume/fork/import

**Files:** Modify: `crates/davinci-agent/src/lib.rs`, `crates/davinci-coding-agent/src/main.rs`, `src/davinci_interactive.rs`; Test: `crates/davinci-agent/tests/context_integrity_contract.rs`

**Interfaces:** Produces `ContextLifetime::{Global,Workspace,Session}` with session lifecycle rebind.

- [ ] **Step 1 — Write the failing regression** `session_switch_discards_previous_start_context`. Assert: A plugin SessionStart note specific to session A never appears in session B; fresh applicable start hook runs only after trust gate; global durable settings remain..
- [ ] **Step 2 — Prove red on baseline.** Run `cargo test -p davinci-agent session_switch_discards_previous_start_context -- --nocapture`. Expected: test fails for the identified behavior (or mark as already fixed and report evidence; never make an unnecessary fix).
- [ ] **Step 3 — Implement one targeted change.** Tag context resources with session identity and lifetime; clear/recreate session-scoped entries atomically across all session switch entrypoints.
- [ ] **Step 4 — Verify green and regression boundaries.** Re-run `cargo test -p davinci-agent session_switch_discards_previous_start_context -- --nocapture`; expected: pass. Also run the module-specific suite, `cargo fmt --all -- --check`, and relevant Clippy targets. Save test transcript/receipt and verify no new permissions or default behavior changed.
- [ ] **Step 5 — Review and commit.** Reviewer inspects exact code and config/data compatibility; commit as `fix(context): isolate startup resources per session`. Do not proceed when the gate is red.

### Task 9: Preserve multimodal context and full source recovery

**Files:** Modify: `crates/davinci-agent/src/runtime/context_vm/{events,compiler,fold,retrieval,sources}.rs`, `src/lib.rs`; Test: `crates/davinci-agent/tests/context_vm_integrity.rs`

**Interfaces:** Produces `RetrievalCursor { source_ref, byte_offset }` and structured `RecoveredContext` with supported media.

- [ ] **Step 1 — Write the failing regression** `old_image_and_oversized_line_recover_exact_source`. Assert: After fold an image-containing user request retains retrievable actual image blocks or explicit unsupported diagnostic; active-mode ephemeral image/block input remains structured rather than being reduced to text; a 200KB single-line JSON is pageable bytewise to the exact original hash..
- [ ] **Step 2 — Prove red on baseline.** Run `cargo test -p davinci-agent old_image_and_oversized_line_recover_exact_source -- --nocapture`. Expected: test fails for the identified behavior (or mark as already fixed and report evidence; never make an unnecessary fix).
- [ ] **Step 3 — Implement one targeted change.** Add bounded image/artifact lookup keyed by trusted session references and byte-offset pagination to `retrieve_context`; retain or explicitly reject unsupported structured ephemeral content in `lib.rs`; no fabricated reconstruction from text excerpts.
- [ ] **Step 4 — Verify green and regression boundaries.** Re-run `cargo test -p davinci-agent old_image_and_oversized_line_recover_exact_source -- --nocapture`; expected: pass. Also run the module-specific suite, `cargo fmt --all -- --check`, and relevant Clippy targets. Save test transcript/receipt and verify no new permissions or default behavior changed.
- [ ] **Step 5 — Review and commit.** Reviewer inspects exact code and config/data compatibility; commit as `fix(context): recover full source and multimodal evidence`. Do not proceed when the gate is red.

### Task 10: Maintain correct deterministic state transitions and contradictions

**Files:** Modify: `crates/davinci-agent/src/runtime/context_vm/{reducer,fold}.rs`; Test: `crates/davinci-agent/tests/context_vm_integrity.rs`

**Interfaces:** Produces deterministic supersession guard and replayable transitions.

- [ ] **Step 1 — Write the failing regression** `later_user_correction_supersedes_earlier_goal`. Assert: User says use Redis then corrects to SQLite; after fold exactly SQLite is active while Redis is retired; assistant speculation cannot supersede user policy..
- [ ] **Step 2 — Prove red on baseline.** Run `cargo test -p davinci-agent later_user_correction_supersedes_earlier_goal -- --nocapture`. Expected: test fails for the identified behavior (or mark as already fixed and report evidence; never make an unnecessary fix).
- [ ] **Step 3 — Implement one targeted change.** Apply ordered evidence references, explicit correction lifecycle and source-bound transitions even with summarizer failure; bound ledger growth without discarding latest user authority.
- [ ] **Step 4 — Verify green and regression boundaries.** Re-run `cargo test -p davinci-agent later_user_correction_supersedes_earlier_goal -- --nocapture`; expected: pass. Also run the module-specific suite, `cargo fmt --all -- --check`, and relevant Clippy targets. Save test transcript/receipt and verify no new permissions or default behavior changed.
- [ ] **Step 5 — Review and commit.** Reviewer inspects exact code and config/data compatibility; commit as `fix(context): preserve corrections across fold`. Do not proceed when the gate is red.

### Task 11: Persist assistant records and durable resume boundaries transactionally

**Files:** Modify: `crates/davinci-agent/src/turn.rs`, `src/lib.rs`, `crates/davinci-session/src/jsonl_repo.rs`; Test: `crates/davinci-agent/tests/context_integrity_contract.rs`

**Interfaces:** Produces `persist_committed_turn(...) -> Result<Receipt, PersistenceError>` used at turn boundary.

- [ ] **Step 1 — Write the failing regression** `write_failure_surfaces_before_successful_turn_completion`. Assert: Injected append failure prevents successful final response claim, leaves original session readable; resume/branch switch restores only current branch and exact previous checkpoint. On long journals, indexed recovery must preserve semantics while bounded startup work is measured against full replay..
- [ ] **Step 2 — Prove red on baseline.** Run `cargo test -p davinci-agent write_failure_surfaces_before_successful_turn_completion -- --nocapture`. Expected: test fails for the identified behavior (or mark as already fixed and report evidence; never make an unnecessary fix).
- [ ] **Step 3 — Implement one targeted change.** Propagate selected ignored persistence results at commit sites; preserve existing torn-tail backup and transaction journal behavior. Benchmark 10k-entry resume, and add a verified optional indexed checkpoint only if restore evidence shows a real scaling problem.
- [ ] **Step 4 — Verify green and regression boundaries.** Re-run `cargo test -p davinci-agent write_failure_surfaces_before_successful_turn_completion -- --nocapture`; expected: pass. Also run the module-specific suite, `cargo fmt --all -- --check`, and relevant Clippy targets. Save test transcript/receipt and verify no new permissions or default behavior changed.
- [ ] **Step 5 — Review and commit.** Reviewer inspects exact code and config/data compatibility; commit as `fix(session): make context persistence visible`. Do not proceed when the gate is red.

### Task 12: Multi-mode 300-turn quality, branch and resume gate

**Files:** Create: `crates/davinci-evals/src/behavior/context_lifecycle.rs`; Modify: `docs/context-vm-measurements.md`; Test: `crates/davinci-evals`

**Interfaces:** Produces `ContextLifecycleScorecard`; marks each previously suspected audit defect confirmed/fixed/already fixed/unreproduced.

- [ ] **Step 1 — Write the failing regression** `context_lifecycle_no_authority_regression`. Assert: Across off/hybrid/active, manual/auto compaction, synthetic notices, branch, shutdown and cold resume, authoritative requirements and source references survive; record context tokens, latency and provider-cache deltas, no automatic default mode flip..
- [ ] **Step 2 — Prove red on baseline.** Run `cargo test -p davinci-evals context_lifecycle_no_authority_regression -- --nocapture`. Expected: test fails for the identified behavior (or mark as already fixed and report evidence; never make an unnecessary fix).
- [ ] **Step 3 — Implement one targeted change.** Build deterministic event fixture plus authorized opt-in live provider quality campaign; document offline vs live results separately and retain anonymized evidence.
- [ ] **Step 4 — Verify green and regression boundaries.** Re-run `cargo test -p davinci-evals context_lifecycle_no_authority_regression -- --nocapture`; expected: pass. Also run the module-specific suite, `cargo fmt --all -- --check`, and relevant Clippy targets. Save test transcript/receipt and verify no new permissions or default behavior changed.
- [ ] **Step 5 — Review and commit.** Reviewer inspects exact code and config/data compatibility; commit as `feat(evals): gate context mode with lifecycle evidence`. Do not proceed when the gate is red.

## Release checklist

- [ ] All task-specific failing tests were demonstrated against the baseline or explicitly documented as already fixed.
- [ ] Every task passes focused tests, formatting and relevant Clippy; existing test suite passes with no suppressed failures.
- [ ] Scenario-level tests and negative cases run on disposable workspaces with fake services where possible; live-system tests run only with authorized dependencies.
- [ ] Documentation, rollback procedure, opt-out and error reporting are updated; no credential, memory text or user path appears in broad logs.
- [ ] Merge behind a bounded feature gate, collect real-world diagnostics, then separately authorize default-on behavior.
