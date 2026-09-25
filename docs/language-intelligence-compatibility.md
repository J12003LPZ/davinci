# Language Intelligence Compatibility Report

**Implementation branch:** `codex/2026-09-25-rust-python-lsp`

This report records what the Rust/Python language-intelligence implementation
contains and what has actually been executed. It deliberately does not turn a
defined, ignored, skipped, or lockfile-blocked test into interoperability proof.

## Scope implemented

The branch contains one canonical language-intelligence owner for
TypeScript/JavaScript, Rust, and Python, with:

- the existing eight `lsp_*` tools;
- language-aware project resolution and session identity;
- rust-analyzer and BasedPyright/Pyright adapters;
- scoped LSP client callbacks and dynamic document diagnostics;
- bounded synchronization, cancellation, restart and session admission;
- core `SemanticService` compatibility through the shared owner;
- CLI/native shared ownership and opt-in SDK attachment/detachment;
- parent-backed graph semantic forwarding with role authority;
- explicit Rust/Python semantic anchors for `code_query`;
- permission-bound retained LSP output and `retrieve_output`;
- per-language status, provenance, freshness and partial-coverage metadata;
- a three-platform deterministic workflow and separately provisioned live lane;
- real-CLI evaluation and reproducible startup/polyglot benchmark tooling.

The implementation boundary remains read-only semantic queries. Rename/apply,
arbitrary code actions, formatting, completion UI, call hierarchy, remote LSP,
Ruff aggregation, and Rust/Python structural dependency indexing are not
advertised as complete.

## Backend matrix

| Backend | Fixture coverage in source | Real-server test in source | Executed in this implementation session |
| --- | --- | --- | --- |
| TypeScript / `typescript-language-server` | Yes | `real_typescript_semantics_and_edit_synchronization` | No |
| rust-analyzer | Yes | `real_rust_lsp_semantics` | No |
| BasedPyright | Yes | `real_basedpyright_lsp_semantics` | No |
| Pyright | Yes | `real_pyright_lsp_semantics` | No |

The manual live workflow provisions rust-analyzer `2026-09-21`,
BasedPyright `1.40.1`, Pyright `1.1.414`, TypeScript `5.9.3`, and
`typescript-language-server` `5.3.0`. It records the selected runtime
versions before running the ignored tests.

## Deterministic acceptance coverage

The deterministic fixture and public tests cover, among other cases:

- all eight tool names remain stable;
- TS/JS, Rust and Python route to distinct family/profile sessions;
- same-root polyglot identity cannot collide;
- core/native coordinate conversion and shared ownership;
- separate worktrees get separate session identities;
- static Cargo/Python root discovery without marker execution;
- scoped configuration callbacks and dynamic diagnostic registration;
- cancellation and late-reply correlation;
- stale/versioned/unversioned diagnostic handling;
- partial notification delivery becoming `resync_required`;
- source bytes changing during a pending query becoming `stale_result`;
- per-family session cap eviction;
- project-local Python interpreter selection requiring project trust;
- Rust semantic `code_query` anchors bypassing the TS/JS structural index;
- retained LSP evidence being denied after permission revision;
- governor reset revoking stored output IDs;
- LSP overflow reaching the same host governor used by `retrieve_output`;
- Windows job-object and Unix process-group cleanup paths in the LSP transport;
- Linux process-tree RSS reporting, with `null` on unsupported platforms.

Public boundary tests are in:

- `crates/davinci-coding-agent/tests/lsp_surface_parity.rs`
- `crates/davinci-coding-agent/tests/lsp_graph_isolation.rs`
- `crates/davinci-coding-agent/tests/transaction_intelligence.rs`

## Side-effect sentinels

### Rust

The real rust-analyzer fixture contains:

- a build script that writes `build-script-ran` if executed;
- a local proc macro that writes `proc-macro-ran` if executed;
- a fixed `Cargo.lock`;
- an assertion that the project-local `target/` directory remains absent.

The navigation profile sends build scripts/proc macros/checks disabled,
`--locked`, `--offline`, `CARGO_NET_OFFLINE=true`,
`RUSTUP_AUTO_INSTALL=0`, and an analysis target directory outside the
checkout. The live test fails if the build/proc-macro sentinels run or if the
fixture lockfile/project target is modified.

Project trust is still required. This profile is not described as an OS sandbox:
Cargo/toolchain/project configuration remains project-controlled input.

### Python

A deterministic test proves an untrusted project `.venv` cannot reach server
launch or interpreter selection. A live BasedPyright test deliberately creates a
project virtual environment with both `.pth` and `sitecustomize.py`
sentinels and verifies they execute only after the trusted semantic launch path
selects that interpreter.

BasedPyright receives `baselineMode=discard`. The real fixture hashes the
documented default baseline path `.basedpyright/baseline.json` before and
after semantic edits.

