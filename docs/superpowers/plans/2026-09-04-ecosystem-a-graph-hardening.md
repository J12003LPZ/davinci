# Ecosystem Gate A — Graph Hardening Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Eliminate the known Graph Engineering correctness risks and make graph topology, replay, mutation provenance, deadlines, and review coverage trustworthy before deeper ecosystem coupling.

**Architecture:** Harden the existing native graph implementation in place. Typed graph data becomes the source of truth; deterministic code owns transitions and verification; workers remain isolated child processes. Do not add any model calls.

**Tech Stack:** Rust 1.83.0, existing `davinci-coding-agent` graph modules, serde/serde_json already in workspace, git subprocesses through existing helpers.

**Spec:** `docs/superpowers/specs/2026-09-03-graph-engineering-hardening-design.md` and `docs/superpowers/specs/2026-09-04-davinci-ecosystem-integration-design.md`

## Global Constraints

- Zero new model calls.
- Preserve single-writer mutation semantics.
- Verification pass/fail remains deterministic.
- Graph workers remain unable to commit/push/reset/checkout or otherwise mutate Git state.
- Every new failure mode must end in explicit graph state, never implicit approval.
- Tests are local/fixture-only.
- Do not mix ecosystem context/cache/learning work into this gate.

---

### Task 1: Unify Artifact Contract Validation

**Files:**
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/validate.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/types.rs`

**Interfaces:**
- Consumes: existing `ArtifactKind`, submitted JSON artifacts.
- Produces: `pub fn artifact_contract(kind: ArtifactKind) -> ArtifactContract` and one validation path used by both advertised schema and runtime validation.

- [ ] **Step 1: Add a regression test for classification `milestones` drift**

Add an inline test proving the advertised contract and runtime validator agree on whether `milestones` may be absent/null.

```rust
#[test]
fn classification_schema_and_runtime_acceptance_match_for_milestones() {
    let artifact = serde_json::json!({
        "complexity": "simple",
        "reason": "small change"
    });
    let contract = artifact_contract(ArtifactKind::Classification);
    assert_eq!(contract.accepts(&artifact), validate_artifact(ArtifactKind::Classification, &artifact).is_ok());
}
```

- [ ] **Step 2: Run the focused test and verify it fails before refactor**

Run:

```bash
cargo test -p davinci-coding-agent classification_schema_and_runtime_acceptance_match_for_milestones -- --nocapture
```

Expected: FAIL because schema/runtime requirements differ.

- [ ] **Step 3: Introduce one typed contract representation**

Use a small internal structure rather than maintaining independent JSON schema and handwritten requirement lists.

```rust
#[derive(Clone, Copy)]
pub struct FieldRule {
    pub name: &'static str,
    pub required: bool,
    pub allow_null: bool,
    pub kind: FieldKind,
}

pub struct ArtifactContract {
    pub kind: ArtifactKind,
    pub fields: &'static [FieldRule],
}
```

`artifact_schema()` must be generated from `ArtifactContract`; runtime validation must read the same rules.

- [ ] **Step 4: Keep semantic validators separate**

Field shape belongs to the contract. Cross-field semantic checks such as enum combinations or non-empty arrays remain explicit functions invoked after structural validation.

- [ ] **Step 5: Add parity tests for every artifact kind**

For each `ArtifactKind`, create representative valid and invalid values and assert advertised/runtime acceptance parity.

- [ ] **Step 6: Run graph validation tests**

```bash
cargo test -p davinci-coding-agent native_extensions::graph::validate -- --nocapture
```

Expected: PASS.

- [ ] **Step 7: Commit**

```bash
git add crates/davinci-coding-agent/src/native_extensions/graph/validate.rs crates/davinci-coding-agent/src/native_extensions/graph/types.rs
git commit -m "fix(graph): unify artifact schema and runtime contracts"
```

---

### Task 2: Require Real Evidence Provenance

**Files:**
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/validate.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/briefings.rs`

**Interfaces:**
- Consumes: evidence artifact findings.
- Produces: rejection of findings whose `refs` array is empty or contains blank entries.

- [ ] **Step 1: Write failing tests**

