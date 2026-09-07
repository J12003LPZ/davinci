# Davinci / pi-rust Harness Capability and Token-Efficiency Engineering Plan

> **Status:** Architecture and implementation plan only. No implementation is authorized by this document.
> **For agentic workers:** After explicit implementation authorization, use Superpowers executing-plans or subagent-driven-development, with test-driven development and verification-before-completion. Do not interpret unchecked tasks as permission to execute them now.

**Date:** 2026-09-06  
**Repository:** `C:\Users\sergi\Desktop\pi-rust`  
**Inspected baseline:** working tree on `main`, HEAD `2edc963`, with 79 pre-existing Git status entries at the start of this audit. This is a working-tree audit, not an audit of a clean commit.  
**Goal:** Increase verified task capability and quality per total token, improve effective cache reuse and latency, and strengthen memory, execution, and resource governance without rewriting the harness.  
**Architecture:** Retain the Rust agent loop, provider adapters, specialized engineering graph, generic workflow runtime, native extensions, session formats, and existing safety gates. Connect them through a small set of authoritative interfaces for usage, budgets, context, capabilities, and durable evidence.  
**Tech stack:** Existing Rust workspace, pinned dependencies, serde/serde_json, existing HTTP/WebSocket integrations, JSONL/session infrastructure, Ollama embeddings, optional Qdrant, and existing SQLite dependencies where a measured migration justifies them. No new orchestration framework is proposed.  
**Specification:** The attached `Pasted text(5).txt`, especially its audit requirements, 18-section output contract, and planning-only stop condition. This file is the single consolidated deliverable; earlier September 5 plans remain unchanged.

## Global constraints and evidence conventions

Preserve `AGENTS.md` and `CLAUDE.md` requirements: the active Cargo workspace is authoritative; `vendor/davinci` is reference-only; preserve protected upstream system, compaction, and branch-summary prompt constants verbatim; retain the pinned Rust toolchain and exact dependency versions unless a separately justified change is approved. Do not modify archived crates or stale TypeScript package stubs as substitutes for active Rust code. Preserve protocol/provider compatibility separately from the product version.

Do not overwrite, stage, reset, clean, or incorporate unrelated working-tree changes. In particular, preserve existing untracked September 5 planning files and `plugins/`. Implementation must begin with a fresh status and content-hash inventory because another process or developer may change this tree after the audit.

**Observed** means supported by inspected executable code. **Declared** means a type, helper, comment, or setting exists but production enforcement was not demonstrated. **Risk** is an inference from code, not a reproduced incident. **Proposed** describes future behavior. **Historical** describes a previous execution report, not a test run during this audit. Numeric improvement targets below are proposed release gates, not measured savings.

No builds, tests, live provider calls, local Ollama/Qdrant requests, benchmarks, package installations, or implementation changes were performed for this plan. Public documentation was checked read-only. Actual deployment configuration, private credentials, production transcripts, account pricing, service versions, memory volume, and production cache-hit distributions were not established. Source inspection can identify mechanisms and missing guarantees; it cannot establish their production frequency or dollar impact.

## 1. CURRENT-STATE ARCHITECTURE

### 1.1 Request lifecycle actually present

The coding-agent entry point resolves model/authentication, loads or reuses the extension host, attaches the shared tool executor, snapshots built-in/MCP/native tool specifications, resets the system prompt, invokes extension startup hooks, and injects native memory as ephemeral context. It then constructs runtime services and enters `Agent::run_loop`. Each internal iteration prunes old tool results, optionally compacts context, calls the provider with retry handling, persists assistant output, prepares and authorizes tools, executes tool lanes, and appends results. Settling the outer prompt indexes conversation-derived memory and supplies learning evidence. [S01, S02, S03]

Several important details are already correct: pruning precedes expensive compaction; tools have preparation/execution/finalization stages; permission decisions precede execution; read lanes preserve result order; unknown or mutating work is normally serialized; provider reasoning configuration is passed through runtime options rather than only described in prose. The identity suffix names the current provider/model/thinking level and is appended after the base system prompt, not prepended. [S01–S03]

```text
CLI / interactive / RPC entry points
  -> model, auth, settings, trust, extension host
  -> base prompt + extension modifications + ephemeral memory
  -> runtime handle / hooks / capability registry
  -> agent turn loop
       -> prune -> compact when needed -> provider request / stream / retry
       -> permission + tool ledger -> read lanes or mutation barriers
       -> output governor / evidence -> tool-result continuation
  -> session persistence + settled memory indexing + learning review

Specialized graph tool                  Generic workflow tools
  -> deterministic engineering graph     -> WorkflowExecutor + WorkflowSpec
  -> isolated role workers               -> phase dependencies + worker requests
  -> typed artifacts / review / verify   -> artifact references + joins
```

### 1.2 Subsystem map

| Subsystem | Current implementation and strengths | Constraint or unverified integration |
|---|---|---|
| Prompt assembly | `main.rs::complete_prompt_with_host`, base-prompt reset, extension overrides, plan appendix, late identity suffix | Tool definitions are captured before startup hooks and reused inside the loop; final wire accounting is only a byte-based estimate in the inspected path. |
| Context | Agent history/pruning/compaction; specialized graph context packets; generic `ContextBroker` | Multiple assembly paths. Native broker-registration helpers are present, but not called by the inspected main request path. |
| Native caching | `davinci-ai/cache.rs`, provider request serialization, graph role affinity keys | Main requests use `PI_GRAPH_CACHE_KEY` when supplied, otherwise provider/session fallback; universal cache identity is not demonstrated as the main-path authority. |
| Token accounting | Provider usage fields, `RunStats`, graph `WorkerUsage`, protocol billing categories | No demonstrated single complete ledger covering every attempt, child, compaction, retrieval, and background learning cost. |
| Token governor | Reversible output compression, stored output retrieval, read ledger, limited duplicate-search suppression | Primarily deterministic tool-output middleware, not an adaptive per-request/per-node resource scheduler. |
| Vector memory | Local JSONL records; Ollama query/document embeddings; lexical plus cosine scoring; optional remote upserts | Retrieval scans local records. Qdrant is not queried by the inspected search path. Writes rewrite the whole local file. |
| Memory learning | Conversation extraction, promoted memory kinds, separate learning evidence/reviewer/skill lifecycle | Ordinary message indexing broadly treats user text as tasks and assistant text as decisions; ordinary records lack much of the provenance supported by richer learning records. |
| Engineering graph | Typed artifacts, deterministic controller, graph topology validation, isolated worker processes, replay fingerprints, mutation/review infrastructure | Existing cost/time guards default to unlimited; usage arrives after work has begun; no unified parent/child reservation system. |
| Generic workflows | Phase specs, all/any/quorum joins, task/agent registries, cancellation, artifact references | Execution is serial in the inspected loops. Several spec limits are not runtime-enforced there. Missing runner can produce canned success. |
| Tools and agents | Permission policy, MCP classification, mutation barriers, read-only subagents, batch/evidence support | Dynamic capabilities and advertised schemas need a single versioned source; nested retries and resource limits need shared ownership. |
| Persistence/recovery | Session logs, runtime event rehydration, graph run/artifact files, workflow overflow files | Generic workflow registries and inline artifacts remain in memory; overflow files alone do not constitute durable workflow recovery. Some persistence errors are discarded. |
| Structured outputs | Graph artifact contracts/validation; provider-specific request shaping; Responses item/ledger types | A type declaration or JSON schema is not proof of live use, semantic correctness, or lossless native replay. |
| Observability/evals | Runtime bus/log subscribers, run stats, ecosystem telemetry, Codex telemetry types, paired eval/oracle helpers | Existing independent oracle improvements must be preserved. A complete runner-driven baseline and per-attempt telemetry are still needed. |
| Security/privacy | Trust gates, permission deny semantics, role allowlists, redaction, graph verification | Redaction is best effort; memory scopes, remote deletion, artifact ownership, and recovery permissions need explicit end-to-end contracts. |

### 1.3 What “graph engineering” means here

This repository contains **two execution systems**, not a single knowledge graph. The specialized `native_extensions/graph` subsystem is an engineering-task state machine over a validated execution topology and typed artifacts. Roles include classifier, researcher, test analyzer, historian, planner, writer, and reviewer. The controller handles bounded revision/replanning loops; topology types include explicit edge conditions and cycle validation. The generic `davinci-agent/runtime/workflow` subsystem describes phase dependencies and workers with all/any/quorum joins. Neither should be replaced by a new framework merely because it has a graph-shaped API. [S10–S13]

The specialized graph is materially more developed in mutation tracking, worker isolation, and replay compatibility. The generic workflow implementation needs correctness work before higher concurrency. In particular, its `execute_phase` walks workers serially, retries runner errors without an effect-aware result type, and substitutes a success string when no runner exists. It ignores artifact-store errors at the success site. Its resume logic can treat the existence/count of artifacts as join satisfaction without a complete current-input compatibility proof. [S12]

### 1.4 Existing September 5 work: preserve, do not redo

The earlier plan and its execution checkpoint document a partial implementation, not completed acceptance. The checkpoint records fixes for missing subagent imports, unsafe success comparisons, broad backend capability classification, cancellation during HTTP retry waits, and recovery-aware pruning. Inspected code supports retaining the newer independent `OracleObservation`/manifest-bound evaluation path and disjoint usage semantics. Do not reopen those historical bugs as though they remain unchanged. [S14, S15]

Carry forward the unfinished themes: complete accounting, real feature/profile consumers, shared run budgets, nested attempt lineage, durable evidence retention, safe recovery, dynamic tool disclosure, actual instruction/schema cache wiring, simple-path ablations, and a real evaluation runner. This plan extends them with the deeper memory and dual-orchestrator findings. Historical test counts in the checkpoint are not a fresh green baseline for the current dirty tree.

## 2. CRITICAL FINDINGS

| Rank | Priority | Finding and evidence | Consequence and confidence |
|---|---|---|---|
| 1 | P0 | Generic workflows can claim success without a runner; success-side artifact writes are ignored. `workflow/executor.rs:601–626`. [S12] | Observed false-success path. Production reachability depends on which entry points supply a runner, but the executor contract itself is unsafe. |
| 2 | P0 | Resource controls are fragmented. Generic workflow cost/deadline/max-turn fields are declared without corresponding enforcement in the inspected execution loops; engineering graph spend/time defaults are zero/unlimited. [S10, S12] | Observed enforcement gap/defaults, not proof of runaway production spending. More parallelism would amplify this risk. |
| 3 | P0 | Memory persistence rewrites JSONL with `fs::write`; ordinary indexing persists before and after synchronous embeddings. Query/index operations run beneath host locks. [S01, S06] | Observed work amplification and blocking path; crash truncation, stale-snapshot overwrites, and contention are inferred failure risks. |
| 4 | P0 | Local clear does not delete remotely upserted memory; identity/dedup fields do not consistently include kind/profile/scope, and embeddings lack a complete compatibility identity. [S06] | Observed lifecycle gaps. Scope collisions, incompatible-vector scoring, and incomplete deletion require regression tests before migration. |
| 5 | P0 | The recovery-output limit admits the first oversized line; ordinary memory search lacks a serialized text cap and is exempt from governor compression. [S06, S07] | Observed bypasses of intended context bounds, especially for minified JSON or long records. |
| 6 | P1 | Native memory search performs local scanning, fixed-weight score fusion, and top-k truncation; configured candidate depth is not used in that search algorithm. [S06] | Observed scaling/selection limit. Qdrant upserts currently add work without accelerating that retrieval path. |
| 7 | P1 | Reusable context/cache/capability abstractions coexist with older direct paths. Broker registration helpers are not connected in the inspected main request flow. [S01, S04, S05, S08] | Observed integration gap. Adding another abstraction would worsen maintainability without changing requests. |
| 8 | P1 | Generic workflow state is largely in-memory, while main creates fresh runtime/workflow objects per outer prompt. Some graph checkpoint failures are ignored. [S01, S11–S13] | Observed persistence limitations; reliable cross-process resume is not established by runtime-log rehydration alone. |
| 9 | P1 | Bytes withheld, cache-affinity keys, and successful final usage do not prove total token/cost improvement. [S03, S07, S09, S14, S16] | Measurement gap. Compression retrieval, failed attempts, replay-credit accounting, and learning can hide costs elsewhere. |
| 10 | P2 | Tool-surface snapshots, full-schema identity, model routing, and native continuation helpers need integration-level proof. [S01, S05, S08, S17] | Risks of stale advertised tools, unnecessary prefix changes, or unsupported backend behavior; do not enable globally based on helper tests alone. |

