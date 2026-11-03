# DaVinci: Claude Code 2.1.316 Terminal Parity Design Amendment

**Status:** Proposed design amendment for review; no implementation authorized or performed by this document.
**Amended 2026-11-01:** The implementation plan's Decisions section overrides this document where they conflict: live reply streaming stays, exact parity excludes longer text, Claude-only surfaces are dropped, and work ships behind the `tui.reference` setting.
**Reference:** `C:\Users\sergi\Desktop\lean-claude-ui`, Claude Code native Windows x64 2.1.316 (binary SHA-256 `e896a9d8da519353a49bf69ca4a695ba7e09067752687c675b3969041e632d52`).
**Previous design:** `docs/superpowers/specs/2026-09-20-claude-code-terminal-parity-design.md`.
**Implementation plan:** `docs/superpowers/plans/2026-11-01-claude-code-2.1.316-tui-parity-implementation.md`.
**Repository inspected:** `C:\Users\sergi\Desktop\davinci`, branch `fix/droid-main-stabilization`; plan is not a claim about the installed application.

## 1. Intent and precedence

Reproduce the observable **terminal user experience** of the pinned Claude Code CLI 2.1.316, including normal chat, startup, classic/fullscreen behavior, menus, tool blocks, input editor, terminal modes, permission interaction, streaming lifecycle, background agents, scroll/copy/mouse and accessibility. Aim at cell-level visual and action-level behavior parity, not a superficial style approximation. Use original independent Rust implementations and recorded public/observable behaviors; do not transplant proprietary source, binary assets, Clawd artwork, or Anthropic brand identity.

This amendment supersedes *conflicting reference constants and behavior* in older 2.1.278/2.1.281/2.1.301 plans but **does not cancel** approved DaVinci-specific graph/recovery work. Historical plans remain available for provenance, not as authority for 2.1.316 styling. Evidence from the pinned version controls version-sensitive behavior. When evidence says PARTIAL/SOURCE_SUPPORTED/UNVERIFIED/BLOCKED/EXPECTED_FAIL, never silently promote to parity.

**Allowed deliberate differences:** DaVinci name/mark, real provider/model names, actual subscription/account/usage data, supported settings and actions, DaVinci-specific graph/context/cache/memory/safety/extension views. These should use the same structural UI grammar and measured spacing; fabricated Claude features, costs or permissions are prohibited. A literal trademark/asset-identical clone is not the goal.

## 2. Sources and fidelity

Reference root: `C:\Users\sergi\Desktop\lean-claude-ui`.
Authoritative metadata: `artifacts/data/baseline.json`, `EVIDENCE_MATRIX.json`, `TEST_MATRIX.md`, `GAP_ANALYSIS.md` and `REMAINING_UNCERTAINTIES.md`.
Visual truth: `artifacts/frames/reference/index.json` and 99 versioned `*.json` styled-cell captures (schema `claude-tui-styled-capture/2`), their matching PNG/SVG and terminal recordings.
Behavior truth: `LIVE_STREAMING_SPEC.md`, `PERMISSION_DIALOG_SPEC.md`, `AGENTS_TUI_SPEC.md`, `FULLSCREEN_INTERACTION_SPEC.md`, `INPUT_EDGE_CASES.md`, `STATE_MACHINES.md`, `PARITY_CHECKLIST.md`.
Baseline: 77 tracked features = 65 VERIFIED, 5 PARTIAL, 3 SOURCE_SUPPORTED, 2 EXPECTED_FAIL, 1 UNVERIFIED, 1 BLOCKED. 99 reference test cases = 97 PASS, 2 EXPECTED_FAIL. These are **reference tests, not DaVinci tests**.

Reference profiles must carry version, renderer, dimensions, terminal type/version, color mode, semantic theme, runtime data fixture, ordered input events and timestamps. Windows Terminal fullscreen at comparable dimensions is primary. Classic and ConPTY are distinct profiles. Do not assert macOS/Linux/tmux/VS Code parity without their own evidence.

## 3. Approach evaluation

- **A — incremental modernization of DaVinci's live Ratatui/Crossterm shell (recommended).** Adapt the active `davinci_interactive::run` → `davinci_tui::davinci::runtime::Session` route; reuse editor, transcript, theme, state, host adapters and existing tests. Introduce a measured reference profile and test adapter. Lowest risk to real providers, permissions, graph and storage.
- **B — pivot the default to the legacy pi-style `InteractiveTui`.** It already supports regular/fullscreen mode switching, but changing the live entrypoint would require migrating DaVinci's feature-rich shells, decisions and agent state. Larger host/regression risk; use reusable primitives from it only where justified.
- **C — import/rewrite a new renderer from scratch.** Highest cost and security/portability risk, needlessly duplicates the two existing terminal engines. Rejected.

**Selected as an implementation-planning default, subject to review:** A. Do not cut over at runtime until golden-reference comparisons and safety gates show it is correct. Existing `--legacy-tui` path remains a rollback option.

## 4. Architecture and ownership

