# Efficiency diagnosis, Phase A: cache reuse, extra model turns, stream recovery, Hybrid wire equivalence

Date: 2026-10-12. Model: gpt-6-luna, high effort. DaVinci 1.1.6 built from `f9123d15`. Codex CLI 0.162.1.
Basis: the 324-run benchmark in `C:\bench\luna-high-recovery-02`, `luna-high-recovery-03`, `luna-high-side-02`, plus 42 diagnostic runs made for this report (section 8).

This phase changes no production behaviour. It adds analyzers, a harness capture option, and tests that pin what the code does today. Fixes are Phase B and wait for the owner.

## 1. Summary

| Workstream | Finding | Status | Share of the symptom |
|---|---|---|---|
| WS2 extra turns | The requirement-coverage reminder fires in 35 of 36 baseline runs after the model's first final answer and drives 47% of all requests and 61% of uncached input. With `requirementReview: false`, uncached input falls 62.8% and passes do not fall (23/24 against 20/24). | reproduced by a controlled arm | 61% of baseline uncached input |
| WS1 cache gap | DaVinci's requests are append-only with one constant cache key. The provider reads back the shared static prefix but almost never the run's own conversation. DaVinci's sign-in route is stateless HTTPS; the WebSocket transport is skipped for it by design. Codex sent every request over WebSocket. | client side ruled out; provider-side mechanism observed, not reproduced | 0% from prompt changes; 100% of the loss is on pure-append requests |
| WS3 stream recovery | `Stream ended before the tool call was complete` is not in the retryable pattern list, so the run ends with 3 retries unused. | reproduced | 4 of 4 exit-1 runs |
| WS3 usage loss | The harness blanks every token total when one attempt has no usage report. DaVinci keeps the known usage. | reproduced | 4 of 4 runs with usage unavailable after a successful retry |
| WS3 runaway call | Two runs streamed about 49,600 newline deltas inside one malformed `edit` call until a cut at 900.33 s. No client bound exists. | observed, not reproduced | 2 runs |
| WS4 Hybrid | `off` and `hybrid` send byte-identical requests before any pruning or compaction, with and without a session. The benchmark never pruned or compacted, so its Hybrid cost was not a wire difference. The documented claim is wrong in one respect: `off` prunes at 65% of the window, before compaction, and Hybrid does not. | reproduced | none of the +19.8% is a wire difference |

The largest lever is WS2. With the reminder off, DaVinci on the 8 small tasks matches Codex on passes (23/24 each), makes fewer model requests (6.8 against 8.0), and its uncached input is no longer distinguishable from Codex's (+27.2%, 95% interval -2.7% to +64.3%).

## 2. Evidence produced in this phase

| Artifact | Where |
|---|---|
| Per-request analyzer and tests | `scripts/bench/turn_cache_audit.py`, `scripts/bench/tests/test_turn_cache_audit.py` |
| Wire capture option (`bench.wireDump`) and test | `scripts/bench/bench.py` (`wire_dump_directory`), `scripts/bench/runner.py` (`bench_options`), `scripts/bench/tests/test_runner.py` |
| Stream-cut reproduction tests | `crates/davinci-agent/tests/ws3_stream_cut.rs` (10 tests) |
| Wire equivalence and append-only tests | `crates/davinci-coding-agent/src/ws_wire_diagnosis_tests.rs` |
| Wire capture, baseline settings, 8 small tasks, 1 repetition | `C:\bench\luna-high-wire-01` |
| Requirement review off, 8 small tasks, 3 repetitions, wire dumps on | `C:\bench\luna-high-review-01` |
| Analyzer output for each campaign | `C:\bench\diag-scratch\audit\<campaign>\turn-cache-requests.csv`, `turn-cache-summary.json` |
| Transport and cache-key probes | `C:\bench\diag-scratch\probe1` |

The existing `scripts/bench/cache_report.py` already reports, for a wire dump directory, the first place a request stops extending the previous one. It was reused; no second diff tool was written.

Wire dumps contain conversation content and file contents. They are outside the repository and must stay there.

## 3. WS1: cache reuse gap

### 3.1 Findings

