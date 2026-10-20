# DaVinci Coding-Success Quality — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Improve successful coding behavior and truthful completion without mandatory extra model calls or duplicate prompts.

**Architecture:** Extend existing verification receipts/completion hooks, debugging capability routing, Context VM and failure observations; change a behavior prompt only after a controlled failure demonstrates it would be useful. Keep safety gates deterministic.

**Tech Stack:** Rust 1.88, existing `davinci-agent` tests, `davinci-evals` behavior/debugging fixtures and JSON traces.

**Spec:** `docs/superpowers/specs/2026-09-14-davinci-harness-optimization-design.md`, `00-INDEX.md`.

## Global constraints

- No universal forced plan or mandatory review model call on small tasks. Keep the user's selected model and reasoning effort.
- A passing tool command must actually test the modified behavior; `--collect-only`, `--no-run`, masked status, unrelated tests or checks predating the last mutation are **not** verified completion.
- A prompt instruction is not deterministic evidence. Treat claimed and observed behaviors separately.

## Review focus

- Last check predates an edit; completed ≠ verified (B1).
- Reproducer missing due to environment; response must say *not reproduced*, not *fixed* (B2).
- Compaction omits a user prohibition or exact error; continuation cannot silently abandon it (B3).
- Repeated failing verification creates unlimited retry or changes the wrong root cause (B4).
- A failed tool request should become an explicit observation; no hidden permission promotion (B4).

---

### Task B1 — Seal completion after latest mutation

**Files:** `crates/davinci-agent/src/{completion.rs,turn.rs,verification.rs,verification_notice.rs}`, `src/verification/{acceptance.rs,discovery.rs}`, existing `tests/{inline_verification,completion_requirements,harness_requirement_completion}.rs`. Add `tests/final_mutation_verification_contract.rs` only if needed.

**Interfaces:** Consumes existing mutation generation, command receipts, changed paths and verification coverage; produces typed verified / partial / inconclusive / failed disposition. Reuse existing verification state and hooks.

- [ ] **Step 1 — Audit:** Trace mutation generation increments, verification scope detection, final response classification, hook continuation budget and saved receipt reconciliation.
- [ ] **Step 2 — Test:** `latest_edit_invalidates_old_passing_check`, `masked_exit_status_is_not_passing_evidence`, `tests_not_executed_cannot_prove_completion`, `unrelated_test_does_not_cover_changed_path`, `fresh_targeted_verification_can_seal_completion`, `denied_verification_yields_unknown_not_success`.
- [ ] **Step 3 — Run red:** `cargo test -p davinci-agent --test final_mutation_verification_contract --offline --locked` if created; run existing three targeted test suites as controls.
- [ ] **Step 4 — Implement when red:** Tighten existing verification-generation and receipt predicates; do not add a second completion gate. Preserve a legitimate final *partial/unverified* answer when checks cannot execute.
- [ ] **Step 5 — Green:** Run focused controls and the new tests. Assert absence of fabricated verification and at most the configured bounded correction turns.
- [ ] **Step 6 — Record:** Publish the completion-state truth table with fixture evidence; **commit only if authorized**.

### Task B2 — Behavioral debugging quality, not another debugging prompt

**Files:** `crates/davinci-agent/src/prompt/capabilities/{debugging.rs,router.rs,gate.rs}`, `crates/davinci-evals/fixtures/behavior/debugging/routing.json`, `crates/davinci-evals/fixtures/debugging-hard.json`, `crates/davinci-evals/fixtures/repos/debugging-*`, `crates/davinci-evals/src/behavior/{scorer.rs,runner.rs}`.

**Interfaces:** Uses existing debugging-capability gating and eval trace schema. Produces paired behavioral quality report. No product-code API is proposed at baseline.

