# P3: Browser / Playwright Verification

Status: design approved by the user on 2026-09-17; implementation in progress.
Execution sequence: 4 of 12.
Dependencies: P2 and P4 green; existing interaction_testing protocol/evidence/artifact contracts.
Requirements authority: project section 8 and cross-cutting sections 18-40;
project numbers are kept stable while execution order follows the program design.

### Shared Graph browser transport checkpoint

Writer and TestAnalyzer browser tools remain deferred and require the existing
authenticated parent task-coordinator channel. Contextual worker dispatch forwards
all ten tools with its live cancellation signal and bounded 35-second transport
deadline. Transport failure has no local-engine fallback. The parent carries the
normal session's controller and trusted supervisor, creates a worker process lease,
and uses the same native browser implementation. Each worker tracks only its own
opaque contexts. Teardown closes those contexts even after tool permission is
revoked; the shared engine and artifact budget survive other workers' teardown.
Unenforceable hard execution contracts fail closed rather than silently executing
the browser outside a contracted sandbox.

Executed evidence:

- Worker dispatch was RED: an unavailable parent returned the disabled local
  browser fallback. It is GREEN after forwarding through the parent channel.
- Role/transport/deferred-tool regression passed after the original RED
  `browser_open` transport requirement failure.
- Explicitly enabled real Chromium test
  `graph_browser_transport_shares_server_and_isolates_worker_contexts`: one passed,
  zero skips. Two authenticated worker transports reuse the same real managed
  HTTP server; browser cookie state is isolated, foreign context IDs are denied,
  all ten actions are exercised, revoked snapshot permission is denied, and
  teardown removes exactly the departing worker's context. This is a transport
  fixture, not yet the full Graph scheduler/worker frontend evaluation.
- The real fixture exposed Node failing to resolve a relative script from a
  Windows verbatim current directory (`EISDIR`, `lstat 'C:'`). Supervisor startup
  now simplifies that spelling only if both paths canonicalize to the same
  authorized directory. The non-ignored
  `graph_worker_node_resolves_relative_script_from_canonical_workspace` regression
  passes, retaining the Writer role's prohibition on inline Node evaluation.
- Focused library browser checks: 17 passed, two real-backend cases explicitly
  ignored in that offline invocation. Parent coordinator checks: eight passed,
  one actual-browser case explicitly ignored. The actual Graph case was then
  run explicitly and passed. Agent process-manager checks: 16 passed. CLI
  extension-host checks: 12 passed. The actual normal-session browser test also
  passed with both executor attachments, zero skips.
- Affected-package formatting check and all-target Clippy with `-D warnings`
  passed for agent and coding-agent. Diff whitespace and production dispatch,
  authority, ownership, teardown and Windows path changes were reviewed solo.

Native-adapter prerequisite `707674f` has green CI `35294015238`, workflow lint
`35294015248`, and SARIF interoperability `35294015230`. This checkpoint still
requires its own exact-head CI. Distinct Graph worktree binding, contracted browser
execution, scheduler/worker end-to-end evaluation, in-flight cancellation, recovery,
cross-platform listener proof and transaction-bound evidence remain open. The full
program and remaining P3 gates are unchanged.

### Native normal-session dispatch checkpoint

All ten browser tools now use the existing native registry and context-aware CLI
executor attachments. The shared controller reserves startup without holding the
native-host lock over process I/O. Each action consumes the current P2 request
authority and rechecks its opaque owner/session/lifetime binding and OS listener
ownership before accepting the result. Navigation accepts only a bounded local
path under that server's origin; actions accept role/name, label or test-id
selectors. Arbitrary scripts, protocols, origins and unknown fields are rejected.
Tools declare both process and network effects while keeping their conservative
permission class. Outputs explicitly identify themselves as observations, not
transaction verification or completion proof.

`browserVerification` is optional global host configuration (`enabled`, absolute
`node`, absolute external Playwright `package`, pinned `version`). Project settings
can disable it; they cannot enable a disabled host or replace host executable,
package or version pins. Malformed feature settings disable the feature without
dropping unrelated settings. No package installation or browser launch occurs at
ordinary startup. Disabled calls return structured source/test fallback.

Executed checks for this checkpoint:

- Actual installed Chromium test
  `normal_browser_native_dispatch_actions_revocation_and_cleanup`, explicitly
  enabled with the trusted Node/Playwright paths from the baseline. One test
  passed with both normal and shared executor attachments, zero skips. It starts
  a real P2-owned Node HTTP listener, observes a planted button failure, edits
  `index.html` through the engine, opens a fresh context on the same server,
  exercises all ten native tools, observes corrected DOM/ARIA, retains a PNG
  artifact without binary tool output, denies a revoked snapshot permission, and
  closes the context and server. Foreign-owner and cancelled context requests
  are rejected without destroying the authorized caller's browser. The initial
  teardown assertion incorrectly expected `stopped`; P2 transitions through
  `stopping` asynchronously to `exited`. The fixture now polls the same server
  record with a five-second deadline to prove terminal cleanup. This fixture is
  deterministic provider dispatch; it is not
  a live LLM evaluation or the whole-program login acceptance.
- Four offline native/settings cases passed: bounded parser and schema, project
  pin protection and disable-only behavior, disabled/cancelled no-start behavior,
  and malformed settings preserving unrelated values. The actual Chromium case
  remains explicitly ignored in the ordinary offline test invocation.
- Capability-effect regression was RED with `Other("other")` for `browser_open`,
  then GREEN after all ten tools declared process and network authority. All
  949 agent library tests passed, zero skips or failures.
- Coding-agent library browser checks: 15 passed, one explicitly ignored actual
  browser case. CLI extension-host checks: 11 passed. CLI settings checks: 20
  passed. No failures. Commands used `cargo test --offline --locked`, the affected
  package, target and module selectors, prefixed by `rtk proxy`.
- Final `rtk proxy cargo fmt -p davinci-agent -p davinci-coding-agent --check`
  and `rtk proxy cargo clippy --offline --locked -p davinci-agent
  -p davinci-coding-agent --all-targets -- -D warnings` passed. Diff whitespace
  and the changed registration, configuration, dispatch and adapter paths were
  reviewed solo. Incomplete lifecycle/evidence gates listed below remain open.

Socket-proof prerequisite `645e2eb` has green CI run `35292905555`, workflow lint
`35292905598`, and SARIF interoperability `35292905478`. Those runs prove the
prerequisite commit, not the native adapter's unpushed changes.

Remaining P3 work includes shared parent Graph transport/role integration,
in-flight request cancellation, stale-resource reconciliation, macOS listener
ownership and IPv6-only listeners, source/action/transaction-bound receipts,
public retained-artifact retrieval, the required normal/Graph frontend evaluation,
full affected-package/eval gates, final diff review and exact-head CI. P3 remains
in progress; the later nine projects and all global acceptance/performance/report
requirements remain in the program scope.

### Initial HTTP network boundary implementation checkpoint

Implemented `interaction_testing/browser_network.js` using Node built-in HTTP.
It validates canonical trusted origins before listening, checks every absolute
request before outbound I/O, does not follow redirects, replaces Host, strips
proxy credentials and hop headers, and owns inbound/outbound connection cleanup.
Bounds: 32 origins, 64 inbound sockets, 32 active requests, 4096 total requests,
16 KiB headers, 8192-character URLs, 1 MiB request bodies, 16 MiB responses,
15-second request deadline, and five-second header deadline. This is a development
foundation; authorized HTTPS CONNECT and WebSocket upgrades currently return 403
and remain required implementation work.

Observed RED: the initial Node acceptance file exited 1 with MODULE_NOT_FOUND
before any of its six cases ran. After implementation, all six cases passed;
four additional tests covering header filtering, held upstream cancellation,
concurrent admission, and response-byte bounds passed (10 total, zero skipped).
Command: `rtk proxy node --test crates/davinci-coding-agent/src/interaction_testing/browser_network.test.cjs`.

Actual installed Chromium acceptance ran `browser_network.real.test.cjs` with
`DAVINCI_TRUSTED_PLAYWRIGHT_TEST_PATH` pointing explicitly to the host-installed
Playwright package recorded in `evidence/p3-browser-baseline.json`. One actual
browser test passed, zero skipped. Both a foreign-port redirect and a foreign-port
image subresource produced zero hits at the forbidden server. The allowed
same-origin redirect retained its final URL and expected DOM. The test always
closes its context, browser, proxy and fixture servers through teardown hooks.
Without an explicit trusted test package, this optional real test skips; that
skip is not acceptance. No runtime dependency installation or project module
resolution was performed.

