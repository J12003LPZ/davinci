# DaVinci Simplification and Speed Implementation Plan

> **For agentic workers:** After separate authorization to implement, use `superpowers:executing-plans` task by task. Use subagents only when explicitly selected and justified by independent work. Checkboxes describe future actions, not completed work. This plan authorizes no implementation, staging, commits, pushes, paid requests, installations, or deployment.

**Goal:** Make the existing OpenAI-focused coding harness simpler and faster by eliminating unnecessary model calls, repeated local work, eagerly initialized optional components, and competing implementation paths without weakening correctness, recovery, or user control.

**Architecture:** Keep the current Rust agent loop and existing capability, request, cache, process, verification, and worker owners. Shorten the ordinary execution path first. Do not build a replacement harness, universal cache, workflow framework, optimization service, or new planner to simplify the old ones.

**Tech stack:** Existing Rust workspace, repository-pinned Rust 1.83.0, exact dependency pins, existing Python benchmark tooling, and current fixture infrastructure. Reconfirm the pinned toolchain at execution time; do not upgrade it as part of a speed experiment.

**Spec:** The user's request to remove overengineering and improve speed, with OpenAI-only cache optimization. This is a companion and sequencing correction to `docs/superpowers/plans/2026-10-01-openai-harness-production-plan.md`, not another feature roadmap. Its safety and production-acceptance requirements remain binding; its optional architecture work is deferred as described below.

**Date:** October 1, 2026. **Workspace:** `C:\Users\sergi\Desktop\davinci-main`. **Status:** plan only. **Version control:** do not commit this plan. The original plan remains unchanged.

## Global constraints

- Optimize new inference and provider-cache work for OpenAI only. Keep public Responses and ChatGPT-backed Codex as separate contracts. Preserve other providers' compatibility; do not delete adapters merely because they are unused by this user.
- Never trade away permission checks, current authorization, sandbox boundaries, cancellation, durable mutation records, owned-process cleanup, exact recoverable evidence, or honest completion status.
- Do not change existing user settings, installed plugins, instruction files, credentials, or sessions as a side effect of optimization. Proposed process-policy edits require explicit approval.
- Prefer deleting redundant work to adding memoization. Prefer one request-scoped value to a new persistent cache. Extend existing records before adding new durable state.
- Do not raise timeouts, suppress errors, remove assertions, or stop testing difficult cases to manufacture a speed improvement.
- Preserve documented CLI, RPC, extension, session, and TypeScript-parity contracts. A breaking simplification requires a separate migration decision.
- Tests are deterministic and offline/loopback by default. Live OpenAI experiments require route, model, request ceiling, spend ceiling, and explicit authorization. No paid calls run merely to author this document.
- Preserve all failed attempts and unknown usage in reports. A faster failure, a lower cache ratio, and a smaller codebase are not interchangeable measures of success.
- Do not create a new feature flag for every small deletion. Use existing rollout controls for behavioral experiments; use ordinary version rollback for proven equivalent local changes.
- Do not stage, commit, push, initialize Git, change branches, replace binaries, or deploy while preparing this plan. Implementation and version-control authorization remain separate.

## Review focus

1. A plugin is disabled, changed, or revoked between inspection and execution: unnecessary discovery may be skipped, but execution must still reject the stale approval. Owned by O04.
2. A source file changes externally, including a same-size/same-timestamp replacement: reuse must not certify stale edits or tests. Owned by O05, O06, and O10.
3. A slow read, nested batch, or cancellation races with a pending mutation: preserve ordering, bounds, and complete result accounting. Owned by O08.
4. A feature is disabled or an optional service is unavailable: ordinary startup and unrelated prompts must not initialize it, yet explicit requests must work or report the limitation. Owned by O03 and O09.
5. A smaller prompt or fewer checks makes a task finish early: incomplete requirements, missing tests, and failed attempts must remain visible. Owned by O02, O07, O10, and O12.

---

## 1. Decision: shorten the common path before expanding capabilities

Three possible approaches were considered.

| Approach | Tradeoff | Decision |
|---|---|---|
| Rewrite a minimal harness | Smaller initial implementation, but recreates integrations, invalidation, recovery, and compatibility risk | Reject for this program |
| Add caches, routers, workers, and control layers everywhere | May help individual workloads but adds state, invalidation rules, configuration, and maintenance | Defer unless a trace proves an existing owner cannot solve the problem |
| Remove unnecessary work inside existing owners | Incremental, reviewable, and measurable without replacing the product | Selected |

The default coding path should be: interpret the task in the normal model turn, retrieve focused evidence, perform bounded changes, run appropriate verification, and deliver the result. Separate classifier calls, plan approval machinery, reviewer calls, graph runs, memory reviews, and compactions are conditional, not obligatory stages of every request.

This is not an instruction to remove required planning or independent review from high-risk work. It is an instruction not to apply the most expensive workflow to every task.

