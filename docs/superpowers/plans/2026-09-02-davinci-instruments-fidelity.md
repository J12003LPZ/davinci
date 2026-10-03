# davinci instruments fidelity — implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the seventeen davinci command sheets (`3a`–`6d`) render row for row like the artboards in `docs/ui/Pi TUI Instruments.dc.html`.

**Architecture:** A `SheetChrome` descriptor per sheet feeds the header, status bar, hint row and composer mode from one place; `app::compose` draws a sheet top-anchored under the header with the hint row pinned last. New `ui` helpers (`window`, `column_header`, `footnote`, `selection_bar`) carry the shared look; each view is then matched to its artboard's text dump.

**Tech Stack:** Rust 1.83, ratatui `Line`/`Span`, inline `#[cfg(test)]` tests, the `dump_every_screen_for_the_mockup_audit` frame dump.

Spec: `docs/superpowers/specs/2026-09-02-davinci-instruments-fidelity-design.md` — its *Per-sheet contract* section is the row contract for Tasks 7–23. Extract each artboard's text with the snippet in the spec's *Source of truth* section and keep it beside you.

## Global Constraints

- No colour literal outside `crates/pi-tui/src/davinci/theme.rs`.
- Every state carries a glyph; colour is never the only signal. `NO_COLOR` must still read.
- Prose wraps at 74 columns (`ui::MEASURE`). Views return `Vec<Line<'static>>`.
- Paths are `.pi\…` / `%USERPROFILE%\.pi\agent\…`, never `.davinci\…`.
- Tests never touch the network or the real `~/.pi`.
- Meter tip is `◸` everywhere after Task 1.
- Facts not known live are omitted, never invented (spec rule 13).
- `cargo test -p pi-tui` and `cargo clippy -p pi-tui --all-targets -- -D warnings` pass after every task. `cargo fmt` before each commit.
- Commit after every task with the trailer:

  ```
  Co-Authored-By: Claude Fable 5.1 <noreply@anthropic.com>
  Claude-Session: https://claude.ai/code/session_01B82ex4RDB6A5isMoQBNRRj
  ```

Frame dump for a visual check at any time:

```
cargo test -p pi-tui dump_every_screen_for_the_mockup_audit -- --ignored --nocapture
```

---

## File structure

| File | Responsibility |
|---|---|
| `crates/pi-tui/src/davinci/theme.rs` | `METER_TIP` becomes `◸`. |
| `crates/pi-tui/src/davinci/ui.rs` | `window`, `column_header`, `footnote`, `selection_bar`, `hint_row`. |
| `crates/pi-tui/src/davinci/views/sheet.rs` (new) | `SheetChrome`, `Composer`, `chrome(model)` dispatch. |
| `crates/pi-tui/src/davinci/views/chrome.rs` | header/status/composer consult `sheet::chrome`. |
| `crates/pi-tui/src/davinci/app.rs` | `panel` top-anchors, echo row, hint row last; composer per mode. |
| `crates/pi-tui/src/davinci/model.rs` | new fact fields per sheet. |
| `crates/pi-tui/src/davinci/fixtures.rs` | fixtures carry the artboards' facts. |
| `crates/pi-tui/src/davinci/views/{cogitator,settings,thinking,login,keys,resume,tree,compact,export,graph_run,vectors,governor,securitas,trust,officina,recovery,diff,mcp,permissions}.rs` | one `chrome()` each; bodies matched to the artboard. |
| `crates/pi-coding-agent/src/davinci_session.rs` | `open_*_sheet` fill the new facts. |
| `docs/ui/design.md`, `CLAUDE.md` | §11 Command sheets; reference note. |

---

### Task 1: Meter tip `◸`

**Files:**
- Modify: `crates/pi-tui/src/davinci/theme.rs:42`
- Modify: `crates/pi-tui/src/davinci/ui.rs` (tests asserting `╸`)
- Modify: any other test asserting `╸` (`grep -rn '╸' crates/pi-tui/src crates/pi-coding-agent/src`)

- [ ] **Step 1: Change the constant**

```rust
pub const METER_TIP: &str = "◸";
```

- [ ] **Step 2: Run `cargo test -p pi-tui` and fix every assertion that expected `╸`** — replace the glyph in the expected strings only; do not change widths.

- [ ] **Step 3: `grep -rn '╸' crates` must return only the spec/plan docs.**

- [ ] **Step 4: Commit** `feat(tui): meter tip is ◸, as the mockups draw it`

---

### Task 2: `ui::window`, `ui::column_header`, `ui::footnote`, `ui::selection_bar`, `ui::hint_row`

