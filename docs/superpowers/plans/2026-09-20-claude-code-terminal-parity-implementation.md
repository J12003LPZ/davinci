# Whole-Terminal Claude Code Parity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rebuild every DaVinci terminal surface to match the pinned Claude Code CLI, then integrate the approved optional graph without losing usable input or independent background execution.

**Architecture:** Keep Rust, Ratatui, Crossterm and the existing execution contracts. Replace the shared presentation primitives, shell layout and focus routing; adapt every view to those components. Introduce scoped task/draft/event state at the host boundary only where actual concurrent work requires it.

**Tech Stack:** Existing workspace dependencies and lockfile; `davinci-tui`, `davinci-coding-agent`, and targeted agent/protocol changes only when required by task identity and lifecycle tests. Use offline fixtures, not paid model calls, for regression evaluation.

**Spec:** `docs/superpowers/specs/2026-09-20-claude-code-terminal-parity-design.md`.

**Branch:** `J12003LPZ/terminal-ui-rebuild-20260920`.

**Inspected head:** `953fbc7ca254c429ee7c27645ca1f8179d2cb72e`.

**Approval:** The user approved the written specification. This implementation plan is awaiting review and execution-method selection. No product-code change or passing-build claim is represented by this document.

## Global Constraints

- “Rebuild **the entire DaVinci terminal UI as a 1:1 visual and interaction match to Claude Code**.” Whole-terminal parity is the primary deliverable; the graph is an additional feature.
- Reference: **Claude Code v2.1.278**. Native Windows Terminal fullscreen is the primary profile; classic rendering is a separately recorded profile. Do not mix versions, terminal metrics, themes or renderer profiles.
- “Permitted differences are DaVinci's product identity, real model/provider names and capabilities, actual account/configuration data, and its additional graph and harness features.” Never mislabel another model as Claude or display fabricated balances, branches or controls.
- “Keep Rust, Ratatui and Crossterm and retain the existing providers, tools, storage, permissions and graph execution contracts unless a tested interaction requires a targeted change.” Keep the existing lockfile and dependency versions unless a demonstrated requirement needs a reviewed change.
- “Graph execution does not force open a canvas or remove the prompt.” Closing a view never stops a run. Inspecting an agent never silently redirects submission.
- “Never change permissions, telemetry consent, project trust, credentials or execution defaults merely to match a screen.” Preserve unknown configuration keys, explicit custom themes and rollback data.
- “Static regions require zero unexplained cell/style differences.” Missing reference captures are blocking evidence, not permission to call an invented layout a match.
- Rendering sizes: **40x12**, **80x24**, **100x30**, **120x40**, **160x50**. Dark/light, reduced-color and no-color; native Windows and POSIX; regular and fullscreen.
- “Do not merge into `main` or restart the user's running harness without authorization.” Never modify the shared checkout or commit screenshots, recordings, fonts, secrets or compiled binaries.

## Review Focus

1. A catalog refresh or filtering operation moves a selected model: accept by provider/model identity, never by a stale visual row. Tests in Task 5.
2. A permission expires or a different run requests approval while a dialog is visible: no stale key or click grants a different action. Tests in Tasks 6 and 8.
3. A resize occurs during multiline paste, IME input or attachment editing: preserve draft, cursor, attachments and correct focus. Tests in Tasks 2, 3 and 9; IME also has a native manual gate.
4. A reconnect delivers duplicated, reordered or old-generation task events: reconcile authoritative state without corrupting a different task or losing retained output. Tests in Task 8.
5. A tool or extension emits long Unicode output, terminal controls or credential-like text: rendering and copy remain bounded, safe and consistently redacted. Tests in Tasks 2, 4 and 7.

## Observed Code Constraints

These are observations from the inspected branch, not assumptions about a future implementation:

- `davinci/app.rs::compose_frame` owns shell composition and returns `ComposedFrame`, including microphone and graph geometry. `body_with_graph` treats `Screen::GraphRun` as a replacement screen.
- `views/graph_run.rs::chrome` sets `Composer::Hidden`. Changing that field alone would not fix input because `handle_key` also routes non-Agent screens to `InputOwner::Surface`.
- `app.rs::handle_global_key` handles shell shortcuts before the shared editor. Its source explicitly gives Ctrl+U and Ctrl+B to DaVinci instruments, conflicting with the reference editing/backgrounding behavior. Update routing and displayed hints together.
- `ui.rs::paper_label` and `print_rule` introduce paper labels, uppercase decorations and textured separators. All consumers must migrate; do not merely change the brown RGB values.
- `model.rs::Entry::Tool` already carries state, target, summary and output. Retain this data and replace its presentation rather than throwing away useful output.
- The host is `crates/davinci-coding-agent/src/davinci_interactive.rs`. It owns agent events, submission and approval channels. Its `NativeApproval` and `NativeDecision` have live flags and deadlines; preserve those checks.
- `views/settings.rs` retains the runtime's setting ordering; `views/cogitator.rs` currently preserves catalog source indices. A searchable replacement must maintain backend identity across refreshes.

## Execution Preparation and Current Limitations

