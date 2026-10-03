# DaVinci vs Codex CLI subscription-efficiency comparison

Status: **pending live measurement**

Updated: 2026-10-03

The comparison runner is `scripts/compare-codex.mjs`. A valid result requires:

- the same model for Codex CLI and DaVinci;
- fresh copies of the same fixture repository;
- at least five mixed-size tasks;
- three runs per harness per task;
- task-specific check commands that return success only when the requested work is complete.

Before collecting the full sample, verify the JSON event shapes and usage completeness
on a bounded task per harness. Missing or invalid usage appears as `null`/unknown,
never zero. Codex input includes cached input. DaVinci uses raw nullable counters
from physical `provider_observation` attempt receipts, including retries and host
operations present in the stream. Their normalized assistant copies and logical
rollups are not counted again. Started but unfinished attempts and conflicting
receipts remain unknown. Explicitly reported zero is still valid.

The `usageSource` field distinguishes `provider_attempts`, `codex_turns`, and
`legacy_unverified`. Legacy DaVinci assistant/session snapshots lack field-presence
evidence; their normalized zeroes must not be presented as measured counters.
`scripts/measure-codex-cache.mjs` follows the same rule. These different evidence
scopes are not interchangeable measures of complete account consumption.

Run the local parser regressions without making provider calls:

```powershell
node --test scripts/tests/subscription-usage.test.mjs scripts/tests/subscription-usage-review.test.mjs
```

The runner includes all attempts in pass rates. A pass requires a successful
harness process and successful task checks. Compare matched successful work as
well as failures; lower tokens on an incomplete task are not an improvement.
Transcripts may omit retries and auxiliary calls, so transcript totals are a
screening measurement rather than complete account consumption.

API catalog prices and API-dollar-to-credit conversion are not subscription
measurements. The runner no longer estimates credits. Record account usage windows
separately and account for concurrent sessions. Included allowance and purchased
credits have different rules; use the
[official pricing guidance](https://learn.chatgpt.com/docs/pricing) and account
usage dashboard for those quantities.

## Results

No matched live results are recorded here. The October 3 changes were validated
locally without authenticated Codex CLI or ChatGPT-plan calls.

When measurements are available, record:

| Harness | Model | Mean fresh input | Mean cached input | Mean output | Unknown-usage runs | Mean wall time | Pass rate |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Codex CLI | pending | pending | pending | pending | pending | pending | pending |
| DaVinci | pending | pending | pending | pending | pending | pending | pending |

Also record the DaVinci commit SHA, Codex CLI version, task definitions, and any cases where DaVinci is worse. Do not infer or fabricate provider cache hits from structural cache keys.
