# davinci TUI — design specification

## Claude Code conversation contract (September 2026)

The default conversation UI follows the captured Claude Code 2.1.281 frames in
`docs/ui/claude-code-reference/`. Those frames are the visual ground truth for
the composer, completion list, footer, transcript rows, working line, permission
panel, and picker panels. The implementation keeps DaVinci's own product identity
and behavior where the reference contract explicitly differs.

Intentional differences from the captured reference:

1. The product name, version, and DaVinci block-letter logo remain DaVinci's.
2. The completion line omits Claude Code's local-time `done` clock.
3. User `!` shell commands do not automatically start a model reply afterwards.
4. Failed calls are not folded into grouped exploration rows.
5. Always Approve keeps the explicit words `no prompts`.
6. The voice `[mic …]` label remains on the top rule when voice is enabled.
7. The working area has no tip row and no `/btw` behavior.
8. Light-theme Claude Code colors are documented values rather than captured frames.

Enter runs the highlighted slash command, Tab only completes it, Enter on an
`@` file suggestion inserts the path, `?` in an empty composer opens the
shortcuts panel, and a leading `!` switches the composer into shell mode.
`NO_COLOR`, narrow terminals, and the non-default themes remain supported.

The editorial print notes below describe the optional `vox` theme. They are no
longer the visual contract for the default conversation surface.

### Experimental local voice

The composer upper rule carries a right-aligned mic control. Its mouse target
comes from the same composed frame, follows short transcripts, and disappears
on modal or clipped surfaces. It does not consume editable width. Ctrl+T and
mic click share one action; default tool expansion moves to Alt+T while explicit
bindings and legacy/tree contexts retain their meaning. Labels and hotkeys use
the effective map.

Preparing, REC elapsed time, stopping, transcribing and cancelling are visible.
Only a capture acknowledgement enables REC. Final text is one undoable insertion
at the live caret; Enter cannot send while voice is active or before the inserted
draft is drawn and its guard expires. Setup requires explicit download/import
consent and never starts capture. See [local voice](../voice-input.md) for settings,
privacy and the unverified platform/release acceptance gates.

The current Rust interface uses an editorial print system inspired by the supplied
newspaper collage references: warm ink, aged paper, crimson and navy layers,
and yellow callouts. The historical `Pi TUI Mockups.dc.html` remains a reference
for feature layout and behavior, not the current color or typography treatment.

Screens: `1a` startup · `1b` transcript · `1c` Disegno plan · `1d` Instrumenta
palette · `1e` Codex workspace (160 cols) · `1f` Memoria + Cogitator · `1g` 80
cols · `1h` NO_COLOR · `2a` Grafo · `2b` Memoria vector recall · `2c` Mensura
token governor.

### September 2026 interface revision

The optional `vox` theme is selectable beside `dark` and `light` in `/settings`
and first-time setup. It adds distressed clipping ends, coarse print rules, and
brighter crimson selection bands while retaining ink, cream, navy, and yellow.
The existing light and dark palettes are preserved. Color-depth fallbacks and
`NO_COLOR` remain supported. Preview it with
`cargo run -p davinci-tui --example editorial_preview --offline -- vox`.

The masthead combines a four-row screen-print D with a yellow DAVINCI clipping,
actual version, model, thinking level, and working directory. Cream paper strips
carry uppercase screen titles; section summaries keep their original case because
they can contain IDs and paths. The terminal keeps its own monospace font.

A short conversation keeps the masthead and places the composer after its content;
a full conversation anchors the composer at the bottom. Crimson bands separate
user messages and focused selections. Navy supports secondary panels. Static,
irregular print rules frame the composer; texture stays out of code and prose.
The caret is cream, and state glyphs retain meaning without color.

This is a character-grid adaptation: no photographs, rotated scraps, custom fonts,
image protocols, random grain or animation behind readable content. Geometry,
keyboard ownership, commands, wrapping and scrolling remain unchanged. The default
legacy palette follows the same ink and sepia colors; custom themes retain their
existing configuration.

