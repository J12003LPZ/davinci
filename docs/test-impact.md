# Test impact intelligence

DaVinci can suggest a small first test tier for TypeScript and JavaScript changes.
The native tools work in ordinary sessions and authorized Graph roles. Planning
runs locally over the existing repository AST index and CacheRuntime; it does not
execute scripts, launch Node, start an LSP, or make model/network calls.

## Inputs and evidence

`test_related`, `test_impacted`, and `test_plan` share the same deterministic
analysis. Use a workspace-relative `path`, a `paths` array, or existing repository
`symbolIds`. Paths use forward slashes; at most 64 combined inputs are accepted.
`limit` is 1–100 (default 25). Unknown fields are rejected. A symbol selects its
containing file's dependency impact; this is not symbol-level dataflow analysis.

Results contain stable test paths, owning package paths, and reason chains:

- `ast`: direct or transitive reverse import relationships.
- `test-map`: a matching source/test filename stem in the same package.
- `config`: package or workspace configuration can affect all its tests.
- `unresolved-import-candidate`: a possible relationship to a deleted or missing
  source, explicitly weaker than a resolved dependency.

Test filenames include `*.test.*`, `*.spec.*`, and files under `test`, `tests`,
or `__tests__` directories, within the AST index's TS/JS extensions. Each row
explains its selection. Cycles terminate, and bounded traversal can return partial
coverage. Historical test relationships are currently unavailable.

`total` counts collected tests; `remaining` counts rows omitted from this response.
Rows are capped by count and 64 KiB, and analysis evidence by 4 MiB. Narrow changed
inputs if the response is clipped. The existing token governor can retain exact
tool responses for `retrieve_output`; retained output is historical evidence.
A partial traversal's total is a lower bound, not the number of all affected tests.

## First tier and broader verification

`first_tier` and `broader_verification` contain `program`, `argv`, and relative
`cwd`, with reasons and package-manager evidence. These are suggestions requiring
the existing execution authorization. A plan is not a successful test result.

Package ownership uses the nearest bounded `package.json`. Root `workspaces`
arrays and their `packages` form recognize include/exclude globs. Package-manager
selection uses the nearest `packageManager` declaration or recognized lockfile;
ambiguous managers produce warnings, and the default npm convention is labeled.

Plain Node test scripts can run selected paths directly. Plain Vitest, Jest, and
Playwright test scripts receive their supported filters; other scripts retain the
complete package test command because their filtering semantics are unknown.
Each command has at most 64 path filters and 8 KiB of path arguments; larger sets
fall back to complete package tests. At most 32 package commands are returned,
with an explicit warning if more must be planned. Package lifecycle scripts remain
part of broader checks even when the first tier invokes Node directly.

Broader package checks remain present when no tests are found. Root configuration
changes include declared workspaces. Incomplete source or metadata analysis adds
all discovered packages to broader verification. Missing scripts and command limits
leave verification unresolved. Complete the required package/CI checks before
claiming success; selected tests never waive them.

## Freshness, permissions, and configuration

The merged settings accept `testImpact.enabled` (default true). Setting it false
disables only this capability. Malformed test-impact settings also disable this
subsystem without losing unrelated settings. Status via `/test-impact-status`
does not initialize the repository index or watcher.

Repository observation starts lazily at the first index query. Warm plans reconcile
directory inventory and file metadata, consume bounded watcher events, and reread
explicit changed paths. Unchanged source records and reverse mappings can be reused.
This avoids full source-content reads, not directory enumeration or metadata reads.

A watcher is not an atomic filesystem snapshot. Use `refresh: true` at completion
to force full content reconciliation. Observation also reconciles fully after an
interruption, overflow, backend failure, configuration/ignore change, or the
30-second full-refresh interval. Disable observation independently through
`repoIntelligence.observeChanges: false` to use full content reads every time.
A continuously changing workspace can return a retryable unavailable result.

The existing read policy is checked before each indexed source/configuration read
or reuse, and again before delivering the result. This includes warm mapping hits.
Sensitive, excluded, outside-workspace, symlink, and Windows reparse paths are
rejected. Cancelling the normal agent cancels subsequent authorization/read gates.
Snapshots are immutable, refresh work is serialized per index, and host locks are
released before analysis. Observer resources end with the last shared index owner.

Status and response telemetry report request/failure counts, failure categories,
latency, mapping hits, source bytes/files read, reparses, metadata bytes read, and
selected counts. Source counters exclude configuration/ignore reads and directory
enumeration. `indexed_tests_not_selected` describes the plan; it does not claim
those tests were actually skipped or safely omitted.

## Limits and measured evidence

This uses the existing static import resolver, not the full TypeScript module
resolver. Dynamic imports, custom loaders, runtime injection, unconventional tests,
JSONC/extended configuration, and ambiguous exports can prevent complete selection.
Package inspection does not parse installed dependencies or recursively index
`node_modules`. pnpm workspace YAML membership and detailed lockfile/package
inspection belong to the separate package-intelligence project.

The deterministic monorepo evaluation runs real generated Node tests with no
downloads or model calls. See the frozen
[baseline](superpowers/plans/2026-09-17-engineering-program/evidence/p1-monorepo-baseline.json)
and [after sample](superpowers/plans/2026-09-17-engineering-program/evidence/p1-monorepo-after.json).
On the recorded Windows debug-profile sample:

| Measurement | Before | After |
| --- | ---: | ---: |
| Tests selected first | 2 by related-files heuristic | 3 by test impact |
| Planted failures caught / present | 2 / 3 | 3 / 3 |
| Warm source bytes read | 6,173 | 59 |
| Warm source reparses | 0 | 0 |
| Warm operation latency | 15.9386 ms, index refresh only | 37.1114 ms, authorized complete test plan |
| Selected suite runtime | 86.6941 ms, misses one failure | 96.8832 ms, catches all three |
| Full mutated suite runtime | 232.0035 ms, 23 tests | 233.1745 ms, 23 tests |

The after sample had zero false-positive selected paths and zero missed planted
failures in this fixture. This is not a general accuracy guarantee. The complete
plan performs more authorization and command analysis than the old refresh-only
operation, so reduced source I/O does not imply lower total latency for every repo.
Cold planning was 83.2957 ms; an observed one-file edit was 40.8377 ms. Watcher-disabled
fallback read all 46 source files. An explicit zero-test case retained broader
verification and did not count a zero-test execution as success.

The tests cover normal Agent discovery/edit/replan/real test execution without
Graph, Graph role projection, current-policy revocation, cancellation, external
timestamp-preserving edits, concurrent refresh, bounded output retrieval, malformed
metadata, and conservative command fallbacks. CI runs the native fixture and normal
agent path on Windows, Linux, and macOS; its current result is recorded in the
[program ledger](superpowers/plans/2026-09-17-engineering-program/README.md).