A planning-time check found the connected desktop **offline**. The local container has Git and Node, but no `cargo`, `rustc` or `claude` on PATH; `git ls-remote` failed with `Could not resolve host: github.com`. GitHub connector reads and document writes remain available. The archive-download route also failed. No Rust test or native capture ran.

Before Task 1, reconnect an authorized build host and verify tools. Reuse the existing branch; do not create a second unrelated branch, run `git switch` in another session's checkout, or overwrite dirty work. Read `AGENTS.md`, inspect `git status` and `git worktree list`, then attach this branch to its own worktree using the repository's isolation rules. If already checked out elsewhere, use that dedicated worktree only after confirming it is not owned by another active session. Record the exact worktree and branch in the triage report.

Run the following only in that isolated worktree, after plan approval:

```sh
git status --short
git branch --show-current
git rev-parse HEAD
rustc --version
cargo --version
cargo metadata --locked --no-deps --format-version 1
cargo test --locked -p davinci-tui
cargo test --locked -p davinci-coding-agent
```

Capture exit codes and the baseline failures before editing. Do not attribute pre-existing failures to this work or call them passes. Do not install a global replacement binary or stop running agents as part of setup.

## File and Interface Map

Paths below are relative to the repository. `TUI` is a documentation abbreviation for `crates/davinci-tui/src/davinci`; `HOST` means `crates/davinci-coding-agent/src`. Expand these prefixes when editing.

| Owner | Files | Responsibility |
| --- | --- | --- |
| Reference evidence | New `docs/ui/claude-code-reference/{README.md,manifest.json,actions.json}`; new `crates/davinci-tui/tests/terminal_parity.rs`; new `crates/davinci-tui/examples/terminal_parity_capture.rs` | Pinned environment, full surface inventory, recorded inputs, deterministic output and comparison |
| Shared presentation | Modify `TUI/{theme.rs,ui.rs}`; new `TUI/components/{mod.rs,picker.rs,dialog.rs}` | Measured theme and geometry, Unicode-safe rows, list/dialog styling |
| Shell and focus | Modify `TUI/{app.rs,model.rs,runtime.rs}`; new `TUI/{layout.rs,focus.rs}`; modify shared `crates/davinci-tui/src/{interaction.rs,keybindings.rs,editor.rs}` where tests require it | Explicit region ownership, draft preservation and correct key routing |
| Conversation | Modify `TUI/views/{startup.rs,chrome.rs,transcript.rs,markdown.rs,highlight.rs,semantic.rs,opera.rs,studio.rs,completion.rs}` | Welcome, editor, assistant/user content, tools, activity and suggestions |
| Selectors | Modify `TUI/views/{settings.rs,cogitator.rs,thinking.rs,instrumenta.rs,resume.rs,tree.rs}` | Real configuration/model/session selection through matched components |
| Safe dialogs | Modify `TUI/views/{approval_modal.rs,decision_modal.rs,ask.rs,trust.rs,permissions.rs,login.rs,secret_input.rs}` | Reference-style authorization/questions without changing policy |
| Remaining surfaces | Every remaining module in `TUI/views/mod.rs`, including help, recovery, MCP, voice-related chrome and utilities | No old UI islands |
| Runtime adapter | Modify `HOST/davinci_interactive.rs`; new `HOST/davinci_interactive/{task_router.rs,view_events.rs}`; new `TUI/workspace.rs` | Scoped tasks, drafts, lifecycle and event reconciliation |
| Graph extension | Modify `TUI/views/{grafo.rs,graph_run.rs,graph_layout.rs,graph_canvas.rs,graph_inspector.rs,graph_nav.rs,graph_perf.rs}` | Optional dock/list/canvas, meaningful activity and safe controls |
| Release validation | New `docs/ui/terminal-parity-report.md`; update `README.md` and existing UI documentation; new offline fixtures under `crates/davinci-tui/tests/fixtures/terminal_parity/` | Coverage, reproducible gates, build/restart/rollback instructions |

New names in this map are planned files, not claims they exist. Keep changes focused: the host may delegate to its new modules, but do not refactor unrelated tools or providers.

## Task 1: Establish Reference Captures and a Failing Parity Harness

**Files:** Reference evidence files from the map; `TUI/fixtures.rs`; `crates/davinci-tui/tests/terminal_parity.rs`.

**Consumes:** Approved spec; actual pinned reference CLI; existing `Model::new`, `fixtures`, `app::compose_frame`.

**Produces:** A manifest covering every screen, overlay and nested state; a capture executable; a comparison test named `terminal_parity_reference_gate`. The manifest schema is version 1. Each case records `id`, `surface`, `state`, `reference_version`, `renderer`, `theme`, terminal dimensions, environment, input trace, reference-frame digest, candidate-frame digest and explicit variable-field normalization ranges. Capture locations are outside source control.

- [ ] **1. Record baseline and provenance.** Verify `claude --version` is the pinned version. Record OS, terminal version, font name/size, cell dimensions, renderer and settings in both profiles. Use isolated configuration and fixture repositories; do not copy personal credentials into fixtures. Obtain returning-user captures on an authorized reference session, with secrets excluded. Account-unavailable states remain blocked.
- [ ] **2. Write a failing inventory test.** The harness must reject missing captures, wrong versions and mismatched renderer profiles. It must never create its own expected frames from DaVinci output.