- [ ] **Step 1 — Baseline:** Verify debugging capability trigger, non-trigger and versioned prompt bundle. Document the existing 7-step evidence chain and token budget; do not reword it just for Claude similarity.
- [ ] **Step 2 — Add independent cases:** Add fixtures for `reproduce_before_edit`, `error_swallowing_root_cause`, `stale_cache_wrong_layer`, `intermittent_race_reproduction`, `missing_dependency_unverified`, `do_not_mask_failed_test` with expected actions and independently executable acceptance commands. Use existing fixture structure, not an invented schema.
- [ ] **Step 3 — Freeze measurements:** Validate corpus/fixtures offline with `cargo run --offline --locked -p davinci-evals -- corpus verify`. Run only offline scorer/tests at this stage; authenticated runs need explicit approval.
- [ ] **Step 4 — Implement only if evidence justifies it:** Make a minimal versioned capability/router instruction change, or improve tool-result evidence visibility, but not both in the same A/B candidate. Preserve absence of mandatory extra model calls on trivial tasks.
- [ ] **Step 5 — Paired gate:** Using approved provider runs only, compare baseline and candidate on held-out debugging tasks with same model/effort/budget, record pass rate, repair iterations, wrong-file edits and verified completion. Reject cost-only wins that lose solved tasks.
- [ ] **Step 6 — Record/revert:** Keep candidate only if independently verified gain outweighs variance and costs; otherwise retain original prompt. No commit without approval.

### Task B3 — Continuity after compaction and resume

**Files:** `crates/davinci-agent/src/compaction.rs`, `src/runtime/context_vm/{fold.rs,reducer.rs,retrieval.rs,sources.rs}`, existing `tests/{context_vm_integrity,context_vm_durability,context_vm_replay,context_vm_retrieval}.rs`; add `tests/context_continuity_acceptance.rs` if uncovered.

**Interfaces:** Source-ref-backed Context VM is the authoritative history/replay system; prose summary is nonauthoritative. Reuse current fold/retrieve APIs.

- [ ] **Step 1 — Inspect:** Determine actual preservation and retrieval rules for user constraints, exact paths/errors, latest edits and test receipts; identify which facts must be pinned vs eligible for compaction.
- [ ] **Step 2 — Test:** `user_prohibition_survives_fold_and_resume`, `exact_error_and_file_path_remain_retrievable`, `last_mutation_generation_survives`, `unknown_effect_is_not_summarized_as_success`, `truncated_tool_output_has_recovery_pointer`.
- [ ] **Step 3 — Red:** `cargo test -p davinci-agent --test context_continuity_acceptance --offline --locked` or matching existing suites.
- [ ] **Step 4 — Repair when red:** Change only fold/retrieval/source-ref projection needed for lossless authority and safe fetch-back. Never inflate permanent context with full logs where a durable retrievable source suffices.
- [ ] **Step 5 — Green:** New test plus `cargo test -p davinci-agent --test context_vm_replay --test context_vm_retrieval --test context_vm_integrity --offline --locked`; assert exact authority/receipt continuity after restart and compaction.
- [ ] **Step 6 — Record:** Capture before/after contextual facts and actual token effect; **commit only if authorized**.

### Task B4 — Bounded feedback-driven repair loop

**Files:** `crates/davinci-agent/src/{turn.rs,completion.rs,decisions.rs}`, `src/prompt/{coding.rs,verification.rs}`, `tests/{harness_requirement_completion,completion_hooks,luna_reliability_followups}.rs`. Create `tests/failure_repair_loop.rs` if absent.

**Interfaces:** Reuse `HarnessVerification` and existing correction/stop policy. Emit factual tool failure, authorization and verification state into the next turn; do not invent green receipts.

- [ ] **Step 1 — Audit:** Inspect how failing command output, deny/unknown execution, truncation and completion-hook refusal become next-turn observations. Measure existing retry/turn budgets.
- [ ] **Step 2 — Test:** `verification_failure_informs_next_turn`, `second_edit_invalidates_previous_receipt`, `permission_denied_never_auto_retries_privileged_tool`, `bounded_repair_does_not_loop_forever`, `failed_command_retains_exit_status_and_relevant_output`.
- [ ] **Step 3 — Red:** `cargo test -p davinci-agent --test failure_repair_loop --offline --locked` if introduced, plus existing related tests.
- [ ] **Step 4 — Minimal fix:** Adjust the authoritative failure packet/observation and correction gate, not a generic "always retry" prompt. Keep retries bounded and avoid repeating unknown side effects.
- [ ] **Step 5 — Green:** Run new cases and `cargo test -p davinci-agent --test completion_hooks --test harness_requirement_completion --offline --locked`; assert no false verification.
- [ ] **Step 6 — Record:** Separate behavior improvements from runtime-integrity repairs in evaluation reports; **commit only if authorized**.

## Gate G2

A coding-success change is accepted only with repeatable task-level evidence, not an increased number of planning/tool calls or a passing behavioral text rubric alone. Keep old prompts if candidate fails independent acceptance.
