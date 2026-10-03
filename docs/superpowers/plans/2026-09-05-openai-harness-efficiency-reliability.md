# OpenAI Harness Efficiency and Reliability Implementation Plan

> **For agentic workers:** Execute this plan task-by-task with `superpowers:executing-plans`. Use `superpowers:test-driven-development` for code changes, `superpowers:systematic-debugging` for failures, and `superpowers:verification-before-completion` before completion claims. Default to one implementing agent. Delegate only a genuinely independent, bounded investigation whose results can be reconciled without overlapping writes. Track execution with the checkboxes below.

**Goal:** Improve verified software-engineering task success and long-running reliability while reducing unnecessary context, model calls, tool work, latency, and compute cost.

**Execution checkpoint:** See [execution evidence and remaining work](2026-09-05-openai-harness-efficiency-reliability-execution.md). The complete plan is not yet implemented or accepted.

**Architecture:** Keep Davinci's provider-neutral agent, existing scheduler, permissions, sessions, native extensions, and runtime. Make the existing OpenAI compatibility profile demonstrably functional, then introduce small, measurable changes at request preparation, execution control, evidence recovery, and evaluation boundaries. Prefer deleting or leaving optional machinery inactive over adding another orchestrator.

**Tech Stack:** Rust 2021; Rust 1.83.0 repository toolchain contract; the existing 13-crate Cargo workspace; exact-pinned dependencies; JSONL sessions; existing SQLite/session infrastructure; existing Responses/SSE/WebSocket adapters; inline Rust tests and offline fixtures.

**Spec:** The user-supplied `Pasted text(4).txt` engineering brief, restated in Section 1, together with the repository contracts in `AGENTS.md`, `CLAUDE.md`, `docs/superpowers/specs/2026-09-03-codex-efficiency-design.md`, and the existing harness-reliability and ecosystem designs. This document is the requested planning deliverable, not permission to silently rewrite those contracts.

**Status:** Plan only. Application source was not changed while preparing this document. No improvement, competitive superiority, or passing application test suite is claimed.

**Inspection date:** September 5, 2026. **Repository:** `C:\Users\sergi\Desktop\pi-rust`. **Observed branch / HEAD:** `main` / `99c55c0`. The inspected working tree already contained 20 changed or untracked entries; it is not equivalent to a clean checkout of that commit. Line references below describe the inspected working tree and must be re-located by symbol before implementation.

## Global Constraints

- Preserve CLI, interactive TUI, print, JSON, and RPC behavior; preserve `.davinci` and legacy `.pi` session/config compatibility.
- Preserve authoritative TypeScript behavior at vendor commit `853a80d26c90a14c1886f0ebb8ffaae133ca2185`; `vendor/davinci/` is reference-only. `packages/` is not an implementation reference.
- Preserve upstream system, compaction, and branch-summary prompt constants verbatim. Optimize composition and optional product behavior, not these protected strings.
- Keep Rust 1.83.0 compatibility and exact `=x.y.z` dependency pinning. Add no external dependency unless an existing dependency or standard-library implementation is demonstrably insufficient.
- Put tests in inline `#[cfg(test)] mod tests` blocks. Fixture data may live under `crates/davinci-parity/fixtures`; do not create a new external `tests/` hierarchy.
- Tests must never invoke live providers or external network services. Use fixture injection and isolated agent/session directories. An independently approved live benchmark is an experiment, not an ordinary test.
- Preserve deny-over-allow permissions, project trust, mutation barriers, graph writer isolation, validated artifact contracts, patch-journal recovery, and cancellation. Tool discovery does not grant execution authority.
- Preserve all pre-existing edits. Do not reset, stash, stage, commit, publish, install packages, read credentials, or modify `plugins/` as part of this planning task.
- Do not claim an optimization exists because its type, flag, comment, or design document exists. Prove the actual request/execution path consumes it.
- Unknown usage, price, capability, or execution outcome is unknown, not zero, supported, free, or successful.

---

## 1. Product contract and decisions

### 1.1 What the requested improvement must accomplish

The brief prioritizes real task success, low token/context consumption, accurate tools, fewer unnecessary model calls, and reliable long-running completion. It explicitly prefers targeted retrieval, stable short instructions, deterministic checks, bounded workflows, explicit state, local recovery, and complexity justified by measurements. It does not require more agents, a larger prompt, a new memory system, or another planning framework.

The acceptance order is: safety and correct work; verified completion; reliable recovery; then efficiency. A fast incorrect patch, a short answer that did not perform the task, or an aborted run with few tokens is not a performance improvement.

The existing Codex-efficiency design makes ChatGPT/Codex OAuth the primary deployment path and public OpenAI Responses the secondary path. Retain that product priority. Validate each backend separately; public API documentation is not proof that an OAuth backend, Azure endpoint, or custom proxy accepts the same request options.

### 1.2 Considered approaches

| Approach | Benefit | Limitation | Decision |
| --- | --- | --- | --- |
| Incremental, measured hardening of existing runtime and provider profile | Reuses working boundaries; isolates improvements and regressions | Requires honest baseline and integration tests | Recommended |
| Prompt-only simplification | Small code change; may remove avoidable instructions | Cannot repair missing wiring, retry multiplication, crash recovery, or weak evaluation | Limited composition experiment after correctness |
| Responses-native rewrite or mandatory multi-agent pipeline | Could expose more provider-specific features | Large regression surface, parity conflict, coordination overhead, unproved benefit | Not in this plan |

### 1.3 Instruction conflicts and their resolution

The brief invites prompt simplification and removal of machinery, while repository instructions protect prompt constants and upstream behavior. Do not silently choose one. Keep protected constants and default compatibility behavior unchanged. Any behavior-changing optimization must be an explicitly selected product profile or a separately approved contract change, with parity tests demonstrating that the legacy path still works.

Repository documentation describes older implementation states in places. Source and fresh test results are authoritative for what is implemented. Historical plans are useful design evidence, not current proof of passing tests or achieved efficiency.

### 1.4 Non-goals

No whole-repository rewrite, unconditional model routing, required learning reviewer, required multi-agent graph, new vector database, replacement session store, removal of non-OpenAI providers, UI redesign, browser-tab continuation mechanism, or promise to bypass provider limits. No live paid benchmark or credential access is authorized by creating this plan.

---

## 2. Architecture assessment

### 2.1 Important execution flow

```text
CLI / TUI / print / JSON / RPC
    -> build_agent: settings, permissions, tools, MCP, sessions, resources
    -> complete_prompt_with_host: host hooks, active tool schemas, prompt,
       ephemeral memory, cached context overhead, runtime subscribers
    -> Agent::run_loop_inner
         cancellation / queued messages / job notices
         -> provider-view pruning -> optional compaction
         -> complete_with_retry -> provider request builder
              -> Codex WebSocket path or HTTP/SSE path
              -> provider-level retry -> streaming decoder
         -> persist assistant -> prepare / run / finalize tools
              permission gate + ledger + read lanes / mutation barriers
         -> persist tool results -> next model turn or terminal event
    -> session/runtime logs, statistics, optional learning and graph feedback
```

Compaction is itself potentially a model call. Nested workers, graph workers, and learning reviews can add work outside the main logical-turn counter. A complete efficiency measurement must include those calls without counting the same call twice.

### 2.2 Existing capabilities to reuse, not rebuild

- `davinci-agent`: scheduler, batch execution, evidence store, pruning, compaction, permissions, tool ledger, subagents, runtime events/context/tasks/workflows, and `RunStats`.
- `davinci-ai`: provider request construction, streaming decoders, retry classifiers, Responses ledger, request-shape hashes, transport implementations, cache settings, and reasoning configuration.
- `davinci-coding-agent`: mode wiring, native/JS extensions, graph execution, token governor, memory, learning, security verification, and resource snapshots.
- `davinci-session` / `davinci-session-sqlite`: authoritative conversation persistence and existing indexing/cache mechanisms.
- `davinci-evals` / `davinci-parity`: reusable harness entry points, metric/report structures, paired comparisons, fixture and parity infrastructure.

The untracked `runtime/capabilities.rs` is already referenced by the dirty agent implementation. Treat it as in-progress work to reconcile, not an absent feature to recreate. A simple-task path should use existing primitives without starting graph/team/learning workflows merely because they are available.

---

## 3. Evidence-backed findings and priorities

Labels distinguish an observed defect from an integration risk or a hypothesis requiring measurement. Relative effort is engineering complexity, not a delivery-time estimate.

