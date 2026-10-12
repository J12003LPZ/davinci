# Cache evidence

## Campaign identity

The original paired campaign is under the local benchmark root,
`luna-high-recovery-02/vm-off-cm-off+codex`. Its `campaign.json` records:

- DaVinci source `f9123d153fcf83e15b599576c27faa69e35ea72f`, version 1.1.6.
- Pinned binary SHA-256
  `d67211ba48c86f72b0e7e4cbd8c53af8c97350a098d092164a1433185026ce44`.
- Codex CLI 0.162.1, source revision unavailable.
- Runner `6995ad02a0a169ee4b72cb84af8b657ea60aa4dd`.
- Fixture hash
  `41b2e52aab9e8ac0edacf3655809227b2b6b7822d775a982d9b4fa1f5d652366`.
- `gpt-6-luna`, high effort, default service tier, counterbalanced order.
- VM Off, Full tools, stable prompt, automatic verification.
- DaVinci `--no-session`, Codex `exec --ephemeral`; no explicit Governor
  override was established from campaign settings.

Identity is recorded campaign evidence, not proof the current installed binary
matches. Product-source comparison against the task checkout found only three
benchmark-runner files different.

This campaign is diagnostic-only: native host access and public generators
containing grader/reference solutions preclude promotion-grade holdout claims.
An offline hash check matched 89 frozen fixture files across seven inspected
families. It does not repair the assurance limitation.

## Separate completed capture campaign

The existing `luna-high-wire-01` campaign was inspected, not launched, stopped,
altered or counted as a newly authorized measurement. Its plan/progress record
eight measured runs, seven passes; a separate smoke is excluded.

Its `wire-baseline/campaign.json` records runner
`0473000e03c7e97502102d5a81b0e0e3b8cc730f`, fixture hash
`02ae43b86a8b36f217280cf37bd2afb03ce9cda5d5dd6e91c57928ddd946e083`,
and the same pinned product source/binary.

Across `wire-baseline/wire`:

- 85 logical captures and 85 usage sidecars.
- 77 consecutive transitions preserve every earlier input item exactly.
- All 77 include the exact acknowledged native assistant block from stdout.
- Function-call/output IDs associate without duplicate output IDs.
- Sidecar usage agrees with complete stdout attempt usage in all 85 captures.
- Instructions, ordered schemas, model, effort and cache key remain fixed
  within each task; instructions/schemas/key also remain fixed across tasks.
- 111 tools, sorted by name; no observed compaction or historical pruning.

Safe canonical JSON SHA-256:

| Component | Digest |
|---|---|
| Instructions | `55bde5fddb782b749743a9111d55886dfaf22551724f2f2c427e4e9bd2c36f96` |
| Tools | `d9bd448fd9d5494999212b09a78a17bd0cc05442c5e893cc20072ebddcf8825d` |
| Cache key | `9e6dbee074d29829907ba6dada76e8e8862bd7ba297a4633e947ead3cf69e458` |

### Concrete stable transition with zero caching

In `wire-baseline/wire/t4-rename-r0`:

| Logical capture | Input items | Prior retained items | Input / cached / output |
|---|---:|---:|---:|
| `0001-11080` | 2 | initial | 10823 / 0 / 53 |
| `0002-11080` | 6 | 2 | 11041 / 8704 / 120 |
| `0003-11080` | 17 | 6 | 11424 / 8704 / 662 |
| `0004-11080` | 28 | 17 | 12150 / 8704 / 103 |
| `0005-11080` | 35 | 28 | 12407 / 8704 / 65 |
| `0006-11080` | 37 | 35 | 12939 / 8704 / 301 |
| `0007-11080` | 40 | 37 | 13256 / 9728 / 239 |
| `0008-11080` | 45 | 40 | 13545 / 0 / 137 |
| `0009-11080` | 52 | 45 | 14747 / 10752 / 251 |
| `0010-11080` | 57 | 52 | 15099 / 9728 / 65 |

Files append `-logical.json` and `-usage.json` to those identifiers. Transition
7→8 retains all 40 previous items and stable segments despite caching dropping
to zero. Its raw file-byte common prefix is 27,621 bytes, **not** a
model-visible token-prefix measurement.

Capture totals: input 1,052,535; cached 707,072; uncached 345,463;
output 16,540. Initial input/cached: 86,527 / 53,248.
Initial cached share 61.539%; later share 67.683%; overall 67.178%.
These are existing observations, not after-fix performance results.

## Unknowns and decision

No final HTTP wire files or matching Codex request bodies are available.
Logical captures precede final request preparation. Headers, real transport
lineage, routing, provider cache lifetime/eviction/admission and model-token
prefixes remain unobserved.

**No unexpected logical divergence was found. Prefix mutation is not
established as the cause of the provider gap.** Do not re-sort tools,
routinely rewrite history, add unsupported cache fields, prewarm, or infer a
WebSocket/cache cause from transport differences alone.

Use the sanitized offline report on existing captures only. New raw capture
creation or live probes require separate authorization and access controls.