```rust
#[test]
fn evidence_finding_rejects_empty_refs() {
    let value = serde_json::json!({
        "findings": [{"summary": "auth lives here", "refs": []}]
    });
    assert!(validate_artifact(ArtifactKind::Evidence, &value).is_err());
}

#[test]
fn evidence_finding_rejects_blank_ref() {
    let value = serde_json::json!({
        "findings": [{"summary": "auth lives here", "refs": ["  "]}]
    });
    assert!(validate_artifact(ArtifactKind::Evidence, &value).is_err());
}
```

- [ ] **Step 2: Run and verify failure**

```bash
cargo test -p davinci-coding-agent evidence_finding_rejects -- --nocapture
```

- [ ] **Step 3: Implement `validate_non_empty_refs`**

```rust
fn validate_non_empty_refs(value: &serde_json::Value) -> Result<(), String> {
    let refs = value.as_array().ok_or("refs must be an array")?;
    if refs.is_empty() || refs.iter().any(|v| v.as_str().is_none_or(|s| s.trim().is_empty())) {
        return Err("evidence finding requires at least one non-empty ref".into());
    }
    Ok(())
}
```

Use Rust-1.83-compatible syntax if `is_none_or` is unavailable; do not raise the toolchain.

- [ ] **Step 4: Update the worker contract text**

`briefings.rs` should say in one sentence that every finding requires at least one concrete reference. Do not expand the worker prompt beyond that.

- [ ] **Step 5: Run tests**

```bash
cargo test -p davinci-coding-agent native_extensions::graph -- --nocapture
```

- [ ] **Step 6: Commit**

```bash
git add crates/davinci-coding-agent/src/native_extensions/graph/validate.rs crates/davinci-coding-agent/src/native_extensions/graph/briefings.rs
git commit -m "fix(graph): require evidence provenance"
```

---

### Task 3: Strict Integer Budget Parsing

**Files:**
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/config.rs`

**Interfaces:**
- Consumes: `.pi/graph.json` numeric budget values.
- Produces: `fn parse_u64_budget(value: &Value, name: &str) -> Result<u64, String>` and `fn parse_usize_budget(...)` with no float truncation.

- [ ] **Step 1: Add failing tests for fractional, negative, and overflowing budgets**

```rust
#[test]
fn integer_budget_rejects_fractional_value() {
    assert!(parse_u64_budget(&serde_json::json!(3.7), "maxWorkers").is_err());
}

#[test]
fn integer_budget_rejects_negative_value() {
    assert!(parse_u64_budget(&serde_json::json!(-1), "maxWorkers").is_err());
}
```

Also test a value greater than `u64::MAX` represented as JSON text/number if serde_json permits it; otherwise test conversion boundaries through the parser helper.

- [ ] **Step 2: Run tests and verify current truncation behavior fails the contract**

```bash
cargo test -p davinci-coding-agent integer_budget_rejects -- --nocapture
```

- [ ] **Step 3: Parse integers as integers**

Use `Value::as_u64()` for non-negative integer values. Never convert through `f64` then cast.

```rust
fn parse_u64_budget(value: &Value, name: &str) -> Result<u64, String> {
    value.as_u64().ok_or_else(|| format!("{name} must be a non-negative whole integer"))
}
```

- [ ] **Step 4: Preserve default-on-invalid behavior only where current config policy requires it**

Malformed config may report and proceed with defaults, but it must not silently reinterpret `3.7` as `3`.

- [ ] **Step 5: Run graph config tests**

```bash
cargo test -p davinci-coding-agent native_extensions::graph::config -- --nocapture
```

- [ ] **Step 6: Commit**

```bash
git add crates/davinci-coding-agent/src/native_extensions/graph/config.rs
git commit -m "fix(graph): reject fractional integer budgets"
```

---

### Task 4: Make Task Success Clear Superseded Errors

**Files:**
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/types.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/controller.rs`

**Interfaces:**
- Produces invariant: `TaskStatus::Succeeded => task.error.is_none()`.

- [ ] **Step 1: Add a retry regression test**

