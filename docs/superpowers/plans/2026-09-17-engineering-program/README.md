# Engineering program ledger

Status: design package approved; P1, P2, P3, P4 and P5 validated; P7 next.
P5 head `9a3fdf9` passed CI, including Windows/Linux/macOS native matrix,
normal-agent and Graph paths. Its evidence and limitations remain in its plan.
Goal scope: all twelve projects and the global acceptance/performance/report gates.
Reference: [program design](../../specs/2026-09-17-engineering-program.md) and
[unchanged supplied requirements](../../specs/2026-09-17-engineering-program-requirements.md).

## Authoritative workspace checkpoint

- Date: 2026-09-17.
- Fetched remote main: `ca9fe69cd0da21bf161af25b2bed681748fb0d58`.
- Current branch: `codex/browser-verification-01a0ad48`, stacked on validated P4.
- Worktree: `C:/Users/sergi/.claude-worktrees/pi-rust-9416e5cee6/01a0ad48`.
- Git worktree admin path differs from common Git directory; isolation verified.
- Shared checkout has pre-existing changes and divergent local main. Preserved.
- Prior PRs 6/7/8 provide LSP, Repo AST and CacheRuntime; source inspected.
- No agents spawned. RTK used for all shell commands. Headroom MCP compressed
  architecture search output; its separate optional HTTP proxy was unreachable,
  while the compression tool itself returned compressed content successfully.
- A design or plan file is not implementation evidence.
- User approval record: **approved 2026-09-17**. The user answered "Approve the
  design and all twelve plans" in the session input prompt. This satisfies the
  design-approval gate in section 34 for the described program.

## Ordered subprojects

| Order | Project/plan | Implementation | Design approval | Local gates/eval | CI |
| --- | --- | --- | --- | --- | --- |
| 1 | [P1 Test Impact Intelligence](01-test-impact.md) | Complete; PR #9 open | Approved | Package tests, fmt, Clippy, integration, security and eval passed | Green at `6b3aad9` |
| 2 | [P2 Persistent Process Manager](02-process-manager.md) | Complete; PR #10 open | Approved | Package tests, fmt, Clippy, integration, security and eval passed | Green at `bfc60e3` |
| 3 | [P4 Transactional Edit Engine](04-transactional-edits.md) | Complete; PR #11 open | Approved | Package, integration, security, concurrency, normal/Graph and eval gates passed | Green at `a2054d3` |
| 4 | [P3 Browser / Playwright Verification](03-browser-verification.md) | Complete; PR #12 updated | Approved | Package tests, fmt, Clippy, real Chromium normal/Graph paths, network confinement, RPC verification and eval passed | Green at `cdbfe00` |
| 5 | [P5 Package / Dependency Intelligence](05-package-intelligence.md) | Complete; PR #13 open | Approved | Package tests, fmt, Clippy, integration, security and eval passed | Green at `9a3fdf9` |
| 6 | [P7 Build Intelligence](07-build-intelligence.md) | Complete; PR pending | Approved | Package tests, fmt, Clippy, integration, security and eval passed | Pending push |
| 7 | [P6 Git Intelligence](06-git-intelligence.md) | Planned | Approved | Not run | Not pushed |
| 8 | [P9 Change Impact Engine](09-change-impact.md) | Planned | Approved | Not run | Not pushed |
| 9 | [P8 Deterministic Hook / Policy Engine](08-hook-policy.md) | Planned | Approved | Not run | Not pushed |
| 10 | [P10 Verification Planner](10-verification-planner.md) | Planned | Approved | Not run | Not pushed |
| 11 | [P11 Workspace Snapshot / Safe Sandbox Layer](11-workspace-snapshot.md) | Planned | Approved | Not run | Not pushed |
| 12 | [P12 Continuous Agent Evaluation Framework](12-continuous-evals.md) | Planned | Approved | Not run | Not pushed |

P4 precedes P3 as in the supplied Phase A order. P5 precedes P7 because build
intelligence must reuse package/workspace metadata and graph rather than create
another package parser. All other prerequisite edges are documented in the design.

## Requirement coverage and proof still required

This maps every numbered requirement section to its implementation/evidence owner.
"Mapped" means planned, not achieved. The supplied file retains every detailed
bullet, example, invariant, named tool and acceptance requirement for final audit.