Render a reproducible, offline contact sheet of actual Ratatui buffers:
`cargo run -p davinci-tui --example editorial_preview --offline > preview.html`.
It includes wide and narrow screens plus monochrome; fixture data is illustrative.

- Graph runs use the terminal-native [Living Blueprint](graph-living-blueprint.md):
  deterministic dependency cards, safe completed-group folding, explicit smart
  follow, and a public-execution inspector. Small terminals retain a worker ledger.
- Governor status shows actual stored output metadata, configuration state,
  and explicitly estimated token savings. Compression and unchanged-read
  deduplication produce an eight-second notice above the composer without
  taking focus. The notice is omitted below 12 rows; the status command remains
  available. Only the latest notice is shown during a burst of tool completions.
- Vector memory shows the recall flow, record distribution, storage, health,
  and the actual retrieval mode (automatic, on demand, or off).

Feature footers list working slash commands instead of unimplemented single-key
actions. Arrow keys and Page Up/Down scroll the three feature sheets, bounded
to their content. Older mockups retain their original artwork and sample data.

---

## 1. Principle

The terminal is a notebook, not a dashboard. The transcript is the interface;
every other instrument is summoned, used, and dismissed. Decoration appears only
where the user is waiting or reading a plan (startup, empty state, Disegno,
projections) and never in the transcript body.

Three hard rules:

1. **One panel at a time.** No permanent split panes. `1e` is the only screen
   with a persistent sidebar and it is opt-in at ≥120 cols.
2. **Color is never the only signal.** Every state also has a glyph (§4).
3. **Nothing animates that the user is reading.** Motion is limited to the
   caret and one spinner (§8).

---

## 2. Color tokens

Truecolor values. Map to the nearest ANSI-256 when the terminal reports fewer,
and drop to `NO_COLOR` (§9) below 16.

| Token       | Hex       | Role |
|---|---|---|
| `background` | `#1D1516` | terminal ground |
| `surface`    | `#5E1C16` | user messages, selection, panel fill |
| `surface_alt`| `#182033` | secondary panels, sidebar |
| `border`     | `#9C6C4F` | panel rules, separators, inert glyphs |
| `text`       | `#D8A687` | primary copy, code, prompt, caret |
| `muted`      | `#C59574` | secondary copy, tool arguments, keybind hints |
| `primary`    | `#F3D90D` | editorial yellow: identity, in-progress, selection, Δ |
| `secondary`  | `#D8A687` | identifiers, thinking level, memoria |
| `success`    | `#D8A687` | completed tools, additions, healthy caps |
| `warning`    | `#F3D90D` | attention, soft-cap breach, governor proposals |
| `error`      | `#E6A080` | failures, deletions |

Dimmed layer (behind a modal, `1d` and `1f`): `text → #80604D`,
`muted → #70503E`, `primary → #827522`, `border → #4A3028`. Never blur, never
tint — just drop the ramp.

Yellow carries focus and active work. Muted text carries tool arguments;
success, warning, and error colors reinforce their status glyphs. Muted sepia
(`#C59574`) and error ink (`#E6A080`) are lifted print tints for readable contrast
on crimson; the darker reference reds are surface colors rather than small text.

```rust
pub struct Theme {
    pub background: Color, pub surface: Color, pub surface_alt: Color,
    pub border: Color, pub text: Color, pub muted: Color,
    pub primary: Color, pub secondary: Color,
    pub success: Color, pub warning: Color, pub error: Color,
}
impl Theme { pub const DA_VINCI: Self = /* table above */; }
```

No color literal outside `Theme`. Widgets take `&Theme`.

---

## 3. Type and grid

One monospace face, the terminal's own. Mockups render `Cascadia Mono` with
`JetBrains Mono` as the substitute. Requirements: full box-drawing coverage
(`─ │ ╭ ╮ ╰ ╯ ┬ ┴ ├ ┤ ╱ ╲ ━ ╸`), geometric shapes (`◉ ○ ◌ ◐ ◑ ◒ ◓ ◜ ◝ ◞ ◟ ◆ ◇`)
and `Δ · ✓ × ! ↳ ⌕ ❯ ⟐ ● ⎿` plus block elements for the welcome mascot.
If box-drawing is unavailable, fall back to ASCII
frames (`+ - |`) rather than mixing widths.