Construct a task that fails attempt 1 and succeeds attempt 2; assert final status is `Succeeded` and `error == None`.

- [ ] **Step 2: Run and verify failure**

```bash
cargo test -p davinci-coding-agent successful_retry_clears_prior_error -- --nocapture
```

- [ ] **Step 3: Centralize terminal task state mutation**

Add helpers such as:

```rust
impl GraphTask {
    pub fn mark_succeeded(&mut self) {
        self.status = TaskStatus::Succeeded;
        self.error = None;
    }

    pub fn mark_failed(&mut self, error: String) {
        self.status = TaskStatus::Failed;
        self.error = Some(error);
    }
}
```

Replace direct success assignments in controller paths.

- [ ] **Step 4: Add an invariant assertion in persistence/status tests**

Persisted successful tasks must deserialize without stale errors.

- [ ] **Step 5: Run graph tests and commit**

```bash
cargo test -p davinci-coding-agent native_extensions::graph -- --nocapture
git add crates/davinci-coding-agent/src/native_extensions/graph/types.rs crates/davinci-coding-agent/src/native_extensions/graph/controller.rs
git commit -m "fix(graph): clear stale task errors after retry"
```

---

### Task 5: Enforce Active Run Deadlines

**Files:**
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/process.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/worker.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/controller.rs`

**Interfaces:**
- Produces: worker execution API accepting an absolute run deadline and terminating the full process tree when it expires.

```rust
pub struct WorkerDeadline {
    pub run_deadline: Option<std::time::Instant>,
    pub role_timeout: Option<std::time::Duration>,
}
```

- [ ] **Step 1: Add a fixture child that sleeps beyond the deadline**

Use the existing worker fixture mechanism; do not invoke a provider.

- [ ] **Step 2: Write failing test**

```rust
#[test]
fn active_run_deadline_kills_running_worker() {
    let result = run_fixture_worker_with_deadline(Duration::from_millis(50));
    assert!(matches!(result, Err(WorkerError::RunDeadlineExceeded)));
}
```

Also assert the child process is gone rather than merely the controller returning.

- [ ] **Step 3: Compute the effective deadline once**

Effective stop time is the earlier of role timeout and absolute run deadline.

- [ ] **Step 4: Reuse existing process-tree termination logic**

Do not introduce a second kill implementation. Route deadline expiry through the same Windows/Unix process-tree abort path used by explicit graph abort.

- [ ] **Step 5: Persist an explicit terminal reason**

Run/task state should record `run deadline exceeded`, not generic worker failure.

- [ ] **Step 6: Run focused tests**

```bash
cargo test -p davinci-coding-agent graph_deadline -- --nocapture
```

- [ ] **Step 7: Commit**

```bash
git add crates/davinci-coding-agent/src/native_extensions/graph/process.rs crates/davinci-coding-agent/src/native_extensions/graph/worker.rs crates/davinci-coding-agent/src/native_extensions/graph/controller.rs
git commit -m "fix(graph): enforce active run deadlines"
```

---

### Task 6: Persist Explicit Graph Topology and Ready-Frontier Scheduling

**Files:**
- Create: `crates/davinci-coding-agent/src/native_extensions/graph/topology.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/mod.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/types.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/controller.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/store.rs`

**Interfaces:**

```rust
pub struct GraphDefinition {
    pub graph_id: String,
    pub version: u32,
    pub nodes: Vec<NodeDefinition>,
    pub edges: Vec<EdgeDefinition>,
}