| Section | Requirement | Owner and authoritative proof |
| --- | --- | --- |
| 0 | Program, not monolithic patch | Design + twelve plans; separate sequential implementation commits/PRs and green milestone ledger. Planning artifacts exist; execution still pending. |
| 1 | Inspect current main and architecture | Fetch/status/log and source inspection recorded in design; refresh before each milestone. |
| 2 | Detect/use prerequisites | Existing source owners and gaps recorded in design; integration tests must prove reuse. |
| 3 | Target architecture/no duplicates | Design diagram and final diff/API audit; same registry/cache/bus/evidence paths. |
| 4 | Twelve projects | All twelve linked plans and eventual independent gates. |
| 5 | Determinism/evidence/bounds/fallback/permission/shared paths | Every project's production contract + failure/security/normal/Graph tests. |
| 6 | Test Impact | P1: explained deterministic test selection, package boundaries, warm refresh and broader checks. |
| 7 | Persistent Process Manager | P2: argv policy, ownership, all process fields, ring output, reuse/restart/descendant cleanup. |
| 8 | Browser/Playwright | P3: actual managed browser, accessibility/console/network/viewport/artifacts/cleanup and network policy. |
| 9 | Transactional Edit | P4: all lifecycle/provenance fields, stale apply/rollback and crash recovery tests. |
| 10 | Package Intelligence | P5: npm/pnpm/yarn/workspaces/installed metadata, selective symbols and content-bound cache tests. |
| 11 | Git Intelligence | P6: bounded object/symbol history, factual provenance, immutable cache and no mutation. |
| 12 | Build Intelligence | P7: package managers/Turbo/Nx/Vite/Next/references, affected target commands and native-cache preservation. |
| 13 | Hook/Policy | P8: existing bus, every applicable typed event, trust, timeout, warn/block/ignore and real gates. |
| 14 | Change Impact | P9: all eight result sections, all six evidence sources, incomplete analysis and transaction input. |
| 15 | Verification Planner | P10: deterministic cheapest valid sequence, security escalation, CI parity and actual evidence. |
| 16 | Workspace Snapshot | P11: scoped worktree identity, checkpoint/diff/restore and newer-user-edit preservation. |
| 17 | Continuous Evals | P12: twelve task categories and every listed metric, offline versus live separation and flags. |
| 18 | Unified capability discovery | Each native registration + deterministic positive/negative intent, permission/role/workspace tests. |
| 19 | Normal mode first-class | Each project's real normal Agent dispatch integration plus final edit/verify scenario without Graph. |
| 20 | Graph integration | Existing same-tool role allowlists, parent resource reuse and denied-role tests. |
| 21 | Cache runtime | P1/P5/P6/P7/P9 exact dependency keys, authorization, invalidation and corrupt/unavailable fallback; no live process/page serialization. |
| 22 | Output/token efficiency | Stable bounded summaries/rows/provenance/remaining and existing exact output retrieval tests. |
| 23 | Transaction/verification closed loop | P4/P9/P10/P12 actual understand -> impact -> edit -> checks/browser -> source-bound completion. |
| 24 | Performance | Per-project baseline/on-off evaluations and P12 startup/memory/cold/warm/activation/process/browser/cache report. |
| 25 | Concurrency | P1/P2/P3/P4/P8 bounded locks, shared startup, no I/O under global locks, cancellation/shutdown tests. |
| 26 | Windows/Unix | Local Windows plus Linux/macOS CI for path/process/Git/Node/browser/locking behavior. |
| 27 | Security | Every subsystem current permission/trust/boundary tests; immutable cache never bypasses authorization. |
| 28 | Failure model | Optional unavailable structured fallback; transaction integrity fails closed. Failure injection required. |
| 29 | Settings | Independent serde/merge/default/disabled flag tests, lazy construction, critical knobs only. |
| 30 | Observability | Existing telemetry/evidence/status paths record every listed outcome; no fabricated counters. |
| 31 | TDD/testing | Recorded failing/passing targeted cases, integration/failure/security/concurrency, normal and relevant Graph gates. |
| 32 | Evaluation-driven development | Frozen baseline before each subsystem, actual relevant after eval and honest comparison. |
| 33 | Prohibitions | Final diff/design audit plus tests for no duplicates/eager startup/untrusted execution/stale overwrite/false completion. |
| 34 | Fourteen completion gates | Per-project exact evidence rows below; pending means incomplete. |
| 35 | CI | Feature branch pushes, exact SHA run/job status, failed-log diagnosis/fixes until green; no force push. |
| 36 | Order | Dependency table and sequential milestone history; explained P5-before-P7 adjustment. |
| 37 | Global acceptance | P12 real seventeen-step non-Graph login task and selective Graph assignment. |
| 38 | Global performance acceptance | P12 measured improvements/tradeoffs for all ten named outcomes; no every-metric-on-every-task claim. |
| 39 | Final deliverable | Final architecture/files/tools/metrics/quality/reliability/validation/limitations/follow-up report with exact evidence. |
| 40 | Guiding principle | Evidence-first normal task scenario and before/after evaluation, with correctness and authorization preserved. |

## Per-project completion record template

Copy this compact record into the project's completed entry only when evidence exists.

1. Approved design: message/review identity and unchanged scope.
2. Implementation plan: linked version.
3. RED/GREEN: exact test, failing cause/output and passing command/count.
4. Affected package tests: command, exit, count and source SHA.
5. Format: command and result.
6. Clippy: command and result.
7. Integration: normal dispatch/process/filesystem/browser proof as applicable.
8. Security: invariants covered, negative cases and results.
9. Normal mode: actual agent edit/verify route, Graph absent.
10. Graph: same capability/role/parent-resource evidence or justified inapplicability.
11. Benchmarks/evals: raw artifact, baseline/after, scope, false positives/negatives.
12. Docs: user-facing behavior/config/limits and API ownership updated.
13. Diff review: reviewed scope, remaining issues and resolution.
14. CI: exact commit/run/job links and successful relevant conclusion.

