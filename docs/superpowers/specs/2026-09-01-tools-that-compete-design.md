# Tools that compete — phase 3 design

Date: 2026-09-01. Branch: `rust-rewrite`. Roadmap:
`2026-09-01-competitive-harness-roadmap.md`, row 3. Builds on phase 1 (live
turns) and phase 2 (the permission gate).

## Why

With turns that stream and a gate that asks, the harness still lacks the
tools Claude Code and Codex CLI users reach for every hour: a shell command
that runs while the conversation continues, a way to read a web page or
search the web, a ledger the model keeps of what it is doing, notebooks that
read as cells rather than as JSON, and a transcript that shows what a tool
came back with without burying the answer. Every one of these is visible in
the first ten minutes of use, which is where a harness is judged.

## What ships

| Piece | Where | Outcome |
|---|---|---|
| Background jobs | `pi-agent/src/jobs.rs`, `tools.rs` | `bash` / `powershell` take `background: true`; `job_output` and `job_kill` follow it; a finished job is announced to the model on its next turn and to the user at once. |
| Web | `pi-agent/src/web.rs` | `web_fetch` (page → readable text) and `web_search` (Brave with a key, DuckDuckGo without). Fixture-driven in tests. |
| Todo ledger | `pi-agent/src/todo.rs` | `todo` tool holds the model's plan; davinci draws it as the STUDIO ledger and the `1c` plan sheet; `/todo` lists it. |
| Notebooks | `tools.rs` (`read`, `edit`), `notebook.rs` | `.ipynb` reads as numbered cells with outputs; `edit` matches inside cell sources; `notebook_edit` inserts, replaces and deletes cells. |
| Collapsible output | `pi-tui` transcript | Every tool line keeps what came back; `ctrl+t` (and a `/settings` row) shows or hides it. Failures keep their four rows. |
| Highlighted diffs | `pi-agent/src/edit_diff.rs`, `pi-tui/src/davinci/views/highlight.rs` | `edit` and `write` return a diff; davinci draws a Δ block after each with keywords, strings, comments and numbers in their own ink. Fenced code and the `/diff` sheet use the same lexer. |

No new dependency. Diffing is a Myers line diff written here; HTML is read
by a tolerant tag scanner; the lexer is a table of keywords per language.
`similar` stays declared and unused; `syntect` would bring its own colour
tables into a design that allows colour literals in one file.

## Background jobs

### Tool surface

`bash` and `powershell` gain an optional `background: boolean`. When true the
command is spawned exactly as a foreground one (same shell, prefix, cwd,
transport) and the call returns at once:

```
Started background job 1: `cargo build --release`
Read its output with job_output {"jobId": 1}; stop it with job_kill.
```

with `details: { jobId, pid, command }`. `timeout` is ignored for background
jobs (the job runs until it exits or is killed).

`job_output { jobId, wait?: seconds, tail?: lines }` returns the output
collected so far (both streams, in arrival order), then a status line —
`[job 1 running · 12.4s]` or `[job 1 exited 0 · 31.2s]`. `wait` blocks up to
that many seconds (cap 600) for the job to exit before answering. `tail`
returns only the last N lines. Output is capped at 4 MB in memory; the head
is dropped and a `[earlier output dropped]` line leads the buffer.

`job_kill { jobId }` kills the process tree (Windows: `taskkill /T /F`;
Unix: the process group) and answers with the exit status and how long the
job ran. `job_output` on an unknown id is an error naming the known ids.

### Book-keeping

`Agent.jobs: Arc<Mutex<JobBook>>` (`jobs.rs`): ids from 1, per job the
command, start `Instant`, `Child`, output buffer shared with two reader
threads, and `Status { Running, Exited(i32), Killed }` set by a waiter
thread. The book is shared because the tool thread writes it, the davinci
loop reads it every tick, and the loop injects notices from it. All jobs are
killed when the book is dropped (`pi` exits) — nothing outlives the
session.

### Notices

The model learns a job finished at its next opportunity, never mid-tool:

- Inside a running loop: at the top of every iteration, after steering
  messages are injected, `inject_job_notices` appends one user message per
  finished, un-announced job.
- Between turns: `prompt()` prepends the same messages before the user's
  text, so a job that finished while the user was typing is in context when
  they send.

