# Automatic Local Memory, Ollama, Setup Wizard, and Governor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Provide one-time guided setup and automatic safe restart recovery for Ollama/EmbeddingGemma, authoritative local memory and the Token Governor.

**Architecture:** Reuse native `/setup` and existing JSONL/vector/governor logic. A typed host-owned profile, consented supervisor and truthful probes coordinate the existing implementations without making Qdrant, Docker or SQLite a dependency.

**Tech Stack:** Rust, TUI, JSONL, Ollama HTTP API (`/api/tags`,`/api/embed`,`/api/pull`), atomic user config, Windows subprocess lifecycle.

**Spec:** [`../specs/2026-10-17-davinci-reliability-and-zero-maintenance-design.md`](../specs/2026-10-17-davinci-reliability-and-zero-maintenance-design.md)

**Dependencies / execution order:** After Trust Plan 01 for service consent; can ship before context/design/Qdrant plans.

## Global Constraints

**Path shorthand:** Any unqualified `src/...` or `tests/...` in this plan is relative to `crates/davinci-coding-agent/`. Explicit `crates/...` paths are repo-root-relative. For the Design Studio plan, `design-ui/...` is relative to `crates/davinci-coding-agent/`.

- This is a plan, not an implementation. Start from a clean isolated worktree of the exact target `main` commit. Confirm historical audit findings still reproduce, especially after recent compaction changes.
- Rust workspace/pinned toolchains and existing feature flags, permission policy, sandbox, provider identity, durable stores and cancellation must stay intact. No live package installs, cloud requests or data mutations without explicit operator approval.
- Local JSONL remains authoritative. Preserve disabled-mode parity, existing slash commands, recovery safety and provenance; no mandatory Docker/Qdrant/SQLite.
- New tasks must use red-green TDD, targeted verification, one purpose per commit, reviewer sign-off and rollback/feature gate before default enablement.

## Review Focus

- Existing Ollama model but no daemon: automatic start without a download or duplicate process — Tasks 2/4/6.
- Corrupt existing memory file: never silently treat as empty and overwrite — Task 3.
- Model download interrupted at unknown outcome: do not infinite-loop or report green — Tasks 5/7.
- User disables service: reboot or `/setup` must not reenable it — Task 10.
- Governor store offline or unwritable: explain exact retrieval failure, preserve saved artifacts — Task 9.

---

### Task 1: Define a single setup profile and effective-config resolver

**Files:** Create: `src/native_extensions/setup_profile.rs`; Modify: `src/native_extensions/mod.rs`, `src/native_extensions/setup.rs`; Test: `tests/setup_memory_supervisor.rs`

**Interfaces:** Produces: `SetupProfile`, `EffectiveSetupConfig`, `load_effective_setup(agent_dir, env) -> Result<...>` used by all later setup tasks.

- [ ] **Step 1 — Write the failing regression** `setup_profile_defaults_precedence_and_roundtrip`. Assert: Default Ollama+embeddinggemma/768, local backend, autostart and safe repair true; Qdrant false; existing user configuration retained; env overrides shown explicitly; invalid profile is not silently reset..
- [ ] **Step 2 — Prove red on baseline.** Run `cargo test -p davinci-coding-agent setup_profile_defaults_precedence_and_roundtrip -- --nocapture`. Expected: test fails for the identified behavior (or mark as already fixed and report evidence; never make an unnecessary fix).
- [ ] **Step 3 — Implement one targeted change.** Use schema-versioned, atomic host-owned profile and typed `effective_settings` layered over current `vector-memory.json`/governor files, not parallel contradictory copies.
- [ ] **Step 4 — Verify green and regression boundaries.** Re-run `cargo test -p davinci-coding-agent setup_profile_defaults_precedence_and_roundtrip -- --nocapture`; expected: pass. Also run the module-specific suite, `cargo fmt --all -- --check`, and relevant Clippy targets. Save test transcript/receipt and verify no new permissions or default behavior changed.
- [ ] **Step 5 — Review and commit.** Reviewer inspects exact code and config/data compatibility; commit as `feat(setup): persist typed setup choices`. Do not proceed when the gate is red.

### Task 2: Create health states and dependency discovery

**Files:** Create: `src/native_extensions/setup_health.rs`; Modify: `src/native_extensions/memory_page.rs`; Test: `tests/setup_memory_supervisor.rs`

