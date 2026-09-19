# P5: Package / Dependency Intelligence

Status: design approved by the user on 2026-09-17; implementation pending.
Execution sequence: 5 of 12.
Dependencies: P1 workspace metadata and CacheRuntime; P3 complete in sequential delivery.
Requirements authority: project section 10 and cross-cutting sections 18-40;
project numbers are kept stable while execution order follows the program design.

## Outcome

Read actual installed TS/JS package version, exports/types and dependency reasons without installing or executing package code.

## Scope and ownership

Proposed files/modules (not claims of completed edits):

- crates/davinci-coding-agent/src/native_extensions/package_intelligence/ (new)
- native_extensions/workspace_metadata/ shared P1 discovery
- native_extensions/mod.rs, extension_host.rs, settings.rs and normal host wiring
- crates/davinci-agent/src/{permission,permission_risk}.rs; Graph roles
- New npm/pnpm/yarn/workspace/installed-package offline fixtures and package_intelligence tests/evals

The main session is the only writer. No subagents, parallel writers or unrelated cleanup.
Reinspect these paths and callers at implementation time because main may advance.

## Baseline before implementation

Fixture task using ordinary manifest/type-file reads: count calls/bytes and whether guessed API exists for two installed versions. Capture cold/warm selective reads and missing node_modules behavior.

## Ordered implementation and RED/GREEN work

1. RED: declared range differs from lock/installed version, conditional exports, scoped package, multiple workspace versions, missing types, conflicting lockfiles and selective symlinked pnpm layout.

2. Reuse minimal workspace discovery from P1, then parse package.json, package-lock.json, pnpm-lock.yaml and yarn.lock with bounded maintained parsers or deliberately specified format adapters. Verify upstream formats and local dependencies before implementation; unsupported versions return partial, never invented exact resolution.

3. Resolve requested package in its actual workspace scope; record installed manifest/version, lock provenance, declared ranges, type entry, exports and dependencies/dependents. Handle workspace links and pnpm links by validating targets against approved workspace/store roots, never generic symlink escape permission.

4. Implement package_info/exports/symbol/dependents/why with stable bounded evidence. Selectively inspect requested installed .d.ts/source metadata for qualified symbols such as z.object. Never evaluate package code, loaders, config JS or lifecycle scripts.

5. Use installed package evidence first, lock evidence second and manifest range last, with conflicts/missing installation stated. Follow conditional exports conservatively with environment conditions visible; do not equate declared exports with semantic symbol existence.

6. Cache parsed manifests/locks and selective package results through CacheRuntime Package namespace with content hashes, installed version and workspace identity. Current read authorization applies before lookup and return; invalidation catches install/lock/content changes.

7. Register read-only native tools, deferred dependency-task activation, independent setting/status and normal mode. Graph researchers/writers receive the same bounded tools when allowed.

8. GREEN: installed-version API correctness, path/security/failure tests, parallel same-package lookup, selective read eval, related crate/fmt/lint/CI; document actual lock formats and conditional-resolution limits.

## Required targeted validation

- npm v1/v2/v3 locks; pnpm supported lock generations; Yarn classic/Berry fixtures, workspaces and duplicate versions, aliases/optional/peer dependencies; explicit unsupported formats.
- Malicious package name/path/export target, missing/broken store links, sensitive/outside files and a lifecycle script that must never run.
- Warm read count and cache invalidation on same-version content change; conflicting metadata cannot yield authoritative fabricated API.
- Normal agent package lookup -> safe edit -> fixture type/test evidence without Graph; Graph role discovery and cache revocation.

## Settings and fallback

packageIntelligence.enabled defaults true with lazy selective reads. Store roots come from validated installation metadata/trust, not model arguments.

## Specific acceptance guard

Do not recursively index node_modules in RepoIntelligence. No API correctness gain is claimed from guessed or merely declared package versions.

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
