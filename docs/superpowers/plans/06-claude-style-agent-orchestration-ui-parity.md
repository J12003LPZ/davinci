# Claude Code-Style Agent Orchestration and Live TUI Parity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver functional 1:1 parity with the supplied Claude Code multi-agent terminal workflow: main-agent orchestration, live clickable worker roster and transcripts, real branch/worktree detail, token/time visibility, queued steering and follow-ups, and Esc/click-to-main navigation without interrupting workers.

**Architecture:** Extend the existing Rust runtime registry, `SubagentProgress`, agent mailbox/queues and DaVinci TUI rather than creating a second scheduler. Normalize per-worker activity/usage/worktree snapshots into a bounded event stream, render a focusable main/worker view, and durably record accepted message delivery states. Preserve permission and session boundaries.

**Tech Stack:** Rust, davinci-agent, davinci-coding-agent, davinci-tui (ratatui/crossterm), session JSONL, Git/worktree metadata, existing RuntimeBus/WorkerController, unit + PTY/ConPTY integration fixtures.

**Spec:** [`../specs/2026-10-17-claude-style-agent-navigation-design.md`](../specs/2026-10-17-claude-style-agent-navigation-design.md)

**Source baseline:** Inspected `main` through `c1973e80e81744a1189c32b6e3368411fc1b6450` on 2026-10-17/18. Rebase every task onto current main in an isolated worktree and rerun its failing regression before editing; recent commits modify `davinci_interactive.rs`. This packet is **planning only**, not validated implementation.

## Global Constraints

- Functional parity means screenshot workflow, not exact reproduction of a proprietary CLI build or pixel-perfect host fonts.
- Keep `agentTeams=false` and `dynamicWorkflows=false` defaults. One-shot and background subagents work without opting into teams; existing worker permissions and authority ceilings remain intact.
- Main and worker view changes are **presentation only**. `Esc` in worker focus returns main and never aborts work; `Esc` on main retains existing behavior.
- Show truthful tokens: separate provider-reported from provisional estimates, and unknown when neither exists. Live elapsed is based on monotonic time; cannot reset on UI navigation.
- Show Git branch only when verified from worktree state, otherwise `shared workspace — no separate branch`, `detached HEAD`, or `unknown` as appropriate.
- An acknowledged queued instruction cannot disappear silently. Journal acceptance before ACK; distinguish queued/delivered/applied/rejected/unknown and replay idempotently. Do not restart completed one-shot workers automatically.
- Never leak child hidden reasoning, secrets, raw escape codes, untrusted tool payload as executable instructions, or another session's worker events.
- Keep UI usable at 40-column width and under heavy stream load. Backward-compatible JSONL and RPC format upgrades; retain full test and no-feature behavior.
- Before any code use Superpowers systematic-debugging/TDD, isolation via worktree, review per task, and verification-before-completion. User review of spec/plan is required before execution.

## Review Focus

1. A background worker finishes just as a user submits a message: exactly one terminal receipt, not silent drop or restart — Task 11.
2. A worker emits a credential or ANSI control escape inside tool output: redacted, inert UI display — Task 5 and Task 13.
3. User hits Esc while viewing a running worker: main view returns with no abort flag and composer draft unchanged — Task 7.
4. DaVinci crashes between queue persistence, delivery and application: replay never silently loses acknowledged instructions or executes them twice — Task 10.
5. Agent switches worktrees or a deleted worktree has no Git branch: UI displays actual/unknown state without wrong branch attribution — Task 4.

## Delivery order

| Release gate | Tasks | Independently shippable result |
|---|---|---|
| A — Runtime observability | 1–5 | Honest activity, branch and token snapshots; no UI navigation changed |
| B — Live TUI parity | 6–9 | Clickable worker focus, Esc-to-main, live refresh, worker composer targeting |
| C — Durable orchestration | 10–12 | Queue receipts, terminal-state handling, cross-mode/main coordination and RPC consistency |
| D — hardening/acceptance | 13–14 | Security, stress, Windows/Linux full-stack approval and rollout |

