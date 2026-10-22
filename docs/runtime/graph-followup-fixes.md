# Graph follow-up integrity fixes

Branch `fix/graph-fork-history-budgets` builds on `fix/graph-recovery-integrity`
(commit `85a78738`), preserving the previous six fixes. Findings 7–13 were supplied
in the second audit. Finding 8 (untracked-directory replay hashing) is already
fixed and covered by the base branch's nested-untracked-file regression.

## Implementation plan

1. Reject overflowing 32-bit budget fields; require a stopped controller for
   fork, rewind and budget edits, with guards before lease acquisition.
2. Use dependency descendants for fork/rewind invalidation; clear stale receipts
   and restart cursors without discarding independent completed work.
3. Add a checkpoint sequence independent of control revision, archive every
   successful checkpoint, and use sequence history for default graph diff.
4. Materialize forks in isolated workspaces from exact original baselines;
   preserve completed artifacts as inherited evidence and retain parent attempt
   provenance without reusing parent worker sessions. Validate before publishing.
   Reject checkpoints whose bytes/cursors cannot be reconstructed safely.
5. Add focused regressions, run affected Graph tests and compile/format checks,
   review, push the branch and attempt the separate PR.

## Verified mechanisms

- Budget parsing checks the five `u32` ceilings at validation and conversion;
  values above `u32::MAX` cannot wrap into the unlimited sentinel.
- Fork/rewind/budget mutation guards report that a controller must stop before
  attempting its workspace lease. Pausing continues to retain ownership.
- Dependency invalidation uses topology/task edges and explicit historical
  dynamic delivery relationships. New dynamic workers persist those dependencies.
- `checkpointSequence` defaults to zero when reading old checkpoints and advances
  on successful saves independently of the control revision. Each prior snapshot
  is archived as `checkpoint-N.state.json`; existing control revision archives
  remain available. Default `/graph diff` uses the preceding checkpoint and reports
  both identities. Blob collection includes the new snapshot names.
- A fork materializes recorded file bytes into
  `.davinci/graph/forks/<child>/workspace`. Git repositories use a detached
  worktree without a checkout, populated from verified checkpoint bytes. Other
  workspaces use a separate directory. Explicit parent-side resume routes to
  that child while retaining active-run ownership for status/stop/shutdown.
- Preserved artifacts are copied and validated. Child attempts start with fresh
  identities; parent sessions are never rebound. The pinned parent retains the
  original attempt evidence. Child state publishes only after resume validation.
- Historical cursors restore earlier writer boundaries and retain preceding
  milestone completion receipts. Rewind preflights the cursor before code effects.

## Supported boundaries and limits

Missing/corrupt checkpoint bytes, unavailable historical cursors, expired parent
deadlines, symlinks/submodules without historical identity, or independently
preserved writer effects whose placement cannot be proved are errors, not
successful forks. Legacy incomplete runs are preserved and rejected. Earlier
multi-writer code rewind requires its exact execution boundary and is rejected
when it cannot be represented safely; no filesystem effects start in that case.

Fork snapshots cover regular files captured by the baseline. Ignored/generated
runtime dependencies and an entire historical Git index are not checkpointed.
Git bookkeeping starts from current HEAD while recorded file contents are restored.
Full checkpoint history increases retained disk state; this change does not prune
required transition evidence or claim measured performance gains.

## Validation

Four initial regressions failed against the base branch, then passed after the
initial fixes. Expanded coverage includes Git/non-Git materialization, two
milestones, earlier-writer forks, full native dry-run resume, missing preimages,
unsupported symlinks, previous milestone receipts and checkpoint-only blob refs.
Final checks passed:

- `cargo test -p davinci-coding-agent --lib native_extensions::graph --locked -j 2`:
  466 passed, 5 existing ignored, 0 failed.
- `cargo check -p davinci-coding-agent --locked -j 2`: passed.
- `cargo fmt --all -- --check` and `git diff --check`: passed.

Tests used Rust 1.88.0, disabled debug symbols/incremental artifacts, and two
build jobs. Independent review found the multi-milestone receipt and missing-link
issues; both are addressed and covered by the expanded regression. No live provider
calls, paid services, benchmarks or installed-binary replacement.

## PR review follow-up

Checkpoint publication now compares the caller's expected checkpoint sequence
under the state-file lock before changing companions. Reload canonical state
on conflict; a shared control revision does not authorize an older snapshot.

Recovery changes retain immutable `checkpoint-N` snapshots. Diagnostic-only
updates retain at most 16 recent `progress-N` snapshots (16 MiB soft byte budget).
A single latest diagnostic snapshot larger than 16 MiB may exceed the soft
budget; older diagnostics are evicted, and the hard total below still applies.
This preserves the preceding diff snapshot for large runs. Current canonical
state still includes all diagnostics and spending. Diff can
read recent progress snapshots; forks consume recovery boundaries. Snapshot
history has a 512 MiB / 4096-file hard capacity. When essential snapshots alone
exhaust capacity, publication fails before overwriting canonical state or
evicting recovery evidence; the controller follows its existing persistence
failure stop path. Archive the completed run before continuing.

Validation: stale same-revision snapshots conflict; 2000 progress checkpoints
retain a bounded file window; essential-capacity exhaustion preserves old bytes.
Graph suite: 469 passed, 5 existing ignored. Compile and formatting checks passed.
