# Natural-Language Design Studio Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Let users type `/design <brief>` without flags, receive three meaningful browser-preview concepts, pick and revise one, and separately approve implementation.

**Architecture:** Reuse the existing session-backed design controller, pinned compiler, browser companion and strict sandbox; introduce inferred design intent, diagnostics and explicit quality gates instead of a second artifact framework.

**Tech Stack:** Rust CLI/design controller, React companion, Node 24.19.0, Playwright 1.62.1, Windows AppContainer, Linux strict namespaces, openai-codex subscription route.

**Spec:** [`../specs/2026-10-17-davinci-reliability-and-zero-maintenance-design.md`](../specs/2026-10-17-davinci-reliability-and-zero-maintenance-design.md)

**Dependencies / execution order:** After Plan 01 trust and Plan 02 setup-health contracts; can be developed in parallel with Plan 03 context integrity.

## Global Constraints

**Path shorthand:** Any unqualified `src/...` or `tests/...` in this plan is relative to `crates/davinci-coding-agent/`. Explicit `crates/...` paths are repo-root-relative. For the Design Studio plan, `design-ui/...` is relative to `crates/davinci-coding-agent/`.

- This is a plan, not an implementation. Start from a clean isolated worktree of the exact target `main` commit. Confirm historical audit findings still reproduce, especially after recent compaction changes.
- Rust workspace/pinned toolchains and existing feature flags, permission policy, sandbox, provider identity, durable stores and cancellation must stay intact. No live package installs, cloud requests or data mutations without explicit operator approval.
- Local JSONL remains authoritative. Preserve disabled-mode parity, existing slash commands, recovery safety and provenance; no mandatory Docker/Qdrant/SQLite.
- New tasks must use red-green TDD, targeted verification, one purpose per commit, reviewer sign-off and rollback/feature gate before default enablement.

## Review Focus

- `/design` without flags should infer dashboard instead of implicit landing — Tasks 2/3.
- Successful source generation with browser blocked must retain artifacts and explain recovery — Task 6.
- Output schema malformed and retries exhausted must identify exact error and preserve last good revision — Tasks 1/5.
- Generated page may not make network calls or escape confinement — Task 8.
- Choosing a design must not silently edit repository source — Task 7.

---

### Task 1: Capture actual /design failure boundaries before modifying generation

**Files:** Modify: `crates/davinci-coding-agent/src/design/{controller,generation,model,host}.rs`; Test: `crates/davinci-coding-agent/tests/design_artifact_diagnostics.rs`

**Interfaces:** Produces `DesignFailureReceipt { phase, code, artifact, revision, retryable, repair_count }`.

- [ ] **Step 1 — Write the failing regression** `design_error_reports_exact_stage_and_recoverable_revision`. Assert: A recorded invalid `variants` output reports `phase=Generate`, schema path, recoverable artifact/revision, model route and remaining bounded attempts; no raw credentials or giant responses..
- [ ] **Step 2 — Prove red on baseline.** Run `cargo test -p davinci-coding-agent design_error_reports_exact_stage_and_recoverable_revision -- --nocapture`. Expected: test fails for the identified behavior (or mark as already fixed and report evidence; never make an unnecessary fix).
- [ ] **Step 3 — Implement one targeted change.** Trace command -> admission -> provider -> parsed tool response -> compile -> confined render -> host open using structured error receipts, not guessed catch-all error messages. Use repository readiness fixtures.
- [ ] **Step 4 — Verify green and regression boundaries.** Re-run `cargo test -p davinci-coding-agent design_error_reports_exact_stage_and_recoverable_revision -- --nocapture`; expected: pass. Also run the module-specific suite, `cargo fmt --all -- --check`, and relevant Clippy targets. Save test transcript/receipt and verify no new permissions or default behavior changed.
- [ ] **Step 5 — Review and commit.** Reviewer inspects exact code and config/data compatibility; commit as `fix(design): expose generation root-cause evidence`. Do not proceed when the gate is red.

### Task 2: Parse natural-language /design commands without mandatory flags

