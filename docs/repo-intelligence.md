# Repository intelligence

DaVinci builds a lazy, persistent structural index for TypeScript and JavaScript.
Use it to orient within a repository, locate symbols, follow imports, and choose
small source ranges to read. Indexing and query routing run locally without model
calls, embeddings, Node, a language server, or network access.

## Tools

All paths are workspace-relative, use `/`, and must remain inside the workspace.
Results default to 25 entries. `limit` accepts 1–100 and is also capped by settings.
Unknown arguments and invalid paths are rejected.

| Tool | Inputs | Evidence |
| --- | --- | --- |
| `repo_map` | optional `path`, `depth` (1–12), `limit` | Module and language counts, configuration paths, likely entry points, connected files |
| `symbol_search` | `query`; optional `path`, `kind`, `limit` | Exact, prefix, substring, then subsequence name matches |
| `file_symbols` | `path`; optional `limit` | File outline, signatures, stable IDs, source ranges |
| `file_dependencies` | `path`; optional `limit` | Imports, named reexports, reverse importers, external specifiers |
| `symbol_relationships` | `symbolId` or `path` plus `line`; optional `limit` | Containment, calls, inheritance, type uses, imports/exports |
| `related_files` | `path`, `symbolId`, or task `query`; optional `limit` | Ranked dependencies, importers, distance-two neighbors, likely test pairs, configuration |
| `code_query` | `query`; optional `path`, `limit`, `includeEvidence` | Automatically selected structural, semantic-provider, and text evidence |

`/repo-index-status` reports the root, configured cache mode, initialization,
supported languages, file/symbol/edge counts, last refresh time, and parse failures.
Status does not initialize or refresh the index.

## Unified routing and provenance

Routing is deterministic. A quoted query selects bounded literal text search.
Dependency questions containing a known filename or symbol select import evidence.
An exact symbol mentioned in a question selects its definition, structural edges,
and dependencies; change/impact/test questions also include related files. Other
questions use lexical repository ranking and bounded text search, including docs
and supported text files outside the AST language set.

When a host supplies `SemanticLanguageProvider`, symbol questions can additionally
request definitions, references, implementations, type definitions, or diagnostics.
There is no Language Intelligence/LSP host on the implementation baseline
`a50fdc6`; this feature supplies the typed adapter boundary and tests it with an
injected provider. It does not start an LSP server. Provider failure falls back to
structural evidence with a warning, and outside-workspace results are rejected.

Each result identifies `source` (`ast`, `lsp`, `text`, or `config`) and `confidence`
(`structural`, `semantic`, `textual`, or `heuristic`). AST call/import links are
syntax evidence, not semantic references. Related-file scores are heuristics.
Ranges use one-based lines, zero-based UTF-8 byte columns, and exclusive ends.
`total` counts collected candidates; `truncated` marks result clipping. Text scans
also expose `text_search_partial` when a scan or output budget prevents completeness.
Parse failures and unresolved constructs are exposed separately from found facts.
Large results use the existing token governor and lossless `retrieve_output` store.

## Languages and static-analysis limits

V1 recognizes `.ts`, `.tsx`, `.js`, `.jsx`, `.mts`, `.cts`, `.mjs`, and `.cjs`.
Official Tree-sitter grammars extract functions, arrow-function variables, classes,
methods, interfaces, aliases, enums, namespaces, properties, imports, exports,
named reexports, inheritance, simple calls, and type uses. CommonJS literal
`require` dependencies and assignments exporting existing identifiers are recorded.
Malformed source can produce partial structure instead of aborting the repository.

Stable IDs hash the relative path, kind, qualified name, and duplicate occurrence.
Moving lines preserves IDs. Renaming/moving a symbol or changing the occurrence
order of duplicate declarations can change them.

