# DaVinci Runtime Integrity — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Establish fault-tolerant tool execution, valid tool-result histories, generation-safe compaction and exclusive worktree ownership in DaVinci's real runtime paths.

**Architecture:** Build on the existing operation journal/ToolCallLedger, turn loop, Context VM and WorktreeManager. Extend integration coverage at the public Agent/CLI seam; do not duplicate lower-level stores. Fix only a red failure or a proven missing boundary.

**Tech Stack:** Rust 1.88.0; current `davinci-agent` tests, offline local subprocess fixtures, SQLite/JSONL, temp Git repositories.

**Spec:** `docs/superpowers/specs/2026-09-14-davinci-harness-optimization-design.md` plus `00-INDEX.md` in this plan pack.

## Global constraints

- Use current repo `AGENTS.md` and the master program's global constraints.
- Do not assume the existence of `tests/operation_crash.rs` means the production shell→session→resume path has been fully verified.
- Preserve unknown outcomes; a started external mutation is **never** safe to replay solely because result publication was interrupted.
- Keep current authorization, retry safety, result projection, cancellation and duplicate-ID controls.

## Review focus

- Started, side-effecting tool crashes after START and before terminal result; unknown state on resume (R1).
- Side effect completed but result outbox publication crashes; publish at most once and do not execute twice (R1).
- Incomplete SSE and tool/result orphaning around compaction; no unpaired wire history and no side effects from uncommitted stream (R2).
- Summary publication races with a concurrent user turn; stale summary discarded (R3).
- Concurrent identical branch lease requests; one winner, no dirty worktree deletion (R4).

---

### Task R1 — End-to-end killed-tool resume: actual Agent/CLI path

**Files:** Inspect `crates/davinci-agent/src/turn.rs`, `src/tool_ledger.rs`, `src/runtime/operations/{coordinator.rs,recovery.rs,retry_safety.rs}`, `src/runtime/session.rs`; existing tests `tests/{operation_crash,operation_recovery,operation_process}.rs`. Create, only if coverage missing: `crates/davinci-agent/tests/agent_tool_resume_integration.rs`. Add to existing `docs/runtime/execution-recovery-verification.md` evidence rather than creating a second recovery report.

**Interfaces:** Consumes existing Agent tool dispatch, journal/restart and session persistence interfaces; produces verified outcome classification and next model observation. No new public API planned.

- [ ] **Step 1 — Inspect and record baseline:** Trace a real `Agent::run_loop` shell call through durable intent, effect start, process supervisor, result commit, next-model projection and resumed-session recovery. Mark exactly which transition existing crash tests cover.
- [ ] **Step 2 — Write failing test if needed:** `agent_crash_after_started_effect_reports_unknown_without_replay`: fixture command creates `START`, sleeps, then creates `END`; terminate owner and worker after `START` and before `END`; resume the same session; assert count(`START`)=1, count(`END`)=0, result explicitly *outcome unknown*, and no new dispatch of that operation. Add `agent_crash_after_effect_before_publication_projects_once`: assert postcondition observed, tool result projected once and operation never repeated.
- [ ] **Step 3 — Run red:** `cargo test -p davinci-agent --test agent_tool_resume_integration --offline --locked` (or existing equivalent if it already covers the actual Agent seam). Expected before any fix: failing new invariant, or document `PRESENT_AND_PROVEN` if already green.
- [ ] **Step 4 — Minimal repair when red:** Update the authoritative operation state transition/recovery owner, not a parallel flag or prompt. Persist distinct `NOT_STARTED`, `STARTED_UNKNOWN`, `RESULT_COMMITTED` semantics through the actual shell adapter and publication boundary.
- [ ] **Step 5 — Green and verify regression:** Run new test plus `cargo test -p davinci-agent --test operation_crash --test operation_recovery --test operation_idempotency --test operation_publication --offline --locked`. Expected: zero regressions, exactly-once durable projection when committed, zero automatic replay for unknown effects.
- [ ] **Step 6 — Review/deliver:** Record injected phase, state transitions, captured marker counts, and diff; **commit only if separately authorized**.

### Task R2 — Tool-use/result history pairing and cut-stream consistency

**Files:** Inspect `crates/davinci-agent/src/{turn.rs,tool_ledger.rs,session_persistence_tests.rs}`, `crates/davinci-ai/src/{stream_decoder.rs,stream_decoder_anthropic.rs}`, `crates/davinci-agent/tests/ws3_stream_cut.rs`. Create only if uncovered: `crates/davinci-agent/tests/tool_result_pairing_recovery.rs`.

**Interfaces:** Input is saved/provider history and authoritative tool ledger states; output is either a valid history with exactly one result per committed tool use or a typed, fail-closed recovery error. Reuse existing persistence and history projection types; avoid inventing replacement wire types.