---

### Task 1: Define screenshot parity contract and reproducible fixtures

**Files:** Create `crates/davinci-tui/tests/agent_focus_parity.rs`; Modify `crates/davinci-tui/src/davinci/fixtures.rs`; Extend `crates/davinci-coding-agent/src/davinci_interactive_tests.rs`.

**Interfaces:** Create test fixture `AgentParityScenario` with main, three workers (oneshot/background/teammate), deterministic event timestamps/usage, three worktree modes; expose it only from test helper modules.

- [ ] **Step 1 — Failing test:** `claude_reference_roster_and_focus_contract` checks screenshot roster labels, main row, running/finished states, activity, token/time cells and presence of return-to-main affordance. Separate `one_shot_workers_render_when_teams_disabled` tests default settings.
- [ ] **Step 2 — Baseline red:** `cargo test -p davinci-tui claude_reference_roster_and_focus_contract -- --nocapture`; record missing capability vs already-fixed result.
- [ ] **Step 3 — Implement fixtures only:** Create a frozen reference scenario and expected semantic rows, including an explicit unknown-usage worker; avoid snapshots that assert OS-specific font pixels.
- [ ] **Step 4 — Verify fixture stability:** Run `cargo test -p davinci-tui agent_focus_parity -- --nocapture`; expected stable red for unimplemented behavior, self-tests green.
- [ ] **Step 5 — Review and commit:** `test(tui): pin agent focus parity contract`.

### Task 2: Normalize reliable worker activity and identity events

**Files:** Modify `crates/davinci-agent/src/subagent_progress.rs`, `src/runtime/events.rs`, `src/runtime/registry.rs`; Create `crates/davinci-agent/src/runtime/worker_activity.rs` and `crates/davinci-agent/tests/worker_activity_parity.rs`.

**Interfaces:** `WorkerActivityEvent { sequence: u64, agent_id: AgentId, generation: u64, timestamp_ms: i64, kind: WorkerActivityKind, summary: String, outcome: Option<String> }`; `WorkerActivityStore::append(event)`, `::after(agent_id, sequence, max)`. Hook `ProgressReporter::observe()` to the store, retaining existing `AgentEvent::SubagentProgress` API.

- [ ] **Step 1 — Failing tests:** `mixed_worker_modes_emit_ordered_agent_scoped_activity` and `tool_start_finish_are_correlated`; assert no cross-agent logs, monotonic cursor and stable tool ID matching.
- [ ] **Step 2 — Red:** `cargo test -p davinci-agent mixed_worker_modes_emit_ordered_agent_scoped_activity -- --nocapture`.
- [ ] **Step 3 — Implement:** Add typed normalized events at existing tool start/end and lifecycle boundaries; retain current eight-call summary compatibility; separate sensitive raw tool data from host-safe display summaries.
- [ ] **Step 4 — Green:** Targeted tests, `cargo test -p davinci-agent subagent_progress`, `cargo fmt --all -- --check`.
- [ ] **Step 5 — Review/commit:** `feat(agents): normalize worker activity stream`.

### Task 3: Make token usage and elapsed time truthful everywhere

**Files:** Modify `crates/davinci-agent/src/subagent_progress.rs`, `src/runtime/control.rs`, `src/runtime/registry.rs`, `crates/davinci-coding-agent/src/davinci_surfaces.rs`, `crates/davinci-tui/src/davinci/model.rs`.

**Interfaces:** `WorkerUsage { reported_total: Option<u64>, provisional_total: Option<u64>, source: UsageSource }`; `WorkerSnapshot.usage`; UI `AgentRow.tokens` and `AgentRow.usage_unknown`, plus `usage_source`. Reconcile existing `SubagentProgress.tokens` into snapshots by stable ID and generation.

