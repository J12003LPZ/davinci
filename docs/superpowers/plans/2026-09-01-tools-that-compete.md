# Tools that compete — implementation plan

Spec: `docs/superpowers/specs/2026-09-01-tools-that-compete-design.md`.
Every task ends with `cargo test -p <crate>` green; the last with `make fmt`,
`make clippy`, `make test`.

## 1. Diff and notebook plumbing (`pi-agent`)

- `edit_diff.rs`: `diff_lines` (Myers), `generate_diff_string` (TS shape),
  `first_changed_line`. `edit` and `write` return `details.diff`.
- New `notebook.rs`: parse/render cells, edit-in-cell, `notebook_edit`
  (replace/insert/delete). `read` and `edit` branch on `.ipynb`.
- Tests: diff ops, diff string worked example, notebook read/edit.

## 2. Todo, jobs, web (`pi-agent`)

- New `todo.rs`: `TodoList`, `todo` tool, rendered text, session entry.
- New `jobs.rs`: `JobBook`, spawn/read/kill, notices; `bash`/`powershell`
  `background`, `job_output`, `job_kill`; `Agent.jobs`;
  `inject_job_notices` in the loop and in `prompt()`.
- New `web.rs`: fetch (HTML → text), search (Brave / DuckDuckGo), fixtures,
  offline refusal.
- `tools.rs`: `ToolContext { jobs, todos }`, `execute_tool_with`, specs,
  `BUILTIN_TOOLS`; `permission.rs`: `ToolClass::Network`, subjects, rules.
- `default_system_prompt`: ledger, jobs and web guidance.
- Tests as listed in the spec.

## 3. Drawing (`pi-tui`)

- New `views/highlight.rs`; `Theme::syntax`.
- `Entry::Tool.output`, `Model.show_tool_output`; transcript rows for
  collapsed/expanded/failed; Δ hunk highlighting and row caps; markdown
  fences; `/diff` sheet hunks.
- Keybinding `davinci.tools.expand` (`ctrl+t`); `app.rs` toggle.
- Fixtures unchanged in output; tests for rows, caps, colours, `NO_COLOR`.

## 4. davinci wiring (`pi-coding-agent`)

- `davinci_session.rs`: `instrument_of` / `state_of` / `target_of` /
  `verb_of` / `summary_of` for the new tools; output kept on the tool
  entry; Δ block after edit/write/notebook_edit; todo → STUDIO + plan
  sheet; job rows from the tick poll; `/todo`, `/jobs`; settings row
  *Tool output*; corpus rows; todo restore on resume.
- `settings.rs`: `showToolOutput`, `webSearch.braveApiKey`.
- Tests: Δ block, ledger, job row, commands, settings row.

## 5. Finish

- fmt, clippy, tests; release build copied to `~/.cargo/bin/{pi,davinci}.exe`.
- Drive a scripted background job, todo and edit through the ConPTY harness.
- Roadmap row 3 cites the spec; CLAUDE.md tool list; HANDOFF.md; memory
  note. Commit.