**Recommended order:** make success and spending truthful; close context/deletion/recovery holes; wire existing components; then tune retrieval, caching, routing, and parallelism against independent task outcomes. The largest plausible gains come from avoiding unnecessary work and context—not from making every prompt shorter at any cost.

## 3. TARGET ARCHITECTURE

### 3.1 Smallest coherent change

Retain the existing execution engines and introduce or finish five shared contracts: `UsageLedger`, `RunBudget`, versioned `ToolSurface`, `ContextPlan`, and durable `EvidenceRef`. These are proposed contracts, not claims that the types already exist. Extend the current runtime handle and broker rather than building another service framework. Keep provider-specific wire behavior in `davinci-ai`, not in graph prompts.

```text
Authenticated request + project trust + explicit user goal
                         |
             existing application bootstrap
                         |
      scoped runtime services and capability snapshot
        |                |                     |
   UsageLedger       RunBudget             EvidenceStore
        |                |                     |
        +------- workload/risk routing --------+
                         |
               ContextPlan construction
          /              |                 \
 stable instructions   task state      memory/skills/evidence
          \              |                 /
         final wire-size check + output/reasoning reserve
                         |
          provider profile + stable ToolSurface
                         |
            attempt admission / request / stream
                         |
         authorized tools or orchestrator adapter
          /                              \
 engineering GraphController        generic WorkflowExecutor
     scoped workers                    scoped workers
          \                              /
        artifact validation + effect reconciliation
                         |
          independent verification / final synthesis
                         |
       durable outcome + gated memory/learning queue
```

The root agent owns synthesis and conflict resolution. Controllers own admission, transitions, cancellation, and reconciliation. Workers own only assigned tasks and outputs. Storage owns durability and deletion. Provider adapters own wire semantics and usage normalization. No agent decides its own permissions, declares its own tests passed, or silently expands its parent's budget.

### 3.2 Ownership and integration boundaries

| Contract | Proposed owning location | Required consumers |
|---|---|---|
| Normalized usage/attempt events | Extend `davinci-ai` types/stream adapters; run aggregation in `davinci-agent/runtime` | Main loop, graph children, subagents, compaction, learning, evals |
| Admission and budget leases | Proposed `crates/davinci-agent/src/runtime/budget.rs` | Provider attempts, tools, both orchestrators, background work |
| Versioned authorized tool surface | Existing `runtime/capabilities.rs` plus host/MCP registration | Permission validation, schema rendering, cache identity, routing |
| Context selection and rendering | Existing `runtime/context.rs` with native source adapters | Main requests and both worker systems |
| Evidence references and lifecycle | Existing `evidence.rs`, governor output store, workflow/graph stores through adapters | Pruning, compression recovery, checkpoints, reviewer coverage |

Do not force all stores into one schema immediately. Begin with shared identifiers, access checks, retention/pinning semantics, and adapters. Likewise, keep graph role-specific artifact contracts while sharing scheduling/budget primitives with workflows.

A runtime's long-lived registries must outlive an individual prompt where background workflows depend on them. Keep new run/turn IDs per request, but separate session/service lifetime from request lifetime. Register subscribers once per owning lifecycle; avoid duplicate hook execution after reuse. Persist recovery-critical state before declaring success, while noncritical analytics may remain fail-open with a visible dropped-event counter.

## 4. TOKEN ECONOMICS

### 4.1 Count the complete lifecycle

Measure three separate quantities: logical context occupancy, provider-reported/billed usage, and transmitted bytes. A continuation can reduce transmitted bytes without reducing billed context. A cache hit can reduce cost and prefill latency without reducing logical input tokens. Local compression can reduce one prompt while causing another model call or recovery read. Keep these distinctions in every report.

Preserve the existing protocol's **disjoint** billing buckets: `input`, `cache_read`, `cache_write`, and `output`. `Usage::from_tokens` adds these categories; it does not expect an inclusive provider input count in `input`. Existing Responses decoding already subtracts read and write buckets from inclusive input. Add provider-specific validation and coverage, not a global reinterpretation. Reasoning is a reported subset/annotation where the provider includes it in output, not a second charge added to that output. [S09, S16; D2]

```text
logical_input = uncached_input + cache_read_input + cache_write_input
normalized_total = logical_input + output_including_provider-billed_reasoning
run_cost = sum(cost of each unique billable attempt)
         + embedding/reranking/learning costs where applicable

net_tokens_saved = baseline whole-run tokens - candidate whole-run tokens
net_cost_saved   = baseline whole-run cost   - candidate whole-run cost
```

For newer public OpenAI cache-write reporting, input normalization must distinguish both read and write subsets; for Claude, the documented input/read/write fields are already separate. Treat missing usage as unknown, not zero. Validate inconsistent negative/overlapping totals rather than silently masking them with saturating arithmetic alone. [D1, D3]

### 4.2 Where work is spent or plausibly wasted

| Area | Evidence-based mechanism | Target reduction and accounting requirement |
|---|---|---|
| Repeated prompt/schema material | Repeated main and worker requests, captured tool schemas | Stable versioned surfaces; measure native cached/uncached tokens, not just key reuse. |
| Low-value memory | Broad message extraction, fixed-k injection, no final memory-search text cap | Gate writes, calibrate retrieval, include only justified excerpts; count query embeddings and reranking. |
| Repeated memory maintenance | Full-file rewrites and synchronous embedding passes | Incremental durable writes, content-addressed embeddings, coalesced bounded indexing. |
| Duplicate context across workers | Separate graph, main, and broker paths | Single retrieval snapshot with scope-specific projections; count all child inputs. |
| Tool outputs | Static compression, recovery tool, pruning/evidence | Bounded previews with real recoverable references; subtract subsequent retrieval costs from savings. |
| Retry chains | Agent retry plus provider retry plus worker retries | One attempt lineage and shared retry allowance; include failed/cancelled paid attempts. |
| Graph overhead | Classification/research/planning/review and worker startup | Risk-appropriate simple route and scoped roles; retain mandatory verification. |
| Compaction and learning | Separate lifecycle calls, potentially repeated summaries/reviews | Trigger by measurable value/pressure; charge the originating workload and report delayed costs. |

### 4.3 Budgets: constraints first, allocations second

Let `W` be the verified model context limit, `R` the generation reserve including supported reasoning behavior, `G` a conservative estimation guard, and `B_run_remaining` the remaining run allowance. Admission requires final estimated input plus `R + G <= W`, and a sufficient run lease for the next attempt. The governor must not fill a large window just because it exists.

Use workload-relative allocations of the **admissible input budget**, not hard-coded fractions of every model's full window. Starting experimental ceilings: instructions/tool definitions 25%, current task and durable state 20%, recent history 25%, retrieved evidence 20%, memory/skills 10%. These are soft allocations, not truncation permissions. Actual protected instructions and the user's task are measured first; unused allocations are borrowed by useful evidence. If mandatory content exceeds the allowance, select a supported larger context or report a budget boundary—never silently discard it.

Treat the existing graph packet's approximate 2,500-token total, 1,200 memory tokens/four hits, and 1,000 skill tokens/two skills as a baseline to compare, not a universal optimum. Charge wrappers and reference metadata in the final rendered packet. Model-specific output and reasoning reserves must be measured from task outcomes; a tiny reserve that repeatedly yields incomplete responses is not an efficiency gain. [S18; D2]

No dollar-saving percentage is promised before a paired baseline exists. For subscription/OAuth usage without marginal billing information, report tokens, latency, quota pressure, and unknown monetary cost. Catalog-estimated equivalent API cost must be explicitly labeled hypothetical.

## 5. CACHE STRATEGY

### 5.1 Separate four identities

1. **Prefix affinity identity:** backend profile, model snapshot, permission/isolation scope, stable instruction version, exact canonical tool surface, role/output contract, and cache-relevant configuration. Exclude run IDs, timestamps, live retrieval scores, and changing task state.
2. **Full request/replay fingerprint:** exact ordered context, tool/config versions, user goal, dependency artifacts, source revisions, and relevant workspace-content evidence. Dynamic inputs belong here; this identity must change when replay correctness changes.
3. **Embedding cache identity:** normalized text plus model digest/version, dimensions, query/document task prefix, normalization, chunking, and schema versions.
4. **Tool/result cache identity:** normalized arguments, actual source content/freshness domain, permission scope, tool implementation/schema version, and effect classification. Default to no caching for mutations or ambiguous external state.

A key never grants permission and never establishes factual freshness. Do not use Git HEAD/status alone to suppress reads of dirty, ignored, external, or mutable targets. Preserve the current conservative main-path decision to supply no search state hash when freshness cannot be proved. [S01, S05, S07, S08]

### 5.2 Stable prefix and dynamic suffix

Render stable instructions and the smallest justified stable tool palette deterministically. Preserve instruction authority and protected upstream prompt bytes. Keep project rules versioned by content, not by transient metadata. Append current task state, timestamps, selected evidence, memory, and per-request metadata at the latest semantically valid position. Do not demote authoritative application instructions to user data just to increase cache reuse.

Canonicalize JSON objects and tool ordering only where ordering is semantically irrelevant; preserve arrays, conversation ordering, Unicode text, and schema semantics. Hash full live tool definitions and relevant permissions, not only names or a newly constructed built-ins-only registry. Invalidate schema caches on native/JS/MCP registration changes. Recompute the authorized snapshot at provider boundaries when tools change; do not depend on the stale outer-loop capture. [S01, S05, S08]

The existing model/thinking identity suffix is already late within the system string. Measure its actual effect before changing it. Runtime settings must remain real API controls; deleting explanatory prose is not a substitute for selecting a valid backend reasoning configuration.

### 5.3 Current first-party behavior and compatibility gate

Public OpenAI documentation checked on 2026-09-06 describes rendered-prefix matching, routing influence—not guaranteed hits—from `prompt_cache_key`, and model-dependent caching modes, breakpoints, retention, and write accounting. For newer supported models it documents explicit breakpoints and `prompt_cache_options`; earlier models differ. Therefore, do not universally assume the old 1,024-token/retention behavior or treat every OpenAI-compatible endpoint as the same backend. These public API facts do not verify the authenticated Codex backend. [D1]

Create fixture-backed profiles keyed by parsed HTTPS origin, provider/API identity, model capability and authentication mode. Retain the recently tightened resolver. Support only fields proved for that profile, including cache-write accounting. Unknown proxies and unsupported features take the conservative path. Selection of explicit boundaries, retention, and tool-loading features is an experiment behind real request-path flags, not prompt prose. [S15; D1]

Claude's documented input/read/write accounting and cache control require their own adapter. Do not copy OpenAI usage semantics or TTL fields into that implementation. [D3]

### 5.4 Application caches and invalidation

Begin with exact deterministic caches: canonical schema serialization, instruction-file reads keyed by content/freshness, embeddings, and in-flight identical authorized reads. Semantic answer caching remains P3, restricted to low-risk read-only workloads with independently valid source versions. Never cache a successful mutation as a reusable answer to another mutation request.

Persist cache entries with schema version, owner/scope, content hash, creation/expiry, dependency identities, and deletion generation. Delete/revoke by namespace generation or explicit tombstone; a stale entry must fail closed under permission changes. Native provider cache retention is separate from application deletion and cannot be represented as instantly erased by local cleanup.

Telemetry: prefix-version changes by component; exact request-shape changes; eligible input share; actual read/write/uncached token fractions; cold versus warm comparisons; retention mode; overflow/routing indicators where available; request preparation time; and net cost after cache writes. Do not prewarm with paid synthetic requests without a break-even experiment. Do not mark dynamic memory `stable_for_cache` merely because an individual record is immutable: selection and ordering can change. Freeze useful context within an explicit epoch, with correct replay invalidation.

## 6. GRAPH ENGINEERING

### 6.1 Preserve the specialized engineering graph

