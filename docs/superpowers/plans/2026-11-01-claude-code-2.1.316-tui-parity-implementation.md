# Claude Code 2.1.316 → DaVinci TUI Implementation Plan

**Goal:** Match the observable Claude Code CLI 2.1.316 Windows terminal experience in DaVinci: exact layout, colours, terminal behaviour and interactions, with DaVinci's own name, mark, wording and real provider data.

**Status (2026-11-02):** P00–P20 are implemented and committed. P13/P13b verified the model picker, Help tabs, config search/tabs, real usage values, the rewind selector, the 20-column context grid with `/context all`, and the read-only permissions tabs/search; their 18 panel frames remain PARTIAL because the reference frames carry their own scrolled transcript, content and inline panel placement. P14/P14b verified page/wheel scroll, the jump hint, click-to-place caret, click-to-expand (a second click does not collapse), drag selection across a resize, and the detailed transcript viewer live; the reference goldens need its resumed 12-turn fixture and stay PARTIAL. P15 verified the dashboard's groups, counters, empty state, rename, pin, find and two-step delete, with off-mode agents unchanged; `claude agents` as a separate CLI entry is out of scope. P16's spike sized a classic renderer at 1,500–2,500 lines and, as the plan allows, defers `/tui` to its own plan; fullscreen ships. P17 verified `--screen-reader` and the persisted `reduceMotion` toggle; the two frames differ in identity/notice/anchor and stay PARTIAL. P18's 30 screens and 4 overlays are recorded in `p18-views-ledger.md`. P19's `run_suite.py` and report ran the full Python suite; P20's gates all pass (first prompt 61 ms, input 1.3 ms internal, resize 20.5 ms, PageDown 10.2 ms, idle CPU 0.0%, RSS 38.6 MB). Remaining: the human Windows Terminal check and the user's approval before flipping the default; warm-start is not separated from cold. No DaVinci parity is claimed beyond the recorded per-row states.

**Replaces** the earlier version of this plan. Where `docs/superpowers/specs/2026-11-01-claude-code-2.1.316-tui-parity-design.md` conflicts with the decisions below, this plan wins.

**Architecture:** Adapt the active shell (`davinci_interactive::run` → `davinci::runtime::Session`) behind a setting. Build the comparison tools before changing any presentation code.

**Tech stack:** Rust 1.89, Ratatui 0.30.2, Crossterm 0.29, existing DaVinci crates, Python and PowerShell for capture, `@xterm/headless` 6.0 for styled-cell replay.

**Paths**

| What | Where |
|---|---|
| Repository | `C:\Users\sergi\Desktop\davinci` |
| Execution worktree | `C:\Users\sergi\Desktop\davinci-cc2316-parity`, branch `feat/claude-2316-tui-parity` |
| Pinned reference | `C:\Users\sergi\Desktop\lean-claude-ui`, v2.1.316 native Windows x64, binary SHA-256 `e896a9d8da519353a49bf69ca4a695ba7e09067752687c675b3969041e632d52` |

**Evidence baseline:** 77 tracked features: 65 VERIFIED, 5 PARTIAL, 3 SOURCE_SUPPORTED, 2 EXPECTED_FAIL, 1 UNVERIFIED, 1 BLOCKED. 99 reference tests: 97 PASS, 2 EXPECTED_FAIL. 99 styled frames. These test Claude Code, not DaVinci.

**Who does the work:** Opus plans and reviews and never writes code. One Sonnet 5.5 (medium) writes code. GPT 6 Luna High researches. GPT 6.1 Sol High consults.

---

## Decisions

These were taken on 2026-11-01 from the plan review. Each can be reversed, and reversing one changes only the listed tasks.

