# davinci TUI — Ratatouille implementation

An Elixir/[Ratatouille](https://github.com/ndreynolds/ratatouille) implementation of
the davinci terminal UI described in `../design.md` and mocked in
`../Pi TUI Mockups.dc.html`.

```
mix deps.get
mix run run.exs
```

**Windows cannot run this.** Ratatouille draws through `ex_termbox`, whose NIF
compiles termbox from C against `termios.h`; there is no Windows build. Elixir
itself installs fine (Erlang/OTP 28 + Elixir 1.20 work), but `mix deps.compile`
dies at `nmake`. Use WSL, a Linux box, or a container.

Verifying it without a terminal — compiles every module and renders every
screen at 80, 100, 120 and 160 columns, reporting any row that overflows its
width. `MAKE=/bin/true` skips the termbox NIF, which rendering does not need:

```
docker run --rm -v "$PWD:/app" -w /app \
  -e MIX_DEPS_PATH=/tmp/deps -e MIX_BUILD_PATH=/tmp/build -e MAKE=/bin/true \
  elixir:1.16 sh -c "mix local.hex --force && mix local.rebar --force && \
    mix deps.get && mix compile --warnings-as-errors && mix run --no-start check.exs"
```

`elixir:1.16` rather than a newer tag on purpose: termbox's bundled waf needs
Python ≤ 3.10 for a real NIF build, and newer images ship 3.11+.

Requires a terminal at least 80 columns wide. **ctrl+d quits** — `q` and ctrl+c
are deliberately not quit events, because every printable key belongs to the
composer and ctrl+c interrupts the run, never the app (design.md §6).

## Keys

| Key | Surface | Screen |
|---|---|---|
| ctrl+p | Instrumenta — command palette | 1d |
| ctrl+e | Codex — workspace sidebar (≥120 cols) | 1e |
| ctrl+s | Memoria — sessions | 1f |
| ctrl+o | Cogitator — model picker | 1f |
| ctrl+l | Disegno — plan sheet | 1c |
| ctrl+g | Grafo — dependency study | 2a |
| ctrl+r | Memoria — vector recall | 2b |
| ctrl+u | Mensura — token governor | 2c |
| esc | close the instrument in hand | — |
| ↑ ↓ | move the selection in the open list | — |
| enter | send the composer / run the selection | — |
| ctrl+c | interrupt the run | — |
| ctrl+d | quit | — |

Startup (1a) shows whenever the transcript is empty. Type and press enter to
append a turn.

## Commands

The configuration surfaces are opened from the composer, by name, the way the
product opens them — typing one and pressing enter switches screen instead of
appending a turn. `esc` closes whichever is open.

| Command | Surface | Screen |
|---|---|---|
| `/model` | Cogitator — the full model catalog, with credentials and cost | 3a |
| `/settings` | Settings — one value ramp per setting, user and project scope | 3b |
| `/thinking` | Cogitator — thinking level as a budget, and what it becomes per provider | 3c |
| `/login` | Cogitator — device-code flow over the provider credential ledger | 3d |
| `/hotkeys` | Keys — the whole keymap, grouped by surface | 3e |
| `/resume` | Memoria — the session list, and what resuming one carries | 4a |
| `/tree` | Memoria — the session as a tree, forks and all | 4b |
| `/compact` | Mensura — what compaction keeps, folds and costs | 4c |
| `/export` | Memoria — export and share, with the redaction ledger | 4d |
| `/graph` | Grafo — a task run as a graph of isolated workers | 5a |
| `/memory-status` | Memoria — the vector index itself | 5b |
| `/governor-status` | Mensura — what the governor did to your tool output | 5c |
| `/sec-report` | Securitas — a scan you can audit | 5d |
| `/trust` | Fiducia — what a project would load, before it is trusted | 6a |
| `/reload` | Officina — what is loaded, what failed, what it costs | 6b |
| `/diff` | Δ review — every file the turn changed | 6d |

`ctrl+c` opens 6c: it interrupts the run and shows what the interrupt did — what
was kept, what was billed, what is still on disk. `/diff` is this design's name
for 6d; the product has no such command yet.

Screens `3a`–`3e` are drawn in `../davinci-tui-instruments` (the canvas of new
screens) rather than in `Pi TUI Mockups.dc.html`, which stops at `2c`.

Their footers state the product's exits, which is what the design specifies;
this fixture app implements ↑↓, enter and esc, plus ↑↓ scrolling on the keymap
sheet. The rest — `←→ value`, `tab scope`, `k paste api key` — is drawn, not
wired, and a stray letter key goes to the composer as it does anywhere else.

`ctrl+m` from the spec is unreachable: termbox reports it as the same code as
enter (13), so vector recall is bound to **ctrl+r**.

## Module map

| Module | Responsibility |
|---|---|
| `Davinci.CLI` | `Ratatouille.run/2`, quit events |
| `Davinci.Term` | 256-color negotiation, `NO_COLOR`, `--no-animation` |
| `Davinci.Theme` | the only place a color literal lives; dim ramp; glyph vocabulary |
| `Davinci.Ui` | char-grid primitives: segments, boxes, meters, measure, wrapping |
| `Davinci.Model` | state, breakpoints, composer reducers, session fixtures |
| `Davinci.App` | init/update/render, key routing, bottom-anchored layout |
| `Davinci.Views.Chrome` | header, composer, status bar |
| `Davinci.Views.Transcript` | user turns, tool lines, prose, Δ blocks |
| `Davinci.Views.Studio` | mid-turn ledger; collapses below 100 cols |
| `Davinci.Views.{Disegno,Instrumenta,Codex,Memoria,Cogitator,Grafo,Mensura}` | one instrument each |
| `Davinci.Views.{Settings,Thinking,Login,Keys}` | the configuration screens (3a–3e) |
| `Davinci.Views.{Resume,Tree,Compact,Export}` | the session screens (4a–4d) |
| `Davinci.Views.{GraphRun,Vectors,Governor,Securitas}` | the instruments (5a–5d) |
| `Davinci.Views.{Trust,Officina,Recovery,Diff}` | trust, workshop, interrupt, Δ review (6a–6d) |
| `Davinci.Views.Startup` | identity mark and empty state |

## Decisions worth knowing

**Colors.** The design's palette is truecolor; termbox tops out at 256. `Davinci.Term`
asks for the 256-color output mode and reports which palette to build:
`:ansi256` (xterm indices nearest the spec's tokens) or `:basic` (8 named
colors) if the mode is unavailable. Both are defined in `Davinci.Theme`; no
other module names a color. `NO_COLOR=1` switches the whole ramp to greyscale
and turns on bold for active glyphs — state was never color-only, so nothing is
lost (design.md §9, screen 1h).

**Panels are hand-drawn.** Ratatouille's `panel` colors only its title and always
draws its border in the default color, and it can't notch metadata into the
top-right rule. Since the design needs bordered surfaces in `border`,
`primary` (composer) and `warning` (governor proposal), with labels notched into
the rule, `Davinci.Ui.box/1` builds them from `label`/`text` runs on the
character grid instead. That also lets every surface report its exact row count.

**Rows are counted, not guessed.** Every view returns a flat list of one-row
`label` elements, so `Davinci.App.body/2` can subtract the composer's height,
tail-truncate the transcript like a scrollback, and anchor the composer to the
bottom at any window height. The one exception is Codex, which returns a single
`row`/`column` grid — the only persistent split in the product.

**Overlays are drawn, not floated.** Ratatouille's `overlay` element owns its own
inset and fill; the design wants the transcript visible behind the palette with
the ramp dropped (design.md §2). So overlays render the transcript through
`Theme.dim/1` and then draw an inset box over it — same result, full control of
the notch and the tinted selection row.

**Motion.** Two animations, as specified: the caret blink (~1s, step-end) and one
4-frame spinner on the single active Studio step, both derived from the 250ms
runtime tick — so nothing animates per-widget, and `--no-animation` /
`DAVINCI_NO_ANIMATION=1` makes both static.

**Responsive.** `Davinci.Model` owns the breakpoints: <80 transcript and composer
only, overlays full width; <100 Studio collapses, ASCII art and annotations
dropped, status bar abbreviates to `^p`; ≥120 Codex sidebar allowed; ≥150 the
git changes popover appears under the transcript.

## Replacing the fixtures

Screen content lives in `Davinci.Model` (`transcript/0`, `plan/0`, `corpus/0`,
`sessions/0`, `models/0`, `tree/0`, `impact/0`, `recall/0`, `budget/0`,
`catalog/0`, `settings/0`, `thinking_levels/0`, `providers/0`, `device_code/0`,
`keymap/0`, `resume_sessions/0`, `session_tree/0`, `compaction/0`,
`export_ledger/0`, `graph_run/0`, `vector_index/0`, `governor/0`,
`security_scan/0`, `project_trust/0`, `workshop/0`, `failed_run/0`, `review/0`)
as plain data. Point those at your session store, graph and token accountant; the view
modules take data and a theme and know nothing else.
