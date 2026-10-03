# Ecosystem Gate D — Proof, CI, Observability, and Hygiene Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the integrated ecosystem measurable, continuously enforced, and easier for coding agents to reason about by adding compact structured telemetry, offline closed-loop tests, repository CI, and evidence-based dead-code cleanup.

**Architecture:** Reuse existing status/run-stat surfaces instead of creating a new monitoring subsystem. Integration proof lives in offline fixture tests and GitHub Actions. Cleanup happens only after functional integration is proven and remains separate from behavior changes.

**Tech Stack:** Rust 1.83.0, existing Davinci TUI/status structures, GitHub Actions, existing fixture infrastructure.

**Spec:** `docs/superpowers/specs/2026-09-04-davinci-ecosystem-integration-design.md`

## Global Constraints

- Telemetry adds zero model calls and is never injected into prompts automatically.
- CI uses fixture-only tests and no provider/network secrets.
- Do not add a new analytics service or external database.
- Do not delete code merely because it looks unused; prove it is uncompiled/unreferenced.
- Dead-code removal is separated from functional integration commits.
- Keep status output compact and operator-oriented.

---

### Task 1: Define Structured Ecosystem Telemetry

**Files:**
- Create: `crates/davinci-coding-agent/src/native_extensions/ecosystem/telemetry.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/ecosystem/mod.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/types.rs`

**Interfaces:**

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EcosystemStats {
    pub memory_hits: u64,
    pub memory_injected_tokens: u64,
    pub skill_candidates_considered: u64,
    pub skills_injected: u64,
    pub skill_injected_tokens: u64,
    pub context_packet_tokens: u64,
    pub context_fingerprint: Option<String>,
    pub governor_bytes_omitted: u64,
    pub governor_retrievals: u64,
    pub prunings: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub graph_workers: u64,
    pub graph_cost_usd: f64,
    pub security_gate_triggered: bool,
    pub security_result: Option<String>,
    pub learning_reviews_dispatched: u64,
    pub learning_reviews_skipped: u64,
    pub learned_artifacts_applied: u64,
}
```

- [ ] **Step 1: Write serialization/default tests**

```rust
#[test]
fn ecosystem_stats_default_and_roundtrip_are_stable() {
    let stats = EcosystemStats::default();
    let json = serde_json::to_string(&stats).unwrap();
    let decoded: EcosystemStats = serde_json::from_str(&json).unwrap();
    assert_eq!(decoded.memory_hits, 0);
    assert_eq!(decoded.graph_cost_usd, 0.0);
}
```

Also deserialize a persisted graph-state fixture lacking these fields and assert defaults preserve backward compatibility.

- [ ] **Step 2: Run test to verify the type is missing**

```bash
cargo test -p davinci-coding-agent ecosystem_stats -- --nocapture
```

Expected: FAIL before implementation.

- [ ] **Step 3: Implement the data type and export it from `ecosystem/mod.rs`**

Keep the type free of subsystem ownership or behavior.

- [ ] **Step 4: Add `ecosystem_stats: EcosystemStats` to persisted graph run state with `#[serde(default)]`**

- [ ] **Step 5: Run focused tests**

```bash
cargo test -p davinci-coding-agent ecosystem_stats -- --nocapture
```

Expected: PASS.

- [ ] **Step 6: Commit**

```bash
git add crates/davinci-coding-agent/src/native_extensions/ecosystem/telemetry.rs crates/davinci-coding-agent/src/native_extensions/ecosystem/mod.rs crates/davinci-coding-agent/src/native_extensions/graph/types.rs
git commit -m "feat(ecosystem): add structured integration telemetry"
```

---

### Task 2: Wire Telemetry at Existing Subsystem Boundaries

**Files:**
- Modify: `crates/davinci-coding-agent/src/native_extensions/mod.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/vector_memory.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/token_governor.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/learning/mod.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/controller.rs`

**Interfaces:**
- Consumes existing retrieval, governor, learning, graph, provider-usage, and security events.
- Produces accurate `EcosystemStats` without changing subsystem behavior.

- [ ] **Step 1: Write an aggregation fixture**

Feed this deterministic sequence:

```text
memory packet: 3 hits / 700 tokens
skills: 2 candidates / 1 injected / 320 tokens
governor: 18 KB omitted / 1 retrieve_output
provider: 4,000 cache-read / 400 cache-write tokens
pruning: 1
security: triggered + passed
learning: 1 dispatched / 1 artifact applied
```

Assert every `EcosystemStats` field exactly.

- [ ] **Step 2: Add read-only counter accessors where a subsystem does not expose the required fact**

The accessors return snapshots; they cannot alter Governor, Memory, Learning, or Security settings.

- [ ] **Step 3: Update graph stats only at stable lifecycle boundaries**

Increment context facts when a packet is persisted, governor facts after tool lifecycle, usage after worker completion, security after verification, and learning after review result.

- [ ] **Step 4: Prevent resume/replay double-counting**

Reused graph nodes contribute persisted historical provenance but do not increment current-run worker/cost counters as if re-executed.

- [ ] **Step 5: Run focused test**

```bash
cargo test -p davinci-coding-agent ecosystem_telemetry -- --nocapture
```

- [ ] **Step 6: Commit**

```bash
git add crates/davinci-coding-agent/src/native_extensions
git commit -m "feat(ecosystem): collect integration telemetry"
```

---

### Task 3: Extend `/status` and `/graph-status` Compactly

**Files:**
- Modify: `crates/davinci-coding-agent/src/main.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/render.rs`
- Modify: `crates/davinci-coding-agent/src/davinci_sources.rs`
- Modify: `crates/davinci-coding-agent/src/davinci_interactive.rs`
- Modify: the existing TUI status-sheet model/view files that currently render graph/governor/memory facts.

**Interfaces:** compact operator output from `EcosystemStats`.

- [ ] **Step 1: Write rendering tests for non-zero participation**

Pin concise output equivalent to:

```text
context  3 memory / 1 skill · 1020 tok
cache    18.4k read / 0.7k write
compact  18 KB governed · 1 recovered · 1 pruning
verify   tests pass · security pass
learn    1 review · 1 applied
```

- [ ] **Step 2: Add a zero-state rendering test**

Do not fill the status sheet with meaningless zero rows. Keep only rows useful to explain why a subsystem did or did not participate.

- [ ] **Step 3: Wire `/graph-status` to persisted run `EcosystemStats`**

- [ ] **Step 4: Wire normal `/status` to live native-host/session stats where available**

- [ ] **Step 5: Preserve existing SheetChrome/TUI design contracts and animation limits**

No new dashboard or animation.

- [ ] **Step 6: Assert status text is not automatically appended to model context**

- [ ] **Step 7: Run status/TUI tests and commit**

```bash
cargo test -p davinci-coding-agent status -- --nocapture
cargo test -p davinci-tui graph_run -- --nocapture
git add crates/davinci-coding-agent crates/davinci-tui
git commit -m "feat(status): expose ecosystem participation"
```

---

### Task 4: Build a Named Closed-Loop Offline Test Suite

**Files:**
- Create only if shared fixtures are needed: `crates/davinci-coding-agent/src/native_extensions/ecosystem/test_support.rs` behind `#[cfg(test)]`.
- Otherwise add inline `#[cfg(test)] mod tests` blocks beside the owning modules, following repository convention.

**Interfaces:** named test filters used by CI.

- [ ] **Step 1: Create deterministic fixture helpers**

Helpers may create temporary git repositories, local memory/learning stores, canned completer events, governor outputs, and scanner results. They must never contact network/provider services.

- [ ] **Step 2: Add `ecosystem_loop_governor_recovery`**

Graph worker produces oversized allowed output -> governor digest -> `retrieve_output` available -> original content recovered byte-for-byte.

- [ ] **Step 3: Add `ecosystem_loop_cache_affinity`**

Compatible retry gets the same cache key. Changed model, toolset, graph version, or system contract gets a different key. Worker still has `session_id == None`.

- [ ] **Step 4: Add `ecosystem_loop_memory_to_graph`**

Settled turn -> vector index -> later graph packet retrieves bounded relevant memory.

- [ ] **Step 5: Add `ecosystem_loop_learning_to_graph`**

Verified learning artifact -> selected exact skill version -> graph metadata -> successful verification -> that version's success ledger increments.

- [ ] **Step 6: Add `ecosystem_loop_security_gate`**

