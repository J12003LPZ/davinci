# P4: Transactional Edit Engine

Status: design approved by the user on 2026-09-17; implementation pending.
Execution sequence: 3 of 12.
Dependencies: P2 green; existing apply_patch journal, mutation confinement, effects/checkpoints/rewind.
Requirements authority: project section 9 and cross-cutting sections 18-40;
project numbers are kept stable while execution order follows the program design.

## Outcome

Multi-file edits have recoverable provenance; stale application and unsafe rollback return conflict without overwriting another actor.

## Scope and ownership

Proposed files/modules (not claims of completed edits):

- crates/davinci-agent/src/apply_patch.rs and tools.rs
- crates/davinci-agent/src/runtime/{effects,checkpoints,rewind,source_manifest}.rs; new transaction lifecycle module only as needed
- crates/davinci-coding-agent/src/semantic/rename.rs (existing transactional replacement consumer)
- crates/davinci-coding-agent/src/{extension_host,main,settings}.rs and native adapters
- New transaction integration, crash-recovery and normal-session tests

The main session is the only writer. No subagents, parallel writers or unrelated cleanup.
Reinspect these paths and callers at implementation time because main may advance.

## Baseline before implementation

Run current patch/rewind fixtures, record preimage/postimage/journal behavior, demonstrate missing explicit lifecycle across ordinary write/edit calls. Use temporary files and byte hashes, not user files.

## Ordered implementation and RED/GREEN work

1. RED: preview -> externally change one file -> apply refuses every write; apply -> external edit -> rollback refuses to overwrite it. Include multi-file failure/crash and semantic replacement paths.

2. Define transaction identity/owner/workspace/base revision, affected files, before/proposed/applied hashes, sequence and verification state using existing effects/checkpoint contracts. Lifecycle is draft, previewed, applied, verified, committed, rolled_back or conflicted with checked transitions.

3. Wrap existing write/edit/apply_patch and semantic replacements through one transaction coordinator. Expose patch_preview/apply/status/rollback only where needed; keep existing tool compatibility and automatic provenance for single-file calls.

4. Resolve paths with existing confined primitives; reject linked/reparse/sensitive/outside targets according to current policy. Under the existing workspace mutation lane, reauthorize and verify every before-hash before any file write.

5. Persist journal/preimages durably before mutation, bound transaction file/byte counts, and publish applied hashes/effects after successful writes. Preserve create/delete/rename semantics and permissions; fail closed on journal/storage error.

6. Rollback/recover only files whose current bytes and identity match the transaction-owned postimage. Preserve conflict journal for inspection; never use blanket Git reset/checkout to undo. Pin durable recovery blobs independent of evictable computation cache.

7. Connect writes to repo/LSP invalidation and source-bound verification. Only actual current evidence can mark verified; only observed Git commit state can mark committed. Record session/agent/Graph node and sequence in provenance.

8. Wire native status/preview/rollback with current permissions and normal session attachment, then Graph reuse; cancellation and shutdown leave recoverable journals.

9. GREEN: fail-injection, concurrent actor, normal edit/verify and cross-worktree tests; rollback eval; affected package/fmt/lint/CI; document filesystem race and recovery limits.

## Required targeted validation

- New/edit/delete/rename and multi-file patch; stale hashes with same timestamp/length; case/path normalization; symlink/reparse escape; disk-full/write failure.
- Cancellation after journal and between writes; crash recovery with correct owned bytes; unrelated new user content preserved.
- Real normal dispatch wraps edits without Graph, reports transaction ID/effects and invalidates previous successful evidence.
- Concurrent transactions serialize only mutation, not analysis/process I/O; retry cannot apply twice or silently adopt another owner.

## Settings and fallback

editingTransactions.enabled controls explicit coordinated transactions. Existing stale-state/journal safety is never disabled as an authorization bypass. Default enable only after compatibility and recovery gates pass.

## Specific acceptance guard

Do not claim atomic multi-file OS writes. Prove journaled recovery and safe conflict handling through actual file mutations and fault injection.

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
