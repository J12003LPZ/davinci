# Multi-Agent and Durable Workflow Runtime Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans task-by-task. All steps use checkboxes.

**Spec:** `docs/superpowers/specs/2026-10-28-claude-runtime-capability-convergence-design.md`. **Master:** `docs/superpowers/plans/2026-10-28-claude-runtime-gap-closure-master.md`.
**Global constraints:** Rust 1.88/Cargo locked; owner approval before implementation and commits; preserve dirty working tree and openai-codex model/effort/OAuth, effect latch, permissions and user constraints; use deterministic offline tests. File paths are repo-relative, review at execution time. A plan item is not a completed change.

**Review focus:** stale verification after edits; restarted tasks losing receipts; permission revocation mid-operation; error after side-effect commit; unrelated tests passing falsely. Every corresponding task must test relevant cases.

**Goal:** Adapt Claude's bounded, discoverable and resumable orchestration capabilities without copying its stale cache, unsafe claim acceptance or arbitrary concurrency constants. **Architecture:** preserve DaVinci worker admission, task board, journal and artifact owners. **Tech Stack:** `davinci-agent`, `davinci-coding-agent`, `davinci-session`, existing SQLite operation journal.

### Task O1 — Bounded worker resources (G-11)
**Files:** `crates/davinci-agent/src/runtime/worker_admission.rs`, `subagent.rs`, `runtime/workflow/{limits.rs,executor.rs}`, relevant `coding-agent/main.rs`. **Interfaces:** existing `WorkerAdmission` and lease; avoid a second scheduler.
- [ ] RED: unconfigured budget cannot admit unbounded workers; waiter times out or cancels; panic releases lease; priority does not starve existing owner; queue bounded; 1 agent succeeds. Preserve existing one-shot max four and workflow cap configuration until measurement.
- [ ] Implement small explicit default admission policy (propose; freeze numeric limits only after local baseline), configured byte reservation + queue cap and timeout; report estimates as estimates, not true RSS ceilings.
- [ ] GREEN `cargo test --locked -p davinci-agent --lib worker_admission`; concurrency stress and resident-memory measurements on a real local fixture.

### Task O2 — Optional fork/depth/agent handoff (X-01)
**Files:** `crates/davinci-agent/src/{subagent.rs,agent_profiles.rs,team.rs}`, `crates/davinci-coding-agent/src/main.rs`, relevant tool schemas. **Interfaces:** existing `AgentSpawnMode`, worker authorization, reservations, child session policy.
- [ ] RED: depth-one default unchanged; explicit opt-in depth-two/three respects tool-surface removal, permission inheritance, context fork hash, worker identity, abort propagation and resource cap; nested denial returns exact safe reason.
- [ ] Create fresh-vs-fork modes with **safe bounded immutable context selection**; never expose parent secrets, project instructions or tools beyond authority; separate usage and verified result.
- [ ] Benchmark against solo/fresh on valid offline request-shape fixtures; only expand depth default after approved coding-success study. GREEN subagent integration + permission tests. **Gate:** optional if no measured benefit.

### Task O3 — Durable workflow lifecycle and stale cache prevention (G-05)
**Files:** `crates/davinci-agent/src/runtime/workflow/{state.rs,executor.rs,tools.rs,spec.rs}`, `team.rs`, `mailbox.rs`; existing journal/outbox. **Interface:** persist `WorkflowSpec`, launch revision, phase status, notification ID, artifact/repo fingerprint; never new shadow registry.
- [ ] RED: kill after completion before mailbox delivery; restart and list status; resume without caller-supplied spec; duplicate notice ignored; denied/revoked launch cannot resume; modify source after worker success → cached result not reported verified.
- [ ] Persist spec/status/launch via existing durable operations; publish only after committed artifacts and receipts; rehydrate exactly once; preserve safe rerun authority and partial task state.
- [ ] GREEN targeted workflow `wor*` integration suites, restart fixture, and report unknown outcome on crashed mutating worker.

### Task O4 — Worktree cleanup and orphan discovery (G-03)
**Files:** `crates/davinci-agent/src/worktree.rs` and subagent cleanup callers. **Interface:** leased worktree identity, cleanup result, orphan registry/report.
- [ ] RED: failed remove/branch-delete remains visible, dirty worktree never deleted, path already exists reports existing owner, process crash leaves detectable orphan, repeated cleanup idempotent.
- [ ] Add startup scan of managed prefixes, record errors durably, block silent reuse; **never auto-delete dirty/unknown worktrees**. Preserve existing tracked artifacts.
- [ ] GREEN `cargo test --locked -p davinci-agent --lib worktree`; manual opt-in cleanup only after user authorization.

### Task O5 — Workflow budget/admission consistency (G-18)
**Files:** `runtime/workflow/{limits.rs,executor.rs}`, `agent/stats.rs`. **Interface:** enforce only supported budget dimensions; unknown usage cannot authorize more spend falsely.
- [ ] RED: budget exhausted before launch, simultaneous workers don't race allowance, missing provider usage yields unknown/stop by configured policy, non-Codex modes unaffected.
- [ ] Honor existing user budget caps where supported without cost-imputation to Codex included subscriptions; if dollars unsupported on backend, reject with actionable error and offer request/token caps instead.
- [ ] GREEN workflow limit tests and abort/restart tests.

### Task O6 — Optional detached-session supervisor (X-02)
**Files:** `crates/davinci-session/src/lib.rs`, `crates/davinci-coding-agent/src/{main.rs,shutdown.rs}`, optional `davinci-server`. **Interface:** stable job/session IDs, owner lease, append-only status/log, explicit stop/list/attach/resume.
- [ ] RED: two owners cannot append same live session; detached task survives client exit only when intentional; crash stops ownership and status remains recoverable; stop != delete; unauthorized attach denied.
- [ ] Reuse session journal and operation supervisor; opt-in owner process only if background CLI workflow needs it. Do not introduce mandatory daemon or duplicate histories.
- [ ] Verify CLI contract with disposable process tree, Windows jobs and Unix signals; manual TTY attach test required before claiming full parity.

### Task O7 — Team and inter-process message contract (X-13)
**Files:** `crates/davinci-agent/src/{team.rs,mailbox.rs,runtime/tasks.rs}`, optional secure local server. **Interface:** message id, sender authority, owner generation, journal-first outbox, dedupe scope.
- [ ] RED: duplicate delivery, leader crash, teammate stop, cross-session spoofed sender, mailbox full, permission revocation, late completion and lost ack.
- [ ] Preserve current at-most-once scoped mailbox and task-board leases; add opt-in peer routing only once S3 IPC authentication and owning process identity are proved. Claude's team IPC is not dynamically established; do not invent protocol parity.
- [ ] GREEN team/roster/journal tests; cross-platform session boundaries explicit.

**Acceptance:** durable and discoverable workflow state; bounded resources; no unsafe stale reuse; orphan worktrees visible; optional deeper workers and daemon remain off until passed tests and user approval. All original verification receipts remain linked.
