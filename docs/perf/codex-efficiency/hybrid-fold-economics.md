# Hybrid fold economics

Status: fix written 2026-10-22 and tested offline. Live Off vs Hybrid A/B run
2026-10-23 with owner approval (12 runs, see "Live A/B"). Verdict: Hybrid is not
shown to be better. The fix itself (no paid fold that is then thrown away) held
on every live run.

Related: `docs/perf/codex-efficiency/live-2026-10-22.md` (Stage B, `fold14k`
arms), `docs/context-vm.md` (Hybrid mode).

## Problem

In the live `vm-hybrid-cm-off-fold14k` run (threshold 14,000, keep 3,000,
reserve 3,000) hybrid mode sent 13 summarizer requests and never applied a
compaction. The coding prompt grew from 14,843 to 20,497 input tokens across
them. `harness_stats` reported `runtime.compactions: 0` next to `folds: 15`.
Each summarizer request was paid for and its result thrown away.

## Diagnosis

The automatic trigger is `crates/davinci-agent/src/turn.rs:848` (before the
fix: `self.compact(None)`). In hybrid mode `Agent::compact` calls
`Agent::hybrid_compaction` (`crates/davinci-agent/src/lib.rs:4528` before the
fix). That function did two things in this order:

1. `fold_context(FoldReason::Manual, ...)`: a summarizer request, billed as
   provider purpose `compaction`.
2. `compact_messages_with_summary(...)` (`crates/davinci-agent/src/compaction.rs:446`)
   with `threshold - estimated_request_overhead_tokens()` as the limit.

Step 2 returns `compacted: false` when the ledger plus the newest turn do not
fit under the limit (`compaction.rs:501`, "Compaction result does not fit"),
and that is known before step 1. In the 14k run the limit was the threshold
minus the estimated request overhead (system prompt, tool schemas, ephemeral
context), about 13.5k. That leaves about 500 tokens for the ledger and the
recent messages, which no ledger fits. Hybrid cannot compact at that threshold
at all. Before the change it paid a summarizer request every turn for a result
it then discarded; now the preflight skips for free. What stops it compacting is
the setting (a 14k threshold against 13.5k of overhead), not the code.

One more cost was missing from the first attempt: the `retrieve_context` tool
schema is added by the first fold, so it was not in the overhead before then.
The budget now includes it until the fold offers it.

A second, separate case is a turn-1 request with nothing to replace. Turn 1
holds the user prompt and, on cache-sensitive routes, a turn context message
(`[user, turn_context]`). There is no assistant reply in it. The compaction
code found a cut point there (two messages), so the check did not stop it.
The first summarizer request in the 14k hybrid log (row 1 of the baseline) was
sent on turn 1. Its cause was the no-fit case above, not a missing cut point:
the earlier version of this note called it the no-cut-point case, and that was
wrong. The fix requires at least one assistant message in the replaced prefix.

When step 2 fails the history is unchanged, the context is still over the
threshold on the next turn, and the whole sequence repeats. Nothing limited
the repeats.

The deterministic ledger is already current when the trigger fires: the turn
loop runs `context_vm.append_delta` every turn in hybrid mode
(`turn.rs`, the `hybrid_context_vm` branch), and
`context_vm.load_state_from_root()` returns that state at no provider cost. The
summarizer only refines it.

### A counter that was misread

`tokensBeforeFold` and `tokensAfterFold` (`runtime/context_vm/mod.rs`, fold
metrics) are the estimated size of the previous and the new checkpoint ledger,
summed over folds. They are not prompt sizes. The ledger grows as events are
folded in, so `tokensAfterFold` above `tokensBeforeFold` (25,029 > 24,002 in the
hybrid run) is expected and does not show that folds enlarge the prompt. That
open question in `live-2026-10-22.md` is answered: the counter does not measure
it. Field names are unchanged because bench reads them; the meaning is now
documented in `docs/context-vm.md` and on the fields in
`runtime/context_vm/metrics.rs`.

## Baseline

Produced by `scripts/bench/fold_economics.py` from the two 14k logs
(`C:\bench\codex-eff-20261022\stage-b\vm-hybrid-cm-off-fold14k\davinci\m3-state-persistence-r0.stdout.jsonl`
and `...\vm-active-cm-off-fold14k\...`).
"reduction" is the input of the last coding request before the summarizer
request minus the first after it. "est saved" is the positive part of the
reduction times the coding requests that followed, up to the next compaction.
"net" is est saved minus the summarizer's own input tokens. Neither log
carries `hybridCompaction`, so the counters are "not reported".