Keep deterministic role routing, typed `graph_submit` artifacts, per-role tool restrictions, isolated child processes, mutation ownership, topology validation, reviewer coverage, and independent verification. Preserve the existing distinction between simple, standard, and complex paths. Do not claim every path already invokes every review role; instead specify mandatory checks by actual risk and test them on each route. [S10, S11]

Strengthen each node contract with: responsibility, versioned input artifact references, allowed tools and scopes, output schema, effect class, token/cost/attempt lease, deadline, retry policy, completion validation, and reconciliation owner. A node receives a brief plus selected evidence and state references—not the entire parent transcript. Classifiers receive goal and compact repository facts; researchers receive focused questions and source scope; writers receive the approved plan and exact target evidence; reviewers receive graph-owned changes, coverage obligations, and verification results.

### 6.2 Scheduling and state

Use the existing ready frontier and dependency semantics. Acquire a parent lease before spawning; permit bounded independent read work; serialize writes to shared state. Bound queue length and total active work across graphs, tools, generic workflows, and learning—not independently in every layer. When limits tighten, stop admitting optional work before cancelling necessary verification.

Represent revision cycles as explicit attempt/version transitions with the existing cycle/replan limits. Reject duplicate IDs, unknown dependencies, conflicting writers, unreachable required nodes, and unsatisfied joins. Check conditional edges against outcomes rather than structural reachability alone. Do not introduce arbitrary recursive graph expansion or speculative writers. Any later expansion needs a bounded depth/node count and a demonstrated quality or critical-path benefit.

Snapshot or lock shared inputs for parallel reads. Reconcile output in deterministic source/contract order; keep first-completion timestamps separately for latency analysis. Cancellation must reach every child and prevent new admissions. Reclaim a lease only after terminal acknowledgement or conservative reconciliation; an expired heartbeat does not prove a remote billable operation stopped.

### 6.3 Generic workflow correctness before concurrency

Remove production canned success. Require a runner or an explicitly injected fixture runner. Validate/store artifacts before task/phase success, propagate persistence errors, and reconcile agent/task states on early any/quorum completion. Workers never started must become skipped/cancelled rather than remain running.

Enforce declared limits, including deadline, admitted concurrency, total attempts/agents, cost where known, and max turns through the actual child runner. Propagate the configured working directory and resolved worktree path rather than presenting a placeholder path as established isolation. Use the live capability registry for mutating-tool classification in both initial execution and resume. [S12]

Resume joins must count distinct successful expected workers, not arbitrary artifact count. Match the current workflow/spec, worker input, permission/tool surface, model contract, dependency artifacts, and actual workspace-content fingerprint. Validate overflow artifacts through their metadata and content, not by reading a nonexistent inline `worker` field. Rehydrate durable manifests and indexes on a new process; persistence of overflow bytes alone is insufficient. [S12, S13]

### 6.4 Durable execution and partial-result reuse

Persist `admitted -> started -> output_received -> validated -> committed` transitions, with effect observations and artifact hashes. Only committed validated outputs satisfy dependencies. Store the immutable spec and versioned dependency fingerprints alongside the run. Treat failed checkpoint writes as a recovery boundary: halt new side effects and report inability to guarantee resume.

Split historical cumulative effort from new spend in a resumed run. Current graph replay credits previous usage to totals; keep that history but never bill or reserve it as a new provider attempt. Reuse successful read/research artifacts only under compatible content and permission fingerprints. Re-run verification against the final actual state. A writer's ambiguous failure enters reconciliation, not an automatic whole-worker rerun.

Higher concurrency is P1 only after these guarantees and global budget leases exist. Cross-phase parallel execution and any/quorum racing should be evaluated against serial baselines. Worktree-based multi-writer execution remains opt-in and requires independently reviewable ownership plus conflict-aware integration.

## 7. VECTOR MEMORY

### 7.1 Current lifecycle and immediate corrections

The actual lifecycle is broad conversation extraction, redaction, fixed-size chunking, local deduplication, full JSONL persistence, synchronous embeddings, another local persistence, and best-effort Qdrant upsert. Retrieval filters local records, embeds the query when dense vectors exist, computes local cosine and substring overlap, adds importance, sorts, and truncates. There is no Qdrant search in that path, no demonstrated full expiration/consolidation lifecycle, and ordinary clear only removes the local store. [S06]

Keep the useful offline lexical fallback and 120-second dense-failure cooldown. Do not require a running vector service for basic agent use. Do not label stored conversation assertions as verified knowledge simply because an assistant produced them. Preserve raw session records independently of long-term memory write gating.

### 7.2 Authoritative record and isolation model

Introduce a versioned record with: stable memory ID; logical repository and actual workspace identity; principal/user scope; project/session/profile scope; memory kind; normalized content hash; original source references; observation and validity timestamps; confidence source; verification status; trust class; salience; use/outcome counters; supersedes/contradicts links; expiry/deletion generation; and embedding compatibility identity.

Fix deduplication and ID construction together. Include kind and the full authorized namespace consistently; include scope when scope affects ownership. A repository remote can identify a logical project but is not sufficient user/workspace isolation. Avoid globally lowercasing case-sensitive paths. Explicitly define whether worktrees share project memory and how untrusted clones are isolated.

Embedding identity must include the actual model revision/digest when available, dimensions, query/document task prefix, chunking version, normalization, and truncation policy. Equal vector dimensions do not prove compatible spaces. Existing EmbeddingGemma prefixes should be preserved until model-specific tests justify changes. Google's documentation confirms 768-dimensional examples; dimension reduction is an experiment, not a default migration. [D5]

### 7.3 Persistence and indexing

First close the loss window with locked, atomic persistence and visible error reporting. Then move authoritative metadata/writes to a transactional store only when volume/concurrency measurements justify it; existing SQLite infrastructure/dependencies make this a candidate, not a mandate. JSONL can remain an export/migration format. Do not maintain two writable authorities indefinitely.

Use an outbox for embedding/upsert/delete work. Commit the memory record and job atomically, then process a bounded queue outside the extension-host lock. Coalesce identical content across the same authorized compatibility namespace. Recover pending jobs after interruption; use idempotent upserts and tombstones. Readers use a current versioned snapshot or shared repository service, not deep-cloned record vectors that silently become stale.

Ollama's current embedding endpoint accepts batches and exposes truncation controls; its documented default can truncate overlong inputs. Make truncation explicit and test boundary behavior so a successful embedding does not silently represent only part of a record. Preserve the existing narrow legacy fallback for genuinely unsupported endpoints; malformed modern responses and persistent failures need classification, not unbounded per-record retry. [D4]

Qdrant is a rebuildable projection, not a second source of truth. Either use its filtered query path where scale justifies it, or disable unused remote upserts by a clear setting. Validate collection/vector configuration and payload indexes before enabling query traffic. Pin behavior to the installed server version, which was not probed during this audit.

### 7.4 Retrieval pipeline

```text
query + authorized namespace + token/latency lease
  -> workload intent and exact-identifier detection
  -> metadata/expiry/trust/source-validity filter
  -> bounded lexical candidates + compatible dense candidates
  -> rank fusion -> exact/semantic deduplication
  -> optional reranker only when its benefit pays for its cost
  -> diversity and contradiction-aware selection
  -> marginal-value-per-token packing
  -> final serialized cap, IDs, provenance, bounded excerpts
```

Replace incomparable fixed score mixing with calibrated scores or rank fusion. Start with lexical and dense reciprocal-rank fusion, evaluated independently for natural-language and identifier-heavy queries. Qdrant documents hybrid prefetch/fusion support, but using a newer feature requires version confirmation. [D6]

Do not rerank the whole collection. Bound candidate depth separately from injected top-k; honor the configured candidate limit or remove it if intentionally unsupported. Use a bounded heap/index rather than cloning vectors for every scored result. Exact error codes, file paths, and symbols should remain retrievable without depending on semantic similarity.

Start adaptive injected k at 0–4, with up to the existing six only when measured evidence gain and allowance justify it. Return no memory when relevance is weak. Deduplicate against already-present task/history/evidence. Handle an oversized candidate by skipping it or producing a provenance-preserving excerpt, not by breaking selection and discarding all smaller later candidates. Count ID/score/source wrappers and closing markers. Make `memory_search` itself bounded even when classified lossless by the output governor.

### 7.5 Write gating, consolidation, and deletion

Separate episodic observations from durable semantic facts, explicit constraints, verified fixes, and task outcomes. Gate writes using durable relevance, novelty, scope, sensitivity, evidence, and user intent. Assistant assertions begin unverified; automatic verification requires concrete external observations. Preserve explicit corrections and contradictions rather than collapsing them into one confident summary.

Consolidate only within an authorized scope and compatible topic/validity range. Store source IDs and revision history for synthesized memories, charge consolidation cost, and never manufacture certainty. Prefer deterministic duplicate/supersession cleanup before model summarization. Give transient task observations shorter retention than stable constraints, with explicit expiry rules and operator overrides.

Deletion immediately blocks retrieval through a tombstone, then removes local records/vectors, Qdrant payloads, embedding/result caches, pending writes, and derived summaries that expose the deleted material. Retried outbox jobs must not resurrect it. Specify separately what remains in session logs/backups and for how long; do not promise deletion from provider-managed caches. Report remote deletion pending until acknowledged.

Metrics: judged precision@k, recall on a labeled answerable set, useful-context rate, irrelevant injected token fraction, contradiction/stale-memory rate, provenance completeness, write acceptance rate, embedding reuse, indexing lag, retrieval p50/p95, and downstream verified-task effect. “Used in an answer” is not proof the memory was correct.

## 8. TOKEN GOVERNOR

### 8.1 Extend rather than replace output compression

Retain reversible compression, error/multimodal preservation, retrieval tooling, and the conservative freshness rules. Separate the present `TokenGovernor` middleware from a new runtime admission/budget service. The former governs output representation; the latter governs whether and how much work may run. Both emit the same usage/evidence events. [S07]

Proposed leases carry request/run/node/agent IDs, workload and risk class, context allowance, generation reserve, remaining provider attempts, tool-call allowance, wall deadline, optional priced cost cap, and optional-work allowance. Reserve atomically before fan-out. Settle using unique provider-attempt IDs and monotone reported usage; return unused allowance only once. Unknown final usage remains reserved/uncertain until reconciled.

### 8.2 Policy

Use deterministic task classification initially: trivial answer, focused read-only investigation, ordinary code edit, broad refactor, recovery, and high-risk change. Adjust allocations using observed complexity signals, failure history, context pressure, evidence demand, and verified backend limits. Do not make a separate LLM call merely to choose a budget for trivial work.

Cache awareness may change expected monetary cost and routing, but never context-window occupancy. A cheap cache read still consumes context. Output reserve includes supported reasoning behavior and must be enforced in actual provider parameters. `StreamOptions.max_tokens: None` in the current main path is not a run-level budget. Integrate a provider-specific output ceiling through the real request builder, with explicit unsupported behavior. [S01; D2]

Keep legacy zero/unlimited graph settings readable. Add an explicit bounded policy profile with finite attempt/tool/deadline safeguards; do not silently change the meaning of existing zero values. Promote bounded defaults only after migration tests and documentation. When prices are unknown, enforce tokens/attempts/time and label the dollar cap unverified or require an operator-defined conservative estimate—never treat unknown cost as free.

### 8.3 Graceful degradation ladder

| Order | Action | Information or guarantees that must survive |
|---|---|---|
| 1 | Exact deduplication and remove repeated status/tool wrappers | User goal, authority, references, errors, effect status |
| 2 | Drop weak/duplicate memory; reduce candidate/injection depth | Relevant constraints and required evidence |
| 3 | Compress low-value old tool results into durable references | Full recovery access and original effect outcome |
| 4 | Replace completed history with structured task state | Decisions, outstanding obligations, contradictions, provenance |
| 5 | Summarize only selected low-value history when amortized value is positive | Source-linked state and original recoverable transcript |
| 6 | Cancel/skip optional research, extra agents, speculative reviews | Mandatory verification and safety checks |
| 7 | Choose an approved alternative model/route after compatibility checks | Required capabilities, permissions, scope, state fidelity |
| 8 | Stop safely or request justified additional allowance | Honest partial outcome, remaining work, durable checkpoint |

