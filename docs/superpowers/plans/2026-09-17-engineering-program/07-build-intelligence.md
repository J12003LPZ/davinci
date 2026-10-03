# P7: Build Intelligence

Status: design approved by the user on 2026-09-17; implementation pending.
Execution sequence: 6 of 12.
Dependencies: P5 package/workspace graph, P1 script discovery and CacheRuntime.
Requirements authority: project section 12 and cross-cutting sections 18-40;
project numbers are kept stable while execution order follows the program design.

## Outcome

Suggest repository-native build/typecheck commands for affected workspace packages with concrete configuration evidence.

## Scope and ownership

Proposed files/modules (not claims of completed edits):

- crates/davinci-coding-agent/src/native_extensions/build_intelligence/ (new)
- P5 workspace/package metadata read contracts
- Native registration/settings/permission/Graph role adapters
- New Turbo/Nx/Vite/Next/tsconfig fixtures and build_intelligence tests/evals

The main session is the only writer. No subagents, parallel writers or unrelated cleanup.
Reinspect these paths and callers at implementation time because main may advance.

## Baseline before implementation

Measure current manual package/config reading and full-workspace build selection on fixed monorepo fixtures; retain executed fixture commands and target correctness.

## Ordered implementation and RED/GREEN work

1. RED: npm/pnpm/yarn/bun detection, workspace scripts, tsconfig references, Turbo dependencies, Nx targets, Vite/Next packages and cyclic/unknown configuration.

2. Reuse package graph instead of reparsing workspace manifests. Parse supported declarative turbo.json, nx.json/project.json/package target metadata and tsconfig references with bounded reads; treat executable config as opaque evidence requiring fallback.

3. Implement workspace_packages, build_targets, build_dependencies, build_affected and build_command with typed target/config provenance and stable ordering. Reverse dependency traversal includes necessary dependent builds.

4. Emit program/argv/cwd for actual package manager and target runner; verify filter syntax with repository/installed tool evidence. Unknown plugin-inferred Nx targets and dynamic configs return incomplete plus known package fallback, never invented targets.

5. Describe native cache use (Turbo/Nx/tsbuildinfo/package stores) without replacing or clearing those caches. No automatic execution or package download.

6. Cache discovered targets/config graph in CacheRuntime Build keyed on exact package/config/lock identities; current permissions apply on retrieval.

7. Wire relevant native capabilities into normal sessions and Graph planner/verifier roles, plus independent enabled/status settings.

8. GREEN: command correctness executes only fixture-safe commands, no-eager-execution tests, config-change/cache tests, affected-package eval and crate/fmt/lint/CI.

## Required targeted validation

- Cross-package public API change, unrelated package edit, root config change, package rename, reference cycles and unknown build scripts.
- Proposed commands remain suggestions; scripts containing arbitrary text never execute during discovery.
- Normal dispatch discovers affected target, then separately authorized process/test execution produces real receipt; Graph uses same tools.
- Cache hit/invalidations and partial analysis retain broader required build checks.

## Settings and fallback

buildIntelligence.enabled defaults true; discovery is lazy and read-only. No project-native cache configuration is changed automatically.

## Specific acceptance guard

The smallest target list must include required dependents. Speed improvement cannot be claimed by omitting a failing dependent build.

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

## P7 implementation evidence

Implementation is completed on `codex/build-intelligence-01a0ad48` in the isolated worktree.

| Gate | Evidence/status |
| --- | --- |
| 1. Approved design | User approved the design and all twelve plans on 2026-09-17; project section 12 and cross-cutting sections 18-40. |
| 2. Plan | [P7 ordered plan](07-build-intelligence.md). |
| 3. RED/GREEN | Initial compilation and fixture tests failed; all 12 integration tests passing after implementation. |
| 4. Affected package tests | `rtk proxy cargo test --offline --locked -p davinci-coding-agent --test build_intelligence`: exit 0 (12 passed); `rtk proxy cargo test --offline --locked -p davinci-agent`: exit 0 (73 passed, 3 ignored). |
| 5. Format | `rtk proxy cargo fmt -p davinci-agent -p davinci-coding-agent --check`: exit 0. |
| 6. Clippy | `rtk proxy cargo clippy --offline --locked -p davinci-agent -p davinci-coding-agent --all-targets -- -D warnings`: exit 0. |
| 7. Integration | 12 integration tests passed in `build_intelligence.rs`: workspace package discovery, Turborepo target pipeline, Nx target defaults & dependsOn, TypeScript composite project references, non-omission downstream reverse dependency traversal, deterministic build command synthesis for Turbo/Nx/pnpm, cyclic dependencies, custom tasks, Vite/Next framework detection, path traversal rejection, CacheRuntime caching & telemetry, NativeExtensionHost registration and permission classification. |
| 8. Security | All 5 build intelligence tools classified as `ToolClass::Read`. Traversal attempts in package, scope, or file paths rejected. Zero discovery execution: manifests and config files inspected declaratively with no compiler or package code execution. Project-native caches preserved without modification or clearing. |
| 9. Normal path | Normal Agent session dispatch executes `workspace_packages`, `build_targets`, `build_dependencies`, `build_affected`, `build_command` and `/build-status` command without Graph. |
| 10. Graph | Graph role allowlist updated: `Role::Researcher`, `Role::Planner`, `Role::Writer`, `Role::Reviewer`, and `Role::TestAnalyzer` authorized for all 5 tools; `Role::Classifier` denied (least privilege). Unit test `build_intelligence_tools_follow_role_selection` passed. |
| 11. Evaluation | [p7-build-final.json](evidence/p7-build-final.json) captures full verification evidence, supported runners, security invariants, and test results. |
| 12. Docs | Updated `07-build-intelligence.md` and `README.md` program ledger. |
| 13. Review | Solo source and diff audit across touched crates, fixtures, tests, and documentation. No subagents used per instruction. |
| 14. CI | Green on head `950ab505d87c3569b693ca21085d2e269080551b` and PR #14 ([PR #14](https://github.com/J12003LPZ/davinci/pull/14)). Push CI: [run 35370012630](https://github.com/J12003LPZ/davinci/actions/runs/35370012630) (22/22 jobs success); Push SARIF: [run 35370012618](https://github.com/J12003LPZ/davinci/actions/runs/35370012618); PR CI: [run 35370058879](https://github.com/J12003LPZ/davinci/actions/runs/35370058879) (22/22 jobs success); PR SARIF: [run 35370058953](https://github.com/J12003LPZ/davinci/actions/runs/35370058953). |