**Files:** Modify: `crates/davinci-coding-agent/src/design/commands.rs`; Test: `crates/davinci-coding-agent/tests/design_artifact_diagnostics.rs`

**Interfaces:** Produces `DesignIntent { brief, explicit_kind: Option<DesignKind>, variants: u32 }` consumed by controller.

- [ ] **Step 1 — Write the failing regression** `design_brief_without_flags_keeps_full_intent`. Assert: `/design Create a modern dashboard for AI agents` retains whole brief, generates intent inference path and defaults to three concepts; legacy `new --kind --variants` still parses exactly as before..
- [ ] **Step 2 — Prove red on baseline.** Run `cargo test -p davinci-coding-agent design_brief_without_flags_keeps_full_intent -- --nocapture`. Expected: test fails for the identified behavior (or mark as already fixed and report evidence; never make an unnecessary fix).
- [ ] **Step 3 — Implement one targeted change.** Keep advanced flags optional; avoid treating ordinary words that resemble reserved verbs as errors by explicit `new`/`--` compatibility; sanitize user prompt as data, not shell args.
- [ ] **Step 4 — Verify green and regression boundaries.** Re-run `cargo test -p davinci-coding-agent design_brief_without_flags_keeps_full_intent -- --nocapture`; expected: pass. Also run the module-specific suite, `cargo fmt --all -- --check`, and relevant Clippy targets. Save test transcript/receipt and verify no new permissions or default behavior changed.
- [ ] **Step 5 — Review and commit.** Reviewer inspects exact code and config/data compatibility; commit as `feat(design): natural language first command UX`. Do not proceed when the gate is red.

### Task 3: Infer design kind from brief and enforce concept diversity

**Files:** Modify: `crates/davinci-coding-agent/src/design/{controller,generation}.rs`; Test: `crates/davinci-coding-agent/tests/design_artifact_diagnostics.rs`

**Interfaces:** Consumes `DesignIntent`; produces `ResolvedDesignBrief { kind, variants, brief, source }`.

- [ ] **Step 1 — Write the failing regression** `infer_product_dashboard_and_generate_three_distinct_directions`. Assert: A dashboard brief yields product kind without `--kind`; three directions have differing layout/typography/content strategy; explicit flags override inference; unsupported intent gets deterministic documented default..
- [ ] **Step 2 — Prove red on baseline.** Run `cargo test -p davinci-coding-agent infer_product_dashboard_and_generate_three_distinct_directions -- --nocapture`. Expected: test fails for the identified behavior (or mark as already fixed and report evidence; never make an unnecessary fix).
- [ ] **Step 3 — Implement one targeted change.** Add bounded classifier using permitted existing model route (or transparent deterministic fallback), account requests to budget, preserve 1..3 limit; do not let classification consume hidden unlimited attempts.
- [ ] **Step 4 — Verify green and regression boundaries.** Re-run `cargo test -p davinci-coding-agent infer_product_dashboard_and_generate_three_distinct_directions -- --nocapture`; expected: pass. Also run the module-specific suite, `cargo fmt --all -- --check`, and relevant Clippy targets. Save test transcript/receipt and verify no new permissions or default behavior changed.
- [ ] **Step 5 — Review and commit.** Reviewer inspects exact code and config/data compatibility; commit as `feat(design): infer design type automatically`. Do not proceed when the gate is red.

### Task 4: Make installation and runtime preflight self-explanatory

**Files:** Modify: `crates/davinci-coding-agent/src/design/{setup,runtime}.rs`, `src/davinci_interactive.rs`; Test: `crates/davinci-coding-agent/tests/design_artifact_diagnostics.rs`

**Interfaces:** Produces `DesignRuntimeHealth { state, diagnostics, fix_action }` and actionable setup options.