Soft thresholds trigger selection and optional-work reduction before hard admission denial. Initial shadow-mode pressure bands can be 70%, 85%, and 95% of the **admissible input allowance after reserves**, then tuned. They are not percentages of raw model context and do not authorize arbitrary truncation.

Detect no-progress loops through repeated normalized actions plus unchanged evidence/state, not semantic resemblance alone. Share retry allowance across provider, agent, and graph layers. Early stopping requires fulfilled acceptance criteria or an explicit terminal boundary; it must not convert budget exhaustion into success.

## 9. CONTEXT ENGINEERING

Use one `ContextPlan` with a manifest of selected items, source/authority class, permission scope, content/version hash, provenance, estimated rendered tokens, stability epoch, and exclusion reason. Final serialization should be inspectable in sanitized fixtures without logging raw private content in production.

Wire the existing broker through main and worker paths behind a parity flag. Disable the corresponding legacy native injection when the broker is active; assert that the same memory or skill is not injected twice. Avoid collecting native sources while holding broad extension-host locks. Respect `automatic_retrieval` consistently in every adapter, including the current `MemoryContextSource`, and use scoped live handles rather than deep record clones. [S01, S04, S06, S08]

Selection order is authority and necessity first, then relevance, evidence quality, freshness, novelty, and marginal benefit per rendered token. Protected instructions are not merely another ranked source. Scores from memory and skills must be calibrated before direct comparison. Use a final aggregate renderer cap; per-source estimates alone cannot cover wrappers, tool schemas, multimodal data, or provider-specific formatting.

Structured task state contains the current goal, accepted plan, verified facts with references, modified-file manifest, completed checks, unresolved failures, decisions and their rationale summaries, pending tool-call IDs, and next obligations. Do not store hidden reasoning as a required state format. Preserve provider-native opaque items only through supported transport/storage contracts, with appropriate privacy handling.

Compaction is an explicit epoch change. Retain original session/evidence references, snapshot tool-call/result associations, reset duplicate-read visibility bookkeeping as needed, and invalidate replay lineage after rewritten context. Compare its one-time token cost and subsequent savings before triggering repeatedly. Do not compact merely to increase an attractive cache percentage.

Treat tool results, memory, repository comments, retrieved documents, and worker outputs as data. Escape or structurally encode delimiters/attributes in wrappers; a `<context_data>` tag alone is not an injection defense. Enforce authorization and allowed-tool policies outside model text. Poisoned memory must not redefine instructions or widen a worker's permissions.

## 10. AGENT + TOOL ORCHESTRATION

### 10.1 Multi-agent contracts

| Worker pattern | Responsibility and input | Output contract / tools | Budget, timeout, failure, owner |
|---|---|---|---|
| Focused researcher | One independent source question; goal excerpt, source scope, references | Bounded evidence artifact with citations; read/search only | Child lease and parent deadline; partial evidence allowed, no success fabrication; root reconciles |
| Test analyzer | Identify relevant existing checks and interpret supplied results | Test obligations and observed outcomes; read-only by default, execution only when authorized | Separate execution cost allowance; missing result is unknown; controller owns verification |
| Planner | Build implementation dependencies from selected evidence | Versioned plan artifact; no mutation tools | Bounded output/reasoning lease; invalid artifact fails validation; root approves routing |
| Writer | One approved change scope and current input fingerprint | Patch report plus actual changed-file/effect observations; authorized mutation tools only | Exclusive workspace ownership or verified worktree; ambiguous interruption enters reconciliation; controller owns integration |
| Reviewer | Graph-owned diff chunks, checks, risk, and coverage manifest | Issues/verdict plus complete coverage IDs; read-only tools | Protected verification lease; incomplete coverage blocks approval; root resolves conflicts |
| Learning reviewer | Settled verified evidence, scoped candidate memories/skills | Candidate recommendations with source/version IDs; no automatic permission elevation | Low-priority bounded queue, cancellable and deduplicated; failed review must not block foreground completion |

Do not spawn agents for trivial work or send every worker the global transcript. Share immutable retrieval results only when scope and freshness match. Parallelize independent reads, not competing writes or shared-state updates. Keep the existing eight-wide read lane as a baseline; global backpressure may lower it. Separate provider concurrency, local CPU/indexing concurrency, tool lanes, and graph worker limits.

### 10.2 Tool surface and execution policy

Finish a single live capability inventory with full schema, effect class, permission requirements, output policy, idempotency/reconciliation support, and version. Treat MCP hints as advisory classifications subject to policy; unknown effects stay serialized and protected. Advertising a tool never grants execution permission.

Use a small stable core palette per role. Load/disclose additional schemas on demand only through a backend-supported or harness-managed mechanism with integration tests proving they become callable at the next request. Do not advertise nonexistent tools or infer support from a helper module. Stable schema reuse can be better than repeated selection churn; compare both on the same workload.

Route by a verified capability/risk matrix, not model-name marketing. Honor an explicit unavailable model selection with a clear error unless fallback is explicitly configured and surfaced; the inspected main path's provider/first-model fallback deserves a regression test. Prefer the current capable model for high-risk edits until smaller-model paired results justify escalation policies.

### 10.3 Retries, asynchronous work, and ambiguous outcomes

Classify failures as transport-before-send, request accepted/response unknown, provider rejection, malformed model output, tool failure without side effect, partial/ambiguous side effect, deadline/cancellation, and persistence failure. Retry only eligible classes within the shared allowance. A timeout after an irreversible operation is not proof it failed.

Persist operation IDs and reconcile actual filesystem/external state before retrying mutations. Maintain read-only result reuse with content and permission freshness. Preserve the recently improved cancellable retry waits; extend cancellation through queues, nested children, embedding requests, and verification processes. Circuit breakers should be scoped to the failing provider/service and return typed degradation reasons.

Bound asynchronous queues and lifecycle ownership. Make learning and embedding work recoverable, coalesced, and lower priority than foreground work. Account for their costs against the originating task and report both foreground completion and fully settled cost. Avoid abandoned threads surviving a recreated runtime without a reachable cancellation/registry owner.

## 11. OBSERVABILITY + EVALUATION

### 11.1 Event and metric contract

Extend existing runtime/telemetry infrastructure instead of adding a separate observability stack first. Correlate principal-safe scope, session, run, turn, graph/workflow, node, agent, provider attempt, tool call, evidence, memory, and artifact IDs. Include backend profile, model/config/schema/prompt versions, outcome, monotonic timestamps, and usage completeness.

Record disjoint uncached/read/write/output usage, reasoning annotation, actual/estimated price provenance, context estimates versus observed usage, tool schema bytes/tokens, injected memory/evidence IDs and sizes, compression/recovery bytes, retries and fallbacks, admission waits, cancellation latency, and dropped telemetry. Do not include raw prompts, credentials, source text, or encrypted reasoning payloads in ordinary telemetry. Use typed allowlists: blanket removal of every field containing “token” would also erase the numeric metrics needed to evaluate efficiency. [S16]

Dashboards/initial local reports: (1) verified task outcomes and costs, (2) context/cache component changes, (3) memory precision/freshness/injection, (4) graph critical path and admission/worker activity, and (5) recovery/deletion/security failures. Render local JSONL summaries first; add a hosted dashboard only if operational use justifies it.

### 11.2 Representative benchmark suite

Include focused Q&A, exact symbol/error lookup, small edits, multi-file features, broad refactors, long-session compaction, large/minified/multimodal tool outputs, interrupted mutations, stale replay, concurrent scope-separated memory writes, poisoned/contradictory memory, unavailable embeddings/Qdrant, missing runner, budget exhaustion, malformed/refused/incomplete provider output, tool discovery changes, unknown backend profiles, and graph all/any/quorum joins.

Use a fixed manifest with independently run acceptance commands and complete changed-file inventories. Preserve the current `OracleObservation` and manifest fingerprint protections: model-generated success strings and imported boolean-only reports cannot authorize promotion. Cover deleted, new, staged, unstaged, and forbidden files. [S14]

The normal test path remains offline and isolated, with fixture provider/embedding/clock/storage seams. Do not read real account credentials or contact live services. A separately authorized production-like cache experiment may later be needed to prove provider hit rates; synthetic fixture cache counters prove accounting, not actual service cache behavior.

### 11.3 Proposed promotion gates

| Gate | Proposed threshold / method |
|---|---|
| Safety and correctness | 100% of critical permission, false-success, duplicate-side-effect, deletion/isolation, recovery, and hard-cap fixtures pass; zero forbidden mutations. |
| Task quality | No regression on deterministic acceptance cases. For stochastic paired tasks, predeclare a noninferiority margin; starting proposal: lower 95% confidence bound on success-rate difference above -2 percentage points, with all high-risk cases passing. Inconclusive results do not promote. |
| Accounting | Every fixture attempt has one terminal accounting record; totals reconcile exactly to supplied provider events; unknown usage is explicitly unknown. |
| Efficiency | On the fixed paired set, target at least 15% lower total normalized tokens or 15% lower priced cost, with no more than 5% p95 latency regression and the quality gate satisfied. Evaluate both all-run and mutually successful subsets. |
| Memory | Target precision@k >= 0.80 and irrelevant injected tokens <= 10% on a labeled set; no recall loss on required-evidence cases; zero scope/deletion leaks; provenance present for every injected record. |
| Context bounds | All rendered packets and recovery outputs respect configured limits, including Unicode, wrappers, one-line output, images/structured output, and minimum useful recovery metadata. |
| Cache | Exact stable-prefix fixtures and invalidation cases pass. In separately authorized eligible warm-service tests, target a >=20 percentage-point cached-input-share gain or a net cost/latency win; never require a hit when the service does not guarantee it. |
| Durability | Inject interruption/failure at every persistence transition; no fabricated completion or duplicate mutation, and committed artifacts remain readable after a fresh process. |
| Latency | Report p50/p95, first useful result, foreground completion, fully settled time, and graph critical path; do not hide background work beyond the measurement window. |

Use paired seeds/fixtures, counterbalanced order, cold/warm separation, repeated trials, and a held-out task set. Report failures, aborts, missing measurements, and confidence intervals alongside savings. Large gains from failing sooner or skipping verification are rejected. Keep correctness fixes even when they add a small cost; optimize their cost separately without weakening the invariant.

## 12. PRIORITIZED ROADMAP

Each recommendation below is a proposed change. Impact is directional until measured. Difficulty is relative within this repository; no delivery estimates are implied.

### R01 — P0: Complete usage and attempt accounting
**Problem:** Fragmented final-response counters and byte-savings proxies. **Change:** One normalized attempt lineage and whole-run ledger, preserving disjoint protocol categories. **Why/expected impact:** Makes every later optimization and budget enforceable and comparable. **Token impact:** Directly neutral, reveals hidden waste. **Cache impact:** Accurate read/write accounting. **Latency impact:** Small bounded event overhead. **Reliability impact:** Unknown/failure spend becomes visible. **Difficulty:** Medium. **Dependencies:** Baseline inventory. **Risks:** Double counting replay/cumulative events; logging sensitive content. **Verification:** T02 fixtures reconcile retries, children, compaction, learning, partial usage, and duplicate stream events exactly.

### R02 — P0: Make generic workflow success truthful
**Problem:** Missing-runner canned success, ignored persistence errors, incomplete join/resume proof. **Change:** Fail explicitly; validate and durably store outputs before success; distinct-worker joins and effect-aware recovery. **Why/expected impact:** Removes false completion and unsafe retries before adding concurrency. **Token impact:** May spend more than a fake-success path, which is not a valid baseline. **Cache impact:** Neutral. **Latency impact:** Small durability overhead. **Reliability impact:** High. **Difficulty:** Medium. **Dependencies:** T01; R10 for full durable restart. **Risks:** Tests relying on canned fallback; compatibility of old artifacts. **Verification:** Missing runner, disk failure, stale/duplicate/overflow artifacts, and interrupted mutation fixtures.

### R03 — P0: Close output and context-cap holes
**Problem:** First oversized recovery line and uncapped memory-search text; wrapper estimates can overflow. **Change:** Final serialized byte/token checks, continuation offsets, metadata reserve, skip/excerpt oversized candidates. **Why/expected impact:** Prevents accidental context blowups while retaining evidence. **Token impact:** Lower pathological input. **Cache impact:** Fewer emergency compactions. **Latency impact:** Usually lower, occasional explicit retrieval. **Reliability impact:** High under large outputs. **Difficulty:** Low–medium. **Dependencies:** Existing evidence interfaces. **Risks:** Splitting UTF-8 or losing tool/error semantics. **Verification:** T05 adversarial-cap/property-style fixtures and exact recovery reconstruction.