Hybrid. Only completed root-actor attempts count, and consecutive summarizer requests with no coding request between them are one compaction row (cost summed). Here every compaction was a single request, so 13 compactions in 13 summarizer requests (unknown = no coding request before it):

| # | turn | reqs | summarizer in | summarizer out | coding in before | coding in after | reduction | net |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 1 | 1 | 1 | 1,009 | 451 | unknown | 10,743 | unknown | unknown |
| 2 | 8 | 1 | 5,020 | 686 | 13,975 | 14,843 | -868 | -5,020 |
| 3 | 9 | 1 | 2,474 | 351 | 14,843 | 14,962 | -119 | -2,474 |
| 4 | 10 | 1 | 2,350 | 325 | 14,962 | 16,506 | -1,544 | -2,350 |
| 5 | 11 | 1 | 2,683 | 977 | 16,506 | 16,672 | -166 | -2,683 |
| 6 | 12 | 1 | 2,611 | 248 | 16,672 | 16,764 | -92 | -2,611 |
| 7 | 13 | 1 | 2,615 | 660 | 16,764 | 16,870 | -106 | -2,615 |
| 8 | 14 | 1 | 2,706 | 336 | 16,870 | 16,974 | -104 | -2,706 |
| 9 | 14 | 1 | 2,889 | 421 | 16,974 | 17,473 | -499 | -2,889 |
| 10 | 16 | 1 | 4,115 | 989 | 18,885 | 19,384 | -499 | -4,115 |
| 11 | 17 | 1 | 3,026 | 413 | 19,384 | 19,594 | -210 | -3,026 |
| 12 | 17 | 1 | 3,368 | 597 | 19,594 | 19,876 | -282 | -3,368 |
| 13 | 19 | 1 | 3,378 | 1,396 | 20,165 | 20,497 | -332 | -3,378 |

Row 1 is a summarizer request on turn 1, before any coding request. It was a
no-fit case, not a no-cut-point case (see Diagnosis).

Regenerate the full report (rows and counters) with:

```
python scripts/bench/fold_economics.py \
  C:\bench\codex-eff-20261022\stage-b\vm-hybrid-cm-off-fold14k\davinci\m3-state-persistence-r0.stdout.jsonl \
  C:\bench\codex-eff-20261022\stage-b\vm-active-cm-off-fold14k\davinci\m3-state-persistence-r0.stdout.jsonl
```

The script reads only completed attempts of the root actor for sizes and
costs. A failed attempt (a refused request) still separates two compactions, so
compaction, refused request, recovery compaction is two rows. Usage comes from
`raw_usage`, or from `usage` (`input + cacheRead + cacheWrite`) when `raw_usage`
is missing.

The "cached share before" is unknown in both logs because the first compaction
request precedes every coding request.

## What changed

All in `crates/davinci-agent/src/lib.rs` unless noted.

1. **Dry run before the summarizer, for automatic attempts below capacity**
   (`hybrid_compaction`). Recovery paths (manual `/compact`, the overflow retry,
   an automatic attempt at or above capacity) skip it and pay the fold as before,
   then fit the result. The dry run is not a strict lower bound (see Trade-off),
   and a false skip there must not be able to stick a full window. It calls the same
   `compact_messages_with_summary` with the deterministic ledger and the same
   budget as the real call, through one helper (`hybrid_replacement`) so the
   fit math is not duplicated. If the dry run reports `compacted: false`,
   `hybrid_compaction` returns not compacted without calling `fold_context`.
   If the ledger state cannot be loaded, it folds as before.
   - The dry-run state (`hybrid_dry_run_state`) does the `continues` check
     `fold_context` does: a VM that holds another branch's ledger starts from
     nothing, then the deterministic reducer adds the unfolded events. The
     parent narrative is dropped, because the fold replaces it with the
     proposal's (a probe with a 1,000-byte parent narrative and a "short"
     proposal made the dry run need 2,408 tokens against 2,159 for the folded
     ledger; with the budget between them, the preflight skipped a recovery the
     old code made).
   - `hybrid_replacement` tries the threshold budget first for every caller.
     A recovery path falls back to the capacity budget only when nothing fits
     under the threshold. Capacity alone returned the first fitting cut, which
     keeps the most: from the probe history (`fill(4,400)`, a 12k prompt, "ok",
     "next"; threshold 10k, capacity 38k) a manual `/compact` kept 12,525 tokens
     over the threshold and the next turn paid for a second fold, where the
     threshold budget fits 525 tokens in one.
   - `hybrid_replacement` applies the assistant-reply rule per cut, on
     automatic attempts only: a cut whose replaced prefix has no assistant
     message is skipped and a later cut is tried (the rule lives in
     `compact_messages_with_summary_requiring`, so the dry run and the real call
     share it). Recovery paths do not apply it. An earlier version vetoed the
     whole result after the first fitting cut, which blocked a window holding
     `[huge prompt, small tool call, huge tool result]`: the only cut replaces
     the prompt, and /compact, the overflow retry and the automatic path all hit
     the veto.