Handoff also names all created/modified files, public interfaces, cache/source
identities, intended next dependency and remaining unsupported cases. Never mark
a project complete with a placeholder in a required gate.

## Measurements and validation

The requirements copy matches the attachment byte-for-byte (SHA-256
`bed91f546b31aafb4955fa25c4dccaac204fc36c086245e94df3c1043dfb5352`).
A deterministic document check passed for the fifteen Markdown files: twelve
project plans, every requirement section 0-40 mapped, local links present, and
no trailing whitespace/conflict markers in generated documents. Source API review
corrected the design references to `WorkspaceSnapshot` and `register_with`.
Existing source inspection, document structure checks and a cold requirements
comparison are the three planning review passes. No independent agent review ran,
per the explicit solo instruction.

Pre-implementation measurements on Windows with Node v24.19.0:

- Existing `repo_intelligence_measured_evaluation`: one test passed. Its 200-source
  fixture read 40,847 source bytes during warm validation; warm persistent load
  took 25.053 ms and a single-source edit took 25.2492 ms, reparsing one source.
  These are descriptive debug-profile samples, not production speed claims.
- New explicit [P1 monorepo baseline](evidence/p1-monorepo-baseline.json): three
  packages and 23 real Node tests. The unchanged suite passed all 23. A planted
  token regression caused three failures in the full suite, taking 232.0035 ms.
  Filtering current `related_files` output to test paths selected two tests,
  caught two failures in 86.6941 ms, and missed the downstream cross-package
  `packages/web/test/login.test.mjs` failure. This heuristic is not a complete
  test-selection implementation or permission to omit broader verification.
- The monorepo warm refresh read all 6,173 source bytes despite zero reparses;
  the one-file edit also read 6,173 bytes while reparsing one file. P1 must improve
  source I/O as well as selection correctness. Timing is a single captured sample.

Executed baseline commands (both passed one explicit ignored eval):

```text
rtk proxy cargo test -p davinci-coding-agent --test repo_intelligence_eval --offline --locked -- --ignored --nocapture
rtk proxy cargo test -p davinci-coding-agent --test test_impact_baseline --offline --locked -- --ignored --nocapture
```

The first existing-eval run used `rtk cargo test`, whose summary omitted metrics;
it was rerun through `rtk proxy` to capture them. The new baseline's initial console
output was truncated, so a second sample was captured directly as the linked JSON.
No production acceptance or CI success is claimed from these baselines. Subsequent
implementation evidence is recorded below.
The prior merged PR test counts are not evidence for this new program.

## Next action

P1 passed exact-head CI in PR #9. Publish and monitor P2 on the stacked
`codex/process-manager-01a0ad48` branch in the same isolated worktree. P4 begins
only after P2's required exact-head platform CI is green.
Approval is recorded above; routine implementation decisions need no new approval.
Keep the full program goal active until all global acceptance is verified.

Targeted validation of the baseline fixture passed: `rtk cargo fmt --check` and
`rtk proxy cargo clippy -p davinci-coding-agent --test test_impact_baseline --offline --locked -- -D warnings`.
Headroom was also invoked on the compact plan data; that call returned 0 tokens
saved (`router:noop`). No compression savings are claimed for that call.

## P1 implementation evidence

Implementation and exact-head CI are validated at
`6b3aad9fd28b96f85c8115fe4099bea6e4557b17`; PR #9 remains open for review.
User-facing behavior and limits are documented in [test impact](../../../test-impact.md).

