# DaVinci OpenAI-Only Production Harness Implementation Plan

> **For agentic workers:** After separate authorization to implement, use `superpowers:executing-plans`, or `superpowers:subagent-driven-development` when the user explicitly selects that execution method. Execute one task and its verification cycle at a time. Steps use checkbox syntax for tracking. This document does not authorize implementation, paid calls, staging, commits, pushes, installations, or deployment.

**Goal:** Increase independently verified, production-ready outcomes per unit of OpenAI spend and elapsed time, while reducing unnecessary model requests, newly processed input, repeated tool work, and integration failures.

**Architecture:** Improve the existing Rust agent loop, capability registry, OpenAI request preparation, context/evidence stores, scheduler, worker runtime, and evaluation tooling. Keep authority in deterministic runtime code, judgment in the model, and acceptance in independently checkable evidence. Do not add a replacement orchestration framework or a general multi-provider optimization layer.

**Tech stack:** Existing Rust workspace and Rust 1.83.0 toolchain; exact dependency pins; current JSONL/SQLite persistence, native tools, Python benchmark tooling, and JavaScript fixtures. Reuse existing dependencies. A new dependency requires a demonstrated gap and a separate compatibility decision.

**Spec:** The user's requested production-harness direction and subsequent OpenAI-only clarification, formalized in Sections 1–6 below. Read `docs/readiness/production-harness-readiness-plan.md` for historical rationale, `docs/readiness/README.md` for reported implementation/acceptance status, and `docs/cache/openai-cache-contract.md` for existing invariants. This plan supersedes earlier recommendations to optimize Anthropic or run same-Claude-model campaigns during the present scope.

**Date:** October 1, 2026. **Workspace:** `C:\Users\sergi\Desktop\davinci-main`. **Status:** planning deliverable; implementation not started. **Version-control instruction:** do not commit this plan. No version-control mutation is part of this delivery.

## Global constraints

- OpenAI models only for new inference, routing experiments, cache optimization, and live acceptance. Preserve other providers' existing compatibility through offline regressions; do not optimize, remove, or activate them.
- Distinguish public OpenAI Responses, supported earlier OpenAI routes, and ChatGPT-backed Codex. A shared model name does not establish a shared API contract.
- This is a plan, not implementation approval. No source changes, dependencies, paid requests, credential changes, live worker launches, or account/resource changes are authorized by authoring it.
- Do not stage, commit, push, initialize Git, create branches, or replace installed binaries while delivering the plan. A subsequent implementation authorization does not implicitly authorize committing this document.
- Tests written during implementation must be deterministic and fixture/loopback-based by default. External provider campaigns require an explicit model, route, request limit, finite spend limit, and authorization identifier.
- Preserve fail-closed authorization, user restrictions, branch/worker ownership, exact recoverable evidence, and acknowledged tool results. Optimization cannot make a prohibited operation legal.
- Keep the repository-pinned toolchain and dependency conventions. Preserve TypeScript parity contracts unless a divergence is explicitly approved; do not silently reword parity compaction prompts.
- No feature is promoted from offline correctness alone. Separate implementation, offline validation, live efficacy, and release acceptance.
- Preserve user settings, credentials, sessions, unrelated edits, and files from other sessions. Never delete them to make a test or benchmark pass.
- Report missing usage, unknown side effects, incomplete verification, unsupported isolation, and unavailable diagnostics as unknown or unsupported, not success or zero.
- Prefer extending current structures to creating parallel state stores. Each durable fact has one authoritative owner and explicit derived projections.
- Model output, retrieved pages, repository text, plugins, and worker reports cannot grant permissions or certify their own correctness.

## Review focus

Five cross-cutting cases need explicit coverage rather than an assumption that the happy path generalizes:

1. A permission or delegation restriction changes during queued work: deny at dispatch and on redirect, resume, child launch, and cache reuse. Owned by S01, S02, C04, and A02.
2. The server accepted work but the response or process died: retain uncertain outcomes and prevent duplicate mutation/replay. Owned by S05, C06, and A03.
3. Two processes modify shared state or a file changes while preserving its timestamp and size: prevent lost updates and stale acceptance. Owned by S03, Q02, and X02.
4. A model returns plausible but incomplete output after tests pass: requirement coverage and final-artifact identity remain mandatory. Owned by Q01, Q02, and P01.
5. An optimization looks efficient only because work failed, usage was missing, or cache was warmed by another arm: retain denominators and isolate treatment histories. Owned by M01, E01, E02, and R01.

## Contents

1. Outcome and scope
2. Evidence baseline and corrections
3. Architecture and contract ownership
4. OpenAI cache design
5. Metrics, economics, and decision gates
6. Delivery sequence and execution rules
7. S: trust, state integrity, and cancellation
8. M: measurement and budget ownership
9. Q: completion correctness and bounded repair
10. C: OpenAI cache and transport engineering
11. X: context and evidence efficiency
12. T: tool selection and scheduling
13. A: agent and subagent orchestration
14. P: production acceptance
15. E: evaluation and controlled experiments
16. R: rollout and simplification
17. Verification command reference
18. Requirement coverage and final acceptance
19. Sources and evidence policy

---

## 1. Outcome and scope

The optimization order is: preserve authority and user data; improve task correctness; reduce total cost per verified success; reduce end-to-end latency; then tune component metrics such as cache ratio and tool counts. No single percentage may hide a regression in a preceding objective.

“Production-ready” means that the requested behavior has evidence appropriate to its risk, the integrated artifact is identified, relevant user journeys and failure paths were exercised, limitations are explicit, and the release/rollback procedure is known. It is not a promise that no defect can ever exist. Tasks that were only implemented or locally tested must not be labeled operationally verified.

**In scope:** OpenAI request shaping and caching, tool efficiency, context recovery, deterministic budgets and orchestration, selective OpenAI worker routing, task-complete verification, safe state transitions, long-session recovery, product-level acceptance, and a reproducible Codex comparison.

**Out of scope:** Anthropic cache tuning; new provider integrations; an OpenAI Agents API migration; another agent framework; always-on model reviewers; automatic global learning promotion; autonomous production deployment; new voice functionality; and an unsupported claim of being universally better than every competitor.

Codex is the primary live comparator under this scope. Claude Code remains a product/architecture reference. A live Claude Code comparison requiring another inference provider is deferred. Be explicit that beating Codex does not establish a measured Claude Code advantage.

## 2. Evidence baseline and corrections

Treat evidence by class: **current source inspection**, **repository-reported historical result**, **current official API documentation**, **proposed design**, or **future acceptance evidence**. Do not merge those classes.

| Evidence | What it establishes | What it does not establish |
|---|---|---|
| Earlier local source review | Specific call paths and plausible failures in URL permissions, delegation parsing, plugin persistence/update, stream cleanup, and session listing | Live reproduction or a verified fix for those findings |
| Existing readiness documents [L1–L2] | Completion reminders, hooks, recovery, cost controls, and evaluation infrastructure have reported implementations | Current integrated-tree correctness or superior live task performance |
| Historical 12-task × 3-repetition comparison [L1] | Reported equal composite success of 22/36 and 0/12 large-task successes, with lower DaVinci latency and input use in that campaign | A current baseline or a general product advantage |
| OpenAI cache contract and source [L3–L5] | Existing policy, wire manifest, native replay, and usage structures to extend | A provider cache hit merely because a local fingerprint matches |
| Local Codex probe record dated September 26 [L4] | Recorded rejection of several request shapes on one tested backend/model combination | Support on all current or future models, or complete behavioral validation of accepted shapes |
| Official OpenAI references checked October 1 [O1–O7] | Publicly documented behavior for the routes and models those documents describe | Acceptance of the same fields on the ChatGPT-backed adapter |

**Important correction to the earlier discussion:** `docs/cache/codex-backend-probe.md` is more specific than older “probe not run” wording elsewhere. It reports an authenticated September 26 probe: cache options, explicit breakpoint shape, prewarm, and `additional_tools` received HTTP 400; remote compaction received HTTP 404. `allowed_tools`, custom grammar, assistant phase, and `tool_choice_none` were accepted, but reported cache counts were zero. Preserve those negative capability results until a new, authorized, scoped probe supersedes them. Acceptance of a request does not prove its intended semantics or savings. [L4]

The source directory had no `.git` metadata when checked for this plan. Do not initialize Git to hide that limitation. Local development can identify an immutable exported snapshot by a file manifest, but a release/superiority campaign still needs a verifiable source revision and executable identity supplied through an authorized workflow.

No tests, live benchmarks, or provider probes are represented as newly run by this planning document. Earlier execution of pre-existing test binaries is not a fresh build of current source.

## 3. Architecture and contract ownership

### 3.1 Extend the existing implementation

| Boundary | Existing owners | Required improvement |
|---|---|---|
| Permission and policy | `crates/davinci-agent/src/permission.rs`, `delegation.rs`, `turn.rs` | One interpretation; revision-aware enforcement at actual dispatch |
| OpenAI capabilities and wire shaping | `crates/davinci-ai/src/openai_cache_policy.rs`, `responses_request.rs`, `responses_tools.rs`, `stream.rs` | One capability-gated request plan; stable ordered prefixes; supported tool exposure |
| Replay and transport | `crates/davinci-ai/src/responses_ledger.rs`, `codex_transport.rs`, `stream_reader.rs`, `provider_retry.rs` | Exact native replay, owner-bound continuation, cancellation, uncertainty-aware recovery |
| Context and local evidence | `crates/davinci-agent/src/prepared_context.rs`, `provider_budget.rs`, `runtime/cache/`, `runtime/context_vm/` | Relevant working set, trustworthy invalidation, recoverable omitted evidence |
| Tool execution | `crates/davinci-agent/src/tools.rs`, `batch.rs`, `scheduler.rs`, `tool_ledger.rs` | Fewer model round trips without removing barriers or visibility |
| Workers and workflow | `crates/davinci-agent/src/subagent.rs`, `runtime/team.rs`, `runtime/workflow/`; `crates/davinci-coding-agent/src/native_extensions/graph/` | Bounded task contracts, one root budget, scoped writers, integration verification |
| Completion and receipts | `crates/davinci-agent/src/completion.rs`, `command_receipt.rs`, `verification.rs`, `living_plan.rs` | Requirements supported by fresh evidence, not model assertions |
| Measurement and campaigns | `crates/davinci-agent/src/stats.rs`, `crates/davinci-evals/src/openai_cache_eval.rs`, `scripts/bench/` | Complete usage, all-attempt economics, independent acceptance, uncertainty intervals |

Existing symbols inspected include `OpenAiCacheCapabilities`, `PromptCacheWirePlan`, `EffectiveOpenAiCachePolicy`, `CacheIntent`, `PreparedProviderRequest`, `WireManifest`, `NativeResponsesResumeRecord`, `ResponsesLedger`, `ProviderContextBudget`, `RequestCapacity`, and `WorkerSlotCapacity`. Extend these owners where applicable; do not recreate them under new names. [L5]

### 3.2 Cross-task contract definitions

The following are **planned logical records**, not claims that matching Rust structs already exist. Each task names the existing owner into which its record should be mapped. Freeze field meanings before independent implementations begin; use the same meanings across CLI, RPC, worker, and benchmark paths.

| Contract | Required fields and semantics | Producer → consumers |
|---|---|---|
| Task identity | Root task ID; session/branch lineage; prompt revision; source snapshot; executable identity | M00 → every measurement, receipt, worker, and campaign |
| Attempt receipt | Unique attempt ID; parent/root IDs; exact requested/returned model and tier when exposed; route; timing; terminal outcome; usage completeness; provider response reference | M01 → M02, M03, C05, E02 |
| Budget reservation | Root and child ownership; resource amounts; deadline; pending/committed/released state; idempotent reconciliation reference | M02 → A01–A04, C06 |
| Capability evidence | Backend/auth route; endpoint trust; exact model; feature; accepted/rejected/unknown; tested semantics; evidence date; contract revision | C01 → C02–C07, T01 |
| Prepared context identity | Policy revision; authorized tool surface; source/dependency revisions; immutable ordered provider representation; output budget | C02/X01 → replay, request dispatch, cache diagnosis |
| Requirement evidence | Requirement ID; original source clause; assertion/test intent; implementation scope; receipt/artifact refs; covered/failed/unverified/waived-by-user state | Q01/Q02 → Q03, P01, P03 |
| Worker contract | Task/parent IDs; pinned base; goal and non-goals; scope; allowed tools; budget; deadline; required output; evidence refs | A01/A02 → worker, integration controller |
| Acceptance record | Artifact/source identity; acceptance level; check results; known limitations; approver authority; rollback reference | P01–P03 → final delivery, E02, R01 |