| # | Decision | Why | If reversed |
|---|---|---|---|
| D1 | **Keep live reply streaming.** Spinner, token counter, commit at completion and flush on interrupt follow the reference, but the reply body still paints as it arrives. | The reference hides the body because of one flag hard-disabled in 2.1.316 (`mrn(){return!0}`). Long replies with no visible text feel worse. | P11 hides the body until completion and removes the exceptions in the ledger. |
| D2 | **1:1 means layout, colour, glyphs and behaviour, plus short functional labels.** Longer copy (tips, help prose, onboarding, lessons) is DaVinci's own and is compared by structure only. | Exact prose would mean copying Claude's text. | The Structure tier below becomes Exact for the affected goldens. Get legal advice first. |
| D3 | **Drop Claude-only surfaces:** powerups, feedback survey, custom API-key consent, voice. | They have no DaVinci purpose. The survey would also imply sending data. | Add a task for each. |
| D4 | **Everything ships behind an off-by-default setting** (`tui.reference`, overridable with `PI_DAVINCI_REFERENCE=1`, read once at `Session::open`). | `--legacy-tui` opens the older chrome, not today's shell, so it can't be the rollback for this work. | Keep the branch unmerged until every gate passes. |
| D5 | **Keep the worktree.** | The main checkout is shared with other sessions. This is an exception to the usual no-worktree rule. | Work in the main checkout. |
| D6 | **Drop one-off release announcements, and let the conversation close the gap.** The notice about a Claude Code default change is not drawn, and the conversation starts right under the header. | It announces a Claude product change and has no DaVinci purpose, and a permanent blank gap would be a visible defect. A `compact` rule, declared per golden with exact rows, removes the notice and its separator from the expected frame only. Vacated rows must be blank, or match a stated expectation. A scrolled golden with no such expectation stays a recorded limitation. | Reserve blank rows, or write a DaVinci notice for the same rows. |
| D7 | **No external Claude links in DaVinci's own copy.** The reference's underlined security link is not drawn. | The link goes to Claude's documentation. Those cells must be blank, checked by a `dropped` rule. | Link to a DaVinci page, but none is documented. |

**D4 exception:** the `/config` row for `tui.reference` itself appears with the setting off, because people need it to turn the setting on. `PI_DAVINCI_REFERENCE=1` overrides a stored `false`. Every other off-mode frame and terminal byte stays unchanged.
**Dropped goldens (4):** `powerup_modes__01_modes_lesson`, `powerup_modes__02_next_lesson`, `mock_stream2__00_key_dialog`, `voice_probe__02_voice`.

**Streaming exceptions, body region only:** `mock_stream_rmoff__f10`, `mock_stream_scroll2__01_mid`.

---

## Global constraints

- **Version:** New evidence must come from pinned 2.1.316. Older parity plans don't override a 2.1.316 capture.
- **Copying:**
  - Don't copy Anthropic trademarks, the Clawd mark, source or long text.
  - Implementers must not read `artifacts/source`, the code carved from `claude.exe`.
  - Don't call this clean-room. The package's specs draw on that carved code.
  - Goldens, carved code, the ledger, results and anything else derived from Claude's interface stay out of the DaVinci repo, in `CC2316_EVIDENCE` or the reference package. Get legal advice before shipping publicly.
- **Security:** Keep project trust, approval expiry and nonce checks, cancellation, sandboxing, credentials and durable task lifecycle. A visual change must never grant execution or change permissions.
- **Subscription:** No new API-key requirement, billable service or speculative model call, and no change to Codex subscription routing. Automated runs use only the loopback fixture. Don't claim token savings without measurements from the authenticated route.
- **Code preservation:** Keep the pinned toolchain and dependencies. With the setting off, the shell must behave as it does today, and existing `terminal_*` tests stay unchanged.
- **Side effects:** Each commit, push, merge, install or default change needs separate approval. Don't touch real settings.
- **Platforms:** Windows Terminal fullscreen comes first. Classic, ConPTY, VS Code, macOS, Linux and tmux each need their own evidence.

---

## How parity is measured

### Tiers

Every golden is assigned one tier in the P04 ledger.

| Tier | Compared | Used for |
|---|---|---|
| **Exact** | Text, width, position, fg/bg, modifiers, cursor, terminal modes | Chrome, glyphs, functional labels such as `Press up to edit queued messages` |
| **Structure** | Everything in Exact except text inside named regions | Help prose, onboarding copy, panels listing DaVinci's own commands or settings |
| **Exception** | Reason recorded, region excluded or golden dropped | D1 and D3 only |

**P0** means a checklist P0 row in `PARITY_CHECKLIST.md` or `priority: P0` in `EVIDENCE_MATRIX.json`. Where they differ, the checklist wins.

### Region rules, not cell masks

DaVinci's text has different widths from Claude's, so cell masks can't work. A rule names its golden, a row and column region, a kind and a reason:

| Kind | Effect |
|---|---|
| `mark` | The DaVinci mark must stay inside the box (header rows 1–3, columns 0–9). Its contents aren't compared. |
| `identity` | Expected text is replaced from a substitution table, then re-padded. Style is still compared. |
| `data` | Text is dynamic, such as a path, count or timestamp. Style is still compared. |
| `structure` | Text is ignored. Geometry and style are compared. |

