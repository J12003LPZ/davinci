# P1: Test Impact Intelligence

Status: design approved by the user on 2026-09-17; implementation in progress.
Execution sequence: 1 of 12.
Dependencies: Existing RepoIntelligence and CacheRuntime.
Requirements authority: project section 6 and cross-cutting sections 18-40;
project numbers are kept stable while execution order follows the program design.

## Outcome

Select the smallest explained first test tier for a TS/JS edit while retaining required broader completion checks. Warm local edits must avoid rereading every unchanged source file when continuous change observation is available.

## Scope and ownership

Proposed files/modules (not claims of completed edits):

- crates/davinci-coding-agent/src/native_extensions/test_impact/ (new typed analysis, discovery and tool adapters)
- crates/davinci-coding-agent/src/native_extensions/repo_intelligence/{manager,index,graph,mod}.rs (typed dependency access and bounded incremental invalidation)
- crates/davinci-coding-agent/src/native_extensions/workspace_metadata/ (new minimal shared manifest/package ownership discovery; later reused by P5/P7)
- crates/davinci-coding-agent/src/{settings,extension_host,main}.rs and native_extensions/mod.rs
- crates/davinci-agent/src/{permission,permission_risk}.rs and prompt/capabilities/ routing as required
- native_extensions/graph/roles.rs; coding-agent tests/test_impact.rs and tests/test_impact_eval.rs (new)

The main session is the only writer. No subagents, parallel writers or unrelated cleanup.
Reinspect these paths and callers at implementation time because main may advance.

## Baseline before implementation

Use the existing RepoIntelligence refresh/related_files and repository scripts on a fixed monorepo fixture. Record correct tests, unselected planted failures, source bytes/read count, parse count, cold/warm/incremental latency and selected versus full-suite runtime. The current refresh rereads all bounded source contents.

## Ordered implementation and RED/GREEN work

1. RED: fixture packages/auth, web and unrelated package; paired test, direct/transitive import test, cycle, alias, exported symbol, deleted source, config change and unrelated tests. Assert deterministic reasons and package ownership; reproduce full warm source-read behavior.

2. Expose normalized dependency edges and source identity from RepoIndex through a small public read API. Preserve existing resolver/parser semantics; unresolved/dynamic imports remain explicitly incomplete.

3. Add lazy bounded change observation owned by RepoIntelligence. Validate maintained cross-platform backend/dependency before adding it. Subscribe before scan; drain queued changes; invalidate config/ignore/root changes and overflow; rescan on watcher failure. Test same-size timestamp-preserving edits, external writers, add/delete/rename and worktree separation.

4. Cache reverse dependency/test mapping in CacheRuntime Test namespace, keyed on exact frozen source/config identity. Use current authorization around computation and result delivery. Mapping reuse does not establish source freshness.

5. Discover package.json ownership, declared workspace membership and test scripts from bounded confined reads. Recognize *.test/spec.{ts,tsx,js,jsx,mts,cts,mjs,cjs}, __tests__, tests and test. Framework evidence comes from manifest scripts/dependencies/config, never executing them.

6. Implement test_related, test_impacted and test_plan over changed paths/symbol IDs. Include paired, direct and bounded transitive evidence chains. Identify Vitest/Jest/Node/Playwright and custom scripts. Historical evidence is used only if independently reliable; report unavailable until P6 can supply it.

7. Produce command argv/cwd suggestions with framework-compatible filters and an explicit broader package/workspace verification tier. Unknown script/filter semantics get a package command plus warning, not a fabricated targeted invocation.

8. Integrate settings testImpact.enabled, lazy native registration, deferred task-relevant discovery, Read classification and sensitive-path checks. Clone service handles before slow work outside the host lock. Wire normal sessions first, then Graph planner/test-analyzer roles and aggregate status.

9. GREEN: run targeted tests and eval. Review source-read and missed-failure results, then related crate tests, fmt, Clippy and exact-head CI. Document supported frameworks, conservative fallback and partial analysis.

## Required targeted validation

- Deterministic stable ordering and explanatory chains; monorepo boundary and cross-package imports; malformed manifest/unresolved import/disabled index do not masquerade as no tests.
- Normal Agent -> discovery -> authorized native call -> existing edit -> updated plan -> fixture test command evidence, with Graph absent. Revocation after warm cache hit denies returned evidence.
- Graph role projection exposes only authorized test capabilities and retrieve_output; no Graph-prefixed duplicate tools.
- Concurrency: same-index requests share work; startup/watch overflow/cancellation cannot deadlock or return stale complete evidence.
- Offline eval compares selected tests against full fixture suite with planted failures and mandatory broader checks. Report zero-test, false-positive and false-negative cases explicitly.

## Settings and fallback

testImpact.enabled defaults true with lazy construction; cache follows host config. No eager watcher at DaVinci startup. Disabled path retains ordinary edit/test execution.

## Specific acceptance guard

P1 cannot pass the performance gate by counting only AST reparses while rereading all sources. Windows warm local edit and fallback metrics must be separately evidenced.

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
