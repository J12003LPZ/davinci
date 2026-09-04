# Ecosystem Gate D — Proof, CI, Observability, and Hygiene Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the integrated ecosystem measurable, continuously enforced, and easier for coding agents to reason about by adding compact structured telemetry, offline closed-loop tests, repository CI, and evidence-based dead-code cleanup.

**Architecture:** Reuse existing status/run-stat surfaces instead of creating a new monitoring subsystem. Integration proof lives in offline fixture tests and GitHub Actions. Cleanup happens only after functional integration is proven and is kept in separate commits.

**Tech Stack:** Rust 1.83.0, existing Davinci TUI/status structures, GitHub Actions, existing fixture infrastructure.

**Spec:** `docs/superpowers/specs/2026-09-04-davinci-ecosystem-integration-design.md`

## Global Constraints

- Telemetry adds zero model calls.
- Telemetry is not injected into prompts automatically.
- CI uses fixture-only tests and no provider/network secrets.
- Do not add a new analytics service or external database.
- Do not delete code merely because it looks unused; prove it is uncompiled/unreferenced.
- Dead-code removal is separated from functional integration commits.
- Keep status output compact and operator-oriented.

---

### Task 1: Define Structured Ecosystem Telemetry

**Files:**
- Modify: `crates/davinci-coding-agent/src/native_extensions/ecosystem/telemetry.rs`
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

- [ ] **Step 1: Add serialization/default tests**

Ensure new fields deserialize with defaults so older persisted run/session data remains readable.

- [ ] **Step 2: Run and verify missing API**

```bash
cargo test -p davinci-coding-agent ecosystem_stats -- --nocapture
```

- [ ] **Step 3: Keep telemetry aggregation pure**

Use existing subsystem counters/snapshots. No filesystem scans or model calls merely to render status.

- [ ] **Step 4: Persist graph-run ecosystem stats**

Store with run state/checkpoint so `/graph-status` can explain a completed run without reconstructing it from logs.

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-coding-agent/src/native_extensions/ecosystem crates/davinci-coding-agent/src/native_extensions/graph/types.rs
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
- Consumes: existing retrieval/governor/learning/graph/security events.
- Produces: accurate counters without cross-subsystem ownership transfer.

- [ ] **Step 1: Add a fixture event sequence and expected aggregate stats**

Example sequence:

```text
memory packet 3 hits / 700 tokens
2 skills considered, 1 injected / 320 tokens
governor omits 18 KB
retrieve_output called once
provider reports 4k cache read / 400 write
one pruning
security gate passes
learning review dispatched and applies one artifact
```

Assert exact `EcosystemStats` output.

- [ ] **Step 2: Add read-only counter accessors to subsystem owners where missing**

Do not let telemetry mutate Governor/Memory/Learning behavior.

- [ ] **Step 3: Update stats at stable lifecycle boundaries**

Prefer event-derived increments over repeatedly recomputing totals.

- [ ] **Step 4: Prevent double-counting on resume/replay**

Persist counters with attempt/run IDs or mark replayed nodes as reused so status represents actual current-run work.

- [ ] **Step 5: Test and commit**

```bash
cargo test -p davinci-coding-agent ecosystem_telemetry -- --nocapture
git add crates/davinci-coding-agent/src/native_extensions
git commit -m "feat(ecosystem): collect integration telemetry"
```

---

### Task 3: Extend `/status` and `/graph-status` Compactly

**Files:**
- Modify: `crates/davinci-coding-agent/src/main.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/render.rs`
- Modify: `crates/davinci-coding-agent/src/davinci_sources.rs` and/or `davinci_interactive.rs` where current live sheet facts are assembled
- Modify: TUI model/view files only where needed for existing status sheets

**Interfaces:** compact operator output.

- [ ] **Step 1: Add rendering tests before wiring**

Pin concise rows such as:

```text
context  3 memory / 1 skill · 1020 tok
cache    18.4k read / 0.7k write
compact  18 KB governed · 1 recovered · 1 pruning
verify   tests pass · security pass
learn    1 review · 1 applied
```

Do not render zero/no-op rows unless useful to explain state.

- [ ] **Step 2: Wire `/graph-status` to persisted `EcosystemStats`**

- [ ] **Step 3: Wire normal `/status` to session/native host stats where available**

- [ ] **Step 4: Preserve existing sheet frame/design contracts**

No new dashboard or animation.

- [ ] **Step 5: Ensure status rendering never enters model context automatically**

- [ ] **Step 6: Run TUI/status tests and commit**

