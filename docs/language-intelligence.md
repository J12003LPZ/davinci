# Native Language Intelligence

DaVinci exposes one read-only semantic subsystem for **TypeScript/JavaScript,
Rust, and Python**. The native eight-tool surface, core `code_*` compatibility
facade, CLI, opt-in SDK attachment, and graph-worker forwarding all converge on
the same `LanguageIntelligence` owner instead of maintaining independent
language-server registries.

## Supported languages and backends

| Family | Source files | Semantic backend |
| --- | --- | --- |
| TypeScript/JavaScript | `.ts`, `.tsx`, `.mts`, `.cts`, `.js`, `.jsx`, `.mjs`, `.cjs` | existing native TypeScript or `typescript-language-server` behavior |
| Rust | `.rs` | `rust-analyzer` |
| Python | `.py`, `.pyi` | BasedPyright by default; Pyright explicitly supported |

DaVinci does **not** install a language server, Rust component, Python
environment, package, or interpreter. Discovery and `/lsp-status` are
data-only and never run `--version`, Cargo, Python, npm, pip, rustup, or a
server merely to improve status.

The manually provisioned compatibility workflow uses these reproducible
candidate versions:

- rust-analyzer release `2026-09-21`
- BasedPyright `1.40.1`
- Pyright `1.1.414`

Those pins describe the live test lane, not a claim that every developer has
those versions installed.

## The eight semantic tools

| Tool | Inputs | Result |
| --- | --- | --- |
| `lsp_definition` | path, line, column | definition locations |
| `lsp_references` | path, line, column, optional `includeDeclaration` | reference locations |
| `lsp_hover` | path, line, column | type/signature and short documentation |
| `lsp_document_symbols` | path | hierarchical document symbols |
| `lsp_workspace_symbols` | query, optional path/language | project symbols |
| `lsp_implementations` | path, line, column | implementation locations |
| `lsp_type_definition` | path, line, column | type-declaration locations |
| `lsp_diagnostics` | path, optional severity | document diagnostics |

Public `lsp_*` positions use **1-based lines and 1-based UTF-16 columns**.
The host-neutral `SemanticService` DTOs remain zero-based; the compatibility
facade converts coordinates exactly once.

Source-based calls infer the family from the validated extension.
`lsp_workspace_symbols` additionally accepts `language` values
`typescript`, `javascript`, `rust`, or `python`. A source path and an
explicit conflicting language are rejected. Legacy pathless/no-language
workspace-symbol calls retain the TypeScript-family route. Ambiguous nested
Rust/Python workspaces require a source path or explicit trusted project root.

Returned locations are normalized to workspace-relative paths and checked
against the active workspace/read policy. External Cargo registry, standard
library, virtual-environment, typeshed, and other out-of-checkout source
locations are omitted rather than silently granting new read authority.

## Project resolution

Project discovery is static and bounded.

**Rust:** DaVinci parses ancestor `Cargo.toml` files with the pinned TOML
parser. It recognizes package roots, virtual workspaces, explicit
`package.workspace`, members/exclusions, nested workspaces, and configured
project roots without executing `cargo metadata`. A standalone `.rs` file
without a supported project is reported honestly rather than being wrapped in a
fabricated Cargo project.

**Python:** the nearest valid `pyrightconfig.json` or `pyproject.toml` is
preferred, followed by bounded packaging/test markers such as `setup.cfg`,
`setup.py`, and `requirements.txt`. `setup.py` is only a marker and is
never executed by discovery. A virtual environment selects an analysis
environment; it does not merge otherwise independent projects.

Manifest reads, directory enumeration, executable discovery, and analysis
environment resolution run through the bounded metadata reader. Traversal,
symlink/junction escape, oversized metadata, unsupported schemes, and expired
request budgets are structured failures.

## Rust navigation profile

Rust uses a restricted navigation profile. The client sends settings that
disable:

- Cargo build scripts
- proc macros
- automatic Cargo checks/check-on-save
- automatic rust-src discovery/installation

The profile also sets Cargo offline/locked arguments, `RUSTUP_AUTO_INSTALL=0`,
and an analysis target directory outside the user's project. A verified
configured `sysroot`/`sysrootSrc` is used when present; otherwise standard
library navigation is reported as partial.

This is an application policy, **not an OS sandbox**. rust-analyzer still needs
project trust because workspace loading can invoke Cargo/rustc, and project
Cargo/toolchain configuration can name project-controlled programs. Build
scripts and proc macros being disabled does not mean “no project code can ever
run.”