The message is `role: user`, `customType: "backgroundJob"`, text:

```
[background job 1 finished · exit 0 · 31.2s] cargo build --release
    Compiling pi-coding-agent v0.1.0
    Finished `release` profile [optimized] target(s) in 30.9s
```

with the last 20 lines of output (or `(no output)`). It is persisted like
any user message so a resumed session still shows why the model said what it
said next.

The user learns at once. davinci polls the book on its 250 ms tick and, for
a job newly finished, pushes `⎿ ✓ job 1 finished · cargo build --release ·
exit 0 · 31.2s` (× on a non-zero exit) as a `manus` tool row, plus its last
four lines as detail when it failed. While jobs run the status bar's left
run carries `· 2 jobs`. `/jobs` lists every job of the session with id,
state, elapsed and command; `/jobs kill <id>` stops one. The legacy chrome
and RPC receive the notices as messages and print nothing extra.

## Web

### `web_fetch { url, maxChars? }`

- `http`/`https` only; anything else is an error. Redirects followed (5).
  Timeout 30 s. Body capped at 10 MB. User agent `pi-rust/<version>`.
- `text/html`: converted to readable text. `<script>`, `<style>`,
  `<noscript>`, `<svg>`, `<nav>`, `<header>`, `<footer>` and comments are
  dropped; headings become `#`, `##`, …; `<p>`, `<div>`, `<li>`, `<tr>`,
  `<br>` break lines; `<li>` is prefixed `- `; `<a href>` renders as
  `text (url)` when the text and the resolved absolute URL differ; `<pre>`
  keeps its whitespace and is fenced; `<code>` is back-ticked; entities are
  decoded (named common set + numeric); whitespace is collapsed elsewhere.
  The `<title>` leads the output.
- `application/json`, `text/*`: passed through.
- Other content types: an error naming the type.
- Output truncated head-first to 2000 lines / 50 KB (`truncate_read`), or
  `maxChars` when given. `details: { url, finalUrl, status, contentType,
  truncation }`.

### `web_search { query, limit? }`

- Provider: Brave Search when `BRAVE_API_KEY` is set (or settings
  `webSearch.braveApiKey`), else DuckDuckGo's HTML endpoint
  (`https://html.duckduckgo.com/html/?q=…`) parsed for result anchors and
  snippets. `limit` defaults to 8, cap 20.
- Output: numbered rows, `1. title\n   url\n   snippet`. `details: {
  provider, query, results: [{ title, url, snippet }] }`.
- The system prompt tells the model to `web_fetch` a result before quoting
  it.

### Offline and fixtures

`PI_OFFLINE=1` or `PI_DISABLE_NETWORK=1` makes both tools fail with
`network disabled`. `PI_WEB_FETCH_FIXTURE=<json path>` maps `url → { status,
contentType, body, finalUrl? }`; `PI_WEB_SEARCH_FIXTURE=<json path>` maps
`query → [ { title, url, snippet } ]`. Tests only ever run against fixtures;
the HTML reader and the DuckDuckGo parser are tested on saved pages.

### Permission

New `ToolClass::Network` for both tools. `read-only` allows them (nothing in
the workspace changes); `ask` and `edits` ask; `auto` allows. The subject is
the host for `web_fetch` (rule `web_fetch(docs.rs)`, session rule
`web_fetch(<host>)`) and the query for `web_search` (session rule
`web_search`). Deny rules still win: `web_fetch(*.internal)` blocks a
host outright.

## Todo ledger

### Tool

`todo { items: [ { text, status } ] }` replaces the whole list; the model
sends it again to change anything (Claude Code's `TodoWrite` shape, which
models already know). `status` is `pending | active | done`
(`in_progress` and `completed` are accepted as synonyms). At most one
`active` item is expected but not enforced. An empty list clears the ledger.
The result is the ledger as text:

```
3 items · 1 done · 1 active
✓ read the parser
◉ add the notebook branch
○ run the tests
```

`Agent.todos: Arc<Mutex<TodoList>>` is shared so davinci reads it without
an event. The list is persisted as a session custom entry (`customType:
"todo"`) after every change and restored on `--resume`.

### Surfaces