OpenAI's latency guidance recommends reducing requests and unnecessary generation, using deterministic operations when suitable, and parallelizing independent work. Treat those as design principles, not promised speedup percentages. [W1]

## 2. Fresh observations and limits

The following were inspected while writing this plan. Source observations identify possible wasted work; they are not profiling results.

| Observation | Current source evidence | Plan decision |
|---|---|---|
| Prepared Context VM access recomputes a fingerprint including messages and session entries before testing the cached revision; a rebuild computes another revision | `prepared_context.rs`, `context_image_revision`, `prepared_context_image` | Reuse a prepared value within one request first. Do not assume this optional path is active on normal sessions. O05 |
| Hook selection calls `hooks_approved` on plugins before filtering their hooks by event; matching is also evaluated later | `plugins/mod.rs`, `hooks_for`, `has_matching_hook`, `run_event_cancellable` | Filter irrelevant event/matcher candidates before expensive approval work; preserve authoritative execution checks. O04 |
| Parallel calls run in capped groups, and the next group waits for the whole previous group | `scheduler.rs`, `run_lanes_with_cancel`, `run_group` | Measure slow-read head-of-line blocking; change only proven parallel-safe regions. O08 |
| Engineering snapshots and bounded single-flight coordination already exist | `engineering_snapshot.rs`; `runtime/cache/singleflight.rs` | Reuse these owners, do not add another repository-index/cache service. O06 |
| Optional package/tmux checks already have a background startup path; the interactive entry point calls it | `startup.rs`, `start_background_checks`; `davinci_interactive.rs` | Verify remaining startup work and lazy activation rather than implementing the same background mechanism again. O03 |
| Session listing now reads the first line through a buffered reader | `davinci-session/src/jsonl_repo.rs`, `JsonlSessionRepo::list` | The earlier whole-transcript read finding is no longer present in this inspected path. Retain regression/measurement only. O01 |
| Learning background review and automatic project/global application already default to false | `native_extensions/learning/config.rs`, `LearningConfig::default` | Preserve this behavior and verify disabled paths do no unnecessary work. Do not claim disabling it is a new speed improvement. O03/O09 |
| The original plan places measure-or-cut after rollout and evaluation work | Original plan, R02 | Move scoped simplification forward. Do not require the entire production program before an independently verified local optimization. |

The original plan's local and attached copies matched SHA-256 `a12a85ba005f31a97400d018def9f6537fae9e8359f989048bf671e5037c71f2` during this review. The project has no `.git` directory at the inspected location. Do not initialize one to make a benchmark identity appear valid; use an authorized source snapshot workflow.

The workspace is live and has changed since the first audit. Old findings are reproduction candidates, not declarations that the same bugs still exist. No builds, tests, profiler sessions, live model calls, or speed measurements were run while authoring this plan.

## 3. Keep, simplify, defer, and remove

| Disposition | Components or behavior | Rule |
|---|---|---|
| Keep | Permission authority, mutation journals, recovery, current-source verification, exact evidence, root budgets | Optimize representation or duplicate computation, not their guarantees |
| Simplify first | Optional startup, hook filtering, prepared-request reuse, repeated scans, redundant model steps, overlapping tool exposure | One measured bottleneck and one responsible owner per change |
| Keep opt-in | Graph/team execution, Context VM, background learning, security watch, optional decision advice | Off means no unnecessary initialization, scan, timer, or model call; preserve explicit user activation |
| Defer | Prewarming, adaptive cache economics, speculative parallel model attempts, autonomous learning promotion, broad scheduler rewrites | Require workload evidence after simpler changes are exhausted |
| Remove when proven redundant | Duplicate internal adapters, unused private modules, equivalent repeated transforms, retired experiment branches and flags | First establish reachability, supported contract, migration, tests, and rollback |
| Preserve as compatibility | Public aliases, legacy session readers, non-OpenAI adapters, documented alternate UI paths | Not runtime bloat merely because unused; consider lazy loading or compile-time exclusion only after separate evidence and approval |

Do not merge the transcript store, local evidence cache, provider KV cache, and mutation journal. They represent different facts and failure guarantees. "One owner per fact" is not "one store for everything."

### Relationship to the previous 37-task plan

- Keep M00 and any unresolved S01–S05 safety work as scoped prerequisites. A speed change that does not touch those boundaries need not wait for unrelated plugin or release work.
- Implement only the missing M01/M03 measurements needed for each candidate. Do not invent another all-purpose telemetry platform or ledger.
- Reuse C01/C02/C04–C06 and X01/X02 where relevant; do not reimplement existing capabilities, request manifests, or invalidation.
- Defer C07 and A04. Do not change model or effort while trying to isolate harness speed effects.
- Split X03 conceptually: local profiling is available now; Context VM promotion still needs its dedicated evidence.
- Treat T02/T03 and A01 as targeted candidates, not permission to replace scheduling and process management wholesale.
- Keep Q01–Q03 and P01–P03 as acceptance constraints. Deduplicate verification work, not verification requirements.
- Move the spirit of R02 to the beginning and apply it per change. E01–E03 remain necessary for model-behavior promotion and competitor claims, not for proving an irrelevant hook does zero digest work.

