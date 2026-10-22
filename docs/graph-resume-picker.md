# Resume a graph without its run ID

From an interactive DaVinci session in the graph's original workspace:

- `/graph-resume` opens the **Graph Resume** picker. It shows saved unfinished,
  blocked, paused, and cancelled runs, newest first.
- `/graph runs` opens the same picker with completed runs included.
- **↑/↓** select a saved run; **Enter** shows/hides its metadata without
  executing anything; **r** explicitly resumes the selected run.
- **/** starts a case-insensitive search over goals and run IDs. While searching,
  type text or Backspace; **Enter** leaves search mode, and **Esc** first leaves
  search mode, then closes the picker. **a** toggles showing completed history.
- `/graph-resume --latest` starts the newest non-completed graph directly
  (useful for scripts that intentionally skip the picker).
- `/graph-resume <run-id>` retains the exact-ID path for automation and older
  clients.

The picker **does not** create a new graph, spawn workers, or alter a checkpoint
while navigating. Selecting **r** calls the existing graph resume controller:
the workspace lease, checkpoint version/identity, existing worker-attempt
reconciliation, safety/replay checks, and cumulative budgets continue to apply.
A completed run cannot be resumed: start a new `/graph <goal>`.

## Persistence and performance

Run summaries are stored as small optional `summary.json` files beside each
`state.json` in `.davinci/graph/runs/<run-id>/` (legacy `.pi` roots are
still recognized). The summary includes only the goal, phase, ID, modified time,
task counts, spend, and worker count, never the execution continuation or source
baseline. Writes happen **after** the canonical checkpoint was committed and
cannot cause a successful checkpoint write to report failure.

Discovery streams the canonical JSON and verifies its SHA-256 digest and identity
against the summary's generation stamp. Checkpoint sequences are compared when
present; older checkpoints use the digest alone. Equal timestamps cannot validate
a stale summary. Missing, stale, oversized, malformed, or legacy unstamped summaries
fall back to the full validated run state. Conflicting modern and legacy roots
also take the validated fallback. This avoids constructing the complete execution
state on the indexed path, but still reads and parses the canonical JSON; no
constant-time discovery or measured latency improvement is claimed. The canonical state is always loaded
and revalidated before any real continuation. An index is a performance hint,
not authorization, checkpoint identity, or a substitute for recovery evidence.

The existing run-retention policy remains unchanged. The picker can only display
runs still present on disk. Each workspace has its own graph history.

## PR review validation

A regression simulates failed summary publication followed by equal coarse
modification timestamps on both files. It failed with the timestamp-only check
and now rejects the stale index while listing the canonical completed phase.
All 27 focused persistence tests passed, including healthy-index and corrupt-index
fallback coverage. Locked native compilation, formatting, and diff checks passed.