- davinci: when the model keeps a ledger, the STUDIO box shows it instead
  of the synthesised per-tool steps: `✓` done, `◉` active, `○` pending, and
  the active item's target is the current tool's target (`◉ add the
  notebook branch · edit src/tools.rs`). The `1c` plan sheet (`ctrl+l`)
  shows the same list with numerals. `/todo` prints it between turns;
  `/todo clear` empties it.
- Legacy, `--print`, RPC: the tool result text is the ledger; nothing else.
- System prompt: one paragraph tells the model to keep the ledger current
  on tasks of three or more steps and to mark items done as it goes.

## Notebooks

- `read` on a `.ipynb` parses the JSON and renders cells:

  ```
  # notebook · 12 cells · python
  # [1] markdown
  # Title
  # [2] code
  import pandas as pd
  # out: <DataFrame 3×2>
  ```

  Code cells show `source`; outputs are summarised under `# out:` —
  `stream` text, `text/plain` of `execute_result` / `display_data`, error
  `ename: evalue` — at most 20 lines per output, images noted as
  `# out: image/png`. `offset`/`limit` apply to the rendered lines.
  `details.notebook = { cells, language }`. A file that is not valid
  notebook JSON reads as text.
- `edit` on a `.ipynb` matches `oldText` against each cell's joined source;
  the unique cell that contains it is rewritten (its `outputs` cleared and
  `execution_count` nulled for code cells). Zero or several matching cells
  is the usual "could not find" / "multiple matches" error. Other JSON
  fields are preserved; the file is written back with two-space indentation
  and a trailing newline.
- `notebook_edit { path, cell, mode: replace | insert | delete, source?,
  cellType? }`: `cell` is 1-based; `insert` places a new cell *after*
  `cell` (0 inserts at the top); `cellType` defaults to `code`. Result:
  `Replaced cell 3 of notebook.ipynb` and the cell's new first line.

`notebook_edit` is `ToolClass::Edit` with the path as subject.

## Collapsible tool output

`Entry::Tool` gains `output: Vec<String>` — the result's non-empty lines,
trimmed at the right, clipped to 200 rows on entry — and the model gains
`show_tool_output: bool`. Rendering:

- Collapsed (default): one line, as today. A failed call shows its first
  four output rows indented as detail (design.md §6), drawn from `output`
  rather than from separate `Detail` entries.
- Expanded (`ctrl+t` toggles; `/settings` → *Tool output*, stored as
  `showToolOutput`): the line, then up to 12 output rows indented four
  columns in muted ink, then `… 388 more lines` in border ink. A read of
  400 lines therefore costs 14 rows, never 400. Failures show the same 12.
- Δ blocks obey the same switch: 8 hunk rows collapsed, 40 expanded.

The `1b`/`1g` mockups are unchanged: their failure rows are the same four
lines drawn the same way.

## Highlighted diffs

### Diff on the tool side

`edit_diff.rs` gains `diff_lines(old, new) -> Vec<DiffOp>` (Myers, O(ND))
and `generate_diff_string(old, new, context = 4)` mirroring the TypeScript
function: `+NN text`, `-NN text`, ` NN text`, context trimmed to four lines
each side with `...` between distant changes, `firstChangedLine`. `edit`
returns `details.diff` and `details.firstChangedLine` as TypeScript does;
`write` returns `details.diff` against the previous content when the file
existed (against nothing when new). `notebook_edit` returns the diff of the
cell.

### Drawing

`views/highlight.rs`: `language_of(path | fence) -> Option<Lang>` and
`tokens(lang, line) -> Vec<(Token, &str)>` with `Token { Keyword, String,
Comment, Number, Plain }`. Languages: rust, ts/js/tsx/jsx, python, go,
c/cpp/h, java, kotlin, csharp, ruby, php, swift, sh/bash/zsh, powershell,
json, toml, yaml, sql, elixir, html/xml, css. The lexer knows line and
block comments, single/double/backtick strings with escapes, numbers, and
each language's keyword table; block comments do not carry across lines
(each hunk row is lexed alone; a wrong colour on the rare multi-line
comment is a cost taken for a lexer of one screen).