2. **Fit budget** (`pending_retrieval_schema_tokens`,
   `hybrid_replacement_budget`). Budget = limit - estimated request overhead,
   minus the `retrieve_context` schema until the first fold offers it (unless a
   host overhead override or estimator is set, which already counts the real
   catalog). The overhead is the byte estimate and nothing derived from the
   provider's count. A calibrated overhead (provider tokens minus the estimate
   of the messages it covered) was tried and removed: with a provider that
   counts the messages at three quarters of their byte estimate the gap became
   budget, each replacement landed just under the threshold, and the next turn
   paid for another fold (a probe over 40 turns paid 5 requests where the
   estimate formula pays 4, and 19 against 4 at keep 7,500 in the reviewer's
   run).

   Invariants. For the same history the budget equals the one hybrid
   compaction always used, and every other change here (preflight,
   assistant-reply rule, threshold-first recovery, backoff) only removes or
   defers summarizer requests, so the new code never pays more than the old.
   And a recovery never uses the preflight, so anything the old behavior
   recovered from, the new code recovers from: it never gets stuck behind a
   skip.
3. **Recovery budgets against capacity.** The limit is the threshold for an
   automatic attempt, and `context_window - reserve_tokens` on a recovery path:
   `Agent::compact` (manual `/compact`), the overflow retry (`turn.rs:980`) and
   an automatic attempt whose context has reached capacity. They try the
   threshold budget first (see 1). A full window is recovered, not blocked.
4. **Backoff** (`compact_automatic`, called from `turn.rs:848`). A compaction
   that reached the fold and was not applied records the calibrated context
   size, which is the `tokens` value the trigger passed in (so the later
   `tokens < failed_at` check is on the same scale). The next automatic attempt
   waits until the context is at least `keep_recent_tokens` larger. The backoff
   defers only attempts below the capacity limit; recovery-class attempts
   (automatic at or above capacity, manual `/compact`, the overflow retry) are
   never deferred and never record one, so they retry every turn as before.
   Holding the backoff through the reserve band made a probe (default settings,
   60k window, a 42k first prompt) compact at turn 11 with 11 requests above
   capacity, where the old behavior compacted at turn 1 with 1. It clears when
   the context drops by more than `keep_recent_tokens` (a replaced history; a
   smaller drop can be the scale of a byte-estimate record against a later
   provider count), on an applied compaction, on a change of the window or of
   the compaction settings, and on `restore_branch_state`, which every session
   load and rewind calls.
5. **Off and active are unchanged.** The legacy path and the active fold policy
   were not edited.
6. **Telemetry.** `contextVm.hybridCompaction` in `feature_use()` (so in
   `harness_stats`), hybrid mode only: `attempts`, `applied`, `skippedNoFit`,
   `skippedNoAssistantReply` (the assistant-reply rule left no cut),
   `skippedBackoff`, `notAppliedAfterFold` (a summarizer request was sent and
   the result was not applied), `notAppliedOther` (not applied with no request
   sent), `contextTokensBefore`, `contextTokensAfter`. `attempts` is the sum of
   the six outcomes. Counters live on the agent, not on `ContextVmMetrics`,
   because the runtime (and its metrics) is replaced when a session loads, so
   they survive a session load. Token sums and skip results are message
   estimates.
7. **Report script.** `scripts/bench/fold_economics.py` with tests in
   `scripts/bench/tests/test_fold_economics.py`.

### Tests