### R04 — P0: Make memory persistence, identity, and deletion safe
**Problem:** Full-file overwrite, inconsistent dedup/ID namespaces, incomplete remote clear. **Change:** Atomic locked authority, versioned IDs, compatibility metadata, durable tombstones/outbox. **Why/expected impact:** Prevents lost/contaminated/resurrected memory. **Token impact:** Less duplicate indexing/injection; migration temporarily costs work. **Cache impact:** Valid embedding reuse and reliable invalidation. **Latency impact:** Lower foreground writes after queue separation. **Reliability impact:** High. **Difficulty:** Medium–high. **Dependencies:** T01, T02. **Risks:** Legacy scope ambiguity, migration failures. **Verification:** T06–T08 concurrency, crash, rollback, deletion, and model-change fixtures.

### R05 — P1: Wire a single context-selection path
**Problem:** Parallel direct injection and unused broker adapters. **Change:** Connect existing broker and source handles with parity-gated replacement of legacy injection. **Why/expected impact:** One budget, provenance, scope, and dedup policy. **Token impact:** Less duplicate/redundant context. **Cache impact:** Stable epochs and explicit suffixes. **Latency impact:** Avoids duplicate retrieval and broad-lock waits. **Reliability impact:** Consistent scope/automatic-retrieval policy. **Difficulty:** Medium. **Dependencies:** R03, R04. **Risks:** Missing context during cutover, stale cloned sources. **Verification:** T11 captures final requests for main/graph/workflow with exactly-once injection.

### R06 — P1: Integrate cache-aware provider profiles and stable surfaces
**Problem:** Helper identities do not prove actual request stability/support. **Change:** Full live schema identity, profile-gated cache controls, stable prefix construction, separate replay fingerprint. **Why/expected impact:** More real prefix reuse without incorrect result reuse. **Token impact:** Primarily shifts uncached to cached; avoids useless cache writes where supported. **Cache impact:** High potential. **Latency impact:** Potential lower prefill; measure cold cases too. **Reliability impact:** Unsupported controls fail safely. **Difficulty:** Medium–high. **Dependencies:** R01, R05, T09. **Risks:** Backend differences, permission leaks, schema churn. **Verification:** T09–T12 request fixtures plus authorized cold/warm paired tests.

### R07 — P1: Bounded, evidence-driven hybrid memory retrieval
**Problem:** Local full scans, fixed score mixing, fixed top-k, weak provenance. **Change:** Bounded lexical/dense candidates, compatible vectors, fusion, dedup/diversity, value-per-token selection. **Why/expected impact:** Better useful context per token and predictable retrieval work. **Token impact:** Target lower irrelevant input. **Cache impact:** Query/embedding reuse; suffix changes remain correctly dynamic. **Latency impact:** Better scale; optional reranker adds latency. **Reliability impact:** Fewer stale/contradictory injections. **Difficulty:** Medium. **Dependencies:** R04, R05, R14. **Risks:** Lost exact-match recall, miscalibration. **Verification:** T13–T15 labeled retrieval and downstream task ablations.

### R08 — P1: Adaptive shared run governor
**Problem:** Static middleware and per-layer limits do not bound total work. **Change:** Atomic parent/child leases, output reserves, real provider caps, workload policy, degradation ladder. **Why/expected impact:** Prevents runaway optional work while protecting quality. **Token impact:** Lower retries/overspend; not indiscriminate truncation. **Cache impact:** Cost-aware but occupancy-correct. **Latency impact:** Less wasted work; admission waiting explicit. **Reliability impact:** High. **Difficulty:** High. **Dependencies:** R01, R03, R05. **Risks:** Deadlocks, unfairness, premature stopping, unknown prices. **Verification:** T16–T18 deterministic clocks and concurrent lease accounting.

### R09 — P1: Shared scheduling and durable graph/workflow recovery
**Problem:** Uneven execution guarantees across two engines. **Change:** Reuse budget/cancellation/evidence primitives; enforce limits and current-input replay compatibility; then bounded concurrency. **Why/expected impact:** Stronger existing architecture without a third orchestrator. **Token impact:** Reuses valid completed research and avoids duplicate work. **Cache impact:** Distinct replay and affinity identities. **Latency impact:** Lower independent-work critical path. **Reliability impact:** High with durable transitions. **Difficulty:** High. **Dependencies:** R02, R08, R10. **Risks:** Concurrent mutations, cancellation races, old manifests. **Verification:** T19–T21 joins, partial resume, changed inputs, and crash tests.

### R10 — P1: Durable, permission-scoped evidence lifecycle
**Problem:** Multiple stores and retention policies can outlive references inconsistently. **Change:** Shared reference/access/pinning contract with store adapters and integrity hashes. **Why/expected impact:** Compression and resume remain genuinely recoverable. **Token impact:** Enables safe compact references. **Cache impact:** Exact evidence reuse. **Latency impact:** Small storage overhead, fewer reruns. **Reliability impact:** High. **Difficulty:** Medium–high. **Dependencies:** R03. **Risks:** Unbounded retention, unauthorized reads, disk-full behavior. **Verification:** T07 pin/GC/restart/permission-revocation fixtures.

### R11 — P2: Adaptive tool disclosure and model routing
**Problem:** Broad or stale schemas and implicit fallback can waste context or change capabilities. **Change:** Versioned role palettes, tested discovery activation, explicit fallback policy, risk-aware routing. **Why/expected impact:** Better capability per input token on suitable workloads. **Token impact:** Potential schema reduction, offset by discovery costs. **Cache impact:** May improve or harm prefix reuse; choose by experiment. **Latency impact:** Discovery/escalation can add turns. **Reliability impact:** Capability gates prevent unsupported routes. **Difficulty:** Medium. **Dependencies:** R06, R08, R14. **Risks:** Hidden required tools, weak-model failure. **Verification:** T22/T24 held-out paired tasks and tool-change fixtures.

### R12 — P2: Gated, accountable learning and consolidation
**Problem:** Broad review triggers and memory writes can consume work or promote weak evidence. **Change:** Outcome-linked write/review gates, source-version tracking, bounded queue, conservative consolidation. **Why/expected impact:** Useful long-term improvement without contaminating context. **Token impact:** Lower duplicate reviews/writes; all maintenance charged. **Cache impact:** Embedding and prompt reuse where valid. **Latency impact:** Less foreground contention. **Reliability impact:** Strong provenance/correction lifecycle. **Difficulty:** Medium. **Dependencies:** R04, R07, R08. **Risks:** Over-filtering useful experience; unsafe automatic promotion. **Verification:** T23 contradictory/corrected/failed-task and repeated-review fixtures.

### R13 — P2/P3: Native continuation and advanced adaptation
**Problem:** Transport/item helpers are not proof of lossless production continuation. **Change:** Connect only after raw native item preservation, lineage, cancellation, and fallback tests; experiment with advanced cache/routing controls per profile. **Why/expected impact:** Less transport and preparation overhead where supported. **Token impact:** No assumed billing reduction from fewer transmitted bytes. **Cache impact:** Potentially better append-only reuse. **Latency impact:** Potential win. **Reliability impact:** Must not regress replay/effect safety. **Difficulty:** High. **Dependencies:** R01, R06, R09, R14. **Risks:** Missing opaque items, stale lineage, backend incompatibility. **Verification:** T25 complete/delta/reconnect/compaction/cancel fixtures before any promotion.

### R14 — P0 foundation / P1 expansion: Runner-driven evaluation
**Problem:** Helpers and historical test reports cannot prove current gains. **Change:** Fixed manifest, real offline task runner, independent oracles, usage capture, paired report generation. **Why/expected impact:** Prevents attractive but false efficiency claims. **Token/cache/latency impact:** Measurement infrastructure, not direct savings. **Reliability impact:** High regression protection. **Difficulty:** Medium. **Dependencies:** T01; richer metrics after R01. **Risks:** Overfitting, incomplete file inventory, missing measurements. **Verification:** T03/T26 deliberately fail fast, forge success, omit tasks, and introduce forbidden changes; report must reject them.

## 13. IMPLEMENTATION PHASES

### Phase 0 — Establish an attributable baseline
**Objective:** A fresh, reproducible account of current behavior and spend. **Components:** Guidance/status inventory, `davinci-ai` usage paths, `davinci-agent` stats/runtime, `davinci-evals`. **Tasks:** T01–T03; preserve previous fixes; capture component fingerprints and actual-path coverage; create offline replay fixtures and independent task manifests. **Dependencies:** Implementation authorization. **Migration:** Additive schemas with explicit unknown fields; no semantic rewrite of existing usage. **Tests/evals:** Provider accounting examples, independent oracle rejection, complete task coverage. **Success metrics:** Exact fixture reconciliation and attributable baseline outputs. **Rollback:** Disable new reporting readers; retain original events and additive metadata. **Done:** No savings claim depends on model assertions, missing counters, or a historical green report.

### Phase 1 — Correctness and bounded-output foundations
**Objective:** Close false-success, evidence, persistence, and cap holes before optimization. **Components:** Workflow executor/state, governor/evidence, vector memory. **Tasks:** T04–T08. **Dependencies:** Phase 0 contracts; individual safety fixes need not await every benchmark. **Migration:** Read legacy artifacts/memory; do not infer verified scope/provenance; tombstones take precedence. **Tests/evals:** Missing runner, disk failure, oversized single line, forbidden recovery read, concurrent writers, clear/upsert race, restart. **Success metrics:** All critical fixtures pass; no evidence loss, false completion, or namespace leaks. **Rollback:** Feature flags for representation changes; preserve forward-readable journals/tombstones; never restore unsafe success fallback. **Done:** Every accepted result has valid recoverable evidence and every configured hard cap is respected.

### Phase 2 — Real provider/context/cache integration
**Objective:** Make existing abstractions affect the actual request once, correctly. **Components:** Main/runtime host, capabilities/context/cache, request/stream adapters. **Tasks:** T09–T12. **Dependencies:** Phase 1 safe references/scopes. **Migration:** Shadow manifests then opt-in broker; legacy injection disabled only for migrated paths; prompt constants unchanged. **Tests/evals:** Main/graph/workflow parity, extension/MCP tool changes, provider/profile rejection, final-wire caps, prefix invalidation. **Success metrics:** Exactly-once injection; deterministic stable request components; no unsupported fields; actual observed cache metrics available. **Rollback:** Revert to the legacy assembly path through a real request-level flag without discarding durable state. **Done:** No unconsumed helper is counted as a shipped optimization.

### Phase 3 — Memory precision and scalable maintenance
**Objective:** Increase useful retrieval per token and remove synchronous write amplification. **Components:** Vector memory, learning retrieval, broker adapters, optional Qdrant projection. **Tasks:** T13–T15 and bounded indexing portions of T23. **Dependencies:** Versioned records/outbox, context integration, eval corpus. **Migration:** Shadow retrieval; lazy compatible re-embedding; old index retained read-only until verification, then retired under deletion policy. **Tests/evals:** Lexical fallback, incompatible dimensions/model revision, exact identifiers, contradiction/expiry, queue interruption, remote outage. **Success metrics:** Section 11 memory gates and measured retrieval latency/maintenance cost. **Rollback:** Switch retrieval to the previous compatible index; keep authoritative writes/tombstones and drain or pause projection jobs safely. **Done:** Every injected memory is scoped, attributable, bounded, and demonstrably useful on the labeled suite.

### Phase 4 — Adaptive admission and graceful degradation
**Objective:** Bound whole-run work while preserving high-value information and output reserve. **Components:** Proposed runtime budget module, provider gateway, tools, graph/worker specs, settings. **Tasks:** T16–T18. **Dependencies:** Reliable usage and final context measurement. **Migration:** Shadow decisions, operator-visible bounded profile, then opt-in enforcement; keep explicit unlimited compatibility. **Tests/evals:** Simultaneous fan-out, cancellation, missing usage, quota borrowing, reasoning/output caps, no-progress loops. **Success metrics:** Zero over-admission in fixtures; overspend uncertainty bounded and visible; quality gate intact. **Rollback:** Revert adaptive policy to conservative fixed leases, not to silent unbounded retries. **Done:** Every new provider attempt and child has an admitted allowance and terminal settlement state.