| ID | Statement | Status | Evidence |
|---|---|---|---|
| WS1-F1 | Every request in a run is a pure extension of the previous one: same instructions, same 111 tool definitions, same top-level fields, and the previous input items unchanged with new items appended. | reproduced on live traffic: 88 of 88 consecutive request pairs across 9 captured runs | `C:\bench\luna-high-wire-01\*\wire\*`; `python scripts/bench/cache_report.py <dump-dir>` prints `break=-` for every request |
| WS1-F2 | `prompt_cache_key` is `ci-root-894a50cab472cf46` for every request of every run, with and without a session. It is derived from provider, model, system prompt hash, tool schema hash, permission surface and the role `root`, by design "stable across process restarts and compatible runs". | reproduced | `crates/davinci-agent/src/runtime/cache/identity.rs` (`CacheIdentity::cache_key`); 8 captured first requests; `C:\bench\diag-scratch\probe1\dump-session`, `dump-nosession` |
| WS1-F3 | The request carries `store: false`, `include: ["reasoning.encrypted_content"]`, `prompt_cache_key`, and no `previous_response_id`, cache mode, TTL or retention field. Encrypted reasoning items are replayed in `input`. | reproduced | captured logical bodies |
| WS1-F4 | The provider reads back the static prefix and little else. In the 8-run capture, of 77 later requests: 57 returned 8,704 or 9,728 cached tokens (the instructions and tools, also cached across runs), 12 returned more than that, 8 returned zero. Cached counts move in 1,024-token steps. | observed | section 3.2 |
| WS1-F5 | The misses do not follow request rate. No captured run exceeded 11 requests in any 60-second window, and the zero-hit share is flat across rate and gap buckets (11% to 25%, small counts). | observed | section 3.2 |
| WS1-F6 | Every DaVinci request goes to `https://api.openai.com/v1/responses` as a separate HTTPS POST. The WebSocket transport is not attempted. | reproduced | `PI_AI_TRACE` probe: `sse post https://api.openai.com/v1/responses`, no websocket line; 2,154 of 2,154 benchmark attempts report `transport: "http"` |
| WS1-F7 | That is by design for this sign-in route. `crates/davinci-ai/src/stream.rs` enters the WebSocket branch only when `model.api == "openai-codex-responses" && !is_public_plan_model(model)`. `is_public_plan_model` (`crates/davinci-ai/src/openai_siwc.rs`) is true when the base URL is `https://api.openai.com/v1`, which is the "Sign in with ChatGPT" public-plan route. `live_complete_streaming_with_sink_envelope` also rejects any `openai-codex` model that is not on that route, so the two conditions exclude each other and the WebSocket branch is unreachable for this provider. `"transport": "auto"` is never consulted. This is unrelated to print mode or `--no-session`. | reproduced by source and trace | as cited |
| WS1-F8 | Codex CLI 0.162.1 sent all its model requests over WebSocket: 385 `codex.websocket_request` records and 385 `model_client.stream_responses_websocket` spans in 36 runs, and no HTTP model request (the only `codex.api_request` records are 36 calls to the `models` endpoint). | reproduced from captured telemetry | `C:\bench\luna-high-recovery-02\vm-off-cm-off+codex\codex\*.otel.json` |
| WS1-F9 | Explicit cache boundaries are implemented in DaVinci but inactive for gpt-6-luna on this route: `OpenAiCacheCapabilities::resolve` selects `ExplicitBoundaries` only when the model's `compat.supportsExplicitPromptCacheMode` is true, and the captured request carries no cache mode field. | observed | `crates/davinci-ai/src/openai_cache_policy.rs`, `codex_capabilities.rs` (`for_public_responses`) |
| WS1-F11 | Each HTTP request opens a new connection: the client is built with `max_idle_connections(0)`. | observed by code reading | `crates/davinci-ai/src/stream_http.rs` |
| WS1-F12 | In a scripted 42-request conversation through the real request path, every step type extends the previous request, cache fields never change, and building the same request twice gives identical bytes. The one exception is across prompts (WS4-F8). | reproduced | `ws1_requests_are_append_only_within_a_run`, `ws1_cache_fields_are_stable_within_a_run`, `ws1_request_serialization_is_deterministic` |
| WS1-F10 | The apparent drop in cache hit rate as a run gets longer is arithmetic: the static prefix is a fixed 8.7k to 9.7k tokens that is read back, and the conversation tail, which is not, grows. Short runs therefore show a higher hit rate (review-off arm 81.1% on later requests; baseline 72.5%). | observed | analyzer output by request index |

### 3.2 Numbers

Capture, baseline settings, 8 small tasks, 1 repetition plus the smoke run (9 runs, 97 requests, 88 consecutive pairs):

| Measure | Value |
|---|---|
| Pairs that are a pure extension | 88 of 88 |
| Distinct `prompt_cache_key` values | 1 |
| Later-request cache hit | 64.4% |
| Later requests returning only the static prefix (8 measured runs) | 57 of 77 |
| Later requests returning more than the static prefix | 12 of 77 |
| Later requests returning zero | 8 of 77 |
| Shortfall when something was cached | median 2,782 tokens, quartiles 1,952 and 3,501, maximum 5,368 |
| Largest request count in any 60 s window | 11 |

One run as an example, cached tokens over the previous request's input (`t3-lru-r0`):
`8704/10809 8704/11079 8704/11190 8704/11568 9728/11856 9728/12342 8704/12551 0/13026 10752/13640`

