# davinci instruments — fidelity pass design

The seventeen command sheets (`3a`–`6d`) exist in `pi-tui` but their frame
differs from the mockups. This pass makes the Rust TUI match the
*davinci TUI Instruments* design canvas row for row.

## Source of truth

- `docs/ui/Pi TUI Instruments.dc.html` — the design canvas, vendored
  byte-identical from claude.ai artifact `64f6f1f2-7cd3-478e-9602-8d5350c0447a`.
  Seventeen artboards: `3a` Cogitator model, `3b` Settings, `3c` Thinking,
  `3d` Provider auth, `3e` Keys, `4a` Memoria resume, `4b` Memoria tree,
  `4c` Mensura compaction, `4d` Export, `5a` Grafo worker graph, `5b` Memoria
  index, `5c` Mensura governor, `5d` Securitas scan, `6a` Fiducia trust,
  `6b` Officina, `6c` Interrupt and recovery, `6d` Δ review.
- The artboard text is the row contract. Extract it with:

  ```python
  import re, json, html
  s = open("docs/ui/Pi TUI Instruments.dc.html", encoding="utf-8").read()
  doc = json.loads(re.search(r'<script[^>]*id="appifact-doc"[^>]*>(.*?)</script>', s, re.S).group(1))
  for name, src in doc["content"]["files"].items():
      if not name.endswith(".dc.html"): continue
      t = re.sub(r"<style.*?</style>", "", src, flags=re.S)
      t = html.unescape(re.sub(r"<[^>]+>", " ", re.sub(r"</div>", "\n", t)))
      print("=====", name); print("\n".join(l.strip() for l in t.splitlines() if l.strip()))
  ```

- `docs/ui/design.md` gains §11 *Command sheets* carrying the frame rules
  below. Where design.md and an artboard disagree, the artboard wins
  (CLAUDE.md rule). One known disagreement: the meter tip.
- The Elixir tree `docs/ui/davinci_tui/` stays as it is and is no longer the
  reference for `3a`–`6d`. The Rust view doc comments for those screens cite
  the artboard (`Mirrors artboard 3a of docs/ui/Pi TUI Instruments.dc.html`)
  instead of the `.ex` file. CLAUDE.md is updated to say so.

Paths stay `.pi\…` where the artboards write `.davinci\…`: the binary is `pi`
and shares `~/.pi` with TypeScript `pi`. Everything else is matched verbatim.

## Frame rules

These hold for every command sheet. They live in one place, a
`SheetChrome` descriptor (§ Architecture), not in seventeen views.

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
   `label ━━━◸ ─── used/cap` shape.
4. **The hint row is the last body row**: border colour, hints separated by
   ` │ `, the escape hint (`esc close`, `esc cancel`, `esc done`,
   `esc leave it`) right-aligned. When the row is too wide, hints drop from
   the end; the escape hint never drops. The hint row never scrolls off.
   Where the artboard draws its keys inside a panel (`4c`, `4d`, `6a`), that
   panel's key row is the hint row and no separate row is drawn.
5. **Composer only where the artboard draws one.** Hidden on `3b`, `3d`,
   `3e`, `4a`, `4b`, `4d`. A filter box in the body instead on `3a` (`4a` has
   a `⌕` filter box as its first row, same shape). Prompt with the artboard's
   placeholder or command on the rest. `6a` draws the composer disabled, its
   text `the composer is disabled until you decide` in the dim ramp. The
   composer's own hint row (`enter send … esc close`) is not drawn while a
   sheet is open; the sheet's hint row is the only hint row.
6. **Overflow windows around the selection**, `… n above` / `… n below` in
   border colour, the existing `3b` behaviour moved into `ui::window` and
   used by every list sheet. Panels and footnotes stay whole; only the list
   windows.
7. **Meter tip is `◸`**, everywhere, replacing `╸`. The `1a`–`2c` mockups
   already use `◸` (fourteen times); design.md §6 was wrong. `NO_COLOR` keeps
   the glyph.
