# OpenAI cache and Responses efficiency

This guide describes the OpenAI cache behavior implemented on the `openai-cache` branch. It is an operator guide, not evidence that a provider cache hit occurred. The normative invariants are in [the OpenAI cache contract](cache/openai-cache-contract.md).

## Supported route matrix

| Route | Cache request behavior | Native replay | Notes |
| --- | --- | --- | --- |
| Public OpenAI Responses with the verified explicit-boundary capability | Capability-scoped `prompt_cache_key`, content-block breakpoint, and `prompt_cache_options` | Yes | The current explicit profile uses a stable trusted bootstrap boundary and `ttl: "30m"`. |
| Older public OpenAI Responses | Existing deterministic key/prefix behavior | Yes | GPT-5.6-only fields are omitted. |
| ChatGPT-backed Codex Responses | Adapter-specific affinity and WebSocket/SSE continuation | Yes | Cache affinity is separate from conversation/session ownership. Public Responses diagnostics are not assumed to apply. |
| Azure OpenAI Responses | Existing conservative schema | Yes where the Responses item contract is valid | No speculative public-OpenAI explicit-boundary fields are added. |
| Custom/unknown proxy | Conservative existing schema | Only when the native Responses contract is valid and replay validation succeeds | A deceptive OpenAI-like hostname does not enable new cache capabilities. |

Capabilities are resolved through the authenticated backend/model capability path. Model-name substring matching alone is not sufficient.

## Request shaping

For a verified public OpenAI explicit-boundary profile, DaVinci moves a stable trusted bootstrap into a `developer` message with a breakpoint on its `input_text` block. The rest of the conversation remains ordered after that stable prefix.

The compatibility `cache_retention` setting behaves as follows:

- `short` or `long` on the verified explicit-boundary profile may use the stable bootstrap breakpoint and the supported minimum TTL.
- `none` on that profile emits explicit mode with no breakpoint and no cache key, disabling the provider's implicit breakpoint for that request.
- On older implicit contracts, `none` is only best-effort. It is not a privacy guarantee about undocumented provider behavior.
- Unknown backends do not receive new cache fields.

The pure `PromptCacheWirePlan` is the request-shape authority for these choices.

### Rollback switches

The cache optimizations are independently switchable and default to enabled on this branch:

- `PI_OPENAI_CACHE_EXPLICIT_BOUNDARIES=0` rolls verified public Responses requests back to the legacy/provider-default cache shape for normal cache-enabled requests. It does **not** weaken `cache_retention=none`; strict disable still uses the verified explicit-mode contract.
- `PI_OPENAI_CACHE_NATIVE_REPLAY=0` disables reuse of durable native Responses replay state and falls back to normal full request construction.
- `PI_OPENAI_CACHE_WORKER_AFFINITY=0` stops installing worker bootstrap cache partitions. Worker conversation/session isolation is unchanged.
- Provider comparison diagnostics remain separately sampled by `PI_OPENAI_CACHE_DIAGNOSTICS_EVERY_N`; `0` disables comparison sampling.

`/cache-status` reports the effective runtime feature-switch state. These switches change optimization only; they do not widen permissions, merge conversation ownership, or discard acknowledged tool state.

## Identity and ownership

DaVinci deliberately keeps these identities separate:

- **Cache partition / affinity:** routing and accounting affinity.
- **Ordered prefix fingerprint:** a privacy-safe SHA-256 fingerprint of cache-sensitive wire segments.
- **Conversation lineage:** the durable session/fork/worker owner.
- **Transport lease:** an authenticated live socket and its continuation state.
- **Request correlation ID:** diagnostics only.

Changing a cache partition does not change the prompt-prefix fingerprint. Changing model-visible instructions, tool schemas, output configuration, or earlier input does.

Graph workers do not inherit a root conversation merely because they share a cache key. A real session-owned root may align provider affinity with its cache partition; worker processes without a session ID cannot use that partition as continuation ownership.

## Durable native Responses replay

Responses output is retained as native provider JSON beside the generic UI transcript. Known items containing unknown provider fields fall back to raw JSON so replay does not silently discard future or opaque fields.

A successful native turn can be persisted in the session as a versioned custom record containing:

- exact prior request input items,
- exact native provider output items,
- the terminal response envelope/ID when available,
- the prepared wire manifest,
- the provider-message projection fingerprint used for recovery.

On a later request, full valid-context replay is used only when:

1. the durable record belongs to the same session,
2. the saved provider-message prefix still matches,
3. the stable model/tool/trusted-instruction contract still matches, and
4. the prior response completed in a resumable state.

If any check fails, DaVinci falls back to the normal full request. Aborted, failed, corrupt, or unterminated responses do not become resumable state.

## Usage and pricing

Normalized OpenAI Responses usage uses disjoint buckets:

```text
U = ordinary uncached input
R = cache-read input
W = cache-write input
I = raw provider input = U + R + W
O = output
read_ratio = R / (U + R + W)
```

A zero denominator is reported as unavailable rather than as measured 0%.

Long-input catalog pricing is selected from the raw provider input total before cache buckets are separated. Custom model pricing is kept separate from built-in catalog tiers so a custom model cannot accidentally inherit a built-in threshold.

## Diagnostics

`/cache-status` and the RPC status surface report provider usage separately from local cache/runtime observations. Provider cache diagnostics, when supported by the verified capability, are evidence about the provider comparison only.

Local prefix fingerprints, Context VM affinity, WebSocket reuse, and previous-response continuation are **not** labeled as provider cache hits.