### Phase 5 — Durable orchestration and measured parallelism
**Objective:** Bring generic workflows to reliable execution and reuse shared guarantees across both engines. **Components:** Workflow executor/state/validation, graph controller/store/replay, runtime lifecycle, worker runner. **Tasks:** T19–T22. **Dependencies:** Budget leases and durable evidence. **Migration:** Versioned run manifests; explicit rejection of incompatible resume; avoid joining old and new mutable authorities. **Tests/evals:** Process restart, all/any/quorum, unscheduled worker state, lost checkpoints, changed workspace content, cancelled children, worktree ownership. **Success metrics:** Zero duplicate mutations; valid partial-result reuse; lower critical path on independent tasks with unchanged acceptance quality. **Rollback:** Serialized scheduler with the same manifests and safety gates. **Done:** Concurrency is real, bounded, cancellable, and no longer merely a spec field.

### Phase 6 — Selective advanced optimization and release proof
**Objective:** Promote only optimizations that pay for their complexity. **Components:** Learning, tool disclosure/routing, native transport, eval/reporting. **Tasks:** T23–T26; experiments in Section 15. **Dependencies:** Stable baseline and all correctness gates. **Migration:** Independent feature flags and per-backend allowlists; one experimental variable at a time before interaction tests. **Tests/evals:** Held-out paired workloads, cold/warm service tests only when separately authorized, native item/reconnect fidelity, full regression suite. **Success metrics:** Section 11 quality plus net efficiency gates; maintainability cost documented. **Rollback:** Disable the losing experiment without data loss or weakening security. **Done:** Accepted measurements, documented limits, and no unverified “maximum capability” claim.

## 14. QUICK WINS

Start with missing-runner failure and artifact-write propagation; close the oversized-first-line recovery hole; cap serialized memory-search results; skip oversized retrieval candidates instead of abandoning selection; honor automatic retrieval consistently; expose memory/index/checkpoint errors as typed diagnostics; and ensure explicit model selection cannot silently fall back without policy.

These are low-complexity relative to a new retrieval engine, but still need focused regression tests. Preserve fail-open full evidence when safe storage/compression is unavailable, subject to the final context gate. Do not obtain a smaller prompt by losing the only copy of an error or mutation outcome.

Other inexpensive improvements after accounting: schema serialization caching keyed by the full live capability version, query-embedding reuse inside a request/snapshot, avoiding repeated record/vector clones, and distinguishing historical replay usage from newly charged attempts. Do not re-enable unsafe duplicate-search suppression using Git status alone.

## 15. HIGH-LEVERAGE EXPERIMENTS

| Experiment | Hypothesis / implementation | Measurement and proposed success threshold | Decision rule |
|---|---|---|---|
| E1: Broker versus current direct assembly | One context plan removes duplicate retrieval/injection; shadow then switch only the selected path | Whole-run tokens, exactly-once context IDs, task quality; >=10% input reduction on affected tasks with no required-evidence loss | Promote only with parity and isolation gates; otherwise keep adapters and fix selection |
| E2: Stable palette versus dynamic tool disclosure | Smaller early schemas can beat discovery overhead on tool-rich tasks | Schema tokens, discovery calls, native cache share, completion quality; >=10% net total cost or token gain | Keep stable palette for short/simple tasks if discovery adds more than it saves |
| E3: Explicit cache boundaries by verified profile | Avoiding cache writes for changing suffixes reduces net cost | Actual read/write/uncached usage, cold/warm cost, latency; >=10% cost gain on repeat workloads without regression | Enable only for proved backend/model profiles; no generic OAuth/proxy inference |
| E4: Local bounded index versus Qdrant query | Remote projection pays for itself only beyond a corpus/workload threshold | Labeled recall/precision, p95 retrieval, RAM, index maintenance/outage behavior; >=30% retrieval p95 improvement at equal quality and acceptable operations | Below break-even keep local retrieval and disable unnecessary remote projection |
| E5: Fusion/diversity versus fixed weighted scoring | Better candidate ranking lowers irrelevant context | Precision, required-evidence recall, injected tokens, downstream tasks; >=20% lower irrelevant tokens and no recall regression | Adopt cheap fusion first; add reranker only if incremental net gain survives its cost |
| E6: Adaptive leases versus fixed bounded policy | Workload-aware allocations avoid unnecessary agents/retries without hurting hard tasks | All-attempt tokens/cost, budget stops, task quality; >=15% gain with Section 11 noninferiority | Keep fixed policy for workloads with insufficient evidence or unstable adaptation |
| E7: Simple route versus full engineering route | Low-risk tasks need fewer orchestration stages | Verified success, changed-file safety, worker count, critical path; >=20% lower total tokens or latency on low-risk slice | Never bypass mandatory checks; retain full route for high-risk or uncertain cases |
| E8: Gated writes/reviews versus broad learning | Fewer higher-quality memories outperform broad conversation accumulation | Maintenance cost, stale/contradiction rates, held-out future-task quality; >=25% maintenance reduction without downstream regression | Reject gates that erase useful explicit corrections or durable constraints |
| E9: Native continuation versus full replay | Lossless native continuation saves transport/latency | Native item fidelity, bytes, request CPU, billed tokens/cost, reconnect safety; >=15% transport/preparation improvement with exact fixture equivalence | Promote for supported profiles only; do not market byte reduction as token savings |
| E10: Smaller compatible embeddings | Lower-dimensional/model alternatives may reduce storage/latency | Exact-ID and semantic recall, precision, task impact, RAM/index cost | Promote only after noninferior retrieval/task quality; otherwise retain current model/dimensions |

All thresholds are candidate experiment gates and may be tightened after Phase 0. An unsuccessful experiment is an acceptable outcome; remove or leave disabled complexity that does not provide a measured benefit. Do not run all experiments simultaneously and attribute aggregate gains to each one.

## 16. IMPLEMENTATION BACKLOG

### Execution rules for every ticket

All tasks are unchecked and require separate implementation authorization. Before editing, re-read affected files and relevant guidance. Add a failing regression or invariant test in the existing inline `#[cfg(test)]` modules, implement the smallest change, run the focused offline test, then the affected crate tests. Use provider/embedding/clock/storage seams rather than real network calls. Update serialized compatibility fixtures when required. Never stage unrelated changes.

Paths below are relative to the repository. Proposed new files are labeled **new**. Keep production changes out of archived `davinci-core`. A future worker must verify symbols against the current tree because this document's line numbers are a dated snapshot.

### T01 — P0: Freeze the implementation baseline and coverage map
- [ ] **Files:** existing `AGENTS.md`, `CLAUDE.md`, September 5 plans/checkpoint; proposed fixtures/report types within `crates/davinci-evals/src/`.
- [ ] Record fresh HEAD/status and hashes of all affected files without resetting the dirty tree. Classify old checkpoint work as preserved, unfinished, or superseded.
- [ ] Map actual request consumers of context/cache/capabilities/flags/telemetry/Responses ledger; mark declarations without consumers. Identify the currently supported provider profiles from configuration schemas without reading secrets.
- [ ] **Test/acceptance:** an explicit manifest links each optimization to a live request-path test and each historical failure to a fresh result or an unresolved entry. **Dependencies:** none. **Rollback:** documentation/fixture inventory only.

### T02 — P0: Normalize and aggregate every usage attempt
- [ ] **Files:** `crates/davinci-ai/src/{types,stream,stream_decoder,stream_decoder_anthropic,stream_decoder_completions,cost}.rs`, `crates/davinci-protocol/src/schemas.rs`, `crates/davinci-agent/src/stats.rs`; **new** `crates/davinci-agent/src/runtime/usage.rs`.
- [ ] Add unique attempt lineage, raw usage provenance/completeness, normalized disjoint categories, and deduplicated final aggregation. Separate new spend from historical replay credit.
- [ ] **Tests:** inclusive input=1,000/read=600/write=100 becomes uncached=300; disjoint Claude categories remain disjoint; output=200/reasoning=150 remains 200 charged output. Duplicate events, partial failure, nested retries, and child/background usage count once.
- [ ] **Acceptance:** exact fixture reconciliation; missing prices/usage are unknown, not zero. **Dependencies:** T01. **Rollback:** additive readers/flag; retain raw-compatible usage.

### T03 — P0: Build a real offline benchmark runner
- [ ] **Files:** `crates/davinci-evals/src/{lib,codex_eval}.rs`; **new** `crates/davinci-evals/src/harness_eval.rs` with inline tests and a versioned fixture manifest.
- [ ] Execute controlled tasks through injected runner/provider seams, independently observe allowed file changes and acceptance commands, and construct `OracleObservation` outside assistant output.
- [ ] **Tests:** fabricated “tests passed,” missing/duplicate tasks, forbidden/new/deleted files, incomplete inventory, abort, and zero/unknown token metrics cannot pass promotion. Preserve existing manifest-bound comparisons.
- [ ] **Acceptance:** produces cold/warm and all-run/mutually-successful paired reports with explicit missing data. **Dependencies:** T01; richer usage after T02. **Rollback:** no production behavior change.

### T04 — P0: Eliminate workflow false success and unsafe completion
- [ ] **Files:** `crates/davinci-agent/src/runtime/workflow/{executor,state,validate}.rs`.
- [ ] Require a real or explicitly injected fixture runner; validate/store artifacts before task success; propagate persistence failure. Mark unscheduled early-join workers skipped/cancelled; retain actual completed/failed IDs and retry counts.
- [ ] Make retry decisions distinguish read-only failure from uncertain mutation; stop and reconcile uncertain effects rather than blindly repeating the worker.
- [ ] **Tests:** missing runner, disk-full/store failure, malformed output, any/quorum early exit, and mutation-then-error. **Acceptance:** zero false completed phases. **Dependencies:** T01. **Rollback:** preserve stricter failure semantics; update fixture setup instead of restoring canned success.

### T05 — P0: Enforce final recovery and memory-output caps
- [ ] **Files:** `crates/davinci-coding-agent/src/native_extensions/{token_governor,vector_memory}.rs`, `.../ecosystem/context.rs`, `crates/davinci-agent/src/runtime/context.rs`.
- [ ] Reserve space for reference/truncation metadata; split oversized lines with valid UTF-8 offsets; return continuation metadata; bound model-visible memory-search text. Skip/excerpt oversized candidates and remeasure the final rendered packet.
- [ ] **Tests:** a single 100,000-byte line under a 4,096-byte cap; multibyte text; tiny caps; wrapper-heavy IDs; one huge first candidate followed by small relevant candidates; error/multimodal preservation.
- [ ] **Acceptance:** hard limits hold and full evidence remains reconstructable. **Dependencies:** T01. **Rollback:** safe original evidence retention plus admission failure, not arbitrary truncation.

### T06 — P0: Version memory identity and atomic writes
- [ ] **Files:** `crates/davinci-coding-agent/src/native_extensions/vector_memory.rs`; **new if needed** `.../native_extensions/memory_store.rs` with inline tests.
- [ ] Define namespace/kind/content identity consistently, separate logical repo and workspace, add schema/embedding compatibility metadata, and replace unsafe overwrite with locked atomic persistence.
- [ ] Migrate legacy records conservatively; unknown scope/provenance must not be upgraded to trusted global memory. Record errors and migration counts.
- [ ] **Tests:** concurrent process-equivalent snapshots, interruption before replacement, same text/different kind or scope, case-sensitive paths, equal dimension/different embedding model. **Acceptance:** no lost committed records or scope collision. **Dependencies:** T01/T02. **Rollback:** preserved legacy import plus versioned authoritative export.

### T07 — P0/P1: Introduce durable evidence references and pinning
- [ ] **Files:** `crates/davinci-agent/src/{evidence,pruning}.rs`, `.../runtime/workflow/state.rs`, coding-agent `.../token_governor.rs`, `.../graph/store.rs`.
- [ ] Specify `EvidenceRef` ownership, full integrity hash, size, access policy, pin holders, and deletion/expiry state; adapt existing stores without a forced rewrite. Persist a reference before withholding content.
- [ ] **Tests:** fresh-process recovery, active checkpoint pin surviving retention sweep, permission revocation, corrupt/missing evidence, disk failure, and release/GC races.
- [ ] **Acceptance:** no advertised recovery reference becomes unreadable while legitimately pinned; unauthorized reads fail. **Dependencies:** T05. **Rollback:** keep existing stores behind adapters and preserve live pins.

