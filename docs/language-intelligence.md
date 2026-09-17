# Native Language Intelligence

DaVinci provides read-only TypeScript and JavaScript semantic tools through its
native extension host and existing capability registry. Supported files are
`.ts`, `.tsx`, `.js`, `.jsx`, `.mts`, `.cts`, `.mjs`, and `.cjs`.

| Tool | Inputs | Result |
| --- | --- | --- |
| `lsp_definition` | path, line, column | Definition locations |
| `lsp_references` | path, line, column, optional includeDeclaration | Reference locations |
| `lsp_hover` | path, line, column | Type/signature and short documentation |
| `lsp_document_symbols` | path | Document symbols, including nested symbols |
| `lsp_workspace_symbols` | query, optional path | Project symbols |
| `lsp_implementations` | path, line, column | Implementation locations |
| `lsp_type_definition` | path, line, column | Type declaration locations |
| `lsp_diagnostics` | path, optional severity | Current diagnostic publication/report |

Positions use **1-based lines and UTF-16 columns**. Paths resolve inside the
session's workspace. For workspace symbols in a monorepo, supply a source `path`
in the desired package. Severity is `all`, `error`, `warning`, `information`, or
`hint`. Every tool except hover accepts an optional `limit` from 1 to 200; settings
provide an additional upper bound. References include declarations by default.

Results have stable ordering, relative paths, ranges, totals, and remaining counts.
Hover text is capped at 4 KiB. External locations are omitted and counted. When
records exceed a cap, `fullResult` identifies the existing `retrieve_output` tool
and output ID for the normalized complete list. If storage fails, the bounded
summary still reports the remaining count without promising a retrievable copy.
Transport limits may additionally omit diagnostics; `omittedByTransport` reports
that count. Retention uses the existing governor store and retention policy.

## Installation and backend selection