| Priority | Finding and evidence | Expected benefit / effort | Required proof |
| --- | --- | --- | --- |
| P0 | **Observed build blocker.** `subagent.rs:354` still uses `tool_class` and `ToolClass`; the dirty diff removed their import at line 11. The offline eval command failed with E0425/E0433. | Restore a trustworthy baseline / small | Reproduce, reconcile imports, rerun affected tests |
| P0 | **Observed evaluation weakness.** `codex_eval.rs:158-186` computes efficiency deltas whenever denominators are positive, not only for mutually successful runs despite its comment. Some corpus cases at `233-260` use output substrings or vague requests. | Prevent optimizing for early failure or persuasive text / medium | False-success, fast-failure, missing-metric and real-patch oracle regressions |
| P0 | **Observed recovery-message hazard.** `pruning.rs:49-52` tells the model to re-run a pruned tool; candidates at `77-83` include all old tool results, not just safe reads. | Avoid repeating mutation commands to recover output / small-medium | Pruned mutation output is recovered without executing the mutation again |
| P1 | **Integration gap indicated by scoped search.** `CodexFeatureFlags` and selected `PI_CODEX_*` names appeared in declarations/re-exports/tests, not a demonstrated product consumer. `CODEX_HOT_TOOLS` appeared as a constant and export. | Make experiments and rollback switches real / medium | Flag-on/off request and execution differences at the actual call boundary |
| P1 | **Observed capability overreach.** `codex_capabilities.rs:59-81` enables broad defaults; `resolve:116-125` accepts any OAuth indication or a URL substring as Codex. | Avoid incompatible options and wrong backend identity / medium | Exact backend classification, unknown-capability fallback, request fixtures |
| P1 | **Observed boundedness gap in inspected foreground loop.** `turn.rs:102-145,311-336` continues while tools/queues exist; no explicit per-run request/tool/deadline limit appears there. `TurnEnded.success` at `305-308` is only non-abort, not verified task success. | Stop runaway work and distinguish outcomes / medium | Budget exhaustion, no-progress, false-completion and resume fixtures |
| P1 | **Observed layered retry/cancellation gap.** Outer retries at `turn.rs:408-539` coexist with HTTP retries at `stream.rs:622-628`; `provider_retry.rs:176-180` sleeps without cancellation and skips sleep under `cfg(test)`. | Bound actual attempts and abort latency / medium | Two-layer exhaustion and cancellation during the real retry helper |
| P1 | **Observed incomplete local metric scope.** `stats.rs:43-77` records turns/retries/tools/wall/pruning, but not a complete attributed task-level token/call ledger. Other usage systems exist and must be reconciled. | Explain real costs, including hidden auxiliary work / medium | Every fixture call counted once; missing usage remains missing |
| P1 | **Observed heuristic budget.** `lib.rs:492-506` uses approximate text/schema accounting; `main.rs:1739-1747` caches tool/identity overhead. | Reduce overflow and unnecessary compaction / medium | Boundary fixtures, estimator-vs-usage error, invalidation on schema changes |
| P2 | **Observed limited discovery behavior.** `tools.rs:408-431` searches MCP names and returns names, not a complete deferred-schema activation protocol. `main.rs:1668-1686` snapshots active schemas before the internal loop. | Shrink tool context without making tools unusable / medium | A discovered tool becomes callable in the next request, within permission scope |
| P2 | **Integration question.** `context.rs:13-28` loads both instruction files; `main.rs:6336` stores them. End-to-end instruction inclusion, ordering, and duplicate handling were not established. | Prevent missing instructions before attempting reductions / small-medium | Sentinel instruction reaches actual request once, with provenance |
| P2 | **Measurement hypotheses.** Extension setup, memory retrieval, reviewer calls, repeated context projections, graph stages, and redundant tool surfaces may cost more than they save on simple tasks. | Remove nonessential work where demonstrated / variable | Component ablations with correctness and total-cost accounting |

Do not turn every hypothesis into a new subsystem. Resolve P0 items first; keep or stop later tasks according to their evidence gates.

---

## 4. Inspection and verification baseline

### 4.1 Command actually run during planning

```text
cargo test --offline --locked -p davinci-evals
```

Environment: `PI_OFFLINE=1`, `DAVINCI_OFFLINE=1`, `PI_DISABLE_NETWORK=1`, `PI_LEARNING_DISABLE_BACKGROUND=1`, `RUSTUP_AUTO_INSTALL=0`.

**Observed result:** exit code 101 during compilation of `davinci-agent`; three unresolved-symbol diagnostics at `crates/davinci-agent/src/subagent.rs:354`. No evaluation tests executed. The removed import in the existing dirty diff explains the compiler diagnostics; no fix was applied during planning.

**Not established:** whole-workspace build health, full test health, real provider behavior, measured token savings, wall-time improvements, task success rates, or performance relative to Codex. Earlier reliability-plan test results are historical and must not be presented as this baseline.

### 4.2 Existing work that implementation must preserve

The 20 pre-existing status entries were:

```text
 M crates/davinci-agent/src/lib.rs
 M crates/davinci-agent/src/mcp.rs
 M crates/davinci-agent/src/permission.rs
 M crates/davinci-agent/src/runtime/mod.rs
 M crates/davinci-agent/src/runtime/workflow/executor.rs
 M crates/davinci-agent/src/runtime/workflow/mod.rs
 M crates/davinci-agent/src/runtime/workflow/validate.rs
 M crates/davinci-agent/src/subagent.rs
 M crates/davinci-coding-agent/src/extension_host.rs
 M crates/davinci-coding-agent/src/main.rs
 M crates/davinci-coding-agent/src/native_extensions/ecosystem/cache_affinity.rs
 M crates/davinci-tui/src/davinci/theme.rs
 M crates/davinci-tui/src/davinci/ui.rs
 M crates/davinci-tui/src/davinci/views/chrome.rs
 M crates/davinci-tui/src/davinci/views/sheet.rs
 M crates/davinci-tui/src/davinci/views/startup.rs
 M crates/davinci-tui/src/davinci/views/transcript.rs
 M docs/ui/design.md
?? crates/davinci-agent/src/runtime/capabilities.rs
?? plugins/
```

Refresh this list before execution. Do not assume a clean worktree at `99c55c0` contains the runtime capability changes. A benchmark baseline must identify its full intended tree, including required untracked source, without copying credentials, user sessions, or unrelated plugins.

---

## 5. Measurement contract

### 5.1 Separate the units

| Metric | Definition |
| --- | --- |
| Verified task outcome | Independent oracle result: verified success, verified failure, blocked, aborted, budget exhausted, infrastructure failure, or unverified completion |
| Logical model calls | Requests for new model decisions, tagged foreground, compaction, worker, graph worker, or learning review |
| Provider attempts | Actual wire attempts, including transport/provider/outer retries and recovery; exclude a cancelled backoff that never sent a request |
| Total tokens | Sum of reported input plus output tokens across all attributed calls; retain provider usage provenance |
| Cached input | Subset of input, not an additional token count; retain cache-write counters separately when supplied |
| Reasoning output | Subset of output where provider reports it; do not add it to output again |
| Tool calls | Both model-visible calls and semantic operations, including batch children and delegated work |
| Latency | End-to-end task wall time and separately model wait, tool critical path, setup, retrieval, compaction, verification, retry wait, and optional background work |
| Context | Final request estimate and actual reported input where comparable; stable instructions, schemas, dynamic material, and peak separately |
| Cost | Priced from versioned, backend-appropriate rates only when available; missing or subscription usage is not automatically zero-dollar API cost |
| Reliability | Retry causes, repeated-action events, overflow events, duplicate side effects, cancelled work, and terminal reason |

For accounting validation, a fixture with input=100, cached=60, output=40, reasoning=25 has 140 total tokens, not 225. Cache-write pricing must follow that backend's current metering contract, not an assumed universal formula. A smaller WebSocket payload is a transport result, not proof of fewer billed context tokens.

### 5.2 Proposed release thresholds, not measured results

Safety gates: zero unauthorized mutations, zero duplicate side effects in the recovery suite, no lost accepted changes, and no skipped required verification. Critical fixture cases must all pass.

Performance gate: compare the same tasks, complete datasets, backend/model/effort, permissions, tools, context policy, hardware, and budgets. Report all failures. Among mutually successful pairs, initially require at least two of total tokens, model calls, and task wall time to improve by at least 5% in the median, with no greater than 10% median regression in any required efficiency metric or semantic tool operations. These are proposed engineering thresholds to register before measurement, not claimed improvements or statistically established constants.

For live results, report uncertainty and the sample size. Do not promote solely on tiny medians or a degenerate confidence interval. Require no observed verified-success regression in the pilot and no critical task-class regression; inconclusive evidence keeps the optimization opt-in. Success-conditioned efficiency must be accompanied by completion rates and all-run resource costs, so failures cannot disappear from the report.

---

## 6. File responsibilities and delivery sequence

Use existing modules unless the task below explicitly proposes a focused new file. The two main additions are a small run-control module and a product request-profile module; neither is a new agent framework.

| Area | Existing files to modify | Proposed additions |
| --- | --- | --- |
| Baseline and child scoping | `crates/davinci-agent/src/subagent.rs`, inline tests | None |
| Verified measurements | `crates/davinci-evals/src/{codex_eval,lib,reporter,summary}.rs` | Inline tests and bounded fixture corpus |
| Call accounting | `crates/davinci-agent/src/{stats,turn}.rs`; `crates/davinci-ai/src/{codex_telemetry,stream,provider_retry}.rs`; product `main.rs` | None unless telemetry inspection proves an existing boundary cannot represent attribution |
| Provider profile | `crates/davinci-ai/src/{codex_capabilities,codex_flags,request_shape,stream,responses_ledger,stream_decoder,codex,codex_transport,codex_ws}.rs`; product `main.rs` | `crates/davinci-coding-agent/src/request_profile.rs` |
| Run limits | `crates/davinci-agent/src/{lib,turn,stats}.rs`; product `settings.rs`, `main.rs` | `crates/davinci-agent/src/run_control.rs` |
| Context and evidence | `crates/davinci-agent/src/{lib,pruning,evidence,compaction,context}.rs`; product `main.rs`, `extension_host.rs` | None |
| Checkpoint/recovery | `crates/davinci-agent/src/tool_ledger.rs`, existing `runtime/{tasks,context,events}.rs`; session and product wiring | Reuse session custom entries/runtime log, not a second history store |
| Tool discovery | agent `tools.rs`, `mcp.rs`, `runtime/capabilities.rs`; product `request_profile.rs` | None |
| Comparative runner | `crates/davinci-evals/src/{lib,codex_eval,artifacts}.rs`, its `Cargo.toml` | `crates/davinci-evals/src/bin/harness_eval.rs`; fixtures under `crates/davinci-parity/fixtures/openai-harness/` |