Never use a cache partition as a session owner, an LLM summary as a command receipt, or a worker's self-grade as the root acceptance result.

## 4. OpenAI cache design

### 4.1 Route-specific capability matrix

Use authenticated endpoint identity and existing capability resolution. Never infer support from a substring such as “gpt,” an OpenAI-looking hostname, or a user-controlled compatibility flag alone.

| Route | Baseline policy | Candidate optimization | Fallback |
|---|---|---|---|
| Verified public Responses with modern cache capability | Existing supported policy | Measured append-only or stable-prefix-only policy, diagnostic sampling, documented dynamic configuration/tool updates | Last verified public profile |
| Earlier supported OpenAI route | Existing legacy behavior | Stable prefix, deterministic key where useful, supported retention only | Omit unsupported fields |
| ChatGPT-backed Codex | Existing accepted transport and request shape | Stable instructions/history/schema, native replay, scoped affinity, only semantically verified optional controls | Accepted baseline plus full owner-valid replay |
| Unknown custom/proxy or unsupported route | Preserve existing compatibility without new tuning | None in this program | Conservative shape; block when a required privacy property cannot be established |

Public documentation currently differentiates model generations. Its modern profile supports explicit content-block boundaries, a 1,024-visible-token minimum, at most four cache writes per request, and a `30m` minimum-lifetime setting; older profiles use different retention and boundary behavior. Keys have different purposes across these generations. Store these as dated capability metadata, not universal constants. Do not copy this profile into Codex OAuth. [O1]

### 4.2 Prompt layout and immutable epochs

Define an epoch as a period during which the model, authorized leading tool surface, stable instructions, output contract, and relevant settings are compatible. Keep request IDs, timestamps, worker instance IDs, progress counters, and volatile workspace status out of reusable model-visible prefix material.

Place enduring harness instructions first, stable role instructions next where appropriate, then the task/conversation and changing evidence. Append a new state message instead of editing previously sent text. Preserve the authority of a state item: user instructions stay user instructions; untrusted content never becomes developer policy.

Role-specific worker prefixes may share useful structure, but do not pad every role with unrelated tools or instructions to manufacture a higher hit ratio. Do not cache incomplete tool exchanges as resumable conversation boundaries.

Permission revocation is immediate even when it changes a schema or epoch. Dispatch authority always comes from current policy. A restriction mechanism that leaves definitions visible is acceptable only when the retained schema itself is not sensitive and the backend behavior is verified; otherwise rebuild the surface and accept the cache miss.

### 4.3 Workload policies

Use `CacheIntent` and the existing wire-plan owner rather than a second cache planner:

- **Append-only work:** repeated turns in one coding task. Preserve eligible stable and historical boundaries where supported.
- **Stable-prefix-only work:** repeated role/rubric/bootstrap with different one-shot task payloads. Consider a boundary before the varying suffix; do not pay to write a suffix that will never recur unless it helps completion latency and the tradeoff is measured.
- **Disabled by explicit policy:** use only a verified disable contract. Omitting a key does not by itself establish provider retention/privacy guarantees.
- **Unsupported or unknown route:** use the accepted adapter baseline. A request failure cannot justify progressively removing unrelated safety controls.

Supported public configuration updates may preserve an earlier prefix when reasoning effort changes; late tool definitions have their own ordered input semantics. These are candidate route capabilities, not unconditional instructions to emit new fields. [O1, O3]

### 4.4 Cache economics

Measure ordinary input U, cache reads R, cache writes W, and output O as disjoint billed buckets when available. A local cache hit, reused socket, or continuation ID is not provider cache reuse.

For a reusable L-token prefix, k future full reuses, and per-token prices pU/pR/pW, an illustrative decision estimate is:

**Expected saving = k × L × (pU − pR) − L × (pW − pU).**

This is a planning estimate, not an observed saving. Actual hit probability, prefix changes, writes, retention, latency, task quality, and partial reuse must be included in campaign results. Do not assume k or hit probability without evidence. Do not make a model call just to decide whether to cache each request.

Prewarming remains off by default. A conditional public-route experiment must include startup delay, canceled sessions, unused warming, and billed writes. No warmup spam, periodic keepalives, or artificial padding. Codex prewarming remains disabled under the recorded negative probe.

### 4.5 Diagnostics and replay

Use provider diagnostics only where supported, with local segment differences as a separate fallback. A diagnostic “hit” is not the billed reused-token count; use usage fields. Unknown, expired comparison, and unavailable diagnostic results remain unknown. [O2]

Preserve native provider items exactly, including opaque items, or reject their reuse. Unknown data must not be silently dropped or exposed as human-readable reasoning. Owner-valid full replay is the correctness fallback when continuation is unavailable; a cache identity never authorizes loading another conversation. [L3, L5]

## 5. Metrics, economics, and decision gates

### 5.1 Required measurements

**Primary:** all-attempt cost per verified success = cost of every run, worker, retry, compaction, and enabled background job divided by independent successful outcomes. Zero successes means undefined/no successful delivery, not zero cost.

**Quality:** success by task class; regression-free success; unrelated edits; requirement coverage; unsupported completion claims; cancellation cleanup; permission violations; recovery failures; integration failures.

**Efficiency:** aggregate U/R/W/O; newly processed input U+W; provider requests; expanded tool operations; model-visible round trips; repeated reads/searches; output-retrieval overhead; compaction overhead; worker duplication; CPU/memory and artifact storage overhead.

**Latency:** end-to-end task duration, time to first useful response, provider generation, preparation, queueing, execution, verification, integration, and retries. Measure median and tail distributions. Avoid reliable-looking p95 claims from tiny samples.

**Accounting quality:** exact/estimated/unknown usage and price coverage; requested versus returned model/tier; quota/rate-limit observations; outstanding reservations; background attribution. A subscription credit estimate is not an invoice. Pricing is pinned per campaign and never guessed from a familiar model name.

Aggregate cache-read ratio is sum(R)/sum(U+R+W), with a zero denominator reported as unavailable. Missing W must not be silently converted to measured zero when the route can charge writes but omits the field. Reasoning tokens must not be double-counted when already included in O.

### 5.2 Proposed targets, not claimed results

| Objective | Proposed program target | Promotion requirement |
|---|---|---|
| Success | At least +3 percentage points versus matched Codex | Positive paired confidence evidence, sufficient sample, no hidden regressions |
| All-attempt cost per success | At least 25% lower | Complete usage and explicit estimated-price labeling |
| Newly processed input per success | At least 25% lower | U+W accounted across all attempts |
| Provider requests | At least 20% fewer on mutually successful task pairs | No omitted nested/background requests |
| Median end-to-end latency | At least 20% lower | No meaningful tail regression or quality sacrifice |
| Cache effectiveness | Higher reuse of genuinely reusable eligible content | Lower total task cost or latency; ratio alone cannot promote |
| Safety and data integrity | Zero observed violations in acceptance cases | Any observed violation blocks promotion; zero observations are not a proof of universal absence |

Each experiment predeclares its target, non-inferiority margins, population, minimum sample, and stopping rule before outcomes are inspected. Small development experiments select candidates; they do not justify public superiority claims. A quality improvement may consume more tokens when it improves total economics; disclose that tradeoff rather than forcing every metric to improve at once.

## 6. Delivery sequence and execution rules

| Gate | Tasks | Deliverable |
|---|---|---|
| G0 | M00, S01–S05 | Identified source/build and reproduced, closed trust/state gaps |
| G1 | M01–M03, E01 | Reconciled measurement, budget authority, uncontaminated task suite |
| G2 | Q01–Q03 | Requirement-complete execution and bounded targeted repair |
| G3 | C01–C07, X01–X03 | OpenAI-only cache/context candidate with recoverable evidence |
| G4 | T01–T03 | Lower round-trip and scheduling overhead without weaker authority |
| G5 | A01–A04 | Selective, budgeted, integration-aware delegation |
| G6 | P01–P03, E02–E03 | Product acceptance and independently graded campaigns |
| G7 | R01–R02 | Measured defaults, reversible rollout, completed acceptance package |

Dependency graph: M00 precedes source claims; S and M may proceed independently after identity is fixed; E01 starts with M01; Q depends on trustworthy receipts; C and X depend on measurement and policy boundaries; T uses C's tool-exposure contract; A uses M02, Q, T, and X; P uses Q/S/A integration invariants; E02 measures candidates only after their offline gates; R follows evaluation. C07 is optional and never blocks the supported Codex baseline.

For every task: inspect the named existing path and nearby tests; add the specified regression first; run it and retain the expected failure; implement the smallest change; run the focused and boundary checks; capture evidence; obtain review proportional to risk. A task that already behaves correctly needs a regression/evidence update, not a duplicate implementation. Source locations can move; follow the named symbol and document the relocation.

Use the command labels in Section 17. New integration test filenames and regression names below are **proposed**, not existing passing tests. Every final verification must run from freshly built source. No task ends with an instruction to commit.

Evidence for future execution belongs outside the repository under an operator-chosen run directory, with a manifest, before/after identities, sanitized logs, test outcomes, and disposition. Do not publish sensitive traces. No such execution evidence is fabricated by this plan.

---

## 7. Workstream S: trust, state integrity, and cancellation

### S01 — Canonical URL authorization across every redirect

**Depends on:** M00. **Modify:** `crates/davinci-agent/src/permission.rs`, `crates/davinci-agent/src/web.rs`, `crates/davinci-agent/src/tools.rs`. **Tests:** `crates/davinci-agent/src/permission_tests.rs` and web inline tests; add `crates/davinci-agent/tests/harness_web_authority.rs` for the boundary fixture.

**Contract:** one parsed destination supplies the permission subject, approval display, and network target. Each hop is checked against current permissions and SSRF rules before connection. Preserve existing host-rule semantics rather than silently making port-specific grants broader.

- [ ] Add `harness_url_parser_and_permission_subject_agree`: cover backslashes, user information, encoded host text, mixed case, IPv4/IPv6, and malformed authorities; a denied parsed host cannot be authorized by a different displayed host.
- [ ] Add `harness_redirect_rechecks_current_permission`: an initially permitted destination redirects to a denied public-shaped destination; the second transport is never invoked. Use fixture injection, not an external DNS/network request.
- [ ] Add `harness_redirect_uses_final_document_base`: a redirect changing both domain and directory resolves relative document links against the final URL, without widening fetch permission.
- [ ] Run the new assertions against the current implementation and save the outcomes. Implement shared parsing plus per-hop policy invocation only where the regression demonstrates a gap; retain DNS/address rechecks.
- [ ] Run V-A and the dedicated integration target. Review permission revocation between hops, bounded redirect loops, cancellation while awaiting approval, and absence of credentials in error text.

**Accept when:** the actual contacted destination is the authorized one on every path. **Evidence/rollback:** parser cases and a transport-invocation ledger; revert the candidate implementation if necessary, never bypass a denied hop to recover functionality.

### S02 — Durable, non-ambiguous delegation restrictions

**Depends on:** M00. **Modify:** `crates/davinci-agent/src/delegation.rs`, `crates/davinci-agent/src/lib.rs`, `crates/davinci-agent/src/turn.rs`, `crates/davinci-agent/src/permission_state.rs`. **Tests:** existing delegation/turn tests and new `crates/davinci-agent/tests/harness_delegation_authority.rs`.

**Contract:** user-origin policy events may narrow delegation immediately. An existing prohibition can be lifted only by an unambiguous authorized user action. A model interpretation, quotation, worker report, or retrieved document cannot lift it.

- [ ] Add `harness_negated_allow_never_revokes_delegation_ban`: test “not allowed to use subagents,” contractions, typographic apostrophes, quoted permission examples, and conditional wording; preserve a previous prohibition.
- [ ] Add `harness_delegation_policy_survives_resume_and_compaction`: queued and resumed `agent`, workflow, and graph launches all see the latest restriction.
- [ ] Add `harness_explicit_delegation_reversal_is_user_owned`: a direct structured user reversal works; a tool/assistant event with identical text does not. Ambiguous natural-language reversal leaves the ban intact and explains the supported explicit action.
- [ ] Reproduce the previous parser failure; replace revoking substring behavior with the narrow policy transition contract. Do not claim perfect natural-language understanding.
- [ ] Run V-A and the new integration target. Review mixed directives, batches, aliases, worker ceilings, and a policy change after admission but before launch.