- [ ] **Step 1 — Failing tests:** `agents_from_snapshots_displays_reported_tokens` (existing projection currently omits tokens); `unknown_usage_never_shows_zero`; `elapsed_continues_while_view_changed`; `usage_streaming_estimate_replaced_not_added_twice`.
- [ ] **Step 2 — Red:** `cargo test -p davinci-coding-agent agents_from_snapshots_displays_reported_tokens -- --nocapture` and agent usage tests.
- [ ] **Step 3 — Implement:** Expose canonical usage, tag `~` estimates, calculate elapsed with monotonic time until terminal status then freeze; show `unknown` if provider reports nothing. Do not double-count cache/write/read tokens.
- [ ] **Step 4 — Green:** Both package targeted suites; assertions across 250ms frames and completion.
- [ ] **Step 5 — Review/commit:** `fix(agents): project live usage and stable elapsed`.

### Task 4: Display real Git branch, worktree and changed-file ownership

**Files:** Modify `crates/davinci-agent/src/runtime/control.rs`, `src/runtime/registry.rs`, `crates/davinci-coding-agent/src/davinci_surfaces.rs`; Create `crates/davinci-agent/src/runtime/worker_workspace.rs`, test `crates/davinci-agent/tests/worker_workspace_parity.rs`.

**Interfaces:** `WorkerWorkspace { kind: Shared|Worktree|Unavailable, path: Option<PathBuf>, branch: Option<String>, head: Option<String>, dirty: Option<bool>, changed_paths: Vec<PathBuf> }`; `snapshot_workspace(record, workspace_root) -> WorkerWorkspace` read-only and repository-bounded.

- [ ] **Step 1 — Failing tests:** `isolated_worker_displays_verified_branch`, `shared_worker_has_no_fabricated_branch`, `deleted_worktree_displays_unavailable`, `worker_branch_updates_after_checkout`; fixtures create actual git worktrees, not mock branch strings.
- [ ] **Step 2 — Red:** `cargo test -p davinci-agent isolated_worker_displays_verified_branch -- --nocapture`.
- [ ] **Step 3 — Implement:** Use existing worktree lease metadata and a bounded read-only Git status/ref probe; no shell scripts, no auto-merge, preserve dirty lease state and restrict paths to active repository.
- [ ] **Step 4 — Green:** Worktree tests on Linux and Windows; limit polling and timeout expensive status checks.
- [ ] **Step 5 — Review/commit:** `feat(agents): expose verified worker worktree details`.

### Task 5: Build a bounded, safe, inspectable worker event journal

**Files:** Modify `crates/davinci-agent/src/runtime/worker_activity.rs`, `crates/davinci-agent/src/runtime/mod.rs`, `crates/davinci-agent/src/runtime/events.rs`; Test `crates/davinci-agent/tests/worker_activity_parity.rs`.

**Interfaces:** `WorkerActivityStore::after(...)`, `::summary(...)`, `::evict_terminal(...)`; defaults: recent 200 summaries per worker, max 2 KiB each, hard cap on total per-run activity; persisted optional digest uses existing journal version/cursor.

- [ ] **Step 1 — Failing tests:** `activity_ring_bounded_under_10000_events`, `hostile_terminal_output_remains_inert`, `worker_output_respects_existing_redaction`, `older_cursor_reports_gap_instead_of_inventing_history`.
- [ ] **Step 2 — Red:** `cargo test -p davinci-agent activity_ring_bounded_under_10000_events -- --nocapture`.
- [ ] **Step 3 — Implement:** Summarize permitted calls/results, sanitize ANSI/control payloads and paths, annotate dropped events and retrieve full details only under existing authorization.
- [ ] **Step 4 — Green:** 10k-event stress, sensitive-tool fixture, no hidden thinking exposed; run `cargo test -p davinci-agent worker_activity_parity`.
- [ ] **Step 5 — Review/commit:** `feat(agents): journal bounded visible worker activity`.

### Task 6: Add always-visible agent roster and read-only worker focus model

