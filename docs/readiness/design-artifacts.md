# Design artifacts readiness

Status: **incomplete; draft review, not release certification**. This record distinguishes implementation from verified behavior. The original plan's unchecked boxes remain historical acceptance criteria. The feature defaults off.

The implementation was made in an isolated worktree from the user's current local source snapshot. Snapshot commit `d9725d0c` preserves 88 inherited paths, originally based on remote main `f24ed1c744a31ee49c95802f982740fe59d2de6d`. Before delivery, the feature was integrated onto main `a669549c`: 66 inherited paths were already identical upstream, 20 received the upstream follow-up fixes, and two local planning documents were retained in a separate commit. The original snapshot and feature remain recoverable on `backup/design-artifacts-before-main-01a0ff86`. The original source-only checkout was not modified. No subagents, live model requests, global installation or deployment were used.

## Scope against the plan

| Tasks | Implemented surface | Evidence and remaining limit |
| --- | --- | --- |
| 1–3 | Baseline, typed Rust/TypeScript contracts, session-owned revisions and immutable blobs | Contract generation, store, ownership, CAS, restart, branch retention and quota tests. No derived index is authoritative. |
| 4–6 | CLI/interactive/SDK/RPC admission, paired loopback host and bounded virtual compiler | Disabled-mode tool/prompt parity, one-shot permission, host protocol and compiler tests. Real supervised compiler/host passed on the earlier runtime; the refreshed bundle is separately inventoried. |
| 7–8 | Required native confinement, captures, prototype actions and separate quality states | Browser policy/transport unit fixtures pass. **Generated-content native capture is blocked on this Windows host before child start.** Fake geometry is not native evidence. |
| 9–11 | React companion, source-backed edits, comments/history, Taste profiles and static repository sync | Real Chromium companion E2E uses a recorded Rust dispatcher. Profile provenance and source edits are tested; no end-to-end generated design screenshot or human visual acceptance is claimed. |
| 12 | Subscription-only generation, durable checkpoints, exact route and root limits, bounded repairs | Recorded-model tests cover distinct concepts, repair bounds, unknown outcomes and resume. Offline dispatch and API-key/route changes are rejected. No live subscription generation or visual-quality score was measured. |
| 13 | Source, HTML and PNG export with rights and fingerprint checks | Exact source/no-overwrite/export validation and existing HTML template tests. Successful native PNG and HTML target behavior are not certified. |
| 14 | Accepted revision to native plan, isolated worktree, exact patch transaction and target checks | Tests cover acceptance, dirty/stale targets, authority, plan mode, hashes, accounted drafting and unrelated edits. Reviewed commands run through existing execution owners with actual Node TAP discovery. A real Windows managed-browser flow retained its screenshot after close. Implementation remains pending: these receipts do not certify mock removal, production APIs, RSC/client boundaries or transitive source coverage. |
| 15 | Recovery, owner checks, cancellation, leases and bounded stores/bridge | Torn-tail preservation, terminated corruption, writer contention, all-branch retention, missing blobs, revocation and expiry are covered. In addition to injected interruptions, OS termination at six source/manifest/event boundaries passed reopen and exact retry. Windows target server shutdown passed; **generated-content cross-platform process-tree coverage remains open.** |
| 16 | Fourteen-brief offline evaluation manifest, record validation, setup/rollback docs and scoped CI | The live protocol retains blind order, ties, defects and unknowns. No paid trial, human preference campaign, installed-binary certification or cross-platform release claim. |

## Validation performed

Local environment: Windows x64, repository-pinned Rust 1.83, Node 24.19.0, React 19.3.0, esbuild 0.25.11 and Playwright 1.62.1. Exact hashes and per-target results are in [the evidence inventory](design-artifacts-evidence.json).

