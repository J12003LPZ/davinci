# WebSocket confirmation study: protocol, rule and stop conditions

Status: prepared, not run. No live model call is approved for this study. This file is the pre-registration: the
schedule, metrics, thresholds, missing-data handling and stop conditions below are fixed before any run and are not
changed after seeing results. It is intended to be the last public-task transport study before production hardening
and independently authored holdouts.

Tooling: `scripts/bench/ab_ws_confirm.py` (`schedule`, `commands`, `report`, `verify-pins`, `verify-settings`) and its
56 tests (`scripts/bench/tests/test_ab_ws_confirm.py`). It reuses the row metrics of `ab_cache_key.py` and the per-run
transport proof of `ab_transport.py`. The stop conditions below are `stop_reason`.

Earlier evidence: `ws-transport-ab.md` (15 runs, one repetition) and `http-reuse-ab.md` (10 runs, NOT_PROMISING). They
motivate this study and are not pooled with it.

## Question

On DaVinci's public sign-in route (`https://api.openai.com/v1`), does a persistent WebSocket, with or without
`previous_response_id` continuation, lower total uncached input and wall time against the HTTP default, without
lowering verified coding success? If yes, which is the simplest implementation that earns adoption?

## Arms

One frozen binary, one model configuration, the same Stage 1 base settings. Only `transport` differs, and the
launcher checks that the three settings files differ from `runner.BASE_SETTINGS` in nothing else
(`verify-settings`); every result row is checked again for the same property.

| Arm | `transport` | What it is |
|---|---|---|
| `http` | `auto` | production default, a new HTTP/SSE connection per request (control) |
| `ws` | `public-websocket` | persistent socket, full input replayed on every request |
| `wsc` | `public-websocket-continuation` | same socket, plus `previous_response_id` with new items only |

`ws` is kept as a third arm because it is the simpler implementation. Continuation (state tracking, a full-replay
fallback when the server rejects `previous_response_id`) is adopted only if it earns its keep against `ws`, not
against `http`.

## Pins (checked before every run)

| Item | Value |
|---|---|
| Source | `exp/public-responses-websocket-rustls` @ `5ab199e96c674780db59724f38c8c4e7f05b2ae4` (rustls 0.23.46) |
| Binary | `davinci.exe`, SHA-256 `202f539653f051b39c541b8acae4d5e2403d8a6e4620cf2f77ef56854cac9449` (the checkpoint of the earlier 15-run experiment; no rebuild) |
| CI of the source | run 38810739310, green on attempt 2 (attempt 1: known intermittent Windows `davinci-coding-agent` hang, one targeted rerun, no check weakened); dependency audit passes |
| Manifest | `large_manifest_v2.json`, SHA-256 `49e83faeb7b3091db5ea2952dbc8d180f5acb1b17c44abfd8080ce91e52309e0`; task fixtures unchanged |
| Model | `gpt-6-luna`, effort `high`, service tier `default` |
| Harness | the commit that contains this file; the approval request names its SHA, and the launcher refuses to run unless `git rev-parse HEAD` equals it (`verify-pins`) |

The binary, manifest and harness hashes are compared with `verify-pins`. Every result row must carry the same
binary hash, source commit, model, effort and service tier, and one fixture hash per task (the same across arms and
repetitions). A missing field is an error, not a pass.

## Design

4 tasks (`m1` to `m4`) x 3 repetitions x 3 arms = 36 measured runs, after 3 discarded warm-ups: 39 runs at most.

- A **block** is one (task, repetition) with all three arms. There are 12 blocks.
- Arm order within a block uses the six possible orders, each on exactly two blocks. Each arm sits in each position
  in exactly four blocks, and each task sees three different orders, leading once with each arm. This is generated
  by `schedule()` and tested.
- Blocks run repetition-major (`m1 m2 m3 m4`, then again, then again), so one task never runs back to back.
- Warm-ups (`ws`, `wsc`, `http` on `m2-public-api-migration-v2`) are discarded. The first one is the compatibility
  probe: a handshake, model or usage failure stops the study after at most one cheap request.
- No extra warm-ups, reruns, substituted tasks or extra experiments. Each run is its own single-run campaign.

### Stages

A sign-in token lasts about 60 minutes, so the study runs in three stages with a fresh sign-in and a preflight before
each: stage 1 is the warm-ups plus repetition 0 (15 runs), stage 2 is repetition 1 (12), stage 3 is repetition 2 (12).
Stage boundaries are block boundaries, so no block straddles a sign-in. The pause between stages is not controlled and
is spread over arms by the counterbalancing. A stage does not start if the preflight fails, if the 5-hour usage window
reads 60% or more, or if the weekly window reads 80% or more.

## Metrics per run

First-request cached tokens; cached ratio over requests 2..n; total input, cached and uncached input; output tokens;
cache writes (or unknown); requests; provider attempts and attempts without usage; wall seconds; verified success;
attempt labels (`http`, `websocket`, `websocket-continuation`).

## Missing usage (fixed before any run)

