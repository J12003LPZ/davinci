# DaVinci Harness Optimization Program — Design

**Date:** 2026-09-14  
**Branch:** `feat/davinci-harness-optimization-20260914`  
**Baseline:** `main@a4389b2d60dfb5f3ddeb37e75972ff65bdb013e9`

## Goal

Improve DaVinci's existing agent-harness features without creating parallel subsystems. The primary product objective is:

> Maximize independently verified software-engineering success while reducing provider cost, context pressure, retries, latency, and maintenance burden.

The program extends the systems already present on `main`: Capability Toolbox v1, Token Governor, root context accounting, verification evidence, `RuntimeCapabilityRegistry`, durable tool ledger, Graph, vector memory, learning, MCP, semantic navigation, security scan, and `davinci-evals`.

## Non-goals

This program does **not** add:

- another Graph/orchestration framework;
- another capability or permission registry;
- another memory database;
- another generic planner;
- a model-based Token Governor summarizer;
- an always-on LSP daemon;
- automatic subagents or Graph execution for every task;
- a second skill-learning store;
- broad prompt rewrites unrelated to the features below.

Normal interactive prompt semantics, existing permission boundaries, exact-output recovery, legacy `.pi` compatibility, JSON/print/RPC compatibility, and current provider wire contracts must remain intact unless a task below explicitly changes an internal implementation detail.

---

# Architecture Principles

## 1. Existing authority stays authoritative

The following remain the single sources of truth:

- `RuntimeCapabilityRegistry` for capability metadata;
- permission policy for authorization;
- `ToolExposureState` for provider-visible schema exposure;
- `ResourceLedger` for root/child resource accounting;
- `OutputStore` for exact Governor output recovery;
- `ToolCallLedger` for exactly-once/recovery evidence;
- existing vector-memory and learning ledgers for durable knowledge;
- deterministic Graph verification for pass/fail judgments.

No feature in this program may re-encode those decisions in a competing subsystem.

## 2. Deterministic first, model calls only where the product already requires a model

Selection, compression, verification bookkeeping, recovery, routing, cache identity, scan reuse, and evaluation scoring remain deterministic. The program must not add model calls merely to summarize, rank, verify, or route information that can be handled deterministically.

## 3. Reversible context reduction

Whenever DaVinci reduces model-visible evidence, the exact original must remain recoverable when technically possible. Governor compression, context pruning, and deferred schemas must never silently destroy required evidence.

## 4. Verification is evidence, not a stop reason

A turn is considered verified only when the verification evidence covers the latest mutation scope. A normal model stop, successful unrelated command, or stale prior test result is not sufficient.

## 5. Optimization must be benchmarked

Every optimization that can affect model-visible context or task behavior must expose a deterministic A/B path and must be rejected if it materially lowers verified task success for a cost saving.

---

# Program A — Capability Toolbox v1 Quality

## A1. Node-specific retrieval query

Current Graph context selection can use the overall Graph goal as the retrieval query. Replace this with a deterministic worker query assembled from:

1. worker role;
2. current node/task objective or briefing;
3. known target files/symbols from typed artifacts when available;
4. compact Graph goal context;
5. failure classification on a retry, when the information need changed.

The query must remain deterministic and bounded. It must not include full prior transcripts.

### Invariant

Two workers with different node objectives must be able to retrieve different memory/skill candidates even when they belong to the same Graph goal.

## A2. Context utility calibration

Do not directly compare uncalibrated memory and skill scores as though they share identical semantics. Introduce a deterministic context-utility calculation based on:

- relevance;
- confidence/verification quality;
- applicability/freshness;
- estimated token cost.

Use the utility only for final packet selection under the existing overall Graph context cap. Preserve complete skill bodies; never partially inject a skill.

## A3. Cached semantic skill ranking

If a compatible skill embedding already exists locally, allow skill ranking to combine lexical and semantic similarity. If no embedding is available, fall back immediately to lexical ranking; do not trigger a new remote/network dependency from Graph context assembly.

## A4. Skill applicability metadata

