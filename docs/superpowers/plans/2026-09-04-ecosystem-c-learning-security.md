# Ecosystem Gate C — Learning Feedback and Security Integration Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the learning return path so verified work improves future Graph workers, while adding risk-triggered native security verification without making every task heavier.

**Architecture:** Use exact skill/version provenance from Gate B, a shared deterministic `VerificationBundle`, and a no-model diff/path risk classifier. Learning review becomes signal-gated, but memory indexing stays automatic. Security is inserted only when deterministic risk evidence warrants it by default.

**Tech Stack:** Rust 1.83.0, existing learning/vector-memory/security native extensions, existing graph verification/review path.

**Spec:** `docs/superpowers/specs/2026-09-04-davinci-ecosystem-integration-design.md`

## Global Constraints

- No new director/orchestrator model call.
- Security risk classification uses deterministic code, never a model.
- Default security mode is `risk`, not `always`.
- Learning stays asynchronous/fail-open for foreground turns.
- Memory indexing continues even when the model-backed learning reviewer is skipped.
- Skill outcomes must be attributed to exact injected versions/hashes, not inferred by prompt string search when graph metadata is available.
- Required deterministic test failures always block graph approval.
- Required security failures block graph approval.
- Learning optimization must be measured against artifact quality, not assumed from lower token counts.

---

### Task 1: Introduce a Unified Verification Bundle

**Files:**
- Create: `crates/davinci-coding-agent/src/native_extensions/ecosystem/verification.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/ecosystem/mod.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/verify.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/learning/evidence.rs`