- [ ] **Step 1 — Map existing paths:** Determine where orphaned `tool_use`/`tool_result`, duplicates, aborted streams and resumed compacted history are validated; document what may be repaired vs rejected.
- [ ] **Step 2 — Write tests:** `cut_stream_with_complete_json_never_dispatches`; `orphan_result_id_fails_closed`; `missing_result_after_recovery_has_explicit_nonexecution_or_unknown`; `duplicate_result_does_not_double_publish`; `compacted_history_retains_matching_ids`. Assert each committed model tool call is associated by ID (never by output order), and a partial stream has no tool effect.
- [ ] **Step 3 — Verify red where missing:** `cargo test -p davinci-agent --test tool_result_pairing_recovery --offline --locked`; baseline `cargo test -p davinci-agent --test ws3_stream_cut --offline --locked`.
- [ ] **Step 4 — Make smallest correction:** Reconcile the active transcript at the existing resume/context-projection boundary, anchored in ledger state. Never fabricate a success result or execute again as history repair.
- [ ] **Step 5 — Verify:** Rerun focused tests, plus `cargo test -p davinci-agent --test session_persistence_tests --offline --locked` only if this is a Cargo test target; otherwise run the corresponding `--lib` test filter. Expected: structurally valid provider history, no unsafe replay and no altered permission state.
- [ ] **Step 6 — Deliver:** Record malformed fixture traces and whether each branch repairs or rejects; **no commit without authorization**.

### Task R3 — Context compaction publication freshness

**Files:** Inspect `crates/davinci-agent/src/compaction.rs`, `src/runtime/context_vm/{fold.rs,store.rs,reducer.rs,compiler.rs}`, `src/prepared_context.rs`; existing `tests/{compaction_local_fallback,context_vm_integrity,context_vm_replay,context_vm_durability}.rs`. Add `tests/compaction_generation_race.rs` if no actual production-path race test exists.

**Interfaces:** Consumes current active transcript generation/revision and asynchronous summary/fold outcome; produces a replacement **only if** the source revision still matches. Use existing Context VM/turn generation types.

- [ ] **Step 1 — Inspect:** Locate generation acquisition, snapshot read, fold request, compare-and-publish and fallback. Check whether in-flight summarization and current user turns share the same publication owner.
- [ ] **Step 2 — Add race tests:** `stale_summary_never_overwrites_new_turn`, `failed_or_empty_fold_leaves_active_state_unchanged`, `resume_after_compaction_keeps_tool_pairing` and `source_ref_survives_fold`. Use barriers to hold summarization until a new turn changes the revision; assert stale publication is refused and the newer user constraint remains authoritative.
- [ ] **Step 3 — Red check:** `cargo test -p davinci-agent --test compaction_generation_race --offline --locked` or existing equivalent.
- [ ] **Step 4 — Fix if red:** Apply revision/epoch fencing at the one authoritative transcript replacement point, not only at the summarizer request. Preserve an unmodified active view on failure and retry only from a fresh snapshot.
- [ ] **Step 5 — Green:** Run new test and `cargo test -p davinci-agent --test context_vm_integrity --test context_vm_replay --test compaction_local_fallback --offline --locked`. Expected: stale update rejected, no lost constraints, no duplicate result IDs.
- [ ] **Step 6 — Deliver:** Record a successful interleaving and forbidden interleaving trace; **commit only if authorized**.

### Task R4 — Atomic worktree lease/branch ownership

**Files:** Inspect `crates/davinci-agent/src/runtime/worktree.rs` (`WorktreeManager::create_lease` and release), `src/runtime/team.rs`, `src/subagent.rs`, existing `tests/operation_git.rs`. Create `tests/worktree_lease_concurrency.rs` if needed.

**Interfaces:** Existing `WorktreeManager` lease owner is authoritative. Do not add a second worktree register.

- [ ] **Step 1 — Inspect:** Confirm that branch uniqueness and path creation are atomically coordinated across threads/processes; distinguish in-memory lock from cross-process Git ownership checks.
- [ ] **Step 2 — Write tests:** `same_branch_concurrent_acquisition_has_one_winner` (two synchronized owners; one lease and one `BranchInUse`); `dirty_worktree_is_never_deleted_by_release`; `stale_owner_cannot_release_new_lease`; `nested_cwd_uses_git_toplevel`.
- [ ] **Step 3 — Red:** `cargo test -p davinci-agent --test worktree_lease_concurrency --offline --locked`, or existing equivalent if complete.
- [ ] **Step 4 — Minimal repair:** Make ownership acquisition and cleanup atomic at the existing manager/Git boundary, using repository-native locking and fail-closed verification. Do not delete a path on uncertain owner or dirty status.
- [ ] **Step 5 — Green:** New tests plus `cargo test -p davinci-agent --test operation_git --test delegation_policy --offline --locked`. Expected: exactly one lease owner per branch and preserved user changes.
- [ ] **Step 6 — Deliver:** Include race trace, OS coverage caveat and diff; **commit only if authorized**.

## Gate G1

Mark G1 complete only after R1–R4 are either **PRESENT_AND_PROVEN** with existing source+integration test evidence or fixed with red→green tests. Do not change unsafe replay, compaction or concurrency defaults until this is true.