```bash
cargo test -p davinci-coding-agent status -- --nocapture
cargo test -p davinci-tui graph_run -- --nocapture
git add crates/davinci-coding-agent crates/davinci-tui
git commit -m "feat(status): expose ecosystem participation"
```

---

### Task 4: Build a Named Closed-Loop Offline Test Suite

**Files:**
- Prefer inline `#[cfg(test)]` modules following repository convention.
- If a single shared fixture helper is necessary, create: `crates/davinci-coding-agent/src/native_extensions/ecosystem/test_support.rs` behind `#[cfg(test)]`.

**Interfaces:** named test filters used by CI.

- [ ] **Step 1: Create shared deterministic fixture helpers**

Helpers may create temp git repos, local memory/learning stores, canned completer events, governor outputs, and scanner results. They must never contact network/provider services.

- [ ] **Step 2: Add `ecosystem_loop_governor_recovery`**

Prove allowed graph output can always be recovered losslessly.

- [ ] **Step 3: Add `ecosystem_loop_cache_affinity`**

Prove compatible retries share a key while changed contracts/toolsets/models invalidate it.

- [ ] **Step 4: Add `ecosystem_loop_memory_to_graph`**

Settled turn -> vector memory -> bounded graph context hit.

- [ ] **Step 5: Add `ecosystem_loop_learning_to_graph`**

Verified learning artifact -> selected skill/version -> graph metadata -> verified outcome ledger update.

- [ ] **Step 6: Add `ecosystem_loop_security_gate`**

High-risk graph mutation -> security failure -> approval impossible.

- [ ] **Step 7: Add `ecosystem_loop_full_circle`**

Graph Run #1 -> verify -> learn -> persist -> Graph Run #2 receives learning -> verifies -> outcome updates.

- [ ] **Step 8: Add `ecosystem_invariants_token_and_calls`**

Assert default packet <=2,500 estimated tokens, <=4 memory hits, <=2 full skills, and no extra coordinator completer calls.

- [ ] **Step 9: Run the named suite**

```bash
cargo test -p davinci-coding-agent ecosystem_loop_ -- --nocapture
cargo test -p davinci-coding-agent ecosystem_invariants_ -- --nocapture
```

- [ ] **Step 10: Commit**

```bash
git add crates/davinci-coding-agent/src/native_extensions
git commit -m "test(ecosystem): add closed-loop offline suite"
```

---

### Task 5: Add GitHub Actions CI

**Files:**
- Create: `.github/workflows/ci.yml`

**Interfaces:** repository required checks candidate.

- [ ] **Step 1: Write workflow with pinned repository toolchain**

Use checkout + Rust toolchain derived from `rust-toolchain.toml`; do not silently upgrade Rust.

Minimum jobs/steps:

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
      - name: Install pinned Rust toolchain
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

If the repo already needs platform-specific setup for Linux CI, add only documented deterministic dependencies.

- [ ] **Step 2: Confirm no secrets/environment provider credentials are required**

Search tests for accidental live-provider requirements before relying on CI.

- [ ] **Step 3: Keep ecosystem step separate from workspace tests**

This makes closed-loop failures visible rather than buried in a large test job.

- [ ] **Step 4: Validate workflow syntax locally where tooling exists**

At minimum inspect YAML parse through an available local parser/test; do not require external action-lint installation solely for this task.