8. **Selection is the Instrumenta mark**: the 3-cell copper left bar plus a
   `surface` tint across the row, reused from `instrumenta.rs`, on every
   sheet with a cursor (`3a`, `3b`, `3c`, `3d`, `4a`, `4b`, `5a` workers,
   `5c` outputs, `5d` findings, `6a` files, `6b` extensions, `6d` files).
   The `◉` state glyph stays as well; colour is never the only signal.
9. **Rows the artboard dims** (no credential on `3a`, absent providers on
   `3d`, dismissed findings on `5d`, harmless files on `6a`) render in the
   theme's dim ramp.
10. **Column headers** are uppercase in border colour with a hair rule under
    them in the dim ramp's border colour, as the artboards draw
    `PROVIDER / MODEL … CREDENTIAL`.
11. **Footnotes are two columns**: left in text or muted, right in border,
    via `spread`. Below 100 columns the right column wraps under the left.
12. **Panels keep `╭─ LABEL ─╮`**, the terminal translation `1c` and `2c`
    established for the artboards' CSS boxes. Labels stay uppercase.
13. **Facts that are not known live are omitted**, never invented. A header
    run with one missing fact drops that segment; a status third that cannot
    be computed drops to two segments.

## Per-sheet contract

Each row: header right · status left third · status right · hints · escape ·
composer · body deltas against the current Rust view. Bodies not listed here
already match the artboard's rows.

