# ChatGPT Codex backend probe

Date prepared: 2026-09-24

This document is the evidence gate for Codex-backend-specific request features. The probe implementation lives behind the explicit maintainer command:

```powershell
davinci --codex-probe "$env:TEMP\codex-probe.json" --model gpt-5.6-luna
```

## Current evidence state

The probe has **not been run for this implementation branch**. The connected development machine was offline while the branch was prepared, so no backend capability is recorded here as accepted.

Until a real authenticated run supplies evidence, features whose plan gate depends on the cases below must remain disabled by compatibility flags or explicit opt-in settings.

| Probe case | Current result | Gates |
| --- | --- | --- |
| `baseline` | Not run | Basic route sanity |
| `cache_warm` | Not run | Cache observation |
| `cache_reuse` | Not run | Cache observation |
| `prompt_cache_options` | Not run | Prompt cache options / TTL |
| `explicit_breakpoint_without_instructions` | Not run | Stable bootstrap breakpoint |
| `prewarm` | Not run | Prewarm |
| `allowed_tools` | Not run | Allowed-tools request shape |
| `additional_tools` | Not run | Late tool delivery |
| `custom_grammar_tool` | Not run | Freeform `apply_patch` |
| `assistant_phase` | Not run | Assistant phase replay validation |
| `tool_choice_none` | Not run | Cache-sharing compaction |
| `remote_compact` | Not run | Remote compaction decision |
| `x-codex-*` headers | Not observed | Usage-window header names |

## Recording a run

Run the probe with an authenticated ChatGPT Codex session. Copy only capability outcomes, cache token counts, and header **names** into this document. Do not commit access tokens, account IDs, cookies, authorization headers, or other credentials.

When a case is rejected, record its HTTP status and a short sanitized error. When accepted, record the model and date. Only then enable the corresponding compatibility flag for the model(s) actually covered by that run.
