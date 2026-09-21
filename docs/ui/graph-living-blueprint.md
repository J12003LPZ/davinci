# Graph Run: Living Blueprint

Interactive `/graph` uses a terminal-cell workflow canvas. `Grafo` remains the
separate code/symbol dependency study.

## Choose models before starting

Submitting a new interactive `/graph` goal or saved run opens Graph Setup before
workers start. Select a role, type or paste a model name to filter the available
catalog, and press Enter to choose it. Each of classifier, researcher,
test-analyzer, historian, planner, writer and reviewer can use a different model.
The picker uses exact provider/model IDs from the configured catalog.

Select **Start graph** to apply the choices and launch. Choices remain available
for later graphs in this session; they do not change the main chat model or write
project configuration. **Use session model for all roles** clears role overrides.
Escape returns from a model picker to the roles; Escape at the role list cancels
without launching workers or saving the draft choices. Unavailable configured
models must be replaced before launch. Non-interactive graph commands retain
their existing configuration behavior.

## Reading and controlling the run

Terminal runs show **COMPLETED**, **BLOCKED - goal not completed**, or
**CANCELLED** and add one final result to the conversation, with the reason,
verification details, elapsed time, cost and artifact directory. Finishing all
workers does not imply that verification passed. Results are also reported when
the graph sheet is closed. Older blocked runs with a stale `running` lifecycle
are displayed as stopped.

Press **s** on a blocked or cancelled graph to resume it, or use
`/graph-resume <run-id>`. The controller preserves the goal, run identity and
accumulated spend; superseded implementation artifacts are not replayed as new
work. A completed graph requires a new goal. Resume does not fix a missing goal
specification: check the original prompt with **g** before continuing.

Generated graphs bind revision attempts to the actual preceding writer, plan and
review, so a retry cannot accidentally wait for the next milestone's review.
Saved graph definitions retain their explicit dependencies. The revision display
reports a lifetime total and a per-milestone limit separately; an explicit resume
starts a new delivery attempt without erasing prior revision or cost totals.

Failed verification excerpts preserve panic locations, assertion details and
compiler errors alongside the final output, within the existing 4,000-character
budget. Revision workers receive this excerpt even when a test prints a large
event dump. A failed verification command still blocks completion.

Graph workers inherit the active agent directory's saved compaction settings.
An explicit token or percentage threshold also controls Context VM window-pressure
folds, using each worker model's context window. Structural delta folds can still
happen earlier to bound the event log. Disabling automatic compaction disables
these automatic folds; explicit manual folding remains separate.

Cards follow actual dependencies from left to right. `◉` and heavy borders mark
active workers; `×` marks failure; `!` and a reason mark blocked work. Completed
cards retain `✓`, queued cards retain `○`, and selection says `SELECTED` even
without color. Known phases label lanes; unknown phases use numbered stages.
Pending/ready descendants of failed or cancelled workers receive a blocked
presentation without changing their persisted status.

| Input | Behavior |
|---|---|
| Arrows | Select real dependency neighbors; up/down traverses same-lane peers. In the narrow ledger, move through workers. |
| Enter | Expand a completed summary, or toggle the selected worker's public details. |
| Esc | Close inspection/diff context, then expanded groups, then the sheet. A selected worker remains individually visible. |
| `f` | Explicitly restore follow and recenter on active work. |
| `v` | Toggle Overview/Focus; focus emphasizes selected/active ancestry and descendants, without changing execution. |
| `g` | Toggle the original prompt and worker progress in the inspector. PgUp/PgDn scroll it; Esc returns to worker activity. |
| `p` / `x` / `r` / `d` | Preserve the existing pause/resume, stop, retry and diff action bridge. Synthetic summaries cannot become worker control targets. |
| PgUp / PgDn | Page open details; otherwise pan vertically or page the narrow ledger. |
| Left click | Select a visible card using its rendered geometry. |
| Wheel / Shift-wheel | Pan vertically/horizontally where the terminal supports it. |

Follow starts enabled. Manual selection, navigation and panning disable it;
snapshot refresh does not turn it back on. When parallel active workers fit,
follow includes them together; otherwise it anchors deterministically on the
first active card in spatial order. All operations remain keyboard-accessible.

At least three quiet completed workers can fold only when phase, role, depth,
incoming dependencies and outgoing dependents match. Selected, active, failed,
blocked, cancelled, verification/review-relevant and public-contract-bearing
workers do not auto-fold. Expansion survives ordinary refresh. New workers do
not reorder first-seen peers; unchanged topology retains its geometry.

## Responsive presentation

| Available size | Presentation |
|---|---|
| Width ≥100 and usable graph body ≥16 rows | Full cards and a right inspector using one third of the width (36–72 columns) |
| Width 72–99 (or a shorter wide body) | Canvas and bottom inspector |
| Width 50–71 | Compact cards and bottom inspector |
| Width <50 or graph body <12 rows | Bounded worker ledger with inspection and controls |

The body measurement excludes sheet chrome and graph header/footer. Corrupt
cycles or duplicate IDs fail closed to a ledger with a layout warning. Missing
dependencies are reported without synthesizing nodes. Long text wraps or clips
inside its own region. Motion uses the existing refresh tick only and stops
under `--no-animation`; state glyphs and `NO_COLOR` remain usable without it.

## Data and privacy boundary

`davinci_interactive::graph_sheet` is the actual runtime-to-TUI producer (the
initial plan named `davinci_surfaces.rs`). It supplies raw status, known role
phase, dependencies, attempts, usage, activity, artifact path, lifecycle, run
phase, blocked reason and verification outcomes from the graph snapshot.
Artifact directories use the existing graph-store path helper. Owner, recent
tools and public contract are displayed only when supplied; absent data is
omitted. The TUI never opens graph persistence files.