- `crates/davinci-agent/src/lib_tests.rs`, module `hybrid_fold_economics` (39
  tests). Economics: no fit, no cut point, fit and counters, backoff defers
  until growth, capacity limit overrides it, smaller context clears it, branch
  restore clears it, manual bypass, overflow bypass. Review findings: the fit
  budget is the limit minus the estimated overhead; manual compaction budgets against
  capacity; a full window recovers with the backoff active (through
  `run_loop`); another branch's ledger does not block the dry run; a prefix
  with no assistant reply is not replaced (Appended turn context); the
  retrieval schema is in the budget before the first fold; only hybrid mode
  reports the object; paid-fold and other failures count apart; skip results
  report message estimates; counters survive a session load. Round 3: a full
  window of `[huge prompt, tool call, huge tool result]` recovers when
  automatic at capacity, manual and on overflow; below capacity the lone prompt
  is left alone and counted under `skippedNoAssistantReply`; the automatic path
  picks a later cut after a previous compaction; the backoff defers
  below capacity and never at or above it; the backoff records the trigger's
  `tokens`; the overflow test defers on the automatic path first. Round 4: a
  recovery lands under the threshold when something fits there and falls back
  to capacity otherwise; an overflow retry compacts once in its turn; a small
  drop from a byte-estimate record keeps the backoff; a recovery records no
  backoff;
  a host overhead is not charged the retrieval schema twice. Round 5: over 40
  turns with a provider that counts messages at three quarters of the estimate,
  `a_provider_that_counts_low_does_not_make_every_turn_a_paid_fold` holds the
  paid summarizer requests to 4, 4, 4, 4 and 3 for keep_recent 6,000, 7,000,
  7,500, 8,000 and 9,000 (the calibrated budget paid 5 at 6,000 and fails it).
  Round 6: with a 1,000-byte parent narrative that the next fold shortens, a
  manual `/compact` and an automatic attempt at capacity each recover with one
  paid fold (they failed while recoveries ran the dry run); below capacity the
  dry run without the parent narrative no longer false-skips; a recovery never
  records `skippedNoFit`.
  Round 7: with default settings, a 60k window and a 42k first prompt, a fold
  that cannot apply at turn 0 does not hold turn 1 back (compaction at turn 1,
  at most one request above capacity); a changed window or setting clears the
  backoff; a failed fold reports the full request estimate in both token fields.
- `crates/davinci-agent/tests/context_vm_hybrid.rs`: the `feature_use` test
  pins the new object and that `attempts` equals the sum of outcomes.
- `crates/davinci-coding-agent/src/output.rs`: `harness_stats` carries the
  object, and the off mode does not.
- Existing proof that an early user constraint survives repeated folds and a
  reload: `requirements_survive_repeated_compaction_without_a_session` and
  `requirements_survive_repeated_compaction_and_reload_from_the_session` in
  `crates/davinci-agent/tests/context_vm_hybrid.rs`.

### Trade-off

The dry run uses the deterministic ledger; the real call uses the folded one.
`validate_proposal` merges the summarizer's proposal into the deterministic
state, so the folded ledger is usually at least as large as the deterministic
one. That makes the dry run a close bound in the direction that matters here:
when it says the replacement cannot fit, the folded one rarely would, so
skipping the paid request loses little.

It is not a strict bound. The folded ledger can come out smaller than the
dry-run ledger for three reasons:

- The fold replaces the parent narrative with the proposal's (`reducer.rs`,
  `proposed.narrative.or_else(|| parent.narrative)`, up to 1,024 bytes). The dry
  run now renders without the parent narrative to allow for this.
- The fold drains retired entries over 32. Not modelled.
- A long history is folded in chunks, and the deterministic pass for the next
  chunk caps goals (`keep_first_directives_and_recent`) on a state that already
  holds the previous chunk's proposal goals. Not modelled.

The consequence of a residual false skip is a deferral, not a stuck session.
Below capacity the dry run skips for free and the context keeps growing. At the
capacity limit the attempt is a recovery: it skips the dry run, pays the fold,
and fits the result exactly as the old code did. How often a false skip happens
is not measured, and it should be rare.

The other direction is the cost that remains. A thin margin passes the dry run
and the real fit then fails after the paid fold. The backoff caps that at one
paid request per `keep_recent_tokens` of growth.

## Live A/B (2026-10-23)

Owner-approved, bounded: 3 long tasks (`m5-wide-migration`,
`m6-requirements-ledger`, `m7-log-forensics`) x 2 arms x 2 repetitions = 12
runs, order alternated per repetition. Both arms used compaction threshold
64,000, keep 20,000, reserve 16,384, Codemode off, retries off; only
`contextVm` differed. Model `gpt-6-luna`, effort high, Codex backend route.
Binary built from `a8a98549` (green push CI run 39898233917), runner scripts
at `d10ed0a0`. Every run reported the requested mode as its observed mode. No
auth, quota or route stop. Usage was present on every attempt. Artifacts:
`C:\bench\fold64k-20261022\runs`, report `ab_report.py` / `ab_report.json`,
per-fold rows `fold_economics.json`.

