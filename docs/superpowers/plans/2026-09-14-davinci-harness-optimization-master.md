# DaVinci Harness Optimization Master Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Improve DaVinci's existing harness so verified coding success increases while context, provider cost, retries, and maintenance overhead decrease.

**Architecture:** Keep one integration branch but execute five independently testable subsystem plans in strict dependency order. Reuse `RuntimeCapabilityRegistry`, `ToolExposureState`, `ResourceLedger`, `OutputStore`, `ToolCallLedger`, vector memory, learning, Graph, MCP, semantic service, security scan, and `davinci-evals`; do not introduce parallel replacements.

**Tech Stack:** Rust 1.83 workspace, serde/serde_json, SHA-256 utilities already in the repo, existing GitHub Actions workflows, existing offline test conventions.

**Spec:** `docs/superpowers/specs/2026-09-14-davinci-harness-optimization-design.md`

## Global Constraints

- Work only on `feat/davinci-harness-optimization-20260914`.
- Baseline is `main@a4389b2d60dfb5f3ddeb37e75972ff65bdb013e9`.
- Do not add another Graph system, capability registry, memory database, generic planner, model-based Governor summarizer, or always-on LSP daemon.
- Preserve normal prompt semantics, permission boundaries, exact-output recovery, legacy `.pi` compatibility, JSON/print/RPC compatibility, and current provider wire contracts.
- Selection, compression, verification bookkeeping, recovery, routing, cache identity, scan reuse, and evaluation scoring stay deterministic.
- No live provider/network tests are required for code correctness.
- Every behavior-changing optimization needs a focused regression or deterministic ablation.
- Unknown capabilities stay conservative: mutation-capable, serial, and never auto-replayable.

---

## File Structure and Subplan Boundaries

### Plan 1 — Governor, root context, verification, recovery

`docs/superpowers/plans/2026-09-14-davinci-governor-context-verification.md`

Owns:
- reversible compression for large errors;
- new deterministic Governor content views;
- minimum specialized-view win and per-kind retrieval telemetry;
- enforced root context budgeting and scoped repository instructions;
- scope-aware verification evidence;
- atomic `ToolCallLedger` durability and registry-authoritative side effects.

### Plan 2 — Capability Toolbox, memory, learning

`docs/superpowers/plans/2026-09-14-davinci-capability-memory-learning.md`

Owns:
- node-specific Graph retrieval queries;
- calibrated context utility;
- cached semantic skill ranking;
- skill applicability metadata;
- stronger skill outcome credit;
- memory freshness/compatibility scoring.

### Plan 3 — Graph, MCP, cache effectiveness

`docs/superpowers/plans/2026-09-14-davinci-graph-mcp-cache.md`

Owns:
- failure-type-aware Graph retries;
- bounded failure-specific retry context;
- Graph deferred schemas through existing `ToolExposureState`;
- MCP progressive schema disclosure;
- real provider cache-effect telemetry and cache-miss reason reporting.

### Plan 4 — Lazy semantic navigation and incremental security scanning

`docs/superpowers/plans/2026-09-14-davinci-semantic-security.md`

Owns:
- lazy workspace-scoped semantic sessions with fallback;
- deterministic incremental security file reuse without weakening final sealing.

### Plan 5 — Evals and repository policy

`docs/superpowers/plans/2026-09-14-davinci-evals-repository-policy.md`

Owns:
- internal deterministic A/B coverage for all new behavior;
- Codex/Hermes/OpenCode competitor adapters alongside Claude Code;
- harness-vs-product report separation;
- provider-backed metrics only when configured;
- repository protection verification/reporting.

---

## Execution Order

- [ ] **Task 1: Execute Plan 1 — Governor, root context, verification, recovery**

Reason: later memory/Graph work depends on reliable request shaping, verification evidence, and recovery semantics.

Acceptance gate before Task 2:

```bash
cargo test -p davinci-agent context::
cargo test -p davinci-agent verification
cargo test -p davinci-agent tool_ledger
cargo test -p davinci-coding-agent token_governor
cargo test -p davinci-coding-agent content_router
cargo clippy -p davinci-agent --all-targets -- -D warnings
cargo clippy -p davinci-coding-agent --all-targets -- -D warnings
```

- [ ] **Task 2: Execute Plan 2 — Capability Toolbox, memory, learning**

Acceptance gate before Task 3:

```bash
cargo test -p davinci-coding-agent ecosystem::
cargo test -p davinci-coding-agent learning::
cargo test -p davinci-coding-agent vector_memory
cargo clippy -p davinci-coding-agent --all-targets -- -D warnings
```

- [ ] **Task 3: Execute Plan 3 — Graph, MCP, cache**

Acceptance gate before Task 4:

```bash
cargo test -p davinci-agent tool_search
cargo test -p davinci-agent mcp
cargo test -p davinci-coding-agent graph::
cargo test -p davinci-coding-agent cache_affinity
cargo clippy -p davinci-agent --all-targets -- -D warnings
cargo clippy -p davinci-coding-agent --all-targets -- -D warnings
```

- [ ] **Task 4: Execute Plan 4 — Semantic navigation and security scanning**

Acceptance gate before Task 5:

```bash
cargo test -p davinci-coding-agent semantic
cargo test -p davinci-coding-agent security_scan
cargo clippy -p davinci-coding-agent --all-targets -- -D warnings
```

- [ ] **Task 5: Execute Plan 5 — Evals and repository policy**

Acceptance gate before final integration:

```bash
cargo test -p davinci-evals
cargo run -p davinci-evals -- corpus verify
```

- [ ] **Task 6: Run final integration verification once**

Run on the exact final branch head:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p davinci-evals -- corpus verify
cargo test --workspace
git diff --check
```

Then rely on the repository's standard GitHub Actions for workflow lint, Security SARIF interoperability, CI quality, and native voice matrix. Do not invent overlapping full-workspace commands after this gate.

- [ ] **Task 7: Run deterministic offline ablations before any provider-backed benchmark**

Required offline comparisons:

```text
Governor: generic vs content-aware-vNext
Root context: report-only baseline fixture vs enforced-budget fixture
Capability Toolbox: graph-goal query vs node-specific query
Graph/MCP schemas: full-visible vs deferred-visible catalog
Memory: freshness disabled vs freshness enabled fixture
Security scan: cold scan vs incremental unchanged-file reuse
```

Every ablation must keep a deterministic correctness oracle. If an optimization saves bytes/tokens but lowers the fixture's verified correctness oracle, reject it.

- [ ] **Task 8: Provider-backed benchmark only when credentials are configured**

Use `davinci-evals` to persist real provider usage. Never fabricate missing measurements. Primary release metric:

```text
cost per verified success = total attributable provider cost / independently verified successful tasks
```

Record verified success rate, first-attempt success, input/output/cache tokens, model turns, retries, workers, wall time, scope violations, permission failures, and false success claims.

---

## Commit Discipline

Each subplan task ends with a focused commit. Do not mix independent subsystems into one commit. Suggested prefixes:

```text
feat(governor): ...
feat(context): ...
feat(verification): ...
fix(recovery): ...
feat(capability-toolbox): ...
feat(memory): ...
feat(learning): ...
feat(graph): ...
feat(mcp): ...
feat(semantic): ...
feat(security): ...
feat(evals): ...
```

## Stop Conditions

Stop the current task and report rather than expanding scope if:

- the invariant is already satisfied by current branch code;
- the change would require a new dependency when existing/std facilities can plausibly suffice;
- the implementation would weaken permissions, recovery safety, exact evidence recovery, or verification integrity;
- a provider/network dependency is required merely to make an offline feature function;
- GitHub repository-administration actions are unavailable through the connected GitHub tool.
