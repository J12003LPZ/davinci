# P9: Change Impact Engine

Status: design approved by the user on 2026-09-17; implementation pending.
Execution sequence: 8 of 12.
Dependencies: P1 tests, P4 transactions, P5 packages, P6 Git, P7 builds, Repo AST/LSP.
Requirements authority: project section 14 and cross-cutting sections 18-40;
project numbers are kept stable while execution order follows the program design.

## Outcome

Explain the likely edit blast radius across semantic references, structure, tests, packages, builds, API/config and browser flows.

## Scope and ownership

Proposed files/modules (not claims of completed edits):

- crates/davinci-coding-agent/src/native_extensions/change_impact/ (new composition layer)
- RepoIntelligence SemanticLanguageProvider adapter to existing LanguageIntelligence as needed
- P1/P4/P5/P6/P7 typed result interfaces; native/settings/Graph adapters
- New impact analysis integration fixtures/tests/evals

The main session is the only writer. No subagents, parallel writers or unrelated cleanup.
Reinspect these paths and callers at implementation time because main may advance.

## Baseline before implementation

Compare existing related_files/LSP/manual package/test lookup on a fixed exported-auth-symbol change and UI/config change; record missed downstream effects and tool/read counts.

## Ordered implementation and RED/GREEN work

1. RED: direct/transitive impact, public export, root config, UI path, symbol/transaction inputs and missing LSP produce explained sections with honest completeness.

2. Implement impact_analyze over bounded files, symbol IDs or host-owned transaction ID. Resolve exact before/proposed/applied source identities through P4, not model-supplied transaction paths.

3. Compose AST dependency edges, LSP references, packages, tests, builds and useful Git evidence through typed read APIs. Wire existing semantic-provider seam to existing LSP manager only after demonstrating the missing integration.

4. Return sections Direct semantic impact, Structural impact, Tests, Packages, Build targets, Public API risk, Configuration impact and Potential browser flows. Each item carries evidence source and concrete path/range/command/commit.

5. Deduplicate deterministically, preserve source-specific certainty and warnings, bound fanout/call count/timeouts, and make incomplete LSP/AST/build analysis visible. No opaque numeric confidence alone.

6. Use CacheRuntime only for deterministic source-bound compositions; revalidate current authority and invalidate on transaction/source/config/version changes.

7. Expose relevant normal-mode capability and Graph planner/reviewer adapter; analysis never executes recommended commands or edits.

8. GREEN: integration/permission/timeout/partial tests, impact correctness/read-count eval, related crate/fmt/lint/CI and supported-boundary docs.

## Required targeted validation

- 12 semantic references fixture plus structural imports must remain separately labeled; dynamic unresolved imports stay incomplete.
- Transaction preview input versus applied input, deleted symbols, cross-package reexports, API/config changes, unrelated file and disabled dependencies.
- Normal edit is informed by impact and then verified by real commands; plan/analysis alone cannot mark success.
- Partial source failures, cache revocation and cancellation do not crash normal session or emit misleading complete impact.

## Settings and fallback

changeImpact.enabled defaults true with lazy dependent calls and fixed work limits. Disabled/missing providers yield structured partial evidence.

## Specific acceptance guard

The composition cannot silently call LSP syntax results semantic, or report absence of evidence as evidence of no blast radius.

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
