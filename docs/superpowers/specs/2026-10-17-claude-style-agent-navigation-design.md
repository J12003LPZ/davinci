# DaVinci — Claude Code-style Agent Focus and Live Orchestration Design

**Status:** Proposed architecture for user review; no product implementation, PR or live terminal test performed.  
**Prepared:** 2026-10-17 (reference screenshot supplied by the user, timestamped 2026-10-18).  
**Repository:** https://github.com/J12003LPZ/davinci  
**Reference revision inspected:** `c1973e80e81744a1189c32b6e3368411fc1b6450` for branch ancestry; implementation details were reviewed in the current `main` code and its prior commits. Reconfirm on the implementation worktree.  
**Companion plan:** [`../plans/06-claude-style-agent-orchestration-ui-parity.md`](../plans/06-claude-style-agent-orchestration-ui-parity.md).

## 1. Intent and definition of parity

User wants the terminal UX pictured in the supplied Claude Code reference: a main agent can orchestrate multiple concurrent subagents; a persistent bottom roster lists main and workers, each with status, elapsed wall time, and token usage; the user can click or select a worker to view *its* live transcript and tool activity, see its real Git worktree and branch, queue a message to main or worker while work is running, and click **main** or press **Esc** to return to the main transcript without cancelling the background work. A late instruction must either reach a live worker safely or be returned with an honest terminal-state notification. Main receives worker results and may coordinate subsequent work. Focus changes are presentation-only and never mutate the running task, permissions or model context by themselves.

This is **functional and interaction parity for the screenshot workflow**, not an untestable promise to clone the exact internals, styling, proprietary data, or every future Claude Code feature. Capture golden terminal frames and keyboard/mouse behavior, not a fragile pixel-identical font comparison.

## 2. Current features and observed gaps

| Existing source | What works | Missing or verify |
|---|---|---|
| `crates/davinci-agent/src/subagent_progress.rs` | `SubagentProgress` has agent ID, label, mode, recent calls (last eight), tools, tokens, elapsed, status; event sink forwards updates | Recent text alone is not a full scrollable worker timeline; input/cached/estimated token distinction and live elapsed need clarification |
| `crates/davinci-agent/src/runtime/registry.rs`, `runtime/control.rs` | Registry/worker snapshots and worktree owner state | `AgentRecord` has worktree path but not explicit branch/ref; supervisor snapshots can report usage unknown |
| `crates/davinci-coding-agent/src/davinci_surfaces.rs` | Converts worker snapshots into `AgentRow` | Source conversion sets state, elapsed, owner paths and tool count, but **does not populate `AgentRow.tokens`** |
| `crates/davinci-tui/src/davinci/views/agents.rs` | `/agents` sheet lists workers, status, elapsed, owned paths and token field | Worker detail is a text inspection; no independently navigable worker transcript or branch diff |
| `crates/davinci-tui/src/davinci/app.rs` | Has keyboard selection; `Enter` maps to `Choice::AgentAction{inspect}` | Clickable roster rows and Esc-to-main focus need explicit routing |
| `crates/davinci-coding-agent/src/davinci_interactive.rs` | Normal Enter queues user input while busy, and `run_turns` dispatches afterward; `/agents msg` targets worker | UI queue currently stored in memory; no worker-focused input composer; view switching while busy needs integrated event-loop support |
| `crates/davinci-agent/src/queues.rs`, `turn.rs` | Remote steer/follow-up queues injected at safe boundaries | Queue state and intent classification must be journaled and acknowledged across process interruption |
| `crates/davinci-agent/src/runtime/mailbox.rs`, `runtime/team.rs` | Mailbox receipts and teammate wakeup; idle teammate persists within session | One-shot worker in terminal state must never be silently resurrected; worker readiness and client receipt should be visible |
| `docs/agent-teams.md` | One-shot, background, teammate, workflow modes and `/agents` controls | Native `/resume` cannot restore live teammates, so recovery must disclose terminal state and saved observations |

## 3. User-facing interactions

**Main view:** after workers launch, a compact roster stays at the bottom of the TUI with `main` and the currently scoped workers. Each row shows indicator, name, current one-line activity, elapsed `m:ss`/`h:mm:ss`, tokens (or `unknown`), and a worktree indicator. A footer shows counts of workers and other jobs; no required slash command to open the roster. A minimal-height terminal may collapse the roster behind a one-row summary; it never hides the composer.