**Interfaces:**

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerificationBundle {
    pub commands_ran: usize,
    pub commands_failed: usize,
    pub deterministic_passed: bool,
    pub security: SecurityVerification,
    pub changed_files: Vec<String>,
    pub graph_run_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SecurityVerification {
    NotRequired,
    Passed { scan_id: String },
    Failed { scan_id: String, blockers: usize },
    Unavailable { reason: String },
}
```

- [ ] **Step 1: Add bundle semantics tests**

```rust
#[test]
fn deterministic_failure_never_passes_bundle() {
    let bundle = VerificationBundle {
        commands_ran: 1,
        commands_failed: 1,
        deterministic_passed: false,
        security: SecurityVerification::NotRequired,
        changed_files: vec![],
        graph_run_id: None,
    };
    assert!(!bundle.approval_eligible(SecurityPolicyMode::Risk));
}
```

Also pin required-security failure and unavailable behavior for each policy mode.

- [ ] **Step 2: Run and verify missing API failure**

```bash
cargo test -p davinci-coding-agent verification_bundle -- --nocapture
```

- [ ] **Step 3: Implement pure approval semantics**

`approval_eligible` must be deterministic and small. It must not know about model review content.

- [ ] **Step 4: Adapt graph verification results into bundle**

Preserve existing detailed command results. `VerificationBundle` is the cross-subsystem summary, not a replacement for logs.

- [ ] **Step 5: Adapt learning evidence construction to consume the bundle**

Remove duplicated boolean derivations where practical.

- [ ] **Step 6: Run focused tests and commit**

```bash
cargo test -p davinci-coding-agent verification_bundle -- --nocapture
git add crates/davinci-coding-agent/src/native_extensions/ecosystem crates/davinci-coding-agent/src/native_extensions/graph/verify.rs crates/davinci-coding-agent/src/native_extensions/learning/evidence.rs
git commit -m "feat(ecosystem): unify verification evidence"
```

---

### Task 2: Attribute Graph Skill Outcomes to Exact Injected Versions

**Files:**
- Modify: `crates/davinci-coding-agent/src/native_extensions/learning/types.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/learning/store.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/learning/mod.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/types.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/controller.rs`

**Interfaces:**

```rust
pub struct SkillVersionRef {
    pub name: String,
    pub version: u64,
    pub content_hash: String,
}

pub fn record_skill_version_outcome(
    &mut self,
    skill: &SkillVersionRef,
    outcome: SkillOutcome,
) -> Result<(), String>;
```

- [ ] **Step 1: Add version-attribution tests**

Create skill v1 and v2, inject v1 metadata, record success, and assert only v1's metrics change.

- [ ] **Step 2: Run test and verify current name-only outcome API is insufficient**

- [ ] **Step 3: Add version-aware ledger lookup/update**

If the stored version/hash no longer exists, record a diagnostic and do not attribute the outcome to a newer version.

- [ ] **Step 4: Make graph completion derive outcome from `VerificationBundle`**

Rules:

```text
commands_ran > 0 && bundle approval evidence passed -> VerifiedSuccess
commands_ran > 0 && deterministic/security required evidence failed -> VerifiedFailure
otherwise -> Neutral
```

Review-model disagreement alone must not transform a deterministic success into a verified failure metric unless the graph's final state is changes-required for a real code issue; preserve the distinction in evidence if needed.

- [ ] **Step 5: Record every injected graph skill version once per applicable verification result**

Do not rely on `<skill name=...>` prompt-string detection for graph workers.

- [ ] **Step 6: Keep normal foreground skill outcome path backward-compatible**

Normal agent turns may continue name/metadata tracking until separately improved; do not regress them.

- [ ] **Step 7: Test and commit**

```bash
cargo test -p davinci-coding-agent graph_skill_outcome -- --nocapture
git add crates/davinci-coding-agent/src/native_extensions/learning crates/davinci-coding-agent/src/native_extensions/graph
git commit -m "feat(learning): attribute graph outcomes to skill versions"
```

---

### Task 3: Add Deterministic Learning Review Gating

**Files:**
- Modify: `crates/davinci-coding-agent/src/native_extensions/learning/evidence.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/learning/mod.rs`
- Modify: `crates/davinci-coding-agent/src/main.rs`

**Interfaces:**

```rust
pub fn should_review_evidence(evidence: &LearningEvidence) -> bool;
```

- [ ] **Step 1: Define fixture cases before implementation**

Reviewer **must run** when any of these occur:

- files mutated;
- deterministic verification commands ran;
- graph run completed or failed;
- injected skill received verified outcome;
- user correction/rejection signal exists;
- repeated tool failures meet existing failure-lesson threshold;
- explicit `/learn` request.

Reviewer **must skip** a read-only explanatory turn with no durable signal.

- [ ] **Step 2: Write table-driven failing test**

```rust
#[test]
fn learning_review_gate_matches_durable_signal_policy() {
    for case in review_gate_cases() {
        assert_eq!(should_review_evidence(&case.evidence), case.expected, "{}", case.name);
    }
}
```

- [ ] **Step 3: Implement pure signal gate**

No token estimation or model confidence guessing in this function.

- [ ] **Step 4: Keep vector-memory indexing before the gate**

The settled-turn order must remain:

```text
AgentSettled -> memory indexing -> evidence build -> should_review -> optional background reviewer
```

- [ ] **Step 5: Record skip/dispatch counters for later telemetry**

Do not print noisy notices for every skip.

- [ ] **Step 6: Add a completer invocation-count regression test**

A low-signal read-only turn should index memory but make zero learning-review model calls.

- [ ] **Step 7: Commit**

```bash
git add crates/davinci-coding-agent/src/native_extensions/learning crates/davinci-coding-agent/src/main.rs
git commit -m "perf(learning): skip low-signal background reviews"
```

---

### Task 4: Build a Learning Review Fixture Benchmark

**Files:**
- Create: `crates/davinci-coding-agent/src/native_extensions/learning/benchmark.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/learning/mod.rs`
- Add fixture data under the repository's existing fixture convention if needed; do not create a network benchmark.

**Interfaces:**

```rust
pub struct LearningBenchmarkResult {
    pub median_input_tokens: u64,
    pub dispatched_reviews: usize,
    pub accepted_high_confidence_artifacts: usize,
}
```

- [ ] **Step 1: Define a mixed settled-turn corpus**

Include:

- read-only explanations;
- successful code edits + tests;
- failed edits;
- graph success/failure evidence;
- user correction;
- explicit learn request;
- repeated tool failure.

- [ ] **Step 2: Capture ungated baseline using deterministic reviewer fixture responses**

Do not call a real model. Feed the same canned reviewer outputs where a review is dispatched.

- [ ] **Step 3: Capture gated result**

- [ ] **Step 4: Assert target metrics**

```rust
assert!(gated.median_input_tokens * 100 <= baseline.median_input_tokens * 60);
let allowed_loss = ((baseline.accepted_high_confidence_artifacts as f64) * 0.05).ceil() as usize;
assert!(baseline.accepted_high_confidence_artifacts.saturating_sub(gated.accepted_high_confidence_artifacts) <= allowed_loss);
```

Use integer-safe equivalent if avoiding float in tests is cleaner.

- [ ] **Step 5: If quality target fails, adjust the deterministic gate, not reviewer prompt verbosity first**

- [ ] **Step 6: Commit**

```bash
git add crates/davinci-coding-agent/src/native_extensions/learning
git commit -m "test(learning): benchmark review gating efficiency"
```

---

### Task 5: Add Deterministic Change-Risk Classification

**Files:**
- Create: `crates/davinci-coding-agent/src/native_extensions/ecosystem/risk.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/ecosystem/mod.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/mutation.rs`

**Interfaces:**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeRisk {
    None,
    Low,
    High,
}

pub struct RiskReason {
    pub surface: &'static str,
    pub path: String,
}

pub struct RiskAssessment {
    pub level: ChangeRisk,
    pub reasons: Vec<RiskReason>,
}

pub fn assess_change_risk(mutation: &GraphMutation) -> RiskAssessment;
```

- [ ] **Step 1: Add table-driven path/diff fixtures**

High-risk examples include auth/permission modules, credential stores, shell/process execution, network/TLS, crypto, manifests/lockfiles, protocol/deserialization, filesystem traversal/archive extraction, extension loading.

Low-risk examples include TUI copy, docs, pure formatting helpers unrelated to a sensitive boundary.

- [ ] **Step 2: Write failing classification tests**

- [ ] **Step 3: Implement conservative transparent rules**

Rules should be inspectable path/diff indicators, not a giant semantic static analyzer. Return reasons so operators can see why a gate triggered.

- [ ] **Step 4: Add explicit tests against false-positive-prone generic names**

For example, a documentation sentence containing "token" should not automatically be high risk.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-coding-agent/src/native_extensions/ecosystem/risk.rs crates/davinci-coding-agent/src/native_extensions/ecosystem/mod.rs crates/davinci-coding-agent/src/native_extensions/graph/mutation.rs
git commit -m "feat(ecosystem): classify graph mutation risk"
```

---

### Task 6: Add Security Verification Policy Configuration

**Files:**
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/config.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/ecosystem/verification.rs`

**Interfaces:**

```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SecurityPolicyMode {
    Off,
    Risk,
    Always,
}
```

Default: `Risk`.

- [ ] **Step 1: Add config parsing/default tests**

```rust
#[test]
fn graph_security_verification_defaults_to_risk() {
    assert_eq!(GraphConfig::default().security_verification, SecurityPolicyMode::Risk);
}
```

- [ ] **Step 2: Add malformed-value test**

Malformed values report config error and use existing config fallback policy; never silently mean `off`.

- [ ] **Step 3: Implement config field**

Example:

```json
{
  "securityVerification": "risk"
}
```

- [ ] **Step 4: Test approval semantics for scanner unavailable**

- `off`: NotRequired.
- `risk`: required high-risk scan unavailable => explicit `Unavailable`, fail-open with warning.
- `always`: unavailable => fail-closed.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-coding-agent/src/native_extensions/graph/config.rs crates/davinci-coding-agent/src/native_extensions/ecosystem/verification.rs
git commit -m "feat(graph): configure security verification policy"
```

---

### Task 7: Expose a Non-Interactive Security Verification Entry Point

**Files:**
- Modify: `crates/davinci-coding-agent/src/native_extensions/security_scan.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/mod.rs`

**Interfaces:**

```rust
pub struct SecurityVerifyRequest<'a> {
    pub cwd: &'a Path,
    pub changed_files: &'a [String],
    pub graph_run_id: &'a str,
}

