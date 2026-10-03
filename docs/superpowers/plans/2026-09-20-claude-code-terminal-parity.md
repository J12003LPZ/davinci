# DaVinci Whole-Terminal Claude Code Parity Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rebuild every DaVinci terminal surface to match the pinned Claude Code terminal reference, retaining real DaVinci capabilities and the separately approved optional graph.

**Architecture:** Keep Rust, Ratatui, Crossterm, and the execution engine. Replace shared presentation primitives and input ownership first; migrate every screen and overlay to those primitives. Put target-aware task routing behind a host adapter, then integrate the graph without taking the editor away.

**Tech Stack:** Existing workspace Rust 2021 / repository-pinned Rust 1.83.0 (rustfmt and clippy), Ratatui 0.29.0, Crossterm 0.28.1, serde/serde_json, unicode-segmentation 1.12.0, unicode-width 0.2.0. Do not upgrade dependencies as a side effect of this work.

**Spec:** `docs/superpowers/specs/2026-09-20-claude-code-terminal-parity-design.md` (approved in the conversation).

**Branch:** `J12003LPZ/terminal-ui-rebuild-20260920`

**Inspected branch head:** `953fbc7ca254c429ee7c27645ca1f8179d2cb72e`.

**Execution status:** Plan for review, not a completed implementation. No Rust code in this document has been compiled. No reference captures or product tests are claimed as passing.

## Global Constraints

- Primary requirement: "Rebuild **the entire DaVinci terminal UI as a 1:1 visual and interaction match to Claude Code**."
- Reference: "Claude Code v2.1.278"; native terminal CLI, not Desktop, browser, or VS Code.
- Primary comparison: explicitly configured fullscreen Windows Terminal profile; classic/regular rendering is a separate profile.
- "The earlier illustrative chat layout is not a visual reference."
- "Static regions require zero unexplained cell/style differences."
- "40x12 graceful fallback; 80x24; 100x30; 120x40; 160x50; dark and light reference profiles; reduced-color and no-color fallbacks."
- "Graph execution does not force open a canvas or remove the prompt."
- "Selecting a node only inspects it; sending to an agent requires an explicit action."
- "Never change permissions, telemetry consent, project trust, credentials or execution defaults merely to match a screen."
- "Provider/tool output is untrusted text: preserve secret redaction and sanitize control sequences before display or copy."
- "Keep screenshots and recordings outside source control; commit the manifest, textual fixtures, capture procedure, and comparison report."
- "Do not merge into `main` or restart the user's running harness without authorization."
- All ordinary UI tests and event-replay evaluations are offline. Do not invoke paid model evaluation workflows to validate a renderer.
- Preserve names, capabilities, stored configuration, and identities belonging to DaVinci. Do not label another provider's model as Claude or copy account entitlements into fixtures.

## Review Focus

1. Reordering or removing a filtered provider/model row between selection and Enter must never select a different model. Task 8 owns the regression.
2. A permission response arriving after cancellation, expiry, or switching targets must never authorize the wrong task. Tasks 9 and 12 own the regressions.
3. Pasting wide/combining text, switching views, and resizing while events arrive must preserve the entire draft, caret, attachments, and editor mode. Tasks 2, 3, and 12 own the regressions.
4. Terminal escape sequences in tool output, file names, or extension messages must not write to the clipboard, change the terminal title, or masquerade as app controls. Tasks 2, 5, and 11 own the regressions.
5. A dropped update followed by reconnect, a late terminal event, or a paused graph must not change a different run or claim completion before authoritative verification. Tasks 12 and 13 own the regressions.

## Environment and evidence ledger

Verified in this planning pass:

- The task branch still points to the specification commit above. No product changes were found on that branch at the time of inspection.
- GitHub connector reads are available. The working container's `git ls-remote` failed with `Could not resolve host: github.com`.
- `cargo` and `rustc` were not found on this container's PATH. The repository pins Rust 1.83.0 in `rust-toolchain.toml`.
- Desktop Commander returned `No devices available`. No commands were run on the user's PC.
- Existing `.github/workflows/ci.yml` runs on pushes and pull requests, sets `PI_OFFLINE=1`, and includes native OS jobs. GitHub CI is a potential compilation/test route after source changes; it does not replace interactive Windows/reference capture. No CI success is claimed in this plan.
- Source inspection confirmed `app::handle_key` gives whole screens input ownership and intercepts old global shortcuts before readline; changing colors cannot fix this.
- `views/graph_run.rs::chrome` hides the composer. `app::panel` treats the graph as a takeover screen.
- The existing TUI README contains outdated view paths. Use `views/mod.rs` and `model::Screen` / `Overlay` as the inventory, and update the README during this change.

Image searches found historical screenshots of settings, the model selector, and the conversation. They are discovery evidence, not the pinned v2.1.278 capture set. Do not infer exact current colors or geometry from them. The official repository's demo is also not a substitute for a versioned, full-surface capture.

Reference locations:

- Official repository: https://github.com/anthropics/claude-code
- Pinned release: https://github.com/anthropics/claude-code/releases/tag/v2.1.278
- Interactive behavior: https://code.claude.com/docs/en/interactive-mode
- Fullscreen profiles: https://code.claude.com/docs/en/fullscreen
- Model selector behavior: https://code.claude.com/docs/en/model-config
- Commands: https://code.claude.com/docs/en/commands
- Settings scope: https://code.claude.com/docs/en/settings
- Historical model screenshot (visibly v2.1.32): https://x.com/oikon48/status/2019496879575404998
- Historical settings screenshot: https://henriquesd.medium.com/claude-code-a-practical-guide-to-automating-your-development-workflow-675714db08ed
- Historical conversation screenshot (visibly v2.1.45): https://deepakness.com/raw/claude-sonnet-4-6/

Do not treat bug-report prose or historical screenshots as current product documentation. Verify action traces against the pinned executable; record documentation disagreements instead of silently changing the target.

## Repository map and ownership

All paths below are repository-relative. `TUI` in prose means `crates/davinci-tui/src/davinci`; `HOST` means `crates/davinci-coding-agent/src/davinci_interactive.rs`. These are abbreviations for discussion, not literal directories in commands.

| Boundary | Existing files | New files planned | Responsibility |
| --- | --- | --- | --- |
| Reference tests | `crates/davinci-tui/src/davinci/fixtures.rs` | `crates/davinci-tui/tests/terminal_parity.rs`, `crates/davinci-tui/tests/terminal_parity/support.rs`, `crates/davinci-tui/tests/fixtures/terminal-reference/manifest.json` | Versioned fixtures, cell/style comparison, action traces |
| Visual primitives | `crates/davinci-tui/src/davinci/{theme.rs,ui.rs}` | `crates/davinci-tui/src/davinci/reference_tokens.rs` | Measured palette, geometry, grapheme-safe rows, dialogs, selection |
| Shell and focus | `crates/davinci-tui/src/davinci/{app.rs,model.rs,runtime.rs,term.rs}` | `crates/davinci-tui/src/davinci/{focus.rs,shell.rs}` | Region allocation, view lifecycle, focus; no execution |
| Conversation | `crates/davinci-tui/src/davinci/views/{startup.rs,chrome.rs,transcript.rs,markdown.rs,highlight.rs,semantic.rs,opera.rs,studio.rs}` | `crates/davinci-tui/tests/terminal_parity/conversation.rs` | Welcome, prompt, messages, tools, streaming |
| Pickers | `crates/davinci-tui/src/davinci/views/{completion.rs,instrumenta.rs,settings.rs,cogitator.rs,thinking.rs,resume.rs}` | `crates/davinci-tui/src/davinci/selection.rs` | Stable selection IDs and shared reference navigation |
| Safety | `crates/davinci-tui/src/davinci/views/{approval_modal.rs,decision_modal.rs,ask.rs,permissions.rs,trust.rs,login.rs,secret_input.rs}` | `crates/davinci-tui/tests/terminal_parity/safety.rs` | Dialog presentation without changing authority |
| Runtime adapter | `crates/davinci-coding-agent/src/davinci_interactive.rs`, `crates/davinci-coding-agent/src/davinci_interactive/{graph_feedback.rs,graph_setup.rs}` | `crates/davinci-coding-agent/src/davinci_interactive/task_routing.rs`, `crates/davinci-tui/src/davinci/workspace.rs` | Per-target drafts, per-run events, explicit controls |
| Graph | `crates/davinci-tui/src/davinci/views/{grafo.rs,graph_canvas.rs,graph_inspector.rs,graph_layout.rs,graph_nav.rs,graph_perf.rs,graph_run.rs}` | `crates/davinci-tui/tests/terminal_parity/graph.rs` | Optional graph presentation and hit testing |
| Other surfaces | Every remaining module in `crates/davinci-tui/src/davinci/views/mod.rs` | `crates/davinci-tui/tests/terminal_parity/surfaces.rs` | Full inventory coverage, including extension and voice states |
| Release | `.github/workflows/ci.yml`, `crates/davinci-tui/README.md`, `docs/ui/design.md` | `docs/ui/terminal-parity-report.md`, `docs/ui/terminal-parity-capture.md` | Regression gate, evidence, accurate build/restart instructions |

Do not turn `davinci_interactive.rs` into a wholesale refactor. Move only new target/event routing into the named submodule and retain tested execution paths.

## Task 1: Establish the reference corpus and comparison gate

**Files:** Create the reference-test and documentation files listed in the map. Inspect the existing examples under `crates/davinci-tui/examples` before adding a new capture runner.

**Consumes:** Pinned CLI, separate test account/profile where necessary, known terminal settings, existing `Model` and `app::compose_frame`.

**Produces:** A manifest with one record per reference state; cell snapshots and explicit input traces; `CellSnapshot`, `FrameSnapshot`, `compare_frames`, and fixture helpers in test support. A frame stores width, height, cursor position, and a row-major array of symbol/foreground/background/modifier values. A missing capture is an error, not an empty snapshot.

