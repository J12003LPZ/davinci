# Repository audit and improvements — 2026-09-19

Scope: all 14 active Cargo crates, their public boundaries, root build/CI
configuration, active JavaScript, migration stubs and reference layout.
Baseline: `1106b1b`; branch `leonardojeziellopez/repo-audit-01a0bc03` in the
isolated `pi-rust-9416e5cee6/01a0bc03` worktree. The shared checkout was preserved.
No commit, push or remote deployment was performed.

This was a dependency/module/caller audit with targeted adversarial tests,
not an assertion that every line or possible execution path is defect-free.
Four independent read-only investigations covered security, storage/protocol,
runtime resources, and dependencies/tooling. The main agent implemented and
integrated the changes; an independent reviewer checked the resulting patches.

## Fixed findings

Severity below reflects the inspected application path, not an assigned CVSS score.

| Finding | Severity / impact | Change and regression evidence |
| --- | --- | --- |
| Recursive grep could return credential files selected by a glob | High: disclosure despite directory-level read permission | `davinci-agent/src/tools.rs` filters both rg matches and native candidates through the existing secret-path classifier, including canonical paths. Fixtures cover `.env`, key files, nested secrets, glob variants, context lines and explicit permitted file reads. |
| Git package paths could escape the installation root | High: outside-root copy/replacement/deletion | `davinci-coding-agent/src/packages.rs` rejects raw traversal and unsafe Windows components, then checks canonical confinement before dry-run, fixture and live operations. Tests preserve an outside sentinel for malicious paths and a real linked checkout parent; normal HTTPS/SSH/SCP/ref paths retain their layout. |
| Evidence publication exposed partial blobs and trusted corrupt existing content | Medium: unreliable verification evidence | `runtime/evidence_store.rs` writes and syncs a private temporary file, publishes through the existing directory abstraction without overwriting, and validates an existing/concurrent winner. Tests cover truncated/equal-length corruption and eight concurrent writers. |
| SQLite multi-step writes could commit partial sessions, logs or branch caches | Medium: inconsistent persisted state after failure | A small nested-compatible transaction helper wraps create/import/entry/log/session/cache writes while preserving `&self` APIs. Trigger-injected failures test rollback at lane, sequence, fact and cache stages; nested error/panic tests preserve the caller's transaction. |
| In-process client Hello skipped protocol-version checks | Medium: incompatible state accepted | `davinci-client/src/session.rs` validates envelope and snapshot versions before applying either snapshot or events; rejection clears connection state. Both mismatches have regression coverage. |
| Required Context VM state could exceed its declared budget | Medium: oversized requests and misleading manifests | Compiler rejects required-state overflow and reserves the newest event before optional context. The active run loop blocks a remaining budget failure after folding, before provider dispatch; manifests mark the failure. Tests cover required pages, exact budgets, optional selection and a provider closure that must never run. |
| Provider reader could run ahead indefinitely or allocate an unbounded frame | Medium: memory exhaustion from fast or malformed streams | `davinci-ai/src/stream_reader.rs` bounds read-ahead to eight lines and each raw frame, including its delimiter, to 16 MiB. Non-SSE fallback has the same body cap. Tests cover backpressure, cancellation on receiver drop, overlong lines/multiline frames, exact LF/CRLF boundaries and invalid UTF-8. |
| Cyclic parent chains could hang session walks and exported viewers | Medium: availability on malformed history | Session parent walks track visited IDs. The JS viewer also replaces recursive tree operations with explicit stacks, guards cycles, and builds paths with push/reverse. Tests cover self/two-node cycles, missing leaves and 20,000-entry chains. |
| Process-global behavior telemetry grew without limit | Low: retention and reporting cost | A deque retains the latest 4,096 settled runs in chronological order. The existing getter type is unchanged; retention/eviction/clear tests pass. Documentation identifies reports as recent, not lifetime statistics. |

The main regressions were observed failing before their fixes. Additional
boundary and concurrency cases were added to probe the corrected paths.
Tests use synthetic data; real credentials and user sessions were not used.

The first independent review rejected the Context VM fallback at dispatch.
The revised dispatch gate, manifest failure and newest-event reservation passed
the follow-up review. Two older tests that assumed a successful one-token
compile now reserve the actual mandatory state while retaining their cache
affinity and optional-episode assertions.

## Dependency and size changes

