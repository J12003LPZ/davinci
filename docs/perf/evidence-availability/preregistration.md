# Evidence Availability Measurement — Pre-registration

**Status:** Phase D plan. No live run is authorized by this document.

## Question

Does protecting task-relevant evidence improve verified task success without an unacceptable token cost, compared with the current defaults?

## Arms

| Arm | Configuration |
| --- | --- |
| Control | Production route default: no `DAVINCI_PRUNE_PROFILE`. Cache-sensitive routes use the 65%/35% policy; other routes use 50%/35%. |
| Evidence | `DAVINCI_PRUNE_PROFILE=evidence` on the same route. The profile preserves the route's start and target fractions and changes only the eligible-result order. |

`DAVINCI_PRUNE_PROFILE=default` forces the 50% start even on a cache-sensitive route. It is not the production control and must not be used as one.

## Corpus and trigger coverage

* The 17-task corpus from the Droid Stage 0 study: six `verified-core` Rust tasks, eight public bench tasks, three research fixtures.
* That corpus recorded zero pruning events. **A pruning comparison is admissible only after adding at least three long-context tasks that cross the trigger in both arms.** Each arm must record `prunedResults > 0` on at least one task. Trigger coverage is a prerequisite; if an arm records zero prunes, the pruning comparison is void and only the deterministic harness is reported.
* A task counts only when its grader has an independent oracle. The 90 invalid and 110 structural-only scenarios from the earlier corpus are excluded.
* The deterministic needle harness in `crates/davinci-coding-agent/tests/evidence_availability.rs` runs separately on every PR and is not part of the live denominator.

## Primary metric

**Needle survival and first recovery per layer.** The deterministic harness places a labeled error at Governor index 25 and a labeled failure at Code mode line 250, then asserts the first survives in the visible view and the second survives in the retained body with `omitted_start_line == 110`. It then performs the first recovery and asserts the needle is returned, recording a token estimate for that recovery. In `evidence_availability.rs` the Governor recovery goes through `retrieve`, and the Code mode recovery slices the retained body in memory from the reported line. The `read` tool path is proved separately by `codemode_retained_body_is_recoverable_through_the_read_tool` in `davinci-agent`, which reads the stored body at the marker's offset.

Pass: every assertion passes. This is CI-runnable, deterministic, and requires no model call.

## Diagnostic admission budget

The Governor's diagnostic-preserving view is chosen over the generic digest when it is still a reduction. Record three byte counts for each task: diagnostic view, generic digest, and original content.

If `diagnostic_bytes / original_bytes > 0.90` on a task, flag it for review. Do not use a flagged task to promote a compression default, even if its success result is positive. The deterministic `diagnostic_view_byte_ratio_is_measured` test records the same ratio on every PR.

## Secondary metrics

* Verified task success, paired by task.
* Total input and uncached input tokens.
* Compaction and fold counts, and summarizer tokens.
* First-attempt recovery success: the first `read` or `retrieve_output` after an omission returns the needle.
* Tokens per resolved recovery.
* Prefix breaks by cause.

## Design

* Paired within task: control and evidence run on the same task, model, and effort, in counterbalanced order.
* Repetitions: three per task. Report per-task ratios, the geometric mean, and a 90% bootstrap interval over tasks.
* Pass rule: the deterministic harness passes 100%; verified task success is not lower than control; the interval for tokens per verified success is not worse than +10%. Report uncached input and compactions; do not promote on a token reduction alone.
* Power limit: 17 tasks cannot resolve a 3 percentage-point success difference. A result is directional until a larger independently authored corpus exists.
* Stop rules: stop on provider or authentication failure; stop when the usage window is exhausted; an infrastructure failure is not a result.

## Rollback

`DAVINCI_PRUNE_PROFILE` returns to the route default. Phase A and Phase B are code reverts. No session schema change is involved.

## Current state

* Phase A and Phase B: shipped, default on.
* Phase C: opt-in `evidence` profile, not promoted.
* Phase D: this pre-registration and the deterministic harness; live runs require separate approval.