pub fn build_definition(mode: GraphMode, classification: &Classification) -> GraphDefinition;
pub fn validate_definition(definition: &GraphDefinition) -> Result<(), GraphTopologyError>;
pub fn ready_nodes(definition: &GraphDefinition, state: &GraphRunState) -> Vec<String>;
```

- [ ] **Step 1: Add topology invariant tests**

Test rejection of:

- edge to unknown node;
- unbounded cycle;
- unreachable required node;
- review bypass;
- verify-success edge without verification node;
- two mutation-capable writers that can be ready concurrently.

- [ ] **Step 2: Run tests and confirm new API is missing**

```bash
cargo test -p davinci-coding-agent graph_topology -- --nocapture
```

- [ ] **Step 3: Implement immutable definition types and validator**

Keep topology independent of worker/process code.

- [ ] **Step 4: Build definitions for simple and complex graph modes**

Encode current intended topology, not a new workflow.

- [ ] **Step 5: Persist `GraphDefinition` before the first executable worker starts**

`state.json` or a sibling `graph.json` must carry the immutable definition and version.

- [ ] **Step 6: Replace procedural readiness checks with `ready_nodes`**

Controller still executes nodes; it no longer relies on accidental source order to imply dependencies.

- [ ] **Step 7: Add persisted-state roundtrip tests**

- [ ] **Step 8: Run graph suite**

```bash
cargo test -p davinci-coding-agent native_extensions::graph -- --nocapture
```

- [ ] **Step 9: Commit**

```bash
git add crates/davinci-coding-agent/src/native_extensions/graph
git commit -m "feat(graph): persist explicit execution topology"
```

---

### Task 7: Add Replay Compatibility Fingerprints

**Files:**
- Create: `crates/davinci-coding-agent/src/native_extensions/graph/replay.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/store.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/controller.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/types.rs`

**Interfaces:**

```rust
pub struct ReplayFingerprint {
    pub graph_version: u32,
    pub config_hash: String,
    pub repo_state_hash: String,
    pub input_hash: String,
    pub contract_hash: String,
}

pub fn replay_compatible(stored: &ReplayFingerprint, current: &ReplayFingerprint) -> bool;
```

- [ ] **Step 1: Write compatibility tests**

Same inputs/config/repo/contract => compatible. Change any field => incompatible.

- [ ] **Step 2: Implement deterministic hashing from canonical serialized inputs**

Do not include timestamps or run IDs.

- [ ] **Step 3: Persist the fingerprint with completed reusable nodes**

- [ ] **Step 4: Make resume refuse incompatible node reuse with an explicit reason**

Refusal should cause re-execution, not corrupt the run.

- [ ] **Step 5: Test revision-loop rule remains conservative**

Previously superseded plan/patch nodes must not be replayed even when their fingerprint matches if current policy already forbids reuse after revision entry.

- [ ] **Step 6: Run tests and commit**

```bash
cargo test -p davinci-coding-agent graph_replay -- --nocapture
git add crates/davinci-coding-agent/src/native_extensions/graph/replay.rs crates/davinci-coding-agent/src/native_extensions/graph/store.rs crates/davinci-coding-agent/src/native_extensions/graph/controller.rs crates/davinci-coding-agent/src/native_extensions/graph/types.rs
git commit -m "feat(graph): validate replay compatibility"
```

---

### Task 8: Capture Graph-Owned Mutation Provenance

**Files:**
- Create: `crates/davinci-coding-agent/src/native_extensions/graph/mutation.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/controller.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/types.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/store.rs`

**Interfaces:**

```rust
pub struct MutationBaseline {
    pub files: std::collections::BTreeMap<String, FileFingerprint>,
}

pub struct GraphMutation {
    pub files: Vec<ChangedFile>,
    pub patch_chunks: Vec<PatchChunk>,
}

pub fn capture_baseline(cwd: &Path) -> Result<MutationBaseline, String>;
pub fn capture_graph_delta(cwd: &Path, baseline: &MutationBaseline) -> Result<GraphMutation, String>;
```

- [ ] **Step 1: Add dirty-workspace regression fixture**

Create a temp git repo with one pre-existing uncommitted user edit, then make a distinct writer edit.

- [ ] **Step 2: Write failing test**

Assert `GraphMutation` includes only graph-owned changes and does not attribute the pre-existing edit to the graph.

- [ ] **Step 3: Capture baseline immediately before writer mutation**

Track enough file content/hash information to distinguish pre-existing workspace state from graph-produced state without altering Git state.

- [ ] **Step 4: Capture deterministic post-writer delta**

Include untracked files created by the graph.

- [ ] **Step 5: Persist mutation provenance per writer attempt**

Revision attempts should each have their own delta; final review can compose the current graph-owned result.

- [ ] **Step 6: Run tests and commit**

```bash
cargo test -p davinci-coding-agent graph_mutation -- --nocapture
git add crates/davinci-coding-agent/src/native_extensions/graph/mutation.rs crates/davinci-coding-agent/src/native_extensions/graph/controller.rs crates/davinci-coding-agent/src/native_extensions/graph/types.rs crates/davinci-coding-agent/src/native_extensions/graph/store.rs
git commit -m "fix(graph): track graph-owned mutation provenance"
```

---

### Task 9: Guarantee Complete Review Coverage for Large Diffs

**Files:**
- Create: `crates/davinci-coding-agent/src/native_extensions/graph/review_coverage.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/controller.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/briefings.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/types.rs`

**Interfaces:**

```rust
pub struct ReviewChunk {
    pub id: String,
    pub file: String,
    pub patch: String,
}