**Order:** T0 -> T1 -> T2 -> T3. Then T4/T5/T6, followed by T7/T8/T9/T10, and T11. Keep writes serial where files overlap. T4 can be reviewed independently from T6, but parallel implementation is not required. T8/T9/T10 are conditional experiments, not prerequisites for shipping proven P0/P1 repairs.

Each implementation task ends with a bounded diff review, named verification evidence, and a recorded outcome. Use a scoped commit only when the owner has authorized committing; never use `git add .` on this working tree.

---

## 7. Task-by-task implementation plan

### T0 — Restore a reproducible, non-destructive baseline

**Files:** `crates/davinci-agent/src/subagent.rs`; its existing inline tests. Read the current dirty changes in `runtime/capabilities.rs` and the scoping callers before editing.

**Consumes:** Current working tree and repository instructions. **Produces:** A compiler-valid intended baseline, preserved user edits, and an exact verification ledger; not an efficiency claim.

- [ ] Re-read `AGENTS.md`, `CLAUDE.md`, current status, and the relevant diff. Record full HEAD and hashes of changed source; identify any concurrent edits.
- [x] Reproduce the compilation failure with the command in Section 4. Do not suppress diagnostics or skip the failed dependency.
- [x] When the inspected code is unchanged, restore the removed import while preserving the new registry-based scoping implementation:

```rust
use crate::permission::{tool_class, PermissionMode, ToolClass};
```

- [x] Keep the existing read-only guard at line 354. Removing the guard to make compilation pass is forbidden. A later registry-consistency change must cover this guard and child scoping together.
- [x] Run the affected tests and evaluate the next actual failure, rather than assuming the import fixes the whole workspace:

```text
cargo test --offline --locked -p davinci-agent subagent
cargo test --offline --locked -p davinci-evals
cargo check --offline --locked -p davinci-coding-agent
```

Expected: the three unresolved-symbol errors disappear. Passing tests/checks must be recorded separately; any new blocker is a baseline issue until proven introduced by this work.

- [ ] Review the exact diff and establish a baseline tree identifier. An isolated execution worktree must contain the intended in-progress changes by an owner-approved transfer; a clean HEAD-only worktree is not an equivalent baseline.

**Exit gate:** Relevant code compiles, child permission tests run, and remaining pre-existing failures are explicit. **Rollback:** Undo only this task's exact edit using the inspected content/hash; never restore the entire dirty file to HEAD.

### T1 — Make correctness and release gates resistant to false success

**Files:** `crates/davinci-evals/src/codex_eval.rs`, `lib.rs`, `summary.rs`; inline tests. Preserve existing reporting consumers while adding versioned fields where necessary.

**Consumes:** T0 baseline and existing `PairedTaskComparison`. **Produces:** Independent outcome evidence, explicit metric eligibility, and gates that cannot reward failed-fast runs.

- [x] Add a failing regression in `codex_eval.rs` using existing public structures:

```rust
#[test]
fn failed_fast_runs_cannot_pass_efficiency_gate() {
    let run = |success, wall, calls, tokens| CodexBenchmarkRunMetrics {
        success,
        wall_time_ms: wall,
        model_responses: calls,
        tool_calls: calls,
        uncached_input_tokens: tokens,
        ..Default::default()
    };
    let pairs = vec![PairedTaskComparison {
        task_id: "both_failed".into(),
        generic_metrics: run(false, 1_000, 10, 1_000),
        optimized_metrics: run(false, 100, 1, 100),
        external_cli_metrics: None,
    }];
    assert!(!evaluate_release_gate(&pairs).meets_release_gate);
}
```

- [x] Run `cargo test --offline --locked -p davinci-evals failed_fast_runs_cannot_pass_efficiency_gate`; confirm it rejects the old behavior for the intended reason.
- [ ] Modify efficiency eligibility to require both outcomes verified successful, retain all outcomes in the success-rate denominator, and refuse a pass when there are no eligible pairs. Do not silently convert absent metrics to zero.

```rust
let both_successful = c.generic_metrics.success && c.optimized_metrics.success;
if !both_successful {
    continue; // after recording both outcomes and duplicate-side-effect counts
}
```

- [ ] Replace the ambiguous `success` input at the runner boundary with oracle-derived status. Preserve the legacy boolean in serialized compatibility output only as a derived value. Proposed statuses are `verified_success`, `verified_failure`, `blocked`, `aborted`, `budget_exhausted`, `infrastructure_failure`, and `unverified_completion`.
- [ ] Add inline regressions for empty dataset, both failed, baseline-only success, candidate-only success, missing usage, zero denominator, non-finite deltas, tool-call explosion, changed forbidden file, oracle command not executed, and assistant text claiming success after a failing oracle.
- [x] Require exact expected-case coverage and schema-version compatibility in the benchmark report gate. A subset of easy tasks may produce a report but not a release pass.
- [ ] Keep the existing 10% tool-call no-worsening guard. Add total-token and semantic-operation fields through T2; do not substitute uncached input alone for overall token cost.
- [x] Run `cargo test --offline --locked -p davinci-evals`; update affected report fixtures and review backward compatibility.

**Exit gate:** No text-only or failed-fast artifact passes a correctness gate. **Rollback:** Retain accurate raw measurements and mark promotion unavailable; never restore a permissive gate just to obtain green performance reports.

### T2 — Attribute all calls, attempts, tokens, and outcomes once

**Files:** Agent `stats.rs`, `turn.rs`; AI `codex_telemetry.rs`, `stream.rs`, `provider_retry.rs`; product `main.rs`; eval `codex_eval.rs`, `reporter.rs`. Inspect existing usage events before adding fields.

**Consumes:** T1 outcome schema, existing `RunStats`, provider usage, runtime identifiers and graph resource snapshots. **Produces:** One task-scoped accounting view with provenance; compact UI/RPC summaries remain projections of it.

- [ ] Define a versioned measurement record in the existing telemetry layer with `task_id`, `logical_call_id`, `attempt_id`, purpose, provider/backend/model, configuration fingerprint, start/end, retry category, and optional usage. Use existing IDs where semantically equivalent; do not generate unrelated IDs in each layer.
- [ ] Make purposes explicit: foreground, compaction, nested worker, graph worker, learning review. Wire a call-start/call-end pair at each actual provider boundary, and tag wire retries with the same logical call ID but distinct attempt IDs.
- [ ] Add this pure accounting test to the telemetry module; the helper below is the complete proposed subset rule:

```rust
fn reported_total(input: Option<u64>, output: Option<u64>) -> Option<u64> {
    input?.checked_add(output?)
}

#[test]
fn totals_do_not_double_count_token_subsets() {
    let input = Some(100);
    let output = Some(40);
    let cached_input = 60;
    let reasoning_output = 25;
    assert!(cached_input <= input.unwrap());
    assert!(reasoning_output <= output.unwrap());
    assert_eq!(reported_total(input, output), Some(140));
    assert_eq!(reported_total(None, output), None);
}
```

- [ ] Add fixture events representing one foreground request with two failed wire attempts then success, one compaction call, and one worker call. Assert three logical calls and five wire attempts, no duplicate event accounting, and explicit unknown usage for unreported failed attempts.
- [ ] Record estimate-vs-reported-input pairs only when the same request shape is being compared. Keep estimated input distinct from actual usage. Validate subset counters against totals; invalid usage is a diagnostic, not silently saturated data.
- [ ] Count batch children and worker tool operations separately from direct model tool calls. Attribute concurrent wall time without summing overlapping work into end-to-end wall time.
- [ ] Keep telemetry enabled in both A/B profiles even when optimization flags are disabled. Redact prompts, file bodies, arguments, credentials, and opaque reasoning from default telemetry; store full test evidence only in explicitly controlled fixture artifacts.
- [ ] Add `#[serde(default)]` or versioned optional fields for old records; do not rewrite historical session JSONL files. Replaying a runtime log must not recount provider usage already aggregated.
- [ ] Run the targeted telemetry/stats/reporter tests, then `cargo test --offline --locked -p davinci-evals`. Verify fixture logs with missing usage remain incomplete rather than appearing free.

**Exit gate:** Every actual call in the fixture trajectory is attributable exactly once; unreported usage stays unknown. **Rollback:** Disable new presentation fields if necessary, but keep measurement collection stable for both profiles.

### T3 — Wire a conservative, testable OpenAI request profile

**Files:** AI `codex_capabilities.rs`, `codex_flags.rs`, `request_shape.rs`, `stream.rs`; new product `request_profile.rs` declared from `main.rs`; inline tests. Trace actual consumers before modifying transport modules.