High-risk graph mutation -> required security failure -> approval impossible.

- [ ] **Step 7: Add `ecosystem_loop_full_circle`**

Graph Run #1 -> verification -> learning persistence -> Graph Run #2 receives persisted learning -> Run #2 verification -> outcome ledger update.

- [ ] **Step 8: Add `ecosystem_invariants_token_and_calls`**

Assert packet <=2,500 estimated tokens, <=4 memory hits, <=2 full skills, and no extra coordinator completer calls.

- [ ] **Step 9: Run named suite**

```bash
cargo test -p davinci-coding-agent ecosystem_loop_ -- --nocapture
cargo test -p davinci-coding-agent ecosystem_invariants_ -- --nocapture
```

Expected: PASS.

- [ ] **Step 10: Commit**

```bash
git add crates/davinci-coding-agent/src/native_extensions
git commit -m "test(ecosystem): add closed-loop offline suite"
```

---

### Task 5: Add GitHub Actions CI

**Files:**
- Create: `.github/workflows/ci.yml`

**Interfaces:** repository required-check candidate.

- [ ] **Step 1: Add the workflow**

```yaml
name: CI

on:
  push:
  pull_request:

jobs:
  quality:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v4
      - name: Use repository-pinned Rust
        run: rustup show
      - name: Format
        run: cargo fmt --check
      - name: Clippy
        run: cargo clippy --workspace --all-targets -- -D warnings
      - name: Ecosystem integration
        run: cargo test -p davinci-coding-agent ecosystem_loop_ -- --nocapture
      - name: Ecosystem invariants
        run: cargo test -p davinci-coding-agent ecosystem_invariants_ -- --nocapture
      - name: Workspace tests
        run: cargo test --workspace
```

- [ ] **Step 2: Verify every CI test path is fixture-only**

Search for provider/network dependencies in tests reached by the commands. If a test has an existing fixture environment variable, set it explicitly in the workflow rather than supplying a credential.

- [ ] **Step 3: Keep ecosystem checks as named steps separate from the full workspace suite**

- [ ] **Step 4: Parse the YAML with an available local parser or the repository's existing YAML tooling**

Do not install a new dependency solely for linting this workflow.

