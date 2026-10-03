# OpenAI prompt-cache benchmark and rollout

Status: offline benchmark contract implemented; no paid/live provider benchmark has been authorized or run by this plan execution.

## Purpose

The benchmark compares one cache optimization at a time against a frozen baseline while preserving correctness, isolation, provider accounting, and negative outcomes. Synthetic/fixture data and live provider data must never share an unlabeled aggregate.

The manifest records an exact 40-hex Git revision, provider, model, endpoint family, cache-policy contract revision, baseline and treatment profiles, one candidate feature, repetitions, and every case. Every result row carries the manifest fingerprint.

## Offline gate

Offline/loopback testing is the default and requires no provider credentials. Deterministic fixtures cover append-only reuse, prefix invalidation, unsupported backends, failed-task controls, replay serialization, usage accounting, worker isolation, and diagnostics parsing.

Failed, blocked, aborted, budget-exhausted, infrastructure-failed, and unverified runs remain in outcome denominators. Only mutually verified-success baseline/treatment pairs enter efficiency comparisons. Missing or duplicate pairs are published in the pair audit rather than silently dropped.

Cache-read accounting uses disjoint provider buckets:

```text
raw_input = ordinary_input + cache_read + cache_write
read_ratio = cache_read / raw_input
```

Cache writes are retained in resource totals and long-input pricing is selected from raw provider input.

## Live authorization gate

Live runs are deliberately not started by the eval module. Before any live runner is permitted, all of the following must be supplied deliberately:

- an explicit approval identifier;
- exact provider and model matching the benchmark manifest;
- a nonzero maximum request count;
- a finite, nonzero maximum cost in USD;
- the same approval identifier through the operator-controlled `DAVINCI_OPENAI_CACHE_LIVE_AUTHORIZATION` input (or an equivalent explicit host action that calls the same validator).

The authorization object is a gate, not a credential. API keys, raw prompts, opaque reasoning items, and secrets must not be written to benchmark artifacts.

No live benchmark authorization was supplied during this implementation, so T10 stops at the offline gate.

## Paired benchmark protocol

For each case and repetition:

1. Reset to the same workspace/task fixture and exact manifest revision.
2. Run the baseline profile and collect verifier-owned outcome, usage, attempts, tool calls, side-effect count, latency, provider diagnostics when available, and termination reason.
3. Reset again and run the treatment with exactly one cache feature changed.
4. Keep failed outcomes in the report. Do not substitute assistant claims for verifier evidence.
5. Mark missing, duplicate, or mixed-source pairs explicitly.
6. Compare efficiency only for mutually verified-success pairs.
7. Publish total request/cost/token usage across all rows, not just successful pairs.

Cold and warm sequences must be labeled separately. Small samples must not be described as stable p95 measurements.

## Promotion

Promote one switchable feature at a time. A feature is eligible only when all hard correctness/isolation gates pass, paired verified-success efficiency does not regress outside the predeclared tolerance, and outcome denominators do not worsen. Rollback selects the previous tested compatibility profile; rollback must not discard acknowledged tool results, weaken permissions, or merge conversation ownership with cache affinity.

Provider cache diagnostics, WebSocket reuse, local evidence-cache hits, and local prefix fingerprints are separate signals and must remain separate in reports.