**Consumes:** Existing `CodexFeatureFlags`, backend/model metadata, active tool specifications, permission scope, and T2 telemetry. **Produces:** One resolved effective profile per request lineage, with auditable applied flags and capability decisions.

- [ ] Add offline capability regressions for non-Codex OAuth, a deceptive hostname, a custom proxy, unknown model metadata, Azure isolation, and explicit unsupported features. This concrete regression uses the existing test helper:

```rust
#[test]
fn generic_oauth_does_not_imply_codex() {
    let mut model = test_model("anthropic-messages");
    model.provider = "anthropic".into();
    let caps = CodexCapabilities::resolve(
        &model, Some("https://api.anthropic.com"), true,
    );
    assert!(!caps.generate_false_prewarm);
    assert!(!caps.turn_state_headers);
    assert!(!caps.custom_grammar_tools);
}
```

- [ ] Run `cargo test --offline --locked -p davinci-ai generic_oauth_does_not_imply_codex` and establish the red result before changing classification.
- [ ] Replace substring identity with parsed endpoint origin plus explicit provider/API-family metadata. Generic OAuth is an authentication method, not a backend capability. Unknown/custom origins receive conservative capabilities unless trusted explicit configuration supplies a supported profile; never probe with a generating request.
- [ ] Resolve each effective optimization as `requested && supported`, while keeping safety invariants mandatory. Request-shape changes create a new lineage. Persist the resolved profile fingerprint, not credentials.

```rust
pub(crate) fn effective_flag(requested: bool, supported: bool) -> bool {
    requested && supported
}

#[test]
fn an_opt_in_cannot_invent_backend_support() {
    assert!(!effective_flag(true, false));
    assert!(!effective_flag(false, true));
    assert!(effective_flag(true, true));
}
```

- [ ] Inventory every `CodexFeatureFlags` field and prove its consumer with a request/transport fixture. A switch with no consumer is reported as inactive until wired or removed from advertised settings. Never use an unwired switch to label a run optimized.
- [ ] Define `request_profile.rs` as the product boundary that selects effective flags and advertised tools; keep provider wire types in `davinci-ai`. Construct the profile once for stable configuration and rebuild it on model, backend, permissions, instructions, or schema changes.
- [ ] Keep parity defaults for existing behavior until explicit profile approval. A safety correction may be unconditional; a new experimental capability may not be silently enabled for all OAuth users.
- [ ] Test selected flags through `live_complete_streaming_with_sink`'s request-building/transport seam, not only with constructor assertions. A disabled optimization must have an observable safe fallback; ledger safety cannot be disabled for real mutating recovery merely to make a baseline simpler.
- [ ] Run `cargo test --offline --locked -p davinci-ai codex_capabilities`, profile-module tests, and product compilation. Record which capabilities are fixture-validated versus independently backend-validated.

**Exit gate:** Exact backend classification and effective flags match actual request behavior. **Rollback:** Select the tested compatibility profile; never discard acknowledged tool results or broaden permissions during fallback.

### T4 — Bound the foreground loop and distinguish termination from success

**Files:** New `crates/davinci-agent/src/run_control.rs`; agent `lib.rs`, `turn.rs`, `stats.rs`; product `settings.rs`, `main.rs`; inline tests. Reuse runtime cancellation rather than adding a second cancellation tree.

**Consumes:** T2 logical/wire counters and current cancellation; T3 effective profile. **Produces:** Deterministic run admission and terminal reasons, with observable budget exhaustion.

- [ ] Add a pure budget helper with the following complete core behavior. Keep the caller responsible for incrementing counters at their authoritative boundaries:

```rust
#[derive(Debug, Clone, Copy)]
pub(crate) struct RunLimits {
    pub model_calls: u64,
    pub wire_attempts: u64,
    pub tool_operations: u64,
    pub wall_ms: u64,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct RunUsage {
    pub model_calls: u64,
    pub wire_attempts: u64,
    pub tool_operations: u64,
    pub wall_ms: u64,
}

pub(crate) fn exhausted(limits: RunLimits, used: RunUsage) -> Option<&'static str> {
    if used.wall_ms >= limits.wall_ms { return Some("deadline"); }
    if used.wire_attempts >= limits.wire_attempts { return Some("wire_attempts"); }
    if used.model_calls >= limits.model_calls { return Some("model_calls"); }
    if used.tool_operations >= limits.tool_operations { return Some("tool_operations"); }
    None
}

#[test]
fn admission_stops_at_the_limit() {
    let limits = RunLimits { model_calls: 2, wire_attempts: 3,
        tool_operations: 10, wall_ms: 1_000 };
    let used = RunUsage { model_calls: 2, ..Default::default() };
    assert_eq!(exhausted(limits, used), Some("model_calls"));
}
```

- [ ] Add a scripted `Agent::run_loop` fixture that always requests the same safe read and confirm the old loop lacks the requested configured stopping behavior. Bound the test's completion closure itself so a regression cannot hang the test process.
- [ ] Install admission checks before a new model decision, before each wire attempt, and before starting each new semantic tool operation. A started operation still receives its terminal result; budget exhaustion must not leave a dangling call/result pair.
- [ ] Keep budgets task-scoped, not silently reset on retries, compaction, child workers, or resume. Reserve child budget before launch and reconcile reported usage once. Use one shared atomic admission mechanism for concurrent work; check-then-increment without reservation is insufficient.
- [ ] Initial opt-in evaluation profile: 64 logical model calls, 80 wire attempts, 256 semantic tool operations, and a 30-minute task deadline. These are configurable experiment safeguards, not model context limits or promised completion times. Keep generic/library compatibility behavior unless explicit configuration enables bounded product runs.
- [ ] Detect repeated no-progress actions with normalized tool+arguments, outcome, and authoritative evidence/version. Never suppress a new read just because Git status is unchanged. Unknown freshness disables result reuse. Repeated same failure can emit one compact correction; a third unchanged failure ends as `no_progress`, not success. Job polling and user-requested repetition are explicitly exempt and remain deadline-bound.
- [ ] Map provider failure, abort, budget exhaustion, no progress, permission block, and unverified normal completion separately. A normal assistant stop can be `unverified_completion`; only an independent task oracle or required verification contract produces `verified_success`.
- [ ] Test cancellation with queued messages, a pending tool batch, a running child, exhausted budget, and a long legitimate sequence with changing evidence. Assert `is_streaming` clears and owned process cleanup completes on every terminal path.
- [ ] Run run-control and loop tests, then targeted permission/scheduler tests. Check RPC/JSON compatibility and ensure no model call is used to decide whether a numeric budget is exhausted.

**Exit gate:** Configured runs stop for explicit reasons without losing completed work or claiming unverified success. **Rollback:** Keep terminal reason reporting; turn off the experimental limit profile rather than removing cancellation or permission checks.

### T5 — Make retries globally bounded, cancellable, and failure-specific

**Files:** AI `provider_retry.rs`, `stream.rs`, relevant `codex.rs` / `codex_ws.rs` transport recovery; agent `turn.rs`; existing inline tests.

**Depends on:** T2 and T4. **Produces:** A shared attempt budget and cancellation-aware provider retry path, without repeating acknowledged mutations.

- [ ] Characterize current retry semantics before changing them. `turn.rs` currently uses `max_retries.max(1)` as its number of attempts; the lower provider helper has a different retries setting. Preserve public-setting compatibility until the intended upstream semantics are explicitly tested.
- [ ] Add a cancellation-aware variant of the provider retry helper that receives cancellation, a shared wire-attempt reservation, and an injectable wait function. Keep the old public helper as a compatibility wrapper where necessary. The live streaming callers must use the controlled variant.
- [ ] Replace an uninterrupted provider backoff with bounded waits that check the existing abort signal and task deadline. Inject a fake clock/wait in unit tests, but test the same cancellation decision path as production; `cfg(test)` must not simply skip the relevant logic.
- [ ] Reserve the wire attempt immediately before sending. With a global cap of three, nested outer/provider retries together may send at most three requests, not three times three. A cancelled backoff sends no additional request and does not increment the attempt count.
- [ ] Classify failures using structured status/code/header information first, with conservative text fallback only where the adapter exposes nothing better:

| Failure class | Action |
| --- | --- |
| Invalid request, unsupported parameter, denied permission | Fail with actionable detail; no identical blind retry |
| Authentication expired | At most the existing authorized refresh path; no repeated login or credential acquisition |
| Transient transport/timeout/eligible server error | Retry the smallest request within the shared budget, respecting bounded Retry-After |
| Rate limit with temporary backoff | Bounded wait; honor abort/deadline; do not evade account limits |
| Exhausted quota or nonrecoverable account limit | Block or stop; do not treat every 429 as transient |
| Context overflow before an accepted response | At most one admissible local context repair and retry; count compaction and retry work |
| Missing continuation state | Rebuild from acknowledged canonical state; do not re-execute completed tools |
| Unknown mutation outcome after interruption | Reconcile evidence or stop for operator action; never blindly repeat |

