# OpenAI cache contract

Status: implementation baseline for the `openai-cache` branch.

Baseline commit: `6146d10fd9ffb92375e77f6c4deabac01fc9d50a`
Research/implementation date: 2026-09-21

## Invariants

- Provider cache identity never selects or owns a DaVinci conversation.
- Conversation lineage and WebSocket/previous-response continuation remain private to the authenticated conversation owner.
- OpenAI Responses usage is normalized into disjoint buckets: ordinary input, cache read, cache write, and output.
- A local prefix hash is diagnostic only; it is not proof of a provider cache hit.
- Unknown OpenAI Responses items and unknown fields must be preserved losslessly or rejected for replay.
- Unsupported/unknown backends do not receive speculative cache fields.
- Cache optimization cannot weaken tool authorization, retention/privacy requirements, or mutation recovery.

## Capability contract

Cache behavior is resolved from the existing authenticated backend/model capability detector. Do not enable features from model-name substring matching alone.

Public OpenAI Responses:
- Legacy models: deterministic prefix/key behavior only; no GPT-5.6-only fields.
- GPT-5.6+ profiles explicitly marked by catalog capability may use `prompt_cache_options` and content-block breakpoints.
- Breakpoints are carried by eligible `input_text` blocks.
- `mode=explicit` disables the provider's implicit breakpoint. With no explicit markers, no prompt-cache read/write optimization is requested by this policy.
- The current supported minimum cache lifetime is represented as `ttl=30m`.
- Lookback counts are provider-contract metadata, not a DaVinci correctness constant.

ChatGPT-backed Codex:
- Keep real session/conversation ownership separate from cache partition/affinity.
- New cache fields require adapter evidence and must not be generalized to public Responses, Azure, or custom proxies.
- Socket reuse and previous-response continuation are transport/conversation state, not cache-hit evidence.

Unknown/Azure/custom proxy:
- Conservative existing schema only unless a separately verified capability profile says otherwise.

## Usage contract

For normalized DaVinci usage:

```
U = ordinary uncached input
R = cache-read input
W = cache-write input
I = raw provider input = U + R + W
O = output
read_ratio = R / (U + R + W)
```

A zero denominator is "not available", not 0% evidence.

Responses decoding performs the inclusive-to-disjoint conversion exactly once. Downstream code must not subtract R/W again.

## Request identity classes

- cache partition: routing/accounting affinity only
- prefix fingerprint: ordered wire-visible cache-sensitive content
- conversation lineage: durable transcript/fork/worker ownership
- transport lease: authenticated live connection + continuation state
- request correlation id: diagnostics only

Changing a correlation id or timestamp must not change a reusable prompt prefix. Changing model-visible instructions, tools/schema, output format, or earlier input must.

## Fixture provenance

Fixtures in `crates/davinci-ai/tests/fixtures/openai_cache/` are sanitized deterministic contract fixtures. They do not prove live provider support or a cache hit. Live claims require provider usage/diagnostics and separately authorized benchmarks.
