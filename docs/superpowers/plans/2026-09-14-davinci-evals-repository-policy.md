# Evaluation and Repository Policy Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every new optimization measurable with deterministic offline A/B evidence, finish matched competitor runners, and verify repository protection requirements without fabricating unavailable administration actions.

**Architecture:** Extend `davinci-evals` and existing GitHub Actions only. Keep harness-mode and product-mode competitor reports separate, persist real provider metrics only from real runs, and treat repository branch protection as an external GitHub setting when connector administration is unavailable.

**Tech Stack:** Rust 1.83, existing davinci-evals harness/behavior/competitor modules, existing GitHub Actions workflows.

**Spec:** `docs/superpowers/specs/2026-09-14-davinci-harness-optimization-design.md`

## Global Constraints

- No model judge for deterministic task correctness.
- Infrastructure/configuration failures are not scored as coding-task failures.
- Do not fabricate provider tokens, cache usage, cost, or competitor results.
- Same-model harness comparisons and default-product comparisons must be persisted/reported separately.
- Do not alter GitHub branch protection through code pretending it changes repository settings.

---

### Task 1: Define shared verified-success metrics for optimization A/Bs

**Files:**
- Modify: `crates/davinci-evals/src/harness.rs`
- Modify reporter/table module as needed

**Interfaces:**