- [ ] Add named tests: `nested_retries_share_one_attempt_budget`, `cancel_during_provider_backoff_sends_no_request`, `deadline_truncates_retry_after`, `permanent_failure_is_not_retried`, `failed_attempts_keep_usage_unknown`, and `recovery_does_not_repeat_acknowledged_tool`. These names are proposed tests, not tests claimed to exist today.
- [ ] Cover partial streamed output, malformed terminal events, abort after a tool call was emitted, and fallback after the backend may already have accepted a request. Do not restart a whole agent trajectory for a local provider error.
- [ ] Run `cargo test --offline --locked -p davinci-ai provider_retry`, provider recovery selectors, and agent retry selectors. Read actual test counts; a selector matching zero tests is not verification.

**Exit gate:** Actual wire attempts obey one cap, abort stops backoff promptly, and recovery does not replay acknowledged effects. **Rollback:** Retain global attempt admission and safety classifications; use the simpler tested transport path if an optimization fails.

### T6 — Reduce context safely, with recoverable evidence and final-request admission

**Files:** Agent `pruning.rs`, `evidence.rs`, `lib.rs`, `compaction.rs`; product `main.rs`, `extension_host.rs`; token-governor integration tests.

**Depends on:** T2, T4. **Produces:** Safe output recovery, calibrated budget reporting, and bounded context repair while preserving authoritative history.

- [ ] Add a red fixture containing an old mutating shell result and enough later tool results to make it prunable. Assert that the provider-visible replacement does not instruct the model to execute the mutation again.
- [ ] Materialize selected historical output into the existing evidence store or resolve an existing durable evidence reference before dropping its provider-visible body. The hint must contain a usable reference in `content`, not solely in `details`, because details do not necessarily reach the model.
- [ ] Keep call ID, tool name, outcome, byte/line metadata, and evidence provenance with the reference. A read of that evidence must recover the original result, not execute the original tool. Do not add another retrieval tool when bounded `read` or existing `retrieve_output` can satisfy the contract.
- [ ] Verify that the current permission/tool scope permits reading that particular session-owned artifact. Do not grant broad outside-root reads to make recovery convenient. When evidence storage or authorized retrieval is unavailable, retain the needed result or stop context reduction with an explicit diagnostic; never advertise an imaginary path.
- [ ] Reuse deduplicated references for the same stored result. Bound disk growth and apply session-aware retention: `EvidenceStore` currently has a seven-day sweep, while governor retention differs. Active/resumable checkpoints must not silently depend on already-swept files. Missing evidence is an explicit recovery failure, not permission to repeat a mutation.
- [ ] Retain lossless transcript/session content and all call/result pair identities. Old provider-view bodies may shrink; accepted edits and verification evidence may not disappear from the task checkpoint.
- [ ] Compute admission against the actual next request configuration after active-tool changes and dynamic context assembly. Use:

```text
available_input = model_context_limit - output_reserve - safety_margin
admit only when estimated_final_input <= available_input
```

`output_reserve` includes the provider's generated-output budget, including reasoning where applicable; do not count the same reasoning allowance twice. Model metadata and actual runtime settings determine the limit, not repository line count or a universal 200k threshold. Reject invalid/nonpositive configured limits rather than wrapping arithmetic.

- [ ] Keep byte-based estimates explicitly approximate. Collect model/request-shape-specific estimate error from T2; calibrate only with comparable observed requests. An estimate is never advertised as an exact tokenizer or guaranteed upper bound.
- [ ] Invalidate cached overhead on actual schema, instruction, model, or identity changes. Do not repeatedly serialize the whole catalog when its generation is unchanged. Test UTF-8, long identifiers, JSON-heavy tools, images, empty context, and a system/schema prefix that alone cannot fit.
- [ ] Prune before invoking a summarizer. After pruning or compaction, re-estimate the final request. If still inadmissible, allow only the explicitly bounded recovery path, then stop as `context_exhausted`; do not call compaction indefinitely.
- [ ] Preserve the existing `native_context_pruned()` freshness reset. Test that a legitimate re-read after pruning is not hidden by the governor, and that unknown file freshness executes a fresh read/search instead of a weak Git-state cache hit.
- [ ] Test `pruned_mutation_output_uses_evidence`, `evidence_write_failure_keeps_recoverable_context`, `expired_evidence_is_not_a_rerun_instruction`, `schema_change_invalidates_overhead`, and `oversized_fixed_prefix_stops_without_summary_loop`.
- [ ] Run pruning/evidence/context selectors and the existing governor recovery suite. Record provider-visible byte reduction on the deterministic fixture separately from any unmeasured live token saving.

**Exit gate:** Context reduction preserves recoverability and cannot direct duplicate mutations; inadmissible requests stop or receive one bounded repair. **Rollback:** Keep safe hints and full history; disable aggressive pruning thresholds or calibration when uncertain.

### T7 — Preserve long-running task state across compaction, restart, and transport recovery

**Files:** Agent `tool_ledger.rs`, `compaction.rs`, existing `runtime/tasks.rs`, `runtime/context.rs`, `runtime/events.rs`; AI `responses_ledger.rs`, `stream_decoder.rs`, `codex.rs` / `codex_ws.rs`; existing session custom-entry and runtime-log wiring in product `main.rs`.

**Depends on:** T3–T6. **Produces:** A compact task checkpoint derived from existing state, plus provider-item fidelity tests; not a second conversation database.

- [ ] First trace existing ledger/session save and restore consumers. Serialization derives alone do not establish durability. Identify which state is durable, reconstructable, transient, or currently lost before deciding which fields to add.
- [ ] Define a versioned session custom entry for the minimum missing checkpoint data. Reuse existing task IDs and records. The following is a proposed schema contract, not an existing API:

```json
{
  "type": "harness_checkpoint",
  "version": 1,
  "task_id": "fixture-task-01",
  "status": "in_progress",
  "objective": "Fix the parser and verify its regression tests",
  "constraints": ["Do not modify public API"],
  "baseline_tree": "fixture-tree-hash",
  "owned_changes": [{"path": "src/parser.rs", "content_hash": "fixture-hash"}],
  "completed_steps": ["locate_parser"],
  "next_safe_action": {"kind": "run_verification", "argv": ["cargo", "test", "--offline"]},
  "verification": {"state": "not_run", "evidence_refs": []},
  "provider_lineage": "fixture-lineage",
  "last_acknowledged_call_id": "fixture-call-03",
  "pending_effects": [],
  "remaining_budget": {"model_calls": 12, "wire_attempts": 16, "tool_operations": 40}
}
```

- [ ] Bound inline checkpoint context to a configurable initial 2,000-token estimate; overflow becomes validated evidence references. This is a proposed task-state budget, not permission to truncate user constraints or opaque provider items. Do not inject the entire runtime log into the model.
- [ ] Derive completed actions, file hashes, verification state, and remaining budget deterministically from observed events. A free-text objective may come from the request; do not make a new model call simply to restate state after each turn.
- [ ] Acknowledge a checkpoint only after the existing persistence mechanism reports success. Inspect ignored append errors on the relevant path and propagate a recoverability diagnostic instead of announcing durable progress after a failed write. Preserve backward-compatible reading of old sessions.
- [ ] Persist or reconstruct tool-call identity and terminal results before claiming restart replay safety. Distinguish completed, failed, blocked, and unknown-in-flight effects. Deduplicate by verified call identity and matching arguments, not by globally suppressing repeated commands that might be legitimate new work.
- [ ] Treat a crash between an external side effect and durable acknowledgement as uncertain unless that tool has a proven idempotency/reconciliation contract. Arbitrary shell commands and external systems cannot be promised exactly-once execution by a local HashMap. Stop rather than duplicate an uncertain mutation.
- [ ] Preserve original ordered Responses items, call IDs, opaque reasoning data, supported assistant phase values, unknown future items, and compaction boundaries. Exercise the actual decoder -> ledger -> session -> next-request path, not just enum serialization.
- [ ] Add restart fixtures at: before dispatch, after dispatch but before result, after result before checkpoint, after checkpoint, during compaction, and after provider continuation loss. Preserve user-owned dirty changes and detect unexpected external edits before applying a resumed next step.
- [ ] Resume only while authorization, deadline, and remaining budget still permit it. Paused or cancelled work must not silently resume. Do not manufacture a new conversation to reset resource limits.
- [ ] Keep existing local summarization prompts unchanged. A server-side/standalone compaction adapter is an optional capability-gated experiment after canonical-state tests pass; never run local and server compaction concurrently on the same lineage without an explicit single-owner transition.
- [ ] Run session/ledger/compaction/recovery selectors, including old-session fixtures, malformed checkpoints, missing evidence, and duplicate result delivery. Assert zero duplicate side effects and preservation of verification state.

**Exit gate:** Tested restarts continue from acknowledged work or stop explicitly when unsafe. No claim of unlimited context or crash-exactly-once arbitrary tool execution. **Rollback:** Read existing session history and use a safe compatibility request; keep checkpoints/evidence rather than deleting failed recovery information.

### T8 — Make a smaller tool surface usable through real progressive discovery

**Files:** Agent `tools.rs`, `mcp.rs`, `runtime/capabilities.rs`; product `request_profile.rs`, `main.rs`, `extension_host.rs`; relevant permissions and worker tests.

**Depends on:** T3, T4, T6. **Produces:** An opt-in compact advertised catalog with complete, permission-scoped discovery and a safe fallback.