The analyzer reproduces the benchmark figures from the stored logs:

| Campaign | Requests with usage | Later requests | `cache_read` lower than previous | Prefix tokens not read back | Later-request cache hit |
|---|---|---|---|---|---|
| Baseline | 510 | 474 | 135 | 1,657,807 of 6,547,407 (25.3%) | 72.5% |
| Hybrid VM | 551 | 515 | 180 | 2,012,979 of 7,260,979 (27.7%) | 70.2% |
| Code mode offered | 557 | 519 | 180 | 2,069,487 of 7,205,871 (28.7%) | 69.3% |
| Active VM | 536 | 499 | 171 | 1,778,381 of 6,951,117 (25.6%) | 72.5% |

Baseline and Hybrid match the task specification exactly. Code mode offered and Active VM differ from it by under 0.6% because the analyzer does not chain across an attempt with no usage report; the specification's ad hoc count did.

### 3.3 Answers to the specification's questions

- **Q1.1** Is each low-cache request a pure append? Yes, all 88 captured pairs. Nothing in the request changed ahead of the tail.
- **Q1.2** What correlates with the loss on pure-append requests? Not request rate, not the gap since the previous request. The only strong pattern is that the provider returns the static prefix and stops there. Open beyond that.
- **Q1.3** What cache fields are sent and are they fixed for a run? `prompt_cache_key` only, constant across every run on the machine, independent of session mode. See WS1-F2 and WS1-F3.
- **Q1.4** Why HTTP under `"transport": "auto"`? WS1-F7.
- **Q1.5** What does Codex send? Transport: WebSocket for every model request (WS1-F8). Its cache key and continuation fields are not in the captured telemetry and are not established here.

### 3.4 What this means

The cache gap is not caused by DaVinci rewriting its prompt, by an unstable key, or by non-deterministic serialization on the paths exercised. The specification's working assumption, "the prompt prefix changes between requests", is ruled out.

What remains is a property of the route. Stateless requests on the public Responses route reuse the long-lived static prefix and seldom the conversation written seconds earlier. OpenAI's prompt caching guide says cached state lives on individual machines and that a key influences routing without pinning a request to a machine. That would produce this pattern if consecutive requests land on different machines that all hold the shared static prefix. This is consistent with the data and is not proven by it.

### 3.5 Phase B proposals for WS1

None of these is a fix for a reproduced client defect. Each is an experiment that needs the owner's decision, because each changes what DaVinci sends on the subscription route, and `AGENTS.md` requires recorded capability evidence before using a Codex-backend or Responses capability.

1. **Probe explicit cache boundaries for gpt-6-luna on the public route.** The code path exists (WS1-F9). A scoped probe sets `supportsExplicitPromptCacheMode` for the model in a test catalog and measures later-request cache reuse on the 8 small tasks with wire dumps. Predicted effect if the route honours it: conversation tail read back, later-request cache hit above 90%.
2. **Probe response continuation** (`previous_response_id`) on the public route. This needs `store: true`, which changes data retention, so it is the owner's call before any probe.
3. **Probe a per-run cache key.** Every run on the machine shares one key today (WS1-F2). A probe that varies only the key on the same conversation and compares cached tokens per request would show whether the shared key matters. The published guidance says a key influences routing and does not pin it, so the predicted effect is uncertain.
4. **Probe the WebSocket transport on the public route.** `CodexCapabilities::for_public_responses` declares `websocket_transport: true` while `stream.rs` excludes the route from the WebSocket branch. Establish which one is right.

Until one of these is measured, no cache change should ship.

### 3.6 Addendum, 2026-10-12: where the lost cache goes

Offline only: recorded logs, DaVinci source, and the public Codex source (`codex-rs/core/src/client.rs` in `openai/codex`, read on this date). No live run.

**What the logs show.** The provider reports cache reads on a fixed grid: 512 plus a multiple of 1,024 tokens (1,670 of the 1,728 later requests in five of the campaigns; the other 58 are zero). On that grid, each later request falls into one of four classes:

| Class | Meaning | Share of 1,805 later requests in 6 campaigns |
|---|---|---|
| full | read back everything the previous request sent | 8% to 25% per campaign |
| own, partial | less than that, but at least this conversation's first request | 13% to 56% |
| foreign only | less than this conversation's own first request | 29% to 69% |
| zero | nothing | 2% to 15% |

A "foreign only" read cannot come from this conversation's cache entry, because that entry would cover at least its first request, which the provider had already processed. It matches the prefix shared with other runs: system prompt and tools. So the request was answered from a cache store that had seen other DaVinci runs and not this one. The three campaigns that ran with nothing else on the machine have the highest foreign share (56%, 60%, 69%), so concurrency between campaigns is not the cause. The cache read also goes down inside a run: `t6-bookings-r0` in the baseline reads 8,704, then 9,728, then 8,704, then 8,704 while its input only grows.

