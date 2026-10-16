# HTTP connection-reuse A/B: protocol, rule and stop conditions

Status: prepared, not run. No live model call is approved for this experiment. This file is the pre-registration: the
schedule, metrics, thresholds and stop conditions below are fixed before any run and are not changed after seeing
results.

Tooling: `scripts/bench/ab_http_reuse.py` (`schedule`, `commands`, `report`) and its 31 tests. It reuses the metric
extraction and the comparison function of `ab_transport.py`, so this experiment is judged by the same thresholds as
the WebSocket one. The stop conditions below are `stop_reason`.

Source under test: the `exp/http-connection-reuse` branch (see `docs/cache/http-connection-reuse-experiment.md`
there). The exact source commit and this harness commit are the pins; they are recorded in the approval request and
checked by the launcher before every run.

## Question

The 2026-10 HTTP versus WebSocket A/B (`ws-transport-ab.md`) measured later-request cache reuse of 0.56 to 0.67 per
task over HTTP and 0.84 to 0.96 over one persistent WebSocket. DaVinci's HTTP path opens a new TCP and TLS
connection for every request. Does keeping one HTTP connection per conversation, with the same requests and no
WebSocket, raise later-request cache reuse and lower net uncached input without hurting verified success or wall
time?

## Arms

Both arms use the same binary, the same Stage 1 base settings and the same `gpt-6-luna` at `high` effort. Only
`transport` differs.

| Arm | `transport` | What it does |
|---|---|---|
| `http` | `auto` (the production default and the control) | a new connection for every request |
| `reuse` | `http-reuse` | one pooled connection per conversation |

## Schedule (10 runs maximum)

| Runs | Role | Notes |
|---|---|---|
| 1 to 2 | warm-ups, discarded: `reuse`, `http` on `m2-public-api-migration-v2` | Run 1 is the compatibility probe for the new path. |
| 3 to 10 | 4 tasks (`m1` to `m4`) x 2 arms, one repetition | The first arm alternates by task: `http` first on two tasks, `reuse` first on two. |

No extra warm-ups, reruns, substituted tasks or extra experiments. Each run is its own single-run campaign.

## Metrics per run

First-request cached tokens; cached ratio over requests 2..n; total input, cached and uncached input; cache writes
(or unknown); requests; provider attempts and failed attempts; wall seconds; verified success; attempt labels
(`http`, `http-reuse`, `http-reuse-open`).

Unknown usage is never counted as zero. A run with unknown token totals is unscorable and makes the verdict
INCOMPLETE.

## Validity gates, in this order

1. **INVALID**: a run whose recorded settings or attempt labels do not match its arm.
   - The `http` arm must be all `http`.
   - The `reuse` arm must use only `http-reuse` and `http-reuse-open`. `http` on this arm would mean the one-shot
     path ran.
   - Of the attempts after the first, at least half must be labelled `http-reuse`. An arm that never expected a
     pooled connection did not test the question.
   - Missing labels or settings are invalid too.
2. **STOPPED**: any harness stop. No reruns, no stitching.
3. **INCOMPLETE**: fewer than 8 measured runs, or any measured run with unknown usage.

The `http-reuse` label is the request's expectation, made before it is sent: the previous response on the
conversation ended cleanly less than 55 s earlier. It is not a measurement of the connection. Reuse itself is
verified offline by loopback tests; live it is read from the cache metrics. This is a stated limit of the experiment.

## Decision rule

`reuse` is **PROMISING** against `http` only if all hold (the thresholds of the WebSocket experiment):

- median gain in later-request cache ratio is at least 0.10
- `reuse` has lower uncached input and a higher later ratio on at least 3 of 4 tasks
- net total uncached input is lower
- `reuse` passes at least as many tasks as `http`
- total requests are at most 1.10 times `http`'s
- total wall time is at most 1.10 times `http`'s

**NOT_PROMISING** if net uncached input is not lower or the median ratio gain is zero or negative. Otherwise
**MIXED**.

Reading the result against the WebSocket experiment is descriptive: it recorded later-ratio medians near 0.9 and
uncached input of 146,703 (persistent socket) and 102,617 (continuation) against 420,189 for HTTP over four tasks.
The runs were not side by side, so no "share recovered" statistic is registered. PROMISING means a larger study is
worth requesting. It never supports changing a default. Eight measured runs show a direction, not an effect.

What each outcome would mean:

- PROMISING with ratios near 0.9: the connection explains most of the gap, and a connection-reuse fix on the
  existing HTTP path is the smaller change than WebSocket.
- NOT_PROMISING: the connection alone does not explain it, and WebSocket transport or continuation stays the
  candidate.
- MIXED: a gain that is real but below the bar; report the numbers and do not decide.

## Stop conditions (stop the whole experiment, report, do not rerun)

- a harness stop, a refused request, or a credential or refresh failure
- the reuse arm sends any attempt outside `http-reuse` and `http-reuse-open`, or the http arm sends one outside `http`
- a run's recorded settings do not match its arm
- a run's model, binary hash or source commit differs from the pin
- more than one failed provider attempt (no usage) in any single run, or three in total
- usage totals missing with no failed-attempt count
- the maximum of 10 model runs is reached

## Estimated cost

From the 15-run WebSocket experiment on the same tasks, binary family and route: 31 minutes of wall time and the
5-hour window moved from 39% to 41%, with the weekly window unchanged at 22%. Ten runs are therefore estimated at
about 1 to 2 percentage points of the 5-hour window, under 1 point weekly, and about 20 to 25 minutes. These are
estimates, not measurements, and a usage snapshot is taken before and after.

## What this does not test

Other models, other task families, WebSocket transport, header or retention changes, model routing, or any other
harness. Four public task families cannot support a superiority or generalization claim.