## Python interpreter and baseline policy

Python server runtime and analysis interpreter are separate identities.

Selection prefers a configured interpreter, then supported project
`.venv`/`venv` candidates, then the explicitly inherited environment, and
finally an installed system interpreter. A configured missing interpreter is an
error; DaVinci does not silently substitute another environment.

A project-local interpreter is project code for trust purposes: starting a type
checker may execute that interpreter to discover `sys.path`, including
`.pth` or `sitecustomize` behavior. The selected interpreter therefore
participates in the launch/profile identity and trust boundary.

BasedPyright receives `diagnosticMode=openFilesOnly` by default and
`baselineMode=discard`; Pyright does not receive the BasedPyright-only
baseline setting. DaVinci never creates or updates Python environments.

## Settings

The trusted `languageIntelligence` object supports independent profiles:

```json
{
  "languageIntelligence": {
    "enabled": true,
    "maxSessions": 8,
    "idleTimeoutMs": 300000,
    "typescript": {
      "enabled": true,
      "backend": "auto",
      "requestTimeoutMs": 5000,
      "initializationTimeoutMs": 30000,
      "coldRequestTimeoutMs": 60000,
      "maxSessions": 4
    },
    "rust": {
      "enabled": true,
      "backend": "rustAnalyzer",
      "profile": "navigation",
      "requestTimeoutMs": 5000,
      "initializationTimeoutMs": 30000,
      "coldRequestTimeoutMs": 60000,
      "maxSessions": 2,
      "features": []
    },
    "python": {
      "enabled": true,
      "backend": "auto",
      "diagnosticMode": "openFilesOnly",
      "requestTimeoutMs": 5000,
      "initializationTimeoutMs": 30000,
      "coldRequestTimeoutMs": 60000,
      "maxSessions": 4
    }
  }
}
```

Python backend values are `auto`, `basedpyright`, and `pyright`.
TypeScript preserves `auto`, `typescriptNative`, and
`typescriptLanguageServer`.

Profiles may use a trusted explicit server override:

```json
{
  "server": {
    "program": "/absolute/path/to/server",
    "args": ["--stdio"]
  }
}
```

Rust also accepts configured project roots, `toolchainDir`, `sysroot`,
`sysrootSrc`, target, and bounded feature names. Python accepts configured
project roots and `interpreter`.

A malformed language section disables that language and reports its profile
error without discarding valid sibling profiles or unrelated settings.

## Process ownership, budgets, and cancellation

Session identity includes canonical workspace, resolved project, language
family, and profile fingerprint. Rust/Python/TypeScript files in the same
directory therefore cannot collide in one session.

The default global live-session cap is eight, with family defaults Rust 2,
Python 4, and TypeScript 4. Failed discovery is held in a short bounded negative
cache and does not consume a live server slot. Idle sessions are reclaimable;
busy sessions are not evicted.

One absolute request budget covers discovery, admission, initialization,
document synchronization, server callbacks, and the query. Cancellation sends
`$/cancelRequest` for a dispatched request and discards late replies by
request/server generation; a normal request timeout does not automatically kill
an otherwise healthy server.

The transport handles bounded server-to-client requests including scoped
`workspace/configuration`, authorized workspace folders, supported dynamic
diagnostic registration, diagnostic refresh, progress, and log/show messages.
`workspace/applyEdit` is rejected and arbitrary execute-command behavior is
not exposed.

## Synchronization and diagnostic freshness

The active checkout on disk is the source of truth. Open documents are refreshed
before semantic requests. Local versions advance only after delivery is known;
an uncertain write marks the session for resynchronization instead of guessing
remote state.

Diagnostics are tracked by server generation, provider, URI, and source
version/result identity. Results distinguish:

- `pull-response`
- `versioned-publication`
- `unversioned-publication`
- `diagnostics_pending`
- `stale_result`

An empty diagnostic list is still advisory evidence. It is never represented as
a compiler/test verification receipt, and a quiet interval does not prove
workspace-wide completeness.

## Shared core, CLI, SDK, and graph ownership

The CLI constructs one canonical manager and injects a
`SemanticServiceFacade` into core semantic tools. Native LSP dispatch clones the
language handle before blocking so unrelated host/status operations are not held
behind the global extension-host mutex.

The SDK remains opt-in:

```rust
session.attach_language_intelligence(config, &tools)?;
session.attach_language_output_store(governor)?; // optional
// ...
session.detach_language_intelligence()?;
```