- [ ] Create an isolated worktree from the existing remote task branch, after verifying its latest head. Do not branch again from `main` and lose the approved spec. Fetch, check `git worktree list`, and attach the branch only when it is not in use by another worktree. Read all applicable `AGENTS.md` files in the checkout.
- [ ] Record `rustc --version`, `cargo --version`, `git rev-parse HEAD`, and `claude --version`. Stop reference capture unless the executable reports exactly `2.1.278`. Use a dedicated capture configuration; never read/export the user's credentials.
- [ ] Run the unchanged baseline before writing implementation code:

```sh
cargo fmt --all -- --check
cargo test -p davinci-tui --locked
cargo test -p davinci-coding-agent --locked
```

- [ ] Write the comparator tests first. The production comparison must examine styles even where variable identity text is normalized:

```rust
#[test]
fn style_difference_is_not_normalized_away() {
    let a = FrameSnapshot::single("x", "default", "default", 0);
    let mut b = a.clone();
    b.cells[0].modifiers = 1;
    assert_eq!(compare_frames(&a, &b).len(), 1);
}
#[test]
fn geometry_mismatch_is_a_failure() {
    let a = FrameSnapshot::single("x", "default", "default", 0);
    let mut b = a.clone();
    b.width = 2;
    assert!(!compare_frames(&a, &b).is_empty());
}
```

`FrameSnapshot::single(symbol: &str, fg: &str, bg: &str, modifiers: u16) -> FrameSnapshot` makes a 1x1 frame with no cursor. `compare_frames(&FrameSnapshot, &FrameSnapshot) -> Vec<String>` returns coordinate-specific differences and first rejects dimensions or cell-count mismatch. Derive `Clone`, `Debug`, `PartialEq`, `Eq`, `Serialize`, and `Deserialize` for snapshot structs. Serialize colors as `default`, `ansi:<index>`, or `rgb:<hex>` so terminal defaults are not guessed as RGB.

- [ ] Run `cargo test -p davinci-tui --test terminal_parity --locked` and record the initial failure. Implement the comparator with exact field equality, not a perceptual threshold. Test missing files, duplicate case IDs, unknown profile names, and invalid cell counts as errors.
- [ ] Capture every equivalent surface in the spec: idle/startup, returning session, normal/streaming/tool/error conversation, editor and autocomplete, settings/config and themes, model/effort, permission/question/trust, auth, resume/history/rewind, diff, tasks, help, and applicable utility dialogs. Capture hidden/expanded/loading/error states too. Use deterministic offline DaVinci fixtures matching those states; account-dependent reference states require existing authorized evidence, not a model call silently billed to the user.
- [ ] For each manifest record store `case_id`, `reference_version`, OS/terminal/font identifiers, cell size, viewport, theme, renderer, setup, key/mouse trace, reference evidence location, textual grid location, and named allowed identity substitutions. Substitutions must preserve coordinates and styling; use same-width fixture values rather than masking whole rows.
- [ ] Implement test-only `model_at(width: u16, height: u16) -> Model`, `case_model(id: &str, width: u16, height: u16) -> Result<Model, String>`, and `assert_case(id: &str, model: &Model)`. Manifest lookup must include case ID, viewport size, theme and renderer; one golden file must not be reused across different dimensions. `assert_case` rasterizes the production composed rows through Ratatui's `TestBackend`, compares cells and cursor, and fails on missing evidence. `case_model` must reject unknown IDs and supply real typed fixtures, never return the same empty model for different screens.
- [ ] Rerun comparator tests. Commit only manifest, test code, sanitized textual fixtures, and capture procedure. Keep raw captures outside Git. Do not mark the corpus complete while any required state lacks a capture.

## Task 2: Replace theme and row primitives across the whole app

**Files:** Modify `crates/davinci-tui/src/davinci/{theme.rs,ui.rs,mod.rs}`. Create `crates/davinci-tui/src/davinci/reference_tokens.rs`. Add `primitives.rs` under the new test directory.

**Consumes:** Task 1 measurements. **Produces:** `ReferenceProfile::{Dark, Light}` and `Theme::reference(profile: ReferenceProfile, depth: ColorDepth, no_color: bool) -> Theme`; existing row/surface APIs backed by the reference design.

- [ ] Add failing tests for captured palette roles, straight separators, plain section labels, selection/current distinction, and Unicode clipping. Preserve a grapheme as a unit:

```rust
#[test]
fn clipping_preserves_combining_and_wide_graphemes() {
    assert_eq!(ui::clip("e\u{301}x", 1), "e\u{301}");
    assert_eq!(ui::clip("界x", 1), "");
    assert_eq!(ui::clip("界x", 2), "界");
    assert_eq!(ui::clip("abc", 0), "");
}
```