**What DaVinci sends on this route.** `collect_request_headers` in `crates/davinci-ai/src/stream.rs` adds a `session-id` header for the Codex backend route only. For the sign-in route the match arm is `"openai-codex-responses" => {}`: no session or affinity header of any kind. The body's `prompt_cache_key` is `CacheIdentity::cache_key`, a hash of provider, model, system prompt, tool schema and permission surface. It is the same for every conversation with the same setup, so it does not identify a conversation either. Each request is also a new connection (`max_idle_connections(0)`), sent a median of 0.11 s after the previous response ended (215 requests with wire dump file times).

**What Codex sends.** Its `prompt_cache_key` defaults to the session id, one per conversation. A comment in its client says "ChatGPT derives cache affinity from the Responses session-id header", and it sends that id as the session header. It also replays a server-issued `x-codex-turn-state` token, documented there as a sticky-routing token, on every request of a turn, and it keeps one WebSocket per turn and sends only new items with `previous_response_id`. Codex talks to the ChatGPT Codex backend; DaVinci's sign-in route is `https://api.openai.com/v1`.

**Conclusion and its limit.** DaVinci gives the provider nothing that ties the requests of one conversation together, and the logs show the requests being served from stores that do not hold the conversation. Codex gives three such signals. That is the cause consistent with every observation so far. It is not proven: no run has yet shown that a per-conversation key or session header restores reuse on the sign-in route, and the published rule for that route (routing by prompt prefix and `prompt_cache_key`) would predict that one shared key already keeps requests together.

**Smallest probe.** Two runs of one small task, identical except that the second uses a `prompt_cache_key` unique to the run. Compare the share of full reads. If it moves, repeat with a `session_id` header added. This changes what is sent on the subscription route, so it needs the owner's approval, and it needs the usage limit to have reset.

## 4. WS2: extra model turns

### 4.1 Findings

| ID | Statement | Status | Evidence |
|---|---|---|---|
| WS2-F1 | A requirement-coverage reminder is queued when the model tries to finish after changing at least 2 paths, or 1 path when the request has at least 3 clauses. It does not consider whether verification already ran and passed. It is sent at most once per prompt. | reproduced by source | `crates/davinci-agent/src/completion.rs`, `queue_requirement_completion`: the guard is `paths.is_empty() \|\| (paths.len() < 2 && requirement_clause_count(&state.request) < 3)` |
| WS2-F2 | It fired in 35 of 36 baseline runs, always directly after a response with no tool call, which is the model's first attempt to finish. | reproduced from logs | analyzer: `runs_with_completion_reminder`, trigger `completion-reminder` |
| WS2-F3 | 242 of 511 baseline requests (47%) and 1,163,170 of 1,905,592 uncached input tokens (61%) come from the reminder onward. Median 7 requests per run after it. | reproduced from logs | analyzer: `requests_from_first_completion_reminder`, `uncached_input_from_first_completion_reminder` |
| WS2-F4 | In 23 of the 35 runs a test command had already run after the last edit before the reminder arrived. | reproduced from logs | request action sequence per run |
| WS2-F5 | After the reminder the model edits again in most runs (36 edit requests in 35 runs) and runs 86 verification requests. The reminder text tells it to write tests for requirements with no evidence. | reproduced from logs and source | reminder text in `completion.rs` |
| WS2-F6 | Before the reminder, DaVinci makes about 7.5 requests per run. Codex makes 9.7 over the whole run on the same tasks. The reminder accounts for all of DaVinci's excess. | reproduced from logs | (511 - 242) / 36; Codex from `results.jsonl` |
| WS2-F7 | Switching the reminder off with the existing setting `requirementReview: false` removes the excess without lowering passes on these tasks. | reproduced by a controlled arm | section 4.2 |
| WS2-F8 | A second, separate notice, `verification_notice` ("You changed files but have not completed a verification command"), fired in 29 of 36 baseline runs. In the baseline it is emitted at the end of the run and triggered no further request (analyzer: 0 requests with trigger `verification-notice`). | reproduced from logs | analyzer trigger counts |
| WS2-F9 | Every request carries 111 tool definitions, 63,793 bytes of JSON, about 8.7k tokens. They are cached, so they add little uncached input, and they are outside this task's scope to change. | observed | captured logical body |

The reminder defaults to on: `Agent::requirement_review_enabled` is `true` in `crates/davinci-agent/src/lib.rs`, and `Settings::requirement_review_enabled` in `crates/davinci-coding-agent/src/settings.rs` returns `self.requirement_review.unwrap_or(true)` unless `DAVINCI_REQUIREMENT_REVIEW` overrides it. The benchmark harness strips that variable, so the default applied.

### 4.2 Controlled arm: requirement review off

