# Native repository intelligence

## Reference and acceptance

Implementation baseline: remote main a50fdc6, fetched 2026-09-16. The user attachment
`8d682730-4d4b-483c-a097-57971f12674d/pasted-text-1.txt` is the full acceptance
contract, including all 36 acceptance criteria and its verification commands.
This document records implementation choices; it does not replace that contract.

Outcome: answer repository orientation, symbol, dependency, and change-neighborhood
questions with bounded evidence instead of repeated whole-file reads. Measure
tool calls, bytes read, returned bytes/tokens, latency, and answer correctness
against grep/read fixture workflows.

## Existing architecture

`NativeExtensionHost` owns native controllers and the governor. `ExtensionHost`
registers their specs with `RuntimeCapabilityRegistry`; tool classification supplies
read-only policy. Graph role allowlists and worker hooks enforce additional limits.
Graph workers are child processes, so an Arc alone cannot share the index between
workers. Native tool calls normally hold a broad host mutex; intelligence calls
must clone their controller and release that mutex before filesystem/parser work.
The governor already retains oversized outputs for `retrieve_output`.

Current main contains no Language Intelligence/LSP implementation. Do not copy the
separate language-intelligence worktree. Define a semantic provider trait for
definitions, references, implementations, type definitions, and diagnostics, with
an unavailable default. AST evidence remains structural even when enriched.

No existing parser dependency exists. Existing storage is lightweight JSON/JSONL
and SQLite sessions. Existing security storage demonstrates Windows exclusive
file handles and Unix flock leases. Existing graph storage publishes JSON atomically.
The ordinary search ignore helper is private and omits gitignore negation; it is
not adequate for correct nested repository ignores.

## Choices considered

| Concern | Options | Choice |
| --- | --- | --- |
| Parser | Tree-sitter; SWC; TypeScript subprocess | Tree-sitter with official JS and TS grammars. Local error-tolerant syntax trees, no Node install or compiler process; SWC introduces more compiler surface. Pin dependencies exactly, compatible with Rust 1.83. |
| Storage | Atomic versioned JSON; existing SQLite; per-file JSON | Start with atomic JSON under the agent cache directory. Simple inspectable normalized data. OS lease serializes cross-process refresh and publication. Measure cache size and warm-load latency before considering SQLite. |
| Invalidation | mtime; content hashes; watcher | Hash bounded source files on queries; parse only changed hashes. mtime alone misses same-size/timestamp edits, and watchers add lifecycle/recovery work. Reuse unchanged file records and publish an immutable snapshot. |
| Ignore | private search helper; git subprocess; ignore crate | Evaluate pinned ignore crate for nested rules and standalone repositories. No arbitrary subprocess or scan outside canonical root. |

Parser decision: Tree-sitter 0.24.7 fits the pinned serde_json 1.0.134 and Rust 1.83
baseline; 0.25.10 required a conflicting serde_json minimum. Grammar versions are
TypeScript 0.23.2 and JavaScript 0.23.1. The lockfile pins compatible transitive
tree-sitter-language 0.1.5 and globset 0.4.15.

Public source references: https://docs.rs/crate/tree-sitter/0.24.7 and
https://github.com/tree-sitter/tree-sitter-typescript (official grammars).

## Data and boundaries

Language-neutral files, symbols, ranges, imports, exports and typed edges. Stable
symbol identity uses relative path, kind, qualified name and duplicate occurrence,
never line numbers. Import targets may be local, external, or unresolved. Calls
are resolved only where lexical structure supports them; ambiguous dynamic targets
stay unresolved. Do not promote spelling matches into semantic references.

Each workspace shares a lazy controller via weak in-process registry. Cross-process
workers coordinate a single persisted workspace index with an OS lease, reload
the latest generation under that lease, and reparse only changed files. Read-only
snapshots are immutable; refresh is serialized separately. No AST trees are retained.
Cache compatibility covers root, schema, parser and effective limits. Failed
cache loads rebuild; failed persistence leaves useful in-memory queries available.

Bound traversal, files, bytes, parser work, normalized records, query length and
result count. Reject traversal, symlinks/reparse points, protected files and linked
cache directories. Do not index internal state, generated build directories or
node_modules. Record partial coverage and parse failures.

## Ordered implementation and validation

1. Parser/model: offline fixtures for all eight extensions, symbols, exports,
   imports, class structure, calls, malformed input and stable identities.
2. Scanner/cache/manager: nested ignores, boundaries, hashes, new/changed/deleted/
   renamed files, corruption, bounded input and concurrent/process refresh.
3. Graph/query: dependency resolution, cycles, distances, deterministic ranking,
   test/config pairing, compact maps and outlines, provenance and limits.
4. Unified routing: exact symbols, dependency/semantic intent, literal and bounded
   text fallback; injected semantic provider tested without LSP processes.
5. Native integration: settings, host specs/execution/status, capability classification,
   graph allowlists and governor recovery. No broad host lock during query I/O.
6. Deterministic evaluation and performance measurements: unused startup, cold/warm,
   incremental, symbol/related/unified latency, cache bytes, memory and reparsed count.
7. Required format, clippy and two package test gates; scoped commits, feature-branch
   push, PR and relevant CI. Preserve shared-checkout changes. No subagents.

Every behavior task gets its regression test before implementation. Record actual
test/evaluation outcomes in the completion evidence; zero selected tests is not a pass.

## Initial evidence

- Shared checkout has unrelated dirty/untracked files; isolated worktree used.
- No previous repo-intelligence implementation found on fetched main.
- Baseline native registry test passed before feature integration.
