# Strict Project Trust Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ask once before any project-controlled executable initialization; No exits, Yes enables trusted project resources without bypassing tool approvals.

**Architecture:** Keep the existing canonical trust store and host permissions, move project trust ahead of startup resource discovery, and add separate service-execution consents used later by the memory supervisor.

**Tech Stack:** Rust CLI + TUI, `ProjectTrustStore`, permission policy, existing sandbox.

**Spec:** [`../specs/2026-10-17-davinci-reliability-and-zero-maintenance-design.md`](../specs/2026-10-17-davinci-reliability-and-zero-maintenance-design.md)

**Dependencies / execution order:** Independent first milestone; must precede automatic service startup and active plugin design companions.

## Global Constraints

**Path shorthand:** Any unqualified `src/...` or `tests/...` in this plan is relative to `crates/davinci-coding-agent/`. Explicit `crates/...` paths are repo-root-relative. For the Design Studio plan, `design-ui/...` is relative to `crates/davinci-coding-agent/`.

- This is a plan, not an implementation. Start from a clean isolated worktree of the exact target `main` commit. Confirm historical audit findings still reproduce, especially after recent compaction changes.
- Rust workspace/pinned toolchains and existing feature flags, permission policy, sandbox, provider identity, durable stores and cancellation must stay intact. No live package installs, cloud requests or data mutations without explicit operator approval.
- Local JSONL remains authoritative. Preserve disabled-mode parity, existing slash commands, recovery safety and provenance; no mandatory Docker/Qdrant/SQLite.
- New tasks must use red-green TDD, targeted verification, one purpose per commit, reviewer sign-off and rollback/feature gate before default enablement.

## Review Focus

- Trust gate appears before plugins/hook/MCP loads and never after them — Task 1/2.
- An explicit deny survives app relaunch even when a parent is trusted — Task 3.
- Headless callers have no UI to ask: fail closed with clear error — Task 4.
- Trusting a folder does not authorize installing software or starting daemons — Task 5.
- Session switch must not retain trust from the previous folder — Task 6.

---

### Task 1: Reproduce and map project-resource initialization before trust

**Files:** Inspect: `src/main.rs`, `src/startup.rs`, `src/trust.rs`, `src/permissions.rs`, `src/davinci_interactive.rs`; Test: `tests/strict_trust_startup.rs`

**Interfaces:** Produces: `TrustGateDecision::{Accept,Decline,Preapproved,Unavailable}` and a pure `preflight_project_trust(&Path, &ProjectTrustStore, HostMode) -> TrustGateDecision` in `trust.rs` (proposed API).

- [ ] **Step 1 — Write the failing regression** `strict_trust_no_project_hook_before_prompt`. Assert: An untrusted workspace containing a runnable project hook/MCP configuration opens a trust gate before ANY project code executes; a trace of initialized components is empty at the gate..
- [ ] **Step 2 — Prove red on baseline.** Run `cargo test -p davinci-coding-agent strict_trust_no_project_hook_before_prompt -- --nocapture`. Expected: test fails for the identified behavior (or mark as already fixed and report evidence; never make an unnecessary fix).
- [ ] **Step 3 — Implement one targeted change.** Instrument and trace entrypoints; isolate the earliest trust check **before** extension/plugin/MCP/skills loads. Do not guess that the current warning placement is the only entrypoint.
- [ ] **Step 4 — Verify green and regression boundaries.** Re-run `cargo test -p davinci-coding-agent strict_trust_no_project_hook_before_prompt -- --nocapture`; expected: pass. Also run the module-specific suite, `cargo fmt --all -- --check`, and relevant Clippy targets. Save test transcript/receipt and verify no new permissions or default behavior changed.
- [ ] **Step 5 — Review and commit.** Reviewer inspects exact code and config/data compatibility; commit as `fix(trust): gate project resources before startup`. Do not proceed when the gate is red.

### Task 2: Two-option interactive trust gate and clean refusal

**Files:** Modify: `src/davinci_interactive.rs`, `src/main.rs`; Test: `tests/strict_trust_startup.rs`

**Interfaces:** Consumes `TrustGateDecision`; produces `request_interactive_trust_decision(...) -> TrustOutcome`, propagated to main before host startup.