Cache-miss notices derived locally are explicitly described as estimates. Idle time alone is not treated as proof of provider cache expiry.

## Context VM and workers

Context VM uses an epoch-scoped cache namespace that remains stable while compatible content evolves. A separate content fingerprint records actual prefix changes. A fold rotates the epoch namespace.

Graph workers use bounded context packets and private durable worker sessions. They do not clone the full parent transcript. Worker cache identities are derived from the real resolved provider/model and stable bootstrap inputs; missing model identity is not replaced by an invented default.

## Offline evaluation

`davinci-evals::openai_cache_eval` provides deterministic paired baseline/treatment reporting. Offline fixtures are the default and never call a provider.

Efficiency metrics include only pairs where both baseline and treatment have a verified-success outcome. Failed, blocked, missing, duplicate, or mixed-source pairs remain visible in the pair audit and cannot improve the efficiency result by disappearing from the denominator.

## Live evaluation authorization

Live cache benchmarks are not run implicitly. A live runner must first pass the explicit authorization contract:

- exact provider and model,
- explicit approval ID,
- nonzero maximum request count,
- finite positive maximum cost,
- exact pinned git revision and benchmark manifest.

The approval ID must also be supplied through `DAVINCI_OPENAI_CACHE_LIVE_AUTHORIZATION` and match the manifest. Offline and live rows cannot be mixed in one report.

Do not claim provider cache-hit rates, latency improvement, or monetary savings from offline fixtures. Those claims require separately authorized live runs using provider-reported usage/diagnostics and the paired benchmark contract.

## Useful validation targets

The implementation is covered across the existing workspace package tests, including focused OpenAI cache tests in `davinci-ai`, worker/recovery tests in `davinci-coding-agent`, and the paired benchmark contract in `davinci-evals`.

For local validation with the repository-pinned toolchain and cached dependencies, use the repository's normal offline/locked Cargo workflow. Live provider calls are a separate, explicitly authorized activity.


## Cache-stable turns

On `OpenAiReasoning` routes, DaVinci now keeps provider `instructions` stable across ordinary user turns. Stable prompt modules and user session appends remain in `instructions`; runtime permission state, capability state, plan-mode state, living-plan revisions, and injected memory are persisted as hidden `davinci.turn_context` messages after the active user turn. Non-OpenAI prompt families retain the previous system-prompt behavior.

This is intentionally append-only. A new turn may add new provider input, but it should not rewrite earlier input merely because permission state, the active capability set, memory, or the living plan changed.

Cache-sensitive routes also:

- use the cached pruning profile, starting at 65% of the context window and targeting 35%;
- freeze the authorized provider tool schema before dispatch so later `tool_search` activation does not mutate the leading `tools` array;
- support configurable OpenAI text verbosity and reasoning-summary output.

### Cache-stability controls

- `DAVINCI_TURN_CONTEXT=system` restores per-turn state to the system prompt. `appended` forces the cache-stable path.
- `DAVINCI_PRUNE_PROFILE=default|cached|off` selects legacy pruning, cache-aware pruning, or disables pruning.
- `DAVINCI_OPENAI_VERBOSITY=low|medium|high` controls Responses `text.verbosity`.
- `DAVINCI_REASONING_SUMMARY=auto|concise|detailed|none` controls `reasoning.summary`; `none` omits the field.

### Measuring the effect

Run `node scripts/measure-codex-cache.mjs <session.jsonl>` after a repeated multi-turn Codex session. From request 2 onward, a cache-stable run should report cache reads covering the unchanged prefix instead of falling back to only the stable bootstrap after mode/capability changes.

The repository does not commit fabricated before/after numbers. Live measurements require an authenticated ChatGPT Codex session and should be recorded only from an actual run.


## Subscription-efficiency implementation status

Prepared on 2026-09-24 from the current `main` branch.

### Implemented

- Stable OpenAI reasoning instructions with appended per-turn runtime state, plan-mode state, living-plan revisions, and memory.
- Cache-aware pruning and a fixed authorized provider-tool prefix on cache-sensitive routes.
- Configurable OpenAI verbosity and reasoning summaries.
- Per-assistant native Responses item persistence and replay for the same provider/model.
- A hidden `--codex-probe` maintainer command and sanitized evidence document.
- Capability-ready Codex cache-policy plumbing. New explicit cache options remain disabled unless model compatibility data is backed by a successful probe.
- ChatGPT Codex usage-window parsing, threshold warnings, `usage_limit_reached` retry suppression, and `/cache-status` visibility.
- Economy-model routing for read-only graph roles during `openai-codex/*` sessions. Set `graphEconomyModel` or `DAVINCI_GRAPH_ECONOMY_MODEL`; use `off` to disable it.
- Accumulated provider USD cost in cache telemetry and an explicitly labeled ChatGPT-plan credit estimate in `/cache-status`.
- Cache measurement and Codex CLI comparison scripts.

### Awaiting live backend evidence

The following plan items are intentionally not enabled until the explicit authenticated backend probe records support:

- prompt-cache TTL and explicit bootstrap breakpoints in the model catalog;
- freeform grammar `apply_patch` request wiring;
- `additional_tools` late-tool delivery;
- cache-sharing compaction using `tool_choice: "none"`;
- the remote `/responses/compact` decision;
- final Codex usage-header names if the live backend differs from the conservative parser names.

Live before/after cache measurements and Codex CLI comparison results are also pending. The implementation does not commit synthetic benchmark numbers or claim probe acceptance without an authenticated run.