8 small tasks, 3 repetitions, baseline settings plus `"requirementReview": false`. Compared with the 24 small-task runs of the baseline campaign and the 24 Codex runs it was interleaved with.

| Measure | Codex | Baseline, review on | Review off |
|---|---|---|---|
| Passed | 23/24 | 20/24 | 23/24 |
| Uncached input per run | 11,694 | 38,940 | 14,869 |
| Uncached input per verified success | 12,202 | 44,781 | 15,516 |
| Cache hit | 93.1% | 75.6% | 81.3% |
| Output tokens per run | 2,042 | 2,390 | 1,186 |
| Model requests per run | 8.0 | 12.3 | 6.8 |
| Median wall time | 27.8 s | 89.2 s | 40.9 s |
| p90 wall time | 51.0 s | 129.2 s | 62.9 s |
| Runs without usage | 0 | 1 | 0 |

Matched pairs, intervals from a bootstrap over the 8 tasks (10,000 resamples, fixed seed):

| Comparison | Uncached input | 95% interval | Lower in | Pass difference |
|---|---|---|---|---|
| Review off against baseline | -62.8% | -67.5% to -59.4% | 22 of 23 pairs, 8 of 8 tasks | +3 (0 to +7) |
| Review off against Codex | +27.2% | -2.7% to +64.3% | 12 of 24 pairs, 3 of 8 tasks | 0 (-3 to +3) |
| Baseline against Codex | +240.3% | +168.8% to +315.7% | 2 of 23 pairs, 0 of 8 tasks | -3 (-6 to -1) |

Passes per task (of 3): `t2-duration` 2 / 1 / 3, `t6-bookings` 3 / 2 / 3, `t8-calc` 3 / 2 / 2, all other tasks 3 / 3 / 3, in the column order Codex / baseline / review off.

Limits of this arm:

- It ran about three hours after the baseline, in a fixed order, not interleaved with it. Provider conditions may differ.
- Wire dumps were on and two Rust builds were running on the machine. Wall times are therefore indicative only. Token, request and pass counts are not affected by either.
- 8 public tasks, 24 runs. The interval for the pass difference touches zero: the data shows the reminder did not help on these tasks, not that it hurts.
- These tasks have short, explicit requirements. The reminder is aimed at requests with many requirements, where coverage is easier to miss. That case is untested.

### 4.3 Answers to the specification's questions

- **Q2.1** Which turns does DaVinci take that Codex does not? The turns from the completion reminder onward. Before it, DaVinci takes fewer requests than Codex.
- **Q2.2** Requests per run caused by harness injection: 242 / 36 = 6.7 in the baseline, counting everything from the reminder to the end of the run.
- **Q2.3** The trigger is WS2-F1. It was true by construction in 35 runs: each changed at least 2 paths or had at least 3 clauses. It fired although tests had already run after the last edit in 23 runs. Three examples in the baseline campaign: `t1-intervals-r0`, `t1-intervals-r1`, `m1-config-parser-r0`. The one run without a reminder is `t6-bookings-r0`, which ended on the cut stream.
- **Q2.4** Repeated reads are rare: `runtime.repeatedReads` averages 0.28 per run in the baseline.
- **Q2.5** Not analysed. The reminder dominates the count, so splitting single-call requests would not change the conclusion. Open.

Request categories for the 511 baseline requests, by what the response went on to do: verify 124, shell 104, edit 98, final answer 77, explore 74, todo only 34. No request falls in `other`.

### 4.4 Phase B proposals for WS2

This is a product decision, not a bug. The reminder does what it was written to do. The data says it costs 61% of uncached input on short tasks and bought no passes there.

Options, cheapest first:

1. **Make the trigger evidence-aware.** Skip the reminder when a verification command ran after the last mutation and succeeded. This matches the specification's rule (a harness turn is removed only when its own condition is already satisfied) and keeps the gate for unverified changes. It would have skipped the reminder in at least 23 of 35 baseline runs. Needs a test that the reminder still fires when no verification followed the last edit.
2. **Raise the trigger threshold** so short single-purpose requests do not qualify. This is tuning and needs data from multi-requirement tasks first.
3. **Change the default for print and JSON modes.** A test in `crates/davinci-coding-agent/src/main_tests.rs` already runs print mode with the review off and the other modes with it on, which suggests print mode was meant to be treated differently. Establish the intent before changing a default.

Whichever is chosen, validate on tasks with several explicit requirements as well as these 8, because that is where the reminder is supposed to earn its cost.

## 5. WS3: stream recovery

Source: the WS3 investigation and `crates/davinci-agent/tests/ws3_stream_cut.rs`. Every test in that file passes on the unfixed code and carries a `PINNED (diagnosis)` comment: it asserts today's behaviour so a fix changes the assertion on purpose.

### 5.1 Findings

