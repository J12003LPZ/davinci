# P6: Git Intelligence

Status: design approved by the user on 2026-09-17; implementation pending.
Execution sequence: 7 of 12.
Dependencies: Existing Git inspection, Repo AST and CacheRuntime; P7 green before this sequential milestone.
Requirements authority: project section 11 and cross-cutting sections 18-40;
project numbers are kept stable while execution order follows the program design.

## Outcome

Return bounded factual history, blame/diff/conflict context tied to Git objects and source symbols, keeping interpretations separate.

## Scope and ownership

Proposed files/modules (not claims of completed edits):

- crates/davinci-coding-agent/src/native_extensions/git_intelligence/ (new)
- native_extensions/security_scan/git.rs only for narrow reusable safe Git runner extraction if justified
- native_extensions/repo_intelligence typed symbol/range API
- Native registration/settings/permissions/Graph roles and new git_intelligence fixtures/tests/evals

The main session is the only writer. No subagents, parallel writers or unrelated cleanup.
Reinspect these paths and callers at implementation time because main may advance.

## Baseline before implementation

Use direct bounded Git log/blame/diff on a local temporary history with rename, regression, merge conflict and symbol move. Record commands/calls, bytes, latency and exact expected SHAs/lines.

## Ordered implementation and RED/GREEN work

1. RED: symbol history/introduction/modification, renamed paths, deleted symbols, detached HEAD, shallow history, no Git and conflicting stages.

2. Reuse bounded interruptible argv Git execution patterns. Disable pager, external diff/textconv and unsafe config effects; validate revisions by resolving exact objects, separate revisions/options/paths, and never interpolate shell text.

3. Implement git_symbol_history, git_related_commits, git_changed_symbols, git_branch_diff, git_blame_symbol, git_commit_context and git_conflict_explain as typed requests. Reuse AST parser on selected historical blobs to map symbols/ranges.

4. Return exact commit IDs, line changes, authors/messages where useful and movement only when detectable. Introduction may be unknown in shallow/limited history. Locally available PR metadata is optional evidence; never fabricate remote context.

5. Represent observed facts separately from commit-message suggestions. Conflicts expose stages/base/ours/theirs and bounded contextual overlap; the tool does not resolve or mutate them.

6. Cache immutable Git blobs/trees/commit results in CacheRuntime Git with object ID and algorithm version; mutable branch/worktree results include current identities. Recheck path/permission before cached delivery.

7. Register Read tools and history/regression-specific deferred discovery in normal mode, then allowed Graph research/review roles. Optional Git failure does not disable AST/LSP.

8. GREEN: malformed refs/path traversal/config injection, cancellation/output bounds, concurrency, normal dispatch and history eval; affected gates/CI/docs.

## Required targeted validation

- Revision strings beginning with options, wildcard/pathspec injection, newline filenames, file movement, binary blobs, shallow/unborn repositories.
- History fact checks against constructed commits, no claims of causal intent from message text alone.
- Git inspection leaves index/worktree/refs/config unchanged; denied cached blobs remain denied.
- Warm immutable-object hits and normal-agent regression investigation with actual targeted test evidence.

## Settings and fallback

gitIntelligence.enabled defaults true and lazy; no network fetch and no write Git command in this subsystem.

## Specific acceptance guard

An incomplete history must report unknown/partial introduction and coverage rather than claim the first returned commit introduced the symbol.

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
