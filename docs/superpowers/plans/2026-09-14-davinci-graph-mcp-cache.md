# Graph, MCP, and Cache Effectiveness Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Reduce wasted Graph retries and provider schema/cache overhead while preserving role authorization and existing provider contracts.

**Architecture:** Extend Graph's existing retry loop, worker specification, cache telemetry, `RuntimeCapabilityRegistry`, and `ToolExposureState`. Role authorization remains unchanged; schema exposure becomes a separate provider-view concern. MCP uses the same registry/exposure machinery rather than a second discovery system.

**Tech Stack:** Rust 1.83, existing graph controller/worker, davinci-agent runtime capability registry, MCP registry, provider usage telemetry.

**Spec:** `docs/superpowers/specs/2026-09-14-davinci-harness-optimization-design.md`

## Global Constraints

- Do not change Graph's role permission policy while changing schema exposure.
- `retrieve_output` remains available whenever a worker can produce compressible output.
- Environment/configuration failures do not get model retries when retry cannot repair them.
- Local cache identities must never be reported as actual provider cache hits.
- MCP discovery cannot elevate permission.

---

### Task 1: Classify Graph failure causes deterministically

**Files:**
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/controller.rs`
- Modify graph worker result/error type if needed in `graph/worker.rs` or graph types module

**Interfaces:**

Add:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WorkerFailureClass {
    Timeout,
    ArtifactInvalid,
    VerificationFailed,
    PermissionRefused,
    Environment,
    PlanInvalidated,
    ProcessFailure,
    Unknown,
}

pub fn classify_worker_failure(result: &WorkerResult, last_failure: Option<&str>) -> WorkerFailureClass;
```

Prefer typed signals already present in `WorkerResult`/artifacts. Use string markers only as compatibility fallback.

- [ ] **Step 1: Add table-driven classifier tests**

Cases: timeout, missing/invalid `graph_submit`, permission refusal, spawn/missing binary/environment error, plan invalidation, generic process exit.

- [ ] **Step 2: Run and confirm failure**

```bash
cargo test -p davinci-coding-agent graph_failure_classification_is_deterministic
```

- [ ] **Step 3: Implement classifier**

Keep classifier pure and side-effect free.

- [ ] **Step 4: Commit**

```bash
cargo test -p davinci-coding-agent graph_failure_classification_is_deterministic
git commit -am "feat(graph): classify worker failure causes"
```

---

### Task 2: Make retry policy failure-specific

**Files:**
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/controller.rs`

**Interfaces:**

Add:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RetryDecision {
    RetrySameBudget,
    RetryExtendedTimeout,
    ReviseWriter,
    Replan,
    Stop,
}

pub fn retry_decision(class: WorkerFailureClass, attempt: usize) -> RetryDecision;
```

Required baseline behavior:

```text
Timeout, first retry          -> RetryExtendedTimeout
ArtifactInvalid              -> RetrySameBudget
VerificationFailed           -> ReviseWriter
PlanInvalidated              -> Replan
PermissionRefused            -> Stop
Environment                  -> Stop
ProcessFailure, first retry  -> RetrySameBudget
Unknown                      -> existing bounded retry rule
```

- [ ] **Step 1: Add policy test**

Assert environment/permission failure never schedules a second model worker attempt.

- [ ] **Step 2: Run and confirm current generic loop would retry**

```bash
cargo test -p davinci-coding-agent environment_failure_does_not_burn_graph_retry
```

- [ ] **Step 3: Wire decision into current retry/revision/replan loop**

Preserve existing `maxRevisionCycles`, `maxReplans`, deadlines, and `NODE_ATTEMPTS` upper bounds. The new policy may reduce retries, never exceed current caps.

- [ ] **Step 4: Run Graph retry tests**

```bash
cargo test -p davinci-coding-agent environment_failure_does_not_burn_graph_retry
cargo test -p davinci-coding-agent graph::controller
```

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(graph): make retries failure-aware"
```

---

### Task 3: Add a bounded failure-specific context delta

**Files:**
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/controller.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/ecosystem/context.rs`

**Interfaces:**