```rust
#[test]
fn terminal_parity_reference_gate() {
    let manifest = reference_manifest();
    assert_eq!(manifest.schema_version, 1);
    for case in &manifest.cases {
        assert_eq!(case.reference_version, "2.1.278");
        assert!(!case.input_trace.is_empty(), "{}: no trace", case.id);
        assert!(!case.reference_digest.is_empty(), "{}: no reference", case.id);
        let difference = compare_case(case);
        assert_eq!(difference.unexplained_cell_differences, 0, "{}", case.id);
        assert_eq!(difference.unexplained_style_differences, 0, "{}", case.id);
    }
    assert_complete_surface_inventory(&manifest);
}
```

Define `reference_manifest()` in this test file to deserialize version-1 JSON through existing `serde_json`, and `assert_complete_surface_inventory()` to compare case surfaces to the explicit inventory in Task 7. Do not skip unavailable cases with `filter_map`, `#[ignore]` or an empty test vector. `compare_case(case: &ReferenceCase) -> DifferenceReport` must render the current candidate through `app::compose_frame`, load the separately captured reference, and compute the two difference counts. It must not trust stored pass/fail totals; a missing frame, digest mismatch or render error fails the test. `DifferenceReport` contains `unexplained_cell_differences: usize` and `unexplained_style_differences: usize`.

- [ ] **3. Run red.** `cargo test --locked -p davinci-tui --test terminal_parity terminal_parity_reference_gate`. Before captures and the new renderer, missing evidence or visible differences must fail. Save the failure log.
- [ ] **4. Implement capture and comparison.** The capture example accepts `--case`, `--width`, `--height`, `--theme` and `--renderer`; rejects unknown cases/options; uses deterministic `Model` fixtures; renders through the real frame path; exports every cell's symbol, foreground, background and modifiers, cursor and hit regions. Reference frames must come from the reference terminal session. Compare dimensions and then the full cell/style grid; apply only case-specific approved identity/path/time substitutions. Record animation traces and elapsed timestamps separately. Keep reference digests and provenance in the manifest; do not commit image binaries or fonts.
- [ ] **5. Capture all reference states.** Startup, conversation, prompt/paste, tools, commands, settings, models/effort, permissions, login, sessions, diffs, tasks, help and nested dialogs. Public demo images and older screenshots are reconnaissance only. Write a source-to-case mapping so a current capture, not an unrelated screenshot, controls each equivalent view.
- [ ] **6. Commit the harness and evidence metadata.** `git add docs/ui/claude-code-reference crates/davinci-tui/tests/terminal_parity.rs crates/davinci-tui/examples/terminal_parity_capture.rs crates/davinci-tui/src/davinci/fixtures.rs`; commit as `test(tui): establish pinned Claude Code parity harness`. Remaining product differences stay explicitly failing in the report; do not label the branch ready to merge.

## Task 2: Replace the Shared Visual System

**Files:** `TUI/{theme.rs,ui.rs}`, new `TUI/components/{mod.rs,picker.rs,dialog.rs}`, `TUI/mod.rs`; tests in those modules.

**Consumes:** Measured reference tokens/geometry from Task 1, existing color-depth and no-color modes.

**Produces:** `ReferenceScheme::{Dark,Light}` and `Theme::reference(depth: ColorDepth, no_color: bool, scheme: ReferenceScheme) -> Theme`; measured picker/dialog rows; grapheme-safe existing `ui::clip` and `clip_ellipsis`.

- [ ] **1. Write regression tests before replacing primitives.** Include the complete grapheme, zero-width terminal and style-difference cases.

```rust
#[test]
fn reference_clip_preserves_complete_graphemes() {
    assert_eq!(ui::clip("👩‍💻x", 2), "👩‍💻");
    assert_eq!(ui::clip("e\u{301}x", 1), "e\u{301}");
    assert_eq!(ui::clip("界x", 1), "");
    assert_eq!(ui::clip_ellipsis("abcdef", 0), "");
}
```

- [ ] **2. Run red.** `cargo test --locked -p davinci-tui reference_clip`; also run reference primitive fixtures. Demonstrate actual baseline behavior instead of assuming a test failed.
- [ ] **3. Implement whole-grapheme clipping using existing dependencies.** Preserve width in terminal cells, including combined emoji; prevent arithmetic overflow for unusually long text.

```rust
let mut out = String::new();
let mut used = 0usize;
for g in unicode_segmentation::UnicodeSegmentation::graphemes(text, true) {
    let cells = unicode_width::UnicodeWidthStr::width(g);
    if used.saturating_add(cells) > usize::from(max) { break; }
    out.push_str(g);
    used = used.saturating_add(cells);
}
```

