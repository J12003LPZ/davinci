# Harness gap diagnosis

Execution date: 2026-10-12. Status: diagnosis complete; offline corrections verified.

## Scope and baseline

This package executes the Desktop harness-improvement plan. The reference is
its frozen G1–G6 rubric, not a promise to match another harness's cache share.
The task checkout is `harness-gap-diagnosis`, product source `eaaba04d`.
The benchmark product source `f9123d153fcf83e15b599576c27faa69e35ea72f`
differs only in three benchmark-runner files. Runner and campaign identity
remain separate from source identity.

The user's pre-existing `AGENTS.md` edit is out of scope. There is one source
writer, with independent read-only investigation and review. No live provider
tests, installs, installed-binary replacement, commits, pushes, or deployment
are authorized by this execution.

## Existing ownership

This is an evidence and regression package, not a competing architecture:

- [Cache identity and privacy contract](../../cache/openai-cache-contract.md)
- [Previous cache diagnosis](../openai-cache-diagnosis.md), historical
  sessionless-transport work, not a new proposed correction
- [Cache runtime design](../../superpowers/specs/2026-09-17-cache-runtime.md)
- [Harness optimization program](../../superpowers/plans/2026-09-14-davinci-harness-optimization-master.md)
- [Verification Planner](../../superpowers/plans/2026-09-17-engineering-program/10-verification-planner.md)

Keep Token Governor, Code Mode, Context VM, the existing verification engine,
and operation journals. Do not rewrite durable native history or add another
retry/completion system.

## Frozen review rubric

| Gate | Evidence required |
|---|---|
| G1 | Separate serialized continuity from actual provider cache reuse; explicitly retain unknown backend causes |
| G2 | Classify every request in a successful matched trajectory, reference source events, inspect additional traces before generalizing |
| G3 | Reproduce recovery defects; distinguish interruption and malformed calls; respect journal, cancellation and revocation; count actual side effects |
| G4 | Compare complete prepared Off/Hybrid requests before compaction without normalizing meaningful state |
| G5 | Diagnostics reveal only hashes, sizes, safe tags and counters; deterministic output, absent-field handling and request noninterference |
| G6 | Run affected offline suites, fault scenarios, formatting and Clippy; retain failures and skips |

Each implemented correction must fail its regression before the fix and pass
afterward. Independent review receives the deliverable and this rubric, not
the writer's justification.

## Completion levels

- `DIAGNOSIS_COMPLETE`: complete for the supplied captures, traces and frozen
  benchmark artifacts; unknown backend causes and historical t6 raw frames
  remain explicit limitations.
- `CORRECTION_VERIFIED`: offline regressions and affected suites pass; the
  final source-only safety review accepted terminal status arbitration.
- `EFFICIENCY_IMPROVEMENT_DEMONSTRATED`: not claimed. Controlled live
  measurement requires separate authorization.

Local JSON continuity, hashes, offline tests and fewer emitted bytes cannot
establish real token, subscription, success-rate or speed improvements.
Missing usage remains unknown, never zero.

## Deliverables

- [Architecture and safety boundaries](architecture.md)
- [Cache evidence and unknown backend causes](cache.md)
- [Every available paired request and additional trace](trajectories.md)
- [Recovery incident and eleven fault boundaries](recovery.md)
- [Benchmark integrity and ranked proposals](benchmark-integrity.md)
- [Feature evaluation specifications](evaluations.md)
- [Per-candidate decisions](candidates.json)
- [Validation, failures and skips](validation.md)

Candidate measurement fields distinguish observed saved campaign evidence from
corrected-treatment measurement. Historical usage/grades marked available in a
candidate do not mean a new live run occurred or an improvement was measured.
