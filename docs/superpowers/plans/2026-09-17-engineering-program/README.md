# Engineering program ledger

Status: P1–P12 and the P12 live 17-step follow-up are merged to `origin/main` `5585fd6` (PR #19).
Main CI on that SHA: https://github.com/J12003LPZ/davinci/actions/runs/35420073891 success (attempt 2 after cancelling a hung Windows step); SARIF https://github.com/J12003LPZ/davinci/actions/runs/35420073914 success.
P7 head `950ab50`, P6 head `dcaf02b` and P9 head `726431e` passed CI, including Windows/Linux/macOS native matrix,
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
| 1 | [P1 Test Impact Intelligence](01-test-impact.md) | Complete; PR #9 merged | Approved | Package tests, fmt, Clippy, integration, security and eval passed | Green at `6b3aad9`; on main `5585fd6` |
| 2 | [P2 Persistent Process Manager](02-process-manager.md) | Complete; PR #10 merged | Approved | Package tests, fmt, Clippy, integration, security and eval passed | Green at `bfc60e3`; on main `5585fd6` |
| 3 | [P4 Transactional Edit Engine](04-transactional-edits.md) | Complete; PR #11 merged | Approved | Package, integration, security, concurrency, normal/Graph and eval gates passed | Green at `a2054d3`; on main `5585fd6` |
| 4 | [P3 Browser / Playwright Verification](03-browser-verification.md) | Complete; PR #12 merged | Approved | Package tests, fmt, Clippy, real Chromium normal/Graph paths, network confinement, RPC verification and eval passed | Green at `9f1adb3`; on main `5585fd6` |
| 5 | [P5 Package / Dependency Intelligence](05-package-intelligence.md) | Complete; PR #13 merged | Approved | Package tests, fmt, Clippy, integration, security and eval passed | Green at `9a3fdf9`; on main `5585fd6` |
| 6 | [P7 Build Intelligence](07-build-intelligence.md) | Complete; PR #14 merged | Approved | Package tests, fmt, Clippy, integration, security and eval passed | Green at `950ab50`; on main `5585fd6` |
| 7 | [P6 Git Intelligence](06-git-intelligence.md) | Complete; PR #15 merged | Approved | Package tests, fmt, Clippy, integration, security and eval passed | Green at `dcaf02b`; on main `5585fd6` |
| 8 | [P9 Change Impact Engine](09-change-impact.md) | Complete; PR #16 merged | Approved | Package tests, fmt, Clippy, integration, security and eval passed | Green at `d34e2c4`; on main `5585fd6` |
| 9 | [P8 Deterministic Hook / Policy Engine](08-hook-policy.md) | Complete; PR #17 merged | Approved | Package tests, fmt, Clippy, integration, security and eval passed | Green at `d0f8c49`; on main `5585fd6` |
| 10 | [P10 Verification Planner](10-verification-planner.md) | Complete; PR #18 merged | Approved | Focused 5 planner + 1 eval tests passed; Graph role allow/deny passed | Green exact-head `109c78c`; on main `5585fd6` |
| 11 | [P11 Workspace Snapshot / Safe Sandbox Layer](11-workspace-snapshot.md) | Complete; PR #18 merged | Approved | Focused 6 snapshot + 1 eval tests passed; Writer-only restore Graph deny passed | Green exact-head `109c78c`; on main `5585fd6` |
| 12 | [P12 Continuous Agent Evaluation Framework](12-continuous-evals.md) | Complete; PR #19 merged | Approved | Engineering 11/11; two Chromium 17-step runs passed; classifier deny passed | Green exact-head `9c27b49`; on main `5585fd6` |

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

P1–P12 are merged. Final program report: [engineering-program-verification.md](../../../engineering-program-verification.md).
Main CI on `5585fd6` is green: https://github.com/J12003LPZ/davinci/actions/runs/35420073891. No production restart.

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

## P7 implementation evidence

Implementation is completed on `codex/build-intelligence-01a0ad48` in the isolated worktree.

| Gate | Evidence/status |
| --- | --- |
| 1. Approved design | User approved the design and all twelve plans on 2026-09-17; project section 12 and cross-cutting sections 18-40. |
| 2. Plan | [P7 ordered plan](07-build-intelligence.md). |
| 3. RED/GREEN | Initial compilation and fixture tests failed; all 12 integration tests passing after implementation. |
| 4. Affected package tests | `rtk proxy cargo test --offline --locked -p davinci-coding-agent --test build_intelligence`: exit 0 (12 passed); `rtk proxy cargo test --offline --locked -p davinci-agent`: exit 0 (73 passed, 3 ignored). |
| 5. Format | `rtk proxy cargo fmt -p davinci-agent -p davinci-coding-agent --check`: exit 0. |
| 6. Clippy | `rtk proxy cargo clippy --offline --locked -p davinci-agent -p davinci-coding-agent --all-targets -- -D warnings`: exit 0. |
| 7. Integration | 12 integration tests passed in `build_intelligence.rs`: workspace package discovery, Turborepo target pipeline, Nx target defaults & dependsOn, TypeScript composite project references, non-omission downstream reverse dependency traversal, deterministic build command synthesis for Turbo/Nx/pnpm, cyclic dependencies, custom tasks, Vite/Next framework detection, path traversal rejection, CacheRuntime caching & telemetry, NativeExtensionHost registration and permission classification. |
| 8. Security | All 5 build intelligence tools classified as `ToolClass::Read`. Traversal attempts in package, scope, or file paths rejected. Zero discovery execution: manifests and config files inspected declaratively with no compiler or package code execution. Project-native caches preserved without modification or clearing. |
| 9. Normal path | Normal Agent session dispatch executes `workspace_packages`, `build_targets`, `build_dependencies`, `build_affected`, `build_command` and `/build-status` command without Graph. |
| 10. Graph | Graph role allowlist updated: `Role::Researcher`, `Role::Planner`, `Role::Writer`, `Role::Reviewer`, and `Role::TestAnalyzer` authorized for all 5 tools; `Role::Classifier` denied (least privilege). Unit test `build_intelligence_tools_follow_role_selection` passed. |
| 11. Evaluation | [p7-build-final.json](evidence/p7-build-final.json) captures full verification evidence, supported runners, security invariants, and test results. |
| 12. Docs | Updated `07-build-intelligence.md` and `README.md` program ledger. |
| 13. Review | Solo source and diff audit across touched crates, fixtures, tests, and documentation. No subagents used per instruction. |
| 14. CI | Green on head `950ab505d87c3569b693ca21085d2e269080551b` and PR #14 ([PR #14](https://github.com/J12003LPZ/davinci/pull/14)). Push CI: [run 35370012630](https://github.com/J12003LPZ/davinci/actions/runs/35370012630) (22/22 jobs success); Push SARIF: [run 35370012618](https://github.com/J12003LPZ/davinci/actions/runs/35370012618); PR CI: [run 35370058879](https://github.com/J12003LPZ/davinci/actions/runs/35370058879) (22/22 jobs success); PR SARIF: [run 35370058953](https://github.com/J12003LPZ/davinci/actions/runs/35370058953). |

## P6 implementation evidence

Implementation is completed on `codex/git-intelligence-01a0ad48` in the isolated worktree.

| Gate | Evidence/status |
| --- | --- |
| 1. Approved design | User approved the design and all twelve plans on 2026-09-17; project section 11 and cross-cutting sections 18-40. |
| 2. Plan | [P6 ordered plan](06-git-intelligence.md). |
| 3. RED/GREEN | Initial implementation had unhandled root commits in `diff-tree`, missing shallow repository format checks, and unmerged stage tab-separation in index records; all 11 integration tests passing after implementation. |
| 4. Affected package tests | `rtk proxy cargo test --offline --locked -p davinci-coding-agent --test git_intelligence`: exit 0 (11 passed); `rtk proxy cargo test --offline --locked -p davinci-agent`: exit 0 (73 passed, 3 ignored). |
| 5. Format | `rtk proxy cargo fmt -p davinci-agent -p davinci-coding-agent --check`: exit 0. |
| 6. Clippy | `rtk proxy cargo clippy --offline --locked -p davinci-agent -p davinci-coding-agent --all-targets -- -D warnings`: exit 0. |
| 7. Integration | 11 integration tests passed in `git_intelligence.rs`: symbol history with renames/edits, shallow & bounded acceptance guard, facts vs inference separation, changed symbols AST comparison, branch diff with merge-base, porcelain blame, conflict explain with zero mutation, option injection & path traversal security guards, non-git directory handling, caching & telemetry, permission classification and host registration. |
| 8. Security | All 7 git intelligence tools classified as `ToolClass::Read`. Option injection guards (`--end-of-options`), path traversal prevention, sanitized environment (`GIT_CONFIG_NOSYSTEM=1`, `GIT_CONFIG_GLOBAL=/dev/null`, pager disabled, zero shell interpolation), zero mutation guarantee on unmerged 3-way stages, acceptance guard reporting unknown/partial on incomplete history. |
| 9. Normal path | Normal Agent session dispatch executes `git_symbol_history`, `git_related_commits`, `git_changed_symbols`, `git_branch_diff`, `git_blame_symbol`, `git_commit_context`, `git_conflict_explain` and `/git-status` command without Graph. |
| 10. Graph | Graph role allowlist updated: `Role::Historian`, `Role::Researcher`, `Role::Reviewer`, `Role::Planner`, `Role::Writer`, `Role::TestAnalyzer` authorized; `Role::Classifier` denied (least privilege). Unit test `git_intelligence_tools_follow_role_selection` passed. |
| 11. Evaluation | [p6-git-final.json](evidence/p6-git-final.json) captures full verification evidence, supported capabilities, security invariants, and test results. |
| 12. Docs | Updated `06-git-intelligence.md` and `README.md` program ledger. |
| 13. Review | Solo source and diff audit across touched crates, fixtures, tests, and documentation. No subagents used per instruction. |
| 14. CI | Green on head `dcaf02b5ad366ef0c48ac0017684bb83f2ec2c9d` and PR #15 ([PR #15](https://github.com/J12003LPZ/davinci/pull/15)). Push CI: [run 35372343120](https://github.com/J12003LPZ/davinci/actions/runs/35372343120) (22/22 jobs success); Push SARIF: [run 35372343125](https://github.com/J12003LPZ/davinci/actions/runs/35372343125); PR CI: [run 35372351676](https://github.com/J12003LPZ/davinci/actions/runs/35372351676) (22/22 jobs success); PR SARIF: [run 35372351664](https://github.com/J12003LPZ/davinci/actions/runs/35372351664). |

## P8 implementation evidence

Implementation is completed on `codex/hook-policy-01a0ad48` in the isolated worktree.

| Gate | Evidence/status |
| --- | --- |
| 1. Approved design | User approved the design and all twelve plans on 2026-09-17; project section 13 and cross-cutting sections 18-40. |
| 2. Plan | [P8 ordered plan](08-hook-policy.md). |
| 3. RED/GREEN | Initial implementation uncovered stdin truncation causing json parse errors on payloads, missing depth guard debug trait, and argument lifetime constraints in `HooksRuntimeSubscriber`; all 11 integration tests passing after implementation. |
| 4. Affected package tests | `rtk proxy cargo test --offline --locked -p davinci-coding-agent --test hook_policy`: exit 0 (11 passed); `rtk proxy cargo test --offline --locked -p davinci-agent`: exit 0 (73 passed, 3 ignored); `rtk proxy cargo test --offline --locked -p davinci-coding-agent --lib`: exit 0 (1045 passed, 15 ignored). |
| 5. Format | `rtk proxy cargo fmt -p davinci-agent -p davinci-coding-agent --check`: exit 0. |
| 6. Clippy | `rtk proxy cargo clippy --offline --locked -p davinci-agent -p davinci-coding-agent --all-targets -- -D warnings`: exit 0. |
| 7. Integration | 11 integration tests passed in `hook_policy.rs`: legacy hook backward compatibility, policy rule filtering by event/tool/path globs, failure policies (warn, block, ignore), untrusted project hook rejection, post-load file modification invalidation, BeforeWrite decision blocking file mutation, AfterWrite observer failure tracking unmet completion requirements without retroactive file reversal, BeforeProcessStart blocking tool execution, recursion depth guard limiting depth, bounded JSON streams and timeout process termination, aggregate diagnostics via `/hook-status`. |
| 8. Security | Binding trust to resolved path and SHA-256 content identity. Revalidated upon execution; disk changes invalidate trust immediately (fail closed). Native/model tools prohibited from granting trust or injecting hook definitions. Supervised process execution with bounded streams (64 KiB) and descendant process tree kill. Recursion depth capped at 3. |
| 9. Normal path | Normal Agent session dispatch executes hooks subscribed via `HooksRuntimeSubscriber` on runtime bus events (decision and observer), enforces failure policies, and runs `/hook-status` diagnostics command without Graph. |
| 10. Graph | Graph workers operate under host-bound policy constraints; workers cannot expand or inject hook policies. |
| 11. Evaluation | [p8-hook-final.json](evidence/p8-hook-final.json) captures full verification evidence, decision/observer events, security invariants, and test results. |
| 12. Docs | Updated `08-hook-policy.md` and `README.md` program ledger. |
| 13. Review | Solo source and diff audit across touched crates, models, tools, and tests. No subagents used per instruction. |
| 14. CI | Green on head `cbef3fa4d1dc7fc52b210f2c4161a067a99653dc` and PR #17 ([PR #17](https://github.com/J12003LPZ/davinci/pull/17)). Push CI: [run 35414060812](https://github.com/J12003LPZ/davinci/actions/runs/35414060812) (22/22 jobs success); Push SARIF: [run 35414060714](https://github.com/J12003LPZ/davinci/actions/runs/35414060714); PR CI: [run 35414088579](https://github.com/J12003LPZ/davinci/actions/runs/35414088579) (22/22 jobs success); PR SARIF: [run 35414088573](https://github.com/J12003LPZ/davinci/actions/runs/35414088573). |

## P9 implementation evidence

Implementation is completed on `codex/change-impact-01a0ad48` and merged via PR #16.

| Gate | Evidence/status |
| --- | --- |
| 1. Approved design | User approved the design and all twelve plans on 2026-09-17. |
| 2. Plan | [P9 ordered plan](09-change-impact.md). |
| 3. RED/GREEN | Targeted change-impact suites failed first on `files` vs `path` argument handling and incomplete analysis; then passed. |
| 4. Affected package tests | `cargo test --offline --locked -p davinci-coding-agent --test change_impact`: focused integration passed. |
| 5. Format | `cargo fmt --check` on affected crates: exit 0. |
| 6. Clippy | `cargo clippy --offline --locked -p davinci-coding-agent --all-targets -- -D warnings`: exit 0. |
| 7. Integration | Impact analyzer composes AST, LSP, git, package, build, test, transaction, and workspace evidence. |
| 8. Security | Read-class tools; Classifier denied. |
| 9. Normal path | `impact_analyze` dispatched from the live 17-step normal session. |
| 10. Graph | Role allowlist; Classifier deny covered by host `before_tool`. |
| 11. Evaluation | [p9-impact-final.json](evidence/p9-impact-final.json). |
| 12. Docs | Plan and ledger. |
| 13. Review | Solo diff review. |
| 14. CI | PR #16 CI [35411835669](https://github.com/J12003LPZ/davinci/actions/runs/35411835669) success at `d34e2c4`. |

## P10 implementation evidence

Implementation is completed on `codex/p10-p12-01a0ad48` and merged via PR #18.

| Gate | Evidence/status |
| --- | --- |
| 1. Approved design | User approved the design and all twelve plans on 2026-09-17. |
| 2. Plan | [P10 ordered plan](10-verification-planner.md). |
| 3. RED/GREEN | Planner tests failed first on missing gates/failure handling; then 5 planner + 1 eval passed. |
| 4. Affected package tests | `cargo test --offline --locked -p davinci-coding-agent --test verification_planner --test verification_planner_eval`: 6 passed. |
| 5. Format | `cargo fmt --check -p davinci-coding-agent -p davinci-agent -p davinci-evals`: exit 0. |
| 6. Clippy | `cargo clippy --offline --locked -p davinci-coding-agent -p davinci-agent -p davinci-evals --all-targets -- -D warnings`: exit 0. |
| 7. Integration | Planner outputs, verification gates, failure handling, permissions, settings, telemetry, caching, concurrency, bounded execution. |
| 8. Security | Read class; Classifier denied `verification_plan`. |
| 9. Normal path | `engineering_login_workflow` and live 17-step dispatch call `verification_plan`. |
| 10. Graph | Writer allow / Classifier deny. |
| 11. Evaluation | [p10-verification-planner-local.json](evidence/p10-verification-planner-local.json) and live 17-step evidence. |
| 12. Docs | Plan, ledger, [final report](../../../engineering-program-verification.md). |
| 13. Review | Solo diff review. |
| 14. CI | PR #18 CI [35417186830](https://github.com/J12003LPZ/davinci/actions/runs/35417186830) success. |

## P11 implementation evidence

Implementation is completed on `codex/p10-p12-01a0ad48` and merged via PR #18.

| Gate | Evidence/status |
| --- | --- |
| 1. Approved design | User approved the design and all twelve plans on 2026-09-17. |
| 2. Plan | [P11 ordered plan](11-workspace-snapshot.md). |
| 3. RED/GREEN | Snapshot tests failed first on restore/isolation; then 6 snapshot + 1 eval passed. |
| 4. Affected package tests | `cargo test --offline --locked -p davinci-coding-agent --test workspace_snapshot --test workspace_snapshot_eval`: 7 passed. |
| 5. Format | Same as P10. |
| 6. Clippy | Same as P10. |
| 7. Integration | Checkpoint, diff, restore, isolation, bounded output, failure recovery. |
| 8. Security | Writer-only restore; Classifier denied `workspace_restore`. |
| 9. Normal path | Live 17-step calls `workspace_checkpoint` (measured 51–55 ms). |
| 10. Graph | Writer allow / Classifier deny restore. |
| 11. Evaluation | [p11-workspace-snapshot-local.json](evidence/p11-workspace-snapshot-local.json). |
| 12. Docs | Plan, ledger, final report. |
| 13. Review | Solo diff review. |
| 14. CI | Same PR #18 green run as P10. |

## P12 implementation evidence

Live 17-step follow-up is `9c27b4929b7700ce7d1ecbff976a832b78423e7b`, merged by PR #19 into `5585fd6`.

| Gate | Evidence/status |
| --- | --- |
| 1. Approved design | User approved the design and all twelve plans on 2026-09-17. |
| 2. Plan | [P12 ordered plan](12-continuous-evals.md). |
| 3. RED/GREEN | Engineering eval mapping missed edit/browser_snapshot/verification_plan; then 11/11 passed. Live test failed compile on unused output and double mut borrow; then two ignored Chromium runs passed. |
| 4. Affected package tests | `cargo test --offline --locked -p davinci-evals --test engineering`: 11 passed. davinci-evals and davinci-coding-agent workspace-test jobs on `5585fd6` succeeded. |
| 5. Format | `cargo fmt --check -p davinci-evals -p davinci-coding-agent`: exit 0. |
| 6. Clippy | `cargo clippy --offline --locked -p davinci-evals -p davinci-coding-agent --all-targets -- -D warnings`: exit 0. |
| 7. Integration | Live test `login_button_seventeen_step_normal_dispatch_and_graph_deny` dispatched all 17 required tools with real `process_start` and Chromium `browser_open`/`click`/`snapshot`/`console`/`network`. |
| 8. Security | Classifier deny of `verification_plan` and `workspace_restore`. ACLs/permissions were not weakened. |
| 9. Normal path | Two consecutive local Chromium runs, 4.72s and 4.84s. |
| 10. Graph | Selective Graph deny recorded in the same test. |
| 11. Evaluation | [p12-live-login.json](evidence/p12-live-login.json). Metrics: process_start 102–112 ms, browser_open 383–425 ms, impact cold 58–60 ms / warm 32–35 ms. `startup_ms` and `memory_bytes` are null with reason. |
| 12. Docs | This ledger and [final report](../../../engineering-program-verification.md). |
| 13. Review | Solo diff review. |
| 14. CI | PR #19 CI [35419563201](https://github.com/J12003LPZ/davinci/actions/runs/35419563201) success; push CI [35419553200](https://github.com/J12003LPZ/davinci/actions/runs/35419553200) success; SARIF [35419563181](https://github.com/J12003LPZ/davinci/actions/runs/35419563181) and [35419553243](https://github.com/J12003LPZ/davinci/actions/runs/35419553243) success. Main CI on `5585fd6`: [35420073891](https://github.com/J12003LPZ/davinci/actions/runs/35420073891) success (attempt 2); SARIF [35420073914](https://github.com/J12003LPZ/davinci/actions/runs/35420073914) success. |