## 4. Measurement without creating another project

Reuse existing startup trace, run stats, cache counters, and benchmark scripts. Add a counter or span only when it answers a specific decision. Keep raw diagnostics in an external evidence directory, not in model-visible context.

Measure three different outcomes: first usable composer; time from submit to first real provider/tool activity; end-to-end independently verified task completion. A spinner or early frame is not proof that the application is ready, and overlapping spans must not be summed as sequential wall time.

### Workload matrix

| Workload | Key comparison |
|---|---|
| Cold process, ordinary project | Startup tasks, file reads, process launches, first usable composer |
| Warm process, trivial read-only request | Fixed local overhead and unnecessary model/worker calls |
| Small coding task with tests | Round trips, focused retrieval, verification completeness |
| Large repository | Repeated metadata scans, authorization cost, index reuse, ignored paths |
| Plugins: none, many nonmatching, one matching | Approval/digest work only where it is relevant |
| Long conversation: 10, 100, 300 turns | Preparation CPU, hashing/serialization, allocations; Context VM off and opt-in measured separately |
| Parallel region containing one deliberately slow read | Slot utilization, barriers, cancellation, source-ordered results |
| Explicit graph/workflow or optional service | The feature still functions correctly after ordinary-path simplification |

For cheap timing probes, begin with 30 paired local repetitions and retain the full sample. Collect at least 100 observations before using p95 as a release decision metric, and report uncertainty rather than treating that count as statistical proof. Use deterministic counters for unit assertions instead of tight wall-clock limits. Use existing bounded readiness handshakes for concurrency tests.

**Proposed objectives, not forecasts:** reduce measured local startup/preparation overhead by 25% and model round trips on mutually successful small tasks by 20%; investigate a 15% median end-to-end improvement only after representative authorized runs. No correctness regression is allowed. Use a provisional 5% tail-latency and memory non-regression margin, interpreted with measurement uncertainty. A reduction from 4 ms to 3 ms is not a reason for a large rewrite even if it meets a percentage target.

Local implementation changes require equivalent behavior, affected regression coverage, and a demonstrated reduction in relevant work. Prompt, tool-surface, routing, and delegation-policy changes additionally require independent task-quality evaluation. If evidence is inconclusive, retain the existing default; fewer passing tests or fewer completed requirements cannot count as a win.

Track code and operational complexity together: active model stages, initialized services, scans per request, cross-module owners, configuration switches, and state writers. Do not set a lines-deleted quota. Moving code to a different file is not simplification.

---

## 5. Implementation tasks

Tasks below deliberately use current owner names rather than inventing new public APIs. Proposed regression names do not exist merely because they appear here. Place them in the specified existing module's inline tests or the already established integration suite when private visibility requires it.

### O01 — Establish the current baseline and stop redoing completed work

**Depends on:** none. **Files:** `scripts/bench/readiness_metrics.py`, `scripts/bench/tests/test_readiness_metrics.py`, `scripts/bench/campaign.py`, `scripts/release_identity.py`; read `crates/davinci-session/src/jsonl_repo.rs` and existing startup/cache counters.

**Interfaces:** consume existing run identities, timing records, and counters; produce a frozen baseline and a keep/simplify/defer/remove decision table in the existing campaign output. Do not add a runtime registry for this inventory.

- [ ] Recheck current source, feature settings, toolchain, executable identity, and launch path. Mark earlier findings as reproduced, already satisfied, or unverified. Preserve the original plan.
- [ ] Add `simplification_metrics_keep_overlapping_spans_and_unknown_usage`: nested spans do not double-count wall time; missing usage is unknown; worker costs are not omitted.
- [ ] Run that test with the existing benchmark test runner. A new test must exercise its intended assertion; environment or import failure is not a reproduced defect.
- [ ] Capture the workload matrix using the existing fixture/profiling paths and a fresh isolated build during authorized execution. Count real work and call sites before estimating its value.
- [ ] Rank the top three bottlenecks by absolute critical-path cost and repeated operations. Record already-fixed session-listing behavior instead of changing it again.

**Accept when:** each subsequent candidate has an owner, evidence baseline, expected deleted work, and a focused test. **Rollback:** no runtime change is necessary if existing measurement is sufficient.

### O02 — Remove unnecessary model-driven process overhead

**Depends on:** O01. **Files:** `crates/davinci-agent/src/prompt/composer.rs`, `prompt/coding.rs`, `prompt/tool_strategy.rs`, `prompt/verification.rs`, `completion.rs`; review `AGENTS.md` and `CLAUDE.md` without automatically editing them.