- [ ] **Step 1 — Write the failing regression** `design_preflight_distinguishes_missing_disabled_invalid`. Assert: Missing Node, unsupported version, no Chromium, tampered runtime, environment override, wrong provider, and home/workspace conflict yield distinct actionable errors; no unsafe auto-execute of workspace checkout script..
- [ ] **Step 2 — Prove red on baseline.** Run `cargo test -p davinci-coding-agent design_preflight_distinguishes_missing_disabled_invalid -- --nocapture`. Expected: test fails for the identified behavior (or mark as already fixed and report evidence; never make an unnecessary fix).
- [ ] **Step 3 — Implement one targeted change.** Use existing pinned Windows setup script only from trusted location and explicit consent; integrate its health into `/setup`; retain no-network/untrusted-code execution ban and Linux/macOS limitation messages.
- [ ] **Step 4 — Verify green and regression boundaries.** Re-run `cargo test -p davinci-coding-agent design_preflight_distinguishes_missing_disabled_invalid -- --nocapture`; expected: pass. Also run the module-specific suite, `cargo fmt --all -- --check`, and relevant Clippy targets. Save test transcript/receipt and verify no new permissions or default behavior changed.
- [ ] **Step 5 — Review and commit.** Reviewer inspects exact code and config/data compatibility; commit as `fix(design): perform safe complete preflight`. Do not proceed when the gate is red.

### Task 5: Bound model-schema repair without losing generated drafts

**Files:** Modify: `crates/davinci-coding-agent/src/design/generation.rs`; Test: `crates/davinci-coding-agent/tests/design_artifact_diagnostics.rs`

**Interfaces:** Consumes `DesignFailureReceipt`; preserves checkpoint and bounded `RunPhase` transitions.

- [ ] **Step 1 — Write the failing regression** `one_variant_normalization_and_bad_shape_checkpoint`. Assert: One-concept object is normalized only when schema-valid; arrays of 3 retain all concepts; malformed `variants` after bounded repair returns exact stage/error and existing source revision remains readable..
- [ ] **Step 2 — Prove red on baseline.** Run `cargo test -p davinci-coding-agent one_variant_normalization_and_bad_shape_checkpoint -- --nocapture`. Expected: test fails for the identified behavior (or mark as already fixed and report evidence; never make an unnecessary fix).
- [ ] **Step 3 — Implement one targeted change.** Maintain strict typed validation with a limited repair contract, no silent model-switch or budget resets; persist failure data using immutable artifact evidence.
- [ ] **Step 4 — Verify green and regression boundaries.** Re-run `cargo test -p davinci-coding-agent one_variant_normalization_and_bad_shape_checkpoint -- --nocapture`; expected: pass. Also run the module-specific suite, `cargo fmt --all -- --check`, and relevant Clippy targets. Save test transcript/receipt and verify no new permissions or default behavior changed.
- [ ] **Step 5 — Review and commit.** Reviewer inspects exact code and config/data compatibility; commit as `fix(design): make malformed generation recoverable`. Do not proceed when the gate is red.

### Task 6: Open confined studio automatically on successful design generation

**Files:** Modify: `crates/davinci-coding-agent/src/design/{controller,host}.rs`, `src/davinci_interactive.rs`, `design-ui/src/`; Test: `crates/davinci-coding-agent/tests/design_companion_e2e.rs`

**Interfaces:** Produces user-visible design launch receipt and paired browser open event.

- [ ] **Step 1 — Write the failing regression** `new_design_opens_paired_browser_studio_and_variants`. Assert: After generated revisions commit, authenticated loopback browser studio opens and shows all three variants; a browser-open failure returns local explicit Open action and does not lose artifacts; no internet origin access..
- [ ] **Step 2 — Prove red on baseline.** Run `cargo test -p davinci-coding-agent new_design_opens_paired_browser_studio_and_variants -- --nocapture`. Expected: test fails for the identified behavior (or mark as already fixed and report evidence; never make an unnecessary fix).
- [ ] **Step 3 — Implement one targeted change.** Reuse existing paired companion, one-time token/lease and strict confinement. Launch only verified host runtime; never run generated code in the workspace origin.
- [ ] **Step 4 — Verify green and regression boundaries.** Re-run `cargo test -p davinci-coding-agent new_design_opens_paired_browser_studio_and_variants -- --nocapture`; expected: pass. Also run the module-specific suite, `cargo fmt --all -- --check`, and relevant Clippy targets. Save test transcript/receipt and verify no new permissions or default behavior changed.
- [ ] **Step 5 — Review and commit.** Reviewer inspects exact code and config/data compatibility; commit as `feat(design): launch studio after successful generation`. Do not proceed when the gate is red.

