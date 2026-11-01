# Stage 0 report — retained reference and Stage 1 addendum

**This file does not re-measure Stage 0.** The preregistered Stage 0 report was
produced by PR #264 on commit `ecc8485` and is retained unmodified in that PR.
The numbers below are quoted from it as reported; they are not re-graded,
re-run or adjusted here.

## Stage 0, as reported by PR #264

| Item | Reported value |
| --- | --- |
| Task corpus | 17 tasks × 3 arms × 1 repetition |
| Passes | Droid Direct 12, Droid Script 11, DaVinci 10 |
| Requests | 247 / 111 / 88 |
| Requests on both-pass tasks (geomean) | Script/DaVinci 0.307; Direct/DaVinci 0.393 |
| Uncached input | Direct 1.612× DaVinci; Script 1.274× (interval crosses no-gap) |
| Mid-run cache drops | 16 requests, 60,435 / 185,252 uncached tokens (~32.6%) |
| Pruning / compaction | none observed |
| Initial prompt median tokens | DaVinci 10,802; Droid Direct 10,946; Droid Script 11,845 |

Evidence class: **reported live measurement, small sample, proxy confound**, as
recorded by the Stage 0 report. It is a pilot, not a parity proof.

## Stage 1 addendum (this branch)

- Deterministic behavior fixtures: 12/12 Droid reference scenarios pass
  (broker-level plus the real-guest syntax-error and long-job fixtures).
- Cache evidence is attribution-only: an identical wire prefix with fewer cache
  reads is classified `providerOrUnknown`; missing usage stays null.
- No TTL, breakpoint or hosted-tool shape is sent to the unverified
  `chatgpt.com` Codex route.
- **No new performance measurement was taken.** No matched suite, no allowance
  window, no dollar or quota claim.

## Promotion gate outcome

See [stage1-promotion.json](stage1-promotion.json) and
[the gap plan record](../../superpowers/plans/2026-10-31-droid-gap.md):
`rollout_gate` returns **`off`**. The default stays direct; controlled Script
stays opt-in and behind its recovery release gate.

Missing evidence is reported as missing, not as zero: the 16 historical drops
are not re-classified from archived data, and the subscription allowance delta
is `not measured`.