**Interfaces:** Produces `ComponentHealth { state, code, detail, remediation, last_checked }` and `probe_ollama(...)`, `probe_memory_store(...)`.

- [ ] **Step 1 — Write the failing regression** `health_does_not_claim_dense_ready_when_ollama_down`. Assert: Probe local store read access, configured service API, model existence and test embedding; `denseAvailable` cooldown flag alone cannot yield Ready; disabled stays Disabled..
- [ ] **Step 2 — Prove red on baseline.** Run `cargo test -p davinci-coding-agent health_does_not_claim_dense_ready_when_ollama_down -- --nocapture`. Expected: test fails for the identified behavior (or mark as already fixed and report evidence; never make an unnecessary fix).
- [ ] **Step 3 — Implement one targeted change.** Implement bounded probes for Ollama `/api/tags` and `/api/embed`, map into typed error codes; hard timeout and no mutation in `check`.
- [ ] **Step 4 — Verify green and regression boundaries.** Re-run `cargo test -p davinci-coding-agent health_does_not_claim_dense_ready_when_ollama_down -- --nocapture`; expected: pass. Also run the module-specific suite, `cargo fmt --all -- --check`, and relevant Clippy targets. Save test transcript/receipt and verify no new permissions or default behavior changed.
- [ ] **Step 5 — Review and commit.** Reviewer inspects exact code and config/data compatibility; commit as `feat(setup): report actual component health`. Do not proceed when the gate is red.

### Task 3: Preserve local records and surface unreadable-store errors

**Files:** Modify: `src/native_extensions/vector_memory.rs`; Test: `tests/setup_memory_supervisor.rs`

**Interfaces:** Produces `LocalStoreHealth` on `VectorMemory` and checked `initialize()`/`status` contract without data loss.

- [ ] **Step 1 — Write the failing regression** `corrupt_or_unreadable_local_store_not_emptied`. Assert: A corrupt/unreadable existing JSONL store yields `LOCAL_STORE_UNREADABLE` and preserves bytes; an absent store creates an empty state only after safe initialization; no foreign records are discarded..
- [ ] **Step 2 — Prove red on baseline.** Run `cargo test -p davinci-coding-agent corrupt_or_unreadable_local_store_not_emptied -- --nocapture`. Expected: test fails for the identified behavior (or mark as already fixed and report evidence; never make an unnecessary fix).
- [ ] **Step 3 — Implement one targeted change.** Replace lossy `let _ = load_local()` on init with a retained read/error state surfaced through health and all writes; preserve legacy `.pi` path fallback.
- [ ] **Step 4 — Verify green and regression boundaries.** Re-run `cargo test -p davinci-coding-agent corrupt_or_unreadable_local_store_not_emptied -- --nocapture`; expected: pass. Also run the module-specific suite, `cargo fmt --all -- --check`, and relevant Clippy targets. Save test transcript/receipt and verify no new permissions or default behavior changed.
- [ ] **Step 5 — Review and commit.** Reviewer inspects exact code and config/data compatibility; commit as `fix(memory): make load failures visible and non-destructive`. Do not proceed when the gate is red.

### Task 4: Build an idempotent approved service supervisor

**Files:** Create: `src/native_extensions/service_supervisor.rs`; Modify: `src/runtime_host.rs`; Test: `tests/setup_memory_supervisor.rs`

**Interfaces:** Produces `ManagedServiceSupervisor::reconcile(service, consent, config) -> ComponentHealth`.

- [ ] **Step 1 — Write the failing regression** `supervisor_starts_only_approved_ollama_once`. Assert: Two simultaneous DaVinci launches result in at most one managed start; existing healthy Ollama is reused; no consent means ActionRequired not spawn; no arbitrary project binary may be started..
- [ ] **Step 2 — Prove red on baseline.** Run `cargo test -p davinci-coding-agent supervisor_starts_only_approved_ollama_once -- --nocapture`. Expected: test fails for the identified behavior (or mark as already fixed and report evidence; never make an unnecessary fix).
- [ ] **Step 3 — Implement one targeted change.** Reuse `ServiceAuthorization` from trust plan. Poll readiness with finite deadlines and per-service lock; verify resolved executable and limit environment/cwd; keep ownership identity rather than trusting stale PIDs.
- [ ] **Step 4 — Verify green and regression boundaries.** Re-run `cargo test -p davinci-coding-agent supervisor_starts_only_approved_ollama_once -- --nocapture`; expected: pass. Also run the module-specific suite, `cargo fmt --all -- --check`, and relevant Clippy targets. Save test transcript/receipt and verify no new permissions or default behavior changed.
- [ ] **Step 5 — Review and commit.** Reviewer inspects exact code and config/data compatibility; commit as `feat(setup): supervise approved local Ollama`. Do not proceed when the gate is red.

