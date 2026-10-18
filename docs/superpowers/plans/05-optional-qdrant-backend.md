# Optional Qdrant Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Offer an understandable, truly functional Qdrant database choice for users who want it while keeping local memory, Ollama and ordinary coding independent of Docker.

**Architecture:** Implement Qdrant as a namespace-filtered, repairable derived vector projection of the local authoritative JSONL records, with bounded daemon management only for consented owned containers.

**Tech Stack:** Rust HTTP/client, Qdrant `/readyz` and collections/points API, Docker Desktop optional, Windows user-level consent.

**Spec:** [`../specs/2026-10-17-davinci-reliability-and-zero-maintenance-design.md`](../specs/2026-10-17-davinci-reliability-and-zero-maintenance-design.md)

**Dependencies / execution order:** After Plan 02 local memory profile, service supervisor and consent and Plan 01 trust. Ship behind experimental feature flag until actual evaluation gate passes.

## Global Constraints

**Path shorthand:** Any unqualified `src/...` or `tests/...` in this plan is relative to `crates/davinci-coding-agent/`. Explicit `crates/...` paths are repo-root-relative. For the Design Studio plan, `design-ui/...` is relative to `crates/davinci-coding-agent/`.

- This is a plan, not an implementation. Start from a clean isolated worktree of the exact target `main` commit. Confirm historical audit findings still reproduce, especially after recent compaction changes.
- Rust workspace/pinned toolchains and existing feature flags, permission policy, sandbox, provider identity, durable stores and cancellation must stay intact. No live package installs, cloud requests or data mutations without explicit operator approval.
- Local JSONL remains authoritative. Preserve disabled-mode parity, existing slash commands, recovery safety and provenance; no mandatory Docker/Qdrant/SQLite.
- New tasks must use red-green TDD, targeted verification, one purpose per commit, reviewer sign-off and rollback/feature gate before default enablement.

## Review Focus

- `local` configuration never probes or starts Qdrant — Task 1.
- Remote records of a different repository must not leak into results — Task 2.
- Qdrant down must not destroy local data or block ordinary coding — Task 3.
- DaVinci must never hijack an existing foreign Docker container — Task 4/5.
- No promotion based solely on service availability; require labeled retrieval quality/performance evidence — Task 6.

---

### Task 1: Define remote projection and configuration without affecting local mode

**Files:** Create: `crates/davinci-coding-agent/src/native_extensions/memory_backends/{mod,qdrant}.rs`; Modify: `src/native_extensions/vector_memory.rs`; Test: `tests/qdrant_projection.rs`

**Interfaces:** Produces `ProjectionConfig`, `VectorProjection` consumed by backend selector.

- [ ] **Step 1 — Write the failing regression** `local_mode_never_contacts_qdrant`. Assert: With default local backend Qdrant has zero network/daemon traffic; selecting Qdrant requires explicit user action and stores remote URL/collection plus host-managed consent..
- [ ] **Step 2 — Prove red on baseline.** Run `cargo test -p davinci-coding-agent local_mode_never_contacts_qdrant -- --nocapture`. Expected: test fails for the identified behavior (or mark as already fixed and report evidence; never make an unnecessary fix).
- [ ] **Step 3 — Implement one targeted change.** Add typed `MemoryProjectionBackend::{Local,Qdrant}` plus trait `VectorProjection` with query/upsert/delete/health/rebuild; local authoritative records unchanged.
- [ ] **Step 4 — Verify green and regression boundaries.** Re-run `cargo test -p davinci-coding-agent local_mode_never_contacts_qdrant -- --nocapture`; expected: pass. Also run the module-specific suite, `cargo fmt --all -- --check`, and relevant Clippy targets. Save test transcript/receipt and verify no new permissions or default behavior changed.
- [ ] **Step 5 — Review and commit.** Reviewer inspects exact code and config/data compatibility; commit as `feat(memory): introduce optional vector projection API`. Do not proceed when the gate is red.

### Task 2: Implement collection validation and namespaced remote queries

**Files:** Modify: `src/native_extensions/memory_backends/qdrant.rs`; Test: `tests/qdrant_projection.rs`

**Interfaces:** Produces `QdrantProjection::query(filter, vector, k)` and `verify_collection() -> ProjectionHealth`.