pub struct ReviewCoverage {
    pub required_chunk_ids: Vec<String>,
    pub reviewed_chunk_ids: Vec<String>,
}

pub fn chunk_graph_mutation(mutation: &GraphMutation, max_bytes: usize) -> Vec<ReviewChunk>;
pub fn coverage_complete(coverage: &ReviewCoverage) -> bool;
```

- [ ] **Step 1: Add a large-diff fixture that exceeds one reviewer context payload**

- [ ] **Step 2: Write failing test asserting approval is impossible when one chunk is omitted**

- [ ] **Step 3: Chunk by file/hunk with stable IDs**

Do not split in the middle of an individual patch line if avoidable. Keep each chunk independently attributable.

- [ ] **Step 4: Review chunks sequentially or in bounded existing worker fan-out**

Do not introduce an extra summarizer model. Each review worker returns coverage IDs alongside findings.

- [ ] **Step 5: Require full coverage before final approval**

If any required chunk lacks coverage, final graph state is blocked/changes-required, never approved.

- [ ] **Step 6: Ensure final reviewer briefing receives accumulated findings compactly**

Use structured summaries from review artifacts, not raw duplication of every full patch chunk.

- [ ] **Step 7: Run tests and commit**

```bash
cargo test -p davinci-coding-agent graph_review_coverage -- --nocapture
git add crates/davinci-coding-agent/src/native_extensions/graph/review_coverage.rs crates/davinci-coding-agent/src/native_extensions/graph/controller.rs crates/davinci-coding-agent/src/native_extensions/graph/briefings.rs crates/davinci-coding-agent/src/native_extensions/graph/types.rs
git commit -m "fix(graph): require complete mutation review coverage"
```

---

### Task 10: Gate A Verification and Documentation

**Files:**
- Modify: `README.md`
- Modify: `CLAUDE.md`
- Modify: `docs/superpowers/specs/2026-09-03-graph-engineering-hardening-design.md` only if implementation details need a factual status note; do not rewrite approved design intent.

**Interfaces:** none; this is the release gate.

- [ ] **Step 1: Run focused graph tests**

```bash
cargo test -p davinci-coding-agent native_extensions::graph -- --nocapture
```

Expected: PASS.

- [ ] **Step 2: Run workspace quality gates**

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Expected: all PASS.

- [ ] **Step 3: Update documentation to reflect implemented invariants**

Document explicit topology, active deadlines, replay fingerprinting, graph-owned mutation provenance, and complete review coverage.

- [ ] **Step 4: Run a dry graph fixture**

Use existing `--dry-run`/fixture path. Confirm persisted run contains graph definition, replay fingerprint, and no stale success errors.

- [ ] **Step 5: Commit**

```bash
git add README.md CLAUDE.md docs/superpowers/specs/2026-09-03-graph-engineering-hardening-design.md
git commit -m "docs(graph): describe hardened execution invariants"
```

## Gate A Exit Checklist

- [ ] No known correctness risk from the approved hardening spec remains untested.
- [ ] No model call was added.
- [ ] Graph still has one mutation-capable writer at a time.
- [ ] Verification remains deterministic.
- [ ] Every approval path proves verification and review coverage.
- [ ] Workspace fmt/clippy/tests are green.