- Unknown is never zero. A run is **unscorable** if input, cached or output tokens are missing.
- Any unscorable run, or any missing run, makes the study **INCOMPLETE**. There is no GO or NO_GO from an incomplete
  study, however favourable the remainder looks. Thresholds do not change.
- An incomplete report carries only partial directional evidence, labelled as such: the numbers over blocks in which
  all three arms are complete, with no pass or fail flags. Every excluded block and every missing or unscorable run
  is listed by name, and a labelled lower bound from the reporting attempts is shown for information only. Verified
  passes seen so far are shown too, so a safety problem is visible in an incomplete study.
- No block is dropped to produce a verdict, and no usage is imputed.

## Statistical interpretation

The 12 blocks are paired but are not 12 independent observations: three repetitions of four task families share
fixtures and prompt prefixes. Decisions therefore rest on task-level totals and on a worst-task guard, not on block
counts alone. Effects are reported per task and per repetition for every comparison. The block count of
`uncached lower` is a consistency check. The exact one-sided sign-test p-value is printed as
`nominal_block_sign_p` with a note that blocks are not independent; it is informational, is not part of the decision
rule and is not evidence of a generalizable effect. The result covers four public task families on one model.

## Decision rule

A candidate (`ws` or `wsc`) **PASSES** against `http` only if all hold:

| Check | Threshold |
|---|---|
| Safety: verified passes | candidate passes at least as many runs as `http` |
| Safety: per task | no task loses more than one repetition against `http` |
| Total uncached input | at most 0.60 x `http` |
| Uncached input, blocks | lower in at least 9 of 12 blocks |
| Uncached input, tasks | lower on at least 3 of 4 tasks (repetition totals) |
| Total wall time | at most 0.90 x `http` |
| Wall time, worst task | no task's median wall time above 1.10 x `http`'s |
| Requests | total at most 1.10 x `http` |
| Mechanism | median later-request cache-ratio gain at least 0.10 |
| Hidden cost | total tokens (uncached input plus output) lower than `http`'s |

`wsc` **earns continuation** over `ws` only if all hold: uncached input at most 0.80 x `ws`, wall time at most
0.90 x `ws`, lower uncached input and lower wall time on at least 3 of 4 tasks, and passes not below `ws`.

Verdict:

- `GO_WSC`: `wsc` passes and (it earns continuation, or `ws` does not pass).
- `GO_WS`: otherwise, if `ws` passes. The simpler implementation wins unless continuation shows a meaningful
  additional benefit.
- `NO_GO`: neither passes. A cache gain without the wall-time gain is a NO_GO.
- `INCOMPLETE`, `INVALID` (arm labels, settings, pinned identity or fixtures do not match), `STOPPED`: no verdict.

A GO means production hardening and independent holdout validation are worth doing. It never changes a default, and it
does not support a superiority or generalization claim.

## Stop conditions (stop the whole study, report, do not rerun)

- A harness stop, a refused request, or a credential or refresh failure.
- A run's attempt labels or settings do not match its arm (a fallback to HTTP, or a continuation on `ws`).
- A run's binary hash, source commit, model, effort, service tier or task fixture differs from the pin.
- More than one attempt without usage in a run, or three in total; usage totals missing with no attempt count; a
  WebSocket run with incomplete usage; output tokens not recorded.
- A run that fails verification and also has an attempt without usage (the cause cannot be separated).
- Safety: over completed blocks, a candidate is two or more verified passes behind `http`. This ends the whole
  study (the proposal said "that arm"; stopping the whole study keeps the counterbalancing intact and is the more
  conservative rule).
- The maximum of 39 model runs.

## Estimated allowance

From the 15-run WebSocket experiment (31 minutes; 5-hour window 39% to 41%; weekly unchanged) and the 10-run reuse
experiment (28 minutes; 5-hour window 41% to 44%; weekly 22% to 23%): about 0.13 to 0.30 points of the 5-hour window
and 0 to 0.1 points of the weekly window per run. For 39 runs: about 5 to 12 points of the 5-hour window, 0 to 4 weekly
points, and 80 to 110 minutes of run time over three stages. These are estimates, not measurements; other Codex use
confounds them. Usage is read before and after each stage.

## Before production (kept out of the frozen binary)

Not fixed here, to keep the experiment's binary unchanged:

1. The TLS stale-socket probe can miss a close already buffered by rustls. It needs a single reconnect-and-resend
   before any response bytes, with a loopback TLS test.
2. The header-assembly test cannot prove the real handshake headers. It needs a loopback WebSocket server that
   captures the handshake and asserts the exact header set, with no token in logs.
3. The stats counter overcounts full replays in continuation mode, and two assertions are weak.
4. Fallback to HTTP on a failed or refused upgrade, idle and lifetime reconnect, and, for continuation, a full-replay
   fallback when `previous_response_id` is rejected.
5. Public route only; no assumption that Codex's own backend applies.
6. Merge the rustls bump (PR #192) first.

## What this does not test

Other models, other task families, DaVinci's own provider route, unknown tasks, or any comparison with Codex CLI or
Claude Code. Parity or superiority needs independently authored holdout tasks and a registered head-to-head.