- [ ] Run the primitive tests and verify the failing case before replacing the implementation.
- [ ] Replace character-at-a-time clipping with the already-declared Unicode dependencies:

```rust
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;
let mut used = 0usize;
let mut output = String::new();
for grapheme in text.graphemes(true) {
    let cells = UnicodeWidthStr::width(grapheme);
    if used.saturating_add(cells) > usize::from(max) { break; }
    output.push_str(grapheme);
    used += cells;
}
```

- [ ] Generate the reference token table from measured cells; do not approximate colors from web screenshots. Replace paper-label, textured-rule, crimson-selection, and fixed-74-column assumptions in shared primitives. Retain their APIs during migration where possible so every caller changes together. Let default terminal foreground/background remain default when that is what the reference emits.
- [ ] Route custom themes through the same geometry. Remove color-based theme identity inference from layout decisions. Verify contrast and status labels in reduced/no-color profiles; do not rely on hue alone.
- [ ] Run primitive tests and compare representative dialog, transcript, and picker snapshots. Commit source plus tests together.

## Task 3: Rebuild shell layout and explicit input ownership

**Files:** Modify `crates/davinci-tui/src/davinci/{app.rs,model.rs,runtime.rs,term.rs}` and shared `crates/davinci-tui/src/interaction.rs`; create `focus.rs`, `shell.rs`, and `tests/terminal_parity/editor.rs`.

**Consumes:** Shared reference tokens and the mature `Editor`. **Produces:** `FocusOwner::{Composer, Completion, Dialog, Transcript, Graph}`; a shell frame containing the actual content, composer and panel rectangles; one routing decision shared by keyboard and mouse. Retain `app::compose_frame` as the public rendering entry point.

- [ ] Add a regression for readline keys currently intercepted by old global views:

```rust
#[test]
fn ctrl_u_edits_input_instead_of_opening_usage() {
    let mut m = model_at(100, 30);
    m.composer.set_text("draft");
    let flow = app::handle_key(&mut m,
        KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
    assert_eq!(flow, Flow::Continue);
    assert_eq!(m.screen, Screen::Agent);
    assert_eq!(m.composer.editor().get_text(), "");
}
```

- [ ] Add tests for Ctrl+D with and without text, Ctrl+R, multiline Up/Down, undo, Alt+P draft retention, Shift+Enter/Ctrl+J, paste bursts, literal `?` in nonempty input, permission cancellation, and Unicode resizes. The exact expected transitions come from Task 1's trace, not the old shortcut table.
- [ ] Run the editor tests before changes. Replace the `screen != Agent` ownership shortcut with explicit focused-region ownership. A modal receives keys before global interrupts; the permission host still validates all decisions.
- [ ] Compute reserved editor/hint rows first, then allocate remaining rows. Use saturating arithmetic. A transient dialog may capture typing while visible, but it must retain the full draft and restore the previous focus on close. Normal graph visibility must not imply graph keyboard focus.
- [ ] Have the frame's rectangles drive both rendering and hit testing. Do not maintain separate magic row offsets for graph/microphone/picker clicks. Reserve graceful input/error behavior when the terminal is too small for a panel.
- [ ] Run tests at all five specified sizes and repeat shrink/grow cycles. Commit the shell and routing change independently before migrating screen contents.

## Task 4: Match startup, transcript, markdown, and editor presentation

**Files:** Modify `crates/davinci-tui/src/davinci/views/{startup.rs,chrome.rs,transcript.rs,markdown.rs,highlight.rs,semantic.rs}`; add conversation tests and fixtures.

**Consumes:** Reference shell and actual typed `Entry` values. **Produces:** Claude-equivalent main conversation, welcome, prompt, attachments, markdown, code blocks, links, and status hints with DaVinci identity.

- [ ] Add failing snapshot cases and a concrete multiline fixture:

```rust
#[test]
fn conversation_reference_has_no_legacy_chrome() {
    let mut m = model_at(100, 30);
    m.transcript = vec![Entry::user("Explain this function."),
        Entry::prose("A paragraph.\n\n```rust\nlet value = 1;\n```")];
    m.composer.set_text("Follow-up\nwith two lines");
    assert_case("conversation.multiline-code", &m);
}
```

- [ ] Run conversation tests before the change. Rebuild startup and prompt layout using measured line hierarchy and spacing. Do not carry over the invented cards, always-visible branch header, or permanent target selector from the superseded chat sketch.
- [ ] Render user/assistant messages, list indentation, tables, inline code, fences and links through the shared tokens. Preserve markdown parser behavior, preserve copyable code whitespace, and use viewport width instead of a universal 74-column prose cap.
- [ ] Reuse `Editor` caret/selection/paste state. Do not reconstruct the draft from rendered text. Render attachment chips from actual attachments and show overflow explicitly.
- [ ] Bound visible-row work and preserve scroll position during streaming. Compare classic and fullscreen cases separately. Rerun snapshots plus existing markdown/editor tests; commit.

## Task 5: Match live tool execution, output expansion, and errors