- [ ] **4. Replace paper labels, textured rules, universal uppercase headings and legacy selection bands in the reference profile.** Read actual values from the captured token table. Preserve token APIs only where they remain useful; migrate each caller instead of introducing a second incompatible styling system. Keep explicit custom-theme rendering opt-in. Implement `PickerRow { id: String, label: String, detail: String, enabled: bool }` and `picker_rows(rows: &[PickerRow], selected_id: Option<&str>, width: u16, theme: &Theme) -> Vec<Line<'static>>`. Implement `dialog_rows(title: &str, body: Vec<Line<'static>>, width: u16, theme: &Theme) -> Vec<Line<'static>>` with measured reference geometry. Lists return rows; they do not execute choices.
- [ ] **5. Run green and reference comparison.** Test all widths and color depths; measure contrast and selection legibility with captured profiles. No guessed orange/gray palette counts as a match.
- [ ] **6. Commit.** `git commit -m "feat(tui): replace decorative chrome with reference primitives"` after staging only the listed files.

## Task 3: Rebuild Welcome, Shell, Editor and Input Ownership

**Files:** `TUI/{app.rs,model.rs,runtime.rs,layout.rs,focus.rs}`, `TUI/views/{startup.rs,chrome.rs,sheet.rs}`, shared interaction/keybindings/editor tests.

**Consumes:** Task 2 primitives; reference input traces; existing `Composer` backed by `Editor`.

**Produces:** `UiFocus::{Composer,Completion,Panel,Graph,Inspector,Modal}` (derive `Clone, Copy, Debug, Eq, PartialEq`), `resolve_focus(requested: UiFocus, modal_open: bool, completion_open: bool) -> UiFocus`, and `ShellRegions` containing transcript, optional panel, composer and status rectangles. `allocate_regions(width: u16, height: u16, composer_rows: u16, panel_open: bool) -> ShellRegions` owns geometry; frame hit-testing uses these same rectangles.

- [ ] **1. Write baseline key regressions, including the observed Ctrl+U conflict.**

```rust
#[test]
fn reference_ctrl_u_edits_input_not_usage_screen() {
    let mut m = Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 100, 30, false);
    m.composer.set_text("remove this line");
    let flow = app::handle_key(&mut m,
        KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
    assert_eq!(flow, app::Flow::Continue);
    assert_eq!(m.screen, Screen::Agent);
    assert_eq!(m.composer.editor().get_text(), "");
}
```

Add cursor/attachment snapshots before and after model/settings overlays, multiline paste, cancel and repeated resize. Test Ctrl+D with a nonempty editor separately from exit. Input tests use the reference's measured platform traces, not guessed escape sequences.

- [ ] **2. Run red.** `cargo test --locked -p davinci-tui reference_ctrl_u`; run the new draft/resize cases by name.
- [ ] **3. Implement region and focus ownership.** Calculate reserved editor/status height before allocating transcript/panels. If a terminal is too small, show a deterministic fallback and keep the active input or permission decision reachable. Do not solve clipping by discarding the draft. Route modal keys first, then focused panel/completion keys, then editor input. Background work does not determine keyboard focus.

```rust
pub fn resolve_focus(requested: UiFocus, modal_open: bool, completion_open: bool) -> UiFocus {
    if modal_open { return UiFocus::Modal; }
    let base = if requested == UiFocus::Modal { UiFocus::Composer } else { requested };
    if completion_open && base == UiFocus::Composer { UiFocus::Completion } else { base }
}
```

Use this resolved owner to dispatch the existing `handle_overlay_key`, `handle_screen_key`, `handle_suggestion_key` and editor paths, preserving their concrete `Flow` results. Completion dismissals restore composer focus; closing a panel restores its recorded prior focus. Replace screen-based ownership checks in both native rendering paths and test every transition.

- [ ] **4. Rebuild startup and prompt from the captures.** Match main transcript placement, first-run/returning states, whitespace, prompt separators, multiline growth, cursor, attachment chips, queue markers and contextual hints. Remove the earlier invented cards, permanent branch header and destination row. Keep microphone affordances as a DaVinci extension using the same style. Do not advertise a key until routing implements it.
- [ ] **5. Run green.** Both runtime paths, all sizes; multiline/IME/paste review on native Windows; reference static-cell comparison for startup/editor. Ctrl+B's task action is completed in Task 8, not mapped to the old code split.
- [ ] **6. Commit.** `git commit -m "feat(tui): rebuild reference shell editor and focus routing"`.

## Task 4: Rebuild Conversation, Tools, Activity and Diffs

**Files:** `TUI/views/{transcript.rs,markdown.rs,highlight.rs,semantic.rs,opera.rs,studio.rs,diff.rs,codex.rs}`, `TUI/model.rs`, targeted event-to-entry adaptation in `HOST/davinci_interactive.rs`.

**Consumes:** Real `Entry` data and Task 2/3 presentation; reference fixtures for tool, text, markdown, diff and interruption states.

**Produces:** Reference conversation rendering and per-call expansion state. Define `ToolCallKey { run_id: String, call_id: String }`, deriving `Eq`, `PartialEq`, `Ord`, `PartialOrd`, `Clone`, `Debug`; store expansion in `BTreeMap<ToolCallKey, bool>` rather than a single global flag. Map existing transcript entry locations to these stable keys in the host adapter.