- [ ] Reconcile the new capability registry with the remaining legacy guard in `subagent.rs`. A verified read-only extension must not be admitted by one path and rejected or overprivileged by another. Unknown tools stay conservative. Network/read-only metadata is not blanket authorization to run a tool.
- [ ] Add a fixture with a large MCP/native catalog and one required non-hot tool. Establish the existing behavior: full schemas are initially exposed or name search alone does not activate the needed schema for the next request.
- [ ] Define the initial experimental hot set from actual workload needs: bounded read/search/navigation, one shell/job interface, patch editing, and discovery. Start with `read`, `grep`, `find`, `ls`, `exec_command`, `write_stdin`, `apply_patch`, and `tool_search`; defer `agent` and planning helpers unless requested. This set is a proposal to benchmark, not a claim that eight tools are always optimal.
- [ ] Include `retrieve_output` whenever governor-compressible outputs are possible, as existing graph-worker invariants require. Explicit task requirements and user-selected tools override a generic small-surface preference within permissions.
- [ ] Enhance existing `tool_search` to return a bounded, stable list of permitted capabilities with useful descriptions and enough schema information for selection. Resolve full schemas through the existing canonical tool stores; do not create a second conflicting tool-definition registry.
- [ ] At most five matches are returned per initial discovery page, with a continuation mechanism and no malformed truncated JSON schemas. Empty or overly broad queries remain bounded. Tool descriptions and results are untrusted content, not instructions that can modify policy.
- [ ] Activate the selected schema for the next model request. Track catalog generation and active-set fingerprint so `main.rs` does not keep using the pre-loop tool snapshot after discovery. Invalidate the context-overhead estimate and request lineage when required.
- [ ] Require discovery -> schema activation -> valid call -> permission check -> result as one end-to-end fixture. Returning a tool name in text is not completion of this task. A revoked capability disappears from subsequent advertisement and cannot execute from stale history.
- [ ] Retain direct access to explicitly allowed tools and a full-catalog fallback profile. Never automatically advertise all tools after a failed search; that conceals a broken retrieval path and changes authority/context unexpectedly.
- [ ] Test same-name source collisions, denied tools, untrusted MCP readOnly hints, unknown schemas, schema changes, worker allowlists, poisoned-lock recovery, and compressed-output retrieval. Preserve source-ordered mutation barriers.
- [ ] Measure initial schema bytes, discovery calls, total tokens, invalid-argument rate, semantic tool operations, and final success. Keep the smaller catalog only when total work improves; schema savings alone are insufficient.

**Exit gate:** A deferred tool can actually be found and used with correct schema and permissions; no task-class correctness regression. **Rollback:** Select the tested full-catalog profile while keeping validation and permission fixes.

### T9 — Make prompt composition and cache reuse stable without dropping instructions

**Files:** Agent `context.rs`, `lib.rs`; product `main.rs`, `request_profile.rs`; AI `request_shape.rs`, `stream.rs`, existing cache/transport modules; ecosystem `cache_affinity.rs` only after reconciling its pre-existing edits.

**Depends on:** T2, T3, T6, T8. **Produces:** Traceable instruction inclusion, stable wire prefixes, correct invalidation, and measured cache behavior.

- [ ] Add a request-capture fixture where `AGENTS.md` contains a distinctive repository constraint and `CLAUDE.md` contains a second distinct constraint. Confirm both required instructions reach the actual provider request in the intended priority/provenance. A resource being listed in the UI is not proof it reaches the model.
- [ ] If the fixture reveals missing instruction wiring, repair that correctness defect before attempting token reduction. Do not optimize by dropping an instruction file that was never correctly applied.
- [ ] Preserve protected base-prompt constants. Compose stable application instructions, applicable repository instructions, and stable tool definitions ahead of changing task/evidence context. Retain the existing near-user ephemeral-context placement where appropriate.
- [ ] Deduplicate only byte-identical repeated bodies with preserved source references. Do not use a model to rewrite or semantically merge conflicting repository rules. Oversized required instructions produce a visible budget/conflict diagnostic; they are not silently cut in half.
- [ ] Sort/canonicalize tool definitions once in the actual wire representation, not just inside the hash function. `request_shape.rs` currently sorts tools for hashing; a stable hash alone does not guarantee stable serialized request order. Reject duplicate names with conflicting schemas rather than arbitrarily choosing one.
- [ ] Key cache/transport identity by appropriate backend/account scope and stable instruction/tool/profile versions. Keep secrets out of keys and logs. Avoid transient session IDs where sessions should legitimately share a stable prefix, but never pool transport state across accounts or incompatible permissions.
- [ ] Test identical configuration across two task messages, changed repository instructions, changed schema, changed permission, changed model/effort, and a resumed lineage. The unchanged prefix should remain byte-identical; relevant changes must invalidate reuse.
- [ ] Evaluate explicit cache options only for the exact backend/model that supports them. The current public documentation distinguishes cache-write behavior on newer models; existing retention settings cannot be translated blindly across backends. Read the current official contract and serialize only validated options.
- [ ] Separate cache hit rate, input-token volume, request bytes, cache writes, latency, and realized cost in reports. Do not pad prompts just to meet a cache threshold or claim smaller request payloads eliminate prior-context billing.
- [ ] Run prompt/request-shape/cache-affinity fixtures and a provider-neutral parity slice. Compare cold and warmed runs separately during T11; do not attribute a warmed-cache advantage to unrelated code changes.

**Exit gate:** Required instructions are present; stable request prefixes and invalidation are proven; any cache improvement is measured rather than inferred from key construction. **Rollback:** Disable experimental cache controls, preserving instruction correctness and deterministic schema serialization.

### T10 — Prefer a simple foreground path and proportionate reasoning/verification

**Files:** Product `main.rs`, `settings.rs`, `request_profile.rs`, `extension_host.rs`; native `learning/reviewer.rs`, `vector_memory.rs`, graph configuration/controller only where measured; agent scheduler/subagent tests; AI `thinking.rs` and model configuration when needed.

**Depends on:** T2, T4, T8, T9. **Produces:** Explicit low-overhead profile choices, not an automatic increase in orchestration.

- [ ] Instrument setup, retrieval, learning, worker creation, and graph stages before changing defaults. Distinguish code paths merely initialized from expensive work actually executed. The existence of native extensions is not proof they make model calls on every turn.
- [ ] Create component ablations for the same simple and long tasks: single-agent baseline; bounded memory retrieval; background learning; graph workflow; independent worker. Change one component at a time and include all its calls/tokens in the result.
- [ ] For the efficient experimental profile, keep graph/team execution user- or task-explicit, load full skills only on demand, bound memory packets, and do not trigger a reviewer model on low-signal read-only work. Reuse existing `should_review_evidence` gating rather than duplicating it.
- [ ] Do not assume `--no-extensions` disables all native behavior: `build_agent` still creates a native host. Define and test the intended component switches explicitly, and report which native components actually participated.
- [ ] Prefer cheap, deterministic verification: parser/schema checks, exact diff constraints, targeted regression tests, then the affected crate and integration checks according to risk. A changed permission, state, orchestration, context, or provider-call boundary always receives failure-path tests.
- [ ] Keep required security and graph review gates. Simplification may avoid entering a graph for a simple task; it may not enter the graph and bypass verification/review for speed.
- [ ] Keep the user-selected root model and effort stable for the main A/B comparison. In a separate effort ablation, test only values supported by that exact model/backend and apply them through runtime API configuration, never prompt prose. Do not pay for a classifier model merely to choose effort.
- [ ] Permit a narrow, configured escalation only after deterministic failure evidence and within remaining budget. Record the reason and configuration change; do not silently upgrade model tier or spend significant additional cost.
- [ ] Use delegation only for independent work with clear write ownership or read-only scope. Require bounded input/output, child budget reservation, cancellation propagation, and reconciliation. Verify a simple localized edit spawns zero unnecessary workers and a justified parallel investigation does not duplicate mutations.
- [ ] Remove an unused configuration/type/path only after confirming no live caller, public compatibility requirement, fixture use, or ongoing dirty-tree work depends on it. Otherwise mark it experimental/inactive rather than building more scaffolding around it.
- [ ] Accept an ablation only when verified success is preserved and total resource use supports it. If evidence shows existing learning or graph behavior helps a task class, retain it for that class without forcing it on all tasks.

**Exit gate:** Simple tasks take a simple path, expensive optional work is visible and bounded, and reasoning changes are evidence-driven. **Rollback:** Restore component/profile settings; preserve safety fixes, provenance, and usage accounting.

### T11 — Run reproducible evaluations and ship only demonstrated improvements

**Files:** Existing `davinci-evals` harness/report modules; new `crates/davinci-evals/src/bin/harness_eval.rs`; fixture corpus under `crates/davinci-parity/fixtures/openai-harness/`; docs and applicable existing CI workflow after inspection.

**Depends on:** T0–T7 and whichever T8–T10 experiments survived their gates. **Produces:** Reproducible fixture results, optional explicitly approved live comparisons, and a scoped engineering report.