**Interfaces:** existing prompt composer and completion state remain authoritative. Ordinary planning is part of the main model response, not a new classifier/planner request.

- [ ] Inventory duplicated instructions, universal tournament/reviewer demands, repeated self-ratings, and always-on planning triggers. Distinguish repository-author requirements from removable harness defaults.
- [ ] Add `simplification_small_task_has_no_mandatory_meta_roundtrip` and `simplification_complex_task_preserves_requirement_gate`: a small fixture has no compulsory classifier/reviewer call, while a multipart mutation cannot bypass its evidence check.
- [ ] Run the affected prompt/completion tests, then remove only duplicated harness prose and unconditional optional stages. Preserve user-requested plans and reviews; propose conflicting instruction-file changes separately.
- [ ] Keep output concise by avoiding repeated plans and logs, not by truncating necessary patches, reasoning budgets, or evidence. Do not automatically lower the chosen OpenAI model or effort.
- [ ] Evaluate the candidate on small and complex tasks with model, effort, tools, and service tier held fixed. Reject gains caused by early stopping or omitted requirements.

**Accept when:** fewer model stages and output repetition improve full-task economics without task-quality loss. **Rollback:** existing prompt-profile promotion path; user instruction files stay unchanged unless separately approved.

### O03 — Make optional capabilities genuinely lazy

**Depends on:** O01. **Files:** `crates/davinci-coding-agent/src/startup.rs`, `davinci_interactive.rs`, `davinci_sources.rs`, `main.rs`, `settings.rs`, `native_extensions/mod.rs`; existing learning/security and MCP/LSP initialization owners only when traces identify their work.

**Interfaces:** retain `start_background_checks`, `WorkspaceDresser`, `load_model_runtime`, and existing feature settings. Do not add a startup daemon or a second lifecycle manager.

- [ ] Classify initialization as mandatory before dispatch, already asynchronous, or optional until selected. Trust, policy, and required recovery stay before the relevant operation.
- [ ] Add `simplification_disabled_optional_features_do_zero_initialization` and `simplification_first_use_initializes_once`: count constructors, scans, timers, process launches, and provider requests rather than testing flags alone.
- [ ] Remove duplicate eager initialization and defer only measured nonessential work. Retain required user-approved startup hooks and their ordering; do not suppress them for a better composer timing.
- [ ] Add `simplification_delayed_discovery_preserves_input_and_selection`: delayed workspace/plugin results cannot erase typed input, select a different tool, or change authorization.
- [ ] Compare startup and first-use latency together. An unchanged total wait merely moved from startup to submit must be reported as such.

**Accept when:** optional-off paths are cheap, required startup remains correct, and first use works or reports an actionable limitation. **Rollback:** restore the original initialization point, not broad feature disabling.

### O04 — Filter irrelevant hooks before approval work

**Depends on:** O01; current plugin correctness checks must pass. **Files:** `crates/davinci-coding-agent/src/plugins/mod.rs`, `plugins/hooks.rs`, `plugins/store.rs` only if an existing read helper needs reuse.

**Interfaces:** preserve `hooks_approved` as the authoritative approval decision. Add internal candidate selection within existing `ActivePlugins` methods; it must not be treated as execution consent.

- [ ] Add `simplification_nonmatching_hook_performs_no_approval_scan`: plugins without the requested event or matcher generate no approval/digest work for that query.
- [ ] Add `simplification_matching_hook_rechecks_revocation` and `simplification_first_hook_mutation_invalidates_second`: disabled, changed, replaced, or revoked plugins do not run from an earlier selection.
- [ ] Run the tests against current ordering, then select event and applicable matcher candidates before checking approval. Preserve hook order and the event-specific matcher rules.
- [ ] Avoid redundant discovery-only checks inside one synchronous query where safe. Do not introduce cross-turn approval caching or remove the final dispatch check. Recheck after an earlier hook may have changed state.
- [ ] Compare approval calls, registry reads, digest traversals, and elapsed time for zero/many nonmatching plugins and matching hooks. Rerun the complete plugin approval/revocation group.

**Accept when:** irrelevant plugins do less work and matching hooks retain current consent checks. **Rollback:** return to original selection order; no storage migration.

### O05 — Prepare once per request before considering incremental hashing

**Depends on:** O01. **Files:** `crates/davinci-agent/src/prepared_context.rs`, `turn.rs`, `provider_budget.rs`, `crates/davinci-ai/src/responses_request.rs`; current callers of prepared context only.

**Interfaces:** preserve `PreparedContextImage`, `prepared_context_image`, `PreparedProviderRequest`, and their permission/budget checks. Consumers within a validated request boundary share the same immutable prepared result.