**Accept when:** uncertain wording never expands authority and all child launch paths use the same policy revision. **Evidence/rollback:** event-origin and transition fixtures; a rollback must preserve persisted restrictions.

### S03 — Transactional plugin registry and marketplace state

**Depends on:** M00. **Modify:** `crates/davinci-coding-agent/src/plugins/store.rs`, `crates/davinci-coding-agent/src/plugins/marketplace.rs`. **Tests:** inline store/marketplace tests and new `crates/davinci-coding-agent/tests/harness_plugin_state.rs`.

**Contract:** distinguish absent, readable, corrupt, and inaccessible state. Serialize the entire read–modify–publish operation across processes. Use a unique staged file, durable publication, and a typed conflict/error without losing the previous state.

- [ ] Add `harness_corrupt_plugin_registry_is_preserved`: malformed existing JSON cannot be replaced with a newly initialized store; missing files alone may initialize an empty store.
- [ ] Add `harness_concurrent_plugin_changes_both_survive`: coordinate two real fixture processes changing different records; both changes remain. Add same-process concurrent temporary-file coverage.
- [ ] Add `harness_plugin_publish_failure_preserves_last_good_state`: inject write, flush, publication, and lock-timeout failures; preserve the original and return an error, not success.
- [ ] Reproduce before modification, then reuse an existing cross-process locking/durable-write primitive where compatible. Bind mutation to the version read; never reuse a process-ID-only temporary path.
- [ ] Run V-C and the integration target on Windows plus supported Unix CI. Review bounded lock waits, cleanup of owned temporary files, and release/retry after process death.

**Accept when:** corruption is visible and two successful updates cannot lose each other's changes. **Evidence/rollback:** synchronized process logs and before/after state hashes; preserve recoverable staged state rather than deleting diagnostic evidence after an uncertain publication.

### S04 — Plugin updates preserve user choice and working versions

**Depends on:** S03. **Modify:** `crates/davinci-coding-agent/src/plugins/command.rs`, `crates/davinci-coding-agent/src/plugins/marketplace.rs`, `crates/davinci-coding-agent/src/plugins/mod.rs`. **Tests:** existing command-flow tests plus `harness_plugin_state.rs`.

**Contract:** updating is not enabling or approving. A new version becomes active only after validation and durable registry publication. Refresh failures are explicit; any deliberate cached reinstall is labeled as such.

- [ ] Add `harness_update_preserves_disabled_plugin`: install, approve where relevant, disable, and update; the plugin stays disabled and no hook becomes dispatchable.
- [ ] Add `harness_failed_same_version_update_keeps_working_install`: inject failure after staging and before registry publication, including a `latest` destination; the old registered directory remains usable.
- [ ] Add `harness_failed_marketplace_refresh_is_not_success`: a failed refresh cannot silently report a normal successful update from stale catalog data.
- [ ] Reproduce the gaps; publish distinct validated installation directories through the S03 registry transaction. Retain approval only when content and authorization still match, and preserve the enabled state. Clean obsolete owned versions only after a successful switch.
- [ ] Run V-C and the integration target. Verify path ownership, rollback, concurrent reader behavior, and clear diagnostics when Windows file locks prevent cleanup.

**Accept when:** an unsuccessful update cannot destroy the working version or reverse an explicit disable. **Evidence/rollback:** fixture directory hashes, stored state, and observed hook eligibility; select the previous validated registry pointer for recovery.

### S05 — Cancellation reaches underlying work and preserves uncertainty

**Depends on:** M00. **Modify:** `crates/davinci-ai/src/stream_reader.rs`, `crates/davinci-ai/src/stream.rs`, `crates/davinci-ai/src/http.rs`; existing supervised-process paths only where the same gap is observed. **Tests:** new `crates/davinci-ai/tests/harness_stream_cancellation.rs`.

**Contract:** foreground cancellation stops new dispatch immediately and requests termination of owned I/O/process work. Completion is not claimed until ownership cleanup is observed; remote billing or server execution may remain unknown.

- [ ] Add `harness_cancel_silent_stream_releases_owned_reader`: coordinate a loopback server that sends no new lines; cancellation must not wait for the production idle timeout to release local reader resources.
- [ ] Add `harness_cancel_releases_backpressure_and_reports_partial_usage`: cover a full channel, partial frame, decoder completion race, and missing final usage without counting unknown usage as zero.
- [ ] Run the regressions first. Connect cancellation to the actual transport/read ownership rather than only the foreground receiver; preserve frame limits and exact accepted bytes.
- [ ] Verify no post-cancel tool dispatch and no new retry once cancellation is set. Use readiness barriers instead of timing-only sleeps in the test driver.
- [ ] Run V-I and the integration target; compare repeated canceled sessions for residual threads, sockets, and handles. Record any backend-specific cleanup limitation.

**Accept when:** bounded local cleanup is observed on supported platforms and remote uncertainty remains explicit. **Evidence/rollback:** owned-resource lifecycle traces; do not terminate unrelated user processes or describe local cancellation as a guaranteed reversal of remote work.

## 8. Workstream M: measurement and budget ownership

### M00 — Establish immutable source, build, and configuration identities

**Depends on:** none. **Modify during future implementation only:** `scripts/release_identity.py`, `scripts/bench/campaign.py`, `scripts/bench/runner.py`, and their existing tests. **Read:** `Cargo.toml`, `Cargo.lock`, `rust-toolchain.toml`, `docs/readiness/release-discipline.md`.

**Contract:** every report names the source identity, effective configuration fingerprint, binary hash, build features, platform, and actual launch path. An exported tree without Git is labeled as an exported snapshot, not a known commit.

- [ ] Inventory existing identity behavior and add `test_harness_rejects_unidentified_or_mismatched_binary`: a same-version but different-hash executable fails release/campaign admission.
- [ ] Add `test_harness_export_snapshot_never_invents_git_revision`: local snapshots remain possible with a content manifest; official revision-gated campaigns refuse missing verifiable Git provenance.
- [ ] Run failures, then connect the actual resolved executable and source identity to all campaign arms. Do not initialize Git or modify the user's installed CLI to fabricate a baseline.
- [ ] Verify build features and fixture-enabled binaries cannot be confused with release artifacts. Preserve the existing release validator rather than adding a second authority.
- [ ] Run V-B and V-ID; retain a minimal identity record with no credentials. A fresh product build is performed only during authorized implementation, not this plan delivery.

**Accept when:** each measured executable can be traced to its actual source/build and configuration. **Evidence/rollback:** identity records and negative fixtures; reject a campaign rather than silently using an unidentified binary.

### M01 — One complete attempt ledger across all execution modes

**Depends on:** M00. **Modify:** `crates/davinci-agent/src/stats.rs`, `crates/davinci-ai/src/openai_cache_diagnostics.rs`, `crates/davinci-ai/src/stream.rs`, `scripts/bench/readiness_metrics.py`, and existing receipt consumers. **Tests:** new `crates/davinci-evals/tests/harness_accounting.rs`; existing Python metric tests.

**Contract:** the Section 3 attempt receipt is emitted once per actual provider attempt, linked to one root. Preserve raw usage provenance and normalize inclusive input to U/R/W once, not at every aggregation layer.

- [ ] Add `harness_attempts_and_children_are_counted_once`: root, retry, worker, reviewer, compaction, and background receipts reconcile without double counting nested summaries.
- [ ] Add `harness_unknown_usage_never_becomes_zero`: missing final usage, missing write counts, counter anomalies, unknown pricing, and incomplete streams remain distinguishable.
- [ ] Add `harness_accounting_mode_parity`: equivalent fixture requests in print, RPC, and interactive paths produce consistent foreground totals and separately attributed background totals.
- [ ] Run regressions; extend the existing stats/receipt path. Record requested and returned model/tier where exposed. Keep timestamps and correlation metadata outside model-visible prompt text.
- [ ] Run V-I, V-E, and V-B. Reconcile fixture totals manually from the raw fixture events as an independent check of the aggregator.

**Accept when:** every actual attempt has a terminal or explicitly unresolved record and reports expose their usage-completeness coverage. **Evidence/rollback:** fixture receipt ledger plus reconciliation report; do not silently migrate old unknown fields into measured values.

### M02 — Root-owned resource reservations and backpressure

**Depends on:** M01. **Modify:** `crates/davinci-agent/src/runtime/capacity.rs`, `crates/davinci-agent/src/runtime/control.rs`, `crates/davinci-agent/src/runtime/workflow/limits.rs`, and graph budget callers. **Tests:** new `crates/davinci-agent/tests/harness_root_budget.rs`.

**Contract:** one root owns spend/request/output/time reservations. Children borrow capacity; retries and resumed tasks use the same ledger. Reservations are not measured spend. Unknown completion does not release a reservation as if nothing occurred.

- [ ] Add `harness_children_cannot_multiply_parent_budget`: simultaneous workers, nested launch attempts, and retries cannot exceed the approved root reservation limit.
- [ ] Add `harness_reservations_reconcile_idempotently`: duplicate receipts, restart, delayed completion, cancellation, and crash leave one reconciled charge or an explicit pending uncertainty.
- [ ] Add `harness_unknown_price_blocks_strict_spend_guarantee`: enforce known request/output limits and explain when an exact monetary ceiling cannot be guaranteed with the available route data.
- [ ] Run regressions; extend `RequestCapacity`/`WorkerSlotCapacity` and current budget ownership. Queue work fairly and give cancellation/control traffic a non-starving path.
- [ ] Run V-A and V-E; stress concurrent admission with deterministic synchronization and confirm active/reserved counts return to a justified state after termination.

**Accept when:** no successful admission silently creates a second budget and every reservation has an owner/disposition. **Evidence/rollback:** budget-transition ledger; stop new spending when reconciliation is uncertain rather than widening limits.

### M03 — Actionable latency and waste diagnostics

**Depends on:** M01. **Modify:** `crates/davinci-agent/src/stats.rs`, `crates/davinci-agent/src/runtime/context_manifest.rs`, `scripts/bench/readiness_metrics.py`, `scripts/bench/cache_report.py`; existing status/RPC presentation paths. **Tests:** existing metric/report tests and `harness_accounting.rs`.

**Contract:** measurements distinguish critical-path wall time from sums of overlapping worker durations, and distinguish model tool-call envelopes from expanded operations. Waste counters are diagnostics, not automatic behavioral vetoes.

- [ ] Add `harness_parallel_time_is_not_summed_as_wall_time` and `harness_batch_reports_expanded_operations`: a batch or worker summary cannot improve metrics by hiding inner work.
- [ ] Add `harness_waste_counters_require_matching_state`: repeated-read/search counters compare arguments, evidence presence, permissions, and source revisions, not merely tool names.
- [ ] Run regressions; expose preparation, queue, provider, tool, verification, integration, and retry time. Add digest/retrieval combined overhead, worker duplication, and background contribution.
- [ ] Keep aggregate telemetry free of source text, prompts, secrets, and raw credential-bearing URLs. Make detailed traces separately opted-in with retention controls.
- [ ] Run V-A, V-E, and V-B. Verify old stored stats still load, missing fields remain unknown, and status rendering cannot trigger extra model calls.

**Accept when:** a slow/expensive run can be explained without conflating overlap, hidden work, or missing evidence. **Evidence/rollback:** fixture timelines and before/after reports; preserve previous parsers while versioning new schema fields.

## 9. Workstream Q: completion correctness and bounded repair

### Q01 — Requirement-to-evidence tracking inside the existing completion path

**Depends on:** M01, S02. **Modify:** `crates/davinci-agent/src/completion.rs`, `crates/davinci-agent/src/living_plan.rs`, `crates/davinci-agent/src/turn.rs`, `crates/davinci-agent/src/prompt/verification.rs`. **Tests:** new `crates/davinci-agent/tests/harness_requirement_completion.rs`.

**Contract:** preserve the user's requested clauses and later corrections; associate each requirement with evidence, explicit failure, or an unverified state. Requirement extraction is model judgment; receipt authenticity is runtime authority. Neither proves semantic coverage alone.

- [ ] Add `harness_partial_public_test_pass_does_not_complete_requirements`: a multi-clause task with one green visible test remains unverified for uncovered behavior.
- [ ] Add `harness_user_correction_supersedes_only_targeted_requirement`: a correction updates its referenced requirement without discarding unrelated constraints after compaction/resume.
- [ ] Add `harness_small_task_does_not_require_extra_planner_call`: ordinary localized edits reuse the normal model response and existing bounded reminder rather than mandatory planning/reviewer calls.
- [ ] Run failures; extend the current once-per-prompt requirement reminder and plan state. Record normal, boundary, and invalid-input expectations when relevant; do not invent product requirements from assistant speculation.
- [ ] Run V-A and V-E. Check print/RPC/interactive parity, plan-mode immutability, ephemeral reminder placement after the stable prefix, and a final answer that identifies uncovered requirements.