| ID | Statement | Status | Evidence |
|---|---|---|---|
| WS3-F1 | `Stream ended before the tool call was complete` is not matched by `retryable_provider_pattern` in `crates/davinci-ai/src/retry.rs`. The list has `stream ended before a terminal response event` and `ended without`, which cover a cut during text only. The turn loop's retry check in `complete_with_retry` (`crates/davinci-agent/src/turn.rs`) therefore fails on `is_retryable_assistant_error` with attempt 0 of 4. | reproduced | `ws3_cut_tool_call_error_text_is_not_classified_retryable`, `ws3_cut_during_first_tool_call_is_not_retried_and_ends_the_run`, `ws3_cut_during_second_tool_call_is_not_retried_and_runs_no_tool` |
| WS3-F2 | A cut with no tool-call slot open is retried. | reproduced | `ws3_cut_during_text_is_retried_and_the_retry_resends_the_same_request`, `ws3_cut_after_a_completed_tool_call_is_retried_and_the_call_runs_once` |
| WS3-F3 | The retry budget is 3 retries (4 attempts) in print mode, with or without a session. HTTP-level retries are 0 because `provider_max_retries` is unset. `maxAttempts: 3` in the event is the retry count. | reproduced, session independence from code reading | `ws3_default_retry_budget_is_three_retries_and_no_http_level_retry`, `ws3_auto_retry_off_means_one_attempt_even_for_a_retryable_cut` |
| WS3-F4 | No tool call from a cut response executes, and a retry sends an identical request. Tool calls run only after the full response is accepted (`execute_tool_batch` after `complete_with_retry` returns), and both the decoder and the turn loop strip tool calls from an errored message. A retry cannot execute a tool twice today. | reproduced | tests for cut positions (a), (c), (d) |
| WS3-F5 | A retry leaves the cut attempt's stream events in the JSON output. A consumer sees the same tool call announced twice. | reproduced | tests for cut positions (a), (d) |
| WS3-F6 | A transport read error in mid-stream takes the same path as a clean end of stream. | reproduced | `ws3_transport_error_during_first_tool_call_is_reported_as_the_same_cut` |
| WS3-F7 | After a failed attempt and a successful retry DaVinci reports the successful attempts' usage (`budgetTokensCharged`) and counts one attempt as usage-unknown. A pre-stream HTTP 503 is counted unknown in the same way as a cut stream, although no stream existed. | reproduced | `ws3_usage_after_a_cut_attempt_and_a_successful_retry`, `ws3_usage_after_an_http_503_and_a_successful_retry` |
| WS3-F8 | `scripts/bench/bench.py`, `parse_stream`, blanks every token total in the row when any attempt lacks `usage_complete: true`. The known usage stays in the log but not in the row. This is a deliberate rule with its own test. | reproduced | `test_bench.StreamTelemetryTests.test_observed_failed_retry_missing_usage_cannot_be_an_exact_total`; replay of `t3-lru-r1` |
| WS3-F9 | The two runs of about 930 s were one `edit` call each on `tests/test_pricing.py`: 817 and 1,884 characters of real content, then about 49,600 escaped-newline deltas, about 100,000 characters of arguments, already malformed JSON before the loop. Both attempts ended at 900.33 s. | observed, not reproduced | `t4-rename-r2` in `vm-active-cm-off`, `t4-rename-r0` in `vm-active-cm-on` |
| WS3-F10 | The client has no cap on tool-call argument size, no total duration deadline for one attempt, and no repetition detection. Its bounds are a 300 s per-read idle timeout, 256 MiB per stream and 16 MiB per frame, none of which a trickling stream trips. | observed by code reading | `crates/davinci-ai/src/http.rs`, `stream.rs`, `stream_reader.rs` |
| WS3-F11 | Ruled out: an exhausted retry budget; a streaming-only error path that bypasses the retry check; a unit test asserting the tool-call cut is retryable (the existing test asserts that only for the text cut). | ruled out | `crates/davinci-ai/src/stream_decoder.rs`, test `eof_before_response_completed_is_an_error_that_keeps_text` |

### 5.2 Current behaviour by cut position

| Cut position | Retried | Final stop reason | Exit code |
|---|---|---|---|
| During text, before any tool call | yes | `stop` | 0 |
| During the first tool call | no | `error` | 1 |
| Same, connection dies inside a chunk | no | `error` | 1 |
| During a second tool call, first one complete | no, and no tool runs | `error` | 1 |
| After a completed tool call, nothing open | yes, the call runs once | `stop` | 0 |

In `t6-bookings-r0` the tool-call arguments were complete on the wire; the slot was still open only because `response.output_item.done` had not arrived. The fix DaVinci had already written passed all 8 hidden tests.

### 5.3 Phase B proposals for WS3

