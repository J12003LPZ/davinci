# Engineering program ledger

Status: design package approved; P1 implementation in progress.
Goal scope: all twelve projects and the global acceptance/performance/report gates.
Reference: [program design](../../specs/2026-09-17-engineering-program.md) and
[unchanged supplied requirements](../../specs/2026-09-17-engineering-program-requirements.md).

## Authoritative workspace checkpoint

- Date: 2026-09-17.
- Fetched remote main: `ca9fe69cd0da21bf161af25b2bed681748fb0d58`.
- Branch: `codex/engineering-program-01a0ad48`.
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
| 1 | [P1 Test Impact Intelligence](01-test-impact.md) | Local implementation validated | Approved | Package tests, fmt, Clippy, integration, security and eval passed | PR #9; retry fix awaiting CI |
| 2 | [P2 Persistent Process Manager](02-process-manager.md) | Planned | Approved | Not run | Not pushed |
| 3 | [P4 Transactional Edit Engine](04-transactional-edits.md) | Planned | Approved | Not run | Not pushed |
| 4 | [P3 Browser / Playwright Verification](03-browser-verification.md) | Planned | Approved | Not run | Not pushed |
| 5 | [P5 Package / Dependency Intelligence](05-package-intelligence.md) | Planned | Approved | Not run | Not pushed |
| 6 | [P7 Build Intelligence](07-build-intelligence.md) | Planned | Approved | Not run | Not pushed |
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

Verify P1's exact-head CI in PR #9 before starting P2.
Approval is recorded above; routine implementation decisions need no new approval.
Keep the full program goal active until all global acceptance is verified.

Targeted validation of the baseline fixture passed: `rtk cargo fmt --check` and
`rtk proxy cargo clippy -p davinci-coding-agent --test test_impact_baseline --offline --locked -- -D warnings`.
Headroom was also invoked on the compact plan data; that call returned 0 tokens
saved (`router:noop`). No compression savings are claimed for that call.

## P1 implementation evidence

Implementation is locally validated; the exact-head CI gate remains pending.
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
| 14. CI | [PR #9](https://github.com/J12003LPZ/davinci/pull/9) is published. Initial CI found the retry reparse defect below. Exact-head success remains pending; the native matrix covers Windows, Linux, and macOS. |

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