**Files:** Modify `crates/davinci-tui/src/davinci/views/{opera.rs,transcript.rs,studio.rs}`, `crates/davinci-tui/src/davinci/model.rs`, and host event-to-Entry adaptation in `davinci_interactive.rs`.

**Consumes:** Real tool events and stored output; reference activity traces. **Produces:** Truthful summary/command/output states, per-call expansion, animation, completion, interruption, and errors.

- [ ] Add a failing tool-output fixture before changing rendering:

```rust
#[test]
fn failed_tool_keeps_error_visible_when_collapsed() {
    let mut m = model_at(100, 30);
    m.transcript.push(Entry::tool(State::Failed, "Bash",
        "cargo test --workspace", Some("1s"))
        .with_output("error: regression failed"));
    assert_case("tool.failed-collapsed", &m);
}
```

- [ ] Test success, running, failure, cancellation, long output, exact command preservation and expansion during incoming output. Test an OSC clipboard sequence and cursor-positioning sequence in output: neither may execute or reach copy/export as active escapes.
- [ ] Keep stable call identities in presentation state. Expansion attaches to a call ID, not a rendered row or vector offset. Summaries use actual instrument, target and result; never infer success from missing stderr or from the spinner stopping.
- [ ] Sanitize at the adapter/display boundary using the repository's existing ANSI parsing and redaction facilities. Retain allowable styled text structurally; do not pass untrusted raw control sequences into terminal output.
- [ ] If full output was pruned by the existing storage cap, show that fact. Do not call twelve retained lines the full result. Add bounded retrieval from the existing transcript/output store only where available, without tool execution from views.
- [ ] Compare animation sequences using deterministic ticks, rerun old transcript tests and new cases, then commit.

## Task 6: Match commands, autocomplete, and help

**Files:** Modify `crates/davinci-tui/src/davinci/views/{completion.rs,instrumenta.rs,keys.rs}`, `crates/davinci-tui/src/autocomplete.rs`, `app.rs`, and the existing host command registry.

**Consumes:** Existing DaVinci commands and reference action traces. **Produces:** Reference-style `/`, `@`, command palette and help, with no dead displayed shortcuts.

- [ ] Test accepting completion versus submitting text, dismissing without erasing the draft, an empty result, long descriptions, and Unicode search. Validate `/model` and its argument completion as distinct states.
- [ ] Add stable picker state in `selection.rs` with `visible_ids: Vec<String>` and `selected_id: Option<String>`. Filtering must carry the selected ID if it survives, otherwise choose the first visible ID or None:

```rust
let keep = selected_id.as_ref()
    .filter(|id| visible_ids.contains(id)).cloned();
selected_id = keep.or_else(|| visible_ids.first().cloned());
```

- [ ] Run the new picker tests before modifying commands. Preserve existing command names as aliases when adding reference-equivalent spellings such as `/config` for settings. An alias invokes the existing behavior, not a second implementation.
- [ ] Make displayed hints come from active bindings and actual capabilities. Unsupported DaVinci operations do not get decorative Claude controls. Prevent completion-list reordering from changing the accepted item.
- [ ] Rerun autocomplete tests, action traces and help snapshots; commit.

## Task 7: Rebuild settings and theme migration

**Files:** Modify `views/{settings.rs,sheet.rs}`, `model.rs`, `app.rs`, `theme.rs`, and `davinci_interactive.rs::cycle_setting` / settings-population call sites.

**Consumes:** Typed `SettingRow` keys, scope, values and validation; reference config capture. **Produces:** Actual reference settings layout/search/navigation, correct writes, and safe upgrade of shipped brown presets.

- [ ] Add tests for filtering, scrolling, unavailable values, validation failure, project/user scope and cancel. Capture settings with short and long terminals:

```rust
#[test]
fn settings_scope_and_value_remain_visible() {
    let mut m = model_at(100, 30);
    m.screen = Screen::Settings;
    m.settings_rows = vec![SettingRow {
        key: "autocompact".into(), label: "Auto-compact".into(),
        value: "true".into(), values: vec!["true".into(), "false".into()],
        description: "Compact context before it becomes too large.".into(),
        project: true, ..SettingRow::default()
    }];
    assert_case("settings.project-value", &m);
}
```

- [ ] Run before implementing. Match reference tabs, search and field geometry only where captured and supported. Populate status/usage from real DaVinci state; show unavailable data explicitly. Do not copy account/subscription values.
- [ ] Resolve edits by setting key and scope, not filtered index. Write through the existing settings owner only after validation. Keep an error inline without changing the saved value.
- [ ] Create an allowlisted migration from shipped legacy preset names to the new reference default. Preserve unknown keys, custom theme definitions, credentials and all security/telemetry choices. Save rollback metadata without exporting secrets. An explicit custom theme remains opt-in and is never silently overwritten.
- [ ] Use temporary configuration directories to test before/after JSON: only the approved appearance fields may differ. Rerun persistence and UI tests, then commit.