- `url` **2.5.0 → 2.5.4**, removing `idna` 0.5 and its malformed-Punycode issue.
  A web URL validation regression exercises the affected input. See
  [RUSTSEC-2024-0421](https://rustsec.org/advisories/RUSTSEC-2024-0421.html).
- `rustls` **0.23.43 → 0.23.45**, the patched version for
  [RUSTSEC-2026-0285](https://rustsec.org/advisories/RUSTSEC-2026-0285.html).
- `idna_adapter` is locked to 1.2.0 with ICU 1.5 to preserve Rust 1.83.
  The newer transitive selection failed on that compiler; the final graph builds
  with the repository toolchain. No new production dependency was introduced.
- Removed unused workspace declarations `dirs`, `percent-encoding`, and `similar`.
  `percent-encoding` remains legitimately present transitively.
- Removed 11 undeclared AI files, **106,738 bytes / 3,026 lines**:
  `cost.rs`, `device_code.rs`, `device_oauth.rs`, `events.rs`, `http.rs`,
  `oauth_flow.rs`, `request.rs`, `sigv4.rs`, `transport.rs`, `types.rs`, `vertex.rs`.
  Module declarations, includes, callers, metadata and build/CI references were
  checked. Their obsolete internal imports were not active public exports.
  Historical plan references remain historical; active navigation is corrected.
- Removed an unused private status wrapper; its existing test now checks the
  underlying projection. Public governor artifact APIs remain intact, with
  narrowly scoped annotations for their duplicate private binary compilation.

`cargo-audit` 0.22.2 against RustSec database commit
`d5c17953a895cf19e8d3ce66eaa42b6fcfe1fb16` reported **2 vulnerabilities before,
0 after; 3 informational warnings remain**. The scanner was built in a temporary
tools directory, outside product dependencies. This is a dated advisory snapshot.

## Performance evidence

The exported viewer's actual `getPath` implementation was measured against the
baseline version in Node 24.19, using a 100,000-entry chain and five samples per
version. Median path assembly was **399.9585 ms before / 7.0475 ms after**.
The change replaces repeated array-front insertion with append plus one reversal.
This measures path assembly, not total browser rendering or application latency.

Resource bounds are structural improvements: eight queued provider lines,
16 MiB per raw frame/fallback body, 64 queued rg records and 4,096 telemetry runs.
No provider-throughput, whole-application memory, or paid-model performance claim
is made.

## Coverage and navigation

The new [architecture guide](../ARCHITECTURE.md) maps entry points, ownership,
request/tool/persistence flows, configuration and test locations. Root/crate/AI
READMEs, the legacy workspace description and installer reference text were
corrected. `vendor/davinci`, `packages/*` migration stubs and the archived
`davinci-core` were deliberately retained.

| Active crate | Coverage emphasis |
| --- | --- |
| `davinci-agent` | Permission and tool boundaries, web SSRF, task/graph dispatch callers, context/cache/evidence runtime, execution and session consumers |
| `davinci-ai` | Active module/public export graph, auth/OAuth/request/stream boundaries, transport resources and dependency upgrades |
| `davinci-coding-agent` | CLI/SDK wiring, package management, native/JS extension dispatch, learning/governor guards, export pipeline, settings and TypeSafe boundaries |
| `davinci-session` | JSONL/repository contracts, import and parent relationships, branch/context consumers |
| `davinci-session-sqlite` | Persistence, sequence/fact/lane writes, branch-cache atomicity and failure recovery |
| `davinci-client` | In-process/framed Hello, connection state and event application |
| `davinci-protocol` | Versioned contracts, serialization and framing through client/server callers and tests |
| `davinci-server` | Controller/dispatcher and transport integration through contract tests |
| `davinci-mcp` | Configuration and transport architecture, affected networking dependencies and library tests |
| `davinci-telemetry` | Producers/consumers, retention, serialization and aggregation |
| `davinci-tui` | Session tree construction, public recursive types, terminal ownership and ratatui dependency reachability |
| `davinci-voice` | Manifest, module/worker boundary and runtime integration; no audio/hardware execution |
| `davinci-evals` | Harness/fixture ownership and affected deterministic Context VM replay evaluations |
| `davinci-parity` | Fixture/reference/build wiring; no full upstream differential campaign |

Unchanged areas received architectural and caller-based coverage, not uniform
line-by-line review. Existing SSRF, task-transport, skill-ledger and output-byte
guards were traced; no new bypass was confirmed there. `graph_submit` is serial
on the inspected dispatch paths; its load/store flag would need atomic claiming
if a future path allowed concurrent submission.

## Validation

Environment: Windows, repository Rust 1.83, Node 24.19, cached locked dependencies.
Baseline workspace check and formatting passed. Baseline warnings-denied Clippy
failed on existing lint/dead-code warnings; small equivalent simplifications and
the scoped compatibility annotations address them.

| Check actually run | Result |
| --- | --- |
| `davinci-agent --lib` | 974 passed, including permission, web, evidence and turn paths |
| Context budget + harness manifest integrations | 9 passed |
| Context active/cache/replay/retrieval integrations | 14 passed |
| `davinci-ai` package suite | 185 passed; subsequently expanded reader boundary selection: 5 passed |
| Session + telemetry suites | 45 passed |
| SQLite suite | 9 passed |
| Client + server + protocol + MCP library suites | 67 passed |
| Coding-agent package-management selection | 5 passed, including the Windows directory-link fixture |
| Coding-agent exporter selection / status projection | 6 passed / 1 passed |
| TypeSafe settings/privacy/decision + repo-intelligence integrations | 19 passed |
| Context VM deterministic eval selection | 4 passed, covering seven replay scenarios |
| Exported viewer Node tests | 4 passed; added to the existing CI matrix |
| Final workspace all-target check | Passed |
| Final workspace all-target Clippy with warnings denied | Passed |
| Final workspace formatting check | Passed |
| Coding-agent release build, offline and locked | Passed |
| Installed executable SHA-256, `--version` and `--help` | Build/install hashes match; both smoke commands exited 0 |
| Documentation and packaging checks | 100 local Markdown links resolve; package JSON parses; 11 removed modules are absent and undeclared; installer shell syntax passes |
| Final diff whitespace check | Passed |
| Dependency scan | Exit 0; zero vulnerabilities, three informational warnings |

Checks were selected for changed behavior, security/data boundaries and affected
interfaces. No full-repository test sweep, paid model eval, or simulated benchmark
substitution was used. The final diff and active references were reviewed.

## Residual risks and unverified gates

- **Dependency warnings:** `paste` 1.0.15 is unmaintained (RUSTSEC-2024-0436).
  `lru` 0.12.5 has RUSTSEC-2026-0002 (`IterMut`) and RUSTSEC-2026-0253 (`pop` panic
  safety). They arrive through pinned ratatui 0.29. The inspected application
  does not call those lru APIs or ratatui's layout-cache initialization; its TUI
  renders flat rows. This reduces known reachability, not the underlying defect.
  Removal requires a separately validated ratatui upgrade or maintained patch,
  including Rust-version and public API compatibility review.
- **Deep Rust tree APIs:** nested JSON `Value` and public TUI tree nodes still
  have recursive construction/serialization/drop risks for extreme imported
  histories. Parent-path and JS-viewer fixes do not solve these APIs. A safe
  broader fix needs a fallible depth contract or a flat representation; silent
  truncation would change public behavior. Session import still accepts malformed
  parent relationships, so corrupt histories can have incomplete tree display.
- **Cancellation and total response memory:** receiver drop frees a worker blocked
  in channel send, not one waiting inside a silent socket read. Transport timeout
  still governs that worker. Decoded responses, event histories and provider raw
  items retain accepted data; frame limits are not total-response limits.
- **Filesystem concurrency:** confinement and credential-result filtering are
  application boundaries, not an OS sandbox against arbitrary hostile concurrent
  filesystem changes. rg may open credential files before the parent filters its
  matches; those contents are excluded from tool results. Native grep filters
  before opening. Approved shell/extension commands retain their normal authority.
- **Platform/live coverage:** Linux/macOS-specific syscall paths, real provider
  and OAuth sessions, interactive exported HTML rendering, microphone/voice,
  browser automation and full upstream parity were not exercised on this Windows
  run. No global coverage percentage or complete E2E certification is claimed.

## Local delivery and evidence

The normal command resolves to `C:\Users\sergi\.cargo\bin\davinci.exe`.
The successful release build replaced that executable after preserving and
hash-checking the previous copy at
`C:\Users\sergi\.cargo\bin\davinci.exe.before-repo-audit-20260919-01a0bc03.bak`.
No active davinci process was present at replacement. Command resolution was
checked again afterward; build and installed SHA-256 both equal
`9D7E8C28A2E098953C2594A9939785DFBF699C82C249CA757018AE2CB9C2B2FA`.
The exact installed executable returned `1.0.70` for `--version` and displayed
usage for `--help`; both exited 0 with offline mode enabled. These are launch
checks, not interactive UI or live-provider verification. Existing sessions must
restart to use a replaced binary. User settings, credentials and sessions were
preserved.

Temporary local evidence lives in
`C:\Users\sergi\AppData\Local\Temp\davinci-repo-audit-01a0bc03`:
`verification-results.json`, `dependency-audit.json`, `export-benchmark.cjs`,
`export-benchmark-results.json`, `template-before.js`, `installed-verification.json`,
`install-verified.ps1`, `verify-docs.cjs`, and `critique/`. These machine-local
artifacts are not committed product files.

Status: **DONE_WITH_CONCERNS**. The scoped audit, safe improvements,
documentation and proportional validation are complete. The residual risks
above remain explicit follow-up decisions; this report does not certify the
entire application as vulnerability-free.