- [ ] **1. Add regression fixtures with real existing entry constructors.**

```rust
let entries = vec![
    Entry::user("Explain this change."),
    Entry::tool(State::Active, "Bash", "cargo test --workspace", None)
        .with_output("test parse ... ok\ntest render ... FAILED"),
    Entry::failure("test failed", "render"),
    Entry::prose("The rendering test failed; implementation is not verified."),
];
```

Test running/completed/failed/cancelled calls, independent expansion, code fences, long paths, markdown tables, links, empty lines and a 100,000-line tool stream. A failed command remains visible when successful output is collapsed. Expanded output retains a bounded working set or pages from retained storage; it must not invent missing output beyond existing retention limits.

- [ ] **2. Run red.** Compare those cases using `terminal_parity_reference_gate` and run the new per-call expansion tests.
- [ ] **3. Implement reference message/tool layouts and working animation.** Use actual command/path/summary; unfold details only when requested. Keep the exact failure and cancellation state. Drive animation from injected tick/time for deterministic traces. Preserve scroll anchors as streaming content grows; do not append a fresh duplicate tool card on every delta.
- [ ] **4. Rebuild diffs and sanitization.** Match file lists, line numbers, additions/deletions, context and expanded navigation. Pass tool/extension content through the existing redaction/control-sequence boundary before display and copy; preserve only supported safe styling. Never execute OSC clipboard, title or arbitrary terminal actions from output. Add redaction/copy tests with split escape sequences and fake secret fixtures.
- [ ] **5. Run green.** `cargo test --locked -p davinci-tui`; targeted host event tests; record cell/style comparisons and bounded-output evidence. Verify plain-text copy matches displayed safe content, not raw control bytes.
- [ ] **6. Commit.** `git commit -m "feat(tui): match reference conversation tools and change review"`.

## Task 5: Rebuild Commands, Settings, Models and Reasoning

**Files:** `TUI/components/picker.rs`, `TUI/views/{completion.rs,instrumenta.rs,settings.rs,cogitator.rs,thinking.rs}`, `TUI/{app.rs,model.rs}`, shared autocomplete, host setting/model choice handling.

**Consumes:** `PickerRow`, real setting keys and provider/model IDs; reference config and model captures.

**Produces:** Reference selectors with stable identity. Add `visible_indices(rows: &[PickerRow], query: &str) -> Vec<usize>`; use stable selected IDs and resolve against the current dataset on acceptance. Duplicate model labels are not duplicate IDs: provider and model ID form a tuple.

- [ ] **1. Add failing identity tests.**

```rust
#[test]
fn reference_picker_resolves_source_identity_after_filtering() {
    let rows = vec![
        PickerRow { id: "provider-a/model".into(), label: "Model".into(), detail: "A".into(), enabled: true },
        PickerRow { id: "provider-b/model".into(), label: "Model".into(), detail: "B".into(), enabled: true },
    ];
    assert_eq!(visible_indices(&rows, "B"), vec![1]);
    assert_eq!(rows[visible_indices(&rows, "B")[0]].id, "provider-b/model");
}
```

Also test zero results, disabled items, Unicode queries, insertion/removal during refresh, current-model marker versus navigation focus, and unsupported effort levels. A removed selection is rejected or explicitly refocused; it must not choose its former neighbor.

- [ ] **2. Run red.** `cargo test --locked -p davinci-tui reference_picker`; record config/model screenshot mismatches separately.
- [ ] **3. Implement filtering without reordering backend arrays.**

```rust
let query = query.to_lowercase();
rows.iter().enumerate().filter_map(|(i, row)| {
    let haystack = format!("{} {}", row.label, row.detail).to_lowercase();
    (query.is_empty() || haystack.contains(&query)).then_some(i)
}).collect::<Vec<_>>()
```

Keep identity resolution separate from filtering. A click and Enter resolve the same stable choice from the same rendered frame. Revalidate availability before the host applies it.

- [ ] **4. Rebuild the actual selectors.** Match `/` and `@` completion, full/quick model pickers, effort controls, selected/current states, config tabs/categories/search, descriptions and value positions to their captures. Preserve existing setting scope and persistence. Expose `/config` as an alias to the settings surface while retaining `/settings`; do not create unrelated cloud/account controls. Model authentication/validation failures leave the original selection active and display the failure.
- [ ] **5. Run green and persistence tests.** Save/reload fixtures with unrelated and unknown JSON keys; assert only the intended key changes. Verify session-only versus persistent actions only where implemented by DaVinci and explicitly labeled. Run TUI and host suites; compare every picker state in both reference profiles.
- [ ] **6. Commit.** `git commit -m "feat(tui): match commands configuration and model selection"`.

## Task 6: Rebuild Permissions, Questions, Trust and Authentication

**Files:** All safe-dialog files in the map; `TUI/{app.rs,model.rs}`; `HOST/davinci_interactive.rs` approval/decision integration.

**Consumes:** Task 2 dialog primitives and the existing request IDs, live flags, reply channels and deadlines.