## Task 8: Match model, quick-switch, and reasoning selectors

**Files:** Modify `views/{cogitator.rs,thinking.rs}`, `selection.rs`, `model.rs`, `app.rs`, and host handling of model choices.

**Consumes:** Catalog rows with actual `provider`, `id`, capabilities, current model and supported reasoning levels. **Produces:** Reference picker geometry, current/focus distinction, correct filtered selection and supported effort adjustment.

- [ ] Add `ModelKey { provider: String, id: String }` with equality/hash and a `Choice::CatalogId(ModelKey)` action. Retain legacy index choices only while migrating their callers.
- [ ] Add the identity regression before implementation:

```rust
#[test]
fn same_model_label_in_two_providers_is_not_the_same_identity() {
    let a = ModelKey { provider: "provider-a".into(), id: "shared".into() };
    let b = ModelKey { provider: "provider-b".into(), id: "shared".into() };
    assert_ne!(a, b);
}
```

Also select the second ID, reverse catalog order, then press Enter: the host must still receive the selected ID. Remove that ID before Enter: the UI must require a fresh selection or show an error, never select the replacement row.

- [ ] Run the tests. Resolve `CatalogId` against the current catalog at the host boundary, revalidate availability/authentication, and keep the current model unchanged when resolution fails. Display names are labels, not identities.
- [ ] Use only the chosen provider's supported reasoning values. Preserve the input draft and caret while switching. Distinguish highlighted row from current-model checkmark in colors and no-color form.
- [ ] Match full picker and quick switch snapshots, including zero models, long IDs, auth failure, removed model and unsupported effort. Rerun host choice tests, then commit.

## Task 9: Match safety, questions, login, and credentials

**Files:** Modify `views/{approval_modal.rs,decision_modal.rs,permissions.rs,trust.rs,ask.rs,login.rs,secret_input.rs}` and focused routing. Preserve host `NativeApproval`, `wait_native_approval`, and `NativeDecision` authority.

**Consumes:** Existing request identity, expiry, reply channel and cancellation state. **Produces:** Matched dialogs that cannot grant anything through a layout operation.

- [ ] Add tests for Escape/No, explicit Yes, expiry, responder disconnect, close during resize, duplicate response, and a late Yes after cancellation. Assert the actual host reply is Deny/Cancelled where required, not just that a dialog disappeared.
- [ ] Keep identity-plus-liveness validation adjacent to reply dispatch:

```rust
if !pending.is_live() {
    return ToolApprovalDecision::Deny.into();
}
```

This uses the existing host `NativeApproval::is_live`; it is not permission logic for a renderer. The actual reply path must still check the matching request identity and recheck expiry at the boundary.

- [ ] Run existing approval/decision tests before UI changes. Migrate layout, tabs, comments, selected answers and errors to the measured dialog primitives. Preserve the existing bounded waits and abort guards.
- [ ] Keep masked secrets in `Zeroizing` storage and out of normal drafts, histories, debug output, captured frames and export. Test cancel and failed validation with a unique sentinel secret, asserting its absence from every public representation.
- [ ] Rerun safety and credential suites plus reference snapshots; commit.

## Task 10: Match sessions, history, rewind, and diffs

**Files:** Modify `views/{resume.rs,tree.rs,rewind.rs,diff.rs,codex.rs,studio.rs}`, relevant `model.rs` fields and host navigation adapters.

**Consumes:** Actual session/file/hunk identities and existing history/storage. **Produces:** Reference session and changes interaction; DaVinci's extra tree uses the same components.

- [ ] Add fixtures for search, empty history, long titles, duplicate session names, loading/error, rename/removal between select/Enter, and large diffs. Add one definite typed diff fixture:

```rust
let change = Entry::Delta {
    path: "src/session.rs".into(), adds: 1, dels: 1,
    hunks: vec![Hunk::new(HunkKind::Del, "old"),
                Hunk::new(HunkKind::Add, "new")],
};
```

- [ ] Run failed snapshots. Apply stable ID selection to sessions and file paths, preserving the inspected hunk and scroll anchor during refresh.
- [ ] Treat rewind and restore as explicit operations using the existing host authorization; browsing a preview never modifies files. Distinguish conversation navigation from destructive code restoration.
- [ ] Resolve Git branch/worktree from authoritative metadata. A detached HEAD or non-Git task must be labeled truthfully, not assigned the active foreground branch.
- [ ] Compare classic/fullscreen snapshots, test selected text copy without ANSI control bytes, and run session/diff suites. Commit.

## Task 11: Migrate every utility, extension, and voice surface

**Files:** Modify `views/{compact.rs,context_inspector.rs,memoria.rs,vectors.rs,mensura.rs,governor.rs,budget.rs,export.rs,securitas.rs,recovery.rs,mcp.rs,officina.rs,disegno.rs,task_board.rs,agents.rs,workflows.rs}` plus the voice setup/status render path in `app.rs` and shared chrome.