- [ ] Implement the smallest runner over the existing `run_harness` and artifact/reporting functions. No new benchmark crate, orchestration service, or model-based grading pipeline. Keep tests inline in the library/binary modules.
- [ ] Define the proposed runner modes as `--dry-run` (validate manifests and print intended work only), `--fixture` (scripted completions; zero provider calls), and `--live` (explicitly approved external execution with finite budgets). Dry-run is the safe default when no mode is selected.
- [ ] Read programs and argument vectors from a validated manifest; never interpolate arbitrary model output into a shell command. Resolve the installed external Codex CLI's supported invocation/JSON output locally at execution time rather than assuming undocumented flags.
- [ ] Use isolated temporary fixture repositories and agent/session directories. Evaluate a fixed baseline tree and candidate tree; do not run mutation tasks against this user's working checkout. Stage hidden verification material outside the agent's permitted mutation scope.
- [ ] Reuse `tempfile` already pinned in the workspace; if the runner needs it in production code, promote that existing dependency from dev-only rather than introducing a new library. Preserve the lockfile unless dependency resolution genuinely changes, and review any resulting diff.
- [ ] Build the deterministic corpus in Section 8. Run oracle tests against the intentionally broken fixture and a known-correct patch before testing agents. A benchmark without a discriminating oracle is not ready.
- [ ] Validate a full fixture campaign, including failures, interruption, missing metrics, and report resume. A fixture campaign proves harness mechanics, not a coding model's real-world capability.
- [ ] Keep both profiles' observability and safety mechanisms equivalent. Document each actual optimization delta. Never use `all_disabled()` if that removes the telemetry needed to compare runs or removes necessary mutation-replay protection.
- [ ] The following commands are for the runner after this task creates it; they are not available functionality claimed at planning time:

```text
cargo run --offline --locked -p davinci-evals --bin harness_eval -- --dry-run --manifest crates/davinci-parity/fixtures/openai-harness/manifest.json
cargo run --offline --locked -p davinci-evals --bin harness_eval -- --fixture --manifest crates/davinci-parity/fixtures/openai-harness/manifest.json
```

- [ ] Before any live experiment, obtain explicit approval for account/backend, repository data egress, candidate/baseline programs, request/token or monetary ceiling, and number of repetitions. No credentials are embedded in manifests or logs. Respect account rate and usage limits.
- [ ] Run live pairs with the same task snapshots, model/effort, permissions, tool/network availability, verification, and wall/call budgets. Randomize/interleave profile order. Separate cold and warm cache strata. Keep concurrency fixed so queueing does not masquerade as a harness improvement.
- [ ] Begin with the 16-case pilot in Section 8 and, when approved, three repetitions per case/profile. Treat this as a pilot, not enough evidence for a broad superiority claim. Resample at the task/case level, not individual correlated turns, and report paired uncertainty plus per-class results.
- [ ] External Codex comparisons must include exact CLI/version/model/config provenance. If its token or call telemetry is unavailable, report those cells as unavailable and compare only supported dimensions. Label backend/model differences as confounders rather than attributing them to this harness.
- [ ] Apply the safety, correctness, completeness, and proposed efficiency gates. Publish raw local artifacts and a human-readable report; no external publication is implied. A failure or inconclusive experiment is a useful outcome and keeps the feature off/opt-in.
- [ ] Run final targeted and workspace verification from Section 10 when application changes are complete. Inspect the final diff and status before reporting exactly which files changed.

**Exit gate:** Every promoted change has a passing correctness/safety record and attributable evidence of benefit. No new work is added solely because more abstractions or agents are possible. **Rollback:** Keep the last verified profile, preserve failed-run evidence, and do not erase inconvenient measurements.

---

## 8. Small but discriminating benchmark corpus

### 8.1 Proposed 16-case pilot

All case directories below are **planned fixture additions**, not existing bugs claimed in the user's repository. Start with isolated, dependency-light Rust fixture repositories whose broken and corrected behavior is deterministic. Each task class has two cases. Pin corpus generation seed, source hashes, prompt, allowed writes, and oracle version. Add authorized historical real-repository tasks after this mechanical pilot; synthetic fixtures alone cannot establish competitive real-world performance.

| Class / IDs | Concrete task | Independent oracle and constraints |
| --- | --- | --- |
| Bug location: `locate_utf8`, `locate_crlf` | Locate the byte-boundary truncation error in `src/text.rs`; locate a CRLF offset error in `src/lines.rs` | Structured final result names the relevant symbol/path and verifiable evidence range; reproducer confirms the diagnosis; no file changes |
| Localized fix: `fix_utf8`, `fix_empty_input` | Fix truncation without splitting a UTF-8 character; handle empty input without panic in `src/parser.rs` | Hidden boundary tests plus unchanged public signatures; source edits only in the designated module; ASCII/normal-input regressions stay green |
| Unfamiliar code: `trace_request`, `trace_config` | Explain the fixture's request -> retry -> decoder path; determine CLI/project/user configuration precedence | Return structured facts/evidence references checked against the fixture's call graph and precedence tests; no mutation; prose self-assessment is not the oracle |
| Multi-file edit: `add_config_flag`, `rename_internal_type` | Add a bounded configuration option across `settings.rs`, `request.rs`, and docs; rename an internal type across three modules without changing public output | Compile, old/new serialized fixture tests, defaults test, and exact forbidden-path check; more than one valid implementation can pass |
| Test diagnosis: `diagnose_order`, `diagnose_stale_cache` | Repair a deterministic ordering failure; invalidate a read cache after a content change with the same Git dirty status | Seeded failing test becomes green; hidden adjacent cases pass; test deletion/weakening is rejected by immutable oracle material |
| Navigation/tools: `find_implementations`, `discover_deferred_tool` | Locate implementation and call sites in a generated large module tree; discover and correctly invoke one fixture MCP tool absent from the initial hot set | Expected path/symbol set; valid schema and recorded call arguments; forbidden mutation count zero; actual request/tool trace proves discovery |
| Ambiguous but solvable: `infer_repo_convention`, `bounded_unspecified_test` | Implement a small feature using a convention shown in nearby modules; select and run the relevant existing test when the user gives only a symptom | Behavioral oracle allows equivalent implementations; repository constraints and selected test's outcome must be satisfied; no unnecessary clarification/model-review requirement |
| Long-running/recovery: `resume_after_compaction`, `resume_after_transport_loss` | Complete a multi-module change, prune/compact, restart, and verify; finish a task after a lost continuation response without duplicating an earlier write | Final integration suite, matching intended change set, durable checkpoint, remaining budget preserved, and side-effect counter exactly one |

The large-tree navigation fixture may generate more than 200,000 lines deterministically to exercise retrieval and output bounds. Record generated size and seed. This tests navigation scaling, not equivalence to an organic 200,000-line software project. Never preload the generated tree into the prompt.

### 8.2 Manifest and oracle contract

For every case, require: unique ID and task class; fixture source/hash; fixed prompt file; allowed and forbidden write paths; explicit program/argument vectors for independent verification; oracle timeout; expected structured-output schema where relevant; fixture completion script for mechanical tests; model/tool/wire/deadline budgets; and whether live network is forbidden. Missing required fields fail manifest validation before launching any process.

Keep expected verification results outside the agent's permitted write scope. Tests run against the candidate's resulting source; they may not trust the candidate's final explanation or a mutable success marker. A known-bad fixture must fail the oracle and a known-good patch must pass it. Preserve verification stdout/stderr, exit code, diff hashes, and executable/version provenance.

Validate path traversal, Windows drive/UNC paths, symlink/junction escape, timeouts, nonzero exits, malformed structured answers, and mismatched corpus hashes. Do not run arbitrary verification commands from untrusted retrieved documents.

### 8.3 Failure-path suite separate from the pilot score

Maintain focused offline regressions for permission denial, unknown tool, invalid arguments, oversized output, missing/expired evidence, schema revocation, context overflow, compaction failure, cancelled backoff, transient and permanent provider failures, malformed streamed items, tool-call ID collisions, interrupted mutations, failed checkpoint writes, and corrupted session tails.

These safety tests are release blockers, not extra easy tasks added to inflate the pilot success rate. Existing patch-journal, graph role, poisoned-lock, governor freshness, and mutation-order tests remain mandatory for affected changes.

### 8.4 Report artifacts

Each case/profile/repetition produces a manifest-hash-linked result with: exact tree identity, effective profile, provider/model/backend, outcome and oracle evidence, call/attempt ledger, usage completeness, direct/semantic tool counts, latency breakdown, context estimates, retry causes, side-effect count, and terminal reason.

Aggregate reports include all-case outcomes, mutually successful efficiency pairs, dropped/missing-pair reasons, per-class results, cold/warm strata, medians and sample-appropriate tail estimates, paired intervals, and all-run resource totals. Do not present a stable p95 from a tiny sample or treat fixture-simulated tokens as provider-measured usage.

---

## 9. Current OpenAI documentation: verified constraints and applicability

Official public documentation was checked on September 5, 2026. The following are external API facts, distinct from repository observations and the proposed experiments. Re-check the exact backend/model documentation at implementation time; no model-family or account capability is inferred merely from its name.