**Produces:** Reference dialog presentation with unchanged permission authority. Add a pure guard in the host: `approval_target_matches(displayed_id: &str, active_id: &str, now_ms: u64, expires_at_ms: u64, live: bool) -> bool`.

- [ ] **1. Write guard and integration regressions.**

```rust
#[test]
fn reference_approval_never_accepts_a_stale_target() {
    assert!(!approval_target_matches("old", "new", 10, 20, true));
    assert!(!approval_target_matches("same", "same", 20, 20, true));
    assert!(!approval_target_matches("same", "same", 10, 20, false));
    assert!(approval_target_matches("same", "same", 10, 20, true));
}
```

- [ ] **2. Run red.** Targeted host approval tests and reference modal fixtures. Include Escape, timeout, disconnect, error/panic cleanup and changing selection during output updates.
- [ ] **3. Implement the guard and invoke it at dispatch, not only at render time.**

```rust
live && displayed_id == active_id && now_ms < expires_at_ms
```

Keep the existing monotonic deadline and `NativeApproval::is_live` checks as well. A stale frame's mouse click cannot authorize the replacement request. Cancellation sends the existing deny/cancel response for that exact request.

- [ ] **4. Rebuild trust, permission and question screens; provider login and masked secret entry.** Match titles, selection, descriptions, nested comments, errors and dismissal. Preserve zeroizing secret storage, prohibit secret values in transcript/export/copy, and restore the user's unrelated draft after closing. Do not copy reference account information or grant broader permission because the reference offers an option DaVinci does not support.
- [ ] **5. Run green.** TUI and host suites with fake approval channels and clocks. Native keyboard/mouse verification; evidence that denied/expired actions never execute.
- [ ] **6. Commit.** `git commit -m "feat(tui): rebuild authorization and authentication dialogs"`.

## Task 7: Convert Every Remaining Screen and Overlay

**Files:** Remaining `TUI/views/mod.rs` modules; `TUI/fixtures.rs`; terminal-parity test inventory and voice/extension presentation integration.

**Consumes:** Reference components, existing data and the following mandatory inventory.

**Produces:** An exhaustive mapping to either a captured Claude-equivalent case or a named DaVinci-only extension using matched components. A screen is not done merely because its shared heading changed.

| Screen | Implementation task / view |
| --- | --- |
| Agent | 3–4: startup, chrome, transcript |
| Plan | 7: disegno |
| Grafo | 9: grafo |
| Memoria | 7: memoria |
| Mensura | 7: mensura |
| Models | 5: cogitator |
| Settings | 5: settings |
| Thinking | 5: thinking |
| Login | 6: login |
| Keys | 7: keys |
| Resume | 7: resume |
| Tree | 7: tree |
| Compact | 7: compact |
| Export | 7: export |
| GraphRun | 9: graph_run |
| Vectors | 7: vectors |
| Governor | 7: governor |
| Securitas | 7: securitas |
| Trust | 6: trust |
| Officina | 7: officina |
| Recovery | 7: recovery |
| Diff | 4: diff |
| Mcp | 7: mcp |
| Permissions | 6: permissions |
| Workflows | 7–8: workflows |
| TaskBoard | 7–8: task_board |
| Agents | 7–8: agents |
| ContextInspector | 7: context_inspector |

Overlays: `Instrumenta` (5), `Sessions` (7), `Cogitator` (5), `SecretInput` (6), `Ask` (6). Also audit `budget`, `rewind`, all nested dialogs, notifications and voice setup/status; enum coverage alone is insufficient.

- [ ] **1. Add an exhaustive `Screen` and `Overlay` match in fixture dispatch without a wildcard.** This makes newly added enum variants require coverage. Add case records for empty, loading, error and selected states. Count 28 current screens and five current overlays; compare their names, not only counts.
- [ ] **2. Run red.** `cargo test --locked -p davinci-tui --test terminal_parity`; confirm old-layout variants fail and missing cases cannot silently pass.
- [ ] **3. Convert sessions/history, help, context/memory, usage/budgets, exports/security/recovery, MCP/extensions, plans, tasks and agent views.** Reuse stable pickers and safe dialogs rather than custom full-width forms. Preserve session selection/search, rewind scope, export paths, real counters and diagnostics. For DaVinci-only functions, label them as extensions in the manifest instead of claiming a nonexistent reference screen.
- [ ] **4. Audit extension-owned and voice-owned output.** Keep real callback/data contracts. Translate visual output through the new primitives, bound long extension rows and sanitize their text/copy. Test a voice-send block remains effective while panels are open and that completion does not silently send recorded audio.
- [ ] **5. Run green and inspect the complete surface gallery.** A deterministic traversal opens, navigates and dismisses every case. Check labels and value alignment, focus visibility, error exposure, and no leftover decorative mastheads or brown default-theme islands.
- [ ] **6. Commit.** `git commit -m "feat(tui): migrate remaining terminal surfaces to reference UI"`.

## Task 8: Make Background Work and Submission Targets Real

**Files:** New `TUI/workspace.rs`; `TUI/{model.rs,app.rs}`; new `HOST/davinci_interactive/{task_router.rs,view_events.rs}`; targeted integration in `HOST/davinci_interactive.rs`. Inspect and reuse the existing graph task/worktree adapters before changing any agent/protocol schema.