- [ ] **Step 5: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: enforce davinci ecosystem integration"
```

---

### Task 6: Add Ecosystem Regression Documentation

**Files:**
- Create: `docs/ecosystem.md`
- Modify: `docs/README.md`
- Modify: `CLAUDE.md`

**Interfaces:** developer/operator documentation.

- [ ] **Step 1: Document a producer/consumer matrix**

```text
Vector Memory -> normal turn -> ephemeral retrieval
Memory/Learning -> Graph -> ContextPacket <= 2,500 tokens
Token Governor -> Graph -> digest + on-demand retrieve_output
Graph verification -> Learning -> VerificationBundle
Graph mutation -> Security -> deterministic RiskAssessment
Learning -> future Graph -> bounded skill/memory selection
```

- [ ] **Step 2: Document model-call invariants**

State explicitly: zero new coordinator calls for normal turns and graph runs.

- [ ] **Step 3: Document fallbacks/kill switches**

Include graph context packet disable, security `off`, learning background-review disable, and `cache_key -> session_id` fallback.

- [ ] **Step 4: Document named ecosystem test commands and interpretation of structural vs real-provider cache evidence**

- [ ] **Step 5: Commit**

```bash
git add docs/ecosystem.md docs/README.md CLAUDE.md
git commit -m "docs: document davinci closed ecosystem"
```

---

### Task 7: Prove Dead Source Islands Before Cleanup

**Files:**
- Create: `docs/archive/dead-code-audit-2026-09-04.md`
- Inspect: workspace `Cargo.toml`, crate `lib.rs`/`mod.rs`, documented gotchas, and repository references.

**Interfaces:** evidence document only; no deletion in this task.

- [ ] **Step 1: Enumerate every currently documented candidate**

Start with known uncompiled source files and the archived/non-workspace `davinci-core` crate.

- [ ] **Step 2: Record this evidence for each candidate**

```text
path
module declared: yes/no
workspace member: yes/no
runtime/migration reader: yes/no
repository reference count and purpose
decision: keep / archive documentation / delete
```

- [ ] **Step 3: Run baseline workspace tests**

```bash
cargo test --workspace
```

Expected: PASS before any cleanup.

- [ ] **Step 4: Commit audit only**

```bash
git add docs/archive/dead-code-audit-2026-09-04.md
git commit -m "docs: audit uncompiled source islands"
```

---

### Task 8: Remove Confirmed Dead Sources in Independently Verified Batches

**Files:**
- Delete only paths whose Task 7 decision is exactly `delete`.
- Modify documentation references only when a removed path was mentioned as historical/dead code.

**Interfaces:** cleanup only.

- [ ] **Step 1: Select exactly one conceptual source island from the audit**

Example: if the audit proves `crates/davinci-session/src/jsonl.rs`, `memory.rs`, `backend.rs`, and `conformance.rs` form one uncompiled legacy island with no runtime reader, that island may be one batch. If the audit does not prove that, keep it.

- [ ] **Step 2: Delete only that audited batch**

- [ ] **Step 3: Run compiler/tests immediately**

```bash
cargo check --workspace
cargo test --workspace
```

Expected: PASS.

- [ ] **Step 4: Commit with the exact subsystem name from the audit**

For the example session island above:

```bash
git add -A
git commit -m "chore(session): remove audited uncompiled legacy sources"
```

For a different island, use that audited subsystem name rather than a generic placeholder.

- [ ] **Step 5: Repeat Steps 1-4 separately for each remaining audited `delete` decision**

- [ ] **Step 6: Move useful historical explanation to `docs/archive/` instead of retaining production-looking dead Rust**

---

### Task 9: Run Final Program Acceptance Suite

**Files:**
- Modify: `docs/ecosystem.md` only to record final verified evidence.

**Interfaces:** release proof.

- [ ] **Step 1: Run ecosystem proof**

```bash
cargo test -p davinci-coding-agent ecosystem_loop_ -- --nocapture
cargo test -p davinci-coding-agent ecosystem_invariants_ -- --nocapture
```

- [ ] **Step 2: Run Gate C learning benchmark**

Require >=40% lower median review input tokens and <=5% relative loss of accepted high-confidence artifacts on the fixture corpus.

- [ ] **Step 3: Run full quality suite**

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build -p davinci-coding-agent
```

- [ ] **Step 4: Exercise status surfaces through fixture/offline modes**

Confirm `/status`, `/graph-status`, memory/governor/security/learning status commands render without stale fixture claims in live paths.

- [ ] **Step 5: Check every roadmap acceptance metric**

No metric may be marked complete from architectural intent alone; cite its test/measurement.

- [ ] **Step 6: Record structural acceptance evidence in `docs/ecosystem.md`**

Do not claim real-provider cache-hit gains from fixture tests.

- [ ] **Step 7: Commit**

```bash
git add docs/ecosystem.md
git commit -m "docs: record ecosystem integration acceptance evidence"
```

---

### Task 10: Optional Post-Merge Real-Provider Measurement Without Behavior Changes

**Files:** none required; use existing structured telemetry.

This observational task does not block correctness release if real-provider credentials are intentionally unavailable in CI.

- [ ] **Step 1: Run two compatible graph tasks on an operator-controlled project**

Capture input/output tokens, cache read/write tokens, context packet size, governor compression/recovery, cost, workers, and wall time.

- [ ] **Step 2: Confirm providers that support cache reporting show cache reads for compatible affinity keys**

- [ ] **Step 3: Treat the result as measurement, not an automatic tuning instruction**

Do not add model downgrades, fan-out heuristics, or cache-driven planning from this task. Any such optimization requires a separate design/spec with multiple-run evidence.

## Gate D Exit Checklist

- [ ] Integration participation is explainable from structured telemetry.
- [ ] Closed-loop ecosystem tests have stable CI names.
- [ ] CI runs fmt, clippy, ecosystem loops/invariants, and workspace tests.
- [ ] CI requires no provider secret/network-backed model fixture.
- [ ] Dead-code cleanup is evidence-based and separately committed.
- [ ] Documentation reflects actual runtime rather than aspirational architecture.
- [ ] Final roadmap metrics pass.
- [ ] The project can truthfully call the ecosystem integration target complete.