**Files:**
- Modify: `crates/pi-tui/src/davinci/ui.rs`
- Modify: `crates/pi-tui/src/davinci/views/settings.rs` (use `window`)
- Modify: `crates/pi-tui/src/davinci/views/instrumenta.rs` (use `selection_bar`)

**Interfaces (produces):**

```rust
/// Keep `height` rows around `anchor`, folding the rest into `… n above` / `… n below`.
pub fn window(rows: Vec<Line<'static>>, height: usize, anchor: usize, theme: &Theme) -> Vec<Line<'static>>;
/// Uppercase headers in border colour over a hair rule in the dim ramp. Two rows, each exactly `width`.
/// `columns`: (label, width, right_aligned). Width 0 = take the slack.
pub fn column_header(width: u16, columns: &[(&str, u16, bool)], theme: &Theme) -> Vec<Line<'static>>;
/// Left in the ink given, right in border. One row at ≥100 cols, two below.
pub fn footnote(width: u16, left: Vec<Span<'static>>, right: Vec<Span<'static>>, theme: &Theme) -> Vec<Line<'static>>;
/// The 3-cell selection bar (copper on the tint) or its blank.
pub fn selection_bar(selected: bool, theme: &Theme) -> Span<'static>;
/// Hints joined by ` │ `, the escape hint right-aligned; hints drop from the end, esc never.
pub fn hint_row(width: u16, hints: &[Vec<Span<'static>>], escape: Option<&str>, theme: &Theme) -> Line<'static>;
```

- [ ] **Step 1: Write failing tests in `ui.rs` `mod tests`**

```rust
#[test]
fn window_keeps_the_anchor_and_counts_both_folds() {
    let th = theme();
    let rows: Vec<Line<'static>> = (0..20).map(|i| Line::from(format!("row {i}"))).collect();
    let out = window(rows, 6, 10, &th);
    assert_eq!(out.len(), 6);
    assert!(text_of(&out[0]).starts_with("… "), "{:?}", text_of(&out[0]));
    assert!(out.iter().any(|l| text_of(l) == "row 10"));
    assert!(text_of(out.last().unwrap()).ends_with(" below"));
}

#[test]
fn window_of_a_short_list_is_the_list() {
    let th = theme();
    let rows: Vec<Line<'static>> = (0..3).map(|i| Line::from(format!("row {i}"))).collect();
    assert_eq!(window(rows.clone(), 10, 0, &th).len(), 3);
}

#[test]
fn column_header_is_two_rows_of_the_width() {
    let th = theme();
    let rows = column_header(60, &[("PROVIDER / MODEL", 0, false), ("WINDOW", 6, true), ("CREDENTIAL", 12, false)], &th);
    assert_eq!(rows.len(), 2);
    assert_eq!(width_of(&rows[0]), 60);
    assert_eq!(width_of(&rows[1]), 60);
    assert!(text_of(&rows[0]).contains("PROVIDER / MODEL"));
    assert!(text_of(&rows[1]).chars().all(|c| c == '─'));
}

#[test]
fn footnote_is_one_row_wide_and_two_narrow() {
    let th = theme();
    let left = vec![span("dimmed rows have no credential", th.text)];
    let right = vec![span("switching keeps the transcript", th.border)];
    assert_eq!(footnote(100, left.clone(), right.clone(), &th).len(), 1);
    assert_eq!(footnote(80, left, right, &th).len(), 2);
}

#[test]
fn hint_row_right_aligns_the_escape_and_never_drops_it() {
    let th = theme();
    let hints = vec![vec![span("↑↓ move", th.border)], vec![span("enter select", th.border)], vec![span("ctrl+p cycle ring", th.border)]];
    let row = hint_row(40, &hints, Some("esc close"), &th);
    let text = text_of(&row);
    assert_eq!(width_of(&row), 40);
    assert!(text.ends_with("esc close"));
    assert!(text.starts_with("↑↓ move │ enter select"));
    assert!(!text.contains("ctrl+p"), "{text}");
}
```

- [ ] **Step 2: Run `cargo test -p pi-tui ui::tests` — expect compile failures for the missing functions.**

- [ ] **Step 3: Implement**

```rust
pub fn window(rows: Vec<Line<'static>>, height: usize, anchor: usize, theme: &Theme) -> Vec<Line<'static>> {
    let total = rows.len();
    if total <= height || height == 0 {
        return rows;
    }
    // Two rows may be spent on the fold markers.
    let inner = height.saturating_sub(2).max(1);
    let start = anchor.saturating_sub(inner / 2).min(total.saturating_sub(inner));
    let end = (start + inner).min(total);
    let mut out = Vec::with_capacity(height);
    if start > 0 {
        out.push(Line::from(vec![span(format!("… {start} above"), theme.border)]));
    }
    out.extend(rows.into_iter().skip(start).take(end - start));
    if end < total {
        out.push(Line::from(vec![span(format!("… {} below", total - end), theme.border)]));
    }
    out
}

pub fn column_header(width: u16, columns: &[(&str, u16, bool)], theme: &Theme) -> Vec<Line<'static>> {
    let fixed: u16 = columns.iter().filter(|c| c.1 > 0).map(|c| c.1 + 1).sum();
    let slack = width.saturating_sub(fixed);
    let mut spans = Vec::new();
    for (i, (label, w, right)) in columns.iter().enumerate() {
        let w = if *w == 0 { slack } else { *w };
        let cell = if *right { format!("{label:>w$}", w = w as usize) } else { format!("{label:<w$}", w = w as usize) };
        spans.push(span(clip(&cell, w), theme.border));
        if i + 1 < columns.len() {
            spans.push(span(" ", theme.border));
        }
    }
    let header = Line::from(truncate_run(spans, width));
    let rule = Line::from(vec![span(glyph::METER_EMPTY.repeat(width as usize), theme.dim().border)]);
    vec![pad_line(header, width, theme), rule]
}

fn pad_line(line: Line<'static>, width: u16, _theme: &Theme) -> Line<'static> {
    let mut spans = line.spans;
    let gap = width.saturating_sub(run_width(&spans));
    if gap > 0 { spans.push(pad(gap, None)); }
    Line::from(spans)
}

pub fn footnote(width: u16, left: Vec<Span<'static>>, right: Vec<Span<'static>>, theme: &Theme) -> Vec<Line<'static>> {
    if right.is_empty() {
        return vec![Line::from(truncate_run(left, width))];
    }
    if width >= 100 && run_width(&left) + run_width(&right) + 3 <= width {
        return vec![spread(width, left, right)];
    }
    let _ = theme;
    vec![Line::from(truncate_run(left, width)), indent(2, truncate_run(right, width.saturating_sub(2)))]
}

pub const SELECTION_BAR: &str = "▌  ";
pub fn selection_bar(selected: bool, theme: &Theme) -> Span<'static> {
    if selected {
        span_on(SELECTION_BAR, theme.primary, Some(theme.surface))
    } else {
        span("   ", theme.border)
    }
}

pub fn hint_row(width: u16, hints: &[Vec<Span<'static>>], escape: Option<&str>, theme: &Theme) -> Line<'static> {
    let right: Vec<Span<'static>> = escape.map(|e| vec![span(e, theme.border)]).unwrap_or_default();
    let room = width.saturating_sub(run_width(&right)).saturating_sub(3);
    let mut left: Vec<Span<'static>> = Vec::new();
    for (i, hint) in hints.iter().enumerate() {
        let mut candidate = left.clone();
        if i > 0 { candidate.push(span(" │ ", theme.border)); }
        candidate.extend(hint.iter().cloned());
        if run_width(&candidate) > room { break; }
        left = candidate;
    }
    spread(width, left, right)
}
```

`instrumenta.rs`: replace its private `SELECTION_BAR`/`UNSELECTED_BAR` use with `ui::selection_bar(selected, theme)` and re-export `pub use crate::davinci::ui::SELECTION_BAR;` so its tests keep compiling. `settings.rs`: delete its inline window logic and call `window(rows, height, selected, th)` — `settings::lines` gains a `height: usize` parameter; `app.rs` passes `height`.

- [ ] **Step 4: `cargo test -p pi-tui` passes; `cargo clippy -p pi-tui --all-targets -- -D warnings` clean.**

- [ ] **Step 5: Commit** `feat(tui): shared sheet helpers — window, column header, footnote, selection bar, hint row`

---

### Task 3: `SheetChrome` and `views/sheet.rs`

**Files:**
- Create: `crates/pi-tui/src/davinci/views/sheet.rs`
- Modify: `crates/pi-tui/src/davinci/views/mod.rs` (`pub mod sheet;`)

**Interfaces (produces):**

```rust
//! The frame every command sheet shares: header facts, status segments, hint
//! row and composer mode, one descriptor per sheet (design.md §11).
use ratatui::text::{Line, Span};
use crate::davinci::model::{Model, Screen};
use crate::davinci::theme::Theme;
use crate::davinci::ui::{self, span};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Composer {
    /// No composer under this sheet.
    Hidden,
    /// The composer with this placeholder (or the command that opened it).
    Prompt(&'static str),
    /// The composer is drawn but takes no input; the text sits in the dim ramp.
    Disabled(&'static str),
}

#[derive(Debug, Clone, Default)]
pub struct SheetChrome {
    /// Header right run; empty means `cwd │ branch │ model`.
    pub header_right: Vec<Span<'static>>,
    /// Third segment of `mode · branch · third`.
    pub status_third: Option<Vec<Span<'static>>>,
    /// Status bar right run; `None` is the context meter.
    pub status_right: Option<Vec<Span<'static>>>,
    /// Hints joined by ` │ `.
    pub hints: Vec<Vec<Span<'static>>>,
    /// `esc close`, `esc cancel`, `esc done`, `esc leave it`; `None` draws no hint row.
    pub escape: Option<&'static str>,
    pub composer: Composer,
    /// `> /command` echoed as the first body row.
    pub echo: Option<String>,
}

impl Default for Composer { fn default() -> Self { Composer::Hidden } }

/// A `│`-separated header run: `[("7 files", text), ("+145 -127", ...)]`.
pub fn facts(theme: &Theme, parts: Vec<Vec<Span<'static>>>) -> Vec<Span<'static>>;
/// `label ━━━◸ ─── used/cap`, 12 cells of meter as the status bar draws it.
pub fn status_meter(theme: &Theme, label: &str, fraction: f64, used: &str, cap: &str) -> Vec<Span<'static>>;
/// Border-coloured hint text.
pub fn hint(theme: &Theme, text: &str) -> Vec<Span<'static>>;
/// The sheet's descriptor, or `None` for the transcript, Plan, Grafo, Memoria recall, Mensura and Codex.
pub fn chrome(model: &Model) -> Option<SheetChrome>;
/// The hint row for this sheet, if it has one.
pub fn hint_row(model: &Model, chrome: &SheetChrome) -> Option<Line<'static>>;
```

- [ ] **Step 1: Write failing tests in `sheet.rs`**

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::davinci::theme::{ColorDepth, Theme};
    fn theme() -> Theme { Theme::da_vinci(ColorDepth::TrueColor, false) }
    fn text(spans: &[Span<'_>]) -> String { spans.iter().map(|s| s.content.as_ref()).collect() }

    #[test]
    fn facts_are_bar_separated() {
        let th = theme();
        let run = facts(&th, vec![vec![span("7 files", th.text)], vec![span("+145 -127", th.success)]]);
        assert_eq!(text(&run), "7 files │ +145 -127");
    }

    #[test]
    fn a_status_meter_names_its_unit_and_cap() {
        let th = theme();
        let run = status_meter(&th, "disk", 0.15, "1.2G", "8G");
        let t = text(&run);
        assert!(t.starts_with("disk "));
        assert!(t.ends_with(" 1.2G/8G"));
        assert!(t.contains('◸'));
    }

    #[test]
    fn the_transcript_has_no_sheet_chrome() {
        let m = Model::new(theme(), 100, 44, true);
        assert!(chrome(&m).is_none());
    }
}
```

- [ ] **Step 2: Implement** — `chrome` matches `model.screen`: every sheet arm calls `<view>::chrome(model)` (added per sheet in Tasks 7–23; until a view has one, return `Some(SheetChrome::default())` from a local `fn plain() -> SheetChrome` so the dispatch compiles), `Screen::Agent | Plan | Grafo | Memoria | Mensura => None`. Note `Screen::Agent` with `model.failed_run.is_some()` is `6c`: return `recovery::chrome(model)` when `model.screen == Screen::Recovery`.

```rust
pub fn facts(theme: &Theme, parts: Vec<Vec<Span<'static>>>) -> Vec<Span<'static>> {
    let mut out = Vec::new();
    for (i, part) in parts.into_iter().enumerate() {
        if part.is_empty() { continue; }
        if !out.is_empty() { out.push(span(" │ ", theme.border)); }
        let _ = i;
        out.extend(part);
    }
    out
}

pub fn status_meter(theme: &Theme, label: &str, fraction: f64, used: &str, cap: &str) -> Vec<Span<'static>> {
    let mut run = vec![span(format!("{label} "), theme.muted)];
    run.extend(ui::meter(fraction, 12, theme, None));
    run.push(span(format!(" {used}/{cap}"), theme.muted));
    run
}

pub fn hint(theme: &Theme, text: &str) -> Vec<Span<'static>> { vec![span(text, theme.border)] }

pub fn hint_row(model: &Model, chrome: &SheetChrome) -> Option<Line<'static>> {
    chrome.escape.map(|esc| ui::hint_row(model.width, &chrome.hints, Some(esc), &model.theme))
}
```

- [ ] **Step 3: `cargo test -p pi-tui sheet` passes. Commit** `feat(tui): SheetChrome descriptor for the command sheets`

---

### Task 4: Header and status bar consult `SheetChrome`

**Files:**
- Modify: `crates/pi-tui/src/davinci/views/chrome.rs` (`header`, `status_left`, `status_right`)

- [ ] **Step 1: Failing test in `chrome.rs` tests**

```rust
#[test]
fn a_sheet_with_facts_owns_the_header_right_run_and_the_status_third() {
    let mut m = Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 100, 44, true);
    crate::davinci::fixtures::dress_screen(&mut m, "6d");
    let h = text_of(&header(&m));
    assert!(h.ends_with("7 files │ +145 -127 │ main · 3 commits behind"), "{h}");
    let s = text_of(&status(&m));
    assert!(s.starts_with("agent · main · Δ7 +145 -127"), "{s}");
}
```

(This test passes only after Task 23 gives `6d` its chrome; mark it `#[ignore = "until 6d chrome lands"]` now and un-ignore in Task 23.)

- [ ] **Step 2: Implement.** At the top of `header`: `if let Some(c) = sheet::chrome(model) { if !c.header_right.is_empty() { return spread(model.width, left, c.header_right); } }`. In `status_left`, after the overlay early-return: `if let Some(c) = sheet::chrome(model) { let mut run = vec![span(model.mode(), th.primary), span(" · ", th.border), span(model.branch.clone(), th.secondary)]; if let Some(third) = c.status_third { run.push(span(" · ", th.border)); run.extend(third); } return run; }`. In `status_right`, after the overlay match: `if let Some(c) = sheet::chrome(model) { if let Some(right) = c.status_right { return right; } }`. Move the `Screen::Memoria` and `Screen::Mensura` arms of `header`/`status_left`/`status_right` unchanged into `memoria::chrome` / `mensura::chrome`? **No** — those are `2b`/`2c`, not command sheets; `sheet::chrome` returns `None` for them, so leave those arms in place.

- [ ] **Step 3: `cargo test -p pi-tui` passes. Commit** `feat(tui): header and status bar read the sheet chrome`

---

### Task 5: Sheets fill the body; hint row last; echo row

**Files:**
- Modify: `crates/pi-tui/src/davinci/app.rs` (`panel`, `body`, `compose`)

- [ ] **Step 1: Failing test in `app.rs` tests**

```rust
#[test]
fn a_sheet_starts_under_the_header_and_ends_with_its_hint_row() {
    let mut m = Model::new(Theme::da_vinci(ColorDepth::TrueColor, false), 100, 44, true);
    crate::davinci::fixtures::dress_screen(&mut m, "3b");
    let rows = compose(&m, 44);
    let body_first = text_of(&rows[1]);
    assert!(!body_first.trim().is_empty(), "first body row is blank: {body_first:?}");
    // Rows 1..=42 are the body when no composer is drawn (3b); the hint row is the last of them.
    let hint = rows.iter().rev().nth(1).map(text_of).unwrap();
    assert!(hint.trim_end().ends_with("esc close"), "{hint}");
}
```

(Passes fully after Task 8 gives `3b` its chrome; `#[ignore]` until then, un-ignore in Task 8.)

- [ ] **Step 2: Implement `panel`**

```rust
fn panel(model: &Model, rows: Vec<Line<'static>>, height: usize) -> Vec<Line<'static>> {
    let chrome = sheet::chrome(model).unwrap_or_default();
    let hint = sheet::hint_row(model, &chrome);
    let mut out = Vec::with_capacity(height);
    if let Some(echo) = &chrome.echo {
        out.push(Line::from(vec![span("> ", model.theme.muted), span(echo.clone(), model.theme.muted)]));
        out.push(blank());
    }
    let room = height.saturating_sub(out.len()).saturating_sub(usize::from(hint.is_some()));
    out.extend(ui::window(rows, room, model.sheet_anchor(), &model.theme));
    let mut out = pad_to(out, height.saturating_sub(usize::from(hint.is_some())));
    if let Some(hint) = hint { out.push(hint); }
    out
}
```

`Model::sheet_anchor(&self) -> usize` returns the selection index for the open screen (`catalog_index`, `settings_index`, `thinking_index`, `login_index`, `resume_index`, `tree_index`, `security_index`, `diff_index`, `permission_index`, `keys_offset`; `0` otherwise). Add it to `model.rs`. `Screen::Keys` uses the same path (`keys::lines(model, height)` becomes `keys::lines(model)`; windowing is now `panel`'s job).

- [ ] **Step 3: Composer per mode in `composer_rows`**: `if let Some(c) = sheet::chrome(model) { match c.composer { Composer::Hidden => return Vec::new(), Composer::Disabled(text) => return chrome::disabled_composer(model, text), Composer::Prompt(_) => return chrome::composer(model, None, Hint::None) } }`. Add `pub fn disabled_composer(model, text) -> Vec<Line>` to `chrome.rs`: a `Surface` with border `th.border`, one row `› text` in `th.dim().muted`, no caret. `screen_placeholder` reads `Composer::Prompt(p)` from the chrome before its own table.

- [ ] **Step 4: `cargo test -p pi-tui` passes; run the frame dump and confirm `3a` has no blank rows above its first row. Commit** `feat(tui): sheets fill the body, hint row pinned last, composer per sheet`

---

### Task 6: Fixtures carry the artboards' facts

**Files:**
- Modify: `crates/pi-tui/src/davinci/model.rs` — add fields (all `Default`):

```rust
// on Model
pub catalog_shown: usize, pub catalog_total: usize, pub providers_ready: usize, pub providers_total: usize,
pub catalog_refreshed: String,          // "2h ago"
pub settings_keys: usize,
pub thinking_reserve: String,           // "reserve 10k"
pub thinking_last_turn: String,         // "5.1k of 8k"
pub thinking_output_share: f64,         // 0.38
pub auth_path: String,                  // "%USERPROFILE%\\.pi\\agent\\auth.json"
pub auth_mode: String,                  // "0600"
pub keys_count: usize, pub keys_surfaces: usize,
pub sessions_disk: Option<(u64, u64)>,  // (used bytes, cap bytes)
pub session_name: String, pub session_turns: usize, pub session_branches: usize,
pub commits_behind: Option<u32>,
pub tool_schema_tokens: u64, pub tool_count: usize, pub command_count: usize,
```

and on the existing sheet structs: `Compaction { auto: bool, threshold: u8, compacted_before: String }`, `ExportLedger { session_bytes: String }`, `GraphRunSheet { elapsed: String, parallel: usize, milestone: String, mode: String }`, `VectorIndex { records: String, shards: usize, embedding: String, repository: String, held: String, injected: String, floor_note: String }`, `GovernorSheet { session_id: String, since: String, outputs_total: String }`, `SecurityScan { id: String, state: String, scanned: (u64, u64) }`, `ProjectTrustSheet { first_visit: bool }`, `WorkshopSheet { failed: usize }`, `ReviewSheet { commits_behind: Option<u32> }`, `ResumeRow { lineage: String, size: String }`, `TreeNode { detail: Option<String> }`, `ReviewFile { hunk_header: String }`.

- Modify: `crates/pi-tui/src/davinci/fixtures.rs` — `dress_screen` fills every field with the artboard's value (spec *Per-sheet contract*).

- [ ] **Step 1: Add fields, `cargo build -p pi-tui` compiles (struct literals in fixtures/tests updated with `..Default::default()` where needed).**
- [ ] **Step 2: Fill fixtures per sheet.** Values are in the spec's per-sheet lines (e.g. `3a`: shown 12, total 63, ready 6 of 10, refreshed "2h ago").
- [ ] **Step 3: `cargo test -p pi-tui` passes. Commit** `feat(tui): sheet models and fixtures carry the artboard facts`

---

### Tasks 7–23: one sheet each

Every one of these tasks follows the same shape. The row contract is the spec's *Per-sheet contract* entry for the sheet; the artboard text dump is the oracle.

**Per-task steps:**

- [ ] **Step 1: Add `pub fn chrome(model: &Model) -> SheetChrome` to the view** with the header facts, status third, status right, hints, escape, composer and echo the spec lists. Register it in `sheet::chrome`.
- [ ] **Step 2: Write the chrome test** in the view's `mod tests`:

```rust
#[test]
fn the_sheet_wears_its_artboard_chrome() {
    let mut m = fixture_model();                    // Model::new + fixtures::dress_screen(&mut m, "<id>")
    let c = chrome(&m);
    assert_eq!(text(&c.header_right), "<header facts from the spec>");
    assert_eq!(text(c.status_third.as_deref().unwrap()), "<third>");
    assert_eq!(c.escape, Some("<esc …>"));
    assert_eq!(c.composer, Composer::<mode>);
    let hint = text_of(&sheet::hint_row(&m, &c).unwrap());
    assert!(hint.starts_with("<first hint>"), "{hint}");
    assert!(hint.trim_end().ends_with("<esc …>"), "{hint}");
}
```

- [ ] **Step 3: Write the body test** — one assertion per delta the spec lists for the sheet, on `lines(&m)` text (e.g. `3a`: a row contains `in ctrl+p ring`; a row contains `catalog refreshed ✓ 2h ago`; the first row contains `filter models…` and ends with `12 of 63 shown · 6 of 10 providers ready`).
- [ ] **Step 4: Run — expect failure.**
- [ ] **Step 5: Match the body** to the artboard: header rows through `ui::column_header`, selected rows through `ui::selection_bar` plus the `surface` tint, dimmed rows through `model.theme.dim()`, footnotes through `ui::footnote`, panels through `Surface`. Remove the view's own hint line (the hint row is now `panel`'s). Update the module doc comment to `Mirrors artboard <id> of docs/ui/Pi TUI Instruments.dc.html`.
- [ ] **Step 6: `cargo test -p pi-tui <view>` passes; frame dump the screen and compare by eye against the artboard PNG/text.**
- [ ] **Step 7: Commit** `feat(tui): <id> <name> matches its artboard`

