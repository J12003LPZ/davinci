# Native Language Intelligence

Design baseline: freshly fetched `origin/main`, a50fdc6dfb8fa6aad8829299e8aaac258bf15477.
Scope: all 34 sections of the supplied Language Intelligence specification.
Implementation remains incomplete until every acceptance gate below has evidence.

## Outcome and boundaries

DaVinci can answer semantic TypeScript/JavaScript questions using eight native,
read-only tools. A warmed workspace reuses its server, including across graph
worker processes. No language server starts during ordinary startup. Compiler,
lint and test results remain the verification authority.

V1 extensions: ts, tsx, js, jsx, mts, cts, mjs, cjs. No Rust support, completion,
formatting, rename, code actions, executeCommand, or workspace edits.

## Integration established by inspection

- `native_extensions/mod.rs`: NativeExtensionHost owns controllers, advertises
  NATIVE_TOOLS/command_specs and routes execution. `after_tool` already invokes
  TokenGovernor, including the existing lossless output store/retrieve_output.
- `extension_host.rs`: native_tool_specs becomes RuntimeCapabilityRegistry
  entries through capabilities/register_with. No independent tool registry.
- `davinci-agent/src/permission.rs` and `permission_risk.rs`: classify the exact
  eight tools and subject their paths to existing boundary and sensitive rules.
- `native_extensions/graph/roles.rs`: authorization ceiling and small initial
  schema projection remain separate. Classifiers get no semantic tools.
- `graph/controller.rs`, `worker.rs`, `types.rs`: graph workers are child
  processes, not threads. An Arc alone cannot satisfy shared-session acceptance.
- `davinci-agent/src/runtime/task_transport.rs`: existing per-attempt,
  authenticated loopback transport checks worker allowlists and parent policy.
  Extend this transport with a narrow host-owned tool handler and bounded timeout
  selection; preserve task-command defaults. No second worker runtime or IPC
  service. No fallback to a worker-local server if parent transport fails.
- `settings.rs`: trusted global/project JSON merge already exists. Add optional
  languageIntelligence settings with serde defaults and strict range validation.

## Components

`language_intelligence/mod.rs` is the host facade. `manager.rs` owns an Arc of
session slots keyed by canonical project root within a fixed configuration; each
slot retains its selected backend identity.
Creation is serialized per slot; requests within one session are serialized.
Discovery is lazy and cached; dead sessions get one bounded restart. A finite
session/document limit prevents indefinite accumulation. Shutdown releases all
children. Ordinary coding remains available on structured LSP failure.

`servers.rs` implements a language adapter boundary: extension/language IDs,
project markers, discovery, initialization options. Generic session/transport
and tool names do not depend on TypeScript. A test adapter proves this without
shipping another language. Nearest tsconfig/jsconfig/package ancestor wins,
bounded by the canonical repository root. Tool lookup ascends to the workspace
root for hoisted node_modules. Project TypeScript is explicitly selected.

`transport.rs` owns bounded Content-Length framing, a writer queue and background
reader, ID correlation, bounded diagnostics/stderr and fail-all-pending on EOF
or malformed data. Request deadlines include queued writes. Timeout cancels and
retires an unhealthy process. Unsupported server-initiated requests are rejected;
workspace/applyEdit always returns applied=false. Process handles are reaped.

`session.rs` initializes with honest UTF-16 capabilities, checks server
capabilities, and synchronizes documents before requests. `documents.rs` bounds
source reads, validates canonical workspace paths, tracks versions and full text,
and replaces the entire old range for incremental-only servers. All already-open
documents refresh before requests so edits to dependencies are not stale.
Diagnostics distinguish fresh, pending and unavailable; never present missing
notifications as a successful empty diagnostic result.

`protocol.rs` contains only V1's structured error wrapper. The maintained
lsp-types crate was evaluated against existing serde/url dependencies. V1 uses
the existing JSON representation for eight bounded operations rather than adding
a protocol-wide dependency or copying the full specification. No new dependency
or async runtime was required.

`normalize.rs` validates response shapes and file URIs, filters external paths,
orders and deduplicates records deterministically, caps strings/results, and
reports totals/remaining. `tools.rs` validates strict purpose-built arguments:
1-based line and UTF-16 column; no method, command or server arguments from tools.

## Backend research (2026-09-16)

Primary sources checked live:

- https://github.com/microsoft/typescript-go (README reports native port migration
  to TypeScript, tsc name from 7 RC; its old feature table still says LSP in progress).
- https://github.com/microsoft/typescript-go/blob/main/cmd/tsgo/main.go and lsp.go:
  current native invocation is `--lsp --stdio`, not old `lsp -stdio` examples.
- https://github.com/typescript-language-server/typescript-language-server/blob/master/docs/configuration.md:
  project tsserver.path, disableAutomaticTypingAcquisition and useSyntaxServer=never.
- https://docs.rs/lsp-types/latest/lsp_types/ (maintained typed LSP library).

Selection must inspect installed packages rather than equate today's upstream
main with the user's installation. Auto prefers compatible installed project
tooling: native TypeScript where the installed native package supports LSP;
typescript-language-server with project tsserver.js for JS-based toolchains.
Explicit backend selection is supported. Auto may try a compatible alternative
after bounded initialization failure, and status reports the actual choice,
versions and fallback reason. Launch direct executables/Node package entrypoints
without shell interpolation; never run npx/downloads. Disable automatic typings
acquisition and reject edits. External processes remain advisory, not sandboxed.

## Ordered implementation and evidence gates

1. Transport tests RED/GREEN: split/coalesced frames, header/body limits, invalid
   JSON, correlation, interleaved notifications, EOF, timeout, cleanup and edits.
2. Adapter/documents/normalization tests RED/GREEN: all eight extensions, roots,
   traversal/symlinks, versions/no-op updates, UTF-16, stable compact output.
3. Manager/session tests RED/GREEN: lazy/reuse/concurrent start/different roots,
   restart bounds, capability checks, current contents and diagnostics freshness.
4. Native/settings/permission/graph tests RED/GREEN: schemas, routing, disable,
   existing policy denial, parent sharing across actual worker processes.
5. External offline fixture: definition/references/hover/document and workspace
   symbols/implementations/type definition; type error and corrected diagnostics.
   External test alone may be ignored when a server is not installed.
6. Measure no-LSP startup, first/warm requests, graph worker process count/memory.
7. Documentation covers tools, install, settings, backend/root selection,
   troubleshooting, status, verification distinction and future adapter work.
8. Fresh fmt, workspace all-target Clippy with warnings denied, coding-agent and
   agent tests. Commit only intended paths; push feature branch; open PR; fix and
   monitor relevant CI until green. No force push or merge.

No subagents per explicit user instruction. Only relevant tests plus explicitly
required final gates. Use RTK and Headroom where effective; do not claim savings
without measurements. Shared checkout changes remain untouched.
