# Session cache-key A/B: protocol (prepared, not approved, not run)

Nothing here has spent subscription allowance. `scripts/bench/ab_cache_key.py` only prints a
schedule and commands, and judges results that already exist.

## Question

Does a per-conversation `prompt_cache_key` raise DaVinci's cache reuse on the `openai-codex`
route, compared with the current key shared by all compatible runs?

Current OpenAI guidance treats `prompt_cache_key` as optional for GPT-5.6 and later, so a benefit
is not assumed. The Stage 1 data points both ways:

* Warm-state DaVinci cache ratio is 0.69 (runs 5 to 12 of the campaign) against Codex's 0.90, a
  gap of 0.21. Of the 916k-token uncached gap, the hit rate explains the whole of it (prompt
  volume contributes -4%).
* DaVinci's first four runs sat at 0.46 and the later eight at 0.69 (sd 0.04). The shared key may
  already be giving later runs cross-run reuse of the static prefix (the first request read 8,704 or
  10,752 cached tokens in 8 of 12 runs and none in the other 4). A per-conversation key gives that up, so **the experiment can
  make things worse**, and the rule below treats that as a result, not a failure of the test.
* Every Stage 1 DaVinci request used HTTP; Codex used WebSocket. Transport is a second, untested
  difference and this A/B does not touch it.

## Pilot first (prepared, not approved, not run)

The 20-run study below stays a separately approved follow-up. A smaller pilot decides whether it is
worth requesting, using the same arms, the same build and the same measurements.

**Why a pilot can say something.** Two effects pull in opposite directions and the pilot reads them
separately:

* **Cost of abandoning shared-prefix reuse:** the cached tokens on request 1. Stage 1 read 8,704 or
  10,752 cached tokens on request 1 in 8 of 12 runs, so the shared key probably gives later runs a
  warm static prefix. A per-session key starts cold.
* **Gain within a conversation:** the cache ratio over requests 2..n. In Stage 1, 183 of 208 later
  requests (88%) read back more than 1,024 tokens less than the previous request's prompt, and only
  26 read nothing. Most requests read the static prefix and little of the conversation that grew
  since. Append-only requests rule out a changed prefix as the cause (WS1), so what is left is
  routing, eviction or transport. A private key might help with the first two. It cannot help with
  transport, which the pilot does not test.

**Schedule** (`python scripts/bench/ab_cache_key.py schedule --profile pilot`): 2 discarded warm-ups
(`shared`, then `session`, on the m2 task), then 4 measured pairs, one per task, repetition 0, both
arms back to back, the arm that goes first alternating (shared first in two pairs, session first in
two). That is 10 single-run campaigns; `commands --profile pilot` prints them and launches nothing.
A harness stop ends the pilot with no reruns and no stitching.

**Recorded per run:** cache reads on request 1 and over requests 2..n, uncached input, cache writes
(as measured), requests, wall time, verified success, and the effective cache-key mode. The mode comes
from `effective_settings.experimentalSessionCacheKey` in the result row; the report returns `INVALID`
if any row's recorded mode differs from its arm, or is missing. The build also refuses to start a run
with the switch on and `PI_GRAPH_CACHE_KEY` set, so an arm cannot silently run the other policy.

**Pilot rule (fixed before any run), `report <root> --profile pilot`:**

* `PROMISING` only if all hold: median later-ratio gain at least 0.10; later ratio higher in at least
  3 of 4 pairs; total uncached lower in at least 3 of 4 pairs and lower overall (so the request-1 loss
  has been paid for); session verified successes at least equal to shared; session requests at most
  1.10 times shared.
* `NOT_PROMISING` if total uncached is not lower, or the median later-ratio gain is zero or negative.
* `MIXED` otherwise. `INVALID`, `STOPPED` and `INCOMPLETE` are reported as such.
* The report also prints the median request-1 cached tokens lost, so the cost of leaving the shared
  prefix is visible even when the net result is positive.

`PROMISING` means "request the 20-run study". It never supports changing a default, and four pairs
cannot separate a real effect from run-to-run noise.

**Estimated usage** (from Stage 1 DaVinci runs: about 283k prompt tokens and 108k uncached per run,
about 163 s of provider time per run): 10 runs is about 2.8M prompt tokens, about 1.1M uncached
(the session arm may add up to roughly 9k per run if it loses the shared prefix), and about 30 to 45
minutes of wall time with setup and grading. That is half of the 20-run study and about 0.8 of the
DaVinci half of Stage 1. It is DaVinci only; no Codex runs. How much of the ChatGPT plan allowance that
is cannot be computed offline, and Stage 1 took no usage snapshot, so the pilot should record one
before and after.

## Arms

Both arms run the same experimental DaVinci build, so the key is the only difference.