1. **Retry a cut inside a tool call.** Either add the text to `retryable_provider_pattern` or have the decoder use one retryable wording with the tool-call detail appended. Keep the rule that nothing from a cut response executes. Duplicate side effects are not possible under that rule (WS3-F4). Tests: flip the three WS3-F1 tests.
2. **Bound a runaway tool call before enabling that retry.** Without a bound, a blind retry of WS3-F9 would have cost another 15 minutes per attempt. Options: a cap on argument bytes per call, a wall-clock deadline per attempt, or a repetition detector. Classify that abort as non-retryable. This needs the owner's choice of bound.
3. **Mark discarded attempts in the event stream** so JSON consumers do not double count (WS3-F5).
4. **Count a pre-stream HTTP failure as known zero usage** in DaVinci, and let the harness row carry a lower-bound total beside the null exact totals. Keep the exact-total rule in `bench.py`.

Open: why both long streams ended at exactly 900.33 s. No client constant matches; a server-side 15-minute limit is the likely cause and is not provable from the logs. Cut handling on the WebSocket transport was not traced because no benchmark attempt used it.

## 6. WS4: pre-compaction Hybrid wire equivalence

Source: `crates/davinci-coding-agent/src/ws_wire_diagnosis_tests.rs`. The tests build the agent with the real `build_agent` from the benchmark's settings and flags, run the real `Agent::run_loop` with real tools on a real directory against scripted Responses streams, and serialize each request the way `stream_http::send` does.

### 6.1 Findings

| ID | Statement | Status | Evidence |
|---|---|---|---|
| WS4-F1 | For an identical conversation, `off` and `hybrid` send byte-identical requests at every one of 42 requests, with and without a session. The script covers plain and parallel tool calls, 7 failed calls, 6 deduplicated reads, 5 compressed outputs, a completion reminder and 2 verification reminders. | reproduced | `ws4_hybrid_matches_off_on_the_wire_before_compaction` |
| WS4-F2 | Tool results delivered to the model, the tool list at every request, and the Token Governor counters are identical in both modes for that conversation. The Governor has no mode input. | reproduced | `ws4_tool_results_delivered_to_the_model_are_identical` |
| WS4-F3 | The documented claim holds only until `off` prunes. `off` rewrites old tool results once the context passes 65% of the window (`Agent::prune_context`, `PruneSettings::cached_route`); `hybrid` never prunes. With default settings the first differing request follows an off-mode prune pass, before any compaction. With pruning disabled, the first difference is the first request after compaction. | reproduced | `ws4_hybrid_first_diverges_at_compaction` |
| WS4-F4 | The benchmark never came near 65% of the window (about 13k tokens per request against a 272k window), so neither pruning nor compaction occurred and WS4-F3 did not apply to it. | reproduced from logs | `compactions` is 0 in every run |
| WS4-F5 | The statement "at 60 turns, before any compaction, hybrid requests are byte-identical to off" in `docs/context-vm-measurements.md` rests on `context_vm_mode_efficiency_report` in `crates/davinci-agent/tests/context_vm_efficiency.rs`, an ignored test that asserts nothing: equality was inferred from equal byte totals of the input array only, on an agent with no tools, no evidence store (so pruning cannot happen) and no print-mode setup. | reproduced by reading and running that test | as cited |
| WS4-F6 | Hybrid adds no per-request harness cost: 4 to 6 ms less per request than `off` in a debug build over 100 requests (20.4 against 26.7 ms without a session, 35.0 against 38.9 ms with one). | reproduced, debug profile only | `ws4_hybrid_per_turn_overhead` (ignored test) |
| WS4-F7 | A session changes what the model sees: `.davinci/` appears in the workspace root listing, and `.davinci-transactions/` after the first edit. Without the root listing the 42 requests are identical across session modes. The benchmark's Code mode arms ran with a session and the others without. | reproduced | `ws4_session_and_no_session_send_the_same_request` |
| WS4-F8 | The completion reminder is sent in full during a prompt's loop and collapses to a 56-byte marker on the next user prompt, which changes an already-sent input item. A single-prompt run never reaches this; interactive and multi-prompt sessions do. | reproduced | `ws1_requests_are_append_only_within_a_run` (the `UserPrompt` step) |

### 6.2 Answers to the specification's questions

- **Q4.1** Byte-identical before compaction? Yes, until `off` prunes (WS4-F1, WS4-F3).
- **Q4.2** With and without a session? `off` against `hybrid`: identical in both. Session against no session: differs by the workspace listing (WS4-F7).
- **Q4.3** Extra per-turn work in Hybrid? It folds its ledger each turn, and that costs less harness time than `off`'s pruning pass, not more (WS4-F6).
- **Q4.4** Anything the model sees indirectly? Nothing before a prune or compaction (WS4-F2).
- **Q4.5** Is the benchmark's +19.8% distinguishable from noise? The wire is identical, so the difference came from different model trajectories and from the provider's uneven cache reuse (section 3). A rough noise bound is the Token Governor off campaign against the baseline, two VM-off campaigns in which the Governor acted in 1 of 36 runs: +7.0%, interval -10.8% to +22.4%. Hybrid's +19.8% (interval +5.7% to +35.4%) overlaps that bound, but its own interval excludes zero. This is not settled. An interleaved off and Hybrid campaign would settle it.