Cells outside every region must match exactly. A rule that covers more than a golden's stated region, or has no reason, fails the run. If DaVinci's text doesn't fit a region, that's a failure to fix, not a reason to widen the rule.

### Capture

- **Emulator:** `@xterm/headless` through the package's `xterm_dump.js`, the same replay as the goldens. Don't use pyte for cell comparison: `capture_tui.py` is pyte-based, and the package notes pyte misreads some keyboard sequences.
- **Format:** DaVinci captures use the goldens' schema, `claude-tui-styled-capture/2`.

### Offline backend

Use the pattern already in `crates/davinci-coding-agent/tests/output_schema.rs`:
- A loopback server speaks `openai-responses` SSE, the event shape in `tests/wor37_cache_status.rs`.
- A `models.json` custom provider in a disposable agent dir points at it, with a dummy key.
- The process starts with `env_clear`, isolated `HOME`, `USERPROFILE`, `PI_CODING_AGENT_DIR` and `DAVINCI_CODING_AGENT_DIR`, `PI_DISABLE_NETWORK=1` and `PI_CATALOG_DRY_RUN=1`. Don't set `PI_OFFLINE`: it replaces the provider with a local stub. Without the dry-run gate, startup opens an HTTPS connection that ignores proxy variables.
- A refusing proxy listener and TCP sampling fail the run on any foreign connection. A pass means nothing was observed, not that nothing happened: the guard can't see connections between samples, UDP or DNS.
- `TERM=xterm-256color` and `COLORTERM=truecolor`, as the reference harness sets them.
- No `auth.json` is present, so the subscription route can't be reached.
- The server records every request. A scenario fails if the count differs from what it expects, which also catches duplicate provider requests.

Scenario scripts come from `lean-claude-ui/scripts/scenarios`, with `mock_api.py`'s Anthropic events translated into Responses events.

**Tool mapping**

| Golden | DaVinci tool |
|---|---|
| Read | `read` |
| Write | `write` |
| Update | `edit` |
| Bash | `bash` |

### What exists to compare against

| Profile | Frames |
|---|---|
| 120×46 | 46 |
| 120×44 | 36 |
| 120×40 | 10 |
| 80×24 | 2 |
| 160×50 | 2 |
| 100×30, 60×20, 120×18 | 1 each |
| Light theme | 2 |
| Dark-ANSI | 1 |
| NO_COLOR | 0 |

A missing combination is marked `NO_REFERENCE`, never PASS. New goldens may be captured locally with the pinned binary and the mock, at no model cost. Never derive one by extrapolating from another size.

### Where each check runs

- **Rust tests:** Everything lives in one test binary, `crates/davinci-tui/tests/cc2316/main.rs`, with one module per area, because extra binaries add to CI disk pressure. They assert DaVinci behaviour using their own fixtures and run on CI.
- **Python tool tests:** These use synthetic captures and run on CI.
- **Golden comparison:** This runs on Windows only, against goldens outside the repo. If the goldens are absent it reports SKIP, never PASS.

---

## Procedure for every task

1. Write a failing test for the listed behaviour and its adverse case. Confirm it fails for the intended reason, not from an import or environment error.
2. Make the smallest change in the listed files, behind `tui.reference` where it affects display or keys.
3. Run the task's command, then compare the listed goldens. Record each mismatch as a fix or a ledger entry.
4. Check neighbouring input, focus and security states. Inspect the diff. Update the ledger with test output and limitations.
5. Run the same tests with the setting off where the behaviour is shared.

A task is done when a reviewer can inspect a real DaVinci result next to its reference. A Claude reference PASS proves nothing about DaVinci. Text-only references are marked **(text)** and can't support a cell-level claim.

Rust command form: `cargo test --locked -p davinci-tui --test cc2316 <module>::`

### Adverse tests that must exist

| Case | Owner |
|---|---|
| A permission expires, changes identity or loses focus while its dialog is open; a stale Enter or click must not approve another request | P12 |
| A large paste with wide glyphs, combining marks and emoji is edited and resized while a modal is open; text, attachments, cursor and focus survive without submitting | P08 |
| A late worker event after rename, pin, delete or reconnect must not resurrect a session, update another or cancel hidden work | P15 |
| A mode switch or exit during queued work must not send a duplicate request, lose input or leave raw, alternate or mouse mode on | P05, P16 |
| Escape codes, OSC links, control characters and secrets in tool output must not inject terminal commands or reach the clipboard | P10, P14 |

