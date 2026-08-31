# davinci TUI — design specification

Companion to `Pi TUI Mockups.dc.html`. Every rule here is visible in one of the
eleven mockup screens; the screen id is cited so an implementer can look at the
thing rather than infer it.

Screens: `1a` startup · `1b` transcript · `1c` Disegno plan · `1d` Instrumenta
palette · `1e` Codex workspace (160 cols) · `1f` Memoria + Cogitator · `1g` 80
cols · `1h` NO_COLOR · `2a` Grafo · `2b` Memoria vector recall · `2c` Mensura
token governor.

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
| `background` | `#0B1011` | terminal ground |
| `surface`    | `#101719` | header, status bar, panel fill |
| `surface_alt`| `#0E1416` | composer well, sidebar |
| `border`     | `#453A27` | panel rules, separators, inert glyphs, keybind hints |
| `text`       | `#DDD5C4` | primary copy, active row, code |
| `muted`      | `#80796D` | secondary copy, tool lines, hair/veil strokes |
| `primary`    | `#D58A32` | copper: focus, in-progress, selection, Δ, caret, agent mark |
| `secondary`  | `#52A89C` | verdigris: git branch, paths, identifiers, memoria |
| `success`    | `#74A879` | ✓, additions, healthy caps |
| `warning`    | `#D5A047` | !, soft-cap breach, governor proposals, `M` in git status |
| `error`      | `#C4593F` | ×, deletions, failing tests |

Dimmed layer (behind a modal, `1d` and `1f`): `text → #3f3a31`,
`muted → #5d564c`, `primary → #6b512c`, `border → #2b2519`. Never blur, never
tint — just drop the ramp.

Exactly one accent carries state (copper). Verdigris is reserved for *where
something is* (branch, path, symbol) and never for *what is happening*. That
split is what keeps the palette from reading as decoration.

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
and `Δ · ✓ × ! ↳ ⌕ › ⟐`. If box-drawing is unavailable, fall back to ASCII
frames (`+ - |`) rather than mixing widths.

Line rhythm: one blank line between transcript blocks, none inside a block.
Indent tool lines two columns under the agent mark; indent tool detail (error
bodies, diff hunks) two further.

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
| `›` | composer prompt | primary |
| `>` | user turn, echoed | muted |
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

**AppShell** — header (identity left, `path │ branch │ model` right), transcript,
composer, status bar. Header and status bar are one line each at every width.

**Transcript** — user turns are `> text` in muted, no bubble, no timestamp.
Agent turns open with `◆ davinci`. Prose wraps at 74 columns even when the
terminal is wider; measure never exceeds it.

**ToolCall** — one line: `glyph  instrument · verb   target   duration`. No box.
Failures expand to at most 4 indented lines and keep the exit code (`1b`).

**Studio** — the only box allowed mid-turn. Ledger of ✓ / ◉ / ○ steps with the
active step's target appended in border color. Collapses to one line
(`⟐ studying <path>`) below 100 cols (`1g`).

**Disegno** — Roman numerals I–V in a 4-column gutter, footer reads
`constructio III / V` with a tick meter. One decorative compass in the top-right,
clipped by its own layer so the panel label is never cut.

**Δ block** — `Δ path  +n -m`, then hunks behind a single left rule. Additions
success, deletions error, context muted. No line numbers unless asked.

**Composer** — the loudest element on screen: copper 1px rule, `›` prompt,
blinking block caret. Grows with content; keybind hints below it in border color
(`enter send · shift+enter newline · tab complete · esc cancel`). `ctrl+c`
interrupts the run, never the app.

**StatusBar** — left `mode · branch · Δn +a -d`, right `context ━━━━╸──── 47k/200k`
or, when narrow, `mensura ◐ 21%`. Both forms are meters, not bare numbers.

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
| 80 (`1g`) | no ASCII art, no annotations, Studio collapses to one line, status bar abbreviates (`^p`), paths shorten to crate-relative |
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

The identity mark is a line-drawn portrait after the Mona Lisa, built from the
same box-drawing set as the UI, with the smile as the only copper stroke
(`1a`). It appears at startup and in the empty state, nowhere else. Recurring
motifs across the product: `Δ` for change, `◉` for the thing in hand, Roman
numerals for plans, proportion meters instead of raw counts, and the Latin
instrument names.