Line rhythm: one blank line between transcript blocks, none inside a block.
Tool calls align with assistant bullets. Result summaries begin two columns in;
tool detail (error bodies, diff hunks) begins four columns in.

A tool call names its action, with the result underneath:

```
● Read(crates/davinci-agent/src/lib.rs) · 0.2s
  ⎿ Read 412 lines
● Shell(cargo check -p davinci-agent) · 1.84s
```

Action names are bold, arguments muted, and completed calls green. Failures,
warnings, queued and skipped calls retain distinct glyphs; `NO_COLOR` retains
all state glyphs. A supplied summary appears on the result row. Without a
summary, a collapsed successful call shows its first output line. `ctrl+t`
still expands tool output using the existing line caps.

Panels are drawn with a full rule and a label notched into the top-left corner
of it, label always uppercase, letter-spaced, and prefixed by nothing:

```
╭─ STUDIO ─────────────────────────────╮      label at col 2 of the top rule
```

---

## 4. State glyphs

Fixed vocabulary. Color reinforces, never replaces.

| Glyph | Meaning | Color |
|---|---|---|
| `✓` | done, passed, added | success |
| `◉` | in progress, selected, current | primary |
| `○` | queued, not started | border |
| `◌` | skipped |  muted |
| `×` | failed | error |
| `!` | attention, cap breach, untested | warning |
| `Δ` | file modification | primary |
| `↳` | file read | secondary |
| `⌕` | search / recall | secondary |
| `◆` | agent turn mark | primary |
| `❯` | composer prompt | text |
| `>` | user turn, echoed | text |
| `·` | measurement tick, compass mark | border |

---

## 5. Naming

Latin instrument names sit **beside** plain terms, never instead of them. First
appearance in a session is paired (`TOOLS · INSTRUMENTA`); after that the short
form is fine in panel labels and the status bar. Body copy is plain English.

| Instrument | Surface | Key |
|---|---|---|
| Codex | workspace / file tree (`1e`) | `ctrl+e` |
| Memoria | sessions (`1f`), vector recall (`2b`) | `ctrl+s` / `ctrl+m` |
| Instrumenta | command palette (`1d`) | `ctrl+p` |
| Manus | shell execution (`1b`) | — |
| Cogitator | model / provider picker (`1f`) | `ctrl+o` |
| Mensura | token governor (`2c`) | `ctrl+u` |
| Disegno | plan view (`1c`) | `ctrl+l` |
| Grafo | code graph (`2a`) | `ctrl+g` |
| Studio | reasoning progress (`1b`, `1h`) | inline |

Work verbs, used literally: studying, surveying, tracing, measuring, testing,
constructing, verifying.

---

## 6. Components

**AppShell** — welcome banner, transcript, composer, and quiet footer. Short
conversations leave spare space below the footer. Command sheets and overlays
keep the compact header and fill the window.

**Transcript** — user turns are `> text` on a grey background, preserving typed
whitespace. Replies open with `●` and have no name label or timestamp. Prose
wraps at 74 columns plus a two-column bullet gutter.

**ToolCall** — `● Action(argument)` and an optional `⎿` result row. Failures
expand to at most 4 indented lines and keep the exit code (`1b`).

**Studio** — the only box allowed mid-turn. Ledger of ✓ / ◉ / ○ steps with the
active step's target appended in border color. Collapses to one line
(`⟐ studying <path>`) below 100 cols (`1g`).

**Disegno** — Roman numerals I–V in a 4-column gutter, footer reads
`constructio III / V` with a tick meter. One decorative compass in the top-right,
clipped by its own layer so the panel label is never cut.

**Δ block** — `Δ path  +n -m`, then hunks behind a single left rule. Additions
success, deletions error, context muted. No line numbers unless asked.

**Composer** — neutral horizontal rules, `❯` prompt, white block caret. It grows
to at most eight visible text rows, or one third of a short window. Longer
drafts scroll around the active editor row and count hidden lines. Long logical
rows scroll horizontally to keep the caret visible. Hints use muted ink and
abbreviate with the window. `ctrl+c` interrupts the run.