**Accept when:** missing coverage cannot become verified success merely because some check passed. **Evidence/rollback:** requirement ledger plus independently graded fixtures; retain the simpler existing reminder if the richer representation has no measured benefit.

### Q02 — Fresh command receipts and protected acceptance evidence

**Depends on:** Q01, S03. **Modify:** `crates/davinci-agent/src/command_receipt.rs`, `crates/davinci-agent/src/verification.rs`, `crates/davinci-agent/src/verification/workspace.rs`, `crates/davinci-agent/src/transaction_verification.rs`. **Tests:** `harness_requirement_completion.rs` and existing transaction/receipt tests.

**Contract:** evidence is bound to command identity, source/test/dependency state, environment identity, exit status, test discovery outcome where known, and before/after verification snapshots. A passing command run on stale or changing inputs is not final-artifact evidence.

- [ ] Add `harness_zero_tests_and_echoed_pass_are_not_test_success`: distinguish a real test-run result from a successful shell exit, printed pass text, or no discovered tests. An unknown runner is unverified, not fabricated parsing confidence.
- [ ] Add `harness_source_change_during_check_invalidates_receipt`: changed content, preserved timestamp/size, dependency change, and later relevant edits invalidate the evidence.
- [ ] Add `harness_grader_cannot_be_weakened_by_worker`: independently owned acceptance fixtures are immutable to workers. Legitimate requested updates to obsolete repository tests are allowed but do not redefine the independent acceptance contract.
- [ ] Run failures; connect existing receipts to Q01 without creating a second truth store. Use dependency-scoped invalidation only when its completeness is demonstrated; otherwise rerun the affected broader check.
- [ ] Run V-A and V-E. Review hooks decorating command output, partially failed pipelines, stale artifact reuse, environment-sensitive tests, and final integrated-tree verification.

**Accept when:** a claimed check can be tied to actual execution against the delivered relevant state. **Evidence/rollback:** receipt-to-snapshot map; on an uncertain dependency footprint, fall back to fresh verification rather than accepting a potentially stale result.

### Q03 — Targeted repair and honest terminal outcomes

**Depends on:** Q01, Q02, M02. **Modify:** `crates/davinci-agent/src/completion.rs`, `crates/davinci-agent/src/turn.rs`, `crates/davinci-agent/src/runtime/progress_watchdog.rs`, `crates/davinci-coding-agent/src/completion_delivery.rs`. **Tests:** `harness_requirement_completion.rs` and existing completion hook fixtures.

**Contract:** feed the model a bounded failure packet with failing requirement, source revision, assertion/diagnostic, and recovery reference. Distinguish verified completion, partial/unverified delivery, user cancellation, budget exhaustion, and infrastructure failure.

- [ ] Add `harness_repair_packet_targets_actual_failure`: an isolated failure does not demand a fresh full-repository investigation; exact evidence remains recoverable.
- [ ] Add `harness_repeated_no_progress_does_not_loop_forever`: after three identical failure/state fingerprints, require a changed hypothesis/evidence or a single bounded escalation; a further unchanged outcome ends as blocked/unverified under the root budget.
- [ ] Add `harness_completion_hook_cap_cannot_grant_verified_status`: preserve the existing three-block hook limit, but reaching it cannot manufacture requirement coverage or override safety denial.
- [ ] Run failures; integrate repair with the existing loop, steering, and cancellation queues. A user request to continue can create an explicitly budgeted continuation, not erase prior failures.
- [ ] Run V-A, V-C, and V-E. Test successful repair, intentionally unfixable fixtures, unavailable dependencies, and a user interruption between verification and final delivery.

**Accept when:** repair improves concrete missing evidence and termination remains honest when it cannot. **Evidence/rollback:** minimal failure traces and outcome reasons; disable the candidate repair policy independently of the underlying completion/permission gates.

## 10. Workstream C: OpenAI cache and transport engineering

### C01 — Consolidate capability evidence and negative-probe handling

**Depends on:** M00, M01. **Modify:** `crates/davinci-ai/src/openai_cache_policy.rs`, `crates/davinci-ai/src/codex_transport.rs`, existing authenticated model capability/catalog paths; update `docs/cache/openai-cache-contract.md` and `docs/cache/codex-backend-probe.md` only during authorized implementation. **Tests:** new `crates/davinci-ai/tests/harness_openai_capabilities.rs`.

**Contract:** extend `OpenAiCacheCapabilities` with evidence sufficient to distinguish documented support, actual shape acceptance, verified semantics, and measured efficacy. Evidence is keyed by authenticated route, exact model, feature, and contract revision. Existing `verified_at` text alone is not runtime proof.

- [ ] Add `harness_openai_capability_requires_route_and_model_evidence`: deceptive hosts, custom proxies, model-name aliases, and caller-supplied compatibility flags cannot enable public-only features.
- [ ] Add `harness_codex_negative_probe_is_preserved`: each September 26 rejection remains disabled for its scoped route/model until explicitly superseded. Older “probe not run” prose cannot override the more specific record.
- [ ] Add `harness_accepted_shape_is_not_semantic_or_cache_success`: HTTP acceptance and zero reported reuse do not promote caching or a tool feature.
- [ ] Run regressions; reconcile `OpenAiCacheCapabilities`, effective policy, and actual dispatch so there is one resolved authority. Store a reason for omitted controls and treat stale/unknown evidence conservatively.
- [ ] Run V-I and the new target. Verify offline fixtures never imply live acceptance, and unsupported feature rollback does not alter authentication or privacy requirements.

**Accept when:** every emitted optional OpenAI control has a scoped justification; unsupported Codex controls stay absent. **Evidence/rollback:** capability matrix and request fixtures; select the last verified route profile rather than globally disabling all checks.

### C02 — Immutable prepared requests and stable cache epochs

**Depends on:** C01, M03, S02. **Modify:** `crates/davinci-ai/src/responses_request.rs`, `crates/davinci-ai/src/stream.rs`, `crates/davinci-agent/src/prepared_context.rs`, `crates/davinci-agent/src/turn_context.rs`, `crates/davinci-agent/src/prompt/composer.rs`. **Tests:** new `crates/davinci-ai/tests/harness_openai_prefix.rs` and existing prepared-context tests.

**Contract:** reuse `PreparedProviderRequest` and `WireManifest`; all byte-sensitive decisions consume the same prepared representation. Separate epoch identity, conversation lineage, source/evidence revision, cache accounting partition, and request correlation ID.

- [ ] Add `harness_ordinary_turn_keeps_prior_wire_segments`: append a user turn, progress change, and runtime notice without rewriting earlier compatible messages or top-level definitions.
- [ ] Add `harness_epoch_changes_only_for_real_contract_change`: model, output contract, relevant settings, tool definitions, and trusted-instruction changes invalidate appropriately; correlation IDs do not.
- [ ] Add `harness_untrusted_context_never_changes_authority`: retrieved text, worker findings, and memory remain data, even when their text resembles a developer message or delimiter.
- [ ] Run failures; make preparation produce one immutable provider view reused by estimation, dispatch, local diagnostics, and replay matching. Preserve semantically meaningful ordering and exact opaque bytes; do not indiscriminately sort conversation arrays.
- [ ] Run V-I and V-A. Exercise resumed sessions, branches, user steering, multibyte inputs, and unsupported model settings. Keep authorization revision checks immediately before dispatch.

**Accept when:** ordinary append-only work preserves compatible prefixes without hiding necessary changes. **Evidence/rollback:** ordered-segment diffs and provider-body fixtures; reverting an optimization never discards authoritative history.

### C03 — Workload-aware breakpoints through the existing wire planner

**Depends on:** C01, C02, M02. **Modify:** `crates/davinci-ai/src/openai_cache_policy.rs`, `crates/davinci-ai/src/stream.rs`, `crates/davinci-ai/src/responses_request.rs`; relevant settings mapping only. **Tests:** `harness_openai_prefix.rs` and `harness_openai_capabilities.rs`.

**Contract:** make `CacheIntent`, `EffectiveOpenAiCachePolicy`, and `PromptCacheWirePlan::resolve` agree on the actual emitted request. Append-only and stable-prefix-only are explicit workload policies; the current legacy-retention bridge must not silently erase that distinction.

- [ ] Add `harness_cache_intent_reaches_actual_wire_request`: inspect generated requests for each intent and each route; report the effective fallback when unsupported.
- [ ] Add `harness_breakpoint_survives_history_append`: preserve the selected boundary as history grows; changing a suffix must not accidentally remove the intended lookup point. Include a mode transition and an extended-message case.
- [ ] Add `harness_breakpoint_constraints_and_disable_are_enforced`: validate eligible content, capability minimums/write limits, complete tool exchanges, and strict-disable refusal on a route that cannot establish the requested property.
- [ ] Run failures; extend the current pure planner, keeping boundary selection deterministic. Use the Section 4 economic estimate only as a tunable heuristic backed by workload data; do not request another model decision.
- [ ] Run V-I and V-E, then E02 only with explicit live authorization. Compare complete task economics, including newly written tokens, rather than advertising an improved read ratio alone.

**Accept when:** selected policy changes the intended wire shape and survives negative cases without speculative fields. **Evidence/rollback:** paired request manifests and, later, provider receipts; keep the previous supported policy selectable per route.

### C04 — Stable tool exposure without stale authorization

**Depends on:** C01, C02, S01, S02. **Modify:** `crates/davinci-agent/src/tools.rs`, `crates/davinci-agent/src/lib.rs`, `crates/davinci-agent/src/turn.rs`, `crates/davinci-ai/src/responses_tools.rs`, `crates/davinci-ai/src/stream.rs`. **Tests:** new `crates/davinci-agent/tests/harness_tool_exposure.rs`; OpenAI request fixtures.

**Contract:** resolve a deterministic initial authorized tool surface before request preparation. Deferred definitions are introduced only through a route-supported mechanism and retain ordered replay positions. Current permission checks remain authoritative for every call, including discovered/batched tools.

- [ ] Add `harness_tool_surface_is_stable_across_ordinary_turns`: repeat the same authorized catalog with stable names, descriptions, schema serialization, and order.
- [ ] Add `harness_discovered_tool_has_real_supported_exposure`: a newly discovered tool is genuinely callable through the emitted contract. Unsupported late delivery triggers an explicit new schema epoch or a supported fallback, not a fictitious “activated” success.
- [ ] Add `harness_revoked_tool_cannot_dispatch_from_cached_schema`: revocation wins after discovery, preparation, queueing, and batch expansion; remove sensitive definitions when visibility itself is restricted.
- [ ] Run failures; preserve the optional-argument semantics of existing tool schemas, including deliberately non-strict tools. Do not make optional arguments required to improve a cache metric.
- [ ] Run V-A, V-I, and the integration target. A public-route `additional_tools` candidate must preserve its input position [O3]; the recorded Codex rejection remains a hard capability constraint.

**Accept when:** cache stability and discovery cannot create a permission bypass or an unusable advertised tool. **Evidence/rollback:** schema fingerprints, actual dispatch records, and negative permission fixtures; fallback to a known supported surface with an explained cache epoch change.

### C05 — OpenAI cache diagnosis and economically correct reporting

**Depends on:** M01, M03, C02, C03. **Modify:** `crates/davinci-ai/src/openai_cache_diagnostics.rs`, `crates/davinci-ai/src/responses_request.rs`, `scripts/bench/cache_report.py`, existing status/RPC consumers. **Tests:** `harness_accounting.rs`, `harness_openai_prefix.rs`, and existing cache-report tests.

**Contract:** provider diagnostics, billed usage, local segment mismatches, local evidence-cache hits, and transport reuse remain separate fields. Sample comparison diagnostics on eligible ordinary requests; do not create extra baseline requests invisibly.