- [ ] Count actual preparation/fingerprint invocations by mode. Establish whether Context VM preparation is reached in the user's current configuration before calling it a normal-path bottleneck.
- [ ] Add `simplification_one_request_reuses_prepared_image` and `simplification_prepare_budget_change_revalidates`: unchanged consumers share a result, but newly exposed retrieval schemas still trigger required budget validation.
- [ ] Add `simplification_context_reuse_rejects_direct_mutation`: public-field changes, branch changes, permission revocation, model changes, and source-policy revisions must invalidate reused state.
- [ ] Pass the prepared result to size checks, manifests, provider construction, and eligible UI consumers instead of independently rebuilding it. Limit reuse to the boundary where mutation is controlled.
- [ ] Measure 10/100/300-turn workloads. Defer incremental transcript hashing and public-field encapsulation unless full-history hashing remains material after request-local reuse. Never replace strong validation with an untrustworthy generation counter.

**Accept when:** repeated preparation and allocations decline with equivalent provider input and admission behavior. **Rollback:** existing recomputation path; no claim of constant-time access until measured and justified.

### O06 — Eliminate duplicate scans using existing snapshot ownership

**Depends on:** O01. **Files:** `crates/davinci-coding-agent/src/native_extensions/engineering_snapshot.rs`, `repo_intelligence/mod.rs`, `workspace_metadata/mod.rs`, `test_impact/analysis.rs`, `verification_planner/mod.rs`; `crates/davinci-agent/src/runtime/cache/singleflight.rs` only where a measured shared call requires it.

**Interfaces:** reuse `EngineeringSnapshots` and `SingleFlight`. Consumers request existing facts; they do not each create a new index, watcher, global cache, or persisted metadata copy.

- [ ] Trace scans and metadata work initiated by repo, impact, verification, build, and language consumers on one unchanged task. Identify truly identical inputs and authority scope.
- [ ] Add `simplification_shared_facts_compute_once_for_same_revision` and `simplification_fact_reuse_respects_reader_scope`: compatible concurrent consumers share work, but a narrower reader does not receive unauthorized facts.
- [ ] Add `simplification_external_edit_and_ignore_change_invalidate`: source changes, directory additions, ignore/config changes, watcher overflow, and uncertain freshness require revalidation; same-size/same-timestamp replacement must not validate stale mutation or acceptance evidence.
- [ ] Route proven duplicate consumers through the existing snapshot. Prefer affected dependency scopes over full-repository checks only where that narrower scope is sound. Reuse does not grant permission.
- [ ] Measure file reads, stat calls, mutex hold time, and index builds. Do not weaken existing all-path authorization merely because it is expensive. If callbacks or I/O under a lock cause measured contention, narrow the critical section only with a revision-checked publish and concurrency regression.

**Accept when:** unchanged compatible consumers avoid duplicate work while changed or unauthorized state is rejected. **Rollback:** consumers retain their safe recomputation path. This task does not promise adversarial freshness from file timestamps alone.

### O07 — Simplify OpenAI tool and context delivery

**Depends on:** O01, O02; affected authority and replay regressions must pass. **Files:** `crates/davinci-agent/src/tools.rs`, `lib.rs`, `turn_context.rs`, `pruning.rs`; `crates/davinci-ai/src/openai_cache_policy.rs`, `responses_request.rs`, `responses_tools.rs`; `crates/davinci-coding-agent/src/native_extensions/token_governor.rs` and `settings.rs` where already responsible.

**Interfaces:** one existing capability decision, one ordered tool surface, one prepared request, and the current evidence-retrieval contract. Preserve route-specific policy rather than generalizing public API fields to Codex OAuth.

- [ ] Inventory repeated schema construction, duplicate advertised aliases, repeated context injection, and compression immediately followed by retrieval. Count expanded tool operations separately from batch envelopes.
- [ ] Add `simplification_tool_surface_keeps_required_tool_discoverable`, `simplification_prefix_stays_stable_without_policy_change`, and `simplification_revocation_changes_dispatch_immediately`. Do not retain sensitive schemas or old authority for cache stability.
- [ ] Remove redundant descriptions and transforms; reuse immutable definitions when valid. Compare the existing Full and Lean paths instead of creating a third tool router. Preserve compatible aliases at input while testing a smaller advertised surface.
- [ ] Add `simplification_output_digest_retains_failure_and_cursor`: bounded output contains decisive errors and usable recovery instructions. Track total digest-plus-retrieval cost and retain exact evidence.
- [ ] Evaluate OpenAI requests with model, effort, verbosity, service tier, and route fixed. Report ordinary input, cache reads, cache writes where exposed, output, requests, and verified success. Do not promote based on local prefix matches alone.

**Accept when:** fewer round trips or less processing lowers total task cost or latency without tool-discovery or correctness regressions. **Rollback:** existing surface/cache controls. No prewarm calls, keepalive traffic, padding, speculative new endpoint features, or new cache policy engine.

Provider reuse depends on a matching rendered prefix and compatible settings. Preserve useful stable instructions and prior messages, but do not retain useless history just to raise the ratio. Compaction can still be worthwhile when total work falls despite a lower ratio. Model/endpoint-specific controls remain evidence-gated. [W2]