| Task | Sheet | View file | Fixture id | Un-ignore |
|---|---|---|---|---|
| 7 | 3a Cogitator model | `cogitator.rs` (`catalog`) | `3a` | |
| 8 | 3b Settings | `settings.rs` | `3b` | app.rs test from Task 5 |
| 9 | 3c Thinking | `thinking.rs` | `3c` | |
| 10 | 3d Provider auth | `login.rs` | `3d` | |
| 11 | 3e Keys | `keys.rs` | `3e` | |
| 12 | 4a Memoria resume | `resume.rs` | `4a` | |
| 13 | 4b Memoria tree | `tree.rs` | `4b` | |
| 14 | 4c Mensura compaction | `compact.rs` | `4c` | |
| 15 | 4d Export | `export.rs` | `4d` | |
| 16 | 5a Grafo worker graph | `graph_run.rs` | `5a` | |
| 17 | 5b Memoria index | `vectors.rs` | `5b` | |
| 18 | 5c Mensura governor | `governor.rs` | `5c` | |
| 19 | 5d Securitas | `securitas.rs` | `5d` | |
| 20 | 6a Fiducia trust | `trust.rs` | `6a` | |
| 21 | 6b Officina | `officina.rs` | `6b` | |
| 22 | 6c Recovery | `recovery.rs` | `6c` | |
| 23 | 6d Δ review | `diff.rs` | `6d` | chrome.rs test from Task 4 |

Sheet-specific notes beyond the spec:

- **3a** — the filter box is a `Surface` with border `th.secondary`, row `› filter models…` and the count right-aligned inside via `spread(inner, …)`. Composer hidden; the `3a` composer key handling (`Choice::Catalog`) stays.
- **3d** — the device code box is a nested `Surface` 26 cells wide placed right of the steps: build the panel body rows by `spread`ing step text against the box's rows one for one (rows: top rule, code letter-spaced `W Q P T - F J 4 M`, `expires in 8m 41s`, bottom rule).
- **4c** — side-by-side panels: at width ≥ 100, build both `Surface`s at `(width − 2) / 2` and zip their `lines()` with two spaces between; otherwise stack.
- **5a** — stage strip: `spread(width, stages, [elapsed])` where stages is `✓ classify ── ✓ investigate ── … ○ done` and the connector `──` is border colour.
- **6a** — `escape: None` (keys live in the panel), `composer: Composer::Disabled("the composer is disabled until you decide")`.
- **6c** — screen is `Screen::Recovery`; `status_third = "interrupted"` in warning; context `58k/200k` comes from the fixture's `context`.
- **6d** — `+n` right-aligned width 5 in success, `-n` width 5 in error, note column right-aligned; hunk header row `@@ 214,7 +214,18 @@ impl OpenAiProvider` from `ReviewFile::hunk_header`.

---

### Task 24: `/mcp` and `/permissions` adopt the frame

**Files:** `views/mcp.rs`, `views/permissions.rs`

- [ ] `chrome()` for each: header facts `n servers │ m tools` (mcp) / `mode ask │ n rules` (permissions) from what the views already hold; `status_third` = `"mcp"`/`"permissions"` count; hints = the view's existing hint text split on ` · `; `escape: Some("esc close")`; `composer: Composer::Prompt("/mcp")` / `Prompt("/permissions")`. Bodies unchanged except removing their own hint line.
- [ ] Tests as in Tasks 7–23 (chrome only). Commit `feat(tui): mcp and permissions sheets wear the shared frame`