### Task 5: Download and validate embedding model with one-time consent

**Files:** Modify: `src/native_extensions/setup.rs`; Create: `src/native_extensions/setup_actions.rs`; Test: `tests/setup_memory_supervisor.rs`

**Interfaces:** Produces `SetupAction::{Install,DownloadModel,StartService,RepairProjection,Disable}` and `execute_approved_action(...)`.

- [ ] **Step 1 — Write the failing regression** `ollama_model_present_skips_pull_missing_requires_consent`. Assert: Existing compatible `embeddinggemma` is reused; missing model offers explicit Download with progress and cancellation; no download happens from a health probe; API vector dimensions match 768..
- [ ] **Step 2 — Prove red on baseline.** Run `cargo test -p davinci-coding-agent ollama_model_present_skips_pull_missing_requires_consent -- --nocapture`. Expected: test fails for the identified behavior (or mark as already fixed and report evidence; never make an unnecessary fix).
- [ ] **Step 3 — Implement one targeted change.** Use bounded pull progress tracking already present in `setup.rs`; preflight before repair; unknown or interrupted download is reconciled, not silently restarted.
- [ ] **Step 4 — Verify green and regression boundaries.** Re-run `cargo test -p davinci-coding-agent ollama_model_present_skips_pull_missing_requires_consent -- --nocapture`; expected: pass. Also run the module-specific suite, `cargo fmt --all -- --check`, and relevant Clippy targets. Save test transcript/receipt and verify no new permissions or default behavior changed.
- [ ] **Step 5 — Review and commit.** Reviewer inspects exact code and config/data compatibility; commit as `feat(setup): consented embedding model install`. Do not proceed when the gate is red.

### Task 6: Automatic restart recovery and degraded-state lifecycle

**Files:** Modify: `src/runtime_host.rs`; Create: `src/native_extensions/service_notifications.rs`; Test: `tests/setup_memory_supervisor.rs`

**Interfaces:** Produces `ServiceIncident { component, code, state, since, action }` and event emission consumed by TUI/status.

- [ ] **Step 1 — Write the failing regression** `cold_relaunch_restores_ollama_and_deduplicates_alerts`. Assert: After simulated reboot and service outage, approved autostart reattempts once, searches still function via clearly labeled lexical fallback, recovered state posts one notice, repeat failures back off..
- [ ] **Step 2 — Prove red on baseline.** Run `cargo test -p davinci-coding-agent cold_relaunch_restores_ollama_and_deduplicates_alerts -- --nocapture`. Expected: test fails for the identified behavior (or mark as already fixed and report evidence; never make an unnecessary fix).
- [ ] **Step 3 — Implement one targeted change.** Invoke reconcile asynchronously after trusted configuration loads; add state transition fingerprint, startup-bound timeout and capped retry/cooldown; never block a normal turn repeatedly.
- [ ] **Step 4 — Verify green and regression boundaries.** Re-run `cargo test -p davinci-coding-agent cold_relaunch_restores_ollama_and_deduplicates_alerts -- --nocapture`; expected: pass. Also run the module-specific suite, `cargo fmt --all -- --check`, and relevant Clippy targets. Save test transcript/receipt and verify no new permissions or default behavior changed.
- [ ] **Step 5 — Review and commit.** Reviewer inspects exact code and config/data compatibility; commit as `feat(setup): recover approved service at project reopen`. Do not proceed when the gate is red.

### Task 7: Safe automatic memory projection repair

**Files:** Modify: `src/native_extensions/vector_memory.rs`, `src/native_extensions/setup.rs`; Test: `tests/setup_memory_supervisor.rs`

