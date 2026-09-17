# P3: Browser / Playwright Verification

Status: design approved by the user on 2026-09-17; implementation in progress.
Execution sequence: 4 of 12.
Dependencies: P2 and P4 green; existing interaction_testing protocol/evidence/artifact contracts.
Requirements authority: project section 8 and cross-cutting sections 18-40;
project numbers are kept stable while execution order follows the program design.

## Outcome

An ordinary session verifies a real local frontend flow and returns bounded DOM/accessibility, console/network and screenshot evidence.

## Scope and ownership

Proposed files/modules (not claims of completed edits):

- crates/davinci-coding-agent/src/interaction_testing/{browser,artifacts,mod}.rs
- crates/davinci-coding-agent/src/native_extensions/browser_verification/ (new host adapter)
- Host-owned optional Playwright bridge under existing scripts/ layout (new, exact package location settled after installed-backend inspection)
- crates/davinci-coding-agent/src/{settings,extension_host,main,shutdown}.rs and native_extensions/mod.rs
- crates/davinci-agent/src/{permission,permission_risk}.rs; Graph role adapter; new browser integration/eval fixtures

The main session is the only writer. No subagents, parallel writers or unrelated cleanup.
Reinspect these paths and callers at implementation time because main may advance.

## Baseline before implementation

Run current fixture browser adapter and demonstrate its FixtureOnly receipt classification. Inspect installed Node/Playwright/browser versions separately; measure real backend cold startup before adding reuse.

## Ordered implementation and RED/GREEN work

1. RED: native browser discovery/dispatch absent, real bridge launch/cleanup, selectors, blocked cross-origin/subresource/redirect/WebSocket traffic, missing executable and artifact overflow.

2. Verify maintained installed Playwright APIs/version and choose a pinned optional host-owned bridge. Follow official network/context/ARIA docs linked from the program design. Do not import project node_modules code into the trusted bridge or auto-download at runtime.

3. Extend existing typed JSONL commands with open, snapshot, click, type, select, console, network, accessibility, screenshot and close. Validate message sizes/request IDs/selector lengths and reject arbitrary evaluate, CDP, downloads/uploads and persistent user profiles.

4. Attach browser_open to a current authorized P2 dev-server lease; validate origin and resource ownership. Establish context-wide HTTP(S), redirects/popups/subresources and WebSocket enforcement before navigation; block service workers. Permit additional origins only through trusted project/network policy.

5. Collect console errors, request failures and HTTP error statuses with bounded queues. Provide deterministic role/name/test-id selectors, accessibility tree snapshots, viewport bounds and explicit action timeout.

6. Store screenshots/traces with existing artifact budgets and immutable references; no model-facing base64 or page/cookie persistence in disk cache. Bind evidence to current source/transaction, browser identity and action sequence.

7. Share browser engine resources safely while separating per-owner contexts and mutable page state. Reserve startup to avoid duplicate launches; serialize per-context operations, not unrelated sessions. Cancel/close/shutdown cleans bridge and descendants.

8. Wire native tools through normal dispatch, current permissions, output governor and relevant frontend discovery; Graph uses the same implementations with role restrictions.

9. GREEN: deterministic fake bridge/offline security tests, actual real-browser local login fixture and cleanup/console/network assertions, related crate gates and platform CI. Missing optional backend returns unavailable but does not satisfy real-browser acceptance.

## Required targeted validation

- No backend startup in ordinary text edit/status; absent Node/Playwright allows source/test fallback without reporting browser success.
- Local page with planted click bug, console error and failed network request must yield failing source-bound verification; corrected page passes actual interaction.
- Unauthorized navigation, crafted origins/userinfo, file/data schemes, popup/redirect, WebSocket and cross-owner session IDs fail.
- Screenshot artifact bounds, cancellation during launch/action, permission revocation between calls, simultaneous startup and final context cleanup.

## Settings and fallback

browserVerification.enabled is independently configurable; backend path/version is trusted host configuration. Default enabled capability availability remains lazy and reports missing backend without install.

## Specific acceptance guard

FixtureOnly receipts and source inspection cannot prove RealBrowser completion. Accessibility snapshots are evidence, not an unsupported full accessibility compliance claim.

## Completion gate and handoff

The approved [program design](../../specs/2026-09-17-engineering-program.md)
and original [requirements](../../specs/2026-09-17-engineering-program-requirements.md)
are the reference. Shared authorization, bounded-output, cache, lifecycle, telemetry,
settings and security invariants apply in addition to the project-specific steps.