- [ ] **Step 1 — Write the failing regression** `qdrant_query_filters_repo_scope_and_tombstones`. Assert: Only matching repo/profile/scope/version points return; tombstoned/retired memory never appears; wrong collection vector size or model identity is `ActionRequired`, not coerced..
- [ ] **Step 2 — Prove red on baseline.** Run `cargo test -p davinci-coding-agent qdrant_query_filters_repo_scope_and_tombstones -- --nocapture`. Expected: test fails for the identified behavior (or mark as already fixed and report evidence; never make an unnecessary fix).
- [ ] **Step 3 — Implement one targeted change.** Probe `/readyz`, validate collection schema & dimension 768 for selected model, use secure bounded HTTP client with optional API-key secret storage; implement query path and remote filtering.
- [ ] **Step 4 — Verify green and regression boundaries.** Re-run `cargo test -p davinci-coding-agent qdrant_query_filters_repo_scope_and_tombstones -- --nocapture`; expected: pass. Also run the module-specific suite, `cargo fmt --all -- --check`, and relevant Clippy targets. Save test transcript/receipt and verify no new permissions or default behavior changed.
- [ ] **Step 5 — Review and commit.** Reviewer inspects exact code and config/data compatibility; commit as `feat(memory): add actually used Qdrant query path`. Do not proceed when the gate is red.

### Task 3: Write/index reconciliation and outage fallback

**Files:** Modify: `src/native_extensions/memory_backends/qdrant.rs`, `src/native_extensions/vector_memory.rs`; Test: `tests/qdrant_projection.rs`

**Interfaces:** Produces `ProjectionRepairReceipt { indexed, lag, skipped, failures }`.

- [ ] **Step 1 — Write the failing regression** `qdrant_rebuild_and_outage_keep_local_memory`. Assert: Writes go to authoritative JSONL first; Qdrant lag is tracked and reindexed idempotently; loss/outage falls back to local search with visible degraded state; nothing deleted remotely beyond owned namespace..
- [ ] **Step 2 — Prove red on baseline.** Run `cargo test -p davinci-coding-agent qdrant_rebuild_and_outage_keep_local_memory -- --nocapture`. Expected: test fails for the identified behavior (or mark as already fixed and report evidence; never make an unnecessary fix).
- [ ] **Step 3 — Implement one targeted change.** Use outbox/rebuild cursor stored with projection metadata; stable remote IDs, bounded batches, hash/embedding-identity validation and repair logic.
- [ ] **Step 4 — Verify green and regression boundaries.** Re-run `cargo test -p davinci-coding-agent qdrant_rebuild_and_outage_keep_local_memory -- --nocapture`; expected: pass. Also run the module-specific suite, `cargo fmt --all -- --check`, and relevant Clippy targets. Save test transcript/receipt and verify no new permissions or default behavior changed.
- [ ] **Step 5 — Review and commit.** Reviewer inspects exact code and config/data compatibility; commit as `feat(memory): reconcile optional Qdrant from local records`. Do not proceed when the gate is red.

### Task 4: Discover Docker services without assuming container ownership

**Files:** Create: `src/native_extensions/docker_discovery.rs`; Modify: `src/native_extensions/setup.rs`; Test: `tests/qdrant_projection.rs`

**Interfaces:** Produces `QdrantDiscovery { service, container, owner, docker_engine, health }`.

- [ ] **Step 1 — Write the failing regression** `existing_qdrant_detected_no_unapproved_docker_mutation`. Assert: DaVinci detects running Qdrant and its exact endpoint; a third-party container is never stopped, reconfigured, or assigned a restart policy without explicit approval; Docker Desktop missing is actionable not fatal..
- [ ] **Step 2 — Prove red on baseline.** Run `cargo test -p davinci-coding-agent existing_qdrant_detected_no_unapproved_docker_mutation -- --nocapture`. Expected: test fails for the identified behavior (or mark as already fixed and report evidence; never make an unnecessary fix).
- [ ] **Step 3 — Implement one targeted change.** Use read-only endpoint/API and `docker` state interrogation; management commands only for explicitly owned container identity; prevent untrusted path/argument injection and honor remote-service no-start behavior.
- [ ] **Step 4 — Verify green and regression boundaries.** Re-run `cargo test -p davinci-coding-agent existing_qdrant_detected_no_unapproved_docker_mutation -- --nocapture`; expected: pass. Also run the module-specific suite, `cargo fmt --all -- --check`, and relevant Clippy targets. Save test transcript/receipt and verify no new permissions or default behavior changed.
- [ ] **Step 5 — Review and commit.** Reviewer inspects exact code and config/data compatibility; commit as `feat(setup): discover existing Qdrant and Docker`. Do not proceed when the gate is red.