1. **Reference adaptation** (offline, independent): manifest + reference loader + canonical style/cell comparison + trace adapter. Golden files are out-of-repo where necessary; versioned sanitized, license-safe fixtures only in DaVinci.
2. **Terminal engine** (`davinci-tui/src/davinci/runtime.rs` and `term.rs`): capability negotiation, measured update/resize performance, alt/main-screen behavior, cell frame deltas, cursor/selection/mouse and reliable teardown. Do not assume Ratatui's full-frame diff has the same wire behavior as the reference.
3. **Pure presentation** (`app.rs`, `theme.rs`, `ui.rs`, `views/*`): reference layout for header/transcript/composer/footer, shared panels, modal overlay, typed cells, response blocks and animations; no tool execution in views.
4. **Input/focus state** (`model.rs`, `app.rs`, shared `editor.rs`): single event arbitration by active context; composer's history/paste/chords/suggestions/attachments; no stale acceptance after a focus change.
5. **Host event bridge** (`crates/davinci-coding-agent/src/davinci_interactive.rs` and submodules): translate existing authenticated OpenAI/agent/permission/graph events into faithful display-state transitions. Keep prompt queues and task IDs scoped and durable.
6. **DaVinci-only surfaces:** restyle extensions, graphs, memory, budgets and security views with the same chrome without manufacturing Claude-equivalent commands or outputs.

Existing default live route: `main.rs::run_interactive` selects `davinci_interactive::run` unless `--legacy-tui` or `PI_DAVINCI=0`, and it owns a Ratatui alternate-screen `Session`. The legacy route uses `InteractiveTui` and `ChromePanes`. Any renderer unification must have explicit fallback proof.

## 5. Behavioral contract and invariants

- Same terminal geometry and styled-cell appearance for equivalent, stable reference fixtures after separately normalizing product/model/account values; compare characters, widths, RGB/ANSI fg/bg, all modifiers, cursor and enabled terminal modes.
- State transitions are testable, not merely displayed: modal focus, search, model picker, permission modes, composer actions, stash, paste, queue/send-now, interrupt, scroll anchors, tool collapse/expand, agents pin/rename/delete, and classic/fullscreen switching.
- Mock-SSE reference 2.1.316 **does not paint incremental assistant body text** during normal streams: spinner/token counter update; body commits at `message_stop`; partial body flushes on interrupt/send-now. This is version-scoped and must not be generalized to future releases.
- Approval actions preserve actual DaVinci policy, trust and expiration. A styled dialog must never approve another request after selection, timeout, mode change or reconnect. Deny/cancel must not write.
- Background sessions remain running when the dashboard closes; stopped, failed and completed records remain distinguishable; session identity persists across renames, pins and filters.
- Interactions never silently change provider, account, model/effort, credit usage, permissions, sandbox, hooks, agent controls or tool scopes.
- Untrusted output and OSC/ANSI controls are sanitized; render and clipboard both preserve privacy and avoid injecting terminal control sequences.
- Use only real measured data for usage/context/plan and agents. Unknown is not zero; one UI interaction never spends extra Codex allowance unless it is itself the authorized model/tool operation.
- Preserve existing preference files and unknown keys. Explicit custom themes and `--legacy-tui` remain opt-in rollback paths.

## 6. Measurable acceptance

**P0:** All comparable *VERIFIED* reference actions are mapped to tests, all baseline P0 tests pass, no unauthorized action or mode change, zero unexplained styled-cell differences in masked static reference fixtures on primary Windows Terminal profile, no terminal-mode leaks on quit/panic/cancel, no lost input under resize/interrupt, and stable agent/task identity.

**P1:** Complete the high-value remaining user-facing surfaces, classic/fullscreen switching, settings/help/pickers, backgrounds, Unicode/paste, reduced color/no color, accessibility. All known reference limitations must be listed as exceptions, not hidden.

**Performance targets** (targets, not observed DaVinci results): cold first prompt <2s, warm <500ms, input-to-paint p95 <16ms, resize-to-paint p95 <50ms, idle RSS <150MB, idle CPU <1%, scroll paint p95 <16ms; compare equivalent test machines/terminals and disclose misses. Avoid sacrificing correctness or token efficiency for a cosmetic benchmark.

**Release:** first gated opt-in, then default only after deterministic headless and human Windows Terminal checks. No independent full-platform 1:1 claim without terminal-specific testing.

## 7. Remaining uncertainties and non-goals

Still open in the reference baseline: image drag/drop, voice recording, reduced-motion suppression, cursor shape, VS Code integration, boot-strike banner, full keybinding sweep, Clawd animation, OSC 8 hover/links, macOS/Linux/tmux/Ghostty and ConPTY Ctrl+L/Shift+Up. Do not block the Windows-first core implementation on unrelated polish; preserve them as explicit follow-up gates.

Non-goals: cloning Claude's proprietary model/backend, altering Codex authentication/subscription routing, introducing mandatory API-key billing, changing DaVinci's reasoning/graph execution architecture just to imitate labels, copying copyrighted source/assets, or adding a permanent dashboard over the reference chat.

## 8. Review decisions and handoff

The plan adopts **Approach A** as the least invasive default. If the reviewer wants literal Anthropic branding/assets or a full legacy-runtime replacement, that changes both legality/architecture and requires a separate decision before implementation. Until review, this is a proposed design amendment and implementation plan only. No product code, installed binary, model spending or user settings are changed.