### T08 — P0/P1: Durable memory outbox and complete deletion
- [ ] **Files:** vector memory/store from T06, `crates/davinci-coding-agent/src/native_extensions/mod.rs`, `extension_host.rs`, `main.rs`.
- [ ] Commit record plus embedding/upsert/delete job durably; process bounded jobs outside host locks. Tombstone first, then delete every projection/cache; prevent stale upsert resurrection.
- [ ] **Tests:** clear during queued upsert, Qdrant unavailable, retry after restart, duplicate job delivery, deleted derived summary, and foreground query during indexing.
- [ ] **Acceptance:** immediate local retrieval exclusion, visible remote-pending state, eventual acknowledged deletion, no foreground full embedding pass under the broad host lock. **Dependencies:** T06/T07. **Rollback:** pause queue safely; never discard tombstones.

### T09 — P1: Connect backend capability and feature profiles
- [ ] **Files:** `crates/davinci-ai/src/{codex_capabilities,codex_flags,cache,request,stream}.rs`, coding-agent `main.rs`/settings integration.
- [ ] Preserve exact-origin/auth classification; make feature flags actual request consumers with a conservative fallback. Encode supported cache/structured/reasoning/continuation controls by profile, not generic OAuth inference.
- [ ] **Tests:** public API, known authenticated backend, unknown proxy, incompatible provider, malformed URL, unsupported field, and disabled feature flag reaching final wire output.
- [ ] **Acceptance:** no unsupported controls emitted; cache read/write usage remains profile-correct. **Dependencies:** T02. **Rollback:** legacy-compatible provider path via explicit flag.

### T10 — P1: Make authorized tool schemas a versioned live surface
- [ ] **Files:** `crates/davinci-agent/src/runtime/capabilities.rs`, `crates/davinci-agent/src/{mcp,permission}.rs`, coding-agent `extension_host.rs`, `main.rs`, `.../ecosystem/cache_affinity.rs`.
- [ ] Hash full current built-in/native/JS/MCP definitions and effective permission scope. Rebuild/activate the snapshot at the appropriate provider boundary after extension or discovery changes; memoize serialization by version.
- [ ] **Tests:** same names/different schemas, registration/unregistration, tool-order permutations, permission revocation, before-start hook changes, and mid-loop discovery.
- [ ] **Acceptance:** advertised tools, authorization, cache identity, and actual executor agree. **Dependencies:** T09. **Rollback:** stable old palette with current permissions; never stale authorization.

### T11 — P1: Wire the existing context broker without double injection
- [ ] **Files:** `crates/davinci-agent/src/runtime/context.rs`, coding-agent `runtime_host.rs`, `main.rs`, native `vector_memory.rs`, learning adapters, `.../ecosystem/context.rs`.
- [ ] Use shared live sources with explicit scopes/versions; respect automatic retrieval; collect without broad host locks; disable the matching legacy injector per migrated path.
- [ ] **Tests:** captured main/graph/workflow requests show a memory once; false automatic-retrieval flag yields none; source updates are visible; deadline/failure returns bounded partial context; escaped wrappers preserve data status.
- [ ] **Acceptance:** final serialized cap, provenance manifest, no cross-scope context. **Dependencies:** T05/T08/T10. **Rollback:** assembly flag plus legacy parity capture.

### T12 — P1: Separate prefix affinity and replay fingerprints
- [ ] **Files:** `crates/davinci-agent/src/runtime/cache.rs`, coding-agent `.../ecosystem/cache_affinity.rs`, `.../graph/{replay,worker}.rs`, provider cache/request shaping.
- [ ] Split stable prefix grouping from exact ordered request/replay identity; include full live schemas and isolation; keep dynamic suffix out of affinity while in replay. Version keys and log component change reasons.
- [ ] **Tests:** changed timestamp/task suffix retains eligible stable affinity; changed permission/model/tool schema/contract invalidates; reordered evidence changes replay when rendered order changes; scope separation always holds.
- [ ] **Acceptance:** actual request fixtures—not hash helpers alone—prove boundaries. **Dependencies:** T09–T11. **Rollback:** old key version with correct replay rejection.

### T13 — P1: Build labeled memory retrieval evaluation
- [ ] **Files:** `crates/davinci-evals/src/harness_eval.rs`, native memory/learning inline fixtures.
- [ ] Create labeled exact-identifier, semantic, stale, contradictory, adversarial, cross-scope, and no-answer cases; record useful spans and required provenance.
- [ ] **Tests:** metrics expose irrelevant tokens and missed mandatory evidence; a repeated cited false memory does not count as useful.
- [ ] **Acceptance:** precision/recall and downstream task measurements can be computed for old/new retrieval from the same snapshot. **Dependencies:** T03/T06. **Rollback:** evaluation-only.

### T14 — P1: Implement bounded hybrid candidate selection
- [ ] **Files:** native `vector_memory.rs`, **new if needed** `.../native_extensions/memory_retrieval.rs`, broker source adapter.
- [ ] Bound lexical/dense candidates, honor candidate limits, avoid full vector clones, fuse ranks, deduplicate against current context, preserve exact identifiers, and pack by measured rendered cost. Add adaptive k and optional diversity.
- [ ] **Tests:** fallback without embeddings, candidate ceiling, incompatible vectors, oversized-first-hit, duplicate history, weak-query empty result, and source freshness.
- [ ] **Acceptance:** Section 11 retrieval gates and no downstream quality regression. **Dependencies:** T11/T13. **Rollback:** previous compatible retrieval strategy with the new hard caps/isolation retained.

### T15 — P1/P2: Decide and implement the vector projection strategy
- [ ] **Files:** memory store/retrieval adapters, native configuration and status, optional Qdrant request fixtures.
- [ ] Benchmark local indexed/bounded retrieval versus filtered Qdrant queries at representative corpus sizes. Choose an explicit local or remote projection profile; disable unused remote work. Add version/collection checks and rebuild tooling only for the chosen path.
- [ ] **Tests:** projection lag, missing collection, service outage, namespace filter, reindex after model change, and tombstone exclusion.
- [ ] **Acceptance:** documented break-even decision and matching quality/latency gates. **Dependencies:** T08/T13/T14. **Rollback:** authoritative local store plus compatible fallback.

### T16 — P1: Add atomic run-budget leases
- [ ] **Files:** **new** `crates/davinci-agent/src/runtime/budget.rs`, runtime module/handle, stats/events.
- [ ] Implement admission, child reservation, protected verification/output allowance, monotone settlement, cancellation, and explicit unknown-cost handling. Keep policy separate from ledger arithmetic.
- [ ] **Tests:** concurrent reservations cannot over-admit; duplicate settlement is idempotent; cancelled-but-unconfirmed attempts do not free uncertain spend; budget borrowing preserves mandatory reserve.
- [ ] **Acceptance:** deterministic accounting invariants under concurrent fixture schedules. **Dependencies:** T02/T07. **Rollback:** conservative fixed leases via policy selection.

### T17 — P1: Enforce provider/tool/child limits at execution boundaries
- [ ] **Files:** `crates/davinci-agent/src/{turn,subagent,tools}.rs`, coding-agent `main.rs`, `.../graph/{types,worker,controller}.rs`, workflow executor, provider stream options.
- [ ] Require a lease before each actual attempt/spawn/tool run, pass supported output limits, share retry budgets, propagate deadline/cancellation, and settle children without double counting streamed and final usage.
- [ ] **Tests:** agent/provider/worker retry multiplication, incomplete output reserve, nested subagents, slow child cancellation, no-price provider, and late usage after abort.
- [ ] **Acceptance:** zero unadmitted fixture attempts and explicit bounded overspend uncertainty. **Dependencies:** T09/T16. **Rollback:** serialized fixed-budget path.

### T18 — P1: Add adaptive context-pressure and workload policy
- [ ] **Files:** runtime budget/context, agent pruning/compaction, coding-agent settings/status, graph routing policy.
- [ ] Start shadow-mode workload classification and pressure bands; implement Section 8 degradation in order with reason codes. Preserve explicit unlimited settings while introducing bounded profiles.
- [ ] **Tests:** mandatory context too large, low-value memory drops first, optional agent skipped, essential verification retained, summary cost charged, and honest budget-exhausted outcome.
- [ ] **Acceptance:** quality gate plus whole-run improvement against fixed bounded policy. **Dependencies:** T11/T14/T17. **Rollback:** fixed policy; no data loss.

### T19 — P1: Persist workflow lifecycle and current-input resume proof
- [ ] **Files:** runtime workflow executor/state, runtime host/main lifecycle, session runtime-log integration.
- [ ] Persist spec, artifact metadata/indexes, transitions, dependency fingerprints, effect status, and ownership. Reuse long-lived service registries appropriately. Validate distinct expected worker outputs and resolve overflow content before resume.
- [ ] **Tests:** fresh process with inline/overflow artifacts, altered spec/tool/permission/source content, duplicate quorum outputs, checkpoint write failure, and unknown mutation outcome.
- [ ] **Acceptance:** only committed compatible outputs resume; no false joins or duplicate effects. **Dependencies:** T04/T07/T17. **Rollback:** reject unsupported manifests rather than unsafe replay.

### T20 — P1: Enforce generic workflow scheduling and limit semantics
- [ ] **Files:** runtime workflow executor/spec/validate, agent scheduler/subagent interfaces.
- [ ] Enforce max parallel agents, total admissions/attempts, max turns, deadline, and known-cost cap. Run independent work concurrently only under leases and effect-aware isolation; make all/any/quorum cancellation and states correct.
- [ ] **Tests:** barrier-controlled overlap, global max concurrency, early quorum, cancellation before admission, retry count limits, and actual worktree path/working directory propagation.
- [ ] **Acceptance:** measured concurrency with deterministic joins and all safety fixtures. **Dependencies:** T17/T19. **Rollback:** width-one scheduler preserving budgets and durable state.

### T21 — P1: Integrate engineering graph budgets, replay, and checkpoint failure handling
- [ ] **Files:** coding-agent `.../graph/{controller,topology,replay,store,process,review_coverage,verify}.rs`, ecosystem resource/telemetry.
- [ ] Reserve before fan-out, retain role contracts, split historical/new spend, stop on recovery-critical persistence failure, and reconcile writer ambiguity. Verify every graph mode's risk-dependent review/verification obligations.
- [ ] **Tests:** dirty/ignored target changes, resumed research, child timeout, graph-owned versus unrelated mutations, incomplete review coverage, and finite bounded policy alongside legacy zero values.
- [ ] **Acceptance:** no duplicate mutations or skipped mandatory review; replay savings visible without rebilling history. **Dependencies:** T12/T17/T19. **Rollback:** existing topology with conservative serial/bounded execution.

### T22 — P2: Prove tool discovery activation and role-palette trade-offs
- [ ] **Files:** capabilities/MCP/tool manager, provider `deferred.rs`/request shaping, main request boundary, eval runner.
- [ ] Wire discovery to real authorized schema activation where supported. Compare stable role palettes with deferred/dynamic disclosure; preserve tool-loading history and correctness.
- [ ] **Tests:** newly discovered tool callable next request, denied tool never executable, incompatible backend fallback, schema version change, and discovery budget exhaustion.
- [ ] **Acceptance:** E2 net benefit and unchanged task acceptance. **Dependencies:** T10/T12/T17/T20. **Rollback:** stable palette.

### T23 — P2: Gate and account for learning/consolidation
- [ ] **Files:** native `learning/{evidence,reviewer,retrieval,policy,store,skill_manager}.rs`, vector memory, settled main path.
- [ ] Coalesce evidence review by source version/outcome; maintain bounded low-priority work; gate durable writes; preserve contradictions/corrections and exact injected skill version attribution.
- [ ] **Tests:** same settled event twice, failed verification, permission denial, stale skill version, user correction, tombstoned source, and queue cancellation/restart.
- [ ] **Acceptance:** E8 cost and downstream-quality gates; no unverified automatic promotion. **Dependencies:** T08/T13/T17. **Rollback:** disable optional promotion/review while preserving evidence.