| Gate | Evidence/status |
| --- | --- |
| 1. Approved design | User approved the design and all twelve plans on 2026-09-17. |
| 2. Plan | [P1 ordered plan](01-test-impact.md). |
| 3. RED/GREEN | Observed failing cases for missing deleted-source candidate evidence, lockfile-wide selection, incomplete-import broader checks, authorization after a normal agent edit, and interrupted inventory recovery; the corrected targeted suites passed. |
| 4. Affected package tests | `rtk proxy cargo test -p davinci-agent -p davinci-coding-agent --offline --locked`: exit 0. Existing ignored/live evaluations were not counted as executed. |
| 5. Format | `rtk cargo fmt --check`: exit 0 after source and evaluation edits. |
| 6. Clippy | `rtk proxy cargo clippy --workspace --all-targets --offline --locked -- -D warnings`: exit 0. |
| 7. Integration | Test-impact integration suites: 14 analysis/security/command cases and 6 observer cases passed; existing repository integration suites also passed in the affected package run. |
| 8. Security | Current read-policy revocation, sensitive/outside/excluded inputs, malformed fields, disabled capability, cancellation, bounded commands/output, interrupted inventory recovery, and shared-refresh cases passed. No package script runs during planning. |
| 9. Normal path | `test_impact_integration_tests::normal_agent_discovers_edits_replans_and_runs_tests_without_graph` passed with both executor attachment paths: real Agent tool discovery, native query, built-in edit, new source identity, real Node tests, and subsequent read revocation. No Graph tools or external LLM were used. |
| 10. Graph | Role projection test passed: existing native tools, parent authority, deferred discovery, output retrieval, and denied roles. No duplicate Graph tool or registry. |
| 11. Evaluation | [After artifact](evidence/p1-monorepo-after.json): explicit ignored evaluation passed with real Node tests, 3/23 selected, all 3 planted failures caught, no false-positive paths in this fixture, warm 59/6,173 source bytes, force/fallback/zero-test cases. [Baseline](evidence/p1-monorepo-baseline.json) remains frozen. |
| 12. Docs | User guide, repository freshness guide, documentation index, and this ledger updated. |
| 13. Diff review | Reviewed shared index refresh/observer lifecycle, bounded traversal, command argv, current permissions/cache delivery, host lock release, settings, and normal/Graph dispatch. No subagents, per user instruction. |
| 14. CI | [PR #9](https://github.com/J12003LPZ/davinci/pull/9): [CI run 35235736772](https://github.com/J12003LPZ/davinci/actions/runs/35235736772), workflow lint, and security interoperability passed at `6b3aad9fd28b96f85c8115fe4099bea6e4557b17`. The Windows/Linux/macOS native jobs each passed 14 impact, 6 observer, 1 normal-agent, and 1 explicit evaluation test. [Platform evidence](evidence/p1-ci-platforms.json) contains the actual evaluation objects parsed from those job logs. |

Executed explicit after evaluation:

```text
rtk proxy cargo test -p davinci-coding-agent --test test_impact_eval --offline --locked -- --ignored --nocapture
```

This passed one test. The full correct fixture passed 23 tests; selected correct
tests passed 3. The intentionally broken fixture failed exactly 3 tests in both
full and selected executions, as required by the evaluation. Those expected
fixture failures are not failing Rust tests.

The after sample measures warm total planning at 37.1114 ms versus 83.2957 ms cold.
The older refresh-only baseline was 15.9386 ms warm, so this is a source-I/O and
selection-correctness improvement, not a claim that the complete authorized plan
is faster than the narrower old operation. Source counters exclude metadata and
directory enumeration. Exact timings are one debug-profile Windows sample.

### P1 handoff surface

- New `test_impact/{mod,analysis,commands,tools}.rs` owns planning, settings,
  telemetry and native schemas; `workspace_metadata/{mod,manager}.rs` supplies
  bounded package ownership/script/manager facts for later P5/P7 reuse.
- Existing RepoIntelligence now exposes typed dependency edges, missing local
  candidates, confined configuration reads and observed authorized refresh.
  Its immutable index, existing parser/resolver, persistence and lease remain
  the owners; `observation.rs` adds bounded in-process change hints.
- CacheRuntime's Test namespace stores the reverse mapping under source/config
  content identity. It does not establish freshness or override read authority.
- Native host, extension host, normal CLI context, permission classification/risk,
  settings and Graph roles wire the same tools. Slow planning runs outside host
  locks. The observer has no independent daemon or serialized live state.
- Tests: reusable monorepo fixture, frozen baseline, after evaluation, native
  analysis/security/commands, external writer/concurrency, and normal Agent flow.
- CI adds only the native test-impact platform/evaluation matrix.
- `notify 8.2.0` supplies the cross-platform observer. The lockfile holds
  `notify-types 2.0.0` to preserve the repository's Rust 1.83 toolchain.
- Limits remain explicit: static TS/JS imports, conservative pairing, incomplete
  resolver/config cases, no history yet, no installed-package inspection,
  bounded traversal/commands, periodic/explicit content reconciliation.
- Next input is P2's approved process-manager plan, after P1 CI is green.

### CI correction: reparse reuse during reconciliation

The [initial Linux package job](https://github.com/J12003LPZ/davinci/actions/runs/35193158815/job/105110104119)
failed the existing warm-edit regression at `a7f94531b5a640b2c051d2f7ac2b80047070dc5d`:
`impact_warm_requests_read_only_changed_sources_and_allow_forced_reconciliation`
reported two reparses for one changed file. A late watcher event correctly forced
another content reconciliation, but each attempt reused the old published index
instead of the records already parsed in the preceding attempt.

Refresh now keeps unpublished attempt records as the next attempt's parse seed.
The retry still hashes every source and only reuses a record when its hash matches;
it publishes nothing until reconciliation settles. The regression assertion is
unchanged. After this correction, the 14 impact, 6 observer, 7 repository, and
3 cross-process repository tests passed locally, as did the explicit after eval,
format check, and affected library Clippy. The exact-head CI rerun is the remaining
gate. The initial Linux native job independently passed all 22 impact/observer/
normal-agent/eval cases; its log is available on the same CI run.

### CI correction: ignore changes before watcher delivery

At `722a23a41cda28980151960646af124fdb93808b`, 21 CI jobs passed and the
[macOS native job](https://github.com/J12003LPZ/davinci/actions/runs/35193764325/job/105112057975)
failed the existing ignore-change regression: inventory correctly removed ignored
files, but the refresh was labeled `observed` because the native notification had
not arrived. The scan now fingerprints the ignore contents it already reads;
changed, added, or removed ignore rules force full content reconciliation without
depending on event timing. The regression retains its `full` assertion and now
also verifies that an unaffected source is reread without being reparsed.

After this correction, 14 impact, 6 observer, 7 repository, and 1 overflow/backend
tests passed locally, along with the explicit after evaluation, formatting, and
affected all-target Clippy. CI subsequently passed on all three platforms at
`6b3aad9fd28b96f85c8115fe4099bea6e4557b17`, completing the P1 gate before P2 began.

## P2 baseline and implementation checkpoint

P2 starts from P1's verified head, on `codex/process-manager-01a0ad48` in the same
isolated worktree. Remote `main` was fetched and remains
`ca9fe69cd0da21bf161af25b2bed681748fb0d58`.

The [frozen Windows baseline](evidence/p2-process-baseline.json) uses the existing
`JobBook::register`, stdin, bounded output, and cancellation APIs with real
loopback Node servers. Two identical executable/argv requests created two jobs
and six processes including children/grandchildren. Readiness took 129.4012 ms
and 118.254 ms. Stdin round trip succeeded. A 5 MiB single-line burst was bounded
and marked dropped; line-boundary trimming retained only the final 36 bytes.
Cancellation closed all six fixture listeners in 783.5255 ms. In a separate host
process that exited without destructors, three fixture listeners remained alive
after 200 ms. Explicit fixture cleanup then closed all three. This is an observed
short-lived orphan window, not a claim about indefinite survival.

Executed: `rtk proxy cargo test -p davinci-agent --test process_manager_baseline --offline --locked -- --ignored --nocapture`
passed one explicit evaluation, with the artifact environment variable set to an
absolute output path. Formatting and baseline-target Clippy passed. An initial
fixture probe needed a socket-disconnect error handler; a relative artifact path
also failed because Cargo runs this test in the crate directory. Both harness
issues were corrected before recording the successful artifact.

The existing JobBook now has supervised records and explicit owner leases.
A private helper owns the OS process lifetime before spawning the requested
child: Windows Job Object, Unix session/group. Host pipe loss and final lease
release trigger cleanup; output stays in the existing bounded JobBook buffer.
Observed RED/GREEN includes the missing supervisor, immediate reuse of a stopped
lifetime, approval transfer, and an existing shell deny missed by process_start.

### P2 local completion gates

| Gate | Evidence |
| --- | --- |
| 1. Design | Approved by the user on 2026-09-17. |
| 2. Plan | Approved [P2 plan](02-process-manager.md), section-7 requirements and shared invariants. |
| 3. RED/GREEN | Observed failures before supervisor implementation, stopped-lifetime reuse correction, one-call approval transfer, existing shell-deny enforcement, logical wait across restart backoff, and the Windows npm verbatim-path correction. The corresponding regressions now pass. |
| 4. Affected package tests | `cargo test -p davinci-agent -p davinci-coding-agent --offline --locked -- --quiet`: exit 0, 3,194 passed, 16 ignored across 27 targets. Includes 877 agent unit tests, 1,014 coding-agent library tests and 1,221 binary tests. Ignored optional evaluations are not counted as executed. |
| 5. Format | `cargo fmt --check`: passed after formatting. |
| 6. Clippy | `cargo clippy --workspace --all-targets --offline --locked -- -D warnings`: passed. |
| 7. Integration | 15 supervisor test entries (12 substantive cases, 3 helper entries), 2 public process contract tests, and 2 packaged-binary entries passed. Covers descendant cleanup, held pipes, startup concurrency/cancellation, owner isolation, literal stdin, output bounds and private CLI entry before normal initialization. |
| 8. Security | Current policy on all six tools, cross-shell denies, exact one-call consent, changed/expired/revoked consent, hard contracts, role ceilings, cwd escape, environment injection, restart revocation and no one-call replay passed. Native Linux and macOS CI also passed the cwd symlink regression. |
| 9. Normal path | `process_manager_integration_tests`: 2 entries passed, including the normal `Agent::run_loop` fixture in both shared and nonshared executor modes. Covers enabled/disabled settings, deferred discovery, reuse, token-governor output plus `retrieve_output`, and shutdown. |
| 10. Graph | `graph_managed_process`: 5 tests passed. Includes authenticated multi-worker lease reuse/final release, role/mode/contract denial, composed native output retrieval, and an actual saved Graph controller dispatch with host provenance and worker-exit cleanup. |
| 11. Evaluation | One explicit ignored packaged lifecycle evaluation passed; [Windows after artifact](evidence/p2-process-after-windows.json) is compared with the unchanged [baseline](evidence/p2-process-baseline.json). Two requests reuse one server, stdin succeeds, 5 MiB output is bounded, and ordinary descendants disappear after shutdown/host loss. |
| 12. Docs | [Managed process guide](../../../process-manager.md), documentation index, plan and ledger updated. Lists limits, approval/restart behavior, port provenance and Unix group escape limitation. |
| 13. Diff review | Reviewed ownership/lock order, reservation cleanup, current authority, private helper protocol, environment, restart reconciliation, PID lifetime, session/Graph shutdown, bounded I/O, and test fixtures. Existing background shell APIs remain available; their model-facing endpoints reject managed IDs. Solo review per user instruction. |
| 14. CI | [PR #10](https://github.com/J12003LPZ/davinci/pull/10), head `bfc60e3f698ac30f9fb7500fc24490d028da2439`: [CI run 35247901869](https://github.com/J12003LPZ/davinci/actions/runs/35247901869) passed all 22 jobs; workflow lint 35247901761 and security interoperability 35247901735 passed. Windows/Linux/macOS each passed 11 process unit, 2 public contract, 15 supervisor, 5 Graph, 2 normal Agent, 2 packaged entry and 1 explicit lifecycle evaluation entries. [Platform evidence](evidence/p2-ci-platforms.json) preserves actual evaluation objects and test-result lines from each successful job. |

All shell commands were invoked through RTK. Package logs were summarized by
deterministically parsing Cargo's result lines; zero-test/doc targets are not
counted as test cases. Existing unrelated optional evaluations remained ignored.
The explicit after command was:

```text
rtk proxy cargo test -p davinci-coding-agent --test process_manager --offline --locked -- --ignored --nocapture
```

`DAVINCI_PROCESS_EVAL_ARTIFACT` was an absolute path to the linked JSON. A first
attempt used the default output cursor and therefore did not ask about discarded
bytes; the fixture now requests cursor zero. A later relative artifact path
failed after the behavior assertions; the recorded absolute-path run passed.
The frozen before measurement was not overwritten.

The after sample reports first readiness 190.1967 ms and reuse 1.4052 ms, one root
startup, three fixture processes, shutdown cleanup 351.9869 ms, and zero listener
survivors at the 200 ms host-loss observation. Cold startup is slower than the
old 129.4012 ms sample; repeated startup and ownership improve. Measurements are
single Windows debug runs with real Node processes, not provider-token results.

The Windows npm regression initially exited 1 because Node's module resolver
rejected the canonical verbatim script path. The adapter now removes that prefix
only after verifying the ordinary path resolves to the same canonical entry.
The installed npm `--version` invocation and full affected package tests passed
after the correction. No shell-string fallback or dependency install was added.

### P2 handoff surface

- `jobs/managed.rs` extends JobBook with scope/owner leases, bounded single-flight,
  cursor pages and logical lifetime metadata; `managed/restarts.rs` reconciles
  bounded restarts with continuing authority.
- `jobs/supervisor/{mod,wire,platform,helper,client}.rs` owns the private bounded
  protocol and OS lifetime. Windows uses a private Job Object; Unix uses a session
  group. These are lifecycle primitives, not a hostile-process sandbox.
- `process_manager/{command,schemas,tests}.rs` and `process_manager.rs` own the
  authorized adapter, direct argv/environment resolution and all six tools.
- `approval/dispatch.rs`, permission/risk, argv shell policy, tool dispatch and
  runtime capability metadata preserve approval and contract enforcement.
- CLI/settings/native shutdown and Graph's composed coordinator handler bind the
  same service. Worker lease identity comes from the host, never model arguments.
- Existing `SharedCounters`/`RunStats` expose actual startups/reuses/restarts.
  Live resources and output rings are not serialized into CacheRuntime.
- Tests include the reusable Node server fixture, frozen old-host baseline,
  packaged-binary evaluation, normal-session and Graph dispatch integrations.
- All fourteen P2 gates passed before P4 began. PR #10 remains open and unmerged.
  Next project is P4, using the approved transactional-edit plan. Platform
  evidence is recorded in the next branch to preserve the tested P2 head.

## P4 current checkpoint

The Windows short-name follow-up preserves aliases through edit/delete/recovery
and journals interrupted alias publication. Its 49 focused unit tests, 30
transaction integration/evaluation cases and the formerly failing normal-agent
CI test pass locally. Native exact-head CI remains required; P4 is still open.

Implementation is in draft [PR #11](https://github.com/J12003LPZ/davinci/pull/11)
on `codex/transactional-edits-01a0ad48` in the isolated worktree. The latest
checkpoint in the [P4 plan](04-transactional-edits.md) supersedes the historical
notes below. P4 remains incomplete; P3 has not started. The plan records executed
RED/GREEN checks and limitations. Ordinary mutations and explicit transaction
tools share durable provenance, current authority and conflict-safe recovery.
Source-bound command receipts and conservative Cargo target-root coverage are
connected to normal dispatch. Git commit observation is available through
`patch_status` with `observe_commit: true`, with current metadata authority and
exact committed-image checks. Normal previews capture an observed base revision
when current metadata policy allows it; otherwise the revision remains unknown.
Policies with read-deny rules conservatively prevent Git observation until its
internal metadata reads can be authorized individually.

The latest changes passed three Git observation integration tests, 56 permission
tests, 23 approval-focused tests, and all 35 turn tests. The latter include real
command verification and one-time transaction read approval. Agent library
Clippy with warnings denied passed. These targeted results do not close the
affected-package, Graph end-to-end, fault-injection, metadata, eval, or platform
CI gates. P3 has not started. The paragraphs below retain earlier checkpoint
evidence; the current status and latest P4 plan entry supersede their pending items.

The recovery follow-up adds atomic no-clobber active-marker publication and
cleanup after failed rollback preparation. Four storage fault-injection tests
passed across marker, apply and rollback journal failures. The 18-case transaction
integration suite passed after fixing cancellation during the final authority
check; an additional interrupted-rollback recovery test passed separately.
Agent-library Clippy passed. These results advance recovery coverage without
closing the remaining P4 platform, metadata, integration or evaluation gates.

Graph provenance follow-up: the real parent launcher exposed a child agent-ID
mismatch. The child now validates and retains the parent-assigned ID. The launcher
recovery fixture passed, as did the two-prompt write/rollback regression and all
four CLI transaction tests. The launcher fixture deliberately has no submitted
Graph artifact and is correctly rejected as incomplete; full successful Graph
lifecycle coverage remains open. See the P4 plan for the exact evidence scope.

The subsequent real-launcher test now covers successful edit plus `graph_submit`
using a bounded offline call sequence. Parent acceptance, exact artifact content,
transaction provenance, effect recording and authorized recovery passed. This
closes the missing successful worker lifecycle case; it does not claim a full
multi-node Graph run or completion of P4's metadata/package/CI gates.

Windows metadata follow-up now records/restores owner and primary group alongside
the DACL. Two metadata regressions and 29 transaction/commit/verification tests
passed, including non-default group recovery and refusal after group-only external
changes. Unix metadata behavior and the final package/review/CI gates remain open.

Unix owner/group and full permission-mode preservation are now implemented with
journal validation. Serialization and validation tests passed on Windows; two
Unix-only filesystem regressions await Linux/macOS execution. Unix extended
attributes/ACLs are still outstanding, so this is not a completed metadata gate.

Bounded Linux/macOS extended-attribute capture and restoration now feed the same
transaction images and stale-state checks. Local parsing/bounds tests and Clippy
passed; native attribute tests are added but unexecuted. macOS ACL handling and
native platform validation remain open before P4 can pass its metadata gate.

P4 foreground shell capture now rejects incomplete evidence on reader errors,
panics and stream overflow. Output retention is bounded at 16 MiB per stream;
overflow is drained and reported as failure before receipt creation. Four focused
capture tests, two receipt tests and four transaction-verification integration
tests passed locally, as did affected Clippy. Descendant-held pipe lifecycle,
macOS ACLs, native platform tests and the final P4 completion gates remain open.

The macOS ACL adapter is now implemented with bounded portable serialization,
descriptor-based restoration, journal validation and metadata-only conflict
detection. The format test and 29 existing transaction integration cases passed
on Windows, as did affected Clippy. Native macOS recovery/conflict tests are
added but unexecuted; native compilation/runtime evidence remains required.

The affected agent/coding-agent package gate passed 3,265 tests (21 ignored) on
Windows. A subsequent red/green fix now discovers durable transactions for
resumed sessions and agents without a runtime ledger. Actual Cargo verification
passes the eight-case success/failure/authorization matrix; two discovery tests
cover owner boundaries and bounded reads. P4 still requires native platform
validation, descendant-pipe lifecycle completion, final checks, review and CI.

## P3 implementation evidence

Implementation and exact-head CI are validated at `cdbfe00afdbd8c2839066ea63782f211e698be26`;
PR #12 updated for review. User-facing behavior and limits are documented in
[browser verification](../../browser-verification.md) and [P3 plan](03-browser-verification.md).

| Gate | Evidence/status |
| --- | --- |
| 1. Approved design | User approved the design and all twelve plans on 2026-09-17. |
| 2. Plan | [P3 ordered plan](03-browser-verification.md). |
| 3. RED/GREEN | Observed failing cases: Windows supervisor output queue exit race, graph coordinator tool selection without host, missing source observation recheck, RPC verify_browser route; all passing after implementation. |
| 4. Affected package tests | `rtk proxy cargo test --offline --locked -p davinci-agent -p davinci-coding-agent`: exit 0; 1241 passed in coding-agent lib, 0 failed, 15 ignored; all agent suites passed. |
| 5. Format | `rtk proxy cargo fmt -p davinci-agent -p davinci-coding-agent --check`: exit 0. |
| 6. Clippy | `rtk proxy cargo clippy --offline --locked -p davinci-agent -p davinci-coding-agent --all-targets -- -D warnings`: exit 0. |
| 7. Integration | Real Chromium tests passed with zero skips: `normal_browser_native_dispatch_actions_revocation_and_cleanup` (1 passed), `rpc_browser_verification_receipt_wire_exchange` (1 passed), `graph_browser_transport_shares_server_and_isolates_worker_contexts` (1 passed), `graph_scheduler_writer_uses_authenticated_browser_transport` (1 passed). 5 real Node tests passed; 33 deterministic Node tests passed. |
| 8. Security | Read authority verified via `check_current_source_read`; dedicated loopback proxy enforces origin, blocks redirects/foreign subresources/unauthorized WebSocket upgrades (0 forbidden hits); no CDP or arbitrary eval; no project node_modules resolution; immutable content-addressed screenshot artifacts; context isolation. |
| 9. Normal path | Real normal-session Chromium test passes complete frontend failure/fix flow without Graph; transactional edit to `index.html`, managed dev server, DOM/ARIA assertions, clean console/network, PNG retention, RealBrowser receipt. |
| 10. Graph | Actual `run_graph` scheduler Writer path, authenticated parent coordinator transport, context isolation, host-derived TransactionOwner, Phase::Done completion. |
| 11. Evaluation | [p3-browser-final.json](evidence/p3-browser-final.json) records fresh verification of all real browser flows, deterministic and real network confinement, and security invariants; earlier baselines and checkpoints preserved. |
| 12. Docs | User guide [docs/browser-verification.md](../../browser-verification.md), documentation index, and this plan updated. |
| 13. Review | Solo source and diff audit across all 12 touched crates files and docs; no subagents per user instruction. |
| 14. CI | Exact-head CI passed at `cdbfe00afdbd8c2839066ea63782f211e698be26`: PR CI [35310203206](https://github.com/J12003LPZ/davinci/actions/runs/35310203206), workflow lint [35310203230](https://github.com/J12003LPZ/davinci/actions/runs/35310203230), Security SARIF [35310203220](https://github.com/J12003LPZ/davinci/actions/runs/35310203220), push CI [35310198092](https://github.com/J12003LPZ/davinci/actions/runs/35310198092) and SARIF [35310198050](https://github.com/J12003LPZ/davinci/actions/runs/35310198050) all green. |

## P5 implementation evidence

Implementation is completed on `codex/package-intelligence-01a0ad48` in the isolated worktree.

| Gate | Evidence/status |
| --- | --- |
| 1. Approved design | User approved the design and all twelve plans on 2026-09-17; project section 10 and cross-cutting sections 18-40. |
| 2. Plan | [P5 ordered plan](05-package-intelligence.md). |
| 3. RED/GREEN | Observed failing test suite on initial implementation due to fixture expectations, missing kebab-case lock parsing, and unvalidated package traversal; all 10 integration tests passing after implementation. |
| 4. Affected package tests | `rtk proxy cargo test --offline --locked -p davinci-coding-agent --test package_intelligence`: exit 0 (10 passed); `rtk proxy cargo test --offline --locked -p davinci-agent`: exit 0 (73 passed, 3 ignored). |
| 5. Format | `rtk proxy cargo fmt -p davinci-agent -p davinci-coding-agent --check`: exit 0. |
| 6. Clippy | `rtk proxy cargo clippy --offline --locked -p davinci-agent -p davinci-coding-agent --all-targets -- -D warnings`: exit 0. |
| 7. Integration | 10 integration tests passed in `package_intelligence.rs`: npm v1, npm v3 with selective symbol resolution, version mismatch detection, pnpm workspace virtual store, yarn classic, yarn berry, conditional exports, path traversal & script non-execution, cache hit/miss telemetry, host registration. |
| 8. Security | All 5 package intelligence tools classified as `ToolClass::Read`. Traversal attempts in package name or workspace parameter rejected. Workspace isolation verified. Zero package lifecycle scripts executed. No recursive indexing of node_modules into RepoIntelligence. |
| 9. Normal path | Normal Agent session dispatch executes `package_info`, `package_exports`, `package_symbol`, `package_dependents`, `package_why` and `/package-status` command without Graph. |
| 10. Graph | Graph role allowlist updated: `Role::Researcher`, `Role::Planner`, `Role::Writer`, `Role::Reviewer`, and `Role::TestAnalyzer` authorized for all 5 tools; `Role::Classifier` denied (least privilege). Unit test `package_intelligence_tools_follow_role_selection` passed. |
| 11. Evaluation | [p5-package-final.json](evidence/p5-package-final.json) captures full verification evidence, supported formats, security invariants, and test results. |
| 12. Docs | Updated `05-package-intelligence.md` and `README.md` program ledger. |
| 13. Review | Solo source and diff audit across touched crates, fixtures, tests, and documentation. No subagents used per instruction. |
| 14. CI | Exact-head CI passed at `9a3fdf9b47216bcfc2ca58bef5d380d1a10c863a`: Push CI [35367111447](https://github.com/J12003LPZ/davinci/actions/runs/35367111447), Push SARIF [35367111099](https://github.com/J12003LPZ/davinci/actions/runs/35367111099), PR #13 CI [35367117486](https://github.com/J12003LPZ/davinci/actions/runs/35367117486), PR #13 SARIF [35367117517](https://github.com/J12003LPZ/davinci/actions/runs/35367117517) all 100% green across platform matrix. |

