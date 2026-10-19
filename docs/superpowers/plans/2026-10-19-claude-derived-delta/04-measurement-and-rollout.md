# DaVinci Measurement, Differential Evals and Rollout — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Convert runtime changes into falsifiable evidence of higher independently verified coding-task completion per unit of time and subscription usage.

**Architecture:** Extend existing `davinci-evals`, runtime telemetry and CI differential workflow; freeze a baseline and compare one change at a time. Treat offline protocol correctness, live efficacy and release authorization as distinct gates.

**Tech Stack:** Rust 1.88.0, existing eval runner/JSON traces/fixtures, `crates/davinci-evals`, GitHub Actions, native tools.

**Spec:** `docs/superpowers/specs/2026-09-14-davinci-harness-optimization-design.md`, `00-INDEX.md`.

## Global constraints

- Live/model-backed tasks consume subscription allowance or paid API usage. **No live campaign without user authorization specifying provider, model, route, maximum runs and spending/usage scope.**
- `openai-codex` subscription runs must not be silently replaced with OpenAI API-key inference. Existing Claude competitor workflow requires specific credentials; do not run it casually.
- A same-model A/B of DaVinci is the primary causality test for harness improvements. An unlike-model Claude-vs-DaVinci comparison is descriptive, not proof of harness superiority.
- Record denominator, missing usage, failures, selected/routed effort, task snapshot, binary/commit hashes, diffs and independent acceptance checks. No cherry-picking successes or excluding retries.

## Review focus

- Empty/missing usage in one run must remain null/unknown, not zero (M2).
- Authentication failure or wrong model route must be classified as infra/config, not task success/failure (M3).
- A CLI exits success while hidden/task-specific tests fail: score as not solved (M3).
- Unequal task snapshots or models invalidate a harness-causality comparison (M4).
- An improvement increases success by luck on a tiny sample: report uncertainty and avoid promotion (M4).

---

### Task M1 — Baseline/gap register (mandatory first task)

**Files:** Read `AGENTS.md`, `docs/runtime/{unified-execution.md,execution-recovery-verification.md,operation-coverage.md}`, `docs/superpowers/plans/{2026-09-14-davinci-harness-optimization-master.md,2026-10-01-openai-harness-production-plan.md}` and relevant source/tests; write a working evidence artifact `target/davinci-delta-baseline/coverage.json` and readable `target/davinci-delta-baseline/README.md` outside the tracked source tree.

**Interfaces:** Coverage records contain `id`, `source_revision`, `status`, `owner_files`, `existing_tests`, `missing_live_path`, `next_action`, `evidence_path`. Allowed status: `PRESENT_AND_PROVEN`, `PRESENT_BUT_UNPROVEN`, `MISSING`, `NOT_WORTH_CHANGE`.

- [ ] **Step 1 — Freeze identity:** Record `git rev-parse HEAD`, `rustc --version`, lockfile digest, baseline provider routes/model configs and working-tree status. Record whether a worktree was created through the approved Superpowers worktree workflow at execution time.
- [ ] **Step 2 — Inspect all 15 mechanisms:** For each, enter source ownership, tests and actual production-path coverage in `coverage.json`. Compare against old plans to avoid overlapping feature work.
- [ ] **Step 3 — Baseline verification:** `cargo fmt --check`; `cargo test -p davinci-agent --offline --locked`; `cargo test -p davinci-evals --offline --locked`; `cargo run --offline --locked -p davinci-evals -- corpus verify`. Capture exit codes and logs. Existing unrelated failures must remain documented and unmodified unless blocking the proposed work.
- [ ] **Step 4 — Triage:** Close fully proven parity items with no edits; make concrete red-test tickets only for unproven or failing runtime paths. Identify any route/feature flag that makes a supposedly existing feature dormant.
- [ ] **Step 5 — Review and record:** Freeze baseline artifact and severity; obtain approval before implementation or live runs. No commits without authorization.

### Task M2 — Structured observability at the user-task boundary

**Files:** `crates/davinci-agent/src/{events.rs,tool_ledger.rs,turn.rs}`, `src/runtime/operations/{outbox.rs,transitions.rs}`, `src/runtime/context_vm/metrics.rs`; `crates/davinci-evals/src/behavior/{trace.rs,runner.rs}`, `tests/operation_publication.rs`. Add focused `tests/runtime_observation_integrity.rs` if needed.

**Interfaces:** Existing typed events/JSON traces; required recorded identities: session/run/task, model/route, model request, tool call ID, permission, dispatch/attempt, result status, compaction generation, mutation generation, verification coverage and terminal reason. No secret-bearing raw prompt logging by default.

- [ ] **Step 1 — Audit:** List already captured fields and missing joins; do not add another tracing framework.
- [ ] **Step 2 — Test:** `interrupted_call_has_distinct_nonexecution_or_unknown`, `inflight_tool_ids_join_to_results`, `final_verified_state_requires_latest_receipt`, `unknown_usage_is_not_zero`, `sensitive_values_never_enter_exported_trace`.
- [ ] **Step 3 — Red:** `cargo test -p davinci-agent --test runtime_observation_integrity --offline --locked` if new. Establish serializable event contract and finite output bounds.
- [ ] **Step 4 — Minimal repair:** Add only missing structured fields to current event/ledger/eval projection; make reads of older saved versions backward-compatible.
- [ ] **Step 5 — Green:** Rerun focused recovery/trace suites and privacy redaction assertions. Verify deterministic identifiers and no false success on missing evidence.
- [ ] **Step 6 — Record:** Capture before/after example JSON with fake fixture values and schema compatibility notes; **commit only if authorized**.