**Worker focus:** click a roster row, or choose it using keyboard selection, to focus the *read-only live agent transcript*: its action timeline, labeled tool calls and permitted result snippets, elapsed timer, token-usage metadata, status, task association, working directory, real Git worktree path and verified branch ref. A worker with no isolated branch is labeled `shared workspace — no separate branch`, **never** assigned a fictitious branch. A completed worker remains inspectable. Main transcript and queued inputs are preserved in memory and persist by the normal journal.

**Navigation:** `Esc` while worker-focused returns to **main** and does not cancel anything, clear queue, close session, or interrupt a worker. Clicking main does the same. `Esc` while already on main retains DaVinci's existing main-turn interruption semantics. `Enter` on focused roster row opens its transcript. `Up/Down` navigate list when roster has keyboard focus. Mouse click targets are stable by AgentId, not transient row index. `Tab`/mouse events must not conflict with editor autocomplete or permission prompts. Focused view has a visible `Viewing: <worker>` header and `Message @worker` composer target; *changing focus alone does not redirect an already drafted main-agent message.*

**Messaging:** pressing Enter while main is busy queues a **follow-up** for main. An explicit `Steer now` action or `/steer <message>` uses the existing safe-boundary queue, not a mid-tool mutation. Worker-focused composer can send a message to the selected worker using the existing mailbox; its target is shown. Queue confirmations must say `queued`, not `applied`, and status transitions are `queued → delivered → applied`, or `rejected` with reason. Late instructions to completed one-shot agents show `already finished; send follow-up to main / launch new worker`; persistent teammates may be woken when idle. Commands intended for the main (`/agents`, `/tasks`, `/config`, etc.) must retain their command routing and never become unintended worker prompts.

**While busy:** viewing a worker must be possible during an active model call. The UI event loop continues processing tool-progress and session input without mutable aliasing of the agent. Agent listing refreshes without losing selection, scrolling or a typed draft; roster contents are updated by stable ID and bounded event replay.

**Worktree detail:** show repository-relative worktree path, branch name if verified, short commit hash, dirty/clean/unknown state and optional reviewable changed-files view. File diffs are *read-only* and do not accidentally merge/apply workers' changes. Worktree ownership and permission rules are inherited from the existing runtime. A shared-folder agent must not pretend to have its own branch. Missing/deleted worktree yields `unavailable` without crash.

## 4. Architecture and contracts

### 4.1 Host-side telemetry and observer

A typed `WorkerActivity` event stream is associated with `(session_id, run_id, agent_id, generation)` and monotonically increasing per-run sequence. Events include lifecycle, tool start/end, safe summarized command/output, latest activity, message receipts, source of token usage and bounded worktree metadata. Reuse existing `RuntimeEventEnvelope`, `SubagentProgress`, `RuntimeRegistry`, `AgentMailbox` and `WorkerController` rather than introducing a parallel scheduler. The host is authoritative for runtime and authorization state; untrusted child output is *data*, never a CLI command or trusted UI control.

Recommended DTOs: `WorkerActivityEvent { sequence, agent_id, generation, timestamp_ms, kind, summary, outcome }`, `WorkerUsage { reported_total, estimated_visible_total, source }`, `WorkerWorkspace { mode, worktree_path, branch, head, dirty }`, and `WorkerViewSnapshot { identity, latest_sequence, state, usage, elapsed_ms, recent_activity, workspace }`. Exact Rust signatures are pinned in the implementation plan after repository-version revalidation.

Keep a bounded in-memory ring (e.g. latest 200 activity summaries per worker, max 2 KiB per summary) and an opt-in/authorized persisted activity digest in the existing session journal with replay versioning. Preserve provider-native raw responses and full tool output only where the existing session/permission model already permits them. Never expose hidden chain-of-thought, unsanitized secrets or executable escape sequences. File content and command output go through existing sanitization/redaction and the same permission ceiling as the active session.

### 4.2 TUI presentation

Use `ViewScope::Main | ViewScope::Worker(String)` as TUI-only state instead of making the TUI depend on davinci-agent IDs. An anchored, clickable roster is rendered across main and worker scopes. `WorkerViewState` has independent scroll position and stable activity cursor. `Choice::FocusAgent { agent_id }`, `Choice::FocusMain` (or equivalent typed UI actions) are resolved by the host and checked against the current run. `Esc` first pops worker focus; only a second `Esc` from main triggers the ordinary main behavior. Old click hit targets are invalidated on resize or roster reordering.

### 4.3 Queue persistence and safe recovery