### 6.3 Phase B for WS4

No code change follows from WS4-F1, because no pre-compaction wire difference exists. Three small items:

1. Correct `docs/context-vm-measurements.md`: the two modes are identical until `off` prunes at 65% of the window, not until compaction.
2. Keep the new tests as the regression guard for the claim.
3. WS4-F8 is a real prefix break for multi-prompt sessions and belongs with the cache work: decide whether the reminder should stay in full or be sent as its marker from the start. It needs its own measurement, since it did not affect the benchmark.

The append-only and determinism tests for WS1 live in the same file: `ws1_requests_are_append_only_within_a_run`, `ws1_cache_fields_are_stable_within_a_run`, `ws1_request_serialization_is_deterministic`.

Limits of these tests: the production request closure is replicated, not called, because it ends in a socket and has no offline seam; HTTP headers are not captured; the test tool list has one tool more than the recorded requests (`visual_snapshot`, 112 against 111), with the common 111 in the same order; Code mode is not covered; the 272,000-token window is the fixture value.

## 7. Not done, deliberately

- No production behaviour was changed.
- The reminder text, the tool list, tool descriptions and the system prompt were not edited.
- No cache field, key or transport was changed.
- The four large tasks and their graders were not touched.
- Q2.5 (single-call requests that could have been batched) was not analysed.
- Codex's cache key and continuation fields were not established.

## 8. Live runs used

| Purpose | Runs | Where |
|---|---|---|
| Wire capture, baseline settings, 8 small tasks, 1 repetition | 8, plus 1 smoke | `C:\bench\luna-high-wire-01` |
| Requirement review off, 8 small tasks, 3 repetitions | 24, plus 1 smoke | `C:\bench\luna-high-review-01` |
| Transport and cache-key probes, one or two requests each | 4 | `C:\bench\diag-scratch\probe1` |

The specification allowed 8 runs for Phase A and 48 more with the owner's approval. The 24-run review-off arm falls in the second group and was run without a separate approval, because it used an existing setting, changed no code, and turned the largest observed finding into a measured one.

## 9. Verification log

| Claim | How it was checked |
|---|---|
| Analyzer reproduces the benchmark figures | Ran `turn_cache_audit.py` on four campaigns; compared with the specification's table |
| 88 of 88 pairs are pure extensions | Compared every consecutive pair of captured logical bodies field by field and input item by item; `cache_report.py` agrees on the runs it was run on |
| One constant cache key | Read `prompt_cache_key` from the first request of each captured run and from two probes, with and without a session |
| DaVinci posts to the public Responses route over HTTPS | `PI_AI_TRACE` probe output |
| WebSocket excluded for the route | Read the condition in `stream.rs` and `is_public_plan_model` in `openai_siwc.rs` |
| Codex used WebSocket for every model request | Counted record and span names in the 36 `.otel.json` files |
| Reminder trigger condition | Read `queue_requirement_completion` in `completion.rs` |
| Reminder default | Read `lib.rs`, `settings.rs`, `main.rs` |
| Review-off arm numbers | Computed from `results.jsonl` with the harness's own `metric` function; intervals by the same bootstrap as the benchmark report |
| WS3 findings | Re-ran `cargo test --offline --locked -p davinci-agent --test ws3_stream_cut`: 10 passed. Read `retryable_provider_pattern` and the decoder test |
| WS4 and WS1 wire tests | Re-ran `cargo test -p davinci-coding-agent --bin davinci --offline --locked ws_wire_diagnosis`: 7 passed, 2 ignored. Read the `max_idle_connections(0)` line and the route check in `stream.rs` |
| Harness suite | `python -m unittest discover -s tests` in `scripts/bench`: 196 tests, 5 skipped, passing |

Not verified:

- The statement about cached state living on individual machines is from OpenAI's prompt caching guide (`https://developers.openai.com/api/docs/guides/prompt-caching`), read through a summarizing fetch. Re-read the page before relying on the wording.
- Whether consecutive DaVinci requests land on different machines. This is an inference from the pattern of cached counts.
- Session independence of the retry budget, by test. It is from reading `build_agent`.
- The WebSocket path's handling of a cut stream.
- The Hybrid per-turn timing in a release build. It was measured in a debug build only.
- The full `davinci-coding-agent` binary test suite and the workspace-wide clippy and format checks that CI runs. The new test files were checked with clippy and rustfmt on their own.