- Workspace `cargo check --workspace --all-targets --offline --locked` passed before final hardening. Workspace Clippy with warnings denied passed after the provider/lease fixes and before the small session-tail backup change. Targeted session tests validate that last change.
- The attempted full workspace test run stopped in the coding-agent library: 1,426 passed, one timeout test failed, 17 ignored in that target. `soft_deadline_leaves_time_for_one_retry_before_hard_deadline` then passed alone. **The full workspace suite is not recorded as passing.** The user subsequently requested only needed tests; broad reruns were not performed.
- All 20 design integration targets ran: 46 tests passed, with two native-runtime tests intentionally ignored without a separately prepared trusted runtime. The offline evaluation target passed both tests. `scripts/test-design.py` enumerates every target; the local run resumed only the remaining targets after correcting the export fixture. Ignored native tests are not passes.
- Session crate: 58 unit and three integration tests passed after adding durable torn-tail backups. The design recovery regression failed without preservation and passed with it.
- Companion: typecheck/build passed; nine compiler/host unit tests and two real Chromium workspace tests passed. Lease expiry and ordinary-call cancellation regressions were first reproduced as failures. The workspace browser tests exercise pairing, revision edits, comments, exports, cancellation and refresh, with a fixture dispatcher.
- Existing browser backend/network/transport/host fixtures: 34 passed. Existing export HTML template tests: four passed. Companion npm audit reported zero vulnerabilities at execution time. YAML parsed and generated contract parity passed. After CI identified an invalid job-level context, actionlint 1.7.7 passed the corrected design workflow locally (optional ShellCheck disabled because it is not installed).
- Final `cargo fmt --all --check` and diff whitespace checks passed. The shipped UI manifest was checked against the actual bundle hashes.
- After integration with main, the 12 existing root-budget tests and 13 design budget/generation/handoff/model tests passed. Only those affected targets were rerun; earlier suite and browser results remain identified as earlier evidence.
- The focused publication interruption test passed all four boundary cases after the test-only hooks were attached. It drops and reopens the writer, verifies old-or-complete source, and confirms retry does not append twice. This tests injected I/O interruption, not an OS power-loss claim. The native test target was compiled after adding CI evidence retention; its known failing Windows capture was not rerun.
- Latest focused checks: all nine handoff tests passed, including real target commands, actual Node discovery, interrupted checks, hook veto/cancellation and a native managed-browser flow. Its retained PNG and JSON receipt are under the external `native-target-evidence` directory. A later focused check passed after binding JavaScript/HTML inputs against stale retries; static sync also passed. The report deliberately remains pending for production integration and RSC/client review.
- Two current-read-authority tests passed. Host evidence reads support canonical Windows paths and dependency files without expanding writable scope; protected paths and current read denial still win.
- Physical termination passed all six boundaries in one focused test after first failing without its test-only checkpoints. This kills the child process without graceful writer cleanup; it does not simulate disk power loss.
- Linux font snapshot setup passed its focused linked-directory, cycle and size-limit test. A simulated `core.autocrlf=true` checkout preserved all 22 hash-bound asset files exactly after adding LF attributes. These address the first Windows/Linux CI failures; remote native capture still requires its own result.
- The changed companion API passed typecheck/build and all six host tests. Earlier full companion E2E evidence remains identified above; it was not repeated for this API allowlist change.
- The next CI run on `8796e575` passed Linux runtime setup and real compiler/host startup. Confined capture failed with an early browser-host exit; bounded native diagnostics are enabled for the next run. Both platform integration jobs reached a stale-input assertion whose expected message had changed after HTML joined static sync. The typed stale-source assertion now passes locally. Private font-cache cleanup was reproduced as a leak, fixed and verified with one focused test. The new native hostile-page/process-tree test compiles and remains pending Linux execution. Focused Clippy and workflow lint passed; Clippy reported a recovered incremental-cache warning.

The Windows traversal export fixture originally normalized away `..` while constructing a verbatim `PathBuf`; it now preserves the raw request spelling. The validator already rejects those raw components. This was a test correction, not a relaxation of export authority.

## Native rendering and release blockers

The explicitly attempted generated-content capture failed with:

```text
MissingCapability: command launch failed before child start:
SandboxUnavailable: no backend can enforce the requested sandbox policy
```

The required network-denied, filesystem-isolated, process-tree-owned policy was retained. No fallback ran generated JavaScript in the companion origin, trusted Node host or an unrestricted browser. The earlier real compiler/host test passed; it proves supervised compilation and host startup, not confined rendering.

The refreshed `runtime-v3` bundle inventories 348 trusted files, the installed Chromium tree and local fonts. This is a prepared runtime, not a successful rendering receipt. The Linux native gate runs on pull requests and retains its log plus PNG and receipt when capture succeeds; this local record does not claim that remote gate passed. macOS is unverified. Cross-platform font metrics are not assumed identical.

Before this can become release-ready:

1. Obtain successful strict native capture, hostile network/process cleanup and primary prototype interaction receipts on supported Windows and Linux confinement backends. Mark macOS unverified until measured.
2. Complete target-specific production API, mock-removal, source-coverage and RSC/client review. The new command/browser receipts explicitly leave these dimensions incomplete; mock API behavior, preview captures and applied patches cannot certify implementation.
3. Finish generated-content process-tree coverage across supported platforms and the remaining target-specific failure matrix. Coverage percentage was not measured; no 80% coverage claim is made.
4. Review the full workspace test limitation and CI on the PR's exact source. Any later live comparison or installed release remains a separate explicitly authorized action.

## Artifact binding and review

The evidence inventory binds local executable, UI, compiler/lock manifest, runtime inventory, browser/font inventories, profile/fixture sources and retained check logs with SHA-256. It excludes the generated inventory itself and this readiness prose from its source-file set to avoid a circular hash. Local binaries, browser packages, screenshots and raw logs are not committed. The companion screenshot is UI-test evidence only and does not depict a generated native-rendered design.

Review used the supplied specification and plan as the reference, a fresh diff inspection, deterministic contract checks, targeted regression failures/passes and browser interaction evidence. No independent reviewer agent was used because the user prohibited subagents. Self-assessment: **7/10; not release-ready** because the native rendering and target-verification gates above remain unresolved.
