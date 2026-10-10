# ChatGPT Codex backend probe

Date prepared: 2026-09-24

This document is the evidence gate for Codex-backend-specific request features. The probe implementation lives behind the explicit maintainer command:

```powershell
davinci --codex-probe "$env:TEMP\codex-probe.json" --model gpt-5.6-luna
```

## Current evidence state

An authenticated probe was run on 2026-09-26 against `gpt-5.6-luna` using the
G1 checkpoint binary. The capture was sanitized before retention: only case
outcomes, cache counts, and header names were kept; turn-state/header values
were not retained.

Until a real authenticated run supplies evidence, features whose plan gate depends on the cases below must remain disabled by compatibility flags or explicit opt-in settings.

| Probe case | Current result | Gates |
| --- | --- | --- |
| `baseline` | Accepted; cached 0, cache-write 0 | Basic route sanity |
| `cache_warm` | Accepted; cached 0, cache-write 0 | Cache observation |
| `cache_reuse` | Accepted; cached 0, cache-write 0 | Cache observation |
| `prompt_cache_options` | Rejected HTTP 400 | Prompt cache options / TTL |
| `explicit_breakpoint_without_instructions` | Rejected HTTP 400 | Stable bootstrap breakpoint |
| `prewarm` | Rejected HTTP 400 | Prewarm |
| `allowed_tools` | Accepted; cached 0, cache-write 0 | Allowed-tools request shape |
| `additional_tools` | Rejected HTTP 400 | Late tool delivery |
| `custom_grammar_tool` | Accepted; cached 0, cache-write 0 | Freeform `apply_patch` |
| `assistant_phase` | Accepted; cached 0, cache-write 0 | Assistant phase replay validation |
| `tool_choice_none` | Accepted; cached 0, cache-write 0 | Cache-sharing compaction |
| `remote_compact` | Rejected HTTP 404 | Remote compaction decision |
| `x-codex-*` headers | Names observed | Usage-window header names |

The prewarm rejection is direct evidence against enabling that request shape
for this backend. The accepted cache cases returned zero cached and zero
cache-write tokens, so they do not justify a cache-option or breakpoint
compatibility flag. The retained probe is at
`C:\Users\sergi\davinci-bench-evidence\request-efficiency\probes\c1-codex-probe-20260926.json`.

## Recording a run

Run the probe with an authenticated ChatGPT Codex session. Copy only capability outcomes, cache token counts, and header **names** into this document. Do not commit access tokens, account IDs, cookies, authorization headers, or other credentials.

When a case is rejected, record its HTTP status and a short sanitized error. When accepted, record the model and date. Only then enable the corresponding compatibility flag for the model(s) actually covered by that run.

## WOR-33 Auto / Plan transition probe (2026-10-09)

The ignored CLI test `wor33_cache_identity_tests::live_auto_plan_auto_provider_cache_reads`
runs the real agent turn loop, permission transitions, frozen builtin tool projection,
appended turn context, and authenticated provider transport. It preserves available
native replay records. It schedules Auto warm-up, Auto, Plan, and Auto return, with
four requests, no retries, a 60-second HTTP idle timeout and a 240-second admission
deadline. It refuses API-key authentication and requires an explicit model and effort.
No new cache capability or alternate billing route is enabled.

To run it after explicit subscription authorization, choose the user's configured
model/effort and a private report path outside Git, then invoke only this test:

```powershell
$env:WOR33_MODEL = 'gpt-5.6-luna'
$env:WOR33_THINKING = 'medium'
$env:WOR33_REPORT = "$env:TEMP\wor33-mode-switch.json"
cargo test -p davinci-coding-agent --bin davinci -- --exact wor33_cache_identity_tests::live_auto_plan_auto_provider_cache_reads --ignored --nocapture --test-threads=1
```

The test records the emitted
cache key, stable request hash, ordered input preservation, actual attempt count,
raw provider cache counters separately from normalized usage, latency, failures,
and available subscription windows. Missing raw counters remain null. A positive
local prefix comparison cannot satisfy its provider-read acceptance assertion.
Completed rows are saved before propagating transport errors.

An initial four-request live run against base revision `67d2c303`, using the
configured ChatGPT-authenticated `openai-codex/gpt-5.6-luna` at medium effort,
kept one cache key and one stable request hash. Earlier input items remained exact
prefixes across both transitions. Normalized usage was:

| Turn | Ordinary input | Cache read | Cache write | Output | Provider call latency (ms) |
| --- | ---: | ---: | ---: | ---: | ---: |
| Auto warm-up | 4488 | 0 | 0 | 5 | 2954 |
| Auto | 4520 | 0 | 0 | 5 | 2040 |
| Plan | 4905 | 0 | 0 | 61 | 3109 |
| Auto return | 5100 | 0 | 0 | 19 | 2216 |

Four calls completed without transport errors: 19,013 ordinary input tokens,
90 output tokens, and 10,319 ms summed provider-call latency. Native resume records
and subscription windows were unavailable. The cache-read acceptance assertion
failed, including the Auto warm-up control. This run retained normalized usage
only, so it cannot establish whether absent raw counters became placeholders.
The probe now retains raw counters and attempt counts as well; that revision has
offline validation and has **not** consumed another live run. No included-plan
savings or provider cache reuse is established. WOR-33 remains open. Explicit
`DAVINCI_TURN_CONTEXT=system` and non-cache-sensitive routes retain their existing
system-prompt behavior; they are outside this subscription acceptance test.

The private initial report is
`C:\Users\sergi\davinci-bench-evidence\wor33-wor37-20261009\mode-switch.json`.

## WOR-37 cache-write reporting

Generic/non-streaming provider decoding (including Bedrock) and Google metadata
now preserve missing writes as unreported. Provider summaries and `cache-status`
use one atomic snapshot with reported/unreported request counts: all missing
writes serialize as `cacheWriteTokens: null` / `cacheWriteStatus: unreported`,
explicit numeric zero remains `reported`, and mixed totals are labeled `partial`
and represent only the reported lower bound. With no recorded requests the status is `none`.
Miss estimates require known write counts; an unknown-write turn cannot become
their baseline. Fixture provider-turn tests exercise decoding through the agent
loop into the summary; they are regression evidence, not live savings evidence.
