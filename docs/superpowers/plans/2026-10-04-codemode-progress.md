# Codemode execution record

Plan: [2026-10-04-codemode.md](2026-10-04-codemode.md)

## Baseline and scope

- Source baseline: `08bf0038a40c469e57a5f12c9d753e580b198e0b` (current `main`, verified 2026-10-04)
- Dedicated branch: `feat/codemode-sandbox`; all repository work uses the GitHub plugin and GitHub Actions. No local checkout or local execution is used
- PRs #89 and #90 are merged into this baseline. Concurrent #91 (`remove-typesafe-jev`) is not merged and is not modified by this work
- Rust remains 1.83.0 with current exact Rust dependency pins. Core defaults, subscription authentication, selected model/effort/tier, and installed binaries remain unchanged
- Only fixture-based execution is authorized. No live provider/subscription probes, merge, release, credentials, or installed-binary changes
- No nested AGENTS.md was found outside the vendored tree; root AGENTS.md applies

## Execution status

| Task | Status | Evidence / gate |
| --- | --- | --- |
| C00 baseline and fixtures | In progress | Source seams inspected; focused baseline CI and new fixtures pending |
| C01 dependency admission | Not started | Published artifacts, exact maintained Node runtime, complete lock/integrity, advisories, and platform smoke tests required |
| C02 bounded host protocol | Not started | C01 admission required |
| C03 structured MCP | Not started | C00 fixture boundary required |
| C04 guarded child dispatch | Not started | C02 contracts and C03 required |
| C05 safe script projection | Not started | C03/C04 required |
| C06 opt-in read-only slice | Not started | C02/C04/C05 required |
| C07 controlled execution | Not started | C06 required |
| C08 recovery | Not started | C07 required; controlled mode must remain unavailable until this gate |
| C09 supported surfaces | Not started | C06/C08 required |
| C10 accounting and evidence | Not started | C05/C08/C09 required |
| C11 comparative fixtures | Not started | C10 required; live campaign remains unrun |
| C12 platform/release gates | Not started | All earlier gates and independent branch review required |

## Preflight interfaces and rulings

- C01 -> C02: the upstream package buffers output before resolving and exposes no configurable collector byte budget in its public options. Admission must characterize this before production integration; final output truncation alone cannot prove a bounded host
- C02 -> C04: `CodeModeRunContext` is host-owned and must never be deserialized from model/IPC input. The broker identity and wire correlations remain separate
- C03 -> C05: MCP structured data must use an internal host-owned channel; `ToolResult.details` is model/UI-visible and includes reserved host bookkeeping
- C04 -> C07/C08: reuse journal-before-effect ordering and current permission rechecks. No parent lock may be held while awaiting children; no script replay is permitted
- C05 -> C10: reserve host-authored failure/recovery facts before output truncation and the final Governor pass
- Ruling: use a dedicated remote branch and GitHub Actions instead of a local worktree/test runner because the user explicitly requested GitHub-plugin-only execution. Exact-head CI replaces local test evidence, without claiming local or installed-binary validation
- Ruling: retain this execution record next to the approved plan to preserve progress across remote commits. No alternate roadmap or product scope is introduced

## Actual verification

Source inspection only so far. No test completion, package admission, platform support, performance improvement, or release readiness is claimed.

## C00 first CI observations

- Head `c64e9e15a6f7656f3130846584909a680a62b4cf`: Rust formatting identified two fixture-only line-wrap differences; applied exactly as rustfmt reported
- The added unrestricted `cargo fetch --locked` step fails before tests on an existing optional dependency, `toml_parser 1.1.3+spec-1.1.0`, whose edition-2024 manifest Cargo 1.83 cannot parse. The product uses pinned `toml_edit 0.22.27`; the incompatible parser belongs to another already-locked graph
- Ruling: prepare only the actual tested package/target graph with `cargo test --no-run --locked`, then run the planned `--offline --locked` tests. Do not change Rust pins or lockfile to make an unrelated all-features/all-targets fetch succeed
- Package characterization job is gated on successful C00 baselines across the three native operating systems. Proposed Node identity is 24.21.0, listed as the current maintained LTS on https://nodejs.org/en/about/previous-releases at inspection. No package has been admitted yet

## C00 verified baseline / C03 test-first

- Linux passed 110 focused tests on PR head `817f39ff182a0866806f349242926196cb8f713e` (PR test-merge `6291326`): batch 4, tool ledger 20, MCP 38, new Codemode baseline 3, operation dispatch 11, Governor 34. [Job evidence](https://github.com/J12003LPZ/davinci/actions/runs/37221623922/job/111493006804)
- Format and full-workspace clippy passed at that head. Windows/macOS validation remains ongoing, not assumed
- Added an executable counted-effect/delayed/lost-response MCP transport fixture to finish the reusable C00 corpus; it invokes the actual MCP client and checks that a lost response does not automatically repeat the effect
- C03 may proceed independently of C01/C02, as the approved plan explicitly permits. Added test-first retention cases against the existing decoder/client; production changes follow only after the expected loss-of-structured-data failures are observed

## C03 production wire retention

- Red evidence: [structured MCP regression job](https://github.com/J12003LPZ/davinci/actions/runs/37222884787/job/111496669159) at head `ff326c1283197ede2328110434abe24602261713`: 9 tests ran, 5 failed for missing structured/output-schema fields and 4 passed. These are expected task C03 failures, not a passing suite
- Implemented optional typed `structured_content` / `output_schema` retention at the wire source. `CallToolResult::text()` is unchanged; private `_meta` and forged bookkeeping remain unmodeled
- Repository-wide MCP type-reference search found no other Rust struct literals requiring new fields. Full CI remains the construction/compatibility check
- Internal full-result accessor tests are committed first and intentionally await the new accessor; C03 is not complete until those tests and supervised-delivery characterization pass
- Ruling disclosed before applying: focused C00 is repeated on Linux to avoid three duplicate full Rust compiles on every test-first edit. All original required CI jobs remain unchanged; package admission and final native Windows/macOS/Linux gates remain required. This changes validation scheduling, not supported platforms or feature scope