**Files:** Modify `crates/davinci-tui/src/davinci/model.rs`, `src/davinci/app.rs`, `src/davinci/views/agents.rs`; Create `crates/davinci-tui/src/davinci/views/agent_focus.rs`.

**Interfaces:** `ViewScope { Main, Worker(String) }`, `WorkerViewState { agent_id: String, scroll: usize, cursor: u64, draft: String }`, `Model::focus_agent(String)`, `Model::focus_main()`. Render roster at bottom on main and worker scopes; focus selection is UI-only.

- [ ] **Step 1 — Failing tests:** `worker_roster_visible_during_main_turn`, `worker_focus_preserves_main_transcript_and_draft`, `finished_worker_remains_readable`, `narrow_terminal_keeps_composer`.
- [ ] **Step 2 — Red:** `cargo test -p davinci-tui worker_roster_visible_during_main_turn -- --nocapture`.
- [ ] **Step 3 — Implement:** Reuse existing `/agents` row and transcript primitives; render selectable main/worker roster with per-worker scroll state and a breadcrumb/header; collapse on tiny displays without hiding user input.
- [ ] **Step 4 — Green:** TUI fixture test at widths 40, 80 and 160; no terminal resize crash.
- [ ] **Step 5 — Review/commit:** `feat(tui): add agent-focused transcript surface`.

### Task 7: Implement mouse click, keyboard selection and Esc-to-main without abort

**Files:** Modify `crates/davinci-tui/src/davinci/app.rs`, `src/davinci/model.rs`, `src/davinci/runtime.rs`, `crates/davinci-coding-agent/src/davinci_interactive.rs`; Test `crates/davinci-tui/tests/agent_focus_parity.rs`.

**Interfaces:** Add `Target::AgentFocus(String)`, `Target::MainFocus`, `Choice::FocusAgent { agent_id: String }`, `Choice::FocusMain`; route through existing hit-cache and message loop. No duplicate keyboard handlers outside core TUI.

- [ ] **Step 1 — Failing tests:** `esc_from_worker_returns_main_without_abort_or_queue_clear`, `click_main_returns_main`, `click_worker_opens_correct_identity_after_roster_reorder`, `click_and_keyboard_navigation_match`, `esc_main_retains_interrupt_behavior`.
- [ ] **Step 2 — Red:** `cargo test -p davinci-tui esc_from_worker_returns_main_without_abort_or_queue_clear -- --nocapture`.
- [ ] **Step 3 — Implement:** Intercept Esc at worker focus first; preserve old main Esc path. Add stable identity mouse target and keyboard row navigation, separate roster hit rows from composer/history clicks. Keep agent permissions and worker execution untouched.
- [ ] **Step 4 — Green:** Mouse+keyboard fixtures, resize hit-cache invalidation and focused-worker completion race.
- [ ] **Step 5 — Review/commit:** `feat(tui): navigate agents and return to main safely`.

### Task 8: Refresh worker focus live while parent turn and tools run

**Files:** Modify `crates/davinci-coding-agent/src/davinci_interactive.rs`, `src/davinci_surfaces.rs`, `crates/davinci-tui/src/davinci/model.rs`; Test `crates/davinci-coding-agent/src/davinci_interactive_tests.rs`.

**Interfaces:** `apply_worker_updates(model, events)` consumes sequence-indexed activity and registry deltas without requiring exclusive mutable access to live `Agent`. Use the existing event channel and `RemoteQueue` shared handle.

- [ ] **Step 1 — Failing tests:** `agent_focus_while_main_busy_keeps_streaming`, `worker_activity_updates_without_losing_scroll`, `worker_reorder_keeps_focus_by_agent_id`, `resize_during_busy_worker_does_not_block_main`.
- [ ] **Step 2 — Red:** `cargo test -p davinci-coding-agent --bin davinci agent_focus_while_main_busy_keeps_streaming -- --nocapture`.
- [ ] **Step 3 — Implement:** Consume normalized events on UI thread, coalesce frames at bounded rate, refresh timers on tick and keep queued input/pending permissions visible; do not make synchronous Git/status calls on render thread.
- [ ] **Step 4 — Green:** Run interactive tests with at least 200 events/sec for 30s, no lag/starvation; verify no changed main-turn ordering.
- [ ] **Step 5 — Review/commit:** `feat(tui): render live worker focus without blocking main`.