pub fn verify_changed_surface(
    &mut self,
    request: SecurityVerifyRequest<'_>,
) -> Result<SecurityVerification, String>;
```

- [ ] **Step 1: Add local fixture scan test**

Sensitive fixture with known blocker => `Failed { blockers: > 0 }`. Clean fixture => `Passed`.

- [ ] **Step 2: Reuse existing scanner lifecycle/analysis primitives**

Do not duplicate vulnerability rules in ecosystem code.

- [ ] **Step 3: Scope scan to graph-owned changed surface where supported**

Do not automatically deep-scan the whole repository for every risk-triggered edit.

- [ ] **Step 4: Ensure no model/network dependency**

Security verification path must stay deterministic/local unless existing scanner optional functionality is explicitly configured; fixture tests disable network.

- [ ] **Step 5: Return structured result rather than transcript text**

- [ ] **Step 6: Commit**

```bash
git add crates/davinci-coding-agent/src/native_extensions/security_scan.rs crates/davinci-coding-agent/src/native_extensions/mod.rs
git commit -m "feat(security): add graph verification entry point"
```

---

### Task 8: Insert Conditional Security Gate Between Tests and Review

**Files:**
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/controller.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/topology.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/types.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/ecosystem/verification.rs`

**Interfaces:**
- Consumes: `GraphMutation`, `RiskAssessment`, `SecurityPolicyMode`, security verifier.
- Produces: `VerificationBundle.security` before review.

