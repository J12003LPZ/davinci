# Design quality evaluation protocol

This protocol is for a separately authorized live campaign. CI validates the frozen manifest and result schema offline; it never starts a provider request or manufactures a human preference score.

## Frozen inputs

`manifest.json` contains fourteen briefs spanning landing pages, product UI, documents and slides, plus missing-browser, hostile-network, revoked-write and mocked-checkout negatives. Each brief includes content constraints, expected states, viewport requirements and a SHA-256 reference to `fixtures/reference.md`. The fixture contains no credentials or customer data. Change the manifest version and reference hashes before using different inputs; retain previous results.

Before a campaign, record the exact old-workflow baseline and Taste-integrated candidate source/profile versions. Use the same authorized subscription model, effort, input content, runtime, viewports and finite root allowance for both arms of a pair. Freeze per-arm request, token and deadline limits before dispatch. Include planning, critique, retry, repair and unknown attempts in root accounting. Do not route either arm to an API-key account or silently change models.

## Execution and records

1. Obtain explicit campaign authorization and an allowed account/model/effort and finite budget. Record that authorization without storing credentials. Configure the real target environment and preflight confinement before requesting generations.
2. Run every frozen eligible brief in both workflows. Preserve failed, denied, cancelled and incomplete runs. Negative briefs must retain their required incomplete/denied outcome. Record source, profile, runtime, fixture and evidence hashes, baseline/candidate identity, actual model attempts, unknown usage, repairs and measured latency.
3. For each output, run deterministic source and security gates, all required viewport/state captures, and the specified primary interactions. Validate the source-to-capture binding. Missing capabilities remain unknown/incomplete; a JSON snapshot or fake browser is not render evidence. Check production integration on the actual target, independently of any mock preview.
4. Randomize A/B presentation order with a recorded seed, conceal workflow labels from human reviewers, and use the same viewport and task rubric. Review content fidelity, hierarchy, spacing, typography, responsive behavior and interaction defects. Retain ties and reviewer disagreements. Preference is recorded only when both arms pass mandatory gates and have identified evidence.
5. Validate each `DesignEvaluationRecord` with its `validate()` method in `davinci_evals::design_quality`. A missing preference or latency is `null`, not zero; unknown usage is explicit. The current record's `requests` and `repairs` describe one bounded operation (maximum 12 and 2). Store each arm's counts separately in the campaign ledger rather than summing two roots into a single operation field.
6. Report all briefs, gate failures, unavailable dimensions, ties, actual preference counts and observed latencies. Include denominators and raw records. Do not infer product quality from manifest/schema tests. Security, source fidelity and mandatory interactions must pass regardless of preference.

Poor preference outcomes require a new versioned profile and a new complete campaign; do not remove difficult briefs or select only successful examples. No live campaign, model-cost measurement or human preference review was performed during initial implementation. The authoritative local execution status is in [readiness](../../readiness/design-artifacts.md).
