# P11: Workspace Snapshot / Safe Sandbox Layer

Status: design approved by the user on 2026-09-17; implementation pending.
Execution sequence: 11 of 12.
Dependencies: P4 transactions, existing worktree/checkpoint/rewind/source-manifest contracts; P10 green.
Requirements authority: project section 16 and cross-cutting sections 18-40;
project numbers are kept stable while execution order follows the program design.

## Outcome

Create cheap scoped checkpoints and explain or safely restore owned changes without copying huge repositories or overwriting newer edits.

## Scope and ownership

Proposed files/modules (not claims of completed edits):

- crates/davinci-agent/src/runtime/{checkpoints,rewind,worktree,effects,source_manifest}.rs
- New thin workspace checkpoint native adapter in coding-agent
- Native/settings/permissions/normal/Graph adapters
- New checkpoint diff/restore security, worktree and crash fixtures/tests/evals

The main session is the only writer. No subagents, parallel writers or unrelated cleanup.
Reinspect these paths and callers at implementation time because main may advance.

## Baseline before implementation

Current checkpoint BlobStore and rewind/worktree behavior on a fixture with tracked/untracked/ignored files and transaction effects. Measure checkpoint bytes versus full checkout.

## Ordered implementation and RED/GREEN work

1. RED: checkpoint -> owned edit -> diff/restore; checkpoint -> user edit -> restore conflict; independent linked worktrees; ignored secrets not captured.

2. Define checkpoint identity using canonical worktree/Git common dir/index/HEAD plus bounded scoped file/effect manifest. A snapshot is not a Git commit and does not stage/commit user files.

3. Reuse existing checkpoint blobs/effects for changed content and Git object identities for immutable baseline data. Do not duplicate full repositories or introduce a new database.

4. Implement workspace_checkpoint, workspace_diff and workspace_restore using existing permission checks and rewind preview/apply. Resolve owner/transaction from host, reject outside/sensitive/linked paths.

5. Before restore, validate current postimages/ownership and mutation lane; leave unrelated/newer edits intact and return conflict when safe restoration is not provable. Preview lists exact target changes.

6. Keep recovery preimages durably available until lifecycle retention permits removal; quota failure blocks unsafe mutation instead of silently evicting necessary recovery. Cleanup is scoped and never a broad worktree sweep.

7. Normal/Graph adapters share checkpoint implementation and distinguish workspace identity; cancellation/crash keeps recoverable records.

8. GREEN: real files/Git fixtures, conflict and storage-failure tests, normal workflow and memory/bytes eval, affected crate/fmt/lint/platform CI/docs.

## Required targeted validation

- Create/delete/rename, untracked scoped files, root change, alternate worktree same HEAD, symlink/reparse target swap and stale postimage.
- Checkpoint restore cannot reset index/refs/user unrelated files or import secrets outside normal scope.
- No full repository duplicate on checkpoint; immutable Git content reused and bounded changed blobs retained.
- Normal risky edit -> checkpoint -> verification failure -> safe owned rollback; Graph worker identity cannot restore another workspace.

## Settings and fallback

workspaceSnapshots.enabled controls user-facing checkpoints; transaction recovery invariants remain mandatory. Explicit scope and bounded retention follow host settings.

## Specific acceptance guard

Git commit/tree identity alone does not capture dirty/untracked state. Safe restore needs recorded scoped hashes and transaction ownership.

## Completion gate and handoff

The approved [program design](../../specs/2026-09-17-engineering-program.md)
and original [requirements](../../specs/2026-09-17-engineering-program-requirements.md)
are the reference. Shared authorization, bounded-output, cache, lifecycle, telemetry,
settings and security invariants apply in addition to the project-specific steps.

Record all fourteen section-34 gates in this project's ledger row: design approval,
plan, observed RED/GREEN, affected package tests, fmt, Clippy, integration, security,
normal path, applicable Graph path, benchmark/eval, docs, diff review, and exact-head
CI. Planned fixtures/test names above are not claims of existing or executed tests.

Validation sequence: focused failing acceptance test; focused passing checks after
implementation; affected crate tests with `--offline --locked`; `cargo fmt --check`;
affected Clippy during development and required workspace Clippy at the milestone;
matching integration/security/eval cases; feature-branch push and actual CI monitoring.
Run broader suites only for affected shared contracts or final acceptance. Record
exact executed selectors, exits and test counts; zero-case runs do not count.

Use a separate coherent commit/PR for the subsystem. With earlier PRs unmerged,
stack on the preceding verified branch and target that branch for review. Preserve
dependency commits without force pushes. Do not begin the next subsystem with
known failures or incomplete required gates. Approval to implement this program
does not authorize merging new PRs into main.

The handoff in README records files/APIs changed, validated commands and results,
metric/artifact paths, head SHA/CI URLs, limitations, and the next dependency input.
No production capability is called done from this plan alone.