### Task 5: One-time authorized optional install, startup and restart policy

**Files:** Modify: `src/native_extensions/setup_actions.rs`, `src/native_extensions/service_supervisor.rs`, `src/native_extensions/setup.rs`; Test: `tests/qdrant_projection.rs`

**Interfaces:** Consumes `ServiceAuthorization`/`QdrantDiscovery`; produces `reconcile_owned_qdrant(...)` and restore receipt.

- [ ] **Step 1 — Write the failing regression** `approved_owned_qdrant_restarts_after_relaunch`. Assert: User chooses Install or Use existing, expressly authorizes creation of owned container/volume; later launch restarts exactly that container if needed; user disabled or manually stopped under disabled intent stays stopped..
- [ ] **Step 2 — Prove red on baseline.** Run `cargo test -p davinci-coding-agent approved_owned_qdrant_restarts_after_relaunch -- --nocapture`. Expected: test fails for the identified behavior (or mark as already fixed and report evidence; never make an unnecessary fix).
- [ ] **Step 3 — Implement one targeted change.** Never auto-install Docker; when Docker Desktop is missing offer official guided install with user approval. Persist owned container identity, volume/config fingerprints; select `unless-stopped` only with authorization and working engine.
- [ ] **Step 4 — Verify green and regression boundaries.** Re-run `cargo test -p davinci-coding-agent approved_owned_qdrant_restarts_after_relaunch -- --nocapture`; expected: pass. Also run the module-specific suite, `cargo fmt --all -- --check`, and relevant Clippy targets. Save test transcript/receipt and verify no new permissions or default behavior changed.
- [ ] **Step 5 — Review and commit.** Reviewer inspects exact code and config/data compatibility; commit as `feat(setup): support optional managed Qdrant`. Do not proceed when the gate is red.

### Task 6: Benchmark remote backend before enabling by default anywhere

**Files:** Create: `crates/davinci-evals/src/behavior/qdrant_projection.rs`; Modify: `docs/superpowers/plans/2026-09-15-memory-projection-decision.md` (new addendum only); Test: `tests/qdrant_projection.rs`

**Interfaces:** Produces `QdrantEvaluationReceipt` and go/no-go recommendation.

- [ ] **Step 1 — Write the failing regression** `qdrant_optional_performance_and_outage_gate`. Assert: Quality parity on labeled corpus, projection lag bound, namespace isolation, tombstones, model-change rebuild and outage recovery pass; compare p95 retrieval vs local. No mandatory Qdrant without user choice..
- [ ] **Step 2 — Prove red on baseline.** Run `cargo test -p davinci-coding-agent qdrant_optional_performance_and_outage_gate -- --nocapture`. Expected: test fails for the identified behavior (or mark as already fixed and report evidence; never make an unnecessary fix).
- [ ] **Step 3 — Implement one targeted change.** Build reproducible local vs Qdrant test corpus; retain source/tool receipts. Treat existing 30% p95 improvement requirement as promotion gate, not guaranteed outcome.
- [ ] **Step 4 — Verify green and regression boundaries.** Re-run `cargo test -p davinci-coding-agent qdrant_optional_performance_and_outage_gate -- --nocapture`; expected: pass. Also run the module-specific suite, `cargo fmt --all -- --check`, and relevant Clippy targets. Save test transcript/receipt and verify no new permissions or default behavior changed.
- [ ] **Step 5 — Review and commit.** Reviewer inspects exact code and config/data compatibility; commit as `test(memory): gate optional remote vector backend`. Do not proceed when the gate is red.

## Release checklist

- [ ] All task-specific failing tests were demonstrated against the baseline or explicitly documented as already fixed.
- [ ] Every task passes focused tests, formatting and relevant Clippy; existing test suite passes with no suppressed failures.
- [ ] Scenario-level tests and negative cases run on disposable workspaces with fake services where possible; live-system tests run only with authorized dependencies.
- [ ] Documentation, rollback procedure, opt-out and error reporting are updated; no credential, memory text or user path appears in broad logs.
- [ ] Merge behind a bounded feature gate, collect real-world diagnostics, then separately authorize default-on behavior.