- [ ] Add `harness_cache_diagnostics_do_not_define_billed_usage`: hit/miss/unavailable/expired-comparison classifications do not overwrite token receipts or imply all input was reused.
- [ ] Add `harness_cache_report_uses_disjoint_weighted_totals`: aggregate sum(R)/sum(U+R+W), price using the correct raw-input tier before separation, and avoid subtracting cache buckets twice.
- [ ] Add `harness_local_miss_reason_is_labeled_estimate`: report changed tools, settings, prefix, or epoch from `WireManifest`; do not claim provider expiry from elapsed time alone.
- [ ] Run failures; start an optional diagnostic sample setting at one in twenty eligible requests for development, with zero meaning disabled. Treat this as a proposed sampling parameter, not a provider requirement.
- [ ] Run V-I, V-E, and V-B. Validate diagnostics from streaming terminal events where supported [O2], secret-free reporting, and correct display of missing price/usage coverage.

**Accept when:** each reported saving or cache observation has the appropriate provenance and denominator. **Evidence/rollback:** raw sanitized receipt fixtures and aggregate reconciliation; disable diagnostics independently from actual request correctness.

### C06 — Owner-bound native replay, continuation, and safe retries

**Depends on:** C02, M02, S05. **Modify:** `crates/davinci-ai/src/responses_ledger.rs`, `crates/davinci-ai/src/codex_transport.rs`, `crates/davinci-ai/src/provider_retry.rs`, `crates/davinci-ai/src/stream.rs`, `crates/davinci-agent/src/turn.rs`. **Tests:** new `crates/davinci-ai/tests/harness_openai_recovery.rs` and existing operation-journal fixtures.

**Contract:** the canonical local history plus exact native provider items is the recovery basis. Continuation is bound to authenticated conversation ownership, model/contract, and a completed matching prefix. Public WebSocket documentation supports incremental continuation and documents full-context recovery conditions; verify adapter semantics separately. [O4]

- [ ] Add `harness_native_replay_preserves_unknown_items_and_tool_pairs`: round-trip opaque/unknown items, complete tool-call/result IDs, and role boundaries without reinterpretation or silent loss.
- [ ] Add `harness_reconnect_never_borrows_another_worker_response`: shared affinity cannot mix root/worker/fork/account state. Missing or invalid continuation falls back only to that owner's valid full context.
- [ ] Add `harness_uncertain_request_does_not_duplicate_effects`: disconnect before headers, after acceptance, during tool arguments, after a tool receipt, and before terminal publication; retain uncertainty and do not reexecute committed side effects.
- [ ] Run failures; connect retries to the root budget and existing operation journal. Rate limits use bounded provider-supported retry guidance; permanent request-shape/quota failures do not loop. An optional unsupported control is disabled once per scoped capability revision, not probed repeatedly in ordinary tasks.
- [ ] Run V-I, V-A, and V-E. Keep streaming buffers and retained event artifacts bounded; profile duplicate raw-event retention separately before optimizing it. Confirm cancellation prevents another attempt.

**Accept when:** supported reconnects recover context without duplicated mutations, cross-owner state, or invented cache hits. **Evidence/rollback:** injected transport failures, request/operation ledgers, and exact replay fixtures; prefer conservative owner-valid replay over uncertain delta reuse.

### C07 — Conditional public-route prewarm, effort-update, and compaction experiments

**Depends on:** C01–C06, Q02, E01. **Modify only when triggered:** `crates/davinci-ai/src/openai_cache_policy.rs`, `crates/davinci-ai/src/stream.rs`, `crates/davinci-ai/src/responses_ledger.rs`, `crates/davinci-agent/src/compaction.rs`, and existing model/effort settings. **Tests:** `harness_openai_prefix.rs`, `harness_openai_recovery.rs`.

**Contract:** public-route experiments remain optional, independently switchable, and bounded by capability evidence and the root budget. They must not alter unsupported Codex behavior.

**Trigger:** authorized public-route usage exists and baseline traces show startup latency, effort-switch prefix churn, or context pressure worth investigating. This task is not required to improve the currently accepted Codex route.

- [ ] Define separate candidate arms: no prewarm versus paid prewarm; fixed versus supported append-only effort update; existing local compaction versus supported provider compaction. Never combine all three in the first experiment.
- [ ] Add `harness_optional_public_features_never_leak_to_codex`: capability and negative-probe gates prevent speculative fields/endpoints on the OAuth adapter.
- [ ] Add `harness_compacted_window_is_preserved_and_requirements_recoverable`: keep the provider's canonical returned context intact, preserve opaque items, and verify constraints/evidence across repeated compactions [O5]. Do not rewrite parity prompts without approved divergence.
- [ ] Run regressions and the applicable V-I/V-A gates before implementing a candidate. Preserve an independent switch per feature; output/reservation costs include abandoned warmups and compaction calls.
- [ ] Run E02 with the authorized route/model. Promote only measured improvement across startup-to-completion latency, total spend, correctness, and recovery. A cache ratio drop after useful compaction is not automatically a failure.

**Accept when:** each enabled candidate independently earns its cost and has a verified fallback. **Evidence/rollback:** separate arm reports and long-session fixtures; leave untriggered or unproven features disabled rather than blocking the rest of the program.

## 11. Workstream X: context and evidence efficiency

### X01 — Bounded, high-signal working sets with exact recovery

**Depends on:** Q01, C02, M03. **Modify:** `crates/davinci-agent/src/prepared_context.rs`, `crates/davinci-agent/src/provider_budget.rs`, `crates/davinci-agent/src/pruning.rs`, `crates/davinci-coding-agent/src/native_extensions/token_governor.rs`. **Tests:** new `crates/davinci-agent/tests/harness_context_evidence.rs`; existing governor fixtures.

**Contract:** mandatory current requirements, policy, active blockers, and complete live exchanges cannot be dropped. Optional historical output can leave the working set only with an authorized exact recovery path and visible, usable reference. Admission uses conservative bounds, not a fabricated exact token count.

- [ ] Add `harness_context_omission_preserves_mandatory_state`: near-window prompts keep the latest full request and corrections; impossible mandatory budgets fail before provider dispatch.
- [ ] Add `harness_digest_and_cursor_support_exact_recovery`: compiler/test/search digests retain actionable failures and a model-visible cursor; Unicode, giant single lines, filtered search, and page continuation recover exact content.
- [ ] Add `harness_compression_cost_includes_rereads`: an output reduction that forces additional reads/model calls cannot report savings from the initial digest alone.
- [ ] Run failures; prefer deterministic formatting/filtering, relevant symbol ranges, and compact failure packets to new summarizer calls. Keep source bytes and warnings intact when the model needs them to edit safely.
- [ ] Run V-A, V-C, and V-E. Validate bounded output under adversarial logs and inaccessible artifacts; a missing artifact triggers honest retrieval failure/recovery, not invented evidence.

**Accept when:** less irrelevant material reaches the model without losing retrievability or task constraints. **Evidence/rollback:** lossless retrieval cases and end-to-end compression/retrieval accounting; independently disable aggressive reduction if recovery costs dominate.

### X02 — Shared repository facts with strong freshness and permission boundaries

**Depends on:** S01–S03, M03, Q02. **Modify:** `crates/davinci-agent/src/runtime/cache/key.rs`, `runtime/cache/workspace.rs`, `runtime/cache/singleflight.rs`; existing repository, language-intelligence, and engineering-snapshot consumers. **Tests:** `harness_context_evidence.rs` and current cache/invalidation suites.

**Contract:** local reuse keys include workspace/branch identity, relevant dependency state, tool/query parameters, and the authorization domain. A shared successful read is reusable only while content and permission validity can be established. Coalescing is restricted to proven read-only operations.

- [ ] Add `harness_same_stamp_content_change_invalidates_critical_evidence`: same size and timestamp must not fool source-sensitive acceptance or mutation preconditions; use content verification at those boundaries.
- [ ] Add `harness_singleflight_does_not_cross_permission_or_workspace`: identical queries by different scopes/branches cannot leak cached content; revocation invalidates reuse eligibility before return.
- [ ] Add `harness_negative_search_result_has_scope_and_freshness`: index lag, file changes, ignored directories, parse failures, and permission denial cannot become a cached global absence claim.
- [ ] Run failures; reuse the existing cache and engineering-snapshot owners. Avoid five independent scans for repository, impact, build, test, and verification consumers when one validated fact set already exists.
- [ ] Run V-A, V-C, and V-E. Measure invalidation cost and fallback rates; when observers overflow or freshness is unknown, rescan or hash the necessary scope rather than silently serving stale facts.

**Accept when:** repeated investigation is reduced without weakening source freshness or access control. **Evidence/rollback:** concurrent read/invalidation tests and scan counts; correctness falls back to fresh reads when certainty is unavailable.

### X03 — Long-session local performance and safe context-VM evaluation

**Depends on:** X01, X02, C05, E01. **Modify only after profiling:** `crates/davinci-agent/src/runtime/context_vm/`, `crates/davinci-agent/src/prepared_context.rs`, `crates/davinci-session/src/jsonl_repo.rs`. **Tests:** existing context-VM suites and new `crates/davinci-session/tests/harness_session_listing.rs`.

**Contract:** keep authoritative history and derived state separate. VM enablement is a measured experiment, not a prerequisite for correctness. Revision shortcuts are allowed only when every relevant mutation path is tracked; otherwise retain the safe verification/hash fallback. Existing documentation states that warm preparation still depends on history bytes. [L6]

- [ ] Profile 100-, 300-, and 1,000-turn fixtures for preparation CPU, peak memory, allocations, page storage, and recovery. These are local fixture sizes, not performance guarantees.
- [ ] Add `harness_public_mutation_invalidates_prepared_context`: direct legacy field mutation cannot bypass invalidation. Add `harness_session_listing_reads_header_not_full_transcript` with a bounded-reader observation rather than an arbitrary timing threshold.
- [ ] Add `harness_context_vm_two_compactions_preserve_user_constraints`: compare off/shadow/active across two compactions, branch changes, missing pages, restart, and source recovery.
- [ ] Run regressions; make only profiled local improvements, including header-only listing where sufficient. Do not claim O(1) preparation while compatibility permits uncontrolled input mutation.
- [ ] Run V-A, V-S, and V-E. E03 decides VM promotion from quality, cost, latency, and recovery; otherwise leave it off and retain the improved ordinary context path.

**Accept when:** observed hot-path cost improves and every compatibility invalidation case stays correct. **Evidence/rollback:** reproducible local profiles plus long-session outcomes; keep the existing mode switch and authoritative-history rebuild path.

## 12. Workstream T: tool selection and scheduling

### T01 — Measure Full, Lean, and task-relevant tool surfaces

**Depends on:** C04, M03, X01, E01. **Modify:** `crates/davinci-agent/src/tools.rs`, `crates/davinci-agent/src/lib.rs`, `crates/davinci-coding-agent/src/settings.rs`, existing family discovery. **Tests:** `harness_tool_exposure.rs` and OpenAI request fixtures.

**Contract:** the core tool surface remains deterministic and genuinely useful. Relevant families may be selected before the first provider request; later discovery follows C04. Compatibility aliases need not be redundantly advertised, but existing runtime names remain supported where required.

- [ ] Add `harness_relevant_tool_family_selection_is_authorized`: exact names and known families select only registered, allowed tools; arbitrary prefixes and missing adapters cannot confer availability.
- [ ] Add `harness_tool_discovery_failure_is_recoverable`: ambiguous queries, unavailable schemas, and pagination return bounded actionable results without inventing a callable tool.
- [ ] Run failures; reduce duplicated descriptions and misleading parameters before adding any new tool. When an existing operation returns source context automatically, authorize each included path as a direct read would be authorized.
- [ ] Compare Full, existing Lean, and Lean with pre-request relevance as separate E02 arms. Include schema tokens, discovery calls, malformed calls, and tasks where the first selection misses a necessary tool.
- [ ] Run V-A/V-I/V-C and the integration target. Promote only a measured winner; do not change the default merely because Lean exposes fewer schemas.

**Accept when:** the chosen surface reduces total round trips/cost without tool-use or success regressions. **Evidence/rollback:** per-task exposure/activation records and errors; retain the previous full/lean switch.

### T02 — Work-conserving read scheduling with mutation barriers

**Depends on:** S01, S02, M02, M03. **Modify:** `crates/davinci-agent/src/scheduler.rs`, `crates/davinci-agent/src/batch.rs`, `crates/davinci-agent/src/turn.rs`. **Tests:** existing scheduler tests and new `crates/davinci-agent/tests/harness_scheduler.rs`.

**Contract:** independent read work within one safe region may refill freed capacity without waiting for the slowest fixed chunk. Preserve source-order result publication and every mutation/unknown-effect barrier. Keep the existing eight-call cap unless separately measured and approved.