**3a Cogitator · model** (`/model`, `ctrl+o`)
`cwd │ branch │ sonnet` · `ring of 3` · context · `↑↓ move │ enter select │ ctrl+p cycle ring │ s scope to ring` · `esc close` · filter box, no composer.
Body: filter box first row, `12 of 63 shown · 6 of 10 providers ready` right-aligned inside it. Ring membership as text ` · in ctrl+p ring` after the model name, not a `◉`. `router :8080` after `llama.cpp / qwen-coder`. Price column header `$/Mtok in · out`. Credential column full text (`! token expired`, `○ no credential`). Footnotes two columns: `dimmed rows have no credential · /login xai to add one` ⟷ `switching keeps the transcript · re-primes the cache`; `catalog refreshed ✓ 2h ago · %USERPROFILE%\.pi\agent\models.json` ⟷ `! 128k of context will not fit a 32k window` (warning, only when the selected model's window is smaller than the current context).

**3b Settings** (`/settings`)
`scope user │ project │ tab switches` · `24 keys` · context · `↑↓ setting │ ←→ value │ tab scope │ r reset to default` · `esc close` · none.
Body: scope row leaves the body (it is the header run). Selected value drawn as a filled chip, other values muted, the artboard's `✓ on` for booleans. Description under the selected row in muted, one wrapped paragraph. Footnotes two columns: `user %USERPROFILE%\.pi\agent\settings.json` ⟷ `written on change · no restart`; `project .pi\settings.json · overrides user · 1 key set` ⟷ `a flag beats both scopes, for one run`. Rows `Show images`, `Image width`, `Skill commands … registers /skill:name` present.

**3c Thinking** (`/thinking`)
`sonnet │ budget knob │ reserve 10k` · `thinking medium` · context · `↑↓ move │ enter select │ shift+tab cycle │ ctrl+t toggle off` · `esc close` · prompt `/thinking high`.
Body: fourth column header `SONNET → GPT`. Footnotes: `last turn thought 5.1k of 8k · ✓ under budget` ⟷ `thinking is billed as output · ◐ 38% of this session's output tokens`. Panel `WHAT THE LEVEL DOES` rows keyed `anthropic │`, `openai │`, `google │` with the provider in secondary.

**3d Provider auth** (`/login`, `/logout`)
`credentials in %USERPROFILE%\.pi\agent\auth.json │ 0600` · `4 of 10 ready` · `mensura ◐ 23% · esc close` · `enter re-authenticate │ k paste api key │ d /logout provider │ r refresh now` · `esc close` · none.
Body: echo `> /login anthropic`. Panel `DEVICE AUTHORISATION · ANTHROPIC` with the code in its own box on the right (`WQPT - FJ4M`, letter-spaced, `expires in 8m 41s` under it), steps `1 · open …`, `2 · enter the code below`, `3 · davinci writes the refresh token and returns here`, then the spinner row `waiting for approval · polled 6×` ⟷ `ctrl+c cancels the login, not the session`. Provider table with `STATE` right-aligned and its glyph (`◉ pending`, `○ absent`). Footnotes: `keys are never echoed, never written to the transcript, never sent to another provider`; `davinci auth print-bearer-token --provider openai-codex` ⟷ `hands one to an external client`.

**3e Keys** (`/hotkeys`)
`39 bindings │ 4 surfaces │ keybindings.json` · `/reload re-reads them` · context · none (the artboard has no hint row; the sheet scrolls with `↑↓`) · `esc close` drawn alone on the hint row · none.
Body: group titles `INSTRUMENTS · OVER THE TRANSCRIPT`, `RUN · WHILE THE AGENT WORKS`, `COMPOSER`, `SESSION LIST · INSIDE MEMORIA`, `SESSION TREE` (title in primary, note after ` · ` in border). Session tree group present (`ctrl+← · ctrl+→`, `shift+l`, `ctrl+d t u l a`). Footnotes: `a key means one thing per surface · ctrl+d quits the shell, deletes in the session list`; `rebind in %USERPROFILE%\.pi\agent\keybindings.json`.

**4a Memoria · resume** (`/resume`)
`this project │ 34 sessions │ 1.2 GB on disk` · `34 sessions` · `disk ━━◸ ─── 1.2G/8G` · `enter resume │ f fork │ ctrl+r rename │ ctrl+s sort │ ctrl+p paths │ ctrl+d delete` · `esc close` · none.
Body: `⌕ filter sessions…` box first, then `6 of 34 shown · sort recent · named only off` in border. Selected session is two rows, the second `forked from provider-parity at turn 12 · Δ7 files · 3 branches · a3a6f31` under the tint. Warning row under `fix-git-hooks`. Footnotes: `selected review-agent-runtime · last message “…”`; full session path ` · 1.8 MB`; `resuming replays the transcript, not the tools` ⟷ `f forks instead of continuing`. Disk cap for the meter: the session dir's volume size, else the meter is omitted.

**4b Memoria · tree** (`/tree`)
`review-agent-runtime │ 42 turns │ 3 branches` · `turn 05 of 42` · context · `↑↓ move │ enter switch to turn │ ctrl+←/→ fold │ shift+l label │ f fork here` · `esc close` · none.
Body: filter chips row `filter  all  no tools  user only  labeled  timestamps on` (active chip filled). Panel `MEMORIA · SESSION TREE` right-notch `3 branches · 1 abandoned · ✓ nothing lost`. Nodes carry a second row where the artboard has one (`abandoned · 2 files reverted`, `Δ 3 +42 -11 label: store-fix`, `branched from 04 · own transcript from here`), `◀ here` on the current turn. Footnotes: `turn 05 · what resuming here would carry`; `context at this point 47k/200k · cost so far $0.84`; `✓ 4 user turns, 4 agent turns, 11 tool results · nothing compacted yet`; `↳ working tree is ahead of this turn · 2 files changed since · the tree does not move your files`; `! branch 06 has its own 9 turns and will not merge back`.

**4c Mensura · compaction** (`/compact`)
`auto-compact on │ threshold 92% │ sonnet` · `1 proposal` · context (184k/200k) · `[enter] compact now [e] evict tool output only [t] raise the threshold [esc] leave it` drawn inside the cost panel, hint row absent · `esc leave it` · prompt `ask davinci…`.
Body: echo `> /compact keep the store.rs decisions verbatim`. `now` / `after` meters with `! 92% of 200k` warning and `31% of 200k` muted. `KEPT VERBATIM` and `FOLDED INTO ONE NOTE` side by side at ≥100 columns (two `Surface`s of half width), stacked below. Folded rows use `−` not `×`; last row `the note itself costs about 1.4k` in border. Footnote `compacted 2× this session · last at turn 24 · recovered 88k` ⟷ `/tree still shows every folded turn`.

**4d Export & share** (`/export`)
`review-agent-runtime │ 42 turns │ 1.8 MB jsonl` · `exporting` · context · none · `[esc] done` inside the share panel · none.
Body: echo `> /export review-agent-runtime.html`. Format chips `.html .jsonl gist` with the note `one page, no assets, opens offline`. Progress `✓ wrote 42 of 42 turns ━━━◸ 2.9 MB · 1.4s`. Panel `SHARE · SECRET GIST` rows as the artboard. Footnotes `.jsonl round-trips · /import resumes it on any machine`; `exports are written next to the cwd, never to the session store`.

**5a Grafo · worker graph** (`/graph`)
`run g-7f2a │ complex │ milestone 2 of 4` · `implement` · `run cost ━━◸ ─── $1.31/$8.00` · `enter open artifact │ v tail a worker │ r resume a stopped run │ a abort` · `esc close` · prompt `/graph-view t6`.
Body: echo `> /graph add … --complex`. Stage strip `✓ classify ── ✓ investigate ── … ○ done` with `6m18s elapsed` right-aligned. Panel right-notch `7 tasks · 3 parallel · 0 blocked`. Worker table columns `worker · policy · artifact · ↑ ↓ $ time`, spinner on the running worker. Ledger: `cost $1.31 of $8.00 ━━◸`, `workers 6 of 12 · at most 3 at a time`, `revision cycles 0 of 2 · replans 0 of 1`, `no run deadline · per-role timeouts unlimited`, `artifacts in .pi\graph\g-7f2a\` ⟷ `ctrl+c aborts the run, keeps the artifacts`.

**5b Memoria · index** (`/memory-status`)
`18,402 records │ 3 shards │ bge-small 384d` · `retrieval on` · `index ━━◸ ─── 6.9k/18.4k` · `enter search │ i reindex │ t toggle automatic retrieval │ x clear this repo` · `esc close` · prompt `/memory-search interrupt handling`.
Body: two lead rows `this repository davinci-rust holds 6,914 of them`, `retrieval automatic · at most 1.5k tokens injected per turn`. Kind meters as now. Panels `WHERE IT LIVES` (`embeddings`, `vectors`, `extraction`, `config` as key column in muted) and `HEALTH`. Footnotes `relevance floor 0.70 · k=6 from 60 candidates · hybrid dense + lexical`; `injected this session 14 chunks · 8.4k tokens · shown in the transcript as ⌕ lines`; `memory-clear drops this repo's 6,914 records` ⟷ `it asks first, and it cannot be undone`.

**5c Mensura · governor** (`/governor-status`)
`governor on │ session 01JB2K │ since 11:04` · `31 compressed` · context · `enter open an output │ d dedupe on/off │ l anti-loop on/off │ r reset counters` · `esc close` · prompt `/governor-status`.
Body: four stat rows each two lines (`31 of 96 results` / `compressed head 40 · tail 40 · the rest on disk`), the number in text and the rest muted. Outputs table header `31 outputs · 2.8 MB · 4 newest shown · dropped when the session ends`, columns id · tool · call · lines · size. Footnotes: `compresses above 8 KB or 300 lines · keeps 40 head, 40 tail, 20 lines it judges important`; `nothing is deleted — the full output is on disk and the model can ask for any range of it`; `%USERPROFILE%\.pi\outputs\01JB2K\` ⟷ `governor-reset clears the counters, not the store`.

**5d Securitas · scan** (`/sec-report`)
`scan s-31c8 │ draft │ network ✓ not used` · `1 critical` · `scanned ━━━◸ ─── 1842/2140` · `enter open the file at the line │ f mark false positive │ p show attack path │ a abort scan` · `esc close` · prompt `/sec-report --severity high`.
Body: spinner row `validating candidate 31 of 44` with its own meter, `1,842 files · 96 skipped · 41.2 MB read`. Severity tally as a row of `critical 1  high 3  medium 6  low 9  informational 14` each in its colour. Findings with path and severity right-aligned, the expanded finding's `rule` and `path` rows beneath. Footnotes `every finding was read out of the file, not guessed · line and evidence attached`; `the scan never left this machine · allow_network false`; `report sealed ✓ sha256 4b1f … c9e0 · 214 KB` ⟷ `.pi\security\s-31c8\report.json`.

**6a Fiducia · trust** (`/trust`, first visit)
`C:\dev\clones\vendor-cli │ main │ first visit` · `untrusted` · `no tools loaded · 0k/200k` · none (the decision keys live in the panel) · none · disabled composer `the composer is disabled until you decide`.
Body: lead paragraph. File rows with the effect column right-aligned in its colour (`executes code` warning, `changes limits` warning, `prompt text` muted, `harmless` border). Panel `DECIDE ONCE` as now with `decision is per path · C:\dev\clones\vendor-cli` ⟷ `changeable later with /trust`. Footnotes `trusted so far 14 projects · ignored 2 · asked again when a path moves`; `%USERPROFILE%\.pi\agent\trust.json · paths and decisions, nothing else`; `--approve trusts for one run without asking` ⟷ `--no-approve is the safe default for scripts`.

**6b Officina** (`/reload`)
`24 tools │ 37 commands │ 21.4k of schema` · `1 failed` · context · `enter open the source │ r reload again │ d disable one │ e show its error` · `esc close` · prompt `ask davinci…`.
Body: echo `> /reload`. Reload ledger as now, the failure's detail rows `TypeError: … · deploy.js:41` and `its 3 tools are not registered; everything else loaded`. Panels `NATIVE · RUST, ALWAYS ON` (right-notch `0ms`, last row `· built in — no node, no install`) and `JAVASCRIPT · NODE SUBPROCESS` (last row `· node v24.19.0 · one process, reused` ⟷ `318ms`). Schema meters with `what every turn carries` ⟷ `21.4k of the window is tool schema · 11%`. Footnotes `-nt disables all tools · -t read,grep,ls keeps three · -xt bash drops one`; `/reload does not restart the session or lose the transcript`.

**6c Interrupt & recovery** (`ctrl+c`)
`cwd │ branch │ sonnet` · `interrupted` · context (58k/200k) · `enter send │ shift+enter newline │ alt+enter queue for after the retry` · `esc cancel` · prompt `continue, but do the sse path first`.
Body: echo `> rewrite the provider adapter to stream`, agent mark, three tool rows. Panel `THE TURN DID NOT COMPLETE` in error colour: `× anthropic returned 429 mid-stream. Retry-After says 12s; this is attempt 2 of 4, backing off 2s, 6s, 12s.`, ledger row `kept 1,204 tokens of reply   files written 0   billed $0.04   session written ✓`, spinner row `retrying in 9s` ⟷ `[enter] retry now [m] finish on opus [esc] stop retrying`. `> ctrl+c` echo. Panel `INTERRUPTED` in warning colour as now, its last row `a second ctrl+c within a second clears the composer; ctrl+d quits` in border, no glyph. Closing paragraph as now.

**6d Δ review** (`/diff`)
`7 files │ +145 -127 │ main · 3 commits behind` · `Δ7 +145 -127` · context · `↑↓ file │ j k hunk │ enter open in codex │ u revert hunk │ c commit` · `esc close` · prompt `fix the two legacy.rs references, then commit`.
Body: file rows with `+n` in success and `-n` in error as two right-aligned columns, then the note column. New file row uses `+` glyph. Hunk pane: rule row `Δ path +64 -19` ⟷ `hunk 2 of 5 · j k to move`, then `@@ 214,7 +214,18 @@ impl OpenAiProvider` in border, then the hunk behind a single left rule as now. Footnotes `! legacy.rs is gone but 2 files still name it · grafo says the build will fail` ⟷ `revert is per file and per hunk`; `✓ 212 of 212 tests pass on the changed crates · 41.2s` ⟷ `nothing here is committed until you say so`.

## Architecture

**`pi-tui/src/davinci/views/sheet.rs`** (new) — the descriptor:

```rust
pub struct SheetChrome {
    pub header_right: Vec<Span<'static>>,      // empty = cwd │ branch │ model
    pub status_third: Option<Vec<Span<'static>>>,
    pub status_right: Option<Vec<Span<'static>>>, // None = context meter
    pub hints: Vec<Vec<Span<'static>>>,        // joined with " │ "
    pub escape: Option<&'static str>,          // "esc close" …, right-aligned
    pub composer: Composer,                    // Hidden | Prompt | Filter | Disabled(&str)
    pub echo: Option<String>,                  // "> /compact …" first row
}
pub fn chrome(model: &Model) -> Option<SheetChrome>; // None for Agent/Plan/Grafo/Memoria/Mensura/Codex
pub fn hint_row(width: u16, chrome: &SheetChrome, theme: &Theme) -> Line<'static>;
```

Each sheet view exposes `pub fn chrome(model: &Model) -> SheetChrome`
beside its `lines`; `sheet::chrome` dispatches on `model.screen`. The
`1a`–`2c` screens and the overlays are untouched and return `None`.

**`chrome.rs`** — `header` and `status` consult `sheet::chrome` first; the
`Memoria` and `Mensura` arms that exist today move into their views'
`chrome` functions. `composer` gains the `Composer` modes; the hint row
under the composer is skipped whenever `sheet::chrome` is `Some`.

**`app.rs`** — `panel` becomes: echo row, then `ui::window(rows, height − 1,
anchor)`, then the hint row. `body` no longer pads a sheet from the top or
draws the transcript behind it. `composer_rows` obeys `Composer`.

**`ui.rs`** — `window(rows, height, anchor) -> Vec<Line>` (moved from
`settings.rs`), `column_header(width, columns, theme) -> [Line; 2]`,
`footnote(width, left, right, theme) -> Vec<Line>` (spread, wraps under
100 columns), `selection_bar` re-exported from `instrumenta.rs`. `meter`
draws `◸`; `theme::METER_TIP` changes.

**`fixtures.rs`** — every sheet fixture carries the new facts (counts,
disk bytes, commits behind, elapsed, schema tokens) so the audit dump renders
the artboard's numbers.

**`pi-coding-agent/src/davinci_session.rs`** — the `open_*_sheet` functions
fill the new `Model` fields from live data: catalog shown and total, providers
ready and total, settings key count, session count and disk bytes (walk the
session dir), commits behind (`git rev-list --count HEAD..@{upstream}`,
omitted when there is no upstream), tool schema token estimate (sum of
serialised tool specs ÷ 4), run elapsed time. Anything unavailable is left
`None` and the segment is omitted (rule 13).

## Testing

- The `dump_every_screen_for_the_mockup_audit` frame dump stays the visual
  audit. Its `3a`–`6d` frames are compared by eye against the artboards.
- `sheet.rs` tests, one per sheet, on the fixture model: header right run
  equals the artboard's facts; status left has the artboard's third segment;
  status right is the sheet's meter or the context meter; the hint row is the
  last body row, contains every hint, ends with the escape hint at the right
  edge; the first body row is not blank; the composer is present or absent
  per rule 5.
- `ui.rs` tests: `window` keeps the anchor visible and counts both folds;
  `column_header` is two rows of exactly the width; `footnote` is one row at
  100 columns and two below; `meter` ends in `◸` when full. Existing tests
  asserting `╸` are updated.
- View tests already present keep passing with their assertions adjusted to
  the new rows. No test touches the network or the real `~/.pi`.

## Out of scope

- The `1a`–`2c` screens, the overlays (`1d`, `1f`) and the Codex split.
- The `/mcp` and `/permissions` sheets, which have no artboard. They adopt
  the frame rules (a `SheetChrome` each) but their bodies are not changed.
- Editing the Elixir reference tree.
- Behaviour: no new keys or actions beyond what the views already handle.
  Hints name keys that already work; a hint for a key that does not yet work
  is drawn in the dim ramp.