---

## Milestones

| Milestone | Tasks | Result |
|---|---|---|
| Setup | P00 | Work saved, branch on current main |
| A: evidence tools | P01–P04 | Manifest, comparator, replay, ledger |
| B: foundation | P05–P07b | Setting, terminal cleanup, theme, geometry, onboarding |
| C: conversation | P08–P11 | Editor, completion, commands and keys, transcript, streaming |
| D: controls and safety | P12–P14b | Permissions, modes, panels, scrolling, transcript mode |
| E: advanced | P15–P18 | Agents, renderer switching, accessibility, DaVinci-only views |
| F: release | P19–P20 | Full report, performance, rollout |

Merge to main once per milestone behind the setting, each with approval.

---

## Tasks

### P00: Save work and rebase

PR #275 merged as `543f9a3b`, and the branch has been moved to that commit. P01, P02, the plan and the spec are all still untracked.

- Commit P01, P02 and the docs. **Needs approval.**
- If main has moved again, bring the branch up to date first.

**Done when:** `git status` is clean in the worktree and the branch contains main.

### P01: Reference manifest — done

`scripts/ui/cc2316_manifest.py`, `scripts/tests/test_cc2316_manifest.py`. Unit tests pass.

**Remaining:** Re-run it against the full package and record the output.

### P02: Styled-cell comparison — done

`scripts/ui/cc2316_compare.py`, `scripts/tests/test_cc2316_compare.py`

**Required:**
- Change it to `compare(expected, actual, rules) -> DiffReport`.
- Expand runs into a cell grid, handling wide and combining glyphs.
- Implement the four rule kinds above and reject invalid rules.

**Adverse:** One corrupted cell or style outside a region fails. An extra trailing cell fails. An over-wide rule fails.

### P03: Replay and DaVinci capture

`scripts/ui/cc2316_replay.py`, `scripts/ui/cc2316_mock.py`, `scripts/tests/test_cc2316_replay.py`

**Required:**
- `replay(binary, scenario, profile) -> Capture`, reusing the package's `tui_harness.py` scenario runner and `xterm_dump.js`.
- The offline backend above.
- Timestamped keyboard, mouse, paste and resize events.
- Produce captures by importing `styled_xterm.process(raw_path, out_dir, name, cols, rows)` from the package, which writes schema `/2` with the goldens' palette and default-colour substitution. `styled_capture.py` is the older pyte schema `/1`; don't use it.
- `tui_harness.py` launches `claude.exe` by default and takes `--exe`. Its scenario actions are reusable, but `mock_api.py` is Anthropic-only. Four `mock_stream2`, `mock_stream_rmoff`, `mock_stream_scroll2` and `mock_interrupt2` goldens have no scenario file under that exact name, so find how they were driven or record them as needing a new scenario.

**First test:** Confirm an interactive `davinci` session accepts the custom provider and reaches the loopback server. Everything after this depends on it, so stop and report if it doesn't.

**Adverse:** A request beyond the expected count fails. An unsupported capability reports SKIP or BLOCKED.

### P04: Coverage ledger

`scripts/ui/cc2316_coverage.py`, `scripts/ui/cc2316_owners.json`, `scripts/tests/test_cc2316_coverage.py`. Output goes to the evidence folder, `CC2316_EVIDENCE`, outside the repo.

**Required:**
- One row per evidence feature (77), plus each of the 91 listed command rows and each keybinding (228).
- Each row has its source owner, tier, golden, regions, state and limitation.
- Fail on any missing row, or a VERIFIED status with no result behind it.
- The committed owners file holds only IDs and DaVinci facts. The owner doesn't want Claude reference data in the repo.

### P05: Setting, terminal modes and restore

`davinci/runtime.rs`, `davinci/term.rs`, settings plumbing, `tests/cc2316/terminal.rs`

**Reference:** RESTORE-01, FULL-01. Modes from `boot__02_settled`.

**Required:**
- Add `tui.reference` (D4), visible in `/config`.
- Exercise alternate screen, bracketed paste, SGR mouse, focus, synchronized output, keyboard enhancements and fallback.
- Check `bracketed_paste` and `focus_events` from the full raw recording. The goldens' values are wrong for these two, because `styled_xterm` only reads modes after the last `?1049h`.
- Restore all modes after normal exit, panic (`PI_DAVINCI_PANIC_FIXTURE` exists), interrupt and renderer switch.