Accepted user follow-up/steer messages receive stable IDs, target and generation, intended delivery kind, arrival order, status, and a session-journal receipt. Persist **before confirming queued**, in an append-only journal form that can be replayed without duplication; only the currently selected session can consume these entries. Delivery/application/rejection are distinct journaled transitions. On crash, reconstruct undelivered messages and avoid reapplying applied ones; unconfirmed in-flight intents are surfaced as `unknown; review before retry`. On session switch, do not transfer queued messages to unrelated runs. If a worker stops before application, emit `rejected: worker finished`, preserve the message for review, and optionally offer to send a new follow-up to main.

### 4.4 Concurrency, terminal discipline and authority

The TUI event thread must be responsive while the model runs on a worker thread. Interactions enqueue typed control messages through existing thread-safe queues/mailboxes. Focus never changes task ownership or prompt destination without an explicit send. Session quit/interrupt returns terminal alternate-screen state safely; terminal-control characters and child output are sanitized. User messages carry genuine user provenance; synthetic progress or worker messages do not become `UserDecision` when compressed. This integrates with the context-integrity workstream rather than bypassing it.

## 5. Delivery options and recommendation

**A — extend existing agent panel and queues (recommended).** Reuse `agents.rs`, TUI event loop, `SubagentProgress`, registry, mailbox and queue code; add a worker-focused transcript, persisted receipt and branch metadata. Least duplication and best compatibility with existing modes.

**B — a separate fullscreen agent dashboard.** Faster visually but forks transcript/keyboard state, risks inconsistent permissions and loses the screenshot's always-visible composer. Not recommended.

**C — terminal multiplexer / separate TTY per worker.** Deep worker visibility but complicated cross-platform PTY lifetimes and unclear authority boundaries; not needed for functional parity. Not recommended.

## 6. Failure handling and security

- Unknown usage is `unknown`, never fabricated zero. Show `~` on estimates and indicate cache/read/write accounting where supported.
- A worktree missing or shared cannot cause worker focus to crash. Branch is verified from actual worktree identity and can change during run.
- A queued message accepted just before worker completion ends with a visible terminal-state receipt, never silent loss or unsupported auto-resurrection.
- On crash/restart, restore **observations and pending intents**, not running processes; show worker `interrupted`/`disconnected` and require explicit relaunch or safe resume if supported.
- Hidden text, credentials, raw tool secrets, shell escape codes, source from other runs and abandoned sessions cannot leak into worker view.
- Worker/host permissions, project trust, sandbox and worktree leases remain unchanged when switching views. A worker cannot approve a tool or project trust on the lead's behalf.
- Esc worker view never cancels work. Esc main preserves existing abort behavior. Session switch clears focus and quarantines pending messages from old session.

## 7. Acceptance criteria

1. Spawn three mixed one-shot/background/teammate workers under a main task; all appear live in a persistent roster, with changing elapsed and correctly labeled token usage.
2. Select worker by mouse click or keyboard; inspect chronological tool summaries and permitted output plus exact branch/worktree. Click main or press Esc returns immediately while task continues.
3. In worker view, queue a message; UI shows stable ID/`queued` and later `delivered`, `applied` or `rejected`. Worker terminal-state conflict surfaces an actionable follow-up to main.
4. While main is busy, type three follow-ups, verify FIFO and distinct receipts; explicit live steer applies at a safe provider boundary. No prompts are sent into an in-flight tool call.
5. Crash and restart after acknowledgement, before delivery and after delivery: no duplicate application, no lost accepted prompts, no worker resurrection; UI shows recovery/unknown state accurately.
6. Screen resize, tiny terminal, Unicode, 200 progress events/sec, mouse hits, clicking main, Esc navigation, permission dialogs and input composition work without blocking/spoofing/leaking.
7. Shared-workspace worker shows no fabricated Git branch. Worktree worker shows verified branch, dirty state and diff review without auto-merge.
8. Windows native TUI/ConPTY and Linux PTY interactive tests pass. Background and one-shot worker execution parity holds when `agentTeams=false`; team features remain opt-in.
9. RPC/SDK queue and worker status reflect the same state and sequence as TUI. End-to-end tests must include a real provider or controlled full-stack stream, not only static rendering fixtures.

## 8. Out of scope for first parity release

- Cloning every Claude Code feature or visual theme; compatibility contract is the screenshot workflow and listed behavior.
- Restoring live teammates or shell processes after OS reboot. Recover recorded state and user instructions, then offer explicit relaunch.
- Auto-merging branches, auto-accepting edits or escalating permissions.
- A new worker execution engine, mandatory agent teams, mandatory Docker/Ollama/Qdrant, or reworking the separate `/design` artifact studio.

## 9. Review gate

This is a proposed written spec for review. Confirm the focus, queue and recovery semantics before authorizing product code. The plan is attached for visibility but does not authorize implementation. No repository commit or PR has been made.