**StatusBar** — conversation footer: permissions, branch, changes and jobs on
the left; labeled context percentage and thinking level on the right. Narrow
windows keep the permission mode first. Feature sheets retain their existing
meters and facts.

**Instrumenta** — inset overlay (52 cols of margin at 100 cols), query line,
result rows of `command · description · kind`, selection marked by a 3-cell
copper left bar plus tinted row. Footer states the corpus: tools, sessions,
files, modes.

**Grafo** (`2a`) — the graph is drawn on a strict column grid: parent connector
column is inherited by every child row, and no vertical may descend through label
text. Below it, an impact list: `glyph  symbol  distance  call sites`, with
untested edges in warning. Header carries `nodes · edges · cycles`.

**Memoria recall** (`2b`) — each hit is two rows: score + summary + location,
then a proportion meter and provenance. Hits below the relevance floor are shown
as held back, with the count, so the retrieval is auditable. Projection panel is
decoration with a job: it shows the query against the session cluster.

**Mensura** (`2c`) — budget by role, one row each: `role  tokens  meter  cap`.
Rows within cap use verdigris, the breaching row copper with a warning cap note.
The governor proposal is a bordered warning block that always states recovers /
keeps / cost / reversible, then keyed actions. Never acts silently.

---

## 7. Responsive

| Width | Behaviour |
|---|---|
| <48 | welcome mascot hidden, short hints, permission mode retained |
| 80 (`1g`) | compact welcome, Studio collapses to one line, footer abbreviates, sheet paths shorten to crate-relative |
| 100 (`1b`) | full transcript, Studio box, overlays inset by 6 cols |
| 120 | overlays inset further; panels may open as right sidebars |
| 160 (`1e`) | Codex sidebar at 250 cells, popovers (git changes) allowed |

Below 80 cols: transcript and composer only. Every panel becomes a full-screen
overlay rather than a squeezed column. Nothing requires a large window.

---

## 8. Motion

Two animations exist. The caret blink at ~1s step-end, and one 4-frame spinner
(`◜ ◝ ◞ ◟`, 250ms per frame) marking the single active Studio step. Panels open
and close in one frame — no slide. Startup may draw the emblem in ≤250ms and
must accept input during it. Everything collapses to static under
`prefers-reduced-motion` / `--no-animation`.

---

## 9. Accessibility

- `NO_COLOR` (`1h`): the full ramp becomes greyscale, `border → #5a5a5a`,
  `text → #e6e6e6`, active glyphs pure white and bold. Every state still reads,
  because state was never color-only.
- Keyboard only. No pointer affordance anywhere.
- Every panel states its own exits in its footer.
- Contrast: body text on ground ≥ 7:1, muted ≥ 4.5:1, border used only for
  non-informational strokes and hints that are repeated elsewhere.
- Numbers are always labelled with their unit and their cap (`47k/200k`, not
  `47k`).

---

## 10. Signature

The welcome uses a small terracotta pixel mascot beside the davinci name and
real session facts. It remains above short conversations and gives way as the
transcript fills the window. Feature screens retain `Δ` for change, `◉` for the
active item, Roman numerals for plans, and proportion meters for budgets.

---

## 11. Command sheets

Screens `3a`–`6d` are command sheets: `/model`, `/settings`, `/thinking`,
`/login`, `/hotkeys`, `/resume`, `/tree`, `/compact`, `/export`, `/graph`,
`memory-status`, `governor-status`, `sec-report`, `/trust`, `/reload`, the
interrupt, and the `Δ` review. Their source of truth is the Instruments canvas
(`docs/ui/Pi TUI Instruments.dc.html`, one artboard per sheet). Every sheet
wears the same frame, described in one place (`views/sheet.rs`,
`SheetChrome`), not in seventeen views:

1. **The sheet fills the body.** Rows start directly under the header. No
   transcript shows behind a sheet, and nothing is bottom-anchored. A sheet
   opened by a slash command echoes that command as its first row
   (`> /compact keep the store.rs decisions verbatim`, muted, as the
   transcript draws a user turn): `3d`, `4c`, `4d`, `5a`, `6b`, `6c`, `6d`.
2. **Header right run is the sheet's facts**, `│`-separated in border
   colour, values in muted or the colour the artboard gives them. Sheets
   with no facts of their own keep `cwd │ branch │ model`.
3. **Status bar left is three segments**: `mode · branch · third`, the third
   set per sheet. **Status bar right** is the sheet's own meter where the
   artboard draws one, otherwise the context meter. Meters keep the
   `label ━━━◸─── used/cap` shape.
4. **The hint row is the last body row**: border colour, hints separated by
   ` │ `, the escape hint (`esc close`, `esc cancel`, `esc done`,
   `esc leave it`) right-aligned. When the row is too wide, hints drop from
   the end; the escape hint never drops. The hint row never scrolls off.
   Where the artboard draws its keys inside a panel (`4c`, `4d`, `6a`), that
   panel's key row is the hint row and no separate row is drawn. A hint
   whose key is not wired yet is drawn in the dim ramp.
5. **Composer only where the artboard draws one.** Hidden on `3b`, `3d`,
   `3e`, `4a`, `4b`, `4d`. A filter box in the body instead on `3a` (`4a` has
   a `⌕` filter box as its first row, same shape). Prompt with the artboard's
   placeholder or command on the rest. `6a` draws the composer disabled, its
   text `the composer is disabled until you decide` in the dim ramp. The
   composer's own hint row is not drawn while a sheet is open; the sheet's
   hint row is the only hint row.
6. **Overflow windows around the selection**, `… n above` / `… n below` in
   border colour (`ui::window`), on every list sheet. Panels and footnotes
   stay whole; only the list windows.
7. **Meter tip is `◸`**, everywhere. `NO_COLOR` keeps the glyph.
8. **Selection is the Instrumenta mark**: the 3-cell copper left bar plus a
   `surface` tint across the row (`ui::selection_bar`), on every sheet with
   a cursor. The `◉` state glyph stays as well; colour is never the only
   signal.
9. **Rows the artboard dims** (no credential on `3a`, absent providers on
   `3d`, dismissed findings on `5d`, harmless files on `6a`) render in the
   theme's dim ramp.
10. **Column headers** are uppercase in border colour with a hair rule under
    them in the dim ramp's border colour (`ui::column_header`), as the
    artboards draw `PROVIDER / MODEL … CREDENTIAL`.
11. **Footnotes are two columns** (`ui::footnote`): left in text or muted,
    right in border. Below 100 columns, or when the two do not fit, the right
    column wraps under the left.
12. **Panels keep `╭─ LABEL ─╮`**, the terminal translation `1c` and `2c`
    established for the artboards' CSS boxes. Labels stay uppercase.
13. **Facts that are not known live are omitted**, never invented. A header
    run with one missing fact drops that segment; a status third that cannot
    be computed drops to two segments. Paths are shown as `.pi\…` with
    `%USERPROFILE%` (or `~`) for the home directory.

### Live theme selection

Open `/settings`, select **Theme**, and press Enter to switch between `dark` and `light`. The interface redraws immediately and the choice is saved for future sessions. Trusted project settings retain their existing precedence over user settings. Light uses aged cream paper, dark printed ink, crimson focus, and yellow headline scraps; terminal color-depth and monochrome fallbacks are preserved.

`/model` lists only models from providers with usable credentials. Log in with `/login` to add a provider. The internal security scan control commands are omitted from slash-command discovery.

Thinking levels are selected in `/model`: use Up/Down to highlight a model and Left/Right to adjust its supported reasoning level. Enter saves both; Escape cancels the pending selection. The choice is remembered per model. `/thinking` is removed from slash-command discovery; Tab reasoning-cycle shortcuts are disabled in the native UI.

`/init [focus]` starts a normal agent turn to inspect the repository and create or carefully update `AGENTS.md`, preserving existing instructions. It uses the active model and normal tool permissions. It does not target `CLAUDE.md`.