### Task 9: Give focused composer an explicit recipient and safe worker messaging

**Files:** Modify `crates/davinci-tui/src/davinci/model.rs`, `src/davinci/views/agent_focus.rs`, `crates/davinci-coding-agent/src/davinci_interactive.rs`; Test `crates/davinci-coding-agent/src/davinci_interactive_tests.rs`.

**Interfaces:** `ComposerRecipient::Main | Worker(String)`; `send_focused_message(recipient, text) -> Result<MessageReceipt, SendError>`; the worker path uses `runtime.mailbox.send_steer` with current generation; command routing remains in host, not worker mailbox.

- [ ] **Step 1 — Failing tests:** `worker_composer_shows_at_recipient`, `message_worker_does_not_steal_main_draft`, `slash_commands_remain_main_scoped`, `worker_message_has_queued_receipt`, `unavailable_worker_send_gives_actionable_error`.
- [ ] **Step 2 — Red:** `cargo test -p davinci-coding-agent worker_composer_shows_at_recipient -- --nocapture`.
- [ ] **Step 3 — Implement:** Prominent `Message @worker` and distinct drafts; Enter sends explicit user text to the selected worker, never interprets a worker message as a project authorization. Typed slash commands for app control route to main.
- [ ] **Step 4 — Green:** TUI and host tests; send while child running and idle; verify no permission bypass.
- [ ] **Step 5 — Review/commit:** `feat(agents): explicit focused worker messaging`.

### Task 10: Make follow-up/steer queue durable with delivery receipts

**Files:** Modify `crates/davinci-agent/src/queues.rs`, `src/turn.rs`, `crates/davinci-session/src/jsonl_repo.rs`, `crates/davinci-coding-agent/src/davinci_interactive.rs`; Create `crates/davinci-agent/tests/queued_instruction_recovery.rs`.

**Interfaces:** `InstructionReceipt { message_id, session_id, run_id, target, generation, kind, state }`, states `Queued | Delivered | Applied | Rejected | Unknown`; add journal entry types `queued_user_instruction` and `user_instruction_state` (validate backward-compatible serializer); `QueueJournal::append_accept()`, `::transition()`, `::recover_unapplied()`.

- [ ] **Step 1 — Failing tests:** `acknowledged_followup_survives_restart`, `acceptance_is_written_before_queued_ack`, `crash_between_delivery_and_apply_never_double_applies`, `main_steer_delivered_at_safe_boundary`, `different_session_never_consumes_queue`.
- [ ] **Step 2 — Red:** `cargo test -p davinci-agent acknowledged_followup_survives_restart -- --nocapture`.
- [ ] **Step 3 — Implement:** Journal accepted intent and state updates atomically using existing session append path, stable IDs, ordered FIFO replay. Treat an unprovable mid-tool application as `Unknown` requiring review, not reexecute blindly. Preserve original message provenance and queue images under existing session storage safety policy.
- [ ] **Step 4 — Green:** File interruption/recovery tests, image payload roundtrip, queue ordering, shutdown/switch tests; run `cargo test -p davinci-agent queued_instruction_recovery`.
- [ ] **Step 5 — Review/commit:** `feat(agents): durable acknowledged user instructions`.

### Task 11: Handle late worker messages and avoid phantom worker resurrection

**Files:** Modify `crates/davinci-agent/src/runtime/mailbox.rs`, `src/runtime/team.rs`, `src/runtime/tools_agent.rs`, `crates/davinci-coding-agent/src/davinci_interactive.rs`; Test `crates/davinci-agent/tests/worker_activity_parity.rs`.