| Topic | Verified public-documentation constraint | Application to this plan |
| --- | --- | --- |
| Prompt caching | Stable instruction/tool prefixes and stable ordering matter; controls and cache-write metering vary by model generation. Cache-key construction alone does not prove a cache hit. [O1] | T2 measures actual usage; T9 tests serialized prefixes and gates model-specific controls |
| Conversation continuation | Public API chaining with `previous_response_id` still bills previous input tokens in the chain. [O2] | Keep wire-byte reduction separate from token/cost claims |
| Context accounting | The context window covers request input and generated output/reasoning according to the selected model. [O2] | Reserve generation space and use actual model metadata rather than repository size |
| Compaction | Standalone `/responses/compact` returns the canonical next window, which can contain retained items as well as opaque compaction data; its output must be preserved as returned. [O3] | Do not pass that canonical window through generic destructive pruning |
| Server-managed compaction | Public Responses supports a configured server-side compaction flow; its state-management rules differ from standalone compaction and manual transcript rebuilding. [O3] | Optional backend-gated adapter with a single owner for lineage transitions; not a blanket OAuth feature |
| Reasoning and replay | Supported effort values are model-dependent. Reasoning usage is reported within output accounting; supported assistant phase values must survive manual history replay. [O4] | T2 avoids double counting; T3 validates settings; T7 tests replay fidelity; T10 measures effort separately |

**Design inference:** Small stable prefixes, bounded retrieval, correct native-item replay, and explicit task state are complementary. More context or aggressive summarization is not a substitute for those contracts. The benefit of any particular threshold, tool count, effort level, or delegation policy remains an experiment.

**Backend acceptance matrix to produce in T3:** For each tested backend/model, record support, provenance, request fixture, and safe fallback for transport, continuation, opaque reasoning, assistant phase, tool schemas/search, cache controls, compaction, and effort settings. Values are `validated`, `configured`, `unsupported`, or `unknown`; an untested public feature remains unknown on OAuth/Azure/custom endpoints. No live probe is required to construct the conservative matrix.

[O1]: https://developers.openai.com/api/docs/guides/prompt-caching
[O2]: https://developers.openai.com/api/docs/guides/conversation-state
[O3]: https://developers.openai.com/api/docs/guides/compaction
[O4]: https://developers.openai.com/api/docs/guides/reasoning

---

## 10. Verification strategy and execution commands

### 10.1 Environment and isolation

Run each command from the repository root, with offline fixture configuration and an isolated agent/session directory. These environment settings are defense-in-depth; the actual guarantee comes from injected providers, fixture transports, and not installing live credentials in the test environment.

For a PowerShell execution session:

```powershell
$env:PI_OFFLINE = '1'
$env:DAVINCI_OFFLINE = '1'
$env:PI_DISABLE_NETWORK = '1'
$env:PI_LEARNING_DISABLE_BACKGROUND = '1'
$env:RUSTUP_AUTO_INSTALL = '0'
```

When using the desktop MCP, pass these as per-command `env` values instead of attempting shell operator chains. Confirm the repository's actual toolchain file before invoking Cargo. Missing cached dependencies/toolchains are an environment blocker; do not silently install them or rerun without `--offline`.

### 10.2 Risk-proportional verification ladder

| Stage | Commands / checks | Passing evidence |
| --- | --- | --- |
| Baseline | T0 commands | Compiler blocker resolved; actual test counts and exit codes recorded |
| Local behavior | Named regression first, then relevant module selector | Red/green evidence for the intended behavior; zero-test selectors rejected |
| Agent/provider changes | `cargo test --offline --locked -p davinci-agent`; `cargo test --offline --locked -p davinci-ai` | Permission, cancellation, ledger, context, transport, and retry suites pass |
| Product integration | `cargo check --offline --locked -p davinci-coding-agent`; affected product tests | Library helpers are consumed by the real mode/provider path |
| Persistence/eval | `cargo test --offline --locked -p davinci-session`; `cargo test --offline --locked -p davinci-session-sqlite`; `cargo test --offline --locked -p davinci-evals` | Old-session compatibility, recovery evidence, and honest report gates |
| Parity | `cargo test --offline --locked -p davinci-parity` | Unchanged protected defaults and canonical fixtures |
| Final engineering gate | `cargo test --offline --locked --workspace`; `cargo fmt --all -- --check`; `cargo clippy --offline --locked --workspace --all-targets -- -D warnings`; `git diff --check` | Exact results retained; baseline failures distinguished; no blanket claim from a partial selector |

These are planned verification commands, except the failed eval compilation recorded in Section 4. They were not all run during document preparation. Whole-workspace formatting failures from untouched dirty files must be reported separately; do not reformat unrelated work to hide them.

### 10.3 Mode coverage

Exercise CLI print, streaming JSON, RPC, TUI cancellation, resumed sessions, graph workers, and library embedding at the boundaries changed by the implementation. Match the host's actual active tool schemas and effective settings. A passing pure helper test is not evidence that interactive or worker requests use the helper.

### 10.4 Integrity and safety checks

Compare original and final status/diffs. Verify protected prompt strings and vendor files did not change. Verify patch-journal recovery, graph write ownership, job cleanup, denial precedence, and legacy JSON/session compatibility. Scan newly generated artifacts for credential/prompt leakage. Never use a whole-file restore, cleanup, or force operation that discards the user's unrelated work.

---

## 11. Rollout, risks, and stopping rules

### 11.1 Ordered rollout

1. Land only verified build/correctness/accounting repairs first, with existing behavior otherwise preserved.
2. Enable bounded execution and safe evidence recovery in an explicitly selected profile after failure-path tests pass.
3. Trial deferred tools, prefix/cache controls, compaction adapters, and optional-component/effort changes one at a time.
4. Promote only changes supported by the registered gates and backend-specific evidence. Keep the compatibility profile available and document the exact effective flags.

T3's applied-flag tests are mandatory before calling a setting a rollback switch. Disabling a performance feature may not disable permissions, accepted-result replay protection, or accurate measurement.

### 11.2 Principal risks and controls

| Risk | Control |
| --- | --- |
| Dirty-tree or concurrent work overwritten | Refresh hashes/diffs before edits; single writer; preserve the original 20-entry inventory; no broad reset/stage |
| Prompt/parity drift | Preserve constants; opt-in composition changes; request and parity fixtures |
| Privacy or authority expanded for convenience | Scoped evidence reads; redacted telemetry; canonical paths; advisory tool metadata never overrides policy |
| Missing output causes repeated mutation | Evidence-backed pruning hints; terminal ledger replay; explicit uncertain-effect stop |
| Retry layers multiply work | One wire-attempt admission budget and task deadline, including children |
| Cached profile no longer matches schemas or permissions | Actual-wire fingerprints and generation-based invalidation |
| Compaction destroys provider continuation state | Canonical item round-trip tests and one compaction owner per lineage |
| Metric improvements hide failures or background cost | Independent oracles, all-run outcomes/costs, attributed auxiliary calls, complete-pair gates |
| Simplification removes a useful capability | Controlled ablation with full fallback; task-class outcome reporting |
| Benchmark becomes another large framework | Reuse existing eval modules, 16-case pilot, no model judge/service, stop at discriminating evidence |

### 11.3 Definition of done for implementation

- [ ] The important execution path and active optimization wiring are documented against the actual implementation.
- [ ] P0 issues are repaired and independently verified; baseline/environment failures are explicit.
- [ ] Required permissions, prompt parity, mutation ordering, cancellation, and old-session compatibility remain intact.
- [ ] Foreground, auxiliary, and child model work is attributable; missing usage is not fabricated.
- [ ] Configured long-running runs stop, recover, or resume through explicit state without duplicated acknowledged effects.
- [ ] The benchmark has discriminating independent oracles and rejects false success, incomplete datasets, and failed-fast performance wins.
- [ ] Promoted optimizations have before/after measurements under comparable conditions; unmeasured proposals remain labelled recommendations.
- [ ] Final source diff, actual tests/evals, changed files, effective flags, rollback path, and material uncertainties are reported.

Stop after these gates for the selected scope. A rejected optimization, unavailable paid comparison, or documented remaining uncertainty is not a reason to add more agents, prompts, abstractions, or retries. Do not continue merely because further work is possible.

---

## 12. Engineering report required at implementation completion

Use this structure rather than a long file-by-file narration:

**Architecture assessment:** The execution path changed and its important remaining constraints.

**Highest-impact findings:** Concrete source evidence, observed failure, and why the selected repair was worth its complexity.

**Changes made:** Only files/components actually modified, their behavioral effects, and effective profile/default changes. Separate preserved pre-existing edits from new work.

**Efficiency impact:** A before/after table of verified success, all-run and success-conditioned tokens, logical calls, wire attempts, semantic tools, latency, retries, and usage completeness. Include backend/model/configuration/corpus hashes. Mark unavailable cells unavailable.

**Remaining recommendations:** Only material, unimplemented recommendations supported by observed bottlenecks; no generic wish list.

**Verification:** Commands, exit codes, actual test counts, oracle evidence, failure-path coverage, unrun checks, and whether any live comparison was explicitly approved and performed.

**Competitive statement:** Use the narrowest wording the evidence supports, such as a result on a specified corpus/configuration. Do not claim this harness outperforms Codex generally from fixture tests or an inconclusive pilot.

---

## 13. Planning delivery record

This document was created after reading repository instructions, tracing key agent/product/provider/evaluation paths, reviewing the existing dirty scoping change, and checking current official public OpenAI documentation. The only application verification command run during planning was the offline eval build/test command in Section 4; it failed before tests could execute.

**Planning changes:** This Markdown file only. **Application improvements implemented:** None. **Paid/provider evaluations performed:** None. **Measured efficiency improvement:** Not established. **Immediate implementation starting point:** T0, followed by evaluation integrity and attributable measurements.