- [ ] Add `harness_slow_read_does_not_idle_free_slots`: a blocked first read does not prevent later independent reads in the same region from starting in available slots.
- [ ] Add `harness_write_barrier_preserves_read_after_write`: no work after a mutation barrier starts early; a later read sees the mutation. Unknown tools and writable shared workers remain barriers.
- [ ] Add `harness_scheduler_cancel_and_panic_close_every_call`: queued cancellation, a worker failure, and an execution panic produce bounded terminal/error handling rather than lost results or a hanging join.
- [ ] Run failures; improve the existing scheduler rather than migrating the entire runtime. Batch operations still receive individual permission checks, receipts, and root-budget attribution.
- [ ] Run V-A and the integration target. Use synchronization to prove ordering and concurrency; benchmark queue occupancy and critical-path time separately from correctness assertions.

**Accept when:** safe capacity is better utilized without violating call semantics. **Evidence/rollback:** deterministic event order and concurrency traces; preserve sequential execution as a diagnostic fallback, not an excuse to suppress a failed parallel invariant.

### T03 — Event-driven owned process readiness and reuse

**Depends on:** S05, M02, M03. **Modify:** `crates/davinci-agent/src/process_manager.rs`, `crates/davinci-agent/src/process_manager/command.rs`, `crates/davinci-agent/src/jobs/managed.rs`, `crates/davinci-coding-agent/src/process_manager_integration_tests.rs`. **Tests:** existing process integration tests and `harness_scheduler.rs` where appropriate.

**Contract:** reuse an owned compatible service/process only when executable/configuration/workspace/environment identity and readiness are valid. Deliver bounded ready/failed/exited events without requiring repeated model polling.

- [ ] Add `harness_service_reuse_checks_identity_and_readiness`: source/config/port changes trigger the correct invalidation; a foreign process on the port is not adopted as owned.
- [ ] Add `harness_readiness_failure_and_restart_are_bounded`: failed startup, exit during readiness, and reconnect cannot leave a task polling indefinitely or returning a false ready state.
- [ ] Run failures; connect existing process lifecycle notifications to tools and worker orchestration. Do not retain a service beyond its allowed scope or carry credentials between worker environments.
- [ ] Measure avoided process restarts and model status checks, accounting for idle resource use. Confirm stale output is not delivered as a new execution result.
- [ ] Run V-A and V-C. Exercise Windows native supervision and supported WSL/Linux paths separately; unavailable ownership/isolation capabilities must be reported rather than emulated by unsafe host execution.

**Accept when:** readiness is established from owned runtime evidence and polling round trips decline. **Evidence/rollback:** process identity/lifecycle logs; restart only the owned affected process or disable reuse if compatibility cannot be proved.

## 13. Workstream A: agent and subagent orchestration

### A01 — Minimal-team strategy and explicit worker contracts

**Depends on:** M02, Q01, X01, T01. **Modify:** `crates/davinci-agent/src/subagent.rs`, `crates/davinci-agent/src/runtime/tools_agent.rs`, `crates/davinci-agent/src/runtime/workflow/limits.rs`, `crates/davinci-coding-agent/src/native_extensions/graph/config.rs`. **Tests:** new `crates/davinci-agent/tests/harness_worker_contracts.rs`.

**Contract:** the lead selects the smallest justified execution strategy. Localized work stays solo; independent uncertainty can receive a bounded read worker; writers require isolation and acceptance criteria. The runtime schedules the plan without additional “which task is finished?” model calls.

- [ ] Add `harness_small_task_does_not_spawn_workers`: a low-risk local change does not automatically trigger a planner, critic, and variant tournament.
- [ ] Add `harness_worker_contract_has_scope_budget_and_output`: a launch without the pinned base, allowed scope/tools, acceptance contract, root reservation, deadline, and result format is rejected or completed from authoritative defaults without widening permissions.
- [ ] Run failures; reuse existing worker/context packet structures. Retain the existing 2,500-token context-packet ceiling where it applies; exact source remains retrievable. Start experiments with a proposed 1,000-token result summary ceiling plus evidence links, never truncating required structured result fields silently.
- [ ] Evaluate solo versus one helper versus two/four-worker configurations on eligible dev tasks. Two/four are experiment limits, not a global default change. Include startup, duplicated investigation, integration, and review costs.
- [ ] Run V-A, V-C, and V-E. Codex's own guidance treats read-heavy parallel work as a useful starting point and warns about write coordination [O7]; require measured evidence before expanding write-heavy fan-out.

**Accept when:** worker launch is purposeful, bounded, and improves the complete outcome economics. **Evidence/rollback:** launch-decision and task-contract fixtures; keep solo execution available when delegation adds no value.

### A02 — Isolate writers and verify integration against the current base

**Depends on:** A01, Q02, S02, S03. **Modify:** `crates/davinci-agent/src/runtime/worktree.rs`, `crates/davinci-agent/src/runtime/contracts.rs`, `crates/davinci-coding-agent/src/native_extensions/graph/controller.rs`, `crates/davinci-coding-agent/src/native_extensions/graph/worker_hooks.rs`. **Tests:** `harness_worker_contracts.rs` and graph revision/persistence suites.

**Contract:** one writer owns each conflicting resource scope. Worktrees isolate files; databases, ports, mutable build outputs, process state, and environment overlays receive separate ownership. The lead integrates only a patch whose base assumptions remain valid and then verifies the combined artifact.

- [ ] Add `harness_parallel_writers_cannot_share_conflicting_resources`: scope collisions, shared databases/ports, and parent-checkout escape are rejected before mutation.
- [ ] Add `harness_stale_worker_patch_cannot_overwrite_user_change`: a user edit or integrated sibling patch after launch invalidates the worker's assumptions; return a conflict instead of silently overwriting.
- [ ] Add `harness_individual_worker_passes_do_not_certify_integration`: independently passing branches that fail together remain an integration failure until repaired and reverified.
- [ ] Run failures; bind writer leases and patch evidence to source identity and policy revision. No Git metadata means isolated-writer workflows fail with a clear prerequisite instead of initializing a repository or writing into a shared checkout.
- [ ] Run V-A, V-C, and V-E. Include file renames, untracked user files, symlink/Windows path aliases, migration ordering, and revocation while a worker is queued.

**Accept when:** worker concurrency cannot corrupt user/sibling work and combined changes get their own receipts. **Evidence/rollback:** lease, patch, and integration ledgers; revert only task-owned effects with conflict checks.

### A03 — Deterministic DAG scheduling, recovery, and worker termination

**Depends on:** A01, A02, C06, T02, T03. **Modify:** `crates/davinci-agent/src/runtime/team.rs`, `crates/davinci-agent/src/runtime/workflow/`, `crates/davinci-coding-agent/src/native_extensions/graph/controller.rs`, `controller_attempts.rs`, `worker_sessions.rs`. **Tests:** graph continuation/recovery suites and `harness_worker_contracts.rs`.

**Contract:** readiness is derived from task dependencies and verified artifacts, not chat messages. One root owns capacity, retries, and termination. A completed worker is accepted only with the expected artifact and required evidence, not merely exit zero.

- [ ] Add `harness_ready_frontier_refills_without_global_wait`: ready independent nodes begin when resources become available; an unrelated long node cannot stall safe integration work.
- [ ] Add `harness_worker_restart_preserves_attempt_and_budget_identity`: crash/resume cannot reset counters, duplicate committed effects, or accept an incompatible prior artifact.
- [ ] Add `harness_root_cancel_stops_all_owned_workers`: cancellation and deadline propagate to waiting/running children; missing submit artifacts and escaped/unowned processes are explicit failures.
- [ ] Run failures; strengthen the existing graph/team controller and event bus. Use bounded watchdogs, fair capacity, and evidence-backed replay; do not introduce a second model-driven coordinator loop.
- [ ] Run V-A, V-C, and V-E. Test a dependency cycle, partial artifact write, lost worker event, stale lease, poisoned lock, and restart during final result publication.

**Accept when:** orchestration makes forward progress or terminates with a precise reason and preserves root accounting. **Evidence/rollback:** deterministic DAG/event replay; cancel safely and retain evidence rather than restarting an uncertain writer from scratch.

### A04 — OpenAI-only model/effort routing with cache-aware evaluation

**Depends on:** A01–A03, C01–C06, E01. **Modify:** `crates/davinci-agent/src/effort.rs`, existing OpenAI model resolution, `crates/davinci-coding-agent/src/native_extensions/graph/config.rs`. **Tests:** worker contract and OpenAI prefix fixtures.

**Contract:** first establish harness improvements with the user's selected model and effort fixed. Routing is a separate explicitly selected policy; it never silently downgrades a requested model or routes to another provider. Exact model capability, context budget, pricing, and output contract must be known.

- [ ] Add `harness_explicit_model_selection_is_not_silently_downgraded`: user-fixed model/effort remains fixed; an unavailable choice is reported rather than replaced covertly.
- [ ] Add `harness_routing_is_openai_scoped_and_episode_stable`: workers choose only approved OpenAI models; a switch records its reason, cost estimate, and cache/epoch effect.
- [ ] Add `harness_cheaper_worker_failure_counts_against_routing`: escalation and repeat investigation remain in total task economics, not hidden in a second task.
- [ ] Run failures; evaluate lower-cost read-only worker assignments against the same-model baseline. Keep correctness-sensitive writing/review on the measured adequate model; expensive models are justified by outcomes, not labels such as “maximum.”
- [ ] Run V-A/V-I/V-C and E02. Promote routing per task class only when success and all-attempt economics justify it; supported effort updates use C07, otherwise a deliberate epoch change is recorded.

**Accept when:** the selected routing policy is transparent and improves measured cost/success tradeoffs. **Evidence/rollback:** per-role/model results and escalations; reverting to the user's fixed model must preserve history, permissions, and budget consumption.

## 14. Workstream P: production acceptance

### P01 — Risk-specific product acceptance on the integrated artifact

**Depends on:** Q01–Q03 and M00; A02 also applies when parallel writers are used. **Modify:** `crates/davinci-agent/src/verification.rs`, existing verification-planner/build/test-impact consumers, `crates/davinci-coding-agent/src/completion_delivery.rs`. **Tests:** new `crates/davinci-evals/tests/harness_product_acceptance.rs`.

**Contract:** requirements select relevant acceptance packs. All packs verify the final integrated state. The harness does not claim universal production readiness from a unit test or from an agent's subjective review.

- [ ] Add `harness_acceptance_pack_matches_change_risk`: data migration, authorization, API contract, UI flow, and simple documentation changes select proportionate checks and record why.
- [ ] Add `harness_final_artifact_not_intermediate_branch_is_accepted`: passing pre-integration or pre-final-edit evidence cannot certify a later build.
- [ ] Add `harness_unverified_external_dependency_blocks_stronger_acceptance`: unavailable real-service testing remains explicit while local evidence can still be delivered honestly.
- [ ] Run failures; require normal/boundary/invalid/concurrent behavior where relevant, persistent data checks, authorization negatives, migration reversibility/compensation, and scoped security findings. Expand from focused to contract/release checks based on demonstrated impact; uncertain impact selects the broader safe check.
- [ ] Run V-A, V-C, and V-E. Freeze independent acceptance fixtures before the implementer sees results; do not redefine a required check to avoid a failure.

**Accept when:** each requirement has appropriate final-state evidence or an explicitly weaker acceptance level. **Evidence/rollback:** requirement × check × artifact matrix; retain incomplete status until the missing gate is satisfied.

### P02 — Running user journeys and operational failure paths

**Depends on:** P01, T03, S05. **Modify:** existing browser-verification/native browser integration, `crates/davinci-agent/src/process_manager.rs`, existing release-quality eval fixtures. **Tests:** `harness_product_acceptance.rs` and existing browser/process fixture suites.

**Contract:** a real isolated application instance, owned service state, and relevant browser/API checks validate behavior. Screenshots support UI review but do not prove authorization, persistence, accessibility, or service integration by themselves.

- [ ] Add `harness_journey_requires_real_persistence_and_auth_boundaries`: exercise a complete synthetic user flow, reload/restart behavior, and an unauthorized negative path against the intended build.
- [ ] Add `harness_operational_failures_have_observable_outcomes`: failed startup, database outage, timeout, cancellation, partial migration, and rollback produce expected health/log evidence without leaking secrets.
- [ ] Run failures; expose test-instance logs, health, browser state, and bounded metrics through existing tools. Include loading/error/empty UI states and accessibility/responsive assertions only for relevant UI changes.
- [ ] Validate native Windows supervision and WSL/Linux isolation separately. A host without the required filesystem/network/process guarantees reports unsupported; it must not silently execute an “isolated” task on the unrestricted host.
- [ ] Run V-A, V-C, V-E, and the applicable existing browser/Node fixture command. External services and production data remain excluded unless separately authorized.