**Interfaces:** `WorkerMessageOutcome::Queued | Delivered | Applied | Rejected { reason }` from existing `SteeringReceipt`; terminal state mismatch maps to `WorkerFinished` plus optional user-initiated `RedirectToMain` action (never automatic).

- [ ] **Step 1 — Failing tests:** `late_message_to_completed_one_shot_is_rejected_once`, `idle_teammate_wakes_and_applies_one_message`, `queued_then_terminal_worker_yields_rejected_receipt`, `repeated_message_id_is_idempotent`, `failed_wake_reports_reason`.
- [ ] **Step 2 — Red:** `cargo test -p davinci-agent late_message_to_completed_one_shot_is_rejected_once -- --nocapture`.
- [ ] **Step 3 — Implement:** Reuse mailbox receipt lifecycle and generation revision checks. UI distinguishes `queued` vs `applied` and offers send-to-main follow-up; never creates a new worker without explicit user instruction.
- [ ] **Step 4 — Green:** Race tests with barrier synchronization across finish/send/drain; validate receipts in TUI and RPC.
- [ ] **Step 5 — Review/commit:** `fix(agents): resolve late worker steering transparently`.

### Task 12: Unify main coordination, teams, workflows and RPC/SDK behavior

**Files:** Modify `crates/davinci-agent/src/runtime/mod.rs`, `src/runtime/workflow/executor.rs`, `crates/davinci-coding-agent/src/rpc.rs`, `src/sdk.rs`, `src/davinci_surfaces.rs`; Test `crates/davinci-coding-agent/src/main_tests.rs`, `crates/davinci-evals/tests/agent_orchestration.rs`.

**Interfaces:** `AgentViewSnapshot` and versioned `agent_activity_update` RPC event expose identical IDs/status/usage/workspace across clients; `agent_focus` is client-local, never broadcast as authorization. Main autowake consumes existing per-run mailbox state when idle.

- [ ] **Step 1 — Failing tests:** `rpc_and_tui_report_same_worker_state`, `main_receives_completion_while_worker_view_selected`, `team_off_still_supports_background_workers`, `workflow_worker_focus_scopes_to_session`, `session_switch_cancels_old_roster_visibility`.
- [ ] **Step 2 — Red:** `cargo test -p davinci-coding-agent rpc_and_tui_report_same_worker_state -- --nocapture`.
- [ ] **Step 3 — Implement:** Share canonical snapshots/receipts, adapt all worker modes to the unified activity DTO; respect teams/workflows feature flags and session binding; add schema versioning for RPC, no new scheduler.
- [ ] **Step 4 — Green:** Agent orchestration evals, RPC tests, manual mixed-mode matrix; ensure end-of-turn main coordination is not blocked by worker-focus UI.
- [ ] **Step 5 — Review/commit:** `feat(agents): converge UI and RPC worker snapshots`.

### Task 13: Redaction, cancellation, race and performance hardening

**Files:** Modify `crates/davinci-agent/src/runtime/worker_activity.rs`, `crates/davinci-tui/src/davinci/views/agent_focus.rs`, `crates/davinci-coding-agent/src/davinci_interactive.rs`; Test `crates/davinci-tui/tests/agent_focus_parity.rs`, `crates/davinci-agent/tests/worker_activity_parity.rs`.

**Interfaces:** No new public API; policy binds permissions and output sanitization to the existing host.

- [ ] **Step 1 — Failing tests:** `hostile_tool_output_cannot_inject_ansi_or_fake_actions`, `other_run_worker_logs_not_visible`, `esc_worker_never_sets_abort_flag`, `terminal_resize_click_targets_not_stale`, `ten_thousand_events_have_bounded_memory`, `hidden_reasoning_never_rendered`.
- [ ] **Step 2 — Red:** Target each case on baseline/preceding tasks; retain reproducible red-green trace.
- [ ] **Step 3 — Implement:** Bound queues/rings, sanitize, gate source reads and diff by current session permission, handle disconnected worker or missing Git data without crashing; preserve terminal restore on Ctrl+C/quit.
- [ ] **Step 4 — Green:** `cargo test -p davinci-tui agent_focus_parity`; `cargo test -p davinci-agent worker_activity_parity`; `cargo fmt --all -- --check`, targeted Clippy.
- [ ] **Step 5 — Review/commit:** `test(agents): harden live focus and queue race boundaries`.