Record all fourteen section-34 gates in this project's ledger row: design approval,
plan, observed RED/GREEN, affected package tests, fmt, Clippy, integration, security,
normal path, applicable Graph path, benchmark/eval, docs, diff review, and exact-head
CI. Planned fixtures/test names above are not claims of existing or executed tests.

Validation sequence: focused failing acceptance test; focused passing checks after
implementation; affected crate tests with `--offline --locked`; `cargo fmt --check`;
affected Clippy during development and required workspace Clippy at the milestone;
matching integration/security/eval cases; feature-branch push and actual CI monitoring.
Run broader suites only for affected shared contracts or final acceptance. Record
exact executed selectors, exits and test counts; zero-case runs do not count.

Use a separate coherent commit/PR for the subsystem. With earlier PRs unmerged,
stack on the preceding verified branch and target that branch for review. Preserve
dependency commits without force pushes. Do not begin the next subsystem with
known failures or incomplete required gates. Approval to implement this program
does not authorize merging new PRs into main.

The handoff in README records files/APIs changed, validated commands and results,
metric/artifact paths, head SHA/CI URLs, limitations, and the next dependency input.
No production capability is called done from this plan alone.

## Observed baseline and initial RED

The next branch is `codex/browser-verification-01a0ad48`, stacked on validated
P4 `a2054d3` in the same isolated worktree. Fetched main remains `ca9fe69`.
The existing adapter has no production caller, and its receipts deliberately
remain FixtureOnly. Its 11 existing focused unit tests passed (1009 filtered).

Installed Node v24.19.0 and the existing host-installed Python Playwright driver
package 1.62.0 launched Chromium 151.0.7922.34 successfully, with service workers
blocked in a new context. Both context and browser closed. One cold sample from
before launch through page creation measured 429.1058 ms; no improvement is claimed.
The actual context routeWebSocket and locator ariaSnapshot methods were functions.
Exact paths and scope are in [p3-browser-baseline.json](evidence/p3-browser-baseline.json).
No project node_modules package was executed and no installation/download ran.
The maintained APIs were also checked against official
[BrowserContext](https://playwright.dev/docs/api/class-browsercontext),
[WebSocketRoute](https://playwright.dev/docs/api/class-websocketroute), and
[ARIA snapshot](https://playwright.dev/docs/aria-snapshots) documentation.

Five regression tests ran and failed before the first production edit: URL
credentials were accepted, passing assertions ignored console/network errors,
JSONL allowed a 1 MiB event, and event/snapshot queues grew beyond the budget.
The selector was `--lib interaction_testing::browser::tests::browser_` (zero
passed, five failed, 1020 filtered). These are existing-adapter defects to fix
before attaching the live backend. Real-browser workflow, native dispatch,
request-time authorization and every P3 milestone gate remain open.

### Existing-adapter safety foundation

The five initial RED cases are GREEN. Evidence now rejects credential-bearing or
non-HTTP origins, caps JSONL before parsing at 64 KiB, and retains at most 128
events. Budget exhaustion makes evidence incomplete/failing; a close event still
updates lifecycle. Policy-denial and malformed-message paths share the budget.
Console/network failures and zero assertions cannot yield successful receipts or
exit outcomes. Strict validation rejects unexpected event fields, including unit
events that Serde's enum-level deny_unknown_fields alone did not reject.

A follow-up unit-field test initially failed (58 interaction cases passed, one
failed). The explicit strict unit-object validation resolved it. Final executed
checks: all 59 interaction_testing unit cases passed (968 filtered); the separate
browser_evidence_eval integration case passed with six deterministic classifications;
library plus eval-target Clippy with warnings denied passed; formatting and diff
checks passed. Solo review also caught and bounded mixed malformed/normal events.
The persisted [classification artifact](evidence/p3-browser-evidence-after.json)
has zero false successes and zero missed valid cases for those six fixtures. It
proves evidence classification only, not real frontend behavior or live cleanup.

Artifact execution first omitted its environment variable due to nested-shell
expansion; the no-artifact test passed. A relative output path then failed because
Cargo runs integration tests from the package directory. The final run used an
absolute artifact path and passed. Neither attempt is counted as a successful
artifact-producing run. The existing JS bridge still needs replacement/hardening,
native tools are not registered, and P3 completion is unproven.