**Accept when:** requested journeys work on the identified running product and failure behavior is observable. **Evidence/rollback:** synthetic journey, logs, checks, and owned-instance identity; stop only owned resources and preserve failure artifacts.

### P03 — Honest acceptance levels and a verifiable delivery bundle

**Depends on:** P01, P02, M00, M01. **Modify:** `crates/davinci-coding-agent/src/completion_delivery.rs`, `scripts/release_identity.py`, existing release/status/RPC reporting, `docs/readiness/release-discipline.md`. **Tests:** `harness_product_acceptance.rs` and identity tests.

**Contract:** distinguish implemented, locally verified, integration verified, release candidate, and deployed/operationally verified. Reuse compatible status mechanisms; any machine-readable schema or exit behavior change is versioned and regression-tested.

- [ ] Add `harness_delivery_level_requires_matching_evidence`: each stronger level requires its defined checks; reaching a reminder/retry cap does not advance the level.
- [ ] Add `harness_installed_hash_mismatch_is_visible`: an installed executable different from the tested build cannot be presented as updated merely because versions match.
- [ ] Add `harness_deploy_and_publish_need_separate_authority`: release packaging does not authorize credentials, account changes, publication, or deployment.
- [ ] Run failures; produce a bundle with requirements, scoped change summary, fresh receipts, artifact/source/configuration identity, known gaps, operational instructions, and rollback/compensation references. Never include actual secrets or opaque reasoning in the human-facing bundle.
- [ ] Run V-C, V-E, V-B, and V-ID. Verify a reviewer can establish what passed without trusting narrative claims; preserve the no-commit/no-deploy instruction until explicitly changed.

**Accept when:** the handoff accurately names the verified artifact and remaining limits. **Evidence/rollback:** machine-readable acceptance record plus human-readable report; incomplete delivery stays useful without overstating its acceptance level.

## 15. Workstream E: evaluation and controlled experiments

### E01 — A private, uncontaminated task suite and frozen baselines

**Depends on:** M00, M01. **Modify:** `scripts/bench/private_suite.py`, `scripts/bench/readiness_protocol.py`, `scripts/bench/campaign.py`, and existing tests; reuse `crates/davinci-evals/src/competitor/report.rs`. **Tests:** `scripts/bench/tests/test_private_suite.py`, `test_readiness_protocol.py`, `test_campaign.py`.

**Contract:** use 40–60 development tasks and at least 150 distinct holdout tasks from 3–5 authorized real repositories as a starting suite. Three repetitions per task are repeated measurements, not three independent tasks. Freeze prompts, mutation scope, grader, model controls, versions, and pricing before tuning.

- [ ] Add `test_harness_reference_and_grader_are_unreachable`: the worker cannot read hidden tests, reference solutions, later Git history, reference branches, or another arm's artifacts. Disable external lookup paths that would reveal the held-out solution.
- [ ] Add `test_harness_task_accepts_equivalent_valid_solution`: independently grade requested behavior rather than an exact reference diff; the documented allowed scope must accommodate legitimate alternative fixes and requested test changes.
- [ ] Validate each starter fails the intended grader and its reference passes; reject ambiguous tasks, environmental breakage, and graders that test undisclosed requirements before the campaign begins.
- [ ] Freeze an identified DaVinci baseline and matched Codex baseline. Match exact OpenAI model, effort, relevant tool access, environment, timeout, and budget where controls permit; disclose unmatchable differences. No live Claude-model arm is authorized in this scope.
- [ ] Run V-B and V-E. Label tasks by size, risk, language, concurrency, external-service needs, interaction mode, and long-session behavior; avoid claiming generalization beyond the sampled repositories/population.

**Accept when:** the suite measures task outcomes without solution leakage or changing the exam after seeing results. **Evidence/rollback:** fingerprinted manifests, validation outcomes, and split-isolation checks; invalidate a contaminated holdout and obtain a genuinely fresh confirmation set.

### E02 — Paired OpenAI campaigns with all-attempt economics

**Depends on:** E01, M01–M03, candidate's completed offline gate. **Modify:** `scripts/bench/campaign.py`, `scripts/bench/readiness_metrics.py`, `scripts/bench/cache_report.py`, `crates/davinci-evals/src/openai_cache_eval.rs`. **Tests:** existing campaign/metrics/cache tests and `harness_accounting.rs`.

**Contract:** one major variable changes per development experiment. Retain all failures, missing rows, duplicates, timeouts, and unresolved usage. Report all-attempt task economics and separate mutually successful pairs. Repetition order is counterbalanced; confidence calculations preserve task pairing and relevant clustering.

- [ ] Add `test_harness_failed_or_missing_rows_cannot_improve_score`: failed attempts stay in cost/outcome denominators; missing evidence cannot become a passed competitor case or a free attempt.
- [ ] Add `test_harness_warmth_is_observed_not_assumed`: workspace reset does not reset provider cache. Use supported separation and independent histories, record starting cache conditions, and label “observed cold” only when usage supports that classification.
- [ ] Add `test_harness_repetitions_are_not_independent_tasks`: bootstrap/resampling retains paired task clusters; small samples do not produce unsupported stable tail-latency or superiority claims. Report limited repository-level generalizability separately.
- [ ] Run V-B/V-E first. Before live execution, require exact route/model, approval ID, finite positive spend limit, nonzero request limit, bounded concurrency, data-retention choice, and source/manifest identity. With any item absent, remain offline.
- [ ] Run an authorized pilot to calibrate campaign cost, then the predeclared dev/holdout sequence. Two harnesses × 150 tasks × three repetitions is 900 task runs before retries; that scale requires explicit budgeting, not a hidden default.

**Accept when:** the reported result is reproducible, fairly scoped, and not improved by accounting omissions or cross-arm warming. **Evidence/rollback:** immutable campaign directory and independently generated report; infrastructure failures get a predeclared disposition applied symmetrically, not ad hoc exclusion.

### E03 — Ablations, long sessions, and production-task coverage

**Depends on:** E01, E02, relevant candidate tasks. **Modify:** existing `scripts/bench/` manifest/report paths, `crates/davinci-evals/src/openai_cache_eval.rs`, and current behavioral/product fixtures. **Tests:** campaign tests plus `harness_product_acceptance.rs`.

**Contract:** isolate component value before promoting combinations. Test the workflows the user will actually run, including interactive/RPC background behavior and production-sized changes, not just print-mode microtasks.

- [ ] Freeze separate arms for requirement review, Full/Lean/relevance, governor output reduction, read-result reuse, scheduler change, solo/two/four workers, fixed/routed OpenAI models, memory/skill injection, and context VM off/shadow/active. Do not run a full combinatorial explosion.
- [ ] Include cold and warm conversations, small requests, cross-component features, long sessions with at least two compactions, user corrections, interruption/resume, repeated tool failures, stale workspace evidence, and combined-writer integration.
- [ ] Test background learning/security review only as explicitly enabled arms, with all receipts charged to their originating root/session. Do not introduce non-OpenAI model/embedding calls as an unnoticed dependency of a supposedly OpenAI-only experiment.
- [ ] Use independent tests plus blinded human review for acceptance dimensions that deterministic checks cannot establish. An LLM judge can assist, but cannot be the only authority for correctness, security, or a production-readiness claim.
- [ ] Run V-B/V-E and authorized E02 campaigns. Evaluate promising combined candidates after individual wins, then use a genuinely untouched confirmation set. Repeatedly inspecting the same holdout makes it development data.

**Accept when:** kept components show marginal value on relevant workloads and their combination remains correct. **Evidence/rollback:** component effect and interaction reports; unmeasured or neutral expensive features stay disabled rather than acquiring permanent default status.

## 16. Workstream R: rollout and simplification

### R01 — Promote measured winners and practice rollback

**Depends on:** completed candidate gates, P03, E02, E03. **Modify:** `crates/davinci-coding-agent/src/settings.rs`, existing compatibility/feature switch owners, `crates/davinci-evals/src/competitor/report.rs`, and applicable existing promotion workflows. **Tests:** existing settings/gate tests and `harness_product_acceptance.rs`.

**Contract:** a candidate must pass hard correctness/authority gates, meet its predeclared quality/efficiency criterion, and retain a working conservative fallback. Success claims state exact competitor version, model, route, task distribution, sample size, and uncertainty.

- [ ] Add `harness_promotion_rejects_missing_or_incomplete_evidence`: no switch changes default from fixture scores, undocumented model changes, missing cost coverage, or insufficient sample alone.
- [ ] Add `harness_rollback_preserves_history_permissions_and_effects`: rollback while a session exists cannot discard acknowledged results, restore revoked access, or reset consumed budget.
- [ ] Run failures; start with explicit preview use, then an operator-approved canary before changing defaults. Do not generate shadow model calls unless separately budgeted; shadow local computations are measured for overhead too.
- [ ] Rehearse rollback for cache policy, tool surface, context reduction, scheduler, worker routing, and optional background components. Stop rollout immediately on observed boundary/data-integrity violations; inconclusive quality evidence is not a win.
- [ ] Run V-A/V-I/V-C/V-E/V-B at the actual changed boundaries and V-F at the integrated release gate. Publish no “better than Claude Code” claim without its own authorized comparable evidence.

**Accept when:** the promoted configuration has measurable benefit and reversible operational behavior. **Evidence/rollback:** acceptance decision with frozen criteria, exact artifact identities, and a tested rollback record.

### R02 — Measure-or-cut and final architecture reconciliation

**Depends on:** R01, E03. **Modify:** only measured redundant paths in existing prompt, cache, worker, or extension modules; update `docs/readiness/README.md`, `docs/openai-efficiency.md`, `docs/tool-discovery.md`, and `docs/context-vm.md` during authorized execution. **Tests:** every removed/changed module's established compatibility suite.

**Contract:** retain one owner per capability, budget, requirement record, and evidence fact. Remove candidate complexity only when its cost and lack of value are demonstrated. Do not remove supported non-OpenAI behavior merely because this optimization program is OpenAI-only.

- [ ] Build a retained/disabled/removed table for each candidate with its measured outcome, runtime overhead, and maintenance/compatibility cost.
- [ ] Audit duplicated prompt rules, unconditional tournaments/reviewer loops, duplicated repository scans, parallel cache state, and overlapping orchestration paths. Propose scoped cleanup; do not delete user instruction files or change their authority silently.
- [ ] Add regressions for every removed fallback/alias behavior that remains part of a supported contract, then simplify only after those boundaries are explicit. Extract small helpers from touched large files only when it clarifies the responsible concern.
- [ ] Run the affected verification lanes and final V-F. Reconcile docs that say a probe was not run with scoped probe evidence; distinguish implementation status from actual measured default decisions.
- [ ] Complete the Section 18 acceptance ledger, listing achieved targets, missed targets, disabled candidates, unresolved limitations, source/build identity, and the next blocked external authorization if any.

**Accept when:** the shipped architecture is simpler than the candidate collection and documentation matches measured behavior. **Evidence/rollback:** keep/cut table and final acceptance bundle; no broad rewrite or feature deletion is justified solely by this task's title.

## 17. Verification command reference

### 17.1 What these commands mean

These are commands for **future authorized implementation**, not checks performed while writing the plan. Run them from the project root with the pinned toolchain. Use a dedicated external `CARGO_TARGET_DIR` per implementation session so fixture builds do not replace the executable a user is running. Set the project's existing offline/network-disable fixture controls in the child test environment; never globally change the user's credentials or configuration.

Every targeted run must report at least one selected test. “0 tests” is not a pass of the named regression. The red run must fail on the intended behavioral assertion after a successful build, not merely because a new test file is missing or dependencies are unavailable. When a baseline already passes, retain the useful regression and document “already satisfied”; do not deliberately break production code to manufacture a finding.

RTK commands follow the project's Windows guidance. Preserve complete underlying logs as evidence; summarized output is not a substitute for exit status and selected-test counts.