Add:

```rust
pub const RETRY_CONTEXT_DELTA_TOKENS: usize = 400;

pub fn build_retry_context_delta(
    memory: &VectorMemory,
    learning: &LearningController,
    role: Role,
    base_query: &WorkerContextQuery,
    failure_class: WorkerFailureClass,
    diagnostic_summary: &str,
) -> ContextPacket;
```

Rules:
- return empty for failure classes that do not change information need (e.g. pure artifact-format retry);
- include failure class + bounded diagnostic text in a derived query;
- cap delta at 400 tokens;
- do not mutate the base packet/fingerprint;
- append delta after stable base packet in retry briefing.

- [ ] **Step 1: Add failing test**

A verification failure mentioning a specific file/symbol should yield a non-empty <=400-token delta; an artifact-format failure should yield empty delta.

- [ ] **Step 2: Confirm failure**

```bash
cargo test -p davinci-coding-agent retry_context_delta_is_bounded_and_failure_specific
```

- [ ] **Step 3: Implement and wire only on retries**

The first-attempt worker cache identity must remain unchanged.

- [ ] **Step 4: Run focused tests and commit**

```bash
cargo test -p davinci-coding-agent retry_context_delta_is_bounded_and_failure_specific
git commit -am "feat(graph): add bounded retry context deltas"
```

---

### Task 4: Reuse `ToolExposureState` for Graph worker provider schemas

**Files:**
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/controller.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/worker.rs`
- Modify only minimal host plumbing needed to set initial exposure in child agent
- Reuse: `crates/davinci-agent/src/runtime/capabilities.rs`

**Interfaces:**

Extend `WorkerSpec` with:

```rust
pub authorized_tools: Vec<String>,
pub initially_exposed_tools: Vec<String>,
```

If `WorkerSpec` already has `tools`, migrate semantics explicitly:

```text
authorized_tools = role permission surface
initially_exposed_tools = provider schema view
```

Recommended initial sets:

```text
Classifier: graph_submit only
Researcher: read, grep, find, graph_submit, tool_search, retrieve_output when needed
Planner: read, grep, find, ls, graph_submit, tool_search
Writer: read, grep, find, ls, edit/apply_patch as current writer contract allows, graph_submit, tool_search, retrieve_output when needed
Reviewer/TestAnalyzer: minimal read/search/shell verifier set + graph_submit/tool_search/retrieve_output as required
```

- [ ] **Step 1: Add synthetic Graph worker schema-size test**

Generate a role with authorized extras including ~100 fake extension/MCP tools. Assert the initial provider schema set is a strict subset, role authorization is unchanged, and a deferred authorized tool is discoverable through `tool_search`.

- [ ] **Step 2: Confirm failure**

```bash
cargo test -p davinci-coding-agent graph_worker_defers_authorized_schemas_without_changing_permissions
```

- [ ] **Step 3: Thread exposure into worker child configuration**

Do not encode hidden permissions in prompt text. Use existing runtime/exposure state or a deterministic child startup argument/env consumed by the normal agent exposure initialization.

- [ ] **Step 4: Guarantee Governor recovery tool**

If any initially exposed or dynamically activatable authorized tool is compressible, `retrieve_output` must be available for the worker. Reuse `ensure_governor_recovery_tool`.

- [ ] **Step 5: Run Graph exposure and role-guard tests**

```bash
cargo test -p davinci-coding-agent graph_worker_defers_authorized_schemas_without_changing_permissions
cargo test -p davinci-coding-agent ecosystem_loop_governor_recovery
cargo test -p davinci-coding-agent graph::worker_hooks
```

- [ ] **Step 6: Commit**

```bash
git commit -am "feat(graph): defer worker schemas behind tool search"
```

---

### Task 5: Apply progressive schema disclosure to large MCP catalogs

**Files:**
- Modify: `crates/davinci-agent/src/mcp.rs` or current MCP registry implementation
- Modify: `crates/davinci-agent/src/tools.rs` (`tool_search_tool`)
- Modify runtime/host MCP attachment path only as required

**Interfaces:**

At MCP registration time, every tool must register:

```text
name
source=CapabilitySource::Mcp
description
schema/schema_hash
read_only/tool_class
version when available
```

But authorization and exposure remain separate.

- [ ] **Step 1: Add large-catalog test**

Register 200 MCP tools, authorize them, expose only the normal hot set + `tool_search`, search for one exact MCP tool by namespace/description, activate it, and assert the next provider tool projection contains only the activated schema in addition to the initial set.

- [ ] **Step 2: Add denied MCP search test**

A catalog entry not in the active authorization surface must not activate even if `tool_search` finds a textual match.

- [ ] **Step 3: Run and confirm current behavior does not satisfy large-catalog exposure**

```bash
cargo test -p davinci-agent mcp_large_catalog_uses_progressive_schema_exposure
cargo test -p davinci-agent tool_search_cannot_activate_denied_mcp_tool
```

- [ ] **Step 4: Wire MCP registrations through existing capability registry/exposure state**

Do not create `McpExposureRegistry` or a second catalog.

- [ ] **Step 5: Run MCP/tool-search tests and commit**

```bash
cargo test -p davinci-agent mcp
cargo test -p davinci-agent tool_search
git commit -am "feat(mcp): defer large catalog schemas progressively"
```

---

### Task 6: Report real provider cache effectiveness by Graph role

**Files:**
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/worker.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/ecosystem/cache_affinity.rs`
- Modify: ecosystem/graph telemetry type that persists run stats

