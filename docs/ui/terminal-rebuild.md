# Terminal rebuild: implementation and review guide

Branch: `J12003LPZ/terminal-ui-rebuild-20260920`.

## What changed

The default terminal presentation no longer uses the brown newspaper theme. The
shared shell, transcript, command suggestions, panels, model picker, settings,
checklists, and graph use neutral surfaces, quiet separators, readable selected
text, and separate success/warning/error colors. Existing built-in `vox`
selections use the new dark presentation; `vox-classic` preserves the former
appearance as an explicit opt-in. This changes rendering, not permission,
credential, trust, telemetry, or execution settings.

The conversation input is pinned at the bottom. Settings and other command
panels remain above it in the terminal workspace, with existing conversation
context retained when space permits. `/config` and `/settings` open the same settings panel, with Status, Config,
Usage and Stats tabs populated from actual DaVinci state, a search field, nearby
values, and optional details. The model picker uses numbered choices, independent
current/focused markers, real provider identity and effort choices. Enter saves
the default; `s` selects for the current session only; `/` enters model search.
No alternative provider/model is renamed to Claude.

The graph is optional. Starting or resuming a run does not force it open. The
conversation shows a compact background summary; `/graph-status` opens the
interactive view. Inside the graph, Tab switches between graph controls and the
conversation editor. Typed `x`, `p` or `r` in the editor are text, not graph
control actions. Enter submits to the main conversation, never implicitly to the
selected worker. Closing the view preserves the run and draft. Multiline paste,
Unicode graphemes and resizing retain the draft. Task names/statuses lead;
commands, output and actual branch/worktree data are inspected on demand.
Missing runtime branch/worktree data is not fabricated.

## Controls

| Location | Action |
| --- | --- |
| Conversation | Ctrl+U clears to line start; Ctrl+E moves to line end; Ctrl+O expands tool output. Ctrl+D does not exit with a nonempty draft. |
| Graph | Tab focuses the conversation input or returns to graph controls. Enter on a node inspects it. Escape closes the view, not the run. |
| Settings search | Type to filter; Enter/Down focuses matching settings; Up focuses tabs. Escape clears search or returns to the list. |
| Settings list | Up/Down selects; Enter/Space changes the selected setting; F1 shows its description, scope and choices. Escape closes. |
| Settings tabs | Left/Right switches tabs. Down returns to search. |
| Model picker | Up/Down selects; Left/Right changes supported effort. Enter persists the default. `s` applies only to the current session. `/` searches, including names beginning with `s`. |

Explicit user keybinding overrides remain supported. DaVinci-only voice and
harness shortcuts are not a claim that every Claude Code shortcut is implemented.
All displayed permission choices still go through the existing authorization
and expiry checks. The current execution engine, including its concurrency and
worktree policies, is retained; this change does not introduce a new independent
multi-conversation scheduler.

## Actual rendered gallery

Generate dark and light review pages from the real Ratatui buffers:

```sh
cargo run --locked -p davinci-tui --example editorial_preview -- dark > terminal-dark.html
cargo run --locked -p davinci-tui --example editorial_preview -- light > terminal-light.html
```

The filename is kept for compatibility with the existing preview command. The
40 gallery cases cover the 28 screens, five overlays, graph variants, narrow
windows and monochrome rendering. They contain explicitly labeled offline
fixture data. They are not recordings of live agents and are not proof of exact
reference parity. No fonts or compiled outputs are committed.

## Reference provenance and remaining acceptance work

The reference is the actual Claude Code **2.1.278** Linux executable. The GitHub
reference job verifies its release digest before launching it in disposable
profiles, with a loopback fixture endpoint and no personal account or paid model
inference. Dark/light 120x40 PTY captures contain cell styles, text, cursor and
input traces. They are not screenshots synthesized from DaVinci.

Validated panel capture run:
https://github.com/J12003LPZ/davinci/actions/runs/35562721751

Artifact: `claude-reference-d6cbc385e8c9b005555a530bc0c48dce46474f4e`.
Archive SHA-256:
`51c0d4e09dd5589164d9123b002828fcd8b001fd6b577c280b64dfc34d601a2e`.

Earlier capture attempts left nested configuration focus open. Their subsequent
panel labels are **not** model/reference evidence. The validated run dismisses
nested focus before invoking each command and records that trace.

The whole-terminal 1:1 requirement remains the acceptance target, **not a claim
that this branch has already met it**. In particular, a complete independent
cell/style comparison, animation comparison, native Windows Terminal/ConPTY
visual and IME review, and an independent usability/code review are outstanding.
Some DaVinci-specific permission, history, and harness controls retain their
existing operations within the new shared panel rather than reproducing
unsupported Claude-only account controls. The reference version's removed agent
wizard is not treated as an equivalent of DaVinci's interactive graph.

## Verification

The branch adds deterministic regression tests for source-safe model/settings
search, session-only model choice, geometry, every screen/overlay's bounds,
reference editor keys, graph focus/submission/paste, and draft preservation.
Existing terminal and host suites remain enabled; two pre-existing ignored TUI
tests are not newly skipped by this work. Reference-matching tests do not replace
authorization, secret masking, event handling or persistence tests.

The branch workflow runs formatting, TUI tests/lint, host integration and rendered
previews. Read the exact job logs and commit IDs before claiming a result. The
host library's CLI integration tests require building the debug `davinci` binary
first; running only `cargo test --lib` in a fresh target directory is insufficient.

```sh
cargo fmt --all -- --check
cargo test --locked -p davinci-tui
cargo clippy --locked -p davinci-tui --all-targets -- -D warnings
cargo build --locked -p davinci-coding-agent --bin davinci
cargo test --locked -p davinci-coding-agent --lib
cargo test --locked -p davinci-coding-agent --bin davinci
cargo check --locked --workspace --all-targets
```

## Trying this branch without disturbing an active checkout

From an existing clone, create a dedicated worktree only after confirming that
the branch is not already checked out by another session:

```sh
git fetch origin
git worktree list
git worktree add --detach ../davinci-ui-review origin/J12003LPZ/terminal-ui-rebuild-20260920
cd ../davinci-ui-review
cargo run --locked -p davinci-coding-agent --bin davinci
```

This builds/runs the review binary in its own worktree. It does not install over
your existing executable or stop running agents. Actual launch still uses the
user's existing provider/configuration unless explicitly given a separate test
profile. Do not grant extra trust or permissions for visual review. Close the
review process normally and retain the previous installation for rollback.
