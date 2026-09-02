# HANDOFF — competitive harness, phases 1–3 landed

## Goal

Turn the Rust `pi` rewrite into a harness that competes with Claude Code and
Codex CLI on engineering and interface. The roadmap is
`docs/superpowers/specs/2026-09-01-competitive-harness-roadmap.md`. Phases 1
("turns that are real", `2d7162e`), 2 ("trust and control", `8442da4`) and 3
("tools that compete", spec
`docs/superpowers/specs/2026-09-01-tools-that-compete-design.md`, plan
`docs/superpowers/plans/2026-09-01-tools-that-compete.md`) are done; phases 4–6
are listed in the roadmap and not started.

## State (2026-09-02)

- Branch `rust-rewrite`. `cargo fmt`, `cargo clippy --workspace --all-targets
  -- -D warnings` and `cargo test --workspace` are green. One Node test,
  `js_host::tests::live_autocomplete_queries_get_suggestions_with_prefix`, is
  flaky under a full parallel run and passes alone — it shares the persistent
  Node host with its neighbours.
- Release binary copied to `~/.cargo/bin/pi.exe` and `~/.cargo/bin/davinci.exe`
  (memory note: `cargo install` does not work in this repo).

## What phase 3 changed

Documented divergence from TypeScript `pi`. No TS counterparts for these tools.

- **Built-in tools** now include `web_fetch`, `web_search`, `todo`,
  `job_output`, `job_kill`, `notebook_edit`. `Agent.tool_context` holds
  `JobBook` and `TodoList` behind `Arc<Mutex<_>>`.
- **Background jobs** (`pi-agent/src/jobs.rs`): `bash`/`powershell`
  `background: true`; notices as `role: user` `extra.customType:
  "backgroundJob"` (`JOB_NOTICE_TYPE`). User sees them via `take_unseen` on
  the 250ms tick; the model via `take_unannounced` at loop-top / `prompt()`.
  Output cap 4MB keep 3MB. Windows kill is `taskkill /T /F`. Drop kills all.
- **Web** (`web.rs`): HTML → text, DuckDuckGo search, Brave if `BRAVE_API_KEY`
  / `webSearch.braveApiKey`. Fixtures `PI_WEB_FETCH_FIXTURE` /
  `PI_WEB_SEARCH_FIXTURE`. `ToolClass::Network`: allowed in `read-only`, asked
  in `ask`/`edits`.
- **Todo** (`todo.rs`): replace-whole list; synonyms `in_progress`/`completed`.
  davinci: the `todo` call becomes the STUDIO ledger and the `1c` plan sheet.
  `/todo`, `/todo clear`.
- **Notebooks** (`notebook.rs`): cell parse/render, in-cell edit, structural
  replace/insert/delete. `read`/`edit` branch on `.ipynb`.
- **Diffs**: Myers line diff in-tree; `edit`/`write` return `details.diff`.
  `AgentEvent::ToolExecutionEnd` carries `details`. davinci draws a Δ block
  from that without re-reading files.
- **Collapsible output**: `Entry::Tool.output` (capped `TOOL_OUTPUT_KEPT=200`).
  Default collapsed; failures show 4 rows; `ctrl+t` / `showToolOutput` shows
  12 + `… N more`. Δ: 8 collapsed / 40 expanded. Syntax via
  `views/highlight.rs` and `Theme::syntax` only.
- **Replay**: a persisted `backgroundJob` user message becomes a manus tool
  row, not a `>` echo.
- **Trust homedir**: `has_trust_requiring_project_resources` now uses
  `pi_session::home_dir()` (USERPROFILE first on Windows) so the user's
  `~/.agents/skills` is not treated as a project resource.

## Verified

- Unit tests across `pi-agent` (jobs, web fixtures, todo, notebook, Myers
  diff, network permission class), `pi-tui` (collapsed/expanded output, Δ
  caps, keyword colour, `NO_COLOR` glyphs, settings row) and
  `pi-coding-agent` (Δ from `details`, todo STUDIO, job row, backgroundJob
  replay, `/todo` `/jobs` in the corpus, `showToolOutput` round-trip).
- `make fmt` / `cargo clippy --workspace --all-targets -- -D warnings` /
  `cargo test --workspace` green.

## Next

Phase 4 in the roadmap ("native MCP client": stdio + streamable-HTTP from
`~/.pi/agent/mcp.json`, tools and resources native, `/mcp` sheet). Phase-2
leftovers still open unless absorbed: `/permissions` sheet needs a mockup;
rule editing from the panel; hooks answering permission questions (phase 6);
ledger `✓` on denied steps (cosmetic); live Codex check blocked by usage
limit.