**Consumes:** Runtime task/run/session identities and authoritative snapshots; Task 3 focus; existing cancellation/approval boundaries.

**Produces:** Task-scoped drafts, commands and events. Proposed types:

```rust
#[derive(Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub struct TaskKey { pub session_id: String, pub run_id: String }
#[derive(Clone, Debug)]
pub struct EventStamp { pub generation: u64, pub sequence: u64 }
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DeliveryTarget { Conversation(String), Run(TaskKey) }
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EventDisposition { Apply, Duplicate, BufferGap, IgnoreOldGeneration }
```

`classify_event(current: &EventStamp, incoming: &EventStamp) -> EventDisposition` handles deltas within the current generation. Generation changes are accepted only through an authoritative snapshot handshake. A new-generation delta is buffered pending that snapshot, never applied speculatively. Draft storage carries the entire existing `Composer` and attachment/editor metadata for each target, not just its string.

- [ ] **1. Write red event and routing tests.**

```rust
#[test]
fn reference_event_gap_requires_reconciliation() {
    let current = EventStamp { generation: 2, sequence: 4 };
    assert_eq!(classify_event(&current, &EventStamp { generation: 2, sequence: 5 }), EventDisposition::Apply);
    assert_eq!(classify_event(&current, &EventStamp { generation: 2, sequence: 4 }), EventDisposition::Duplicate);
    assert_eq!(classify_event(&current, &EventStamp { generation: 2, sequence: 7 }), EventDisposition::BufferGap);
    assert_eq!(classify_event(&current, &EventStamp { generation: 1, sequence: 99 }), EventDisposition::IgnoreOldGeneration);
}
```

Also drive two fake workers concurrently: stop A while B continues; deliver A's delayed completion after switching to B; expire A's approval; reconnect with a missing output chunk; confirm every draft and task state remains scoped.

- [ ] **2. Run red.** Targeted host/TUI tests. Test actual submission and event-adapter entry points, not only the pure ordering helper.
- [ ] **3. Implement event classification, bounded gap buffering and snapshot reconciliation.**

```rust
if incoming.generation < current.generation { EventDisposition::IgnoreOldGeneration }
else if incoming.generation > current.generation { EventDisposition::BufferGap }
else if incoming.sequence <= current.sequence { EventDisposition::Duplicate }
else if current.sequence.checked_add(1) == Some(incoming.sequence) { EventDisposition::Apply }
else { EventDisposition::BufferGap }
```

Do not discard earlier missing output just because a later sequence arrived. Bound buffers and request a fresh authoritative snapshot on overflow. If the current transport already has sequence/generation data, adapt it rather than inventing a second clock. Persist restart/recovery metadata with the existing storage contract; version any necessary protocol change and test both sides.

- [ ] **4. Wire explicit submission/background actions.** Ctrl+B follows the captured reference task behavior; ordinary prompt submission stays in the current conversation. Selecting an agent inspects only. Explicit send-to-run/another-conversation actions show their target and retrieve its draft. Preserve independent cancellation tokens, channels and permission requests; do not reuse one global `running` flag for all jobs.
- [ ] **5. Enforce real worktree isolation and run green.** Reuse existing runtime worktree support for concurrent editing. Never move the user's original checkout under an active task. When isolation cannot be established, block the conflicting launch with a visible explanation, not a fake running row. Inject launch, Git and reconnect failures. Run host/TUI suites and affected protocol/agent suites.
- [ ] **6. Commit.** `git commit -m "feat(runtime): isolate background task input events and lifecycle"`.

## Task 9: Integrate the Optional Readable Graph

**Files:** All graph files in the map; shell region/focus integration; runtime graph-to-view adapter; graph fixtures/tests.

**Consumes:** Task 3 regions/focus, Task 8 task state and real graph snapshot fields.

**Produces:** `GraphPresentation::{Hidden,Activity,Docked,Focused}` and a graph view model that keeps selection, viewport and follow-live independent from task lifecycle and composer focus. Default is `Hidden` with compact background status when relevant.

- [ ] **1. Write failing behavior cases.** Start a run on the conversation and assert the canvas does not force open. Type while it runs; open the graph; inspect a node; return focus to input; close the graph and confirm the run is still active. Resize across dock/list/focused breakpoints; preserve draft/cursor/attachments, selected node and viewport.
- [ ] **2. Run red.** Existing graph tests plus named `reference_graph_` regressions. Include hit-testing after scroll, inspector resize and Unicode titles.
- [ ] **3. Implement meaningful nodes and inspector content.** Use real task titles and explicit running/waiting/blocked/failed/completed states. Show exact current command, output, files, branch/worktree and waiting reason when supplied. Keep IDs, token numbers and contracts secondary. Missing Git context is explicitly absent, not synthesized. Distinguish workers finishing from verification/review and final success.
- [ ] **4. Reuse and improve dependency layout.** Stable ordering keyed by IDs; clear dependency direction; collapsible finished work; filters and keyboard equivalents for clicks. Keep the existing layout/navigation modules where their geometry is valid rather than rebuilding unrelated graph execution. Store follow-live separately; user navigation disables automatic viewport movement until explicitly resumed. Frame geometry and hit regions are generated once and shared by render and input.
- [ ] **5. Run green and graph-specific evaluation.** Replay wide/narrow, empty, cyclic/invalid, large and changing graphs. Validate a 1,000-node fixture without rendering every offscreen inspector detail; record timings on the tested host. Independently review whether a new user can identify current work, blockers and the next action. Graph usability never substitutes for the whole-terminal reference gate.
- [ ] **6. Commit.** `git commit -m "feat(tui): integrate optional readable graph with live input"`.