Extend learned skill metadata with optional deterministic applicability hints such as:

- languages;
- task types;
- relevant path globs;
- required project signals;
- verification categories.

Old skills without these fields remain valid. Applicability narrows or boosts ranking; it must not create permission authority.

## A5. Outcome credit quality

Distinguish at least:

- skill selected/injected;
- skill actually surfaced to the worker;
- verified task success/failure for the exact version;
- whether the skill was relevant to the affected scope when that can be determined deterministically.

A skill must not receive strong positive credit merely because it was injected into a successful task.

---

# Program B — Token Governor vNext, Without a New Governor

## B1. Reversible compression for large error outputs

Current large successful outputs may be compressed, while error outputs pass through. Change the behavior for compressible tools so a sufficiently large error result may use the same reversible path:

1. save the exact original to `OutputStore` first;
2. preserve `is_error = true`;
3. build generic and specialized views;
4. return the smaller view only when it clears the configured benefit threshold;
5. include `retrieve_output` recovery instructions in model-visible text.

Small errors and `LosslessRequired` tools remain verbatim.

## B2. Additional deterministic content shapes

Add specialized handling only for shapes that can be recognized without a model:

- compiler diagnostics;
- test-suite output;
- newline-delimited JSON;
- large JSON objects containing significant arrays/diagnostic fields;
- directory/tree output;
- simple tabular output.

Each router must fail open to the generic reversible Governor path.

## B3. Minimum specialized-view benefit

A specialized view should not replace the generic reversible view merely because it is one byte smaller. Add a configurable/default deterministic minimum win, using percentage and/or absolute-byte reduction. Initial engineering default should be conservative (for example, about 10%). Benchmark before changing the default later.

## B4. Per-kind retrieval telemetry

Track, at minimum:

- compressed outputs by content kind;
- specialized/generic choice by kind;
- original bytes and model-view bytes by kind;
- later `retrieve_output` calls attributable to the stored output/kind;
- retrieval within a small turn window when that association is available deterministically.

This telemetry is observational; it must not silently self-modify routing thresholds.

## B5. Structured view quality

For JSON-like data, retain diverse high-signal records rather than only fixed positional samples. Prefer deterministic identity/signal keys such as `id`, `name`, `path`, `file`, `line`, `status`, `type`, `error`, and `message`, while keeping the exact original recoverable.

---

# Program C — Root Context Becomes Enforced Budgeting

## C1. Move from reporting to shaping

`RootContextAccount` currently reports which contributions would fit a budget while the normal prompt can remain unchanged. Wire the selected/deferred decision into the actual provider request for deferable sources.

Never drop:

- system authority;
- permission/security policy;
- active task/plan/contract authority;
- authoritative project instructions required for the target scope.

Defer before mandatory content:

- low-confidence memory;
- old/recoverable tool evidence;
- inactive skill/reference bodies;
- inactive tool schemas;
- exact duplicate content;
- other explicitly deferred sources.

## C2. Scoped project instructions

Support path-scoped project instruction discovery without model summarization. The closest applicable authoritative instruction files for affected paths should be included along with root authority. Exact duplicate bodies remain deduplicated with provenance retained.

The implementation must preserve current root `AGENTS.md`/`CLAUDE.md` behavior for repositories that do not use scoped instruction files.

---

# Program D — Scope-aware Verification

## D1. Typed verification evidence

Extend verification evidence so the latest mutation generation can record:

- command/verifier kind;
- exit code/result;
- affected mutation paths/targets;
- verification targets inferred deterministically;
- coverage class/confidence.

The system must distinguish a broadly relevant or targeted verifier from an unrelated successful command.

## D2. Completion rules

A mutation may be classified as verified only if successful verification evidence covers the current mutation generation and is relevant to the mutation scope. A later mutation invalidates prior coverage as today.

Read-only work remains `NotRequired`.

No LLM judge is introduced.

---

# Program E — Recovery and Capability Metadata Hardening

## E1. Atomic durable ledger writes

