# Targeted evaluation specifications

These are specifications, not executed live evaluations. Do not launch them
without separate bounded authorization. Reuse existing runners, fixtures,
operation journals and Verification Planner.

## Shared measurement contract

Freeze model, reasoning effort, provider route, task/independent grader,
permissions, session handling, binary/source/runner revisions and repetitions.
Compare corrected DaVinci with its pinned baseline, adding Codex only where
settings are meaningfully matched. Counterbalance order or interleave pairs.
Do not run concurrent campaigns when attributing wall-clock differences.

Record verified success; uncached input per verified success and per run;
total/cached input; provider cache share; first versus later request usage;
model requests; tool calls and actually executed operations; output tokens;
retries/failures; missing/partial usage; median and p90 time. Include failed
runs' consumption. Report uncertainty and missing-usage effects explicitly.
Withhold an improvement claim if missing evidence or variance prevents it.

No single cache percentage or tool count is sufficient. Completion must not
regress to manufacture savings. Subscription-limit savings are not inferred
from provider token counters.

## Context VM

Use 100–300 turns with at least two genuine compactions. Include requirement
changes, corrections, failed and successful verification, later source
retrieval, reload and branch selection. Run deterministic replay separately
from real agent execution.

Acceptance: required facts, source evidence and replay associations survive
all compactions; no unacceptable verified-outcome regression. Compare matched
provider usage for efficiency. Pre-compaction request parity is an integrity
gate, not evidence of long-session savings.

## Token Governor

Use repeated reads, command outputs over 100 KB and noisy test logs. Put
required error evidence near the beginning, middle and end. Retrieve exact
originals before and after compaction. Include storage failure and isolation.

Acceptance: preserve required errors and exact retrieval; transform only newly
inserted evidence, not earlier history. Count extra retrieval and reread
requests. Savings must be net, not displaced work.

## Code Mode

Use sizeable read-only investigations with independent acceptance evidence.
Match session persistence, authority and discovered tool exposure. Account for
schema, discovery metadata, script, envelopes, child calls and final results.

Acceptance: fewer model round trips or less uncached input without a verified
success regression. Parent/child journals and failures remain authoritative.
Do not force activation just to raise utilization.

## Stream recovery

Offline gate: eleven fault boundaries from the approved plan, returned-error
and raw-error paths, persisted associations, bounded attempts, cancellation,
revocation and actual side-effect counts. Malformed arguments with a valid
terminal boundary must not be mistaken for transport interruption. Missing
usage is independent of retry eligibility.

Live acceptance, only if authorized: fewer avoidable terminal failures on
matched workloads without duplicate operations or unsafe continuation. Cache
share need not move for recovery correctness to be valuable.

## Verification and worker handoffs

Bind acceptance requirements to current source and nonempty verification.
Reject stale, incomplete and zero-test receipts. Evaluate compact,
source-referenced handoffs on genuinely independent investigations, not
mandatory workers on short tasks. Do not add another completion engine.

## Benchmark integrity

Inspect the four universally failing task families' prompts, grader contracts,
fixture reference solutions and saved validation output. Do not weaken graders
to manufacture passes. A proven fixture/prompt correction must preserve an
independent holdout and explicitly change comparability. Record blockers when
no valid reference solution or compatible environment is available.