**Consumes:** Existing typed sheet data; matched components. **Produces:** No old-theme islands, including nested and non-enum states.

- [ ] Write an exhaustive screen coverage function with a `match` on every `Screen` variant and no wildcard. Match every `Overlay` variant the same way. A future enum addition must cause a compile error until its case is mapped.
- [ ] For each mapped fixture, use the same extent check alongside its reference/extension case:

```rust
for (width, height) in [(40, 12), (80, 24), (100, 30), (120, 40), (160, 50)] {
    let m = case_model(case_id, width, height).expect("mapped fixture");
    let frame = app::compose_frame(&m, height);
    assert_eq!(frame.lines.len(), usize::from(height));
    assert!(frame.lines.iter().all(|row| row.width() <= usize::from(width)));
    assert_case(case_id, &m);
}
```

- [ ] Run the new cases before migration. Replace individual decorative frames with shared dialogs/rows. Name a specific equivalent reference component for each DaVinci-only extension; do not label an invented screen as a literal Claude match.
- [ ] Sanitize extension titles, messages and copy output. Bound oversized extension widgets so they cannot push the editor out of the terminal. Test zero lines and thousands of lines.
- [ ] Voice setup, recording, failure and send-blocking continue to reflect actual state; redesign must not start the microphone. Test that opening a graph cannot bypass `voice.blocks_send`.
- [ ] Update the surface matrix to include all 28 screens, five overlays, nested dialogs, startup/voice states and extension positions. Rerun every mapped case; commit.

## Task 12: Add real per-target drafts and task-aware routing

**Files:** Create `crates/davinci-tui/src/davinci/workspace.rs` and `crates/davinci-coding-agent/src/davinci_interactive/task_routing.rs`; modify `model.rs`, `app.rs`, host and its graph setup/feedback integration.

**Consumes:** Existing run/session identities and lifecycle snapshots; existing Editor/attachment representations. **Produces:** Explicit destination actions, independent drafts and task streams, correctly scoped pause/stop, and offline event-replay tests.

- [ ] Define `TargetId(String)`, `RunId(String)` and `EventEnvelope { run_id: RunId, epoch: u64, sequence: u64, event: TaskEvent }`. `TaskEvent` has variants `Snapshot(GraphRunSheet)`, `Output { task_id: String, entry: Entry }`, and `ControlAcknowledged { request_id: String, lifecycle: String }`, using the existing typed UI snapshot and transcript representations. Extend the event enum only for an actual host event needed by the interaction tests; never invent success or progress. This is an adapter around the existing execution engine. `Draft` owns the entire existing `Composer` plus its associated attachment/editor-mode state, not just a String.
- [ ] Define `WorkspaceState::switch_target(&mut self, TargetId)` and `apply_event(&mut self, EventEnvelope) -> EventDisposition`, where disposition is `Applied`, `Duplicate`, `StaleEpoch` or `NeedsSnapshot`. A missing sequence triggers reconciliation rather than silently dropping progress.
- [ ] Write regressions before implementation: two targets with different drafts and carets; interleaved output from two runs; stop A while B continues; duplicate/out-of-order terminal events; missing sequence; new reconnect epoch; expired approval on an inactive target.
- [ ] Route incoming events by run ID before touching visible state. Reject stale epochs; deduplicate sequence numbers within an epoch; reconcile a gap using the authoritative snapshot before accepting incremental events again. Terminal success comes from the run lifecycle, not all worker rows becoming Done.
- [ ] Swap complete draft objects on target switches. Never clear or serialize the source draft through the rendered prompt. Selecting a node does not call `switch_target`. An explicit send/target action is required and gets a temporary visible destination indication.
- [ ] Keep run controls pending until acknowledged. Pause requested is not paused; stop requested is not stopped. Queue acknowledgements and failures display against their own run.
- [ ] Use the existing isolated worktree machinery for concurrent editing. Refuse or serialize tasks that would otherwise write the same checkout; never silently launch a second writer into the user's foreground worktree. Do not copy credentials or shared writable databases into independent tasks.
- [ ] Record the scenarios as local event-replay fixtures and execute them with no network/model calls. Commit the routing adapter and tests before graph layout changes.

## Task 13: Integrate the optional readable graph

**Files:** Modify all named `graph_*` files, `grafo.rs`, shell allocation, `workspace.rs`, and graph entry-point behavior.

**Consumes:** Authoritative task snapshots, explicit focus and target routing. **Produces:** Optional compact status, activity list, docked/focused graph and inspector with a usable editor.

- [ ] Add the primary regression before touching graph chrome:

```rust
#[test]
fn graph_visibility_does_not_hide_the_draft() {
    let mut m = model_at(120, 40);
    fixtures::dress_screen(&mut m, "5a");
    m.composer.set_text("Draft must remain visible");
    let frame = app::compose_frame(&m, 40);
    let text = frame.lines.iter().map(ToString::to_string)
        .collect::<Vec<_>>().join("\n");
    assert!(text.contains("Draft must remain visible"));
}
```