### Task 7: Selection, natural-language revision and implementation approval

**Files:** Modify: `crates/davinci-coding-agent/src/design/{controller,acceptance,handoff}.rs`, `design-ui/src/`; Test: `crates/davinci-coding-agent/tests/design_companion_e2e.rs`

**Interfaces:** Produces `DesignSelectionReceipt` referenced by revision and implementation handoff.

- [ ] **Step 1 — Write the failing regression** `select_variant_revise_reload_and_review_without_apply`. Assert: Choose concept B, relaunch session and retain selection; user says “make B more minimal”, creates a new revision; no files in target repo changed until reviewed implementation plan and explicit patch approval..
- [ ] **Step 2 — Prove red on baseline.** Run `cargo test -p davinci-coding-agent select_variant_revise_reload_and_review_without_apply -- --nocapture`. Expected: test fails for the identified behavior (or mark as already fixed and report evidence; never make an unnecessary fix).
- [ ] **Step 3 — Implement one targeted change.** Wire selection to durable revision/acceptance event, natural-language revise to existing design model, and UI compare/zoom/export. Preserve source/evidence distinction.
- [ ] **Step 4 — Verify green and regression boundaries.** Re-run `cargo test -p davinci-coding-agent select_variant_revise_reload_and_review_without_apply -- --nocapture`; expected: pass. Also run the module-specific suite, `cargo fmt --all -- --check`, and relevant Clippy targets. Save test transcript/receipt and verify no new permissions or default behavior changed.
- [ ] **Step 5 — Review and commit.** Reviewer inspects exact code and config/data compatibility; commit as `feat(design): make browser selection persistent and safe`. Do not proceed when the gate is red.

### Task 8: Release gate for actual visual quality and cross-platform confinement

**Files:** Modify: `docs/readiness/design-artifacts.md`; Create: `crates/davinci-evals/src/behavior/design_studio.rs`; Test: `crates/davinci-coding-agent/tests/design_companion_e2e.rs`

**Interfaces:** Produces release evidence report with exact commit/runtime fingerprints.

- [ ] **Step 1 — Write the failing regression** `design_three_concepts_capture_and_strict_browser_pass`. Assert: Windows real runtime renders three concepts at mobile/tablet/desktop, captures screenshots and performs selection/revision; hostile page network/process access remains denied; live provider quality trial is separately approved/budgeted..
- [ ] **Step 2 — Prove red on baseline.** Run `cargo test -p davinci-coding-agent design_three_concepts_capture_and_strict_browser_pass -- --nocapture`. Expected: test fails for the identified behavior (or mark as already fixed and report evidence; never make an unnecessary fix).
- [ ] **Step 3 — Implement one targeted change.** Publish actual native test receipts, known failed cases and rollback paths. Do not claim Claude-hosted artifact parity or production readiness from fixture-only tests.
- [ ] **Step 4 — Verify green and regression boundaries.** Re-run `cargo test -p davinci-coding-agent design_three_concepts_capture_and_strict_browser_pass -- --nocapture`; expected: pass. Also run the module-specific suite, `cargo fmt --all -- --check`, and relevant Clippy targets. Save test transcript/receipt and verify no new permissions or default behavior changed.
- [ ] **Step 5 — Review and commit.** Reviewer inspects exact code and config/data compatibility; commit as `test(design): certify artifact workflow end to end`. Do not proceed when the gate is red.

## Release checklist

- [ ] All task-specific failing tests were demonstrated against the baseline or explicitly documented as already fixed.
- [ ] Every task passes focused tests, formatting and relevant Clippy; existing test suite passes with no suppressed failures.
- [ ] Scenario-level tests and negative cases run on disposable workspaces with fake services where possible; live-system tests run only with authorized dependencies.
- [ ] Documentation, rollback procedure, opt-out and error reporting are updated; no credential, memory text or user path appears in broad logs.
- [ ] Merge behind a bounded feature gate, collect real-world diagnostics, then separately authorize default-on behavior.
