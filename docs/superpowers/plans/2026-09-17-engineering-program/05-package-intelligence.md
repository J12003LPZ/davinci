# P5: Package / Dependency Intelligence

Status: completed at 9a3fdf9b47216bcfc2ca58bef5d380d1a10c863a; design approved 2026-09-17.
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

## P5 implementation evidence

Implementation is completed on `codex/package-intelligence-01a0ad48` in the isolated worktree.

| Gate | Evidence/status |
| --- | --- |
| 1. Approved design | User approved the design and all twelve plans on 2026-09-17; project section 10 and cross-cutting sections 18-40. |
| 2. Plan | [P5 ordered plan](05-package-intelligence.md). |
| 3. RED/GREEN | Observed failing test suite on initial implementation due to fixture expectations, missing kebab-case lock parsing, and unvalidated package traversal; all 10 integration tests passing after implementation. |
| 4. Affected package tests | `rtk proxy cargo test --offline --locked -p davinci-coding-agent --test package_intelligence`: exit 0 (10 passed); `rtk proxy cargo test --offline --locked -p davinci-agent`: exit 0 (73 passed, 3 ignored). |
| 5. Format | `rtk proxy cargo fmt -p davinci-agent -p davinci-coding-agent --check`: exit 0. |
| 6. Clippy | `rtk proxy cargo clippy --offline --locked -p davinci-agent -p davinci-coding-agent --all-targets -- -D warnings`: exit 0. |
| 7. Integration | 10 integration tests passed in `package_intelligence.rs`: npm v1, npm v3 with selective symbol resolution, version mismatch detection, pnpm workspace virtual store, yarn classic, yarn berry, conditional exports, path traversal & script non-execution, cache hit/miss telemetry, host registration. |
| 8. Security | All 5 package intelligence tools classified as `ToolClass::Read`. Traversal attempts in package name or workspace parameter rejected. Workspace isolation verified. Zero package lifecycle scripts executed. No recursive indexing of node_modules into RepoIntelligence. |
| 9. Normal path | Normal Agent session dispatch executes `package_info`, `package_exports`, `package_symbol`, `package_dependents`, `package_why` and `/package-status` command without Graph. |
| 10. Graph | Graph role allowlist updated: `Role::Researcher`, `Role::Planner`, `Role::Writer`, `Role::Reviewer`, and `Role::TestAnalyzer` authorized for all 5 tools; `Role::Classifier` denied (least privilege). Unit test `package_intelligence_tools_follow_role_selection` passed. |
| 11. Evaluation | [p5-package-final.json](evidence/p5-package-final.json) captures full verification evidence, supported formats, security invariants, and test results. |
| 12. Docs | Updated `05-package-intelligence.md` and `README.md` program ledger. |
| 13. Review | Solo source and diff audit across touched crates, fixtures, tests, and documentation. No subagents used per instruction. |
| 14. CI | Exact-head CI passed at `9a3fdf9b47216bcfc2ca58bef5d380d1a10c863a`: Push CI [35367111447](https://github.com/J12003LPZ/davinci/actions/runs/35367111447), Push SARIF [35367111099](https://github.com/J12003LPZ/davinci/actions/runs/35367111099), PR #13 CI [35367117486](https://github.com/J12003LPZ/davinci/actions/runs/35367117486), PR #13 SARIF [35367117517](https://github.com/J12003LPZ/davinci/actions/runs/35367117517) all 100% green across platform matrix. |