DaVinci never installs or downloads a server. For compatibility, install
`typescript` and `typescript-language-server` as development dependencies using
the project's package manager. Node must be available on PATH. Follow the
[server's installation documentation](https://github.com/typescript-language-server/typescript-language-server).

Project packages are preferred, including hoisted `node_modules` up to the
workspace boundary. The nearest project TypeScript `lib/tsserver.js` is explicitly
passed to the compatibility server. This prevents a global server from silently
substituting another compiler version. Standard npm global package layouts and
direct PATH executables are fallback locations. Windows launches Node package
entry points directly; it does not execute `.cmd` or PowerShell shims. With a
global package manager layout that is not discovered, use project-local packages.

`auto` considers installed native TypeScript 7+ (`tsc --lsp --stdio`) and the
native-preview package (`tsgo --lsp --stdio`). A project with a JS-based TypeScript
installation keeps that toolchain through `typescript-language-server --stdio`.
An explicit native override can select the preview. A compatible alternative may
be attempted after initialization failure, within the same two-launch budget.
Upstream native support and installed package support can differ; unsupported
capabilities return structured errors. Native invocation was checked against the
[TypeScript native source](https://github.com/microsoft/typescript-go/tree/main/cmd/tsgo).
The actual external integration test described below exercises the compatibility
backend; native-server interoperability requires an installed native toolchain.

Language-server execution requires project trust and an existing permission grant
for the discovered launch command. Read-only semantic permission alone does not
authorize execution of project code. A denied launch reports the command requiring
permission. Servers use the existing sanitized process environment. Automatic
typing acquisition is disabled for the compatibility backend.

## Settings and status

The existing trusted global/project settings merge accepts:

```json
{
  "languageIntelligence": {
    "enabled": true,
    "typescript": {
      "enabled": true,
      "backend": "auto",
      "requestTimeoutMs": 5000,
      "maxReferences": 50,
      "maxWorkspaceSymbols": 50,
      "maxDiagnostics": 50
    }
  }
}
```

Backend values are `auto`, `typescriptNative`, and `typescriptLanguageServer`.
Timeouts range from 100 to 30,000 milliseconds; caps range from 1 to 200. Invalid
language settings disable this optional subsystem and are reported without
discarding unrelated settings. Set either enabled flag to false to disable it.

`/lsp-status` reports selected backends, server and project TypeScript versions,
project roots, process IDs, open document counts, launch counts, and last errors.
Before a session exists, it reports discovery at the workspace root. Status does
not launch servers; a nested project's actual selection appears after its first
semantic request.

## Sessions, graph workers, and freshness

The nearest ancestor containing `tsconfig.json`, `jsconfig.json`, or `package.json`
is the project root, bounded by the workspace. Server processes start only on a
semantic request. A fixed-config manager shares session slots by canonical project
root; the chosen backend is retained in its slot. Requests serialize per session.
Different projects and graph worktrees have separate sessions; workers in the same
checkout share one parent-owned session through the existing authenticated task
coordinator transport. An unavailable parent never causes a worker-local launch.

Role tool selection remains in the existing graph policy. Researcher, planner,
and writer roles may use all eight tools; reviewers get references,
implementations, and diagnostics; test analyzers get diagnostics. Classifiers and
historians get none by default. Semantic schemas remain deferred until selected.
Sessionless workers have no semantic coordinator authority.

Before each request, DaVinci refreshes every previously opened document from disk,
sending changes only when contents differ. Deleted documents close. Full text is
sent directly or as a replacement of the previous UTF-16 range for incremental
servers. There are limits of eight project sessions, 64 open documents per session,
1 MiB per source file, and 8 MiB per protocol frame. There is no DaVinci workspace
indexing scan. A dead server can restart once; exhausting the launch budget returns
an unavailable result until the DaVinci session is restarted.

Pull diagnostic reports and versioned push publications are distinguished from
unversioned publications. Older document versions are rejected. Missing or
invalidated publications return `diagnostics_pending`, never a successful empty
list. Some servers, including the tested compatibility server, publish no document
version; `unversioned-publication` explicitly exposes that freshness limitation.
Unopened dependencies rely on the server's own filesystem observation.

**Diagnostics are advisory.** Even an empty report does not replace the repository's
compiler, tests, lint, or build. There are no rename, formatting, completion, code
action, or execute-command tools. Server-initiated workspace edits are rejected.
External servers still execute local code under the user's OS permissions; this
read-only protocol is not an operating-system sandbox.

## Troubleshooting

| Result | Action |
| --- | --- |
| `server_not_installed` | Check project-local packages, Node PATH, and explicit backend choice. |
| `server_launch_denied` | Check project trust and the existing launch permission for the reported command. |
| `initialization_failed` | Check backend compatibility and timeout; inspect `/lsp-status`. |
| `unsupported_method` | Use a backend advertising that capability, or ordinary search/read tools. |
| `invalid_source_path` / `outside_workspace` | Use an existing supported file in the active checkout. |
| `request_timeout` / `server_exited` | Retry within the bounded restart policy; otherwise restart the session. |
| `diagnostics_pending` | Retry after server analysis; use compiler verification when fresh diagnostics remain unavailable. |
| `source_too_large` / `document_limit` / `session_limit` | Use ordinary tools or a new DaVinci session for the affected project. |

All these errors leave ordinary coding tools available.

## Offline tests and external verification

Protocol, session, manager, and graph transport tests use local Node fixture
processes, not internet downloads or model calls. The real-server test is ignored
by default. Set `DAVINCI_TEST_TYPESCRIPT` to an existing TypeScript package directory
and `DAVINCI_TEST_LANGUAGE_SERVER` to an existing typescript-language-server package
directory, then run:

```text
cargo test --offline --locked -p davinci-coding-agent --lib real_typescript_semantics -- --ignored --nocapture
```

The test copies those installed packages into a temporary project, creates two
TypeScript source files, checks all eight operations, introduces a type error,
fixes it, and checks synchronization. It prints constructor, first-request, and
warm-request latency plus session identity. It does not modify installed packages.

## Future languages

The transport, session, normalization, tool API, permission path, and graph channel
are reusable. Language-specific extension IDs, project markers, package discovery,
commands, and initialization options live behind the server layer. A fake adapter
test exercises independent project markers/extensions. A future rust-analyzer
phase must add Rust discovery and an adapter, backend capability tests, and real
Rust fixtures. Rust support and mutation tools are intentionally absent from V1.

## Measured verification (Windows, 2026-09-16)

The debug build passed the offline real-server fixture using TypeScript 5.9.3 and
typescript-language-server 5.3.0: all eight tools and the type-error/edit cycle.
Lazy manager construction took 26 microseconds, the first definition 763 ms, and
a warm hover 7.7 ms. These are single-run observations, not performance guarantees.
The native TypeScript backend has discovery/protocol tests but was not installed
for this external verification.

The executable's offline RPC startup, successful `get_state`, and clean exit took
a median 422.8 ms with language intelligence enabled and 415.0 ms disabled. Each
condition had five samples after a discarded warmup, in alternating order. No
language server started despite an installed fixture package. Reproduce with:

```text
node crates/davinci-coding-agent/tests/fixtures/language-startup-benchmark.cjs target/debug/davinci.exe
```

Use `target/debug/davinci` on Unix. The probe uses an isolated temporary workspace
and configuration and prints its location. This measures offline CLI startup,
not interactive TUI rendering or network/model latency.

The authenticated graph-channel fixture ran four simultaneous worker processes,
one shared LSP process, and parent output retrieval. Summed worker RSS was
218,038,272 bytes. This is a deterministic transport fixture, not a model-backed
graph workload or a measurement of the TypeScript server's memory.

Local gates passed: formatting, workspace/all-target Clippy with warnings denied,
the coding-agent package tests, and agent package tests. External-server tests
remain opt-in; ordinary protocol tests require Node but no network downloads.
