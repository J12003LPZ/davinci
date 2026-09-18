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

## P6 implementation evidence

Implementation is completed on `codex/git-intelligence-01a0ad48` in the isolated worktree.

| Gate | Evidence/status |
| --- | --- |
| 1. Approved design | User approved the design and all twelve plans on 2026-09-17; project section 11 and cross-cutting sections 18-40. |
| 2. Plan | [P6 ordered plan](06-git-intelligence.md). |
| 3. RED/GREEN | Initial implementation had unhandled root commits in `diff-tree`, missing shallow repository format checks, and unmerged stage tab-separation in index records; all 11 integration tests passing after implementation. |
| 4. Affected package tests | `rtk proxy cargo test --offline --locked -p davinci-coding-agent --test git_intelligence`: exit 0 (11 passed); `rtk proxy cargo test --offline --locked -p davinci-agent`: exit 0 (73 passed, 3 ignored). |
| 5. Format | `rtk proxy cargo fmt -p davinci-agent -p davinci-coding-agent --check`: exit 0. |
| 6. Clippy | `rtk proxy cargo clippy --offline --locked -p davinci-agent -p davinci-coding-agent --all-targets -- -D warnings`: exit 0. |
| 7. Integration | 11 integration tests passed in `git_intelligence.rs`: symbol history with renames/edits, shallow & bounded acceptance guard, facts vs inference separation, changed symbols AST comparison, branch diff with merge-base, porcelain blame, conflict explain with zero mutation, option injection & path traversal security guards, non-git directory handling, caching & telemetry, permission classification and host registration. |
| 8. Security | All 7 git intelligence tools classified as `ToolClass::Read`. Option injection guards (`--end-of-options`), path traversal prevention, sanitized environment (`GIT_CONFIG_NOSYSTEM=1`, `GIT_CONFIG_GLOBAL=/dev/null`, pager disabled, zero shell interpolation), zero mutation guarantee on unmerged 3-way stages, acceptance guard reporting unknown/partial on incomplete history. |
| 9. Normal path | Normal Agent session dispatch executes `git_symbol_history`, `git_related_commits`, `git_changed_symbols`, `git_branch_diff`, `git_blame_symbol`, `git_commit_context`, `git_conflict_explain` and `/git-status` command without Graph. |
| 10. Graph | Graph role allowlist updated: `Role::Historian`, `Role::Researcher`, `Role::Reviewer`, `Role::Planner`, `Role::Writer`, `Role::TestAnalyzer` authorized; `Role::Classifier` denied (least privilege). Unit test `git_intelligence_tools_follow_role_selection` passed. |
| 11. Evaluation | [p6-git-final.json](evidence/p6-git-final.json) captures full verification evidence, supported capabilities, security invariants, and test results. |
| 12. Docs | Updated `06-git-intelligence.md` and `README.md` program ledger. |
| 13. Review | Solo source and diff audit across touched crates, fixtures, tests, and documentation. No subagents used per instruction. |
| 14. CI | Pending commit and PR creation targeting `codex/build-intelligence-01a0ad48`. |