Relative imports use indexed extension/index-file candidates. Plain-JSON project
path mappings and local package names provide conservative monorepo hints. Named
export aliases/defaults and named reexport chains can resolve calls, with a bounded
cycle check. This is not the TypeScript resolver: JSONC/extended tsconfigs,
conditional package exports, wildcard barrel ambiguity, namespace member dispatch,
CommonJS object/function export expressions, and module-loader customization may
remain unresolved. Metadata files still appear for orientation when hints cannot
be parsed. Dependencies never recursively index `node_modules`.

Computed calls, dynamic import expressions, reflection, runtime injection, monkey
patching, and ambiguous lexical bindings remain unresolved. The index does not
perform type checking or whole-program flow analysis. Test pairs, entry points,
and task-name matches are explicitly heuristic and can miss unconventional layouts.

## Cache, refresh, and boundaries

The agent directory stores `repo-index/<SHA-256 of canonical root>/index.json`.
The directory must be outside the workspace. JSON stores normalized records, not
source bodies or AST trees, with root, schema, parser, and configuration identities.
Source-derived names and signatures are present; protect the agent directory as
you protect other local agent state.

Construction performs no scan, parse, or cache creation. Each query walks bounded
workspace paths, hashes supported source contents, and reparses only changed files.
Deleted/renamed/ignored files are removed. Config hints are reread during refresh.
Hashing intentionally catches same-size edits with preserved timestamps. Existing
repository tools retain full content reconciliation. The shared index also exposes
an observed refresh for [test-impact planning](test-impact.md): fresh inventory,
bounded native change events, file stamps, and explicit changed-path reads allow
unchanged source records to be reused between full reconciliations.

Controllers share immutable snapshots within a process. Graph worker processes
coordinate through the same workspace cache and an OS-owned exclusive lease.
The lease releases on process exit and times out after bounded retries; a busy
lease returns an explicit error rather than duplicating a cold parse. Corrupt,
incompatible, or missing cache data rebuilds. Unavailable persistence uses memory
with a warning. Atomic temporary-file publication prevents partial JSON reads.
A bounded native watcher starts lazily with the index and ends with its last
in-process owner. Overflow, unavailable observation, and periodic/explicit full
refreshes use content hashing. Watcher hints are not an atomic filesystem snapshot.

Traversal respects nested `.gitignore` rules, generated directories, and the
existing sensitive-path policy. Symlinks and Windows reparse points are rejected,
including ignore files; reads reuse the existing confined-file primitive. Limits
include 1 MB/source file, 20,000 source files, 100,000 directory entries, depth 48,
64 MB/source bytes per refresh, 250,000 structural records, 64 MB/cache, 250 ms/parser
call, and 4 MB/text-search content. Limits can yield partial coverage with warnings.

The merged settings object accepts `repoIntelligence` with `enabled` (default true),
`persistIndex` (true), `observeChanges` (true), `maxFileBytes` (1,000,000 maximum), and `maxResults` (25,
maximum 100). Setting `enabled` false keeps schemas discoverable but rejects queries
without indexing. No MCP registration is needed. Runtime capability selection and
Graph role allowlists continue to govern tool availability.

## Evaluation and future adapters

The ignored integration evaluation `repo_intelligence_measured_evaluation` exercises
200 synthetic source files using actual existing grep/read tools as the baseline.
It requires correct evidence and lower returned context for symbol-use, change-
neighborhood, and router-orientation workflows. It measures startup construction,
cold/warm/incremental refresh, query latency, cache bytes, process peak memory,
reparse counts, tool calls, explicit content reads, and estimated tokens (bytes/4).
See [verification evidence](repo-intelligence-verification.md) for measured results
and remaining platform gates. Timings have no pass threshold; this is not a claim
that indexing beats grep on every small repository.

The normalized Rust contracts keep tool semantics language-neutral. A future Rust,
Python, or Go adapter can add grammar/extraction while preserving those tools;
rust-analyzer can implement the semantic provider boundary. Test Impact Analysis,
Git co-change/history, and Package Intelligence can add separately labeled evidence.
[Test impact](test-impact.md) now consumes the existing index; the other adapters
remain separate projects.