### P06: Palette and styles

`davinci/theme.rs`, `tests/cc2316/theme.rs`

**Reference:** THEME-03, ANSI-01, PANEL-03, STYLE-01. `theme_apply__02_light_main`, `ansi_theme__02_settled`, `theme_nav__03_syntax_toggle`.

**Required:** Add `Theme::reference(depth, no_color)` beside `Theme::da_vinci`. Match captured colours and modifiers. User custom themes and NO_COLOR keep working. NO_COLOR is `NO_REFERENCE`.

### P06b: Theme picker

`views/settings.rs`, `davinci/runtime.rs` near line 1895, `davinci_interactive.rs` near line 7093, `tests/cc2316/theme.rs`

**Reference:** THEME-01, THEME-02. `theme_nav__01_theme`, `theme_nav__02_down2`, `theme_apply__01_light_selected`.

**Required:** A picker with the selection pointer, live diff preview and syntax line. DaVinci only has a settings row today.

### P07: Header, chrome and geometry

`views/chrome.rs`, `views/startup.rs`, `davinci/app.rs`, `davinci/ui.rs`, `tests/cc2316/chrome.rs`

**Reference:** BOOT-01, LAYOUT-01 to 04, EXIT-01. `boot__02_settled`, `layout_sizes__01` to `06`, `keys_panels__08_ctrl_c`.

**Required:**
- Three-line header using `mark` and `identity` rules, then transcript region, composer, working line and footer.
- Test short and large drafts, clipping, wide text and resize.

### P07b: Onboarding and trust

`views/startup.rs`, `views/trust.rs`, `tests/cc2316/chrome.rs`

**Reference:** ONBOARD-01, ONBOARD-02, checklist row 2. `onboarding__01_onboarding`, `onboarding__03_after_enter`. Trust gate is **(text)**.

**Required:** Structure tier with DaVinci's own copy. The trust decision's meaning doesn't change.
- Steps with the setting on: the welcome band with DaVinci's own art, the theme step, the security notes in DaVinci's own words, then DaVinci's existing analytics question.
- First run stays gated as `should_run_first_time_setup` has it today. The interactive shell has no launch-time trust gate, only a warning and `/trust`. Don't add one here.

### P08: Editor

The active shell doesn't use `crates/davinci-tui/src/editor.rs`. Don't change it for this task.

`davinci/model.rs` (`Composer`), `davinci/app.rs`, `davinci/paste_burst.rs`, `davinci/runtime.rs` (`open_in_editor`), `tests/cc2316/editor.rs`

**Reference:** PASTE-01, ESC-01, ESC-02, HIST-01, INPUT-12, INPUT-13, INPUT-15, INPUT-16, INPUT-18, checklist row 14. `paste_only__01_paste`, `paste_large__01_paste_12k`, `paste_large__03_unicode`, `esc_clear__01_esc1`, `esc_clear__02_esc2`, `keys_panels__06_history`, `image_paste__02_after_typing`, `vim_mode__04_dw`, `external_editor__01_after_editor`.

**Required:**
- Multiline editing, word movement and deletion, undo, history, double Escape, stash.
- Vim INSERT and NORMAL.
- Paste collapse above 10,000 characters and second-paste expansion.
- Image token, and Ctrl+G with the draft restored byte for byte.
- No modal or graph view takes composer keys.

**Open in the reference:** drag-drop, multiple images, deleting chips. Mark `NO_REFERENCE`.

### P09: Completion and shell mode

`crates/davinci-tui/src/autocomplete.rs`, `views/completion.rs`, `views/keys.rs`, `tests/cc2316/completion.rs`

**Reference:** SLASH-01, SLASH-02. `slash_only__01_slash`, `slash_only__02_tab`. AT-01 and AT-02 are **(text)**. Shell mode has no test ID, so mark it `NO_REFERENCE`.

**Required:** List position, filtering, Tab and Enter acceptance, Escape dismissal, file `@` mentions, click to select. Keep DaVinci-only commands, and never show an unsupported action as working.

### P09b: Commands and keybindings

`views/keys.rs`, `davinci/app.rs`, `tests/cc2316/keys.rs`, ledger