### O08 — Remove measured read-batch stalls without changing mutation order

**Depends on:** O01; ordering and cancellation tests must be current. **Files:** `crates/davinci-agent/src/scheduler.rs`, `batch.rs`, `turn.rs`.

**Interfaces:** preserve `ScheduledCall`, `ToolLane`, source-indexed results, cancellation, and current capacity limits. Keep the scheduler inside the existing runtime; do not migrate the application to another asynchronous framework.

- [ ] Add `simplification_ready_read_reuses_freed_slot` using controlled barriers: with two slots, one blocked read and one completed read, a third independent read starts before the blocked read is released.
- [ ] Add `simplification_serial_barrier_waits_for_all_prior_reads`, `simplification_cancel_starts_no_new_read`, and `simplification_nested_batch_respects_root_capacity`. Results remain in source order and skipped calls are accounted for.
- [ ] Run the new assertions and current scheduler tests. If representative traces show no group-stall cost, defer this task instead of changing scheduling for a synthetic score alone.
- [ ] For a proven bottleneck, refill slots only within the current maximal parallel-safe region. Never cross the next serial barrier. Serial calls, unknown effects, authorization callbacks, journal-owned operations, and shared writers retain required ordering.
- [ ] Measure skewed and homogeneous groups, single-call overhead, threads, and cancellation cleanup. Prefer the smallest bounded queue adjustment; no persistent thread pool unless creation overhead is separately demonstrated.

**Accept when:** relevant skewed workloads have shorter elapsed time, ordinary small batches do not meaningfully regress, and all barrier/cancellation invariants hold. **Rollback:** prior group scheduler. Reduced idle capacity is not a license to run more total work.

### O09 — Keep one lead path and invoke orchestration only when useful

**Depends on:** O01, O02. **Files:** `crates/davinci-agent/src/subagent.rs`, `runtime/team.rs`, `runtime/workflow/limits.rs`, `decision/policy.rs`; `crates/davinci-coding-agent/src/turn_decision.rs`, `native_extensions/graph/config.rs`, `native_extensions/graph/controller.rs` only at observed dispatch boundaries.

**Interfaces:** existing lead agent, worker contract, capacity/budget ownership, and explicit graph/workflow entry points. Do not create a planner above the graph, team, and workflow controllers to choose among them on every task.

- [ ] Trace which paths actually run for an ordinary prompt versus an explicitly requested graph/workflow. Do not assume disabled controllers incur model costs.
- [ ] Add `simplification_plain_prompt_uses_no_optional_controller`, `simplification_explicit_workflow_retains_requested_semantics`, and `simplification_user_delegation_ban_survives_routing`.
- [ ] Remove redundant normal-path advisory or classifier calls where existing deterministic settings suffice. Keep graph and workflow semantics separate where their contracts differ; share only actually identical internal mechanics.
- [ ] For a candidate delegation policy, default small tasks to solo. Start experiments at two workers for independent work, with root budget/capacity and isolated writable resources. Retain explicit user-required review or teams.
- [ ] Compare total startup, duplicate investigation, integration, review, and worker costs. Worker summaries reference exact artifacts; they do not copy the entire transcript back to the lead. Hold OpenAI model/effort routing unchanged in this simplification stage.

**Accept when:** ordinary tasks avoid orchestration overhead and explicitly requested orchestration still works. **Rollback:** existing rollout/default controls. This is not permission to merge all execution engines or remove user-facing features.

### O10 — Deduplicate verification work, not acceptance requirements

**Depends on:** O01; integrate O06 only if those shared-fact changes are adopted. **Files:** `crates/davinci-agent/src/verification.rs`, `command_receipt.rs`, `completion.rs`, `transaction_verification.rs`; `crates/davinci-coding-agent/src/native_extensions/test_impact/analysis.rs`, `verification_planner/rules.rs`, `completion_delivery.rs`.

**Interfaces:** existing verifier-owned receipts and requirement checks. Multiple requirements may reference the same adequate receipt; no second model is needed to restate that a command passed.

- [ ] Inventory repeated identical commands run by the lead, completion hook, reviewer, and finalizer against the same inputs. Classify deterministic checks versus external-state or flaky checks that cannot be safely reused.
- [ ] Add `simplification_unchanged_required_check_uses_valid_receipt`, `simplification_relevant_change_invalidates_receipt`, and `simplification_zero_tests_cannot_cover_requirement`.
- [ ] Reuse a receipt only when source/dependencies, command, environment, fixture setup, tools, and freshness requirements match. If a sound dependency fingerprint is unavailable, rerun the check. A command-string match alone is insufficient.
- [ ] Select affected checks during local iteration. Keep contract, integrated-artifact, failure-path, and release checks where risk requires them. Tests made obsolete by requested behavior need justified updates, not blanket preservation or deletion.
- [ ] Add `simplification_check_reuse_preserves_incomplete_status` and run the final combined-artifact checks. User-demanded fresh runs and deliberate flaky-test repetitions must not be deduplicated away.