- [ ] Add mouse/keyboard equivalence, close-without-stop, editor-focused letter keys, inspector scroll retention, collapse completed group, unknown branch, 40x12 fallback, running verification after all workers finish, and stable positions under incremental updates.
- [ ] Stop entering `Screen::GraphRun` automatically when a job starts. Keep the normal conversation default and expose the graph through an explicit action. Replace `Composer::Hidden` with the shell's real allocated editor region; simply changing that enum alone is not sufficient.
- [ ] Retain dependency-based layout, using stable task IDs for placement and meaningful titles for display. Show state/current action first; move IDs, contracts and token counters to details. Keep crossing edges, clipping and selected-node visibility under regression tests.
- [ ] Dock only when usable widths remain for the conversation, graph and inspector. At narrower sizes use the readable list/focused panel above the reserved editor. Draw and hit-test from the same frame rectangles.
- [ ] Inspector fields display actual command/output, files, branch/worktree, owner, waiting reason and control state. Worker Done plus active verification is still a running workflow.
- [ ] Rerun graph navigation, performance, layout, routing and full-terminal tests. Review graph readability separately from Claude parity; commit.

## Task 14: Full verification, migration review, and release evidence

**Files:** Complete `docs/ui/{terminal-parity-report.md,terminal-parity-capture.md,design.md}`, `crates/davinci-tui/README.md`, reference test inventory and relevant `.github/workflows/ci.yml` checks.

**Consumes:** All implemented tasks and capture/replay results. **Produces:** Evidence-backed branch ready for human review; no automatic merge or restart.

- [ ] Confirm the new appearance is the default for new installs and supported legacy presets. Reopen saved sessions and custom theme/config files from before the change; verify no permission, trust, telemetry or credential mutation.
- [ ] Run all gates in the real build environment, capturing exit codes and logs:

```sh
cargo fmt --all -- --check
cargo check --workspace --all-targets --locked
cargo test -p davinci-tui --locked
cargo test -p davinci-coding-agent --locked
cargo test --workspace --locked
cargo clippy --workspace --all-targets --locked -- -D warnings
```

The commands above are planned, not executed here. Validate existing workspace test prerequisites rather than disabling failing suites. UI replay remains offline even if other repository workflows offer paid evaluations.

- [ ] Run reference comparisons for every required equivalent surface and named extension case, in every matrix profile. A missing fixture, unidentified normalization, unexplained cell/style delta or incomplete inventory is a failure. Keep animation sequence evidence separate from still-frame evidence.
- [ ] Exercise both renderers on native Windows Terminal: ConPTY redraw, rapid resize, mouse, selection/copy, paste, Ctrl+C, permission cancellation, background task switching, reconnect and exit restoration. Smoke-test a POSIX terminal too. A Linux CI success is not evidence of Windows interaction correctness.
- [ ] Obtain an independent whole-branch review and a human side-by-side comparison against the pinned CLI. Do not call a self-review an independent review. Record the reviewer and actual findings. Resolve findings before final readiness.
- [ ] Check `git diff --check` and the staged diff for secrets, captures, font files, unrelated changes and unsafe workflow permissions. Keep large artifacts external. Commit tests, evidence manifest and documentation with the code they validate.
- [ ] Reconcile the task branch with the current remote base without overwriting another contributor. Push only the task branch, open a pull request after the gates pass, and do not merge it.
- [ ] Derive exact build/launch commands from the checked-in binary target and validate them. Provide non-destructive checkout/build instructions. Do not replace a running installed executable or terminate active agents during testing.

## Spec coverage and plan review

| Approved spec section | Owning tasks |
| --- | --- |
| 1: Whole-terminal priority and allowed differences | 2-11, 14 |
| 2: Fixed reference and evidence | 1, 14 |
| 3: Complete surface contract and inventory | 4-11, 13, 14 |
| 4: Interaction parity and correct configuration identities | 3, 5-10, 12 |
| 5: Optional graph and real multitasking | 12, 13 |
| 6: Architecture, safe migration, untrusted output | 2, 3, 5, 7-12 |
| 7: Reference/rendering/interaction/execution/manual gates | 1-14 |
| 8: Reviewed implementation plan before execution | This document |

Execution recommendation: native implementation in an isolated worktree, keeping tightly coupled shell/routing changes under one owner; independent whole-branch review when a reviewer is available. Parallelize read-only audits or genuinely disjoint view migrations only after shared interfaces pass. This ChatGPT session has no connected development device and has not spawned subagents.

Review points: the entire terminal is the primary deliverable; the graph cannot substitute for it; reference capture precedes hard-coded visual tokens; runtime behavior is preserved except for the targeted multitasking requirements. The next execution prerequisite is an accessible terminal environment able to run the pinned reference CLI and capture the required Windows interactions; Rust compilation can run in that environment or the existing CI. A successful CI build alone does not satisfy the reference/manual gates. Product implementation starts after this plan's review and execution-method confirmation.