| Label | Gate command | When to use |
|---|---|---|
| V-A | `rtk cargo test -p davinci-agent --lib --offline --locked` | Agent crate boundary or combined changes; during task iteration select its exact new regression/module first |
| V-I | `rtk cargo test -p davinci-ai --lib --offline --locked` | OpenAI provider/cache/stream boundary |
| V-C | `rtk cargo test -p davinci-coding-agent --lib --offline --locked` | Product integration boundary; satisfy existing fixture-binary prerequisites before tests that launch the CLI |
| V-S | `rtk cargo test -p davinci-session --offline --locked` | Session-listing/persistence boundary |
| V-E | `rtk cargo test -p davinci-evals --offline --locked` | Deterministic evaluator and gate boundary |
| V-B | `python -m unittest discover -s scripts/bench/tests -p "test_*.py"` | Benchmark tooling boundary; record actual Python interpreter and platform skips |
| V-ID | `python -m unittest discover -s scripts/bench/tests -p "test_release_identity.py"` | Identity-validator regressions without installation |
| V-F | `rtk cargo test --workspace --offline --locked` | Final integrated scope, not automatically every small change; use established platform/feature shards and honor explicit exclusions |
| V-FMT | `rtk cargo fmt --all -- --check` | Formatting without edits |
| V-LINT | `rtk cargo clippy --workspace --all-targets --offline --locked -- -D warnings` | Final lint plus affected-crate lint during iteration; feature-specific checks use the recorded required features |

Where existing CLI-launching tests require it, build the isolated fixture executable with `rtk cargo build -p davinci-coding-agent --features test-fixtures --offline --locked`. Point the existing fixture runner at that build using its established configuration; verify the selected executable. Do not use a regular installed binary and misclassify fixture-only failures as product defects. Recheck feature names against the current manifest before execution if the source revision changed.

### 17.2 Exact proposed integration targets

Each command below selects the corresponding new fixture target. Individual regression names in the task can be appended as filters for the red/green cycle. Source tests remain alongside their existing owners where private APIs make integration tests inappropriate; use equivalent exact-name filters in that case and record the location.

| Tasks | Target command |
|---|---|
| S01 | `rtk cargo test -p davinci-agent --test harness_web_authority --offline --locked` |
| S02 | `rtk cargo test -p davinci-agent --test harness_delegation_authority --offline --locked` |
| S03–S04 | `rtk cargo test -p davinci-coding-agent --test harness_plugin_state --offline --locked` |
| S05 | `rtk cargo test -p davinci-ai --test harness_stream_cancellation --offline --locked` |
| M01/M03/C05/E02 | `rtk cargo test -p davinci-evals --test harness_accounting --offline --locked` |
| M02 | `rtk cargo test -p davinci-agent --test harness_root_budget --offline --locked` |
| Q01–Q03 | `rtk cargo test -p davinci-agent --test harness_requirement_completion --offline --locked` |
| C01 | `rtk cargo test -p davinci-ai --test harness_openai_capabilities --offline --locked` |
| C02/C03/C05/C07 | `rtk cargo test -p davinci-ai --test harness_openai_prefix --offline --locked` |
| C04/T01 | `rtk cargo test -p davinci-agent --test harness_tool_exposure --offline --locked` |
| C06/C07 | `rtk cargo test -p davinci-ai --test harness_openai_recovery --offline --locked` |
| X01–X02 | `rtk cargo test -p davinci-agent --test harness_context_evidence --offline --locked` |
| X03 | `rtk cargo test -p davinci-session --test harness_session_listing --offline --locked` plus existing context-VM targets |
| T02–T03 | `rtk cargo test -p davinci-agent --test harness_scheduler --offline --locked` plus existing managed-process fixtures |
| A01–A04 | `rtk cargo test -p davinci-agent --test harness_worker_contracts --offline --locked` plus existing graph/worker fixtures |
| P01–P03/R01 | `rtk cargo test -p davinci-evals --test harness_product_acceptance --offline --locked` |
| M00/E01/E03/R02 | Existing exact Python/module tests named in the task, followed by their boundary labels above |

Commands that launch live providers, execute the Codex backend probe, publish artifacts, change accounts, install binaries, or deploy are intentionally not supplied as automatic next steps. Their manifest/authority prerequisites are part of E02/P03 and must be satisfied first.

### 17.3 Review and execution isolation

A default implementation can remain one implementer per connected workstream with independent review at meaningful boundaries. Parallel work is justified only for non-overlapping scopes with frozen interfaces; shared core files such as `turn.rs`, `stream.rs`, and the settings/receipt owners are integration hotspots, not independent work units.

Use a fresh reviewer for authorization, persistence, replay, budget, and release-gate changes. The reviewer receives the deliverable, contract, and raw verification evidence, not just the author's justification. No mandatory multi-agent tournament is required for every task. No model review may substitute for deterministic evidence.

## 18. Requirement coverage and final acceptance

### 18.1 Traceability to the user's goals

| User goal / prior finding | Implementing tasks | Required evidence |
|---|---|---|
| Lower token use without incomplete work | Q01–Q03, X01–X03, T01 | Requirement success plus U/R/W/O and round-trip totals |
| OpenAI-only caching | C01–C07 | Authenticated route matrix, request fixtures, scoped probe/usage evidence |
| Higher useful cache reuse and less uncached work | C02–C05, X01, A04 | Weighted provider reuse and U+W per verified success, including writes |
| Fewer inefficient tool calls | T01–T03, X02 | Expanded operations, model round trips, duplicate reads, actual critical path |
| Better agents/subagents | A01–A04 | Task contracts, root reservations, isolation, combined-artifact success |
| Higher success rate | Q01–Q03, P01–P03, E01–E03 | Independent task and production acceptance, not self-grade |
| Cheaper and faster | M01–M03, E02–E03 | All-attempt cost per success, median/tail time, completeness coverage |
| Production deliverables | P01–P03, S05, R01 | Identified artifact, running journeys, operations and rollback evidence |
| URL permission mismatch, redirects, and wrong relative links | S01 | Canonical destination and per-hop regression fixtures |
| Negated delegation permission | S02 | User-origin restriction/reversal cases across resume and launch |
| Corruption/concurrent plugin state | S03 | Two-process updates and injected persistence failure |
| Plugin re-enable, destructive update, hidden refresh failure | S04 | Disabled-state preservation and last-good installation recovery |
| Stream reader cancellation and slow session listing | S05, X03 | Owned-resource release and bounded header reading |
| Avoid needless architecture and background spend | R02, E03, M02 | Component ablation and retained/disabled/removed table |
| Do not commit the plan | Global constraints and P03/R02 handoff | No staging/commit/push/init as part of plan delivery |

### 18.2 Required final execution record

- [ ] Source snapshot/revision, actual executable hash, build features, effective configuration, platform, and model/route are identified.
- [ ] Every reproduced S-workstream defect has a regression and verified closure; any non-reproduced finding is explicitly reclassified.
- [ ] All attempts, retries, workers, compactions, and enabled background work reconcile or remain explicitly unknown; no hidden free work is assumed.
- [ ] Required task outcomes have final-state evidence; unresolved requirements retain an incomplete or unverified acceptance level.
- [ ] Optional OpenAI fields are emitted only for verified routes; negative Codex probe evidence is not silently overridden.
- [ ] Tool discovery, caching, worker reuse, and replay preserve current permissions and exact conversation ownership.
- [ ] Long sessions, cancellation, process death, stale evidence, concurrent state writes, and combined-writer integration have passing boundary checks.
- [ ] The independent dev/holdout campaign meets predeclared criteria, or its unmet/inconclusive targets are disclosed without a superiority claim.
- [ ] The final running/build artifact is the one tested; production deployment or account changes occur only with separate authorization.
- [ ] A rollback rehearsal and component keep/cut decision exist for promoted changes; historical documentation is reconciled.

### 18.3 Stop conditions and authorization boundaries

Stop the affected execution path on a demonstrated permission breach, lost user state, unowned side effect, unrecoverable journal inconsistency, unknown strict-budget exposure, or missing required acceptance evidence. Preserve other useful completed work and explain the blocked scope; do not treat the whole program as a reason to fabricate a green result.

A supported Codex-only improvement can ship without C07 or another provider. Lack of a paid campaign budget permits offline engineering and an honest unmeasured result, not a claim of measured savings. No fixed cache-hit percentage can be guaranteed across arbitrary task mixes, models, endpoints, or retention conditions.

**First authorized implementation slice:** M00 plus the S regressions and M01 accounting baseline. **First behavioral slice:** Q01/Q02 measured on the dev set with the current model fixed. **First efficiency slice:** C01–C05 and X01, starting from the accepted Codex route. Add worker routing only after those foundations are measured.

## 19. Sources and evidence policy

### Local project sources

Local paths are relative to `C:\Users\sergi\Desktop\davinci-main`. They are evidence references, not a claim that every historical measurement was independently rerun. The plan's new test targets are proposals and are not included in this source list as if they already existed.

| ID | Source | Use in this plan |
|---|---|---|
| L1 | `docs/readiness/production-harness-readiness-plan.md` | Historical small-suite result and original failure rationale |
| L2 | `docs/readiness/README.md` | Implementation versus outstanding acceptance distinction |
| L3 | `docs/cache/openai-cache-contract.md`; `docs/cache/openai-cache-benchmark.md` | Cache identity, disjoint accounting, live authorization, and evidence gates |
| L4 | `docs/cache/codex-backend-probe.md` | Scoped September 26 accepted/rejected request-shape record |
| L5 | `crates/davinci-ai/src/openai_cache_policy.rs`; `responses_request.rs`; `responses_ledger.rs`; `openai_cache_diagnostics.rs` | Existing OpenAI owners and interfaces inspected for task placement |
| L6 | `docs/context-vm.md`; `docs/prompt-engineering.md`; `docs/tool-discovery.md`; `docs/openai-efficiency.md` | Context/projection limits, prompt budgets, tool surfaces, and operator contracts |
| L7 | `crates/davinci-agent/src/permission.rs`; `web.rs`; `delegation.rs`; plugin state/update source named in S01–S04 | Prior source-level audit hypotheses, to reproduce before fixing |
| L8 | `docs/competitor-benchmarks.md`; `scripts/bench/`; `crates/davinci-evals/src/competitor/report.rs` | Existing campaign and claim-gate infrastructure |

### Official OpenAI references checked October 1, 2026

The public references below establish only their documented route/model behavior. Recheck the relevant reference and capability before implementation or live promotion; preserve the checked version/date in campaign evidence. Source addresses are included for the engineer to inspect directly.

| ID | Official reference | Address |
|---|---|---|
| O1 | Prompt caching | `https://developers.openai.com/api/docs/guides/prompt-caching` |
| O2 | Prompt cache diagnostics | `https://developers.openai.com/api/docs/guides/prompt-caching/diagnostics` |
| O3 | Tool search and ordered additional tool definitions | `https://developers.openai.com/api/docs/guides/tools-tool-search` |
| O4 | WebSocket mode and continuation | `https://developers.openai.com/api/docs/guides/websocket-mode` |
| O5 | Compaction and canonical returned context | `https://developers.openai.com/api/docs/guides/compaction` |
| O6 | Reasoning best practices | `https://developers.openai.com/api/docs/guides/reasoning-best-practices` |
| O7 | Codex subagents | `https://learn.chatgpt.com/docs/agent-configuration/subagents` |

O6 supports keeping reasoning-model instructions direct rather than paying for elaborate visible reasoning rituals. Use this as a prompting hypothesis to evaluate, not permission to remove necessary acceptance instructions. Public Codex source (`https://github.com/openai/codex`) remains an implementation reference; the supplied Claude Code repository (`https://github.com/anthropics/claude-code`) remains an external product reference, not a measured arm in this OpenAI-only plan.

### Authorship and implementation status

This document was authored from the conversation's audit, targeted current source/document inspection, and the official references above. It contains planned behavior, test names, file locations, gates, and future verification commands—not implementation code or invented live benchmark evidence. The task checkboxes remain unchecked because implementation has not begun. Saving this plan does not approve its execution or authorize a commit.

### Save-time workspace concurrency note

The final preservation check compared 3,037 existing project files. During plan delivery, `crates/davinci-ai/src/lib.rs`, `crates/davinci-ai/src/stream.rs`, and `crates/davinci-ai/src/stream_reader.rs` changed, and `crates/davinci-ai/src/stream_http.rs` appeared. The planning actions did not write those files; their contents were left untouched. This is an observed concurrent change, not evidence that any audit finding has been fixed.

Before implementing S05, M01, C02, C03, C04, or C06, repeat the relevant source inspection under M00 against a fresh immutable snapshot and map transport ownership to the current files. Reproduce the identified behavior rather than reapplying an obsolete change. All 83 existing source/document references checked before this note resolved; 16 proposed integration-test paths were explicitly excluded from that existence check.