| arm | verified successes | hidden tests passed | requests | compaction requests | uncached input | total input | wall s |
| --- | --- | --- | --- | --- | --- | --- | --- |
| Off | 1 of 6 | 95.8% | 232 | 4 | 617,668 | 10,065,732 | 1,272 |
| Hybrid | 2 of 6 | 96.4% | 250 | 2 | 539,528 | 9,911,816 | 1,307 |

Per run (verified = every hidden test passed):

| task | rep | arm | verified | hidden | requests | compaction req | peak context | uncached | wall s |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| m5 | 0 | Off | no | 106/123 | 31 | 0 | 35,369 | 39,514 | 150 |
| m5 | 0 | Hybrid | yes | 123/123 | 40 | 0 | 57,385 | 53,354 | 192 |
| m5 | 1 | Off | no | 122/123 | 43 | 0 | 58,173 | 101,103 | 268 |
| m5 | 1 | Hybrid | no | 110/123 | 36 | 0 | 46,222 | 72,783 | 185 |
| m6 | 0 | Off | yes | 167/167 | 51 | 0 | 63,591 | 61,407 | 246 |
| m6 | 0 | Hybrid | yes | 167/167 | 63 | 1 | 63,880 | 158,991 | 339 |
| m6 | 1 | Off | no | 166/167 | 75 | 0 | 70,801 | 79,071 | 435 |
| m6 | 1 | Hybrid | no | 166/167 | 43 | 0 | 60,294 | 54,548 | 240 |
| m7 | 0 | Off | no | 37/41 | 15 | 2 | 116,281 | 187,636 | 74 |
| m7 | 0 | Hybrid | no | 36/41 | 39 | 1 | 118,780 | 145,162 | 190 |
| m7 | 1 | Off | no | 36/41 | 17 | 2 | 116,553 | 148,937 | 99 |
| m7 | 1 | Hybrid | no | 36/41 | 29 | 0 | 63,035 | 54,690 | 162 |

What the evidence shows:

- **Hybrid is not shown to be better.** 2 against 1 verified success in 6 runs
  each is within run-to-run noise: the m5 and m6 results flip between
  repetitions in both arms, and in 4 of 6 pairs the hybrid arm never
  compacted, so those pairs differ only by model variance. Hybrid used 8% more
  requests and 3% more wall time. Its 13% lower uncached input comes almost
  entirely from one pair (m7 rep 1), where the hybrid run stayed under the
  threshold and the off run compacted twice.
- **The fix held.** Across the 6 hybrid runs: 3 attempts, 2 applied, 1 skipped
  for free by the dry run (`skippedNoFit`, m7 rep 0), 0 `notAppliedAfterFold`.
  No summarizer request was paid and thrown away, against 13 of 13 in the
  October 22 14k log. On m7 the off arm paid 2 summarizer requests per run (the
  first on turn 3, before the large read, with no reduction); hybrid paid 1.
- **A fold breaks the cached prefix.** In m6 rep 0 the fold cut the coding
  prompt from 63,880 to 39,958 tokens, but the next request read 0 cached
  tokens (63,488 before), so it was billed fully uncached. With the
  summarizer's own 30,046 input tokens and only 3 coding requests left, that
  run's uncached input was 2.6x the off run's. `fold_economics.py` reports a
  net of +41,720 for it because its estimate counts total input; read that
  column together with the cached share, not alone.
- **Most runs never reached the threshold.** Only m7 (a 100k-token read on turn
  4) crossed 64k in both arms. m5 and m6 peaked between 35k and 71k. The
  comparison is therefore mostly a no-fold comparison.

What would settle it: more repetitions on tasks that cross the threshold in
both arms (m7-like), with uncached input and verified success as the primary
metrics, and a fold that keeps the cached prefix (item 4 of the original
proposal), which this change does not do.

## Not verified

- With the dry run in place, the 14k arm will not compact: about 500 tokens of
  budget cannot hold a ledger. The fix is the threshold setting, not the code.
  That is a configuration finding, not something this change decides.
- The cached-prefix cost of a fold is measured in one run only (m6 rep 0).
- `fold_economics.py` does not yet report uncached input per fold.

## Out of scope: active mode

Active mode also folds nearly every turn at 14k (22 summarizer requests in the
active log) and each fold rotates the cache epoch, which breaks the cached
prefix: cached share fell to 45.5%, and the cache read is stuck at 9,600 tokens
with drops to 0. That is the active-VM fold policy
(`ContextFoldPolicy::decide_automatic`), a separate design question. This
change does not touch it.
