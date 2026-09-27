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