Foundation CI run 35288205170 at pushed head 2f4214c currently has 21 successful
jobs and one running Windows native job. This is not CI for the uncommitted
network helper and does not close P3. The production bridge, request-time native
authorization, HTTPS/WebSocket enforcement, artifacts, cancellation and reuse,
normal/Graph dispatch, planted login failure/fix evaluation, and milestone
package/platform checks remain required.

### HTTPS/WebSocket and actual frontend backend checkpoint

Extended the per-context proxy with bounded HTTPS CONNECT and WebSocket support.
CONNECT validates an exact host:port authority before any outbound connection;
userinfo, paths, schemes, malformed ports, foreign ports and nested tunnels fail
closed. HTTPS tunnels are opaque TLS streams restricted to approved destinations.
For an HTTP-origin CONNECT (Chromium's ws:// path), the proxy sends CONNECT success
and reuses Node's HTTP parser, then requires a matching authorized WebSocket
upgrade before outbound I/O. Ordinary tunneled HTTP and foreign Host headers are
denied. WebSocket upgrades replace Host and strip proxy credentials. Requests
and tunnels share the 32-active/4096-total admission budget; each tunnel has a
15-second lifetime and 16 MiB aggregate byte limit. Close waits for actual owned
socket close events, including upgrades not awaited by server.close itself.

Observed RED: two authorized tunnel tests returned 403 before implementation.
First GREEN attempt exposed a close-event ordering failure; close was fixed and
the focused suite passed. Actual Chromium then exposed that ws:// uses CONNECT,
which the direct HTTP-upgrade fixture had not covered. The restricted HTTP-origin
CONNECT path fixed it. A test-helper defect was also fixed: parser-level closure
before a handshake now rejects the helper promise instead of waiting forever;
one held local test process was interrupted, then the corrected test rerun passed.
Malformed HTTP syntax may close the transport before the application sends 403;
the adversarial acceptance requires that closure or 403, and zero outbound hits.

Added `interaction_testing/browser_backend.js`, a host-only backend resource
adapter. It does not add a permission or capability registry. Setup is lazy;
simultaneous opens reserve one Chromium startup and get separate contexts.
The trusted package loader requires an explicit absolute real path outside the
workspace, checks package name and pinned version, and never auto-installs or
resolves project node_modules. Contexts block service workers and downloads;
origin routes and WebSocket routes precede navigation. Actions are serialized
within each context and use role/name, label or test-id selectors. Unknown
fields and arbitrary scripts/CDP are rejected. Viewports, input text, snapshots,
accessibility, screenshots and aggregate telemetry are bounded. Telemetry
omission prevents healthy evidence. Backend screenshot bytes are internal:
native immutable artifact references through the existing Rust tracker are
still required and not implemented by this checkpoint.

Validation actually performed:

- 20 deterministic tests passed, zero skipped: `node --test` on
  browser_backend.test.cjs and browser_network.test.cjs. This includes shutdown
  during shared startup. Initial backend test file RED was MODULE_NOT_FOUND
  before individual cases executed. All six backend cases now pass, including
  aggregate multibyte telemetry overflow that cannot produce healthy evidence.
- Three actual Chromium network cases passed, zero skipped: HTTP redirect/image,
  ws:// owned/foreign upgrades, and HTTPS/WSS plus a forbidden redirect. The TLS
  fixture generates and removes a one-day self-signed certificate in its own
  temporary directory; ignoreHTTPSErrors is explicitly fixture-only. This does
  not establish production certificate-validation or general network confinement.
- One actual backend frontend evaluation passed, zero skipped: planted login
  failure yielded missing successful UI, console error and HTTP 500; corrected
  page exercised type/select/click, successful DOM, ARIA, PNG and healthy telemetry.
  Two further contexts shared the engine while cookie state stayed isolated.
- Node syntax check and git diff whitespace check passed.

Final combined real-test run passed all four cases, zero skipped. The scoped
results and remaining gaps are recorded in
`evidence/p3-browser-backend-checkpoint.json`.

Trusted local real-test inputs were the explicit Playwright package/version in
`evidence/p3-browser-baseline.json`, and Git's existing OpenSSL executable for
the generated TLS fixture. Real tests skip if these explicit test inputs are
missing; that skip is never acceptance. The three-platform native CI matrix now
runs the deterministic browser tests. Actual browser/native normal/Graph CI
acceptance is still required at the P3 milestone. CI run 35288750675 for the prior
HTTP commit fbe3d9c was observed live, with package tests successful and native
platform/quality jobs still running; this is not CI for this checkpoint.

Next required work: supervised typed JSONL host bridge; P2 process-owner/port
lease integration; current permission/role checks and cancellation at dispatch;
source/transaction-bound immutable artifacts and RealBrowser receipts; optional
host settings; normal and Graph tool registration; concurrency/security/lifecycle
probes (including non-proxied protocols and TLS connection reuse); affected
package tests, final diff review and exact-head green CI. The old fixture-only
browser_bridge.js remains unchanged and must not be used as the production path.

### Correlated JSONL transport checkpoint

Added `interaction_testing/browser_transport.js` and focused deterministic/actual
browser tests. Frames are validated as UTF-8 and capped at 16 KiB before parsing;
responses are capped at 64 KiB after JSON escaping. Positive safe-integer request
IDs must increase, preventing replay without an unbounded history. At most sixteen
requests may remain pending, including output backpressure; output writes have a
five-second deadline. Close and shutdown bypass the per-context action queue,
abort pending opens, close contexts and shut down the shared backend. Protocol
errors terminate input authority and clean up resources. Backend exception text
is replaced by a fixed error to avoid returning page/credential contents.

Screenshot requests require a trusted artifact callback before executing. Only
a bounded artifact reference, byte count and media type enter JSONL. The real test
uses an explicitly identified fixture callback, not the native artifact tracker;
neither this callback nor these tests issue a source-bound RealBrowser receipt.

Observed RED: the new transport test file exited 1 with MODULE_NOT_FOUND before
implementation. GREEN: nine transport cases pass; the combined transport/backend/
network suite passes all 29 deterministic cases, zero skipped. The combined actual
Chromium suite passes five cases, zero skipped, including JSONL page interaction
and PNG handoff. Syntax and whitespace checks pass. The CI native matrix now
includes transport cases. Prior-head b9ee135 CI run 35289618547 was observed live:
all package shards and aggregate succeeded, all three native browser steps passed,
and remaining quality/native steps were still running. That run does not validate
this transport checkpoint.

Required next work remains the supervised Rust bridge and trusted artifact sink,
request-time native authorization with dispatch permits, P2 current owner/port/
lifetime leases, native settings/discovery, normal and Graph dispatch, source/
transaction/action-bound evidence, lifecycle/security probes and milestone gates.
The transport is host infrastructure only; no production caller is wired yet.

### Engine dispatch context prerequisite

Added backward-compatible `CustomToolExecutor::new_with_context` and
`execute_with_context`. The engine forwards the current ToolContext to native
callbacks after its existing contract/effect/ledger gates. Legacy constructor and
execution behavior remain available. A context-dependent callback fails closed
when called without engine context; this API alone grants no authorization.

Observed RED: the new regression tests first failed to compile because the
context-aware constructor was absent. After adding it, a stronger actual approved
dispatch exposed that one-time consent was retained only for process/mutation/
transaction boundaries. The engine now also retains consent for context-aware
custom callbacks outside the builtin tool set. The regression verifies the actual
runtime cancellation signal, session JobBook identity, consumption of the exact
approved request once, rejection of a second consumption, and no session allow
rule. Existing callback compatibility also passes.

Validation: all 943 davinci-agent library tests passed; affected package Clippy
with all targets and warnings denied passed. Coding-agent all-target compilation
passed. This prepares current consent and
resource forwarding but does not yet wire the coding-agent extension callbacks
or implement browser authorization/process leases. P3 remains incomplete.

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

### Supervised browser transport checkpoint

The new host-only `BrowserProcess` launches embedded bridge JavaScript through the
existing OS supervisor in a private directory outside the project. Node and the
pinned Playwright dependency must also be outside the project. It retains stdout
stream identity, limits request/response frames and pending calls, assigns monotonic
host correlation IDs, rejects replay/malformed responses, and invalidates unknown
delivery outcomes rather than retrying. No model-visible registry or permission
system is added. Platform path environment variables use the supervisor's existing
allowlist; loader and credential environment variables are not inherited.

Screenshots use eight bounded, exclusive staging files with separate transfer
identities and content hashes. Rust validates references, size and SHA-256 with a
bounded read before importing bytes into the existing `ArtifactBudgetTracker` under
unique labels. JSONL carries references rather than binary/base64 data. Shutdown
closes browser resources and streams; dropping the host stops its supervised OS
lifetime and removes known private files after reaping.

Actual Windows Chromium regression evidence: navigation to a loopback button page,
role-based click, changed DOM, verified PNG retained in the existing tracker, replay
refusal, zero exit and private-directory cleanup passed. The actual test requires
explicit trusted Node/Playwright settings and is ignored in ordinary offline CI;
it was explicitly enabled locally. Red runs found empty-environment Node crypto
initialization failure, Windows extended-path resolution failure, and a paused
stdin pipe keeping Node alive. The platform allowlist, native realpath resolver,
and post-cleanup stream closure fixed those failures.

Executed checks: 62 affected Rust interaction cases passed (one explicitly ignored
real-browser case), 31 deterministic Node cases passed, the separately enabled
supervised Chromium case passed, coding-agent all-target Clippy with warnings denied
and formatting passed. Solo diff review was used as requested. Native normal/Graph
browser dispatch, current-source/transaction binding and completion receipts remain
unfinished; P3 is not complete.

### Managed listening-socket ownership checkpoint

`ProcessManager::with_verified_browser_dev_server` adds request-time OS listener
proof before browser I/O and before accepting its result, alongside the existing
current permission, cancellation and managed-lifetime checks. Bindings pin the
managed PID's OS creation identity. Windows uses the native TCP owner table and
process snapshot, comparing creation times along descendant ancestry. Linux uses
bounded procfs TCP inode, descriptor and process-identity reads. No shell commands
or new resource/permission registry are introduced. All candidate IPv4 loopback
listeners must belong to the managed child or a verifiable current descendant.
The metadata-only API remains available for compatibility; native browser I/O
must use the verified entry point. This is a point-in-time check and does not
reserve a port against concurrent rebinding between observations.

Executed local Windows validation: the real listening-socket test failed red
before implementation, then passed. The actual managed Node regression passed
for direct and child listeners, denied a foreign listener without invoking the
callback, and rejected callback evidence after port takeover while the managed
process remained running. All 16 affected process-manager tests passed; agent
formatting and all-target Clippy with warnings denied passed. Linux execution
remains to be proven by CI. macOS currently reports ownership unavailable and
fails closed; macOS and IPv6-only listener support still require implementation.
Native browser dispatch, Graph consumption and source-bound receipts remain
unfinished. This checkpoint does not complete P3.

### Engine attachment context checkpoint

Both foreground and shared extension executor attachments now use the existing
context-aware custom executor. Calls without engine context fail closed. The
extension-host entry point reads the live cancellation signal before dispatching
native, JavaScript or manifest tools. The shared host is cloned before dispatch,
preserving release of its mutex before external I/O. Legacy host entry points are
retained for compatibility. This is a dispatch prerequisite, not browser tool
registration or an authorization grant; the browser adapter still must consume
the current process manager, exact dispatch permit and bound managed-server lease.
It does not cancel already-running legacy extension I/O.

Validation: the initial executable regression failed to compile because the
contextual host method was absent. After implementation, both contextual dispatch
regressions passed, all 11 extension-host cases passed, and the real normal-agent
edit/replan/test integration passed. Coding-agent executable/tests Clippy with
warnings denied and package formatting passed. An initial library-only selector
ran zero tests; it is not counted as validation. Diff reviewed solo, as requested.
P3 remains incomplete: native browser dispatch, Graph consumption and real
source/transaction-bound receipts still require implementation and validation.

### Managed dev-server attachment checkpoint

The host-only `ProcessManager::with_browser_dev_server` boundary now checks the
exact request against current permissions, cancellation, workspace, and the
caller's active managed process lease. It pins owner/session/process lifetime and
declared port, then rechecks authority and the binding after host I/O. Historical
status access after lease release remains unchanged. A cached binding does not
grant permission. Declared port metadata remains unverified; this does not prove
that the managed PID owns the listening socket.

Executed validation: all 14 `process_manager::tests` cases passed, including three
browser boundary cases using actual supervised Node processes. They cover foreign
owners, undeclared ports, release while another caller keeps the process running,
pre-dispatch cancellation/revocation, and cancellation/revocation/release during
the callback. Agent formatting and all-target Clippy with warnings denied passed.
The diff was reviewed solo as requested. Native browser dispatch, supervised Rust
bridge integration, real source-bound receipts and retained screenshots remain
unfinished, so this checkpoint does not satisfy P3's completion gate.

### Real redirect boundary experiment

Draft [PR #12](https://github.com/J12003LPZ/davinci/pull/12) targets the validated
P4 branch. Foundation source/eval commit is `2f4214ce1902067a82aba4a9b747cb8f40c3467f`.
Its CI `35288205170` is running; green has not been claimed.

Two real-browser loopback-only probes tested the security assumption behind the
old bridge's route.continue implementation. Even context-wide routing with an
origin check and blocked service workers did not route a redirected URL: the
forbidden destination server received one request and the browser navigated to it.
Installed types.d.ts also explicitly states the handler runs only for the first
URL on a redirect. A post-navigation check would detect the problem too late.

A per-context forward proxy using the same canonical origin allowlist and
`bypass: '<-loopback>'` blocked the cross-origin redirect (403, zero forbidden
server hits), while allowing a same-origin redirect (200, actual final URL and
DOM preserved). All browsers, contexts and temporary servers closed. Exact probe
commands/results are in [p3-browser-redirect-probes.json](evidence/p3-browser-redirect-probes.json).
These temporary probes are experiments, not shipped regression tests or production
implementations, and do not establish HTTPS/WebSocket/general egress confinement.

Implementation refinement: enforce the approved backend origin boundary at a
host-owned per-context forward proxy before Chromium sends requests. Use Node's
built-in HTTP/TCP APIs; do not add a permission registry or project-configurable
proxy. The managed process lease and trusted policy still supply the origin set.
Context routing remains defense in depth and telemetry. Add actual bridge tests
for redirect/subresource/popup traffic, HTTPS CONNECT and WebSocket upgrades,
resource bounds, cleanup and cancellation before accepting this implementation.

### In-flight cancellation checkpoint

Normal and Graph browser actions now pass their live abort flag to the shared
supervised transport. A host-assigned cancellation request interrupts the targeted
open or action, closes its context, and waits for the original response before
retiring its correlation. Control requests have reserved bounded admission.
Unconfirmed cancellation invalidates the transport without retrying an action.
Pre-cancelled calls do not write input. Explicit close remains cleanup authority.

Executed checks: all 11 Node transport tests passed; the focused Rust browser
selector passed 17 tests with two explicitly configured browser fixtures ignored.
The explicitly enabled supervised Chromium fixture passed without skips. It
interrupts a missing-selector action within two seconds, checks empty pending
correlations and no transport failure, then navigates and snapshots another
context successfully. Its first run exposed an uninitialized second page in the
fixture; navigation was added before asserting surviving page content. Package
formatting and all-target Clippy with warnings denied passed.

The real normal-session native dispatch fixture was subsequently extended with
an in-flight missing-selector action. Both attachment variants cancel within two
seconds, remove that context and preserve another context. The explicitly enabled
executable fixture passed without skips. Checkpoint `487b6f6` was pushed; CI
`35295532393` was queued when inspected, so exact-head CI is not yet established.

The authenticated Graph transport Chromium fixture now also admits a third
worker, reuses the same managed server, opens its isolated context and cancels a
missing-selector action. It waits for the parent handler response with client
early-abort disabled, joins worker teardown within two seconds, checks the other
two contexts survive and verifies another worker's DOM. The explicit fixture
passed without skips; all eight offline coordinator-handler tests passed (the
real fixture is ignored in that offline selector).

### Dead backend recovery checkpoint

A new supervised Chromium regression reproduced cached dead-backend reuse (RED),
then passed after adding transport health and controller reconciliation (GREEN).
Exited or invalidated transports lose their context references before request
admission, so stale contexts cannot exhaust the eight-context quota or be revived.
References are dropped outside the store lock. A fresh authorized open lazily
starts a replacement backend; interrupted actions are never replayed.

Executed checks: 17 focused offline Rust browser tests passed with three real
fixtures ignored in that lane. All three explicitly enabled library browser
fixtures passed without skips. The extended normal-session fixture also passed
without skips across both attachment variants: after backend shutdown, an old ID
fails, a new open attaches to the same managed process with a new ID, and a new
click succeeds. This establishes exited-transport recovery; it does not establish
managed-server crash reconciliation or every browser-engine crash behavior.

P3 remains incomplete. Full Graph scheduler evaluation,
managed-server stale-resource reconciliation, remaining platform ownership
proofs, transaction-bound receipts, artifact retrieval and final evaluations still
require implementation or direct evidence. This checkpoint is not its completion
gate, and existing-head CI does not validate these uncommitted changes.

### Managed-server lifetime reconciliation checkpoint

The normal-session Chromium fixture reproduced a stale quota entry after
`process_stop` (RED: one context remained). It now passes across both attachment
variants (GREEN). Before browser admission, the controller observes each
resource's originating managed-owner lifetime and closes expired resources.
Observation uses an opaque weak owner reference, so it neither grants permission
nor retains a stopped session. Process metadata and browser cleanup run outside
the browser store lock. Current request authorization remains mandatory.

The focused ownership regression passed: a released parent lease expires while
its child's shared server lease remains live; dropping the child invalidates its
observer. The explicitly enabled real Graph browser fixture also passed without
skips, preserving independent worker contexts during reconciliation.

This proves explicit process release and owner teardown, rather than every
unannounced crash race. P3 remains incomplete: full Graph scheduler evaluation,
contracted execution, remaining platform ownership proofs, transaction-bound
receipts, artifact retrieval, startup coordination and final evaluations remain.
Exact-head CI is required after pushing this checkpoint.

Additional checks passed: all four `process_manager::tests::browser_` tests,
formatting of both affected crates, all-target Clippy for `davinci-agent` and
`davinci-coding-agent` with warnings denied, and `git diff --check`.

### Shared startup completion checkpoint

Six simultaneous callers reproduced `browser startup busy` from the reservation
flag (RED). The same real-backend test now passes (GREEN): all six receive the
same backend reference and that backend opens Chromium successfully. Startup
reserves one attempt under the store lock, performs supervised I/O outside it,
then publishes one result to the attempt's waiters. A failed attempt releases the
reservation for a fresh caller; its existing waiters retain its original result.
No interrupted action is replayed. Each waiter has a 30-second deadline and
checks its own cancellation signal at most every 10 milliseconds. A cancelled
waiter does not invalidate the shared startup. Browser admission rechecks current
permission, process lifetime and OS socket ownership after waiting.

Executed checks: 30 focused offline Rust browser tests passed; four real fixtures
were ignored in that lane and then explicitly enabled, with all four passing and
zero skips. The real normal-session native fixture also passed across both
executor attachments. Two offline startup cases cover waiter cancellation and
failure sharing/fresh retry. Formatting, all-target coding-agent Clippy with
warnings denied, and diff whitespace checks passed. These checks establish shared
startup and preserve the existing normal/Graph transport behavior; full Graph
scheduler evaluation and the remaining P3 evidence/sandbox/platform gates are
still open. No subagents were used.

Managed-lifetime prerequisite `77a9d4b` CI run `35296495119` was inspected while
active: no failed jobs, with affected coding-agent/native/quality jobs still
running. Workflow lint passed; a pending run is not a completion gate. This
startup checkpoint requires its own exact-head CI after pushing.

### CI fixture isolation checkpoint

Exact startup-head CI run `35296825175`, coding-agent job `105450990735`,
failed in `llama::tests::live_sse_watch_streams_frames_incrementally`: the event
vector was empty before indexing. The live test and neighboring fixture tests
shared process-wide `PI_LLAMA_SSE_REPLY` and `PI_LLAMA_DRY_RUN` overrides without
synchronization. A test-only mutex now serializes all llama management fixture
tests, including the live server test, for the full lifetime of their watch
threads. The live test asserts the event count before accessing the event.

All nine llama tests passed locally with eight test threads and no ignores.
Coding-agent formatting passed. This repairs a CI prerequisite; it does not
complete P3's macOS ownership, sandbox, artifact, receipt or evaluation gates.
The repaired head still requires exact-head remote CI.

### Native ownership CI coverage checkpoint

The native CI lifecycle selector omitted the sibling socket-ownership unit
module. Windows and Linux now run its direct live-listener proof explicitly.
The focused Windows run passed one test with zero ignores: the actual live owner
is accepted, a foreign PID and a closed listener are rejected, and port zero is
rejected. macOS is deliberately excluded from this step while its native proof
remains unimplemented; an empty test selection must not imply platform coverage.

The coding-agent and agent package jobs for fixture-repair head `1cdcd8b` passed
in CI run `35297281753`; that complete run was still active at inspection. Local
diff whitespace checks passed. Local actionlint was unavailable; the pushed
workflow-lint check must validate this workflow change. This checkpoint adds CI
coverage, without completing P3's platform, sandbox or evidence gates.

### Darwin listener ownership implementation checkpoint

The macOS implementation now queries `net.inet.tcp.pcblist_n` and matches every
IPv4 loopback/wildcard TCP listener at the requested port to a live socket
descriptor. It uses the common hashed socket identity from Apple's
[global socket table](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/netinet/in_pcblist.c)
and [per-process socket query](https://github.com/apple-oss-distributions/xnu/blob/main/bsd/kern/socket_info.c).
The historical `so_last_pid` field is not an ownership proof. Descriptor holders
must belong to the managed process tree; birth times prevent admitting a reused
PID. The root identity and global endpoint identities are rechecked after the
descriptor scan. This is request-time observation, not a reservation against
changes after the check.

The parser bounds snapshots at 1 MiB and 4,096 complete groups, requires stable
generation/count framing, and rejects incomplete, ambiguous or unknown record
sequences. Descriptor inspection is bounded at 4,096 processes and 65,536 file
descriptors, with a 32-parent ancestry limit. Unreadable descriptors do not grant
ownership: every global matching listener still needs a proven managed holder.
IPv6-only listeners remain a separate unfinished gate.

RED: the new valid-listener parser regression failed with the unavailable proof.
GREEN: all four focused socket tests passed on Windows with zero ignores,
including three portable Darwin parser cases. The existing managed-root/child,
foreign-listener and takeover regression also passed on Windows. Agent formatting,
all-target Clippy with warnings denied and diff whitespace checks passed.

Native CI now runs the ownership selector and affected agent Clippy on all three
platforms before the downstream integration checks. The macOS lane additionally
checks Rust offsets against its installed C SDK and probes two simultaneous
reuse-port listeners: a genuine child listener must not hide the foreign parent
listener on the same endpoint. These macOS checks have not run locally and must
pass on this checkpoint's exact pushed head before platform completion is claimed.
The earlier `c192282` macOS job passed while ownership was excluded; it is not
evidence for this new implementation. No subagents were used. P3 and the full
engineering program remain in progress.

### Darwin SDK validation correction

CI run `35298203925`, macOS job `105455037534`, compiled the native Rust
implementation and passed six ownership tests, including actual listener
ownership and the simultaneous foreign reuse-port listener regression. Its C
SDK check failed because the installed public SDK omits private `xinpcb_n` and
`xsocket_n` declarations, even when `PRIVATE` is defined. The corrected check
validates the public `xinpgen` and libproc descriptor ABI and constants against
the installed SDK. Private wire-record compatibility remains a real-kernel
runtime gate; it is not claimed to be validated by the public SDK. This
correction needs its own native CI result before the SDK gate is closed.

For this correction, all four portable/Windows socket tests passed without
ignores; agent formatting, all-target Clippy with warnings denied and diff
whitespace checks passed. Production ownership code is unchanged.

### IPv6 managed origin checkpoint

The SDK correction at `00ea149` passed all jobs in CI run `35298502457`,
including the Windows, Linux and macOS native ownership lanes. This closes that
correction's CI gate, without completing P3.

`browser_open` now accepts an optional `host` of `127.0.0.1` or `::1`, defaulting
to IPv4. The selected family becomes part of the immutable dev-server lease;
subsequent actions use that lease and cannot switch origins. Other host values,
including `localhost`, bracketed input and null, are rejected. IPv6 origins use
the canonical bracketed URL spelling. Windows queries the IPv6 owner table,
Linux reads the IPv6 proc listener table, and Darwin checks the IPv6 address and
family in its existing bounded socket records. All matching listeners still
require current managed ownership and process identity checks.

RED: the new actual IPv6 ownership regression failed before implementation.
GREEN: all 22 focused process-manager tests passed on Windows, with zero skips,
including managed root/child listeners, foreign listeners and port takeover for
both address families, plus portable Linux/Darwin address parser cases. Five
offline native browser cases passed; two real-backend cases were explicitly
ignored in that offline lane. The explicitly enabled normal-session Chromium
fixture passed with zero skips across all four address-family/executor attachment
combinations, including the planted UI failure, edit/fix, all ten actions,
revocation, cancellation and cleanup. Affected-crate formatting, all-target
Clippy with warnings denied and diff whitespace checks passed. The explicitly
enabled authenticated Graph transport fixture also passed with zero skips for
both families, proving server reuse, cookie/context isolation, all ten actions,
cancellation, revocation and teardown. This is not the full scheduler evaluation.
Exact-head native CI remains required.

Native Linux/macOS IPv6 behavior is not established by Windows or portable parser
tests; the new head must pass those native CI lanes. Contracted sandbox execution,
distinct Graph worktrees, transaction-bound receipts, public artifact retrieval
and full scheduler/frontend evaluation remain open. No subagents were used.