- [ ] **Step 1 — Write the failing regression** `strict_trust_decline_exits_without_loading`. Assert: Default selection No; selecting No exits, saves explicit deny, loads no project executable resource and does not start background services. Selecting Yes persists trust and continues..
- [ ] **Step 2 — Prove red on baseline.** Run `cargo test -p davinci-coding-agent strict_trust_decline_exits_without_loading -- --nocapture`. Expected: test fails for the identified behavior (or mark as already fixed and report evidence; never make an unnecessary fix).
- [ ] **Step 3 — Implement one targeted change.** Bind the existing picker to the new preflight decision, two options only, canonical path display; do not first launch the interactive shell and only then warn.
- [ ] **Step 4 — Verify green and regression boundaries.** Re-run `cargo test -p davinci-coding-agent strict_trust_decline_exits_without_loading -- --nocapture`; expected: pass. Also run the module-specific suite, `cargo fmt --all -- --check`, and relevant Clippy targets. Save test transcript/receipt and verify no new permissions or default behavior changed.
- [ ] **Step 5 — Review and commit.** Reviewer inspects exact code and config/data compatibility; commit as `feat(trust): add strict yes-no startup gate`. Do not proceed when the gate is red.

### Task 3: Persist canonical decisions and handle parent/alias conflicts safely

**Files:** Modify: `src/trust.rs`; Test: `tests/strict_trust_startup.rs`

**Interfaces:** Produces: stored `ProjectTrustStoreEntry` semantics with provenance and a reusable `resolve_project_trusted(...)` check; no implicit broad trust.

- [ ] **Step 1 — Write the failing regression** `strict_trust_canonical_path_and_explicit_deny`. Assert: A symlink/alias maps to one canonical identity; explicitly denied child overrides trusted parent; corrupt/unreadable trust store fails closed with actionable error..
- [ ] **Step 2 — Prove red on baseline.** Run `cargo test -p davinci-coding-agent strict_trust_canonical_path_and_explicit_deny -- --nocapture`. Expected: test fails for the identified behavior (or mark as already fixed and report evidence; never make an unnecessary fix).
- [ ] **Step 3 — Implement one targeted change.** Reuse `ProjectTrustStore` locking and atomic writes; preserve existing store schema and parent-choice support where explicitly configured, but do not surface parent trust in the two-option startup prompt.
- [ ] **Step 4 — Verify green and regression boundaries.** Re-run `cargo test -p davinci-coding-agent strict_trust_canonical_path_and_explicit_deny -- --nocapture`; expected: pass. Also run the module-specific suite, `cargo fmt --all -- --check`, and relevant Clippy targets. Save test transcript/receipt and verify no new permissions or default behavior changed.
- [ ] **Step 5 — Review and commit.** Reviewer inspects exact code and config/data compatibility; commit as `fix(trust): preserve denial and path identity`. Do not proceed when the gate is red.

### Task 4: Noninteractive and SDK/RPC trust contract

**Files:** Modify: `src/main.rs`, `src/sdk.rs`, `src/rpc.rs`, `src/permissions.rs`; Test: `tests/strict_trust_startup.rs`

**Interfaces:** Consumes `preflight_project_trust`; produces structured `UNTRUSTED_PROJECT` exit/error and `project_trusted` runtime state.

- [ ] **Step 1 — Write the failing regression** `headless_untrusted_project_fails_closed`. Assert: When no interactive picker exists, untrusted project-controlled code is not loaded. Explicit trusted-host overrides are honored only through existing host authority channels, never through project config..
- [ ] **Step 2 — Prove red on baseline.** Run `cargo test -p davinci-coding-agent headless_untrusted_project_fails_closed -- --nocapture`. Expected: test fails for the identified behavior (or mark as already fixed and report evidence; never make an unnecessary fix).
- [ ] **Step 3 — Implement one targeted change.** Install a headless admission check returning a stable machine-readable denial before resource discovery. Preserve global-only nonexecuting behavior where applicable.
- [ ] **Step 4 — Verify green and regression boundaries.** Re-run `cargo test -p davinci-coding-agent headless_untrusted_project_fails_closed -- --nocapture`; expected: pass. Also run the module-specific suite, `cargo fmt --all -- --check`, and relevant Clippy targets. Save test transcript/receipt and verify no new permissions or default behavior changed.
- [ ] **Step 5 — Review and commit.** Reviewer inspects exact code and config/data compatibility; commit as `fix(trust): protect headless entrypoints`. Do not proceed when the gate is red.

