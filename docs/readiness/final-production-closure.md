# Final production closure protocol

This closure continues PR #82, initially pinned at
`dd6725f758a3619892349c59664f1d3f1a5e53fd`, with audit dependency
`ae96d00d715438bc620094da74d8f5d6e3fb73ca` verified as an ancestor.
It does not authorize publication or establish superiority over Codex.

## Confirmed corrections

- Nested Codex sampling spans now retain the nearest logical request identity.
  Traversal continues to classify prewarm traffic and reject cycles or missing
  ancestry. Genuine retries remain grouped; raw provider attempts are unchanged.
  The regression includes two nested siblings, a retry, prewarm, and malformed
  ancestry. It failed before the fix and all 13 accounting tests then passed.
- Standard unittest `OK (skipped=N)` receipts now retain actual passed and skipped
  counts. All-skipped and zero-test runs receive no passing verification credit.
  Invalid counts and failed summaries remain unrecognized as success.
- Unfiltered root `python -m unittest discover` now receives full-suite scope.
  Focused unittest selectors and discovery patterns cannot claim the whole source
  tree merely because they name a test directory. Both defects have RED/GREEN
  regressions; the targeted verification group passed 46 tests.

Historical completion-metrics traces included focused discovery and compound
`cd scripts/bench/tests && python -m unittest discover` commands. The former
does not prove every changed module; the latter does not yield a simple runner
receipt in the current conservative parser, and discovery below the workspace
does not establish root scope. The skip-parser defect therefore was not the
sole explanation of their final unverified state. Freshness, scope, terminal
status, and post-edit invalidation remain mandatory. That historical task is
quarantined and excluded from this closure's live evidence.

## Offline safety evidence

`subscription_policy` fixtures accept intended Codex OAuth and reject API-key,
provider, model, body-model, effort, endpoint, transport, retry and output-cap
substitutions. `subscription_auxiliary_policy` exercises the actual completion
entry point in coding/retry/compaction/reviewer/background threads: a configured
API key is denied before transport and an invented worker root cannot replace
the process budget. `harness_process_budget` checks the budget at the wire.

`harness_root_budget` covers child allowance, resume, deleted/corrupt ledgers,
idempotent reconciliation, sent/unknown/failed/cancelled reservations, release
of unsent work and cold recovery. Unknown work cannot be released as unsent or
reset by reopening. Existing verification tests cover stale dispatch, later
mutation, background work, and failed scopes surviving unrelated passes.
Complete workspace validation is required in addition to these focused checks.

## Fixed acceptance boundaries

No API spend authorized. Subscription-only campaign. Budget by subscription
usage allowance, request count, task count, and wall-clock time; API-equivalent
dollars are reporting-only. Production stays `gpt-6-luna` / `high` on
`openai-codex` / `openai-codex-responses` using ChatGPT subscription OAuth.
Quota, authentication, transport or accounting failure never changes that route.

Exactly six qualified development tasks; at most six primary launches plus four
matched controls/retests, ten total. Each run has 40 provider attempts and 600
seconds; the live campaign has 6000 seconds. No cap increases. Every task needs
an explicit contract, behavioral starter RED, reference GREEN and regression
GREEN before launch. No holdout access. No controls for successful tasks.

All final-source formatting, strict Clippy, locked/offline workspace fixture
tests, Python benchmark/identity tests, production build, lifecycle and identity
checks must pass before live work. Seal one clean source and binary; do not
rebuild between live runs. Six valid completed runs, at least four independent
successes, and no unresolved harness safety/accounting/verification defect are
required. Only then may an isolated public install/rollback drill proceed.
Missing required evidence means NO-GO. Actual final receipts, hashes, task
outcomes and the decision belong to the external closure evidence record;
this protocol does not itself assert that release gates passed.
