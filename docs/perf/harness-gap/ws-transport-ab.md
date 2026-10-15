# HTTP versus public-WebSocket A/B: protocol, rule and stop conditions

Status: prepared, not run. No live model call is approved. This file is the pre-registration: the schedule, metrics,
thresholds and stop conditions below are fixed before any run and are not changed after seeing results.

Tooling: `scripts/bench/ab_transport.py` (`schedule`, `commands`, `report`) and its 36 tests (the stop conditions below are `stop_reason`). The result rows now carry
`provider_transports`, the count of provider attempts per transport label, so every run proves from its own row which
transport and request shape it used.

Source under test: the `exp/public-responses-websocket` branch (see `docs/cache/public-websocket-experiment.md` there).
The exact source commit and this harness commit are the pins; they are recorded in the approval request and checked by
the launcher before every run.

## Question

Does keeping one WebSocket per conversation on DaVinci's public sign-in route, with or without `previous_response_id`
continuation, raise later-request prompt-cache reuse and lower net uncached input without hurting verified success or
wall time? The route is `https://api.openai.com/v1`; Codex's own backend is not involved.

## Arms

All three arms use the same binary, the same Stage 1 base settings and the same `gpt-6-luna` at `high` effort. Only
`transport` differs.

| Arm | `transport` | What it isolates |
|---|---|---|
| `http` | `auto` (HTTP/SSE on this route, the production default and the control) | baseline |
| `ws` | `public-websocket` | persistent connection, full input replayed |
| `wsc` | `public-websocket-continuation` | continuation on top of the persistent connection |

Comparisons: `ws` vs `http` is the transport effect, `wsc` vs `ws` is the continuation effect, `wsc` vs `http` is the
combined effect. A continuation effect is claimed only from `wsc` vs `ws`.

## Schedule (15 runs maximum)

| Runs | Role | Notes |
|---|---|---|
| 1 to 3 | warm-ups, discarded: `ws`, `wsc`, `http` on `m2-public-api-migration-v2` | Run 1 is the compatibility probe. If the handshake, the model, usage or the continuation fails here, the experiment stops after at most two cheap requests. |
| 4 to 15 | 4 tasks (`m1` to `m4`) x 3 arms, one repetition | The arm order rotates per task (`http ws wsc`, `ws wsc http`, `wsc http ws`, `wsc ws http`). With four tasks no order is perfectly balanced; the report says so. |

No extra warm-ups, reruns, substituted tasks or extra experiments. Each run is its own single-run campaign.

## Metrics per run

First-request cached tokens; cached ratio over requests 2..n; total input, cached and uncached input; cache writes
(or unknown); requests; provider attempts and failed attempts; wall seconds; verified success; effective transport,
request shape and attempt labels (`http`, `websocket`, `websocket-continuation`).

Unknown usage is never counted as zero. A run with unknown token totals is unscorable and makes the verdict
INCOMPLETE; a labelled lower bound from the reporting attempts is shown for information only.

## Validity gates, in this order

1. **INVALID**: a run whose recorded settings or attempt labels do not match its arm. The `http` arm must be all
   `http`. `ws` must be all `websocket` (no `http`: that would be a fallback; no continuation). `wsc` must be
   `websocket` and `websocket-continuation` only, with at least one continuation. Missing labels or settings are
   invalid too.
2. **STOPPED**: any harness stop (credentials, model mismatch, transport failure). No reruns, no stitching.
3. **INCOMPLETE**: fewer than 12 measured runs, or any measured run with unknown usage.

## Decision rule (per comparison, then overall)

For a comparison of arm X against baseline B over the four tasks, X is **PROMISING** only if all hold:

- median gain in later-request cache ratio is at least 0.10
- X has lower uncached input and a higher later ratio on at least 3 of 4 tasks
- net total uncached input is lower
- X passes at least as many tasks as B
- X's total requests are at most 1.10 times B's
- X's total wall time is at most 1.10 times B's

**NOT_PROMISING** if net uncached input is not lower or the median ratio gain is zero or negative. Otherwise **MIXED**.

Overall: PROMISING if `ws` vs `http` or `wsc` vs `http` is PROMISING; NOT_PROMISING if both are; otherwise MIXED. The
attribution line says whether the gain comes from the persistent transport, from continuation, from both, or only
from the combination (not separable). PROMISING means a larger study is worth requesting. It never supports changing
a default, and twelve runs show a direction, not an effect.

## Stop conditions (stop the whole experiment, report, do not rerun)

- the WebSocket handshake is refused or the OAuth token is rejected
- the model is rejected on the WebSocket route
- `previous_response_id` is rejected with `store: false`
- usage on `response.completed` is incomplete (receipt not `usage_complete`) for a WebSocket run
- any attempt label outside the arm's allowed set (a fallback to HTTP)
- a run's model, binary hash or source commit differs from the pin
- a credential or refresh failure
- more than one failed provider attempt in any single run, or three in total
- a verified-success regression the harness attributes to the transport (a failure with a transport error)
- the maximum of 15 model runs is reached

## Estimated cost

From the 10-run cache-key pilot on the same tasks, binary family and route: about 260k input tokens per run (mostly
cached), about 2.8 minutes per run, and the 5-hour window moved from 55% to 58% (which included other Codex use on
the account). Fifteen runs are therefore estimated at 4 to 5 percentage points of the 5-hour window (range 3 to 6) and
less than one point of the weekly window, and about 45 minutes of wall time, up to one hour with provider stalls. The
WebSocket arms may be faster. These are estimates, not measurements, and a usage snapshot is taken before and after.

## What this does not test

Other models, the other 3 task families, the ChatGPT backend route, header or retention changes, model routing, or any
other harness. Four public task families cannot support a superiority or generalization claim.