**Accept when:** fewer duplicate deterministic checks produce the same or stronger evidence on the final artifact. **Rollback:** rerun rather than reuse questionable evidence. A stale pass is never an optimization.

### O11 — Delete proven redundant code and retire experiment clutter

**Depends on:** O01 and the affected candidate's tests. **Files:** only exact private paths justified by the inventory; configuration cleanup belongs in `crates/davinci-coding-agent/src/settings.rs`; supported documentation stays in its existing guide. No unconditional module deletion is prescribed.

**Interfaces:** preserve public names, persisted formats, plugin entry points, feature combinations, and platform behavior. One retained implementation per truly equivalent operation; a thin compatibility adapter is acceptable.

- [ ] For each deletion, record callers, feature gates, reflection/registration paths, platform builds, persisted readers, and its replacement or reason for no replacement. Zero grep references alone is not proof of dead code.
- [ ] Add or retain boundary tests for every supported behavior served by the candidate. Exercise feature/platform variants through established CI; do not claim cross-platform coverage from a Windows-only run.
- [ ] Delete internal duplicate code together with its now-unused wiring, dependencies, tests that prove only removed internals, and stale documentation. Preserve behavioral regressions on the retained path. Avoid leaving wrappers around an already redundant abstraction.
- [ ] Consolidate obsolete experiment flags only after a measured winner and explicit migration semantics exist. Honor explicit user settings; warn or translate supported legacy values rather than silently ignoring them.
- [ ] Review complexity before/after: active paths, state owners, switches, dependency/build cost, and developer checks. A large-file split is justified by responsibility, not a claimed runtime speedup.

**Accept when:** the retained behavior is easier to trace and test, with no silent compatibility loss. **Rollback:** ordinary version rollback; persistent data is not deleted to simplify code.

### O12 — Verify combined behavior, promote selectively, and stop adding work

**Depends on:** the selected completed tasks, not every possible task above. **Files:** existing `scripts/bench/campaign.py`, `readiness_metrics.py`, their tests, and established prompt/default promotion tooling.

**Interfaces:** same baseline workload IDs, artifact identities, outcome rules, and cost semantics as O01. No new dashboard is required for acceptance.

- [ ] Rerun selected local workloads with fresh candidate and baseline builds. Test the combined changes as well as isolated wins; cache, hook, scheduler, and initialization changes may interact.
- [ ] Add `simplification_report_keeps_failures_and_cold_warm_separation`: report failed, timed-out, aborted, unknown-usage, and cold-start rows instead of selecting only favorable timings.
- [ ] For semantic changes, run the existing independently graded development campaign with explicit OpenAI authorization. Use untouched holdout confirmation before default promotion; local microbenchmarks cannot prove model task quality.
- [ ] Rehearse rollback without losing history, approvals, completed effects, or consumed budget. If no material win is established, leave the candidate off or remove it; do not broaden its implementation to justify sunk effort.
- [ ] Publish a small before/after evidence table: verified success, all-attempt cost per success, median/tail latency, provider requests, expanded operations, local CPU/memory, and complexity removed. State remaining unknowns and stop this program at the smallest successful configuration.

**Accept when:** the selected changes reduce meaningful work, maintain quality/safety, and remain understandable. **Rollback:** tested version/profile rollback. No Codex or Claude Code superiority claim follows from this local simplification exercise alone.

---

## 6. Execution order and stopping rules

Start with O01. Then prioritize O04 and any measured O03 or O05 waste because they can remove local work without a new execution model. O06 follows only where duplicate scans are observed. O02/O07/O09 need model-behavior evaluation, so keep them as controlled candidates while local work proceeds. O08 is conditional on real skewed-read delays. O10 requires sound receipt identity. O11 happens alongside successful changes, not after every optional subsystem has been built. O12 integrates only the winners.

The dependency graph is intentionally small. A plugin-filter optimization does not depend on implementing a new cache policy, private task importer, model router, or deployment system. Conversely, a change crossing an authorization or recovery boundary cannot skip that boundary's regression tests.

Use one implementer for a localized task. Separate reviewers are reserved for meaningful boundaries, especially authority, persistence, concurrency, and final acceptance. Two independent readers can investigate separate bottlenecks, but multiple writers must not modify the same owner in a shared checkout. Do not add a model reviewer after every mechanical line change.

### Stop or defer a candidate when

- Its path is inactive in the target workload or its overhead is below useful measurement resolution.
- The proposed solution introduces more state owners, configuration, or failure modes than the deleted work warrants.
- It requires weakening permissions, freshness, cancellation, evidence durability, or completion requirements.
- It improves a microbenchmark but not relevant absolute latency, total cost, maintainability, or resource use.
- Its apparent gain depends on changing the model, paying for a faster tier, prewarming outside the ledger, or excluding failed tasks.