## Process and resource behavior

Language servers are lazy. Failed discovery is cached separately and does not
occupy a live slot. The default global cap is eight with family caps of Rust 2,
Python 4 and TypeScript 4.

The LSP transport uses:

- a Windows job object with kill-on-close for the server tree;
- a Unix process group with TERM then bounded KILL escalation;
- stdin EOF as an additional shutdown signal.

Session status includes the server PID and, on Linux, bounded process-tree RSS
summed from `/proc`; unsupported platforms report `rssBytes: null`.

## Retained evidence authorization

A retained LSP result is associated with:

- canonical workspace identity;
- the permission-state scope identity;
- the observed permission revision;
- source/returned paths that were retained.

`retrieve_output` and governor artifact retrieval re-check the current
permission state and path authority. A permission revision denies old semantic
evidence. `/governor-reset` also deletes the current session's stored output
directory, so a previously known output ID cannot bypass the cleared manifest.

## SDK contract

The SDK remains opt-in:

```rust
session.attach_language_intelligence(config, &tools)?;
session.attach_language_output_store(governor)?; // optional
// ...
session.detach_language_intelligence()?;
```

Attachment requires the caller-supplied runtime, honors the session's original
tool exclusions, uses atomic owned capability registration, delegates unrelated
custom tools to any previous executor, and restores prior handlers on detach.
The optional output-store attachment owns `retrieve_output`; without it the SDK
uses bounded summaries only.

## Workflow and measurement artifacts

`.github/workflows/language-intelligence.yml` contains:

1. a deterministic Linux/Windows/macOS matrix;
2. a manual live-server matrix with pinned external backends;
3. the real TypeScript, Rust, BasedPyright and Pyright tests;
4. the project-interpreter trust sentinel;
5. a production CLI build and `scripts/eval-native-intelligence.py --lsp`;
6. 20-sample startup and polyglot benchmark runs;
7. uploaded JSON benchmark artifacts per operating system.

The benchmark command is also available locally:

```text
python scripts/lsp-benchmark.py --binary target/debug/davinci --scenario startup --repetitions 20 --output lsp-startup.json
python scripts/lsp-benchmark.py --binary target/debug/davinci --scenario polyglot --repetitions 20 --output lsp-polyglot.json
```

Unavailable RSS or child-count measurements are serialized as `null`, not
zero.

## Verification status for this implementation session

Integration into PR #49 (2026-09-25) keeps the Rust 1.83 lockfile decision
(`toml_edit 0.22.27`, `indexmap 2.7.1`), so Cargo 1.83 compiles the workspace.

Local evidence, Windows 11 Pro, Rust 1.83.0, Node v24.19.0:

- `cargo check --workspace --all-targets --offline --locked`: pass.
- `cargo fmt --all --check`: pass.
- `cargo clippy --workspace --all-targets --offline --locked -- -D warnings`: pass.
- `cargo test --workspace --offline --locked`: 4501 passed, 0 failed,
  37 ignored before the rust-analyzer readiness gate was added; the focused
  language-intelligence suites pass after it.
- `real_rust_lsp_semantics` with rust-analyzer 1.98.0 (88d9e12a 2026-08-18):
  **pass** (about 18 s). The first run returned an empty definition because the
  query ran before rust-analyzer finished loading the workspace. The client now
  advertises `experimental.serverStatusNotification`, waits for `quiescent`
  within the request budget, and marks results obtained earlier
  `analysisState: "indexing"` / `workspaceCoverage: "partial"`.
- `real_basedpyright_lsp_semantics`, `real_pyright_lsp_semantics` and
  `real_python_project_venv_executes_only_after_trusted_launch`: **unverified**
  (servers not installed on the verification host).
- `real_typescript_semantics_and_edit_synchronization`: not run in this pass.
- Task 19 benchmark (`scripts/lsp-benchmark.py`): not run in this pass; no
  latency, RSS or process numbers are claimed.

Linux and macOS, and every backend marked unverified above, still require the
manual **Language intelligence** workflow before they are advertised.

## Release-readiness handoff

The implementation and evidence harnesses are present. Before advertising a
specific backend/platform as verified, run the manual **Language intelligence**
workflow and retain its per-platform results and benchmark artifacts. A skipped,
ignored, provisioning-failed, or lockfile-blocked job remains unverified.

Do not weaken the following release invariants to make a lane green:

- no implicit language-server/package/toolchain installation by DaVinci;
- no shell fallback for server launch discovery;
- no cross-worktree or indirect graph authority escape;
- no stale/unversioned diagnostics represented as compiler-clean proof;
- no retained semantic output surviving permission revocation or governor reset;
- no project build script/proc macro execution in the restricted Rust profile;
- no silent Python interpreter substitution.
