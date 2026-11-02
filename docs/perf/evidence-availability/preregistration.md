# Evidence Availability Measurement — Pre-registration

**Status:** Phase D plan. No live run is authorized by this document.

## Question

Does protecting task-relevant evidence improve verified task success without an unacceptable token cost, compared with the current defaults?

## Arms

| Arm | Configuration |
| --- | --- |
| Control | Current defaults, `DAVINCI_PRUNE_PROFILE=default`. |
| Evidence | `DAVINCI_PRUNE_PROFILE=evidence` (Phase C) only for the pruning comparison. Phase A and Phase B behavior is shipped, not an arm; a before/after commit comparison is the only way to measure them. |

## Corpus

* The 17-task corpus from the Droid Stage 0 study: six `verified-core` Rust tasks, eight public bench tasks, three research fixtures.
* A task counts only when its grader has an independent oracle. The 90 invalid and 110 structural-only scenarios from the earlier corpus are excluded.
* The deterministic needle harness in `crates/davinci-coding-agent/tests/evidence_availability.rs` runs separately on every PR and is not part of the live denominator.

## Primary metric

**Needle survival and first recovery per layer.** The deterministic harness places a labeled error at Governor index 25 and a labeled failure at Code mode line 250, then asserts the first survives in the visible view and the second survives in the retained body with `omitted_start_line == 110`. It then performs the first recovery and asserts the needle is returned, recording a token estimate for that recovery.

Pass: every assertion passes. This is CI-runnable, deterministic, and requires no model call.

## Secondary metrics

* Verified task success, paired by task.
* Total input and uncached input tokens.
* Compaction and fold counts, and summarizer tokens.
* First-attempt recovery success: the first `read` or `retrieve_output` after an omission returns the needle.
* Tokens per resolved recovery.
* Prefix breaks by cause.

## Design

* Paired within task: control and evidence run on the same task, model, and effort, in counterbalanced order.
* Repetitions: three per task for the 17-task corpus. Report per-task ratios, the geometric mean, and a 90% bootstrap interval over tasks.
* Pass rule: the deterministic harness passes 100%; verified task success is not lower than control; the interval for tokens per verified success is not worse than +10%. Report uncached input and compactions; do not promote on a token reduction alone.
* Power limit: 17 tasks cannot resolve a 3 percentage-point success difference. A result is directional until a larger independently authored corpus exists.
* Stop rules: stop on provider or authentication failure; stop when the usage window is exhausted; an infrastructure failure is not a result.

## Rollback

`DAVINCI_PRUNE_PROFILE` returns to `default`. Phase A and Phase B are code reverts. No session schema change is involved.

## Current state

* Phase A and Phase B: shipped, default on.
* Phase C: opt-in `evidence` profile, not promoted.
* Phase D: this pre-registration; live runs require separate approval.
