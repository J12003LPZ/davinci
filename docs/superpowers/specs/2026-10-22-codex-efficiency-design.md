# Da Vinci: Evidence-Gated Codex Efficiency Design

**Date:** 2026-10-22  
**Status:** Proposed design for owner review; no implementation authorized  
**Target:** `J12003LPZ/davinci`, current default branch at implementation preflight (pin exact SHA before coding)  
**Owner objective:** Maximize **verified coding tasks completed per usable ChatGPT Codex subscription allowance**, without weakening correctness, permissions, recovery, or user intent. Secondarily minimize uncached input, model attempts, latency, API cost on separately authorized API routes, and local memory/CPU overhead.

## 1. Basis and evidence boundaries

This design uses the user's 32-page report, *Optimizing Codex/OpenAI Performance and Cost Efficiency for the Da Vinci Terminal* (especially pp. 1–4, 9–17, 21–31), and the publicly readable Da Vinci repository documentation/source checked October 22, 2026. The report explicitly states that it **did not establish line-by-line current implementation gaps** (p. 32), so generic recommendations are not evidence that Da Vinci lacks a feature.

Repository evidence:

- [Context VM semantics](https://github.com/J12003LPZ/davinci/blob/main/docs/context-vm.md): `off`, `shadow`, `active`, and `hybrid` already exist. Hybrid sends the append-only Off-like transcript and uses a Context VM ledger when compacting; Off still can prune and compact. Hybrid is **not** a new KV cache.
- [Context VM measurements](https://github.com/J12003LPZ/davinci/blob/main/docs/context-vm-measurements.md): in the 300-turn **offline** fixture, prefix-reuse estimates were Off 98.4%, Hybrid 98.3%, Active 61.2%; requirement retention Off 394/883, Hybrid 883/883, Active 883/883 under a deliberately weak summarizer. These are NOT OpenAI cache reads or live task success. The October 19 lifecycle fixture achieved 883/883 for all three modes under different test conditions, so results must remain labeled by fixture and date.
- [Efficiency diagnosis, October 12](https://github.com/J12003LPZ/davinci/blob/main/docs/diagnostics/2026-10-efficiency-diagnosis.md): a requirement-review continuation accounted for approximately 47% of requests and 61% of uncached input in the measured baseline. An off arm reduced uncached input 62.8% in a small matched screening sample with no observed pass drop (23/24 versus 20/24). This is NOT sufficient evidence to disable safety review globally. Those runs did not compact or prune, so Hybrid's measured cost gap there was not caused by mode-specific wire differences.
- [Completion review](https://github.com/J12003LPZ/davinci/blob/main/docs/completion.md): current logic already avoids a review after certain verified checks and bounds it to one per prompt. That skip shipped in `8a87a98e` on Oct 12, **after** the diagnosis baseline, so the 47%/61%/62.8% figures describe the pre-fix harness. Post-fix (Oct 17 public tasks, 24 runs): a gate reminder fired in 9 runs, and 30% of requests came after one; in every such run both `completion.requirements` and `verification_required` fired, so the requirement review's own residual share is not yet known. `DAVINCI_REQUIREMENT_REVIEW` provides an A/B switch. It must not be confused with `autoVerify` or completion hooks.
- [Fixed versus adaptive effort](https://github.com/J12003LPZ/davinci/blob/main/docs/perf/request-efficiency/A1.md): adaptive effort produced more uncached input and two task regressions in the screened campaign. Keep `fixed` until new evidence reverses this.
- [OpenAI cache contract](https://github.com/J12003LPZ/davinci/blob/main/docs/cache/openai-cache-contract.md) and [OpenAI efficiency guide](https://github.com/J12003LPZ/davinci/blob/main/docs/openai-efficiency.md): separate provider cache affinity, ordered prefix fingerprint, authenticated conversation lineage, transport lease, and request ID. Never equate local prefix similarity, a reused socket, and a provider cache hit.
- [Readiness execution record](https://github.com/J12003LPZ/davinci/blob/main/docs/readiness/openai-harness-implementation.md): durable root-budget accounting, explicit subscription-only campaign gates, verification receipts, and known/unknown usage states already exist. Extend these; do not add competing schedulers.
- [Existing retrieval and memory](https://github.com/J12003LPZ/davinci/blob/main/docs/vector-memory.md), [tool discovery](https://github.com/J12003LPZ/davinci/blob/main/docs/tool-discovery.md), and [resource lifecycle](https://github.com/J12003LPZ/davinci/blob/main/docs/perf/resource-lifecycle.md): local vector memory, Lean tool surface, `EngineeringSnapshots`, `CacheRuntime`, bounded native caches, and compressed recoverable output are existing components, not blank-slate features.
- [Cache benchmark contract](https://github.com/J12003LPZ/davinci/blob/main/docs/cache/openai-cache-benchmark.md): offline gates, strict paired analysis, route-specific live authorization and rollback constraints already exist.
- [ChatGPT Codex backend route](https://github.com/J12003LPZ/davinci/blob/main/docs/cache/chatgpt-backend-route.md): the `openai-codex` login sends to `chatgpt.com/backend-api/codex`, but **no live DaVinci request has used it**. Every live result above was measured on the public `api.openai.com/v1` route. Whether the backend route reports usage and cached tokens is unknown, so the first work item is a capped, approved smoke test of that route (plan Task 0).
- Oct 17 public-task comparison (48 runs, direction only): DaVinci used 0.93x Codex's tokens per success at similar passes (21/24 vs 22/24); median wall time was 1.81x, 98% provider wait, attributed to the public route generating about 3x slower. Token efficiency is therefore not the user-visible gap; this design targets subscription allowance per success, not wall time.
- Existing paired analysis: `campaign.paired_metrics`, `readiness_protocol.validate_pairs`, `bench.summarize`/`paired_report` and `readiness_metrics.summarize` already handle pairing, frozen identity and unknown-as-`None`. Only per-verified-success denominators are missing.

## 2. Non-negotiable requirements

1. Preserve current default **Context VM Off**; Hybrid is an opt-in candidate; Active and Shadow remain available. No automatic mode switch until a separate qualified promotion decision.
2. Preserve selected model and reasoning effort in ordinary Codex-subscription sessions. No silent model downgrade, API-key routing, alternate endpoint, billing-route fallback, or invented provider capability.
3. Do not implement a local model KV cache. OpenAI owns provider-side KV reuse. Da Vinci owns cache-friendly request assembly, context selection, durable evidence and safe **application-level** reuse.
4. Keep the authenticated ChatGPT Codex backend distinct from public OpenAI Responses, Azure, Anthropic and custom proxies. `prompt_cache_options`, explicit breakpoints, TTL, remote compaction and prewarm remain capability-gated; no speculative fields are sent.
5. Never weaken permission checks, sandbox policy, actual test receipts, session/worker ownership, cancellation, restart/recovery, provenance, user corrections, or complete native tool-call pairs.
6. Never treat unknown usage as zero. Keep raw input, cache-read, cache-write, ordinary uncached input, output, reasoning, provider attempts, auxiliary model calls, wall-time and verification work separately; avoid double counting.
7. Do not modify user's local configuration, stored sessions, credentials, developer instructions, or installed CLI as a side effect of designing/evaluating. Production changes require explicit authorization.
8. Offline fixture and diagnostic campaigns may run without provider traffic. Live subscription experiments need prior explicit approval, a sealed source and binary, isolated credentials, hard caps and an operator-held stop mechanism.
9. Keep repeated data private: no raw prompt, code, secrets or opaque reasoning in benchmark summaries or Git. Wire dumps remain outside the repo, access-controlled, with bounded retention.
10. Respect the repository's current toolchain, lockfile, supported OSes and source convention as confirmed at preflight; no dependency upgrades solely for efficiency experiments.

## 3. Architecture and responsibility boundaries

```text
                       User request / task contract
                                    |
                    Existing agent loop / permissions
                                    |
              Existing retrieval and context decision owners
                   |                           |
       Repo index / lexical+symbol       Vector Memory / broker
                   |                           |
                   +-------- relevant evidence +
                                    |
            Existing Context VM mode: Off | Shadow | Hybrid | Active
                |   stable instructions / tools first
                |   append-only native items where permitted
                                    |
         Existing capability-scoped provider cache policy / transport
                                    |
              Codex OAuth / public Responses as selected
                                    |
                Existing deterministic verification
                                    |
                   pass -> report verified result
                   fail -> focused evidence / bounded repair
                                    |
               Existing budget, telemetry, eval harness

Cross-cutting: permission scope, freshness, provenance, cancellation,
            session isolation, unknown accounting, rollout/rollback
```

The design adds no new always-on classifier or reviewer. Its first changes are **diagnostic and conditional**, applied to existing owners. Only introduce a new memoized artifact when a measured repeated operation lacks an existing correct owner.

## 4. Competing approaches considered

**A. Cache-first rewrite / universal context gateway:** might centralize policy, but risks new state, invalidation complexity and broken replay. **Reject** without proof of missing capability.

**B. Disable requirement review and enable Hybrid globally:** could save requests quickly, but risks missed user constraints and unverified changes. **Reject** until independent private holdout evidence.

**C. Evidence-first, bounded improvements inside existing owners (selected):** keep old defaults, instrument and measure, remove measured redundant turns, evaluate Hybrid at real compaction boundaries, reuse proven local facts, then promote only independent wins. Slower to declare victory, safer and more credible.

## 5. Work packages and public interfaces

### A. Measurement and completion policy

First confirm the backend route reports usage (plan Task 0). Use existing `scripts/bench/bench.py`, `scripts/bench/campaign.py`, `scripts/bench/turn_cache_audit.py`, `scripts/bench/cache_report.py` and `crates/davinci-agent/src/stats.rs`. Extend `campaign.paired_metrics` with per-verified-success denominators rather than adding a new analysis layer. A separate completion policy experiment varies **only** the existing `requirementReview` switch, then assesses whether a narrower existing condition is worth changing. Independently, close the documented gap where a workspace-wide `cargo check`/`clippy` counts as `verified` and suppresses the review (`docs/completion.md`), which tightens review. This work package is the priority because completion-time gates are the only cost with direct evidence, but its size must first be re-measured: attribute post-reminder requests per gate from existing post-fix logs before any policy work, and judge any policy change on multi-requirement tasks, which is where the review is meant to earn its cost.

Work packages B, C and D are **deferred**: each starts only when a measurement names it as a material cost (compaction cost on real tasks; retrieval or repeated reads; repeated-failure tokens). On the Oct 17 runs tools took under 1 s and 98% of wall time was provider wait.

### B. Cache mode and compaction decisions

Reuse `crates/davinci-agent/src/runtime/context_vm/`, `crates/davinci-agent/src/prepared_context.rs`, `crates/davinci-agent/src/provider_budget.rs`, `crates/davinci-ai/src/openai_cache_policy.rs`, and `crates/davinci-agent/src/runtime/cache/identity.rs`. Run a paired Off/Hybrid/Active/Shadow fixture with actual compaction, cold resume and branching, then an approved live run if warranted. Do not modify request serialization for alleged cache gains without a measured request-shape defect; preserve native Responses items and reasoning replay.

### C. Retrieval and safe local reuse

Use `crates/davinci-coding-agent/src/native_extensions/repo_intelligence/`, `native_extensions/vector_memory.rs`, `native_extensions/engineering_snapshot.rs` and existing runtime `CacheRuntime`. Evaluate lexical-symbol, semantic and hybrid retrieval under a fixed budget. Avoid mandatory embedding service. Add a narrow content-addressed cache only for deterministic, read-only artifacts with explicit input identity and permission/freshness enforcement; otherwise prefer reuse of the existing snapshot and native cache.

### D. Verification and optional routing

Keep `autoVerify`, verification evidence, repair/stop loops and selected Codex model/effort unchanged by default. Test whether bounded, reversible failure evidence reduces retries. Evaluate Lean tools and routing/effort **offline** as independent ablations; do not promote adaptive effort or automatically switch models without a new, complete, high-quality result.

## 6. Metrics and acceptance definitions

Primary metric: **independently verified successful tasks per available subscription usage**, only when an account-side allowance can be reliably observed. Otherwise report this as **unavailable** and use (1) total actual model attempts, (2) uncached input tokens per independently verified success, (3) all-model output tokens per success, and (4) end-to-end time per success as *diagnostic proxies*, NOT subscription-quota equivalents.

Other required metrics: success rate (all attempts counted), first-attempt success, unrelated-edit/regression incidence, exact requirement retention, permission/recovery violations, prompt-cache read fraction (only provider-reported), cache accounting completeness, TTFT (when observed), p50/p90 wall time, fold/summarizer requests, retained bytes, memory, and root budget exhaustion. Provider-side cache reads may be rounded/quantized; do not back-calculate unseen KV entries.

**Proposed promotion gate (must be frozen before private live runs):** no new security, recovery or missing-requirement failure; no unexplained parent-pass/candidate-fail pair; complete applicable accounting; at least a meaningful, predeclared **10–15% improvement** in provider requests or uncached tokens per verified success on a matched development set with uncertainty estimates; no >10% p90 latency regression. These are proposed decision thresholds, not observed results or statistically justified guarantees. Holdout runs must be independent, sealed and outcome-complete; any ambiguous evidence means NO-GO. The existing benchmark/release rules take precedence when stricter.

## 7. Study design

Phase −1 is the only live step before engineering: at most 5 approved requests on the backend route to record which usage fields it returns. A route that returns no usage stops this design. Phase 0 uses no paid model calls: freeze source/config/fixtures, run provider-free validation, characterize 4 context modes and requirement-review on/off, and audit accounting completeness. Include short and long tasks, compaction at least twice, cold/warm sessions, branch/resume, failed verification, cancellation, wrong permission, stale file and malformed native replay. Vary one condition per paired experiment; reset workspace and caches using clearly documented cold/warm conditions.

Only after approvals: developer set of **40–60 real repository tasks** plus a locked untouched holdout if feasible and protected by an independent boundary. For claims of broad superiority, the existing readiness record requires a larger multi-repository campaign (at least 150 independent tasks across 3–5 repositories and matched repetitions). The current public fixture generators/graders are diagnostic, not an independent hidden holdout. Enforce subscription request/task/time caps, no API spend, and no account-wide usage attribution without evidence.

## 8. Explicit exclusions and deferred decisions

- No new KV cache, paid prewarm, remote compaction, speculative explicit-cache request fields on Codex OAuth, or automatic public API fallback.
- No universal per-turn model router, additional mandatory reviewer/planner, unconditional pass@k, or speculative prompt compressor.
- No generic semantic response cache for executable patches or live current-state answers.
- No automatic Lean or Hybrid default promotion from offline prefix reuse alone.
- No rewriting of `CacheRuntime`, `EngineeringSnapshots`, verified receipt tracking, or root-budget accounting to duplicate their responsibilities.

## 9. Review/approval boundary

This file is a **reviewable design**, not permission to implement. The companion implementation plan contains planned RED/GREEN tests and optional commit steps for a future authorized worktree. The owner should approve scope, planned traffic budget, evaluation thresholds and execution method before engineering begins.