Add or extend a persisted measurement record:

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VerifiedRunMetrics {
    pub verified_success: bool,
    pub first_attempt_verified_success: bool,
    pub provider_input_tokens: u64,
    pub provider_output_tokens: u64,
    pub provider_cache_read_tokens: u64,
    pub provider_cache_write_tokens: u64,
    pub provider_cost_usd: Option<f64>,
    pub model_turns: u64,
    pub tool_calls: u64,
    pub retries: u64,
    pub workers: u64,
    pub wall_ms: u64,
    pub scope_violations: u64,
    pub permission_failures: u64,
    pub false_success_claims: u64,
}
```

Aggregate:

```rust
pub fn cost_per_verified_success(runs: &[VerifiedRunMetrics]) -> Option<f64>;
```

Return `None` when no real cost is available or there are zero verified successes; never substitute `0.0` for unknown provider cost.

- [ ] **Step 1: Add metrics aggregation tests**

Cases:
- two successes + one failure with known cost;
- zero successes;
- unknown cost.

- [ ] **Step 2: Run and confirm missing behavior**

```bash
cargo test -p davinci-evals verified_metrics_cost_per_success_is_failures_inclusive
```

- [ ] **Step 3: Implement and serialize metrics**
- [ ] **Step 4: Run reporter tests and commit**

```bash
cargo test -p davinci-evals verified_metrics
cargo test -p davinci-evals reporter
git commit -am "feat(evals): standardize verified-success metrics"
```

---

### Task 2: Register offline ablations for the optimization program

**Files:**
- Modify: `crates/davinci-evals/src/harness.rs` or a focused new `optimization.rs`
- Reuse feature-specific fixtures/tests rather than duplicating production logic

**Interfaces:**

Add named offline ablations:

```text
governor-content-routing-vnext
root-context-budgeting
capability-toolbox-node-query
graph-deferred-schemas
graph-failure-aware-retry
memory-freshness
security-incremental
semantic-navigation
```

Each ablation result records:

```rust
pub struct OptimizationAblationResult {
    pub name: String,
    pub baseline_correct: bool,
    pub candidate_correct: bool,
    pub baseline_units: u64,
    pub candidate_units: u64,
    pub unit_name: String,
}
```

`units` is feature-specific deterministic cost (serialized schema bytes, visible output bytes, worker attempts, files rescanned, etc.), not mislabeled provider tokens.

- [ ] **Step 1: Add registry test asserting every optimization from the spec has a named ablation**
- [ ] **Step 2: Add gate logic**

Candidate is rejected when `baseline_correct && !candidate_correct`, regardless of cost saving.

- [ ] **Step 3: Run**

```bash
cargo test -p davinci-evals optimization_ablation_registry_covers_harness_program
cargo test -p davinci-evals optimization_gate_never_trades_correctness_for_savings
```

- [ ] **Step 4: Commit**

```bash
git commit -am "feat(evals): add optimization ablation registry"
```

---

### Task 3: Finish OpenAI Codex competitor adapter

**Files:**
- Create: `crates/davinci-evals/src/competitor/codex.rs`
- Modify: `crates/davinci-evals/src/competitor/mod.rs`
- Modify command/matched runner only if shared primitives are required

**Interfaces:**

Match the existing Claude adapter shape. Provide:

```rust
pub struct CodexRunner;
impl CompetitorRunner for CodexRunner { ... }
```

Required runner behavior:
- executable resolved from explicit flag/env/path; no install/download;
- isolated temporary fixture workspace;
- timeout enforced;
- stdout/stderr persisted;
- exit status, changed files, deterministic verification oracle, elapsed time recorded;
- provider/model disclosure persisted when configurable;
- unsupported/missing executable => infrastructure failure, not task failure.

- [ ] **Step 1: Add command-construction fixture test**
- [ ] **Step 2: Add missing-binary classification test**
- [ ] **Step 3: Implement using existing competitor command helpers**
- [ ] **Step 4: Run and commit**

```bash
cargo test -p davinci-evals competitor::codex
git commit -am "feat(evals): add matched Codex competitor runner"
```

---

### Task 4: Finish Hermes Agent competitor adapter

**Files:**
- Create: `crates/davinci-evals/src/competitor/hermes.rs`
- Modify: competitor module/CLI dispatch

**Interfaces:**

Implement the same persisted evidence contract as Claude/Codex. Do not assume a specific install layout beyond explicit executable/config input; adapter probe reports unsupported when the required CLI contract is not detectable.

- [ ] **Step 1: Add probe/command fixture tests**
- [ ] **Step 2: Implement fail-closed adapter**
- [ ] **Step 3: Run and commit**

```bash
cargo test -p davinci-evals competitor::hermes
git commit -am "feat(evals): add matched Hermes competitor runner"
```

---

### Task 5: Finish OpenCode competitor adapter

**Files:**
- Create: `crates/davinci-evals/src/competitor/opencode.rs`
- Modify: competitor module/CLI dispatch

**Interfaces:**

Same evidence contract and isolation rules as Tasks 3–4.

- [ ] **Step 1: Add probe/command fixture tests**
- [ ] **Step 2: Implement fail-closed adapter**
- [ ] **Step 3: Run and commit**

```bash
cargo test -p davinci-evals competitor::opencode
git commit -am "feat(evals): add matched OpenCode competitor runner"
```

---

### Task 6: Separate harness comparison from product comparison in persisted reports

**Files:**
- Modify: `crates/davinci-evals/src/competitor/matched.rs`
- Modify: `crates/davinci-evals/src/competitor/report.rs`
- Modify CLI argument parsing if needed

**Interfaces:**

Add:

```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum ComparisonMode {
    Harness,
    Product,
}
```

Persist mode in every comparison manifest and output path. Reporter must refuse to aggregate rows from mixed modes into one ranking table.

- [ ] **Step 1: Add mixed-mode rejection test**
- [ ] **Step 2: Add disclosure test**

Harness mode report includes same-model/model-parity disclosure; Product mode says each system used its configured/default model stack.

- [ ] **Step 3: Implement and run**

```bash
cargo test -p davinci-evals competitor_report_rejects_mixed_comparison_modes
cargo test -p davinci-evals competitor_report_discloses_mode
```

- [ ] **Step 4: Commit**

```bash
git commit -am "feat(evals): separate harness and product comparisons"
```

---

### Task 7: Ensure provider-backed behavior runs preserve real usage and infrastructure status

**Files:**
- Modify behavior A/B runner and persisted manifest types in `crates/davinci-evals/src/behavior/`

**Interfaces:**

Every provider-backed task result must distinguish:

```rust
pub enum RunDisposition {
    Completed,
    VerificationFailed,
    InfrastructureFailure,
    ConfigurationFailure,
    TimedOut,
}
```

If equivalent type already exists, extend/reuse it rather than adding a duplicate.

- [ ] **Step 1: Add fixture where provider credential/model setup fails**

Assert it is excluded from verified-success denominator and is listed as infrastructure/config evidence.

- [ ] **Step 2: Add real-usage passthrough fixture**

Feed known input/cache/output/cost usage and assert exact persistence—no heuristic conversion into billed tokens.

- [ ] **Step 3: Run**

```bash
cargo test -p davinci-evals provider_configuration_failure_is_not_scored_as_task_failure
cargo test -p davinci-evals provider_usage_is_persisted_without_invention
```

- [ ] **Step 4: Commit**

```bash
git commit -am "fix(evals): separate provider infrastructure failures from task results"
```

---

### Task 8: Add one command that verifies the full optimization offline gate

**Files:**
- Modify: `crates/davinci-evals/src/main.rs` / CLI module
- Modify README docs

**Interfaces:**

Add command:

```bash
cargo run -p davinci-evals -- optimization gate --offline
```

It runs only deterministic/offline optimization ablations and writes a report artifact. It must not invoke provider or competitor binaries.

- [ ] **Step 1: Add CLI parse test**
- [ ] **Step 2: Add execution fixture asserting no external runner is called**
- [ ] **Step 3: Implement aggregation/gate**
- [ ] **Step 4: Run and commit**

```bash
cargo test -p davinci-evals optimization_gate_offline_never_invokes_external_runner
cargo run -p davinci-evals -- optimization gate --offline
git commit -am "feat(evals): add offline harness optimization gate"
```

---

### Task 9: Verify and document repository protection requirements

**Files:**
- Modify: `docs/release-quality.md` or current repository quality-policy doc
- Modify CI workflow only if required check names are not stable/clear

**Interfaces:**

Document desired GitHub settings exactly:

```text
main:
  require pull request or equivalent protected update path
  require CI / quality
  require Workflow lint / actionlint
  require Security SARIF interoperability / schema (or actual stable check name)
  block force pushes
  block branch deletion
```

Do not claim the settings are enabled unless verified through GitHub API after an authorized administration action.

- [ ] **Step 1: Fetch current branch/ruleset state through GitHub during execution**

If the connector exposes an administration mutation action, apply the approved policy and immediately re-read it. If it does not, record `external action required` in the final report.

- [ ] **Step 2: Ensure check names in docs match actual current GitHub Actions job names**
- [ ] **Step 3: Commit documentation only**

```bash
git commit -am "docs(ci): define required main protection checks"
```

---

### Task 10: Final eval subsystem verification

- [ ] **Step 1: Run eval crate tests**

```bash
cargo test -p davinci-evals
```

- [ ] **Step 2: Verify corpus**

```bash
cargo run -p davinci-evals -- corpus verify
```

- [ ] **Step 3: Run offline optimization gate**

```bash
cargo run -p davinci-evals -- optimization gate --offline
```

- [ ] **Step 4: Run competitor probe only, not live benchmark**

Probe must report installed/missing/unsupported runners without scoring tasks. Do not launch provider-backed competitors unless explicitly configured at execution time.

- [ ] **Step 5: Commit any generated source/doc changes only; do not commit transient benchmark artifacts unless the repository already tracks them**