| Arm | Settings | Request difference |
|---|---|---|
| `shared` (control) | no `experimentalSessionCacheKey` | none: production behaviour, byte-identical requests |
| `session` | `experimentalSessionCacheKey: true` | `prompt_cache_key` only (verified offline) |

Both arms use the harness defaults of the Stage 1 spec (`gpt-6-luna`, effort `high`, service tier
default, compaction on, transport `auto`, no other settings), the grading environment v2 harness,
the same machine, and the four public v2 tasks. No other model job may use the account.

## Prerequisites (each needs the owner's decision)

1. The experimental commit pushed to a branch with green CI. The checkpoint builder refuses a
   commit without CI proof, so the build cannot be made from a local-only change.
2. A new harness pin that includes grading environment v2 and `ab_cache_key.py`, recorded before launch.
3. Explicit approval of the allowance. Estimate from Stage 1 (about 283k prompt tokens and 108k
   uncached per DaVinci run): 20 runs is roughly 5.7M prompt tokens, 2.2M uncached, and about an
   hour of wall time.

## Fixed schedule (`python scripts/bench/ab_cache_key.py schedule`)

* 4 warm-up runs, discarded, alternating `shared`, `session`, `shared`, `session`, all on
  `m2-public-api-migration-v2`. They equalize process and provider state; they do not decide anything.
* 16 measured runs in 8 task pairs (4 tasks x 2 repetitions). Both arms run each pair back to
  back, and the arm that goes first alternates, so each arm is first in four pairs and neither arm
  gets a systematic head start or a later, warmer slot.
* One single-run campaign per run (`--reps 1 --rep-start N --variant ab-NN-<role>-<arm>`), each in
  its own `BENCH_RUNS` directory. `commands` prints the lines; it launches nothing.
* A harness stop (provider error, quota, cleanup failure, model mismatch) ends the experiment. No
  reruns and no stitching.

## Measurements (per run, from the existing result rows)

* Primary: uncached input tokens (input minus cached), including the cold first request.
* Mechanism: cache ratio over requests 2..n (`later_request_*`); cached tokens on request 1.
* Cache writes (`cache_write_tokens`; reported as measured, the route has reported 0 so far),
  requests, wall time, verified success, hidden-requirement fraction.

## Decision rule (fixed before any run)

`python scripts/bench/ab_cache_key.py report <root>` returns one of:

* `SUPPORTS_FURTHER_TESTING` only if all hold: median per-pair drop in uncached input of at least
  25%; `session` lower in at least 6 of 8 pairs; median gain in cache ratio (requests 2..n) of at
  least 0.10; verified successes in `session` at least as many as in `shared`; total requests in
  `session` at most 1.10 times `shared`.
* `NO_BENEFIT` if the median uncached change is zero or an increase (this includes losing shared-prefix reuse).
* `INCONCLUSIVE` otherwise; `INCOMPLETE` if fewer than 16 measured rows exist.

Sixteen runs show direction only. `SUPPORTS_FURTHER_TESTING` means "worth a larger confirmation or a
transport probe", never "change the default". A two-run outcome is not used for any decision.

## Risks of the session key

* **Loss of cross-conversation prefix reuse.** The static system-and-tools prefix (about 8.7k
  tokens) is currently shared by all runs. With per-conversation keys, a new conversation's first
  request may start cold: roughly 8.7k extra uncached tokens per conversation, about 8% of a mean
  Stage 1 run. Short conversations pay this once and may not recover it.
* **Warm-up dependence.** The shared key may be what lifted runs 5 to 12 to 0.69. The `shared` arm
  is warmed by the discarded runs so the comparison is steady state against steady state; the cold
  start is visible separately in request 1.
* **Routing behaviour is unknown.** The provider documents that a key influences routing and does not pin it.
* **Sub-agents.** The explicit `PI_GRAPH_CACHE_KEY` override still wins and is unchanged. Other worker
  request paths were not traced. A worker that goes through the same request function shares its
  parent's conversation id (one random id per process when no session is stored).

## What this does not test

Transport (HTTP versus WebSocket), headers, retention, model routing, tasks outside the four public
families, or any other harness.

## Unknown usage (added after the 10-run pilot)

A run whose provider attempts did not all report usage has `input_tokens` and `cached_tokens` set to null. The
report treats that run as unscorable:

- `metrics()` returns `usage_known: false` and `None` for uncached, later ratio, first-request cached and cache writes.
  Unknown is never counted as zero.
- `evaluate()` and `evaluate_pilot()` return `INCOMPLETE` with `unscorable_runs`, after the `INVALID` and `STOPPED`
  checks. No threshold changed and nothing is imputed.
- `uncached_lower_bound` is the uncached input of the attempts that did report. It is labeled as a lower bound, is
  informational only and never enters a verdict.

The frozen pilot (`ab-pilot-20261015-b`, runs 7 and 8 unknown) evaluates to `INCOMPLETE`, which is its registered verdict.