---

### Task 25: Live wiring in `davinci_session.rs`

**Files:** `crates/pi-coding-agent/src/davinci_session.rs` (`open_models_sheet`, `open_settings_sheet`, `open_thinking_sheet`, `open_login_sheet`, `open_keys_sheet`, `open_resume_sheet`, `open_tree_sheet`, `open_trust_sheet`, `open_diff_sheet`, and the `/compact`, `/export`, `/graph`, `/memory-status`, `/governor-status`, `/sec-report`, `/reload` openers)

- [ ] For each opener set the fields Task 6 added, from data the opener already has:
  - `3a`: `catalog_shown = catalog.len()`, `catalog_total` = full catalog length before filtering, `providers_ready` = rows with `Credential::Ready|Running`, `providers_total` = distinct providers, `catalog_refreshed` from the models file mtime (`humanize` as `2h ago`; empty if unknown).
  - `3b`: `settings_keys = settings_rows.len()`.
  - `3c`: `thinking_reserve` empty unless the agent exposes a reserve; `thinking_last_turn` from the last assistant usage's reasoning tokens if present; `thinking_output_share` = reasoning ÷ output over the session if present, else `0.0` (view omits the line when `0.0`).
  - `3d`: `auth_path` = `default_agent_dir().join("auth.json")` with `%USERPROFILE%` substituted; `auth_mode` = `"0600"` on unix, empty on Windows (segment omitted).
  - `3e`: `keys_count` = total bindings, `keys_surfaces = keymap.len()`.
  - `4a`: `session_count`, `sessions_disk = Some((sum of file sizes under the session dir, volume total))` — volume total via `fs2`-free approach: skip and set cap to `0` → view omits the meter when cap is 0.
  - `4b`: `session_name`, `session_turns`, `session_branches` from the loaded tree.
  - `6d`: `commits_behind` via `git rev-list --count HEAD..@{upstream}` (already shelling to git for the diff; `None` on any failure).
  - `6b`: `tool_schema_tokens` = Σ `serde_json::to_string(spec).len() / 4`, `tool_count`, `command_count`.
- [ ] A test per opener already exists in `davinci_session.rs` tests? Add one assertion each that the field is set on the fixture-driven path (`PI_CODING_AGENT_DIR` to a temp dir). Where no test harness exists for an opener, add `#[test] fn <opener>_sets_its_facts()` building the `Model` through the opener with an empty temp agent dir and asserting the counts are `0`/`None` rather than panicking.
- [ ] `cargo test -p pi-coding-agent` passes; `cargo clippy --workspace --all-targets -- -D warnings` clean. Commit `feat(cli): sheet openers fill the artboard facts from live data`

---

### Task 26: Docs

**Files:** `docs/ui/design.md`, `CLAUDE.md`

- [ ] Append `## 11. Command sheets` to design.md: the thirteen frame rules from the spec, verbatim in intent, with the `◸` correction to §6's status bar example (`context ━━━━◸ ──── 47k/200k`).
- [ ] CLAUDE.md, davinci paragraph: `docs/ui/Pi TUI Instruments.dc.html` is the source for `3a`–`6d`; the `.ex` tree is the reference for `1a`–`2c` only and is superseded for the command sheets; view doc comments cite the artboard.
- [ ] Commit `docs: command sheet frame rules; instruments canvas is the 3a-6d reference`

---

### Task 27: Final audit

- [ ] `cargo fmt --all`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`.
- [ ] Frame dump every screen; for each of `3a`–`6d` compare against the artboard text dump: header run, status run, first body row, hint row, composer presence. Fix anything that drifted.
- [ ] `make build`; run `./target/debug/pi --davinci --screen 3a` once to confirm the binary still opens a fixture screen.
- [ ] Commit any fixes `fix(tui): audit pass on the instrument sheets`.

---

## Self-review

- **Spec coverage:** rules 1–13 → Tasks 1 (7), 2 (6, 8, 10, 11), 3–5 (1–5, 12–13), 6 (13), 7–23 (per-sheet), 24 (out-of-scope sheets adopt the frame), 25 (live facts), 26 (docs). Testing section → each task's tests plus Task 27.
- **Type consistency:** `SheetChrome`, `Composer::{Hidden, Prompt, Disabled}`, `sheet::chrome`, `sheet::hint_row`, `ui::{window, column_header, footnote, selection_bar, hint_row}`, `Model::sheet_anchor` are the only new names; each is defined in Task 2, 3 or 5 before use.