**Reference:** `COMMANDS.md` (91 rows), `KEYBINDINGS.md` (27 contexts, 228 bindings), TASK-01, `keys_panels__05_todos`.

**Required:** Mark each row as same, DaVinci equivalent or dropped. Implement only the ones marked same.

**Key conflict, with the setting on:**
- `ctrl+t` opens the tasks view, as in the reference.
- `ctrl+o` opens verbose transcript (SCROLL-04).
- DaVinci's tool-output toggle stays available through `showToolOutput` in `/config`.

Bindings the reference marks BLOCKED or SOURCE_SUPPORTED can't be claimed as parity until exercised in Windows Terminal. Some of those rows predate later verified specs, such as Ctrl+Enter and Ctrl+G, so check the focused spec first.

### P10: Transcript, Markdown, tools and diffs

`views/transcript.rs`, `views/transcript_cache.rs`, `views/markdown.rs`, `views/highlight.rs`, `views/opera.rs`, `views/diff.rs`, `davinci/sanitize.rs`, `tests/cc2316/transcript.rs`

**Reference:** TOOL-01 to 04. `top_tools2__01_transcript_top`, `top_tools2__03_pagedown`, `diff_view__03_diff`, `error_view3__02_up4`.

**Required:**
- Read, Write, Update and Bash blocks using the tool mapping.
- Compact and expanded output, line numbers, red and green diff spans, Markdown, error rows.
- Bound large output. Route all tool text through the existing `sanitize.rs`, and don't add a second sanitizer.

### P11: Streaming, interrupt and queue

`davinci/model.rs`, `davinci/app.rs`, `crates/davinci-coding-agent/src/davinci_interactive.rs`, `davinci_interactive_tests.rs`

**Reference:** MSG-10, MSG-10b, MSG-12, MSG-12b, MSG-13, MSG-13b, MSG-13c, MSG-13d, SCROLL-05b. `mock_stream2__f00`, `mock_stream2__f26`, `mock_interrupt2__03_after_submit`, `mock_queue__03_after_enter`, `mock_queue__04_after_esc`, `mock_queue_multi__02_up_edit`, `mock_sendnow__02_final`, `mock_stream_scroll2__04_done`, `mock_stream_rmoff__f26`.

**Exceptions (D1):** MSG-10c, SCROLL-05a.

**Required:**
- Live spinner and token counter, with the body streaming live (D1).
- Interrupt and send-now commit accumulated text.
- Enter queues, Up restores queued edits, Ctrl+Enter preempts.
- The request count stays exact.

**Adverse:** Two Ctrl-C presses during a turn currently send the queued line as a new provider request even when the user is exiting. Decide and test that an exit never starts a new request. Today only two quick Esc presses exit without one.

**Command:** `cargo test --locked -p davinci-coding-agent --bin davinci davinci_interactive`

### P12: Permission dialog

`views/approval_modal.rs`, `views/ask.rs`, `views/decision_modal.rs`, `davinci_interactive.rs`, `tests/cc2316/permissions.rs`

**Reference:** MSG-11, MSG-11b to e. `mock_permission_write__01_perm_dialog`, `mock_perm_approve__03_settled`, `mock_perm_deny__03_denied`, `mock_perm_cancel__02_after_esc`, `mock_perm_session__01_option2`, `mock_perm_session__02_allowed`.

**Required:**
- Map options to the existing `ToolApprovalDecision`: allow once to `AllowOnce`, session access to `AllowForSession`, deny and Escape to `Deny`.
- Keep `AllowAlways` as a DaVinci addition, and keep its rule persistence.
- A dialog option must not change the permission mode except through existing runtime policy.

Only Write was verified in the reference. Other tool categories are `NO_REFERENCE` and keep their current options.

**Adverse:** Expired challenge, reordered or missing request, stale selection, and refusal with file content unchanged.

### P12b: Mode cycle

`davinci/app.rs`, `views/chrome.rs`, `tests/cc2316/permissions.rs`

**Reference:** MODE-01 to 04. `keys_panels__01_mode1` to `04_mode4`.

**Required:**
- Shift+Tab cycles manual, accept edits, plan, auto.
- These map to DaVinci's `ask`, `edits`, `read-only`, `auto`.
- `always-approve` stays out of the cycle.
- The indicator shows only the runtime-confirmed `permission_mode`. A key press requests a change and never gets ahead of policy.