Colour comes from the theme, not the lexer: `Theme::syntax(Token) -> Color`
maps keywords to `secondary`, strings to `success`, comments to `muted`,
numbers to `warning`, plain to the caller's base. In a Δ hunk the sign and
plain text keep the line's colour (success / error); keywords, strings and
numbers take theirs; context rows stay wholly muted. Fenced code in prose
and the `/diff` sheet's hunks use the same function. Under `NO_COLOR` every
role is the same ink and the glyphs carry the meaning, as everywhere else.

davinci pushes `Entry::Gap, Entry::Delta { path, adds, dels, hunks }` right
after a successful `edit` / `write` / `notebook_edit` line, built from
`details.diff` (numbers stripped; `...` becomes a context row `…`). The
tool line's summary stays `+3 -1`.

## Naming and glyphs in davinci

| Tool | Target on the line | Glyph | Instrument | Studio verb |
|---|---|---|---|---|
| `bash` background | the command · `job 1` | ✓ | manus | testing |
| `job_output` | `job 1 output` | ↳ | manus | studying |
| `job_kill` | `kill job 1` | ✓ | manus | testing |
| `web_fetch` | `fetch <host><path>` | ⌕ | instrumenta | surveying |
| `web_search` | `search web "query"` | ⌕ | instrumenta | surveying |
| `todo` | `plan · 3 items` | ✓ | instrumenta | planning |
| `notebook_edit` | `edit <path> · cell 3` | Δ | instrumenta | constructing |

Summaries: `web_fetch` → `412 lines`; `web_search` → `8 results`;
`job_output` → `40 lines · running` / `exit 0`; `todo` → `1 of 3 done`.

## Settings and flags

```jsonc
{
  "showToolOutput": false,           // ctrl+t toggles for the session
  "webSearch": { "braveApiKey": "…" } // optional; env BRAVE_API_KEY wins
}
```

No new flags. `--tools` / `--exclude-tools` already select any built-in by
name, and `web_fetch`, `web_search`, `todo`, `job_output`, `job_kill`,
`notebook_edit` join `BUILTIN_TOOLS`.

## Diagnostics

`PI_AI_TRACE` gains lines for jobs (`job 1 start pid=…`, `job 1 exit 0`)
and web calls (`web_fetch GET … → 200 text/html 48k`). The `/jobs` listing
is the user-facing view of the same book.

## Testing

Fixture-only, as the repository requires.

- `jobs.rs`: a job that prints and exits is announced once; `job_output`
  with `wait` returns after exit; `job_kill` on a sleeping job reports
  `Killed`; the buffer cap drops the head and says so; the loop injects the
  notice before the next completion and `prompt()` prepends it.
- `web.rs`: HTML reader on saved fragments (headings, lists, links, pre,
  entities, script dropped); DuckDuckGo result parser on a saved page;
  fixtures answer `web_fetch` / `web_search`; `PI_OFFLINE` refuses; a
  `file:` URL is refused; permission class and rules.
- `todo.rs`: replace semantics, synonyms, rendered text, persistence entry
  round trip.
- Notebook: read renders cells and outputs; `edit` inside a cell clears
  outputs; `notebook_edit` insert/replace/delete; invalid JSON reads as
  text.
- `edit_diff`: `diff_lines` on insert/delete/replace/identical; the diff
  string matches the TypeScript shape on a worked example; `firstChangedLine`.
- `highlight.rs`: each language's comment and string forms, numbers,
  keyword tables, a line that is only punctuation, unknown language →
  plain.
- `pi-tui`: collapsed vs expanded rows, the 12-row cap and the `… more`
  row, failure keeps four rows, Δ hunk colours per token, fenced code
  highlighted, `NO_COLOR` still reads, tail-render equality.
- davinci: Δ block pushed after an edit; todo args become the STUDIO ledger
  and the plan sheet; a finished job becomes a row; `/todo`, `/jobs`; the
  `showToolOutput` row round-trips through settings.
- End to end (by hand, ConPTY harness): `PI_OFFLINE_TOOL_CALL` scripting a
  background `bash`, `todo`, and an `edit`; the Δ block, the ledger and the
  job row appear; `ctrl+t` expands the read.

## Out of scope

MCP (phase 4), plan mode and subagents (phase 5), hooks and `/cost` (phase
6), streaming tool output mid-call for foreground commands (the TS
`onUpdate` throttle), per-row expand, a `/jobs` sheet (needs a mockup).