Attachment requires the session's existing runtime, preserves original tool
exclusions, registers owned capabilities atomically, delegates unrelated custom
tools to any prior executor, and restores prior handlers on detach. The optional
output-store attachment enables lossless `retrieve_output`; without it the
embedding can operate with bounded semantic summaries.

Graph workers never start a fallback worker-local semantic server. Direct and
nested semantic calls use parent authority, parent workspace binding, role
allowlists, and the remaining caller budget. Current semantic role policy is:

- researcher/planner/writer: all eight
- reviewer: references, implementations, diagnostics
- test analyzer: diagnostics
- classifier/historian: no direct LSP tools

An outer `code_query` permission does not grant an otherwise forbidden nested
LSP operation.

## Output retention and authorization

Semantic lists are capped before model delivery. When lossless overflow is
retained, the normalized full result receives a governor output ID. New retained
LSP records bind the workspace, permission-scope identity, permission revision,
and observed source/target paths. Both text and artifact retrieval re-check the
current permission generation and path policy; a policy revision invalidates
old retained LSP evidence.

This does not retroactively erase a result already emitted into conversation
history. It controls subsequent host retrieval.

## Status and errors

`/lsp-status` is observational: it reports configured profiles, discovery/live
session state, selected project/environment, generations, progress,
limitations, freshness, and bounded last errors without launching a server.

Common structured errors include:

| Error | Meaning |
| --- | --- |
| `server_not_installed` | no eligible installed server |
| `server_launch_denied` | trust/exact launch authority missing |
| `interpreter_not_found` | requested Python interpreter unavailable |
| `project_not_found` / `project_root_required` | project cannot be selected safely |
| `project_resolution_incomplete` | bounded static resolution cannot establish ownership |
| `rust_src_not_installed` | standard-library source coverage is partial |
| `unsupported_method` | selected server does not advertise the operation |
| `diagnostics_pending` | current diagnostic evidence has not arrived |
| `stale_result` | source/profile changed during analysis |
| `request_timeout` | absolute request budget expired |
| `session_limit` | all eligible session slots are busy |
| `source_too_large` / `document_limit` | declared resource bound exceeded |

Ordinary read/search/compiler/test workflows remain available when semantic
analysis is unavailable.

## Scope boundary

This release does not claim automatic rename/apply, arbitrary code actions,
formatting, completion UI, call-hierarchy expansion, Ruff aggregation, notebooks,
remote LSP, or a Rust/Python structural dependency index. Rust/Python semantic
navigation does not make the existing AST/test-impact graph complete for those
languages.

Compiler, type checker, lint, build, and tests remain separate verification.

## Verification

Deterministic protocol/resource tests use the local Node fixture and make no
model calls. The dedicated workflow is
`.github/workflows/language-intelligence.yml`; its required-style deterministic
matrix runs on Linux, Windows, and macOS. Its live lane is manual so provisioning
external language servers is deliberate and visible.

The explicit live tests are:

```text
cargo test -p davinci-coding-agent --locked --lib real_typescript_semantics_and_edit_synchronization -- --ignored --nocapture
cargo test -p davinci-coding-agent --locked --lib real_rust_lsp_semantics -- --ignored --nocapture
cargo test -p davinci-coding-agent --locked --lib real_basedpyright_lsp_semantics -- --ignored --nocapture
cargo test -p davinci-coding-agent --locked --lib real_pyright_lsp_semantics -- --ignored --nocapture
```

The Rust test asserts repository-local targets and checks that analysis leaves
the fixture `Cargo.lock` and project `target/` untouched. The Python tests
exercise definitions/navigation, an introduced/fixed type error, and
BasedPyright baseline preservation. An ignored test existing in source is not
evidence of interoperability until it is explicitly executed.

Public boundary tests live in `tests/lsp_surface_parity.rs`,
`tests/lsp_graph_isolation.rs`, and `tests/transaction_intelligence.rs`.

For real executable evaluation:

```text
python scripts/eval-native-intelligence.py target/debug/davinci --lsp
```

For reproducible timing evidence:

```text
python scripts/lsp-benchmark.py --binary target/debug/davinci --scenario startup --repetitions 20 --output /tmp/davinci-lsp-startup.json
python scripts/lsp-benchmark.py --binary target/debug/davinci --scenario polyglot --repetitions 20 --output /tmp/davinci-lsp-polyglot.json
```

On Windows, use the selected Python executable, `target/debug/davinci.exe`,
and a writable Windows output path. The benchmark reports unavailable resource
measurements as `null`, never zero.