### Task 5: Separate folder trust from service and tool execution grants

**Files:** Modify: `src/permissions.rs`, `src/native_extensions/setup.rs`; Create: `src/native_extensions/service_authority.rs`; Test: `tests/strict_trust_startup.rs`

**Interfaces:** Produces: `ServiceConsent { action, executable_identity, scope, approved_at }` plus `ServiceAuthorization::check(...)` consumed by setup/supervisor.

- [ ] **Step 1 — Write the failing regression** `folder_trust_does_not_grant_service_start`. Assert: Trusting folder alone cannot install, download, spawn Ollama or start Docker and cannot bypass tool ask/sandbox gates..
- [ ] **Step 2 — Prove red on baseline.** Run `cargo test -p davinci-coding-agent folder_trust_does_not_grant_service_start -- --nocapture`. Expected: test fails for the identified behavior (or mark as already fixed and report evidence; never make an unnecessary fix).
- [ ] **Step 3 — Implement one targeted change.** Define user-global, host-owned scoped consent receipts; deny project-written config from granting new service verbs.
- [ ] **Step 4 — Verify green and regression boundaries.** Re-run `cargo test -p davinci-coding-agent folder_trust_does_not_grant_service_start -- --nocapture`; expected: pass. Also run the module-specific suite, `cargo fmt --all -- --check`, and relevant Clippy targets. Save test transcript/receipt and verify no new permissions or default behavior changed.
- [ ] **Step 5 — Review and commit.** Reviewer inspects exact code and config/data compatibility; commit as `feat(trust): separate runtime service consent`. Do not proceed when the gate is red.

### Task 6: Full lifecycle trust revalidation on session switch and reload

**Files:** Modify: `src/davinci_interactive.rs`, `src/main.rs`, `src/runtime_host.rs`; Test: `tests/strict_trust_startup.rs`

**Interfaces:** Consumes trust gate; produces `rebind_trusted_workspace(...) -> Result<(),TrustError>` (proposed host adapter).

- [ ] **Step 1 — Write the failing regression** `project_switch_and_reload_recheck_trust`. Assert: Switching project roots and reloading resources applies the correct decision and never retains previous workspace executable resources..
- [ ] **Step 2 — Prove red on baseline.** Run `cargo test -p davinci-coding-agent project_switch_and_reload_recheck_trust -- --nocapture`. Expected: test fails for the identified behavior (or mark as already fixed and report evidence; never make an unnecessary fix).
- [ ] **Step 3 — Implement one targeted change.** Recheck resolved target folder trust and rebuild trusted scope before loading new project resources; preserve saved deny and saved grant semantics.
- [ ] **Step 4 — Verify green and regression boundaries.** Re-run `cargo test -p davinci-coding-agent project_switch_and_reload_recheck_trust -- --nocapture`; expected: pass. Also run the module-specific suite, `cargo fmt --all -- --check`, and relevant Clippy targets. Save test transcript/receipt and verify no new permissions or default behavior changed.
- [ ] **Step 5 — Review and commit.** Reviewer inspects exact code and config/data compatibility; commit as `fix(trust): revalidate when workspace changes`. Do not proceed when the gate is red.

## Release checklist

- [ ] All task-specific failing tests were demonstrated against the baseline or explicitly documented as already fixed.
- [ ] Every task passes focused tests, formatting and relevant Clippy; existing test suite passes with no suppressed failures.
- [ ] Scenario-level tests and negative cases run on disposable workspaces with fake services where possible; live-system tests run only with authorized dependencies.
- [ ] Documentation, rollback procedure, opt-out and error reporting are updated; no credential, memory text or user path appears in broad logs.
- [ ] Merge behind a bounded feature gate, collect real-world diagnostics, then separately authorize default-on behavior.