**Interfaces:** Consumes health and supervisor; produces `repair_projection(max_batch) -> RepairReceipt`.

- [ ] **Step 1 — Write the failing regression** `repair_rebuilds_only_stale_vectors_without_deleting_records`. Assert: Stale dimension/model identity re-embeds from local records in bounded batches, preserves tombstones/source hashes, and does not silently switch providers or erase records after failed batch..
- [ ] **Step 2 — Prove red on baseline.** Run `cargo test -p davinci-coding-agent repair_rebuilds_only_stale_vectors_without_deleting_records -- --nocapture`. Expected: test fails for the identified behavior (or mark as already fixed and report evidence; never make an unnecessary fix).
- [ ] **Step 3 — Implement one targeted change.** Reuse existing embedding identity, reindex and atomic writes; check model dimension before beginning, idempotently retry unembedded records; expose repair receipt.
- [ ] **Step 4 — Verify green and regression boundaries.** Re-run `cargo test -p davinci-coding-agent repair_rebuilds_only_stale_vectors_without_deleting_records -- --nocapture`; expected: pass. Also run the module-specific suite, `cargo fmt --all -- --check`, and relevant Clippy targets. Save test transcript/receipt and verify no new permissions or default behavior changed.
- [ ] **Step 5 — Review and commit.** Reviewer inspects exact code and config/data compatibility; commit as `fix(memory): make safe reindex restartable`. Do not proceed when the gate is red.

### Task 8: Make /setup wizard and read-only check a single surface

**Files:** Modify: `src/native_extensions/setup.rs`, `src/davinci_interactive.rs`, `src/main.rs`; Test: `tests/setup_memory_supervisor.rs`

**Interfaces:** Consumes `SetupAction`, `ComponentHealth`; produces orchestrated screen and stable `setup` command JSON.

- [ ] **Step 1 — Write the failing regression** `setup_check_is_readonly_setup_offers_actions`. Assert: `/setup check` changes no files/services; `/setup` shows installed/missing and approved actions; Enter on Install/Download invokes only consented action; `/setup trust` compatibility retained..
- [ ] **Step 2 — Prove red on baseline.** Run `cargo test -p davinci-coding-agent setup_check_is_readonly_setup_offers_actions -- --nocapture`. Expected: test fails for the identified behavior (or mark as already fixed and report evidence; never make an unnecessary fix).
- [ ] **Step 3 — Implement one targeted change.** Keep native command JSON/RPC compatible; add TUI chooser and structured progress with one action per approval; do not require users to manually type shell commands.
- [ ] **Step 4 — Verify green and regression boundaries.** Re-run `cargo test -p davinci-coding-agent setup_check_is_readonly_setup_offers_actions -- --nocapture`; expected: pass. Also run the module-specific suite, `cargo fmt --all -- --check`, and relevant Clippy targets. Save test transcript/receipt and verify no new permissions or default behavior changed.
- [ ] **Step 5 — Review and commit.** Reviewer inspects exact code and config/data compatibility; commit as `feat(setup): one-command memory setup wizard`. Do not proceed when the gate is red.

### Task 9: Governor health and exact retrieval test

**Files:** Modify: `src/native_extensions/token_governor.rs`, `src/native_extensions/mod.rs`; Test: `tests/setup_memory_supervisor.rs`

**Interfaces:** Produces `TokenGovernorHealth` exposed through `/setup` and `/governor-status`.

- [ ] **Step 1 — Write the failing regression** `governor_compress_store_retrieve_after_restart`. Assert: A large synthetic tool output compresses with stable reference, hidden line can be retrieved exactly after reload; unwritable output store is Degraded, never reported Ready..
- [ ] **Step 2 — Prove red on baseline.** Run `cargo test -p davinci-coding-agent governor_compress_store_retrieve_after_restart -- --nocapture`. Expected: test fails for the identified behavior (or mark as already fixed and report evidence; never make an unnecessary fix).
- [ ] **Step 3 — Implement one targeted change.** Add non-destructive store probe and explicit disabled/no-op handling; preserve existing governor artifact format and old session output references.
- [ ] **Step 4 — Verify green and regression boundaries.** Re-run `cargo test -p davinci-coding-agent governor_compress_store_retrieve_after_restart -- --nocapture`; expected: pass. Also run the module-specific suite, `cargo fmt --all -- --check`, and relevant Clippy targets. Save test transcript/receipt and verify no new permissions or default behavior changed.
- [ ] **Step 5 — Review and commit.** Reviewer inspects exact code and config/data compatibility; commit as `fix(governor): verify recoverable stored output`. Do not proceed when the gate is red.

