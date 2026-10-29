# Completion and Evaluation Runtime Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans task-by-task. All steps use checkboxes.

**Spec:** `docs/superpowers/specs/2026-10-28-claude-runtime-capability-convergence-design.md`. **Master:** `docs/superpowers/plans/2026-10-28-claude-runtime-gap-closure-master.md`.
**Global constraints:** Rust 1.88/Cargo locked; owner approval before implementation and commits; preserve dirty working tree and openai-codex model/effort/OAuth, effect latch, permissions and user constraints; use deterministic offline tests. File paths are repo-relative, review at execution time. A plan item is not a completed change.

**Review focus:** stale verification after edits; restarted tasks losing receipts; permission revocation mid-operation; error after side-effect commit; unrelated tests passing falsely. Every corresponding task must test relevant cases.

**Goal:** Complete G-06/G-07/G-02 safely and establish a defensible verification/evaluation baseline. **Architecture:** extend present evidence generation/state and host-owned receipts; no mandatory review LLM. **Tech Stack:** `davinci-agent`, `davinci-coding-agent`, `davinci-evals`, Rust/JSON fixtures.

### Task V1 — Validate and finish evaluation oracles (G-06)
**Files:** Modify only as necessary `crates/davinci-evals/src/behavior/{scenario.rs,validity.rs,oracle.rs,runner.rs,executor.rs}`, fixtures `behavior/verified-core.json`, `repos/verified-*-fixture/`, `solutions/`; tests co-located. **Interface:** `ScenarioValidity::{Verified,StructuralOnly,Invalid}`; `OracleVerdict`. No second classifier.
- [ ] Write RED tests proving `git diff --check`, missing tests and missing files cannot produce verified success; baseline must fail; correct variant must pass; unchanged, syntactic-only and incorrect variants must fail. Run `cargo test --locked -p davinci-evals --lib`, confirm expected RED on unimplemented cases.
- [ ] Inspect six currently local verified-core scenarios; validate every oracle checks meaningful semantics; ensure test executables are sandboxed/disposable and timeout/setup errors excluded from denominators.
- [ ] Integrate existing implementations rather than rewrite; run correctness + negative variants at least twice to catch nondeterminism; record corpus digest and `core-200` split (currently 90 invalid/110 structural-only).
- [ ] Gate all success reporting on validity=Verified; preserve structural metrics separately. GREEN `cargo test --locked -p davinci-evals`; record counts and exit codes. No live benchmark.

### Task V2 — Persist generation-scoped completion status (G-07)
**Files:** `crates/davinci-agent/src/{lib.rs,turn.rs,events.rs,verification_notice.rs,completion_status_tests.rs}`, `crates/davinci-coding-agent/src/{main.rs,davinci_interactive.rs}` if host display needs it; existing session JSONL. **Interface:** existing `CompletionStatus`, `CompletionStatusRecord`, `Agent::completion_status()`, `record_terminal_completion_status()`.
- [ ] RED: modify → no test; failed test; pass→later edit; coverage of wrong path; crash after unknown effect; resume; compaction; explicit stop; read-only task. Assert only final-generation passing checks become Verified.
- [ ] Trace current dirty changes and session serde: verify old transcript loading, new terminal state append, correct host status, no stale receipts across fork, no extra request on a text-only turn.
- [ ] GREEN with minimum changes in existing owner; run `cargo test --locked -p davinci-agent --lib completion_status` plus session and compaction integrations; preserve one-reminder budget and explicit Unverified result rather than unbounded Stop-hook imitation.

### Task V3 — Host-owned subagent receipts (G-02)
**Files:** `crates/davinci-agent/src/{subagent_receipt.rs,subagent.rs,team.rs,lib.rs}`; `crates/davinci-coding-agent/src/main.rs`; workflow result projection when relevant. **Interface:** existing `VerificationReceipt`, `ReceiptStatus`, `WorkspaceSnapshot`, `receipt_slot()`, `receipt_for_agent()`.
- [ ] RED: child no tools + false success; failed test + false success; test then edit; wrong artifact digest; concurrent workers; crash/edit; snapshot error; canceled verifier; path outside workspace. Parent never says Verified based on prose or exit=0.
- [ ] Verify trusted host computes workspace digest and test evidence post-final mutation; bind workspace identity, generation, command, exit, receipt and result id. No untrusted agent-authored receipt or full tree hash mistaken for semantic proof.
- [ ] Persist receipt to existing `AgentResultRecorded` before mailbox notification, preserving at-most-once delivery. GREEN: targeted `cargo test --locked -p davinci-agent --lib subagent_receipt`, subagent integration suites, `cargo test --locked -p davinci-coding-agent` as feasible.

### Task V4 — Objective end-to-end gates and metric contract
**Files:** `crates/davinci-evals/src/{optimization.rs,behavior/*}`, `scripts/bench/{campaign.py,readiness_metrics.py}`, `docs/readiness/evaluation.md` where changes are proven necessary. **Interface:** success numerator only independent oracles; costs include failed attempts, missing usage stays null.
- [ ] Add synthetic end-to-end cases: correct, incorrect, no-work false claim, stale verification, unverified worker, resumed workflow with stale result; use provided fixtures, not model statements.
- [ ] Test numerator/denominator, false claim and request-per-verified-success; record unknown cache and subscription quota separately; do not use API token prices to claim subscription savings.
- [ ] Run `cargo fmt --all --check`; affected Clippy and full evaluator suites; record outcome. Live model A/B needs new explicit budget/authorization.

**Acceptance:** V1–V3 tests green and integrated; no false verified status from a model claim; six valid fixture oracles remain meaningful; outdated core-200 cannot inflate success. Document failures/limitations and rollback per task. No commits by default.
