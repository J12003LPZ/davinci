# P12: Continuous Agent Evaluation Framework

Status: design approved by the user on 2026-09-17; implementation pending.
Execution sequence: 12 of 12.
Dependencies: All previous milestones green; existing davinci-evals behavior/trace/scorer/gate/artifacts.
Requirements authority: project section 17 and cross-cutting sections 18-40;
project numbers are kept stable while execution order follows the program design.

## Outcome

Demonstrate actual correctness, efficiency and reliability changes with repeatable feature-off/on evaluations and the full non-Graph acceptance scenario.

## Scope and ownership

Proposed files/modules (not claims of completed edits):

- crates/davinci-evals/src/behavior/ and new engineering scenario/metric extensions
- crates/davinci-evals/fixtures/ engineering TS/JS local repositories and scripted transport traces
- coding-agent normal-session/native/Graph integration tests and real-browser fixture
- Existing CI workflows only for missing offline/platform/browser gates
- docs/engineering-program-verification.md (final evidence report) and program ledger

The main session is the only writer. No subagents, parallel writers or unrelated cleanup.
Reinspect these paths and callers at implementation time because main may advance.

## Baseline before implementation

Reuse recorded pre-subsystem baselines, and rerun the same fixtures with each flag disabled/enabled on the final implementation. Freeze corpus/source/toolchain/command identities and distinguish actual versus estimated/missing metrics.

## Ordered implementation and RED/GREEN work

1. RED: require scenario coverage for repo navigation, symbol finding, TS debugging, safe refactor, installed dependency API, frontend bug, test selection, browser, Git context, transactions, long-running task and normal versus Graph behavior.

2. Extend existing scenario/trace schemas and metrics: task/verification success, incorrect completion claims, calls/reads/bytes, input/output tokens, cache hits, latency, test count/runtime, process/LSP startups, browser success, rollback correctness and CI outcome.

3. Add independent feature flags to compare baseline/capability enabled on identical fixtures. Store actual raw receipts/metrics and source/toolchain IDs; missing data is null with reason, never zero or invented success.

4. Run deterministic offline CI fixtures through actual Agent normal dispatch with replay/scripted model transport and real filesystem/process/test operations. Label this integration correctness, not general live-model coding ability.

5. Run realistic login-button scenario: repo map, LSP symbols, relevant package API/Git context, impact, reused dev server, transaction/edit, updated diagnostics, impacted tests, type/build checks, real browser/login/console/network, transaction verification and evidence-backed completion. No Graph requirement.

6. Run the same native tools with selective Graph role assignments and parent resource reuse, including denied roles. Prove no duplicated Graph-only implementations.

7. Measure startup/memory/activation, cold/warm queries, process/browser startup/count, cache/index reuse and full metrics. Use repeated runs and report distributions; distinguish estimated tokens from provider-observed tokens. Demonstrate required gains without hiding regressions.

8. Add planted failures for stale writes, outdated package API, incorrect test omission, console/network fault and false completion. Score false positives/negatives, rollback correctness and missing evidence.

9. Keep live network/model-provider evals separately opt-in. Reuse existing local model route when authorized; do not require live paid services in normal CI. Missing real browser is a remaining acceptance gate, not a fixture substitute.

10. Run final required CI-equivalent workspace/contract checks once across final assembled tree, plus platform/real-browser lanes. Produce full final report and update every requirement's evidence status before claiming complete.

## Required targeted validation

- All twelve categories and all seventeen global workflow steps have executable coverage and failure assertions.
- Feature-disabled and unavailable-dependency cases preserve ordinary coding; each capability benefits non-Graph sessions.
- Metrics cannot count recommended tests as executed tests or fixture receipts as real browser/CI success.
- Continuous CI gate catches planted regressions; final tree/commit matches recorded tests and CI, without relying on earlier PR status.

## Settings and fallback

Reuse subsystem flags; evaluation mode/corpus/seed/run count are explicit. No hidden background live-provider evaluation.

## Specific acceptance guard

Every section 39 deliverable and section 37/38 acceptance item needs direct evidence. The goal remains active if any required real path, platform gate or metric is missing.

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