**Adverse:** A refused change leaves the indicator unchanged, and the draft survives.

### P13: Pickers and panels

`views/settings.rs`, `views/cogitator.rs`, `views/thinking.rs`, `views/keys.rs`, `views/context_usage.rs`, `views/context_inspector.rs`, `views/plan_usage.rs`, `views/rewind.rs`, `tests/cc2316/panels.rs`

**Reference:** MODEL-01 to 03, CONFIG-01, CONFIG-02, HELP-01 to 03, PANEL-08, PANEL-08b, PANEL-08c, PANEL-10, MSG-14, MSG-14b. `model_nav__01` to `03`, `config_search__01`, `config_search__02`, `help_tabs__01` to `03`, `usage_stats2__01`, `usage_stats2__02`, `usage_stats3__00` to `03`, `rewind_ui__02_rewind`, `context_cmd__02`, `context_cmd__03`, `context_all__02`.

**Required:**
- Tab order, search and selection treatment.
- Effort controls, session-only versus persisted options.
- Help is Structure tier.
- Usage and context show real values, and unknown is never shown as zero.
- A panel closing preserves the draft and unknown settings keys.

### P13b: Permissions panel

`views/permissions.rs`, `tests/cc2316/panels.rs`

**Reference:** PERM-01, PERM-02. `permissions_tabs__01_perms`, `permissions_tabs__03_tab4`.

**Required:** Tabs, search and rules, Structure tier, showing DaVinci's real rules. No edit here can broaden a permission without the existing path.

### P14: Scroll, mouse and clipboard

The active shell's copy path is `copy_to_clipboard` in `runtime.rs`. It doesn't use `crates/davinci-tui/src/clipboard.rs`, so don't change that file for this task.

`davinci/runtime.rs`, `davinci/selection.rs`, `davinci/links.rs`, `views/scrollbar.rs`, `tests/cc2316/mouse.rs`

**Reference:** SCROLL-01, SCROLL-02, SCROLL-03, LAYOUT resize frames, SELECT-01, MOUSE-01, MOUSE-02, MOUSE-03, WT-01. `scroll_fixture__01_bottom`, `04_home`, `06_wheel_up`, `09_resize_80x24`, `10_resize_160x50`, `selection__01_drag_select`, `mouse_cursor__01`, `mouse_cursor__02`.

**Required:**
- Page and wheel scrolling, sticky bottom and jump hint, click to place the caret.
- Tool expand. A second click doesn't collapse, as in the reference.
- Selection highlight, copy on select, selection across resize.
- Fix stale rows with a targeted full redraw, not a permanent one.
- P05 enabled `?1000h ?1002h ?1003h ?1006h` with the setting on, confirming selection, link clicks and drag only by reading the code. Drive them live here.

Hover and OSC 8 are UNVERIFIED in the reference, so mark them `NO_REFERENCE`.

### P14b: Transcript mode

`views/viewer.rs`, `views/transcript_cache.rs`, `tests/cc2316/mouse.rs`

**Reference:** SCROLL-04. `scroll_fixture__08_transcript`.

**Required:** Detailed header, timestamps, model name, verbose content, and `g`/`G`.

### P15: Agents dashboard

`views/agents.rs`, `views/agent_focus.rs`, `davinci/roster.rs`, `davinci/model.rs`, `crates/davinci-coding-agent/src/agent_roster_host.rs`, `tests/cc2316/agents.rs`

**Reference:** AGENT-01, AGENT-01a to g. All `agents_*` goldens (10).

**Required:**
- Empty state; Needs input, Working, Completed and Pinned groups; stopped and failed rows; counters; detail and attach.
- Rename (Ctrl+R), pin (Ctrl+T), find (Ctrl+F), two-step delete (Ctrl+X).
- Rename and pin need a selected row.
- Closing the dashboard never stops work.

**Scope:** The existing view is a worker-control sheet. `claude agents` as a separate command-line entry isn't covered by this task, so record it in the ledger.

### P16: Renderer switching

`davinci/runtime.rs`, `davinci/model.rs`, `davinci_interactive.rs`, `tests/cc2316/modes.rs`

**Reference:** TUI-01, FULL-02. `tui_panel__02_tui_panel`. Other evidence is Windows Terminal screenshots.

**Start with a spike.** The active shell owns the alternate screen, and only the legacy path has a main-screen renderer. So classic mode may need a new render path, and most goldens were captured with fullscreen forced. Report the size of that work before building it. If it's large, fullscreen ships first and classic gets its own plan.