**Interfaces:**

Add role-level counters:

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RoleCacheStats {
    pub input_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub turns: u64,
}
```

Keep local identity/miss reasons separate:

```rust
pub struct CacheObservation {
    pub role: Role,
    pub provider_usage: RoleCacheStats,
    pub identity: CacheIdentity,
    pub miss_reasons: Vec<CacheMissReason>,
}
```

- [ ] **Step 1: Add telemetry aggregation test**

Feed two researcher worker message-end events with known `cacheRead/cacheWrite` usage and one writer event. Assert per-role totals are exact.

- [ ] **Step 2: Add wording/serialization guard**

Ensure a local unchanged cache identity with zero provider `cacheRead` does not serialize as a provider cache hit.

- [ ] **Step 3: Run tests**

```bash
cargo test -p davinci-coding-agent graph_cache_stats_aggregate_provider_usage_by_role
cargo test -p davinci-coding-agent local_cache_identity_is_not_reported_as_provider_hit
```

- [ ] **Step 4: Implement aggregation and reason-coded identity diff**

For consecutive compatible worker attempts, diff current vs previous `CacheIdentity` and persist reason codes only as diagnostic causes. Actual cache-read ratio is computed from provider counters:

```text
cache_read_ratio = cache_read_tokens / max(input_tokens + cache_read_tokens, 1)
```

- [ ] **Step 5: Run cache/worker tests and commit**

```bash
cargo test -p davinci-coding-agent cache_affinity
cargo test -p davinci-coding-agent graph::worker
cargo clippy -p davinci-coding-agent --all-targets -- -D warnings
git commit -am "feat(graph): report provider cache effectiveness by role"
```

---

### Task 7: Add offline schema and retry ablations

**Files:**
- Modify existing ecosystem/eval ablation location

**Interfaces:**

Two deterministic comparisons:

1. Graph role full schema catalog vs deferred provider schema set.
2. Generic retry policy vs failure-aware policy on synthetic failure sequences.

- [ ] **Step 1: Add large-catalog schema fixture**

Acceptance:
- initial serialized schema bytes reduced by at least 30%;
- required deferred tool remains discoverable/callable;
- authorization set identical.

- [ ] **Step 2: Add retry fixture**

Synthetic sequence includes environment failure, timeout, artifact invalid, verification failure. Candidate must use fewer model worker attempts without changing the deterministic terminal disposition.

- [ ] **Step 3: Run and commit**

```bash
cargo test -p davinci-coding-agent graph_deferred_schema_ablation
cargo test -p davinci-coding-agent graph_retry_policy_ablation
git commit -am "test(graph): add schema and retry ablations"
```