Persist `ToolCallLedger` using a same-directory temporary file, file sync, atomic rename, and parent-directory sync where supported. Keep the last known good state or equivalent recovery evidence when practical.

If the durable ledger is corrupt after restart, fail safely: uncertain mutation-capable actions must not be assumed unstarted.

## E2. Remove remaining side-effect classification duplication

Consumers should derive mutating/read-only and replay decisions from `RuntimeCapability` metadata wherever the runtime registry is available. Eliminate hard-coded read-only/mutation lists in the tool ledger when they duplicate registry authority.

Unknown capabilities remain conservative: mutating, serial, and never auto-replayable.

---

# Program F — Memory Freshness and Compatibility

## F1. Freshness metadata

Add optional memory provenance sufficient to detect obvious staleness, such as relevant path/symbol references, source content/revision fingerprints, or verified-at state identifiers.

## F2. Retrieval scoring

Blend existing relevance with deterministic confidence/freshness/compatibility. Old records without freshness metadata remain usable but must not receive invented confidence.

A stale architecture/fact memory should be penalized when its relevant source state has materially changed.

---

# Program G — Graph Efficiency

## G1. Failure-type-aware retry policy

Classify retry causes deterministically into categories such as:

- timeout;
- invalid/missing artifact;
- verification failure;
- permission/policy refusal;
- environment/configuration failure;
- plan invalidation.

Retry behavior should then be specific to the cause. Environment/configuration failures must not burn repeated model attempts when retry cannot fix the environment.

## G2. Failure-specific context delta

Keep the stable/base Capability Toolbox packet for cache affinity. When a retry failure meaningfully changes the information need, allow a small bounded context delta derived from the failure class and diagnostics. Do not rebuild the entire packet for every retry.

## G3. Graph deferred schemas

Reuse `ToolExposureState` and `RuntimeCapabilityRegistry` for Graph workers so role authorization and provider schema exposure remain separate. Graph workers should begin with a minimal useful schema set and activate authorized deferred tools through the existing discovery path.

`retrieve_output` must remain available whenever an exposed/authorized tool can generate Governor-compressed output.

## G4. Provider cache effectiveness telemetry

Report real provider cache-read/cache-write usage by Graph role and, where possible, correlate cache misses with existing reason-coded identity changes. Local hashes alone must not be labeled as provider cache hits.

---

# Program H — MCP Progressive Disclosure

Large MCP catalogs should register capability metadata without forcing every schema into the initial provider request.

Flow:

1. connect/register MCP capability names, descriptions, schema hashes, sources, and permissions;
2. keep most schemas deferred;
3. discover through existing `tool_search`;
4. activate only authorized schemas;
5. execute through the normal permission/runtime path.

No separate MCP discovery registry is introduced.

---

# Program I — Lazy Semantic Navigation

Finish `NativeSemanticService` using lazy per-workspace sessions keyed by canonical workspace/configuration identity.

Requirements:

- no LSP startup at application launch;
- start only after a semantic capability is requested;
- deterministic timeout/health handling;
- fall back to grep/compiler/text navigation when the service is unavailable;
- preserve permissions for any project-adjacent executable language server;
- benchmark by language rather than enabling universally based on assumption.

---

# Program J — Incremental Security Scan

Introduce deterministic scan reuse keyed by at least:

- file content hash;
- scanner/rule-set version;
- relevant scan configuration.

Unchanged files may reuse prior deterministic findings/candidates. Changed files are rescanned. Every completed report remains newly generated and newly sealed; cached file-level evidence must not make the final report mutable or weaken existing lifecycle guarantees.

---

# Program K — Evaluation and Benchmarking

## K1. Internal deterministic A/B

Every behavior-changing optimization must have an ablation or deterministic fixture path. Primary decision metric:

`cost per verified success = total attributable cost / independently verified successful tasks`

Also record verified success rate, first-attempt success, provider tokens/cache usage when available, model turns, tool calls, retries, subagents/workers, wall time, scope violations, permission failures, and false success claims.

## K2. Provider-backed runs

Run provider-backed A/B only when valid credentials/configuration are available. Infrastructure/config failures are evidence and must not be reported as task failures or fabricated benchmark results.

