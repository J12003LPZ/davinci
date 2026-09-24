# DaVinci vs Codex CLI subscription-efficiency comparison

Status: **pending live measurement**

Prepared: 2026-09-24

The comparison runner is `scripts/compare-codex.mjs`. A valid result requires:

- the same model for Codex CLI and DaVinci;
- fresh copies of the same fixture repository;
- at least five mixed-size tasks;
- three runs per harness per task;
- task-specific check commands that return success only when the requested work is complete.

Before collecting the full sample, run one task per harness and verify that both JSON usage pickers report non-zero input/output values. If either CLI has changed its JSON event shape, update the picker before recording results.

## Results

No results are committed yet. The connected development machine was offline while this branch was implemented, so no authenticated Codex CLI or ChatGPT-plan calls were made.

When measurements are available, record:

| Harness | Model | Mean fresh input | Mean cached input | Mean output | Estimated credits | Mean wall time | Pass rate |
| --- | --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Codex CLI | pending | pending | pending | pending | pending | pending | pending |
| DaVinci | pending | pending | pending | pending | pending | pending | pending |

Also record the DaVinci commit SHA, Codex CLI version, task definitions, and any cases where DaVinci is worse. Do not infer or fabricate provider cache hits from structural cache keys.