### T24 — P2: Risk-aware routing and simple-path ablations
- [ ] **Files:** graph config/controller/topology, coding-agent model selection/main, provider capabilities, eval runner.
- [ ] Make explicit model fallback policy observable; compare current/full/simple routes and approved model escalation on held-out workload classes. Do not change mandatory safety checks to win latency.
- [ ] **Tests:** unknown requested model, model lacking required capability, high-risk change forced toward simple route, escalation with budget reserve, and retained verification.
- [ ] **Acceptance:** E7 and model-routing quality gates. **Dependencies:** T03/T09/T18/T21. **Rollback:** current known-capable route.

### T25 — P2/P3: Native Responses fidelity and continuation integration
- [ ] **Files:** `crates/davinci-ai/src/{responses_ledger,codex_transport,codex_ws,request_shape,stream,stream_decoder}.rs`, session persistence integration.
- [ ] Preserve actual native items and opaque payloads instead of reconstructing everything from flattened chat; integrate lineage, delta/full replay choice, flags, and reconnect/cancel behavior only for verified profiles.
- [ ] **Tests:** function/custom tools, reasoning/opaque items, item/call IDs, interrupted streams, changed schema, expired lineage, compaction, and safe fallback without repeated side effects.
- [ ] **Acceptance:** E9 plus exact semantic fixture replay; billed context remains measured separately from wire bytes. **Dependencies:** T09/T12/T19/T21. **Rollback:** faithful full replay or explicit unsupported result, never guessed state.

### T26 — P1 release gate: Run full paired regression and document accepted state
- [ ] **Files:** eval runner/reporting, affected inline tests and existing parity fixtures; update this plan's execution record only after authorization.
- [ ] Run focused then affected-crate/workspace offline checks, distinguish pre-existing unrelated failures, produce paired/held-out reports, and verify every promoted flag affects real requests. Record source/config/model fingerprints and unresolved limits.
- [ ] **Acceptance:** all Section 11 critical gates, noninferiority, measured net improvement, complete coverage, and reversible rollout. **Dependencies:** every promoted ticket; experiments may remain disabled.
- [ ] **Rollback:** individually disable losing optimizations; retain correctness, isolation, accounting, and durable-state fixes.

### Future verification commands (not run for this plan)

Use isolated fixture-owned session/home/memory/evidence directories and offline provider/embedding doubles. The repository's learning fixtures sometimes require the background reviewer path to remain enabled while still using offline providers; do not blanket-disable it and misclassify the resulting fixture failures.

```powershell
# Run only after implementation is explicitly authorized and fixture isolation is set up.
$env:PI_OFFLINE = '1'
cargo test -p davinci-ai
cargo test -p davinci-agent
cargo test -p davinci-coding-agent
cargo test -p davinci-evals
cargo test --workspace
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

These commands are a verification sequence, not a claim that the current tree passes. Focused test filters should be run first for each new inline regression, with deterministic injected seams and no real credentials. A failure in unrelated pre-existing TUI work is recorded and attributed, not silently fixed in an efficiency ticket.

## 17. EXPECTED END STATE

The harness should remain recognizably Davinci/pi-rust: the same Rust product, provider compatibility, permissions, session ecosystem, specialized engineering roles, and useful native extensions. The improvement is in dependable composition: one trustworthy account of work; real parent/child budgets; compact recoverable evidence; scoped and useful memory; versioned stable request components; truthful workflows; and evaluation that can reject false efficiency.

A successful request should use the smallest sufficient route, inject only justified context, avoid repeated authorized reads when freshness is proved, retain expensive reusable prefixes where the backend permits, and reserve enough capacity to verify and finish. A complex request should fan out only independent work, reconcile through typed artifacts, preserve mutation ownership, and resume committed compatible work without blindly replaying side effects.

Limits remain. Provider behavior, cache eviction/routing, reasoning needs, and external-service latency cannot be guaranteed by an application key. Token estimation is imperfect unless a supported exact counter covers the final wire representation. Retrieval can miss evidence; memory can be wrong or stale despite provenance. External exactly-once effects require service-side idempotency or reliable reconciliation. Limited budgets can legitimately stop a task. OAuth/subscription monetary cost may remain unknown. Advanced rerankers, semantic result caches, or speculative agents may not earn their operational complexity and should remain disabled when they do not.

### Verification and remaining uncertainty

This plan distinguishes observed implementation from declarations and inferred risks; preserves existing fixes rather than repeating the older plan; accounts for retrieval, retries, compaction, and learning instead of moving costs off-screen; separates prompt caching from result/replay caching; requires scope-safe deletion and recovery; and conditions added graph complexity on measurable benefit.

The audit did not run or establish a fresh build/test baseline, inspect private production logs or credentials, measure actual service/cache behavior, count a production memory corpus, prove installed Qdrant/Ollama versions, or establish account pricing. Native authenticated Codex backend behavior is not inferred from public API documentation. Components outside the inspected paths require the Phase 0 consumer map before their behavior is changed. Those uncertainties affect expected magnitude and rollout order, not the observed code-level safety and integration findings.

## 18. NEXT 5 ACTIONS

1. **Authorize implementation separately, then execute T01:** refresh the working-tree/hash inventory and reconcile this plan with the September 5 checkpoint without touching unrelated changes.
2. **Execute T02:** establish unique provider-attempt and child/background usage accounting with disjoint cache/input/output fixtures before making token-saving claims.
3. **Execute T03:** build the offline task manifest and independent runner/oracles so current outcomes, costs, and missing measurements are reproducible.
4. **Execute T04 and T05 as focused safety work:** remove workflow false success and close recovery/memory-output hard-cap holes, retaining recoverable evidence.
5. **Execute T06–T08:** make memory identity, atomic persistence, evidence retention, and deletion safe before broker integration, adaptive retrieval, or wider concurrency.

### Repository evidence index

Line ranges identify inspected working-tree evidence, not immutable links. Revalidate them against the file before implementation. Paths are relative to the repository unless noted.

| ID | Source and inspected anchors |
|---|---|
| S01 | `crates/davinci-coding-agent/src/main.rs`: `system_prompt_with_identity` 471–485; `complete_prompt_with_host` 1589–1970; settled indexing 2016–2026. Hash observed `cf5b5168e03a3d28`. |
| S02 | `crates/davinci-agent/src/turn.rs`: loop/pruning/provider boundary 102–145; retry owner 408–539; tool scheduling 542–600. Hash `6f65b1bad2da8480`. |
| S03 | `crates/davinci-agent/src/scheduler.rs`: read/mutation lanes 22–56, execution 79–173; `stats.rs`: `RunStats` 43–77. |
| S04 | `crates/davinci-agent/src/runtime/context.rs`: `ContextRequest`, `ContextItem`, broker collection/selection/rendering/cache-key implementation; inspected through line 260. Hash `418518d0c09fb313`. |
| S05 | `crates/davinci-agent/src/runtime/cache.rs`: `CacheIdentity`, key/diff and tool-name hashing; full file inspected. Hash `263bb491106aac6a`. |
| S06 | `crates/davinci-coding-agent/src/native_extensions/vector_memory.rs`: config/extraction, source adapter 609–658, persistence 742–775, indexing 789–894, search 911–1033, tools/injection/clear 1065–1117, embedding/upsert 1158–1267, richer learning records 1271–1325. Hash `e2603988d3f9865f`. |
| S07 | `crates/davinci-coding-agent/src/native_extensions/token_governor.rs`: config/exemptions, output store, compression/ledger/middleware, recovery 1032–1097; inspected through line 1120. Hash `943db6120513fb27`. |
| S08 | `crates/davinci-coding-agent/src/runtime_host.rs`: source-registration helpers 164–200 and runtime subscribers; `native_extensions/ecosystem/cache_affinity.rs`: graph/cache identities and built-ins registry hashing, inspected through line 260. |
| S09 | `crates/davinci-protocol/src/schemas.rs`: `Usage::from_tokens` 157–184; `crates/davinci-ai/src/types.rs`: usage fields. Protocol hash `78f2ab02229c1821`. |
| S10 | `crates/davinci-coding-agent/src/native_extensions/graph/types.rs`: graph budgets 412–447 and usage 451–481; `graph/topology.rs`: node/edge types and validation 1–185. |
| S11 | `crates/davinci-coding-agent/src/native_extensions/graph/controller.rs`: checkpoint 163–184, budgets 213–254, worker spec/readiness/replay 280–399, usage/artifact completion 600–703. Hash `51664945c45cb5c0`. |
| S12 | `crates/davinci-agent/src/runtime/workflow/executor.rs`: lifetime/background execution 88–265; resume 280–376; phase execution 400–664. `workflow/spec.rs`: complete declared limits. Executor hash `59905fa24911197e`. |
| S13 | `crates/davinci-agent/src/runtime/workflow/state.rs`: in-memory indexes and overflow writes 67–185; `runtime/mod.rs`: runtime ownership/rehydration. State hash `79ff326a8ee2479d`. |
| S14 | `crates/davinci-evals/src/codex_eval.rs`: metrics, independent oracle, manifest-bound report construction 34–185. Hash `b3aaeb7ca30bc02c`. |
| S15 | `docs/superpowers/plans/2026-09-05-openai-harness-efficiency-reliability.md` (opening audit/plan); `...-execution.md` (complete checkpoint). Historical claims only where not independently inspected. |
| S16 | `crates/davinci-ai/src/codex_telemetry.rs` through line 220; provider usage/cache-write search results in `stream.rs`, `stream_decoder.rs`, `stream_decoder_anthropic.rs`, and `images.rs`. |
| S17 | `crates/davinci-ai/src/responses_ledger.rs` through line 260; repository call-site searches for ledger/capabilities/feature flags; `davinci-ai/cache.rs` through line 260. |
| S18 | `crates/davinci-coding-agent/src/native_extensions/ecosystem/context.rs`: packet selection/rendering 133–240; `learning/evidence.rs`: review gates 175–240. |
| S19 | Root `AGENTS.md`, `CLAUDE.md`, `Cargo.toml`, `Makefile`, `package.json`; workspace/project info, project tree, initial Git status. |

### First-party documentation checked on 2026-09-06

Documentation establishes public documented behavior, not observed behavior of this installation. Recheck before implementation and record the backend/model/service version in fixtures. No current prices or model ranking are assumed in this plan.

- **D1 — OpenAI prompt caching:** https://developers.openai.com/api/docs/guides/prompt-caching — rendered-prefix matching, cache-key routing, model-specific modes/retention/breakpoints, and read/write usage. The architectural design above is a proposal; the short compatibility notes and normalization references are the documentation-derived facts.
- **D2 — OpenAI reasoning models:** https://developers.openai.com/api/docs/guides/reasoning — generation limits include reasoning; incomplete output must be handled; model context behavior differs by capability.
- **D3 — Claude prompt caching:** https://platform.claude.com/docs/en/build-with-claude/prompt-caching — separate ordinary input, cache-read, and cache-creation usage categories and provider-specific cache controls.
- **D4 — Ollama embedding API:** https://docs.ollama.com/api/embed — batched input, dimensions, and explicit truncation behavior.
- **D5 — Google EmbeddingGemma inference:** https://ai.google.dev/gemma/docs/embeddinggemma/inference-embeddinggemma-with-sentence-transformers — model-specific embedding usage and 768-dimensional examples; no smaller-vector quality claim is imported.
- **D6 — Qdrant hybrid queries:** https://qdrant.tech/documentation/search/hybrid-queries/ — candidate prefetch/fusion capabilities with version-dependent features.
- **D7 — OpenAI conversation state:** https://developers.openai.com/api/docs/guides/conversation-state — previous-response chaining does not make previous input tokens unbilled. This supports separating transport savings from usage savings.
- **D8 — OpenAI structured outputs:** https://developers.openai.com/api/docs/guides/structured-outputs — schema-constrained output still needs handling for refusals and incomplete responses; a schema is not an independent task-success oracle.

**Planning stop condition:** Current architecture mapped sufficiently for prioritized action; major safety/cost/context bottlenecks identified; graph, memory, governor, caching, and context changes specified; dependencies, phases, tests, rollback, and uncertainty documented. Implementation remains unstarted.