### Task 10: Feature disablement, migration and rollback

**Files:** Modify: `src/native_extensions/setup_profile.rs`, `src/native_extensions/setup.rs`, `src/native_extensions/mod.rs`; Test: `tests/setup_memory_supervisor.rs`

**Interfaces:** Produces `set_feature_enabled(feature, bool)` with transactional profile update and safe reconciliation.

- [ ] **Step 1 — Write the failing regression** `disabled_memory_does_not_spawn_or_reenable`. Assert: After explicit disable, restarts and `/setup check` never spawn Ollama or embed; existing records remain unchanged. Legacy `.pi` state, environment overrides, rollback and profile corruption behave explicitly..
- [ ] **Step 2 — Prove red on baseline.** Run `cargo test -p davinci-coding-agent disabled_memory_does_not_spawn_or_reenable -- --nocapture`. Expected: test fails for the identified behavior (or mark as already fixed and report evidence; never make an unnecessary fix).
- [ ] **Step 3 — Implement one targeted change.** Persist disabled intent before unloading hooks; migrate settings additively with backup, report override conflicts and disable records without deletion.
- [ ] **Step 4 — Verify green and regression boundaries.** Re-run `cargo test -p davinci-coding-agent disabled_memory_does_not_spawn_or_reenable -- --nocapture`; expected: pass. Also run the module-specific suite, `cargo fmt --all -- --check`, and relevant Clippy targets. Save test transcript/receipt and verify no new permissions or default behavior changed.
- [ ] **Step 5 — Review and commit.** Reviewer inspects exact code and config/data compatibility; commit as `feat(setup): honor persistent opt-outs`. Do not proceed when the gate is red.

### Task 11: Productionized status, doc and restart matrix

**Files:** Modify: `docs/vector-memory.md`, `docs/harness-audit-2026-10-01.md` (append a new audit, do not rewrite history); Create: `tests/setup_restart_matrix.rs`; Test: `tests/setup_restart_matrix.rs`

**Interfaces:** Produces a repeatable scenario receipt documenting what is and is not live-tested.

- [ ] **Step 1 — Write the failing regression** `repeated_restart_reconciles_without_manual_commands`. Assert: Ten simulated relaunches, Ollama missing/stopped/healthy, model missing, corrupted config and no-consent all yield correct setup state; notifications do not duplicate or hide failures..
- [ ] **Step 2 — Prove red on baseline.** Run `cargo test -p davinci-coding-agent repeated_restart_reconciles_without_manual_commands -- --nocapture`. Expected: test fails for the identified behavior (or mark as already fixed and report evidence; never make an unnecessary fix).
- [ ] **Step 3 — Implement one targeted change.** Use isolated temp app-data, fake HTTP server and deterministic process shim; add opt-in native Windows run with a real Ollama binary on a disposable profile.
- [ ] **Step 4 — Verify green and regression boundaries.** Re-run `cargo test -p davinci-coding-agent repeated_restart_reconciles_without_manual_commands -- --nocapture`; expected: pass. Also run the module-specific suite, `cargo fmt --all -- --check`, and relevant Clippy targets. Save test transcript/receipt and verify no new permissions or default behavior changed.
- [ ] **Step 5 — Review and commit.** Reviewer inspects exact code and config/data compatibility; commit as `test(setup): cover restart and recovery matrix`. Do not proceed when the gate is red.

## Release checklist

- [ ] All task-specific failing tests were demonstrated against the baseline or explicitly documented as already fixed.
- [ ] Every task passes focused tests, formatting and relevant Clippy; existing test suite passes with no suppressed failures.
- [ ] Scenario-level tests and negative cases run on disposable workspaces with fake services where possible; live-system tests run only with authorized dependencies.
- [ ] Documentation, rollback procedure, opt-out and error reporting are updated; no credential, memory text or user path appears in broad logs.
- [ ] Merge behind a bounded feature gate, collect real-world diagnostics, then separately authorize default-on behavior.