## Task 10: Migrate Defaults and Complete Release Verification

**Files:** `HOST/davinci_interactive.rs` theme/configuration integration and `TUI/theme.rs`; `README.md`; `docs/ui/design.md`; new `docs/ui/terminal-parity-report.md`; offline replay fixtures and tests.

**Consumes:** All preceding tasks and baseline configuration/session fixtures.

**Produces:** Safe default-theme upgrade, complete evidence report, reviewed branch and reproducible build/restart/rollback instructions.

- [ ] **1. Add migration regression tests.** Cover absent theme, shipped brown preset, explicit custom theme, unknown JSON fields, malformed config and read-only storage. Capture permissions, telemetry, trust, credentials and execution defaults before/after; assert they are unchanged. A failed migration must not truncate the original file. Preserve rollback data and do not infer explicit user intent solely from a color value.
- [ ] **2. Run red, then implement the smallest versioned migration.** Change only reference appearance defaults/legacy preset mapping through the existing settings owner. Use atomic writes and existing configuration locking. Do not recursively rewrite user theme files or session transcripts.
- [ ] **3. Run the complete offline verification sequence.**

```sh
cargo fmt --all -- --check
cargo check --locked --workspace --all-targets
cargo test --locked -p davinci-tui
cargo test --locked -p davinci-coding-agent
cargo test --locked --workspace
cargo clippy --locked --workspace --all-targets -- -D warnings
```

Discover the workspace's existing evaluation commands from its manifests/docs and run the affected offline suites in addition to these commands. Never trigger paid live-provider evaluations without authorization. Save command, working directory, commit, exit code and test counts. Fix failures; do not mass-update expected snapshots to erase reference differences.

- [ ] **4. Run reference and interaction gates.** Full matrix and gallery; cell/style comparison with zero unexplained static differences; recorded animation sequences; model/settings identity/persistence; permission cancellation; event replay; two-task independent cancellation; safe rendering and copy. Mark missing reference evidence blocked, not passed.
- [ ] **5. Run native manual gates and independent review.** Native Windows Terminal/ConPTY and one POSIX terminal, both renderers: input, IME, paste, mouse, copy/select, scroll, resize, interruption, reconnect and terminal restoration. Run in a disposable fixture checkout without restarting the user's harness. A reviewer who did not implement the branch compares all core surfaces to the reference, and reviews graph usability separately. If no independent reviewer is available, record the missing gate rather than inventing a review.
- [ ] **6. Final commit and handoff.** Report only what actually ran and which surfaces have verified evidence. Rebase/fetch only with the repository's safety rules and resolve conflicts before final verification. Push the task branch and open the review PR; do not merge it. Derive exact build/start commands from Cargo metadata and the validated native executable, and include them in the report. Do not guess a binary path or automatically restart any service. Retain the previous binary/configuration backup for rollback.

## Plan Self-Review and Approval Handoff

Coverage: spec sections 1–3 map to Tasks 1–7; section 4 to Tasks 3–6 and 8; section 5 to Tasks 8–9; section 6 to Tasks 2–3, 8 and 10; section 7 to Tasks 1 and 10. All 28 screens and five overlays have explicit owners. Five review-focus failure modes each have assigned tests. Capture-driven values are obtained in Task 1, not silently approximated in later tasks.

Recommended execution: **native, sequential execution in an isolated worktree**, since theme, frame geometry and focus are shared dependencies and the currently connected tools do not expose an independent builder/reviewer fleet. A final independent review remains a release requirement. Subagent-driven execution is an alternative in an environment that actually provides isolated workers and reviewers; do not simulate separate agents inside one context.

Before product changes: review this plan and select the execution method. A connected Rust-capable host and access to the pinned reference CLI are also needed. The approved spec remains unchanged; this is not a request to redesign it.

## Reference Research Record

Official sources reviewed during planning:

- https://github.com/anthropics/claude-code
- https://github.com/anthropics/claude-code/releases/tag/v2.1.278
- https://code.claude.com/docs/en/interactive-mode
- https://code.claude.com/docs/en/fullscreen
- https://code.claude.com/docs/en/model-config
- https://code.claude.com/docs/en/settings

Image searches for terminal settings/config and model selection returned older model-picker examples, not a complete pinned-version settings/model capture set. Those examples are not acceptance evidence. The repository's demo and public screenshots help identify states to capture; only versioned native captures under matched conditions establish 1:1 parity. Current documentation is behavioral reference material and must be checked against the pinned executable when it differs.
