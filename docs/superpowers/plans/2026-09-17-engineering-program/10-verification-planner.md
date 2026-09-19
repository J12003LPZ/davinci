# P10: Verification Planner

Status: completed; merged in PR #18 onto `origin/main` `5585fd6`. Design approved 2026-09-17.
Execution sequence: 10 of 12.
Dependencies: P1, P3, P7, P8, P9 and existing source-bound evidence/completion.
Requirements authority: project section 15 and cross-cutting sections 18-40;
project numbers are kept stable while execution order follows the program design.

## Outcome

Produce the cheapest valid verification sequence for the change and require actual matching evidence before completion.

## Scope and ownership

Proposed files/modules (not claims of completed edits):

- crates/davinci-coding-agent/src/native_extensions/verification_planner/ (new deterministic rules)
- crates/davinci-agent/src/runtime/{completion,evidence,contracts}.rs only for necessary integration
- P1/P3/P7/P8/P9 typed adapters; native/settings/normal/Graph surfaces
- New verification_plan fixtures, rule tests and completion integration evals

The main session is the only writer. No subagents, parallel writers or unrelated cleanup.
Reinspect these paths and callers at implementation time because main may advance.

## Baseline before implementation

Current completion/evidence behavior with broad suite recommendations, stale evidence, diagnostic-only and fake-browser receipts. Record runtime/tests and incorrect completion counts.

## Ordered implementation and RED/GREEN work

1. RED: UI Button.tsx edit yields diagnostics, paired test, package typecheck/lint, browser action; public API and security changes expand requirements; docs-only edits do not start browser.

2. Compose changed files/symbols/transaction, package/test/build/impact and CI configuration evidence. Parse known declarative CI commands without executing repository config; opaque/dynamic scripts are explicit unresolved requirements.

3. Implement verification_plan as deterministic ordered tiers with reasons, dependencies, program/argv/cwd, source identity and required/optional status. Cheapest reliable checks precede broader required checks.

4. Define rules for auth, crypto, permissions, credentials, process execution and dependency manifests to require security/compatibility evidence. Preserve repository CI parity and explicit user completion requirements.

5. Integrate plans with existing verification dimensions/evidence records rather than add a second completion engine. Actual process exits, test assertions, browser receipts and CI outcomes satisfy only matching current required checks.

6. Invalidate verification on source/config/transaction changes. Zero tests, missing runtime, diagnostic-only checks, stale outputs and unknown coverage cannot become passing completion.

7. Expose normal sessions and Graph verifier role via same native capability. Hook before-completion gate can inspect required evidence but cannot invent it.

8. GREEN: rule/CI matching, honest-failure/security/normal integration tests, measured targeted versus full-fixture runtime and missed failures, related crate/fmt/lint/CI/docs.

## Required targeted validation

- Simple text edit activates no process/browser/package layer unnecessarily; frontend and Git regression prompts activate appropriate deferred capabilities.
- Auth/permission/manifest changes strengthen required checks; public API change requires affected dependent build.
- Failing/skipped/zero-case/stale or wrong-source receipt blocks successful completion; actual current pass unlocks only covered dimensions.
- Offline end-to-end normal dispatch uses plan -> actual commands -> source-bound evidence; Graph identical rule output under same authority.

## Settings and fallback

verificationPlanner.enabled independently disables recommendations; required existing completion safety is not bypassed. Deterministic rules are not an LLM service.

## Specific acceptance guard

A verification plan is never itself verification. Lower test counts only count as improvement when required planted failures remain caught.

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
