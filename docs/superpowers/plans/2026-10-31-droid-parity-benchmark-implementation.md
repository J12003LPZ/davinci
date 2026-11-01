# Droid-versus-DaVinci Behavioral and Performance Parity Evaluation Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Independently prove safe Droid-equivalent Script behavior and determine whether controlled Script improves verified OpenAI coding success per subscription resource.

**Architecture:** Keep PR #264’s three-arm benchmark and add an explicit DaVinci Controlled Script arm, correct the research fixture adapter, add deterministic Script-security oracles, then pre-register live comparisons. Treat direct Codex vs Droid-through-proxy as a documented product-system comparison unless a causal harness-only control becomes possible.

**Tech Stack:** Python pytest scripts/bench, Rust davinci-evals, native Windows harness and immutable workspaces.

**Spec:** `docs/superpowers/specs/2026-10-31-droid-script-controlled-parity-design.md`. This is a *proposed amendment* to the existing owner plan (`docs/superpowers/plans/2026-10-31-droid-gap.md`); reconcile and approve before product-code implementation.

**Pinned baseline:** [`ecc8485`](https://github.com/J12003LPZ/davinci/commit/ecc8485361dc335aec8b82e7014b94904f347c71), Stage 0 PR #264, `gpt-6.1-sol` comparisons on 17 tasks. **Plan-only deliverable; no code changes, permissions, credentials, paid probes, or live experiments performed.**

## Global Constraints

- Preserve authenticated ChatGPT `openai-codex` without requiring an API key or paid API fallback.
- Root `AGENTS.md` and the existing Codemode C01–C12 security, journal, cancellation and model/effort contracts apply. No product write, commit, install or live subscription probe is authorized by this planning request.
- Keep current default/direct behavior until safe promotion; no speculative model calls or subagents to optimize heuristically.
- Independent behavioral imitation only: no proprietary binary/runtime code or exact private prompt copying.
- Rust toolchain **1.88.0** at inspected repo baseline; use pinned project dependencies and approved optional Node **24.21.0** Codemode sandbox, not an unverified interpreter.
- The public OpenAI API is **not** the ChatGPT Codex OAuth route; use a per-route *observed* capability matrix, not documentation alone.
- Separate full behavioral parity from performance parity and actual subscription-quota benefit; use independent oracles and include failures in total resource calculations.
- Do not change the `2026-10-04` plan's background-service prohibition without explicit owner approval and updated C08 recovery gates.

## Review Focus

1. Tool permission revoked while a Script call waits in a queue: deny before dispatch; test in the dispatch/permission task.
2. Mutation completed but reply lost: no automatic replay, later Script does not falsely imply rollback; test in the recovery task.
3. Process tree survives Script kill: must be owned/reaped or surfaced as unresolved; test in the job lifecycle task.
4. `toolCallId` from another session/branch: reject `status/wait/kill`; test in WaitForScript task.
5. Provider cache drops without any changed local prefix: classify UNKNOWN/provider-side instead of fabricating local cause; test in diagnostics task.

---

## Parity claim labels

- `BEHAVIOR_PARITY`: mocked/deterministic same observable result and recovery semantics (does **not** imply token/quality parity).
- `PERFORMANCE_PARITY`: matched model/effort task corpus meets pre-registered noninferiority and safety/latency conditions.
- `IMPROVED_ON_METRIC`: a specific reported metric is better with confidence; not a global win.
- `JUSTIFIED_DIFFERENCE`: deliberate stronger safety or API mismatch documented and tested.
- `UNTESTED / UNSUPPORTED / UNKNOWN`: explicit blockers and no performance claim.

Stage 0 already implemented loop detection, head/tail truncation and basic Droid arm. Preserve watchdog exemptions and test the no-visible-output-turn exit contract as an explicit fixture too. Keep those as regressions; do not rewrite them to increase task counts.

### Task B01: Repair and pin the 17-task benchmark baseline

**Dependencies:** existing PR #264 benchmark. **Commit gate:** independent reviewer approves each code/test change before it merges; do not accidentally mark this task complete by a green unrelated test.

**Files**
- Modify: `scripts/bench/task_sources.py`
- Modify: `scripts/bench/campaign.py`
- Modify: `docs/perf/droid-gap/stage0-report.md`
- Test: `scripts/bench/tests/test_task_sources.py`

**Interface:** `StudyTaskAdapter` existing; fix grader location and fixture semantics before new scoring

**Required behavior:** Version the research adapter and graders, pin expected artifact location and old/new fixture hashes; ensure a known-good solution passes and deliberately bad answers fail.

**Grounding:** PR #264 research-01/02 were false negatives because fixture written to wrong root. Research-03 has ambiguous 7/8-digit expectations and out-file cleanup semantics; amend rubric with versioned new fixture, never rewrite preregistered report.

**Acceptance:** Any preregistered historical run remains untouched; known-good and deliberately-bad solutions grade as expected.

- [ ] **Step 1 — RED:** Create a test named `research_fixtures_grade_correctly` in `scripts/bench/tests/test_task_sources.py` reproducing the acceptance condition with a disposable fixture. Ensure it fails against the current implementation for the *right reason*.
- [ ] **Step 2 — Record RED:** Run `python -m pytest scripts/bench/tests -q` and capture the expected failed assertion (not a compile or environment error).
- [ ] **Step 3 — GREEN:** Implement the interface and required behavior above in only the named owner files; use existing components and add a focused file if needed, without duplicating the operation journal.
- [ ] **Step 4 — Verify GREEN:** Run `python -m pytest scripts/bench/tests -q` and the relevant neighboring tests. Must pass; on a feature involving effects, perform a fresh subprocess/restart fixture too.
- [ ] **Step 5 — Review/commit:** Run `cargo fmt --all --check` and targeted `cargo clippy -p davinci-agent -p davinci-coding-agent -p davinci-ai --all-targets` for affected Rust crates. Inspect failure and security cases and commit **only if separately authorized**; otherwise leave diff for review.

### Task B02: Add deterministic Script reference and security evaluation classes

**Dependencies:** S19 deterministic Script fixtures. **Commit gate:** independent reviewer approves each code/test change before it merges; do not accidentally mark this task complete by a green unrelated test.

**Files**
- Modify: `crates/davinci-evals/fixtures/behavior/verified-core.json`
- Modify: `scripts/bench/tests/test_gap_study.py`
- Test: `scripts/bench/tests/test_droid_real_shapes.py`

**Interface:** `ScriptEvalCase { id, repo_sha, setup, prompt, actions_expected, hidden_assertions, security_constraints }` proposed fixture contract

**Required behavior:** Build 12 Droid-reference categories plus mutation-recovery and background scenarios; grade via independent tests, not model narration. Duplicate-write and escaped-process attempts are automatic failures regardless of task output.

**Grounding:** Droid §19 used mock; this provides DaVinci deterministic behavior parity but not live success proof.

**Acceptance:** All negative safety cases fail if a mutation occurs; no oracle reads model-created answer as authoritative.

- [ ] **Step 1 — RED:** Create a test named `behavioral_parity_oracle_rejects_unsafe_success` in `scripts/bench/tests/test_droid_real_shapes.py` reproducing the acceptance condition with a disposable fixture. Ensure it fails against the current implementation for the *right reason*.
- [ ] **Step 2 — Record RED:** Run `python -m pytest scripts/bench/tests -q` and capture the expected failed assertion (not a compile or environment error).
- [ ] **Step 3 — GREEN:** Implement the interface and required behavior above in only the named owner files; use existing components and add a focused file if needed, without duplicating the operation journal.
- [ ] **Step 4 — Verify GREEN:** Run `python -m pytest scripts/bench/tests -q` and the relevant neighboring tests. Must pass; on a feature involving effects, perform a fresh subprocess/restart fixture too.
- [ ] **Step 5 — Review/commit:** Run `cargo fmt --all --check` and targeted `cargo clippy -p davinci-agent -p davinci-coding-agent -p davinci-ai --all-targets` for affected Rust crates. Inspect failure and security cases and commit **only if separately authorized**; otherwise leave diff for review.

### Task B03: Run narrow Script-first pilot under explicit live approval

**Dependencies:** B01; B02; S20 opt-in delivery; explicit paid-run approval. **Commit gate:** independent reviewer approves each code/test change before it merges; do not accidentally mark this task complete by a green unrelated test.

**Files**
- Modify: `scripts/bench/campaign.py`
- Modify: `scripts/bench/droid_arm.py`
- Modify: `scripts/bench/gap_study.py`
- Test: `scripts/bench/tests/test_gap_study.py`

**Interface:** `HarnessArm { name: davinci_direct|davinci_script|droid_direct|droid_script, model, effort, route }` (bench configuration)

**Required behavior:** Pilot at most 24 paid runs after new approval, task mix of high fanout, edit→test, long output; record native DaVinci Direct vs Controlled and Droid Direct vs Script. One run/arm minimum, more only within explicitly approved cap.

**Grounding:** Stage 0 on 17 tasks showed W3 trigger only for request count, wall and uncached not clear.

**Acceptance:** Script failure cannot be hidden in success-only comparisons; per-request cache reads and proxy route recorded.

- [ ] **Step 1 — RED:** Create a test named `matched_pilot_reports_unknown_values` in `scripts/bench/tests/test_gap_study.py` reproducing the acceptance condition with a disposable fixture. Ensure it fails against the current implementation for the *right reason*.
- [ ] **Step 2 — Record RED:** Run `python -m pytest scripts/bench/tests -q` and capture the expected failed assertion (not a compile or environment error).
- [ ] **Step 3 — GREEN:** Implement the interface and required behavior above in only the named owner files; use existing components and add a focused file if needed, without duplicating the operation journal.
- [ ] **Step 4 — Verify GREEN:** Run `python -m pytest scripts/bench/tests -q` and the relevant neighboring tests. Must pass; on a feature involving effects, perform a fresh subprocess/restart fixture too.
- [ ] **Step 5 — Review/commit:** Run `cargo fmt --all --check` and targeted `cargo clippy -p davinci-agent -p davinci-coding-agent -p davinci-ai --all-targets` for affected Rust crates. Inspect failure and security cases and commit **only if separately authorized**; otherwise leave diff for review.

### Task B04: Repeat independent matched eval with enough cases for parity claims

**Dependencies:** B03 and sufficient task corpus; explicit paid-run approval. **Commit gate:** independent reviewer approves each code/test change before it merges; do not accidentally mark this task complete by a green unrelated test.

**Files**
- Modify: `scripts/bench/campaign.py`
- Modify: `crates/davinci-evals/src/competitor/report.rs`
- Test: `scripts/bench/tests/test_gap_study.py`

**Interface:** Existing official competitor gate: >=150 shared scenarios, 3 repetitions per harness, suitable CI/noninferiority, no boundary or unrelated edit regressions

**Required behavior:** Use matched immutable repos, controlled model and effort, pre-registered tasks and seeds, integrity-checked artifacts. Report pairing, variance, transport confounds, missing usage, repeat failures. No claim based on vendor benchmark or 17-case pilot.

**Grounding:** docs/competitor-benchmarks.md official claim gate.

**Acceptance:** No “Droid parity proven” status if missing repeated cases, grading oracle, or provider-account counters.

- [ ] **Step 1 — RED:** Create a test named `performance_claim_requires_suite_gate` in `scripts/bench/tests/test_gap_study.py` reproducing the acceptance condition with a disposable fixture. Ensure it fails against the current implementation for the *right reason*.
- [ ] **Step 2 — Record RED:** Run `python -m pytest scripts/bench/tests -q` and capture the expected failed assertion (not a compile or environment error).
- [ ] **Step 3 — GREEN:** Implement the interface and required behavior above in only the named owner files; use existing components and add a focused file if needed, without duplicating the operation journal.
- [ ] **Step 4 — Verify GREEN:** Run `python -m pytest scripts/bench/tests -q` and the relevant neighboring tests. Must pass; on a feature involving effects, perform a fresh subprocess/restart fixture too.
- [ ] **Step 5 — Review/commit:** Run `cargo fmt --all --check` and targeted `cargo clippy -p davinci-agent -p davinci-coding-agent -p davinci-ai --all-targets` for affected Rust crates. Inspect failure and security cases and commit **only if separately authorized**; otherwise leave diff for review.

### Task B05: Track verified-success-per-subscription-usage and resource costs

**Dependencies:** B01; B03. **Commit gate:** independent reviewer approves each code/test change before it merges; do not accidentally mark this task complete by a green unrelated test.

**Files**
- Modify: `scripts/bench/gap_study.py`
- Modify: `crates/davinci-ai/src/provider_observation.rs`
- Test: `scripts/bench/tests/test_gap_study.py`

**Interface:** `ReportMetrics { successful_tasks, total_attempts, logical_requests, fresh_input, cache_read, output_plus_reasoning, wall_ms, account_window_delta? }`

**Required behavior:** Total all attempts include failures. Account allowance window only when actually observable and attribution unconfounded; list-price equivalent stays separate. Track model turns, tool calls, duplicate side effects, RSS and disk.

**Grounding:** User primary objective is Codex subscription longevity, not lowest raw model input.

**Acceptance:** No “saves subscription quota” claim without direct allowance usage; missing windows remain `not measured`.

- [ ] **Step 1 — RED:** Create a test named `subscription_savings_not_inferred_from_tokens` in `scripts/bench/tests/test_gap_study.py` reproducing the acceptance condition with a disposable fixture. Ensure it fails against the current implementation for the *right reason*.
- [ ] **Step 2 — Record RED:** Run `python -m pytest scripts/bench/tests -q` and capture the expected failed assertion (not a compile or environment error).
- [ ] **Step 3 — GREEN:** Implement the interface and required behavior above in only the named owner files; use existing components and add a focused file if needed, without duplicating the operation journal.
- [ ] **Step 4 — Verify GREEN:** Run `python -m pytest scripts/bench/tests -q` and the relevant neighboring tests. Must pass; on a feature involving effects, perform a fresh subprocess/restart fixture too.
- [ ] **Step 5 — Review/commit:** Run `cargo fmt --all --check` and targeted `cargo clippy -p davinci-agent -p davinci-coding-agent -p davinci-ai --all-targets` for affected Rust crates. Inspect failure and security cases and commit **only if separately authorized**; otherwise leave diff for review.

### Task B06: Promote only proven safe/default improvements

**Dependencies:** B02; B04; B05; K01–K07 where applicable. **Commit gate:** independent reviewer approves each code/test change before it merges; do not accidentally mark this task complete by a green unrelated test.

**Files**
- Modify: `docs/superpowers/plans/2026-10-31-droid-gap.md`
- Modify: `docs/codemode.md`
- Modify: `docs/perf/droid-gap/stage0-report.md`
- Test: `scripts/bench/tests/test_gap_study.py`

**Interface:** `PromotionDecision { behavior_parity, security_pass, success_noninferior, resource_benefit, unknowns }`

**Required behavior:** Separate behavior parity, performance parity, and justified difference. Keep off/direct default if evidence inconclusive; opt-in controlled only after safety tests; per-model/route model aliases and rollback. Retain stronger DaVinci security even if Droid has less.

**Grounding:** Stage 0 is explicitly not proof of overall superiority.

**Acceptance:** No default change on partial evidence, no hidden model calls or automatic extra provider requests.

- [ ] **Step 1 — RED:** Create a test named `rollout_gate_keeps_unsafe_mode_off` in `scripts/bench/tests/test_gap_study.py` reproducing the acceptance condition with a disposable fixture. Ensure it fails against the current implementation for the *right reason*.
- [ ] **Step 2 — Record RED:** Run `python -m pytest scripts/bench/tests -q` and capture the expected failed assertion (not a compile or environment error).
- [ ] **Step 3 — GREEN:** Implement the interface and required behavior above in only the named owner files; use existing components and add a focused file if needed, without duplicating the operation journal.
- [ ] **Step 4 — Verify GREEN:** Run `python -m pytest scripts/bench/tests -q` and the relevant neighboring tests. Must pass; on a feature involving effects, perform a fresh subprocess/restart fixture too.
- [ ] **Step 5 — Review/commit:** Run `cargo fmt --all --check` and targeted `cargo clippy -p davinci-agent -p davinci-coding-agent -p davinci-ai --all-targets` for affected Rust crates. Inspect failure and security cases and commit **only if separately authorized**; otherwise leave diff for review.

## Independent reviewer gates

1. **Behavior:** runnable fixture proof for Script, direct, read-only, controlled, background, permission and restart; no fake success from model prose.
2. **Resource telemetry:** independent token accounting and logical request counts, cached vs fresh on exact route, missing windows visible. The proxy confound is retained in every chart.
3. **Official suite:** at least 150 shared cases × 3 repetitions/harness before suite superiority claims; task success noninferiority and no security/unrelated-edit regression.
4. **Default:** opt-in first; decide per model/route and task class only when A/B passes; rollback command and state migration rehearsed.
5. **Subscription:** account quota savings claim is prohibited without observed allowance delta; cached tokens still contribute to rate limits.
