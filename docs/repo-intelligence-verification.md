# Repository intelligence verification

Local validation: Windows x86_64 MSVC, repository-pinned Rust 1.83, 2026-09-16.
Implementation baseline: `a50fdc6`. No subagents or live model calls were used.

## Local gates

| Gate | Result |
| --- | --- |
| `cargo fmt --check` | Passed |
| `cargo clippy --workspace --all-targets --offline --locked -- -D warnings` | Passed |
| `cargo test -p davinci-coding-agent --offline --locked` | Passed; RTK reports 2,155 passed, seven ignored across nine suites |
| `cargo test -p davinci-agent --offline --locked` | Passed; RTK reports 869 passed across seven suites |
| Explicit ignored repository evaluation | Passed, all three scenarios |
| `git diff --check` | Passed |

Focused regression runs established failures before implementing parser/query/native
integration, schema validation, CommonJS/default exports, shadowed calls, monorepo
aliases, filename routing, impact results, text truncation, and named reexport calls.
Existing native registry, Graph role/recovery, and package security tests also ran.
The final package run includes the new process-sharing tests and native retention/
disabled-settings integration. Unix symlink tests are platform-gated and require CI.

## Measurements

These are fresh test/debug measurements on a synthetic 200-source-file fixture
(40,883 fixture bytes including config). They are reproducible using
`cargo test -p davinci-coding-agent --offline --locked --test repo_intelligence_eval -- --ignored --nocapture`.

| Metric | Measured |
| --- | ---: |
| Disabled controller construction plus status, mean of 100 | 0.066139 ms |
| Enabled but unused construction plus status, mean of 100 | 0.074238 ms |
| Cold indexing | 42.5134 ms |
| Warm persistent loading and hash validation | 26.3725 ms |
| Single-file edit refresh | 26.4692 ms |
| Symbol search | 24.1548 ms |
| Related-file query | 24.8154 ms |
| Unified code query | 24.2653 ms |
| Persistent cache | 165,646 bytes |
| Process peak working set | 15,691,776 bytes |
| Cold / warm / edited files reparsed | 200 / 0 / 1 |
| Warm source bytes hash-validated | 40,847 bytes |

Startup measurement isolates the newly added controller path, not whole-CLI launch.
Peak memory includes the test process. Source hashing still reads files on warm
queries; the index saves parsing and model context rather than eliminating I/O.

| Scenario | Calls, baseline → index | Returned bytes, baseline → index | Estimated tokens, baseline → index | Latency ms, baseline → index |
| --- | --- | --- | --- | --- |
| A: AuthService definition and consumers | 3 → 1 | 12,954 → 1,796 | 3,239 → 449 | 19.3368 → 22.0111 |
| B: session validation neighborhood | 6 → 1 | 25,978 → 3,107 | 6,495 → 777 | 20.1481 → 22.2194 |
| C: API router orientation | 4 → 3 | 19,309 → 1,594 | 4,828 → 399 | 18.8188 → 68.9008 |

All expected definition/importer/test/config/router evidence was present. Baselines
execute the existing grep/read tools; they explicitly read 12,762 / 25,605 / 19,140
source bytes, respectively. Indexed workflows require zero explicit `read` calls.
Grep's disk bytes are not instrumented. Estimated tokens use UTF-8 bytes divided
by four, rounded up. Whole-file fixture reads include unrelated implementation
comments. This demonstrates context reduction for these workflows, not a universal
latency improvement or production-repository benchmark.

## Acceptance audit

Numbers correspond to the supplied plan's 36 criteria.

| Criteria | Implementation and evidence |
| --- | --- |
| 1–2 native subsystem, eight extensions | `repo_intelligence` module; grammar/extension fixtures |
| 3 lazy | Constructor/status assertions verify no cache creation or parsing |
| 4 persistent | Warm manager reload and corruption fixtures |
| 5, 29 incremental | Hash-based edits/additions/deletions/rename fixture; evaluation parses only one edited file |
| 6 shared Graph index | Shared controller registry; four subprocesses together parse exactly 40 fixture files once |
| 7 stable identity | IDs survive inserted lines/comments |
| 8–10 exports and structure | Parser and named/default/barrel/relationship fixtures |
| 11–17 seven tools | Native registration/execution and query fixtures |
| 18 deterministic routing | Direct Rust routing; filename, symbol, impact, and quoted-text cases |
| 19 semantic provider | Injected provider returns labeled evidence; outside-root results rejected; no LSP host exists at baseline |
| 20 text fallback | Unsupported docs and bounded literal-search tests |
| 21–22 provenance | AST/LSP/text/config labels; structural evidence never promoted to semantic references |
| 23 bounded outputs | Runtime schema limits, bounded scans/records/cache, query clipping and partial flags |
| 24 retention | Native output compressed by existing governor and recovered byte-for-byte |
| 25 corruption | Invalid cache JSON rebuilt successfully |
| 26 malformed source | Error-tolerant parser test and partial parse status |
| 27 boundaries | Traversal/protected/generated/ignore tests; shared confined-open primitive; Unix links gated to CI |
| 28 startup | Unused initialization performs no scan; isolated constructor measurements above |
| 30 context reduction | Measured A/B/C scenarios above, correctness assertions passed |
| 31 capabilities | Native RuntimeCapabilityRegistry tests and Graph role allowlists/recovery tests |
| 32–34 compatibility, tests, clippy | Required package and workspace gates above |
| 35 CI | [PR #7 checks](https://github.com/J12003LPZ/davinci/pull/7/checks) track the current head; completion requires green CI |
| 36 future adapters | Language-neutral normalized records and semantic provider contract; no Rust parsing added |

Documentation review checked tool/settings fields against schemas, source and
regression tests; ran the evaluation command; and fetched the official parser and
grammar reference pages and checked their titles. Native status/settings behavior
was exercised in tests. No paid evaluation or live LSP session was run.

Known limits are documented in [the user guide](repo-intelligence.md), including
conservative module resolution, dynamic JavaScript, hash I/O, and synthetic timing.
The PR records current CI results, including Linux package tests and Unix boundary
coverage. Use checks for the latest head rather than assuming an earlier green run
covers later edits. A follow-up regression also verifies normalized `./file.ts`
queries; traversal remains rejected.
