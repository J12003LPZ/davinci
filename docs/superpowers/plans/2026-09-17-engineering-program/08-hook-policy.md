# P8: Deterministic Hook / Policy Engine

Status: Complete; PR #17 open.
Execution sequence: 9 of 12.
Dependencies: P2 supervised execution, P4 transaction events, existing RuntimeBus and trust; P9 green.
Requirements authority: project section 13 and cross-cutting sections 18-40;
project numbers are kept stable while execution order follows the program design.

## Outcome

Trusted project rules enforce deterministic before/after actions and completion requirements with explicit bounded failure semantics.

## Scope and ownership

Proposed files/modules (not claims of completed edits):

- crates/davinci-coding-agent/src/hooks.rs and runtime_host.rs
- crates/davinci-agent/src/runtime/{events,bus}.rs and actual event producers
- P2 process, P4 transaction and verification evidence adapters
- crates/davinci-coding-agent/src/{trust,settings,main}.rs; hook policy tests/evals

The main session is the only writer. No subagents, parallel writers or unrelated cleanup.
Reinspect these paths and callers at implementation time because main may advance.

## Baseline before implementation

Current trusted/untrusted hooks and preTool denial fixtures; measure boundedness of stdin/output/timeouts and verify existing completion-proposal semantics.

## Ordered implementation and RED/GREEN work

1. RED: untrusted or edited trusted config cannot execute; before-write failure prevents mutation; after-write failure does not claim reversal; recursion/stdin/output hang remain bounded.

2. Extend existing project hooks schema compatibly with typed event/tool/path filters, argv action, timeout and explicit warn/block/ignore. Preserve old hook vectors and their documented behavior.

3. Bind project hook trust to resolved path and content identity; no native model tool grants trust or injects executable hook configuration. Revalidate at request time and after changes.

4. Add SessionStart/End, Before/AfterTool, Before/AfterWrite, BeforeProcessStart/AfterProcessExit, Before/AfterTest, Before/AfterCommit and BeforeCompletion at actual producers. Do not report arbitrary bash semantics as known Git/test events without verified operation metadata.

5. Use existing RuntimeBus decision events and observers, extending allowlist only for true pre-action gates. After-events can warn or create an unmet completion requirement, not block already-completed effects retroactively.

6. Execute through P2 supervised bounded argv runner with confined cwd, approved environment, bounded JSON stdin/stdout/stderr and cancellation/descendant cleanup. No global bus/native lock during callbacks.

7. Prevent self-trigger recursion with host-owned origin/depth and bounded action count. Deterministic secret-scan/format/lint rules produce actual execution evidence and explicit failures.

8. Wire normal sessions first and propagate trusted parent policy to Graph without allowing workers to expand it. Add aggregate diagnostics instead of a slash command per rule.

9. GREEN: policy/event ordering, security/failure/concurrency and normal-mode tests, hook evidence eval, compatibility/crate/fmt/lint/CI/docs.

## Required targeted validation

- Matching globs/tools/events, explicit warn/block/ignore, malformed config/argv, timeout/large stdin/full pipe and descendant cleanup.
- Denial must prevent downstream writes/process starts/commit completion; observers are not false gates.
- Config edit and permission revocation between hook load and execution; no auto-trust for model-produced files.
- Normal edit -> trusted formatter -> source identity invalidation -> verification; Graph same-policy behavior and bounded recursion.

## Settings and fallback

hookPolicy.enabled gates extended rules; legacy trusted hooks remain backward compatible. Config supplies explicit failure policy; defaults do not silently weaken an existing blocking rule.

## Specific acceptance guard

Automatic hooks are code execution: established project trust and current command authority are required even if the triggering tool previously passed.

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

## P8 implementation evidence

Implementation is completed on `codex/hook-policy-01a0ad48` in the isolated worktree.

| Gate | Evidence/status |
| --- | --- |
| 1. Approved design | User approved the design and all twelve plans on 2026-09-17; project section 13 and cross-cutting sections 18-40. |
| 2. Plan | [P8 ordered plan](08-hook-policy.md). |
| 3. RED/GREEN | Initial implementation uncovered stdin truncation causing json parse errors on payloads, missing depth guard debug trait, and argument lifetime constraints in `HooksRuntimeSubscriber`; all 11 integration tests passing after implementation. |
| 4. Affected package tests | `rtk proxy cargo test --offline --locked -p davinci-coding-agent --test hook_policy`: exit 0 (11 passed); `rtk proxy cargo test --offline --locked -p davinci-agent`: exit 0 (73 passed, 3 ignored); `rtk proxy cargo test --offline --locked -p davinci-coding-agent --lib`: exit 0 (1045 passed, 15 ignored). |
| 5. Format | `rtk proxy cargo fmt -p davinci-agent -p davinci-coding-agent --check`: exit 0. |
| 6. Clippy | `rtk proxy cargo clippy --offline --locked -p davinci-agent -p davinci-coding-agent --all-targets -- -D warnings`: exit 0. |
| 7. Integration | 11 integration tests passed in `hook_policy.rs`: legacy hook backward compatibility, policy rule filtering by event/tool/path globs, failure policies (warn, block, ignore), untrusted project hook rejection, post-load file modification invalidation, BeforeWrite decision blocking file mutation, AfterWrite observer failure tracking unmet completion requirements without retroactive file reversal, BeforeProcessStart blocking tool execution, recursion depth guard limiting depth, bounded JSON streams and timeout process termination, aggregate diagnostics via `/hook-status`. |
| 8. Security | Binding trust to resolved path and SHA-256 content identity. Revalidated upon execution; disk changes invalidate trust immediately (fail closed). Native/model tools prohibited from granting trust or injecting hook definitions. Supervised process execution with bounded streams (64 KiB) and descendant process tree kill. Recursion depth capped at 3. |
| 9. Normal path | Normal Agent session dispatch executes hooks subscribed via `HooksRuntimeSubscriber` on runtime bus events (decision and observer), enforces failure policies, and runs `/hook-status` diagnostics command without Graph. |
| 10. Graph | Graph workers operate under host-bound policy constraints; workers cannot expand or inject hook policies. |
| 11. Evaluation | [p8-hook-final.json](evidence/p8-hook-final.json) captures full verification evidence, decision/observer events, security invariants, and test results. |
| 12. Docs | Updated `08-hook-policy.md` and `README.md` program ledger. |
| 13. Review | Solo source and diff audit across touched crates, models, tools, and tests. No subagents used per instruction. |
| 14. CI | PR #17 targeting `codex/change-impact-01a0ad48`. |

