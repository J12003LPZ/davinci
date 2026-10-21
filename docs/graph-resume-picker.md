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

Discovery uses the summary only if its modification time is at least as recent
as the actual state file; missing, stale, oversized, or malformed summaries
fall back to the full validated run state. The canonical state is always loaded
and revalidated before any real continuation. An index is a performance hint,
not authorization, checkpoint identity, or a substitute for recovery evidence.

The existing run-retention policy remains unchanged. The picker can only display
runs still present on disk. Each workspace has its own graph history.