## K3. Competitor adapters

Complete matched evidence paths for:

- DaVinci;
- OpenAI Codex;
- Claude Code;
- Hermes Agent;
- OpenCode.

Keep two reports separate:

1. **Harness comparison:** same model/configuration where technically possible.
2. **Product comparison:** each product in its intended/default configuration.

No combined leaderboard claim may mix these modes.

---

# Program L — Repository Safety Policy

The desired repository policy is:

- require green CI quality checks;
- require workflow lint;
- require Security SARIF interoperability;
- prevent force-push/deletion of `main`.

This is a GitHub repository-administration setting, not a DaVinci runtime feature. Code/workflow changes may be committed on this branch, but branch-protection mutation is performed only if the available GitHub connection exposes an authorized administration action. Otherwise the final report must state that the repository setting remains external/manual rather than pretending it was changed.

---

# Staging and Commit Boundaries

Implementation should stay on this single integration branch but use independent subsystem commits:

1. Capability Toolbox query + context utility.
2. Skill applicability/semantic ranking/outcome credit.
3. Governor reversible error compression + new routers.
4. Governor minimum-win + per-kind retrieval telemetry.
5. Root context enforcement + scoped instructions.
6. Scope-aware verification.
7. Atomic tool-ledger durability + registry-authoritative side effects.
8. Memory freshness/compatibility.
9. Graph failure-aware retries + retry context delta.
10. Graph/MCP deferred schema exposure.
11. Lazy semantic navigation.
12. Incremental security scanning.
13. Evaluation adapters/ablations/reporting.
14. Documentation and final integration verification.

If inspection shows a capability already satisfies an invariant, skip that implementation rather than recreating it.

---

# Test Discipline

For each behavioral change:

1. write the smallest regression test that proves the missing invariant;
2. run that test and confirm the expected failure;
3. implement the smallest production change;
4. rerun the targeted test;
5. run only the directly related module/crate tests;
6. run crate-level `cargo check`/Clippy at subsystem boundaries;
7. commit the independently correct stage.

Do not run `cargo test --workspace` after every small edit.

At major integration points and before PR completion, run the repository's canonical full gates once on the exact branch head:

- `cargo fmt --all -- --check`;
- `cargo clippy --workspace --all-targets -- -D warnings`;
- repository behavioral/ecosystem gates;
- `cargo test --workspace`;
- workflow lint/actionlint;
- Security SARIF interoperability;
- native voice matrix if the standard CI runs it.

No live provider/network tests are required for code correctness. Provider-backed benchmarks are a separate configured evaluation phase.

---

# Acceptance Criteria

## Correctness

- No regression in current permission, Graph-role, replay, Governor recovery, or prompt compatibility contracts.
- Large error compression remains losslessly recoverable and preserves error status.
- Unrelated passing tests cannot mark a mutation as fully verified.
- Corrupt/uncertain recovery evidence never causes automatic mutation replay.
- Deferred schema activation never elevates permission.

## Efficiency

- Capability Toolbox node-specific retrieval improves relevant-candidate selection in deterministic fixtures without increasing its existing aggregate context cap.
- Specialized Governor views must meet a minimum benefit threshold and expose per-kind retrieval feedback.
- Root request budgeting demonstrably removes/defer optional content while mandatory authority remains present.
- Graph/MCP deferred schemas reduce initial provider schema bytes on large synthetic catalogs while required tools remain discoverable.

## Learning/memory

- Exact skill-version attribution remains intact.
- Stale memory can be deterministically penalized when its referenced source state changes.
- Skill success credit is not awarded solely for being injected.

## Evaluation

- Internal A/B paths are deterministic and offline-capable.
- Real provider metrics are reported only when obtained from real configured runs.
- Harness-vs-product competitor comparisons remain separate.

## Maintainability

- No duplicate authority registry or parallel budget/memory/orchestration subsystem is introduced.
- New metadata fields are backward-compatible with existing persisted data where applicable.
- Unknown capabilities continue to fail conservatively.