**Required:** `/tui` switching and re-entry, with no duplicate composer or footer, no stuck processing line and no repeated request. No survey (D3).

### P17: Accessibility

`davinci/term.rs`, `views/startup.rs`, `views/opera.rs`, `tests/cc2316/a11y.rs`

**Reference:** A11Y-01, A11Y-05. `screen_reader__02_settled`, `reduce_motion_toggle__02_motion_toggled`.

**Required:** Screen-reader text-only mode, and the reduced-motion toggle. Don't invent drag-drop or mark animations. Reduced-motion effects beyond the toggle are PARTIAL in the reference.

### P18: DaVinci-only views

`views/graph_run.rs`, `views/graph_canvas.rs`, `views/mensura.rs`, `views/governor.rs`, `views/mcp.rs`, `views/extensions.rs`, `views/task_board.rs`, `tests/cc2316/extensions.rs`

**Reference:** None. Parity doesn't apply here, so this is a consistency review against the shared chrome and palette.

**Required:**
- Check every `Screen` and `Overlay` variant and nested dialog, and list each in the ledger.
- No features are removed.
- The graph never takes chat input, stops a hidden worker, creates a branch or fabricates account or cost data.

### P19: Full comparison

`scripts/ui/cc2316_report.py`, with `results.md` written to the evidence folder

**Required:** Add a `run_suite.py` adapter that runs the same test IDs against DaVinci snapshots. Run all 95 kept goldens, all P0 actions and all adverse tests with the setting on, then the existing suites with it off. Report only DaVinci results, with failed, blocked and `NO_REFERENCE` shown as they are.

**Command:**

```powershell
cargo test --locked -p davinci-tui && cargo test --locked -p davinci-coding-agent --bin davinci
```

This task checks the whole, so it has no failing-test step of its own.

### P20: Performance and rollout

`scripts/ui/cc2316_perf.py`, with `release-gates.md` written to the evidence folder

Use the package's `measure_perf.py` on the same machine and terminal. `frame_cost.rs` is a timing probe, not a gate, and can be supporting data only. The harness adds 20–50 ms of noise, so sub-frame targets can't be proven with it.

| Metric | Reference median | Gate: no worse than | Goal, recorded not gated |
|---|---:|---:|---:|
| First prompt | 3.35 s | 3.35 s | under 2 s cold, 500 ms warm |
| Input to render | 25 ms | 25 ms | under 16 ms |
| Resize to render | 20 ms | 50 ms | under 50 ms |
| PageDown | 20 ms, 2.9 KB | 20 ms | under 16 ms, under 32 KB |
| Idle CPU | 0–0.8% | 1% | under 1% |
| Idle RSS | 249 MB | 249 MB | under 150 MB |

**Rollout:**
1. Opt-in with the setting.
2. A human check in Windows Terminal.
3. Your approval to change the default.

Don't remove the old paths in this work. Test that open sessions and saved settings survive a switch in both directions. `--legacy-tui` stays as it is.

---

## Regression commands

Run from the worktree:

```powershell
python -m unittest discover -s scripts/tests -p "test_cc2316_*.py" -v
cargo test --locked -p davinci-tui
cargo test --locked -p davinci-coding-agent --bin davinci davinci_interactive
cargo fmt --all -- --check
cargo clippy --locked -p davinci-tui -p davinci-coding-agent --all-targets -- -D warnings
```

Keep commands, exit codes and diagnostics in the ledger. Use `rtk` wrappers where `AGENTS.md` requires them.

## Release gates

- **Visual:** No unexplained difference on any kept P0 golden in the primary profile, under the rules above.
- **Behaviour:** All mapped P0 actions pass in DaVinci. No accidental submission or permission grant. Task identity stays stable across resize, interrupt and reconnect.
- **Safety:** Expired requests can't be approved. Denial can't change files. Tool output can't inject terminal sequences. Cancellation affects only the intended task.
- **Off state:** With the setting off, existing suites pass unchanged.
- **Performance:** Gates in P20 met, with goals and misses recorded.
- **Reference limits:** Image drag-drop, mic-enabled voice, some reduced-motion, cursor shape and mark animation, the full key sweep, OSC 8 hover, VS Code, macOS, Linux, tmux, Ghostty, and ConPTY Ctrl+L and Shift+Up can't be declared parity-complete without new evidence.