### Task 14: Real terminal acceptance, docs, feature flag and rollback

**Files:** Modify `docs/agent-teams.md`, `docs/runtime-orchestration.md`, `crates/davinci-coding-agent/src/settings.rs`, `crates/davinci-coding-agent/src/davinci_interactive_tests.rs`; Create `crates/davinci-tui/tests/agent_focus_terminal.rs`; add CI fixture using existing Windows and Linux terminal harnesses.

**Interfaces:** Introduce `agentFocusUi` rollout switch default off until all gates pass; default on after measured production-like acceptance. Keep legacy `/agents` panel and ordinary composer as rollback path, without deleting receipt history.

- [ ] **Step 1 — Failing tests:** `conpty_click_worker_stream_main_return`, `linux_pty_escape_does_not_cancel_child`, `refresh_and_resize_preserve_selection`, `feature_off_retains_legacy_agent_panel`, `full_stack_worker_finished_late_message_returns_receipt`.
- [ ] **Step 2 — Red:** Document current behavior using golden recorded key/click/event playback on both platforms; capture exactly which acceptance stages remain unverified.
- [ ] **Step 3 — Implement:** Wire feature flag/config, docs, accessibility labels and status hints; support mouse-disabled terminals using keyboard selection; preserve main/session scope on revert.
- [ ] **Step 4 — Verify:** `cargo test -p davinci-agent`, `cargo test -p davinci-tui`, `cargo test -p davinci-coding-agent --bin davinci`, `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings` (report pre-existing failures), plus Windows ConPTY and Linux PTY e2e. Require one actual multi-worker test under a representative provider route; fixture success alone is insufficient. Verify no hidden raw data in logs.
- [ ] **Step 5 — Review/commit:** `feat(agents): ship Claude-style live agent navigation behind verified gate`; acquire independent whole-branch review, reconcile failures, then open PR with screenshot/video receipts and rollback instructions. Do not claim parity while acceptance gates are pending.

## Release acceptance checklist

- [ ] Three live workers remain visible while main runs; branch/worktree, status, time and usage are accurate and regularly refreshed.
- [ ] Mouse selects agent; Enter/keyboard selects agent; Esc from worker and click main return to main without interrupting any task.
- [ ] Worker detail shows its own chronological activity and permitted output, not another agent's transcript or a fabricated branch.
- [ ] Main and worker composer indicate recipient and do not lose typed drafts on focus change.
- [ ] Queued follow-ups and direct worker messages are acknowledged and durably recoverable, with honest `queued`, `delivered`, `applied`, `rejected` and `unknown` states.
- [ ] One-shot, background, teammate, workflow workers display correctly; disabled experimental team/workflow flags stay respected.
- [ ] Windows ConPTY, Linux PTY, resize, mouse disabled, dropped events, terminal closure and restart tests are green.
- [ ] No hidden model reasoning, secrets, cross-session logs or new authority escalations leak into the UI.
- [ ] Live end-to-end evidence recorded against the release candidate commit; rollout switch and fallback path tested.

## Execution handoff

**Recommended:** Superpowers subagent-driven development, one focused task and independent reviewer at a time, with a branch-wide security/TUI review before PR. The feature crosses concurrent queues, a terminal frontend and session persistence; unreviewed parallel edits in these shared files are risky. When the user approves the spec and plan, create an isolated worktree and execute Tasks 1–14 in order; branch and PR require user authorization and all release checks. This file is a plan only.