- [ ] **Step 5: Commit**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: enforce davinci ecosystem integration"
```

---

### Task 6: Add an Ecosystem Regression Matrix to Documentation

**Files:**
- Modify: `docs/README.md`
- Create: `docs/ecosystem.md`
- Modify: `CLAUDE.md`

**Interfaces:** operator/developer documentation.

- [ ] **Step 1: Document subsystem producer/consumer matrix**

Example:

| Producer | Consumer | Contract | Prompt cost |
| --- | --- | --- | --- |
| Vector Memory | normal turn | ephemeral memory block | bounded existing config |
| Memory/Learning | graph worker | `ContextPacket` | <=2,500 total packet |
| Token Governor | graph worker | digest + `retrieve_output` | recovery on demand |
| Graph verification | Learning | `VerificationBundle` | no extra model call |
| Graph mutation | Security | `RiskAssessment` | deterministic |
| Learning | future Graph | skill/memory refs | bounded retrieval |

- [ ] **Step 2: Document model-call budget invariants**

Explicitly state zero new coordinator calls.

- [ ] **Step 3: Document kill switches/fallbacks**

Context packet off, security off, learning review disable, cache-key fallback.

- [ ] **Step 4: Document how to run named ecosystem tests**

- [ ] **Step 5: Commit**

```bash
git add docs/README.md docs/ecosystem.md CLAUDE.md
git commit -m "docs: document davinci closed ecosystem"
```

---

### Task 7: Prove Dead Source Islands Before Cleanup

**Files:**
- Create: `docs/archive/dead-code-audit-2026-09-04.md`
- Inspect: `Cargo.toml`, crate `lib.rs`/`mod.rs`, workspace member lists, search references.

**Interfaces:** evidence document only; no deletion in this task.

- [ ] **Step 1: Enumerate currently documented candidates**

Start with the repository's own known gotchas, including uncompiled source files and the archived `davinci-core` crate.

- [ ] **Step 2: For every candidate, prove module/workspace reachability**

Record:

```text
path
module declared? yes/no
workspace member? yes/no
dependent crate/reference search results
migration/runtime read path? yes/no
decision: keep / archive docs / delete
```

- [ ] **Step 3: Run baseline workspace test before deletion decisions**

```bash
cargo test --workspace
```

- [ ] **Step 4: Commit audit only**

```bash
git add docs/archive/dead-code-audit-2026-09-04.md
git commit -m "docs: audit uncompiled source islands"
```

---

### Task 8: Remove Confirmed Dead Production-Looking Sources in Small Batches

**Files:**
- Delete only paths marked `delete` by Task 7.
- Modify documentation references where necessary.

**Interfaces:** none; cleanup only.

- [ ] **Step 1: Delete the first independently proven batch**

Do not delete more than one conceptual source island per commit.

- [ ] **Step 2: Run compiler/tests immediately**

```bash
cargo check --workspace
cargo test --workspace
```

- [ ] **Step 3: Commit that batch**

```bash
git add -A
git commit -m "chore: remove uncompiled <area> sources"
```

- [ ] **Step 4: Repeat Steps 1-3 for each separately proven island**

Examples may include unreferenced legacy session source files or the non-workspace archived crate, but only if Task 7 proves removal will not discard required historical/vendor/migration content.

- [ ] **Step 5: If historical context is useful, move prose/design explanation to `docs/archive/` rather than retaining dead compilable-looking Rust**

---

### Task 9: Run Final Program Acceptance Suite

**Files:** none unless a regression requires a targeted fix; fixes belong in a new reviewable commit.

- [ ] **Step 1: Run ecosystem proof**

```bash
cargo test -p davinci-coding-agent ecosystem_loop_ -- --nocapture
cargo test -p davinci-coding-agent ecosystem_invariants_ -- --nocapture
```

- [ ] **Step 2: Run learning benchmark**

Require the Gate C efficiency/quality thresholds.

- [ ] **Step 3: Run full quality suite**

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build -p davinci-coding-agent
```

- [ ] **Step 4: Exercise CLI status surfaces with fixture/offline modes**

Confirm `/status`, `/graph-status`, memory/governor/security/learning status commands still render and contain no fixture claims in live paths.

- [ ] **Step 5: Compare program metrics against the roadmap table**

Every requirement must pass before calling integration 10/10.

- [ ] **Step 6: Record final evidence in `docs/ecosystem.md`**

Do not claim real provider cache-hit improvement from fixture tests; distinguish structural proof from production measurements.

- [ ] **Step 7: Commit any final documentation evidence**

```bash
git add docs/ecosystem.md
git commit -m "docs: record ecosystem integration acceptance evidence"
```

---

### Task 10: Optional Post-Merge Production Measurement — No Behavior Change

**Files:** none required; use existing structured telemetry/logging.

This task is observational and should not block correctness release if real-provider credentials are intentionally unavailable in CI.

- [ ] **Step 1: On an operator-controlled real project, capture pre/post compatible graph runs**

Measure:

- input/output tokens;
- cache read/write tokens;
- context packet size;
- governor compression/recovery;
- total cost;
- worker count;
- wall time.

- [ ] **Step 2: Verify stable cache affinity produces provider-reported cache reads where the provider supports them**

Do not tune heuristics from one run.

- [ ] **Step 3: If data suggests a future optimization, create a separate design/spec**

Do not add automatic model downgrades, fan-out heuristics, or cache-driven planning inside this program.

## Gate D Exit Checklist

- [ ] Integration participation is explainable from structured telemetry.
- [ ] Closed-loop ecosystem tests have stable CI names.
- [ ] CI runs fmt, clippy, ecosystem tests, invariants, and workspace tests.
- [ ] No network/provider secret is required for CI.
- [ ] Dead-code cleanup is evidence-based and separately committed.
- [ ] Documentation reflects actual runtime, not aspirational architecture.
- [ ] Final metric table passes.
- [ ] The project can truthfully call the ecosystem integration target complete.