- [ ] **Step 1: Add high-risk graph fixture**

Writer mutates an auth/shell-sensitive fixture file; deterministic tests pass; security fixture reports blocker.

- [ ] **Step 2: Write failing test asserting review approval is unreachable**

Final run must be blocked/changes-required when required security fails.

- [ ] **Step 3: Insert security node/state only when policy requires it**

For `risk`, `High` => run security. `Low/None` => `NotRequired`. For `always`, every mutation run receives security verification. `off` never does.

- [ ] **Step 4: Preserve explicit topology**

If Gate A topology is immutable per run, include a deterministic conditional security node/edge in the definition rather than procedurally sneaking a step into controller code.

- [ ] **Step 5: Make review briefing receive compact verification result**

Do not dump the full security report unless needed; provide blockers/findings through existing artifact/report references.

- [ ] **Step 6: Add clean/high-risk/unavailable policy tests**

- [ ] **Step 7: Commit**

```bash
git add crates/davinci-coding-agent/src/native_extensions/graph crates/davinci-coding-agent/src/native_extensions/ecosystem/verification.rs
git commit -m "feat(graph): gate risky changes on security verification"
```

---

### Task 9: Close the Full Graph Learning Loop

**Files:**
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/controller.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/learning/mod.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/vector_memory.rs` only if fixture control needs a deterministic local embedding/retrieval hook

**Interfaces:** full closed-loop behavior.

- [ ] **Step 1: Build offline Run #1 fixture**

Run #1 receives no learned skill, performs verified workflow, and produces deterministic learning fixture output that activates a project skill and/or high-confidence memory.

- [ ] **Step 2: Assert persistence after Run #1**

Check the learning ledger/SKILL.md/vector-memory fixture store.

- [ ] **Step 3: Build Run #2 with a related goal**

Context packet builder must select the persisted memory/skill under Gate B limits.

- [ ] **Step 4: Assert exact provenance in Run #2 metadata**

Skill version/hash and memory IDs must match Run #1 persisted artifacts.

- [ ] **Step 5: Complete Run #2 with successful deterministic verification**

- [ ] **Step 6: Assert exact skill version success count increments**

This is the core full-circle proof.

- [ ] **Step 7: Assert no extra coordinator model invocation**

Fixture completer call count must equal graph node model calls + expected learning reviewer calls only.

- [ ] **Step 8: Commit**

```bash
git add crates/davinci-coding-agent/src/native_extensions
git commit -m "test(ecosystem): prove graph learning feedback loop"
```

---

### Task 10: Gate C Verification

**Files:**
- Modify: `README.md`
- Modify: `CLAUDE.md`
- Modify: `docs/learning.md`

- [ ] **Step 1: Run learning/security/ecosystem focused tests**

```bash
cargo test -p davinci-coding-agent learning -- --nocapture
cargo test -p davinci-coding-agent security -- --nocapture
cargo test -p davinci-coding-agent ecosystem -- --nocapture
```

- [ ] **Step 2: Run learning benchmark and record fixture metrics in test output/docs**

Require >=40% median review input token reduction and <=5% relative loss of accepted high-confidence artifacts.

- [ ] **Step 3: Run workspace gates**

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

- [ ] **Step 4: Update docs**

Document review gating, exact skill attribution, security `off|risk|always`, and the full graph-learning loop.

- [ ] **Step 5: Commit**

```bash
git add README.md CLAUDE.md docs/learning.md
git commit -m "docs(ecosystem): describe learning and security feedback"
```

## Gate C Exit Checklist

- [ ] Verified graph work can improve later graph work.
- [ ] Skill outcomes are version-accurate.
- [ ] Low-signal turns avoid reviewer model cost but keep memory indexing.
- [ ] Learning benchmark meets both efficiency and quality thresholds.
- [ ] High-risk mutations trigger security by default.
- [ ] Required security failures cannot be approved.
- [ ] No new orchestration model call exists.
- [ ] Full-circle fixture passes offline.
- [ ] Workspace fmt/clippy/tests are green.