### Task M3 — Offline regression and independent task acceptance

**Files:** `crates/davinci-evals/{README.md,fixtures/behavior/,fixtures/competitor/}`, `crates/davinci-evals/src/{behavior/scorer.rs,behavior/gate.rs,competitor/matched.rs}`, `.github/workflows/{behavior-evals.yml,competitor-differential.yml}`.

**Interfaces:** Existing DaVinci eval corpus and comparator. A task is *solved* only if independent task acceptance tests pass on its final isolated workspace, with no invalid authorization or unacknowledged side effects.

- [ ] **Step 1 — Audit coverage:** Map 15 improvements to existing Core-200, debugging-hard, context-VM, competitor and runtime failure-injection suites. Do not duplicate fixtures whose oracle already tests the behavior.
- [ ] **Step 2 — Write scoring regressions:** `terminal_success_with_failing_tests_is_failure`, `test_pass_before_final_edit_is_unverified`, `infra_failure_retained_but_not_solved`, `missing_usage_retained_as_unknown`, `unmatched_task_revision_invalidates_comparison`, `retry_attempts_count_in_denominator`.
- [ ] **Step 3 — Red/verify:** `cargo test -p davinci-evals --offline --locked`, `cargo run --offline --locked -p davinci-evals -- corpus verify`; run only fixture-mode behavior tests that do not contact a provider.
- [ ] **Step 4 — Minimal fix:** Change the current scorer/matched comparison, not create a new evaluator. Introduce coverage for new runtime failure cases only when acceptance requires it.
- [ ] **Step 5 — Green:** Assert independent pass/failed/unknown outcome classes and both attempted/valid-run denominators are reported.
- [ ] **Step 6 — Record:** Save immutable fixture suite/check hashes and scorer diff; **commit only if authorized**.

### Task M4 — Opt-in matched A/B and decision gate

**Files:** `crates/davinci-evals/src/{behavior/compare.rs,competitor/matched.rs,competitor/report.rs}`, `.github/workflows/competitor-differential.yml`, `docs/cache/codex-comparison.md`; output a result bundle under `target/davinci-delta-evals/<date>/`.

**Interfaces:** Existing paired report format. Comparison inputs: baseline/candidate commit, exact model/route/effort, matched workspace SHA/fixture versions, allowed tools/permissions, task acceptance commands and budget/time caps.

- [ ] **Step 1 — Approval gate:** Obtain explicit authorization for a finite live A/B run: model + route (including subscription eligibility), candidate/baseline builds, task list, repeats, time and account usage limits. Without it, stop at offline evidence.
- [ ] **Step 2 — Freeze design:** Use at least 5 diverse tasks and 3 paired repeats as a **screening sample**, following existing comparison docs; plan a larger held-out sample for confident quality claims. Randomize pair order and isolate identical fixture snapshots.
- [ ] **Step 3 — Execute permitted runs:** A/B DaVinci current vs candidate first using same model/effort and routes. Only compare to Claude Code when a defensible matched model/route is possible; otherwise label results cross-product and not harness-causal.
- [ ] **Step 4 — Independently score:** Run acceptance commands after the last mutation; classify failures into task failure, model refusal, permission block, context loss, tool/protocol failure, unknown side effect and infra/config fault.
- [ ] **Step 5 — Analyze:** Report solved tasks, uncertainty (paired confidence interval or bootstrap), time-to-verified-solution, requests, actual cached/fresh/output tokens, unknown usage, retries, unsafe side effects and unreproduced tasks. No arbitrary "10% better" claim from small N.
- [ ] **Step 6 — Decision:** Promote only if **no safety/regression failures** and independently verified solved-task outcomes improve without unacceptable latency/usage increase. Otherwise revert candidate. Release and deployment remain separate user approvals.

## Quality and release gates

| Gate | Required evidence | Meaning of failure |
|---|---|---|
| Static + local | `cargo fmt --check`, affected package tests, `cargo test -p davinci-agent --offline --locked` where relevant | Do not present as passing |
| Safety invariants | Crash/resume, owner fencing, complete paired history, never reissue uncertain side effect | Block shipping |
| Completion honesty | Fresh verification after last mutation, independent acceptance | Do not mark solved |
| Context continuity | Exact requirements and protected facts after compaction/resume | Block context change |
| Live behavior (opt-in) | Paired, same-model, independently scored tasks | No causal pass-rate claim |
| Release | User-authorized review, diff, current CI and rollback | No push/deploy without permission |

## Handoff

Present the baseline coverage report before proposing code changes. Each task is separate and independently reviewable; a task with existing production-grade proof is an evidence-only closure, not a forced patch. Obtain execution-method selection and any live-test authorization from the user before running changes.