Selecting a worker shows its current work and recent public transcript without
requiring Enter. The host refreshes the selected worker's activity once per
second using `graph-view`, and checks both run and worker IDs before displaying
it. The log read is limited to the last 64 KiB; the panel retains up to 40 recent
entries, newest first. Enter exposes additional metadata and enables paging.
The original goal remains available with `g` even when a control notice occupies
the header. Common credential formats are redacted from goals and activity
before they reach the display.

Long clipboard pastes remain drafts, shown as `[paste #1 1234 chars]` when over
1,000 characters or 10 lines. Windows hosts that strip paste markers use input
burst detection with a 500ms quiet window; wait for the marker before pressing
Enter. Unmarked chunks separated by longer pauses cannot be identified as one
paste. Explicit bracketed pastes do not have that timing limitation.
Enter expands the marker on submission. Pauses
between bracketed-paste chunks never turn pasted newlines into submit keys.
If a terminal loses the closing paste marker, Ctrl+C releases the buffered text
and performs the usual cancel action.

Verification shows command/name, outcome, exit code, skip status and elapsed
time; raw output tails and private context fingerprints are not passed through.
Public text containing private-reasoning markers is withheld before wrapping or
clipping. This is defense in depth, not permission to put private reasoning in
public display fields. The inspector exposes execution facts, not model thoughts.

## Offline preview

Run the built CLI with `--davinci --screen blueprint --no-animation --offline`.
This uses the existing native fixture-screen loop, not a second UI runtime.
The explicitly illustrative snapshot contains parallel active workers, failure,
blocked work and three foldable researchers. Try 40, 80 and 120 columns at
40 rows; also set `NO_COLOR=1`. The fixture does not execute worker/controller
actions or call a provider. Exit its sheet with Esc, then quit with Ctrl+D.

## Acceptance record — 2026-09-16

Implemented solo against base `a50fdc6dfb8fa6aad8829299e8aaac258bf15477` in
eight plan tasks. RTK wrapped command execution; Headroom MCP compressed large
evidence. No subagents, renderer dependencies, browser surface or event loop
were added. The original shared checkout and its unrelated changes were left
untouched.

| Requirement | Evidence |
|---|---|
| LB-1, LB-2 | Default GraphRun dispatch, shared deterministic layout, dependency ordering, branch alignment and refresh-position regressions |
| LB-3, LB-11 | Static monochrome glyph/selection tests, blocked/cancelled labels and disabled-animation tick regression |
| LB-4 | Manual movement disables follow; explicit `f`, stable active-frontier viewport and snapshot-refresh tests |
| LB-5 | Equal-frontier folding, attention/selection exclusions, reversible expansion and synthetic-target guard tests |
| LB-6 | Rail/drawer geometry, bounded public inspector and long-detail paging tests |
| LB-7, LB-8 | Scoped key routing, unchanged typed action mapping, real composed-frame mouse tests and microphone-priority regression |
| LB-9 | Widths 0/1/20/32/40/49/50/71/72/80/119/120 across tiny/normal heights, corrupt-topology fallback and ledger-control tests |
| LB-10 | Typed real-GraphRun producer tests for added fields, transitive blocked presentation, unknown-field omission and private-content sentinels |
| LB-12 | Coding-agent graph regression suite, full TUI suite, complete diff review; no controller/persistence/Grafo source changes |

Validation used pinned Rust 1.83.0, offline locked dependencies,
`PI_OFFLINE=1`, and `CARGO_INCREMENTAL=0` for the CI-equivalent run:

- Formatting and workspace/all-target Clippy with warnings denied: passed.
- All 14 package shards listed in the current CI workflow: **4,140 passed,
  7 existing ignored tests**. TUI: 647 passed, 1 ignored; coding-agent: 2,142
  passed, 6 ignored.
- Focused graph checks: TUI 36 passed; coding-agent 714 passed, 1 ignored.
- Prompt contracts: 88 passed; behavioral contracts: 65 passed; ecosystem loop:
  12 passed; invariants: 2 passed; migration baseline: 2 passed.
- CLI build: passed. Native voice: 14 passed, 1 existing ignored; native helper
  build: passed. Missing CMake was supplied as a checksum-verified, task-local
  portable tool, without changing global configuration or repository dependencies.
- All 24 local CI-equivalent command groups exited successfully. Package counts
  above do not double-count focused/contract reruns or the native feature suite.

Manual acceptance used the freshly built executable in real Windows ConPTY at
40×40 (monochrome), 80×40 and 120×40 with animation disabled. The executable copy
matched the build's SHA-256. The narrow ledger, simultaneous active workers,
failure/blocked distinction, selection, safe expansion, adaptive inspection,
follow/focus transitions, mouse selection and wheel routing were exercised.
After closing Graph Run, Ctrl+G still opened the separate Code Graph fixture.
Three preview passes used the same CLI arguments at the three sizes. Automated
refresh simulations cover live state changes; the manual preview was an offline
fixture, not a paid/live-provider run.

Local validation artifacts are under the task's temporary
`graph-blueprint-01a0ac12` directory: `validation-results.json`, per-command logs,
and `manual-{40,80,120}.json` terminal transcripts. They are not repository
assets. Linux/macOS jobs and their native helper builds were not run on this
Windows machine; no cross-platform or real microphone/model acceptance is
claimed. Existing ignored tests remain ignored.