A cleanup may still be worth shipping solely for maintainability. Label it that way rather than claiming speed. Switching to a paid faster OpenAI service tier is a separate cost/latency product choice, not harness simplification.

## 7. Verification commands and evidence

The following are future verification commands, not commands run for this document. Execute from the project root using the pinned toolchain, isolated build artifacts, existing fixture controls, and disposable configuration. Preserve complete command output and exit codes. Tests must select at least one intended case.

| Scope | Existing verification command |
|---|---|
| Agent changes | `rtk cargo test -p davinci-agent --lib --offline --locked` |
| OpenAI request/provider changes | `rtk cargo test -p davinci-ai --lib --offline --locked` |
| Product/startup/plugin changes | `rtk cargo test -p davinci-coding-agent --lib --offline --locked` |
| Session-listing regression | `rtk cargo test -p davinci-session --offline --locked` |
| Evaluation changes | `rtk cargo test -p davinci-evals --offline --locked` |
| Benchmark Python changes | `python -m unittest discover -s scripts/bench/tests -p 'test_*.py'` |
| Formatting | `rtk cargo fmt --all -- --check` |
| Final combined Rust gate | `rtk cargo test --workspace --offline --locked` |
| Final lint gate | `rtk cargo clippy --workspace --all-targets --offline --locked -- -D warnings` |

During each task, run its named regression or affected module first; do not run the entire workspace after every edit. At final integration, run the established required platform/feature matrix and honor explicit exclusions. CLI-launching tests need their established fixture-enabled binary; never overwrite the user's installed executable or mistake a regular binary for the fixture build. Do not turn an unavailable tool or empty test selection into a pass.

The evidence folder should contain the source/build/configuration identities, baseline/candidate counters and timings, red/green behavioral assertions where a defect is reproduced, independent task outcomes for semantic changes, the keep/cut table, and rollback results. Keep it within the existing campaign structure outside tracked source. No extra orchestration or evidence database is needed.

## 8. Sources and authoring verification

### Local primary evidence

- **[L1] Original implementation plan:** `docs/superpowers/plans/2026-10-01-openai-harness-production-plan.md`, especially R02, C07, X03, and the global constraints. The attached copy and project copy had the hash recorded in Section 2.
- **[L2] Request-local preparation:** `crates/davinci-agent/src/prepared_context.rs`, lines 24–99 when inspected; `context_image_revision` hashes current inputs, and `prepared_context_image` retains an immutable result.
- **[L3] Hook ordering:** `crates/davinci-coding-agent/src/plugins/mod.rs`, `hooks_approved` at approximately lines 43–63 and `hooks_for`/`run_event_cancellable` at approximately lines 238–340. Line numbers are snapshot aids; resolve by symbol on current source.
- **[L4] Read-group scheduling:** `crates/davinci-agent/src/scheduler.rs`, `run_lanes_with_cancel` and `run_group`, approximately lines 124–211.
- **[L5] Existing shared facts:** `crates/davinci-coding-agent/src/native_extensions/engineering_snapshot.rs`, `EngineeringSnapshots`; `crates/davinci-agent/src/runtime/cache/singleflight.rs`, `SingleFlight::run`.
- **[L6] Existing startup behavior:** `crates/davinci-coding-agent/src/startup.rs`, `start_background_checks`; `davinci_interactive.rs` calls it and starts `WorkspaceDresser`; `main.rs` has `load_model_runtime` and `DAVINCI_STARTUP_TRACE`.
- **[L7] Updated session listing:** `crates/davinci-session/src/jsonl_repo.rs`, `JsonlSessionRepo::list`, approximately lines 232–273; first-line buffered reading was present.
- **[L8] Existing opt-in learning:** `crates/davinci-coding-agent/src/native_extensions/learning/config.rs`, `LearningConfig::default`; background review and automatic application default to false.

Paths listed as task owners were checked for existence during plan preparation; not every line of every owner was audited. Private implementation details and performance assumptions require the scoped reinspection in O01. Proposed tests and optimizations remain unimplemented by this document.

### External primary references, checked October 1, 2026

- **[W1] OpenAI, Latency optimization:** `https://developers.openai.com/api/docs/guides/latency-optimization`. Used only for the general principles of fewer requests/generation, appropriate deterministic work, and independent parallelism. No published heuristic percentage is treated as a DaVinci result.
- **[W2] OpenAI, Prompt caching:** `https://developers.openai.com/api/docs/guides/prompt-caching`. Used for matching-prefix/settings behavior and the distinction between cache ratio and total processing cost. Exact model controls must be rechecked for the actual authenticated route during implementation.

**Authoring boundary:** this delivery creates a planning document only. Existing source, configuration, the original plan, and installed binaries are not intentionally modified. No new benchmark result or fixed defect is claimed. The document remains uncommitted.
