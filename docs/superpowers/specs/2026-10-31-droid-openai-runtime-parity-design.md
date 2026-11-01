# Droid 1.0 → DaVinci OpenAI Runtime Parity — Design Specification

> **2026-10-31 amendment.** Plan v2 (`docs/superpowers/plans/2026-10-31-droid-gap.md`; landed under that name from the planning bundle's `2026-10-31-droid-openai-runtime-parity.md`) supersedes the following parts of this spec. The original spec is preserved in `v1-backup/`.
>
> - **Intent.** "Behavioral parity with every evidenced Droid feature" is replaced by verified tasks per unit of ChatGPT subscription usage, measured against Droid 1.1.0. Droid 1.0.0 is no longer installed.
> - **A. Script execution.** Now conditional on a measured Script gap: v2 W3, owned by the 2026-10-04 Codemode plan.
> - **B. Cache, prompt and context.** The GPT-5.6+ TTL and write-cost items and native `tool_search` are API-only or were rejected on the backend route. They are replaced by the v2 W2 probe.
> - **Benchmark gates G2/G3.** Replaced by v2's preregistered paired-ratio rule and its fixed live budget.
>
> The security and compatibility invariants and the evidence labels below remain in force.

**Date:** 2026-10-31  
**Status:** Proposed; design and planning only, no runtime changes authorized  
**Target:** `J12003LPZ/davinci` `main` at `e67d80a94119dcf4bee8bf873892b3f66b3d3638` (re-base before implementation)  
**Primary comparison:** Factory Droid product 1.0.0, analyzed Windows internal build `0.244.0`; public Oct 29 article describes build `0.250.0` and internal production. **The builds are not interchangeable.**

## Intent

Deliver **independently implemented, observable behavioral parity** with every evidenced Droid 1.0 feature relevant to efficient OpenAI coding-agent operation, then demonstrate equal-or-better task success, cost and latency on matched tests. Droid is the presumptive winner when DaVinci has no comparable evidence. For fidelity, implement behaviors, contracts and testable effects—not proprietary prompts, binaries, bytecode, private APIs, or code lifted from Factory's executable. Favor DaVinci's existing Rust authority/journal over replacing it with Bun merely to mimic implementation details.

## Source-of-truth ordering and confidence

1. **Droid directly observed in supplied study:** actual product 1.0.0 executable, mock-provider Script experiments and recorded usage (some source-mechanism claims static). `DROID-1.0.0-COMPLETE-RUNTIME-REFERENCE.md`, especially original studies 14, 17, 18, 19 and the corrections register, override early contradictory sections.
2. **Factory public statement:** https://factory.com/news/frontier-harness (2026-10-29). Treat production claims and benchmarking as vendor-reported, valuable but not independently replicated. Public descriptions of build 0.250.0 do *not* prove these features shipped in the studied 0.244.0 binary.
3. **DaVinci actual code/PRs:** pinned `main` and its tests. Presence in code ≠ model performance. Oct 31 PRs #259 and #260 cover verification receipts, trusted oracle, authority, MCP replay, compaction integrity; they explicitly exclude live model/performance proof.
4. **Proposals:** architecture/targets defined here are implementation hypotheses; promote only after reproducible experiments.

Always label each parity claim `RUNTIME_OBSERVED`, `STATIC_OBSERVED`, `VENDOR_REPORTED`, `DAVINCI_CODE_OBSERVED`, `HYPOTHESIS`, `UNKNOWN`, `BLOCKED` as applicable. Distinguish a host-side scripted mock from a real OpenAI model run.

## User priorities and success definition

The objective is not feature count: **maximizing independently verified successful tasks per unit of actual OpenAI model resource, without making requests unsafe or slowing normal tasks unacceptably.** Track session longevity on ChatGPT/Codex separately; subscription allowance is not proven proportional to token-price estimates. Require meaningful correctness labels and include failed attempts, retries and compaction costs in comparisons. Improve startup, RAM, disk footprint as secondary outcomes.

### Primary and secondary metrics

- **Primary:** first-attempt success / pass@1 under an independent oracle; total cost equivalent per *verified* success (input uncached, cached, output and reasoning tracked separately; failed attempts included); genuine request count per verified success; policy/security failures; mutation accuracy/unrelated edits.
- **Secondary:** end-to-end wall time including warm-up if stated; p50/p95 to first request and first response; throughput in tools/run; prompt bytes/tokens; exact cache reuse numerator/denominator; cache-preserving request-prefix fraction; compaction overhead and requirement-retention success; incremental RSS per concurrent session; warm/cold process and Script-worker startup; disk footprint, persistent artifact expiry.
- **Never infer:** subscription quota improvement from token counts alone; cache hits from byte-prefix equality alone; verified success from a model saying “done”; speed superiority from `--version` startup; real-model savings from scripted mock request lengths.

## Scope and exclusions

**In scope:** OpenAI Responses provider adapter, tool definitions/discovery, stable prompt and prefix caching, freeform JS Script/Codemode, raw input blocks, typed nested tool calls and arbitrary dependent execution, write and shell policy, result shaping, durable artifacts and Script background lifecycle, session/replay/rollback/compaction, subagent coordination, model policy/prompt tuning, completion and loop recovery, startup/resource efficiency, benchmark and observability pipeline.

**In scope as explicit unknowns:** Factory-managed provider routing details, account-scoped Script entitlements, production cache-warmer, provider cache retention, complete custom-model alias behavior, script boundary-limit enforcement, production daemon notifications, mission long-run success, Factory Agent Factory internal implementation, product 0.250.0-only behaviors. Investigate observable output; do not claim parity to inaccessible internals.

**Out of scope:** copying Factory proprietary code/text, account/auth/entitlement bypass, assuming `script_only` is available for every OpenAI model, silently enabling mutating Script by default before safety gates, transplanting Factory's opaque internal evaluations, changing provider credentials or model/effort without the user's choice. Existing Anthropic/Google behavior must not regress.

## Chosen architecture and alternatives

**Chosen: provider-gated OpenAI-native PTC where usable, with behaviorally equivalent local Script on DaVinci's Rust authority as full-feature fallback.** OpenAI's public Responses API documents a hosted `programmatic_tool_calling` tool with `allowed_callers`, V8 sandbox, tool discovery and repeated client-owned tool continuations; **whether the Codex OAuth route supports it is unverified**. Probe and compare native PTC first. For unsupported routes or durable local/background behaviors, extend the existing sandboxed QuickJS/WASI-backed Codemode child host, Rust `AgentCodeModeBroker`, native OpenAI Responses call/result replay, operation journal and run supervisor. Add custom-tool freeform wire handling, trusted typed bindings, raw input declarations, controlled writes/Execute, durable run state, transparent output and cache-stable discovery. The Script process has no ambient filesystem, network, environment or raw credentials; every action goes via Rust policy.

**Rejected: reimplement DaVinci as Bun/Droid architecture.** Would discard existing safety and development investment without evidence that Bun is the performance causal factor; adds migration risk.

**Rejected: wrap an actual Droid binary or transplant private source/prompt.** Not a standalone DaVinci harness, cannot guarantee authorization or maintainability, raises IP/licensing concerns, and does not establish OpenAI route equivalence.

**Defer:** shared-daemon restructuring, proactive cache warmer, automatic multilevel subagents, heavy mission fleet until targeted profiling/experiments show independent net benefit. Include parity-contract test cases now; implement after evidence gates.

## Behavioral contracts, by subsystem

### A. Script execution and OpenAI transport

- The OpenAI profile supports native hosted `programmatic_tool_calling` (with original `caller` on continuation) **or** freeform custom tool calls when provider/model/route capabilities positively authorize them; JSON function fallback remains fully operational. Tool schemas and native historical call/result kinds must roundtrip across direct, Script, mode switches, resume and rollback with no orphan results.
- Source supports top-level `await`, loops, conditions, `Promise.all`/`Promise.allSettled`, independent parallel reads and ordered dependent mutations. Bounded internal loops cannot escape quotas.
- Nested `tools.*` bindings have real structured content and full source data if authorized; errors include tool identity, status and actionable guidance. Call namespaces are deterministic, collision-aware and frozen at Script start.
- Raw multiline source/data blocks preserve UTF-8, trailing newline and delimiter-like content; invalid declarations are rejected, never silently changed. Avoid copying Droid's exact source grammar unless needed for observable interoperability.
- Controlled mode exposes authorized class-specific operations (Read, Edit/ApplyPatch, Execute, testing) through exactly the same approval, trust, cancellation, writer-lease and journal boundary as direct calls. Tool APIs may differ in naming but result behavior is equivalent. No script code may grant itself authority.
- End-user and run settings select `off`, `read-only`, `controlled`, `auto` or `direct` as appropriate. Default `auto` only after matched-model pass/cost/security gates; supported-model list is capability-validated and user-overridable.
- Per-run budgets: wall, heap, total calls, in-flight, queued, argument bytes, result bytes, image bytes, output events, total visible output, artifact retention. Droid's **observed/static reference** includes up to 128 in-flight, approximately 1 MiB nested inputs, 1,024 output events, 5 MiB emitted output, and 300 s wait; do **not** blindly raise DaVinci's current safe defaults. Cap increases require stress/profiling gates.
- `text`, `image`, `notify` are explicit output channels; arbitrary tool values stay inside execution. Result truncation is marked, preserves essential evidence, and returns a durable authorized artifact if full data is needed. Short edit summaries do not paste large modified files. No fabricated verification.
- Script runs have durable identity and state machine (`created/running/completed/partial/failed/cancelled/stalled/outcome_unknown`) with wait/status/kill; timeout returns `running` rather than lying about cancellation. Background completion notifies exactly once and wakes the loop without a model polling turn. Uncertain mutation outcomes are never replayed.

### B. Cache, prompt and context

- Stable request prefix by construction: immutable instructions and previously advertised tool schema, append new capabilities only where an adapter supports a cache-preserving representation; do not change earlier historical function-vs-custom tool kinds.
- `prompt_cache_key` and any rolling boundary/retention are **provider-capability gated**. OpenAI GPT-5.6+ supports specific implicit/explicit breakpoint rules and `prompt_cache_options.ttl` of 30m or 12h with different cache-write rates (30m write at 1.25× and 12h write at 2.5× the uncached input rate, per current OpenAI docs); older models use `prompt_cache_retention`. Optimize **total token charges per success**, not cache-hit share alone. Do not emulate Anthropic-only breakpoints or claim behavior on Codex OAuth without actual route evidence.
- Prefer native Responses `tool_search` plus `defer_loading` where supported; its discovered-tool insertion preserves previous cached prefix. Use a stable manual append-only ledger only as a validated fallback. Record typed `CacheMissReason` from exact prepared bytes and provider usage; monitor tool load, Git changes, session rollback, second subagent, errors/retry, context compaction, model switch, resume and new session. Keep logs credential-free.
- Default prompt profile uses a **small initial tool surface**, deferred tool discovery, compact skill/subagent indexes, on-demand bodies, and stable asynchronous reminders. Profile fresh prompt by contribution, not only totals. Test under target OpenAI model before enabling smaller defaults.
- Keep Context VM `off` as the evidence-proven *reference/default* until hybrid or active improves quality/cost. Compare off vs hybrid vs active with **real providers**, preserving user requirements, native tool pairing and recovery. Do not blindly copy Droid's contested 4k/16k/40k compaction configuration; those are distinct roles in local build, not one established target.

### C. Work completion and orchestration

- Host detects three identical non-progressing calls (nudge), six (stop), three output-less turns (stop), with exceptions for genuine progress and long-running polling. Different provider error categories have bounded, safe recovery; partial model generation cannot leave executable orphan tool calls.
- Completion receipt expresses `verified/unverified/verification_unavailable/failed` from trusted host observations, not model prose. Todo plans co-execute with real work where appropriate, but optional bookkeeping cannot cost a whole model round trip without a measurable reason.
- Subagents default to restrained delegation: explicit fanout/depth/root/daemon caps, workspace ownership, verified provenance, parent result receipts, cancellation propagation and cached prompt profile; measure added request overhead against benefit. Full mission/validator workflows remain explicit, not automatic fanout by default.

### D. Startup, background and product ergonomics

- Start UI and accept first input before unrelated MCP/auth/connector initialization when security permits; reuse connector capability snapshot until invalidated; use bounded concurrent startup I/O. Maintain cancellable, observable ready/not-ready state.
- A warm execution host/daemon is an option, not mandatory architecture. Measure first request, first meaningful input, memory per concurrent hot session, and retained disk bytes on Windows/Linux/macOS where supported.
- Preserve current DaVinci native CLI semantics and OAuth path. Network requests and telemetry must not hold quit hostage; any delegated cleanup remains durable and privacy-safe.

## Security and compatibility invariants (non-negotiable)

1. No model-authored JS ambient `fs`, process, network, arbitrary imports, credentials, host globals or mutable runtime authority.
2. Parent Rust always checks current permission revision, trust boundary, workspace root, mutation class and cancellation *immediately before* side effects. Script child claims are non-authoritative.
3. A call whose outcome might have executed is marked `outcome_unknown`/`recovery_required`, never rerun automatically; read-only replay only when proven safe. Operation journal and writer leases govern all mutating children.
4. Every authorization and child result is paired with correct call identity, recoverable after restart/fork/rollback; schema/encoding mismatch fails closed.
5. Secrets, private paths and raw large outputs are not sent to telemetry or model unless authorized. Retention budgets and redaction are enforced.
6. Direct-mode safety, optional read-only Codemode and prior valid sessions continue to work. Existing Anthropic/Google routes do not inherit OpenAI-only features by mistake.

## Confidence-adjusted parity policy

`BEHAVIOR_PARITY`: independently observed equivalent outputs on deterministic probes. `PERFORMANCE_PARITY`: same-model paired task results meet non-inferiority and no security regression. `OPEN_GAP`: known missing behavior. `DEFERRED_ON_EVIDENCE`: capability may not improve task success or operating cost. `UNVERIFIABLE_PRIVATE`: cannot establish exact proprietary internal behavior. Every item in the companion coverage matrix has a status, source class, owner, tests and phase; promotion requires cited artifacts.

## Benchmark gates and promotional policy

- **G0 offline:** unit/property/fuzz and 12+ Script fixture types; all permissions and persistence invariants pass; zero cross-boundary security regressions.
- **G1 controlled provider:** same fixed OpenAI model+effort+route; provider-reported cached/uncached/output/reasoning usage and raw request manifests (secret-redacted); at least five warm/cold paired runs on smoke scenarios; no unknown accounting silently converted to zero.
- **G2 real task corpus:** start with at least 20 oracle-verified diverse tasks, 3 seeded repetitions/arm, paired immutable repos, independent grading. Evaluate pass@1, cost per verified task, model calls and wall time, stratified by task/tool shape; report CI and failure modes. Expand to the existing competitor gate standard (at least 150 scenarios, three repetitions, confidence threshold) before claiming head-to-head superiority.
- **G3 promotion:** an intended default change must show no correctness/security regression; improve primary efficiency with a confidence interval that excludes the relevant no-benefit margin. If ambiguous, keep opt-in, collect more evidence. Shadow mode may collect only authorized and appropriately redacted telemetry.
- Do not treat Factory's 91.5% production reuse or 13,055-token first prompt as guaranteed DaVinci thresholds: populations and model/tokenizer differ. Treat as external references; measure matching scenarios and provider-specific token accounting.

## Dependencies, risks and unresolved points

- Managed Codex OAuth and OpenAI Responses custom tools may have differing supported wire formats; gate by live capability and preserve fallback, not a guessed route.
- A Script process can make independent calls concurrently but mutations must honor file ownership/journal. Overlapping writers, orphan children, cancellation races, partial frames, exceptions, ill-formed JS, timeout and worker crashes receive fault-injection tests.
- `script_only` Droid default depends on effective account configuration; local binary `0.244.0` does not prove article `0.250.0` behavior on other accounts. DaVinci controls its own defaults only after verified success.
- Browser/IDE integration, proprietary Agent Factory internals and signed Factory telemetry are not portable contracts; reproduce externally visible capability and measurement discipline rather than copy internals.

## Acceptance definition

Parity is **not** declared when the code merges. It is declared separately by each matrix row after a passing behavior fixture and (when efficiency is claimed) a matched live OpenAI run. Program is complete only when every relevant row is `BEHAVIOR_PARITY`, `PERFORMANCE_PARITY`, `JUSTIFIED_DIFFERENCE`, or `UNVERIFIABLE_PRIVATE` with written rationale, and no open P0 failures remain.

## References

- Supplied `DROID-1.0.0-COMPLETE-RUNTIME-REFERENCE.md` — especially consolidated chapters 17–19 and 78-claim crosswalk.
- Factory Research, “Droid 1.0: Building a Frontier Harness,” 2026-10-29, https://factory.com/news/frontier-harness
- DaVinci `main` at `e67d80a`, https://github.com/J12003LPZ/davinci/tree/e67d80a94119dcf4bee8bf873892b3f66b3d3638
- PR #259 (oracle/verification receipts): https://github.com/J12003LPZ/davinci/pull/259
- PR #260 (Claude runtime safety delta): https://github.com/J12003LPZ/davinci/pull/260
- DaVinci `docs/codemode.md`, `docs/context-vm.md`, `docs/competitor-benchmarks.md`, `docs/audit/claude-code-parity/implementation/gap-closure-decisions.md`.

## Additional primary OpenAI public API references (current at design date)

- Native Programmatic Tool Calling, hosted V8, `allowed_callers`/`caller` continuation: https://developers.openai.com/api/docs/guides/tools-programmatic-tool-calling
- Native deferred tool search and cache-preserving import: https://developers.openai.com/api/docs/guides/tools-tool-search
- Prompt caching version differences, `prompt_cache_options.ttl` and cache write rates: https://developers.openai.com/api/docs/guides/prompt-caching
- Freeform `custom` tool vs JSON `function` calling: https://developers.openai.com/api/docs/guides/function-calling
