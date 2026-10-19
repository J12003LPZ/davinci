# DaVinci Orchestration and Efficiency — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Improve latency, context quality and selective delegation without weakening deterministic execution order or introducing paid auxiliary calls.

**Architecture:** Enhance current scheduler/RuntimeCapability registry, worktree-aware subagents, prompt-cache stability, MCP progressive schema activation and context budgeting. Require baseline/ablation comparisons before changing defaults.

**Tech Stack:** Existing Rust 1.88.0 workspace and `davinci-evals`; no new infrastructure or dependencies.

**Spec:** `docs/superpowers/specs/2026-09-14-davinci-harness-optimization-design.md`, `00-INDEX.md`.

## Global constraints

- **Do not** increase DaVinci's default 8 parallel calls simply because recovered Claude Code defaults to 10. A parallelism win must not increase interference, denied calls, corruption, resource contention or task failures.
- Do not launch speculative teammates or auxiliary models by default. Delegation must justify the cost and maintain user permission ceilings.
- Keep prompt/static schema stability on `openai-codex`. API cache rules cannot be presumed identical to the authenticated Codex backend.
- Keep the current progressive tool-discovery path; do not build a second MCP registry.

## Review focus

- Parallel read whose arguments actually write/create cache has unsafe effect classification (O1).
- Concurrent writable subagents share a checkout or worktree owner; no unintended conflicts (O2).
- User changes project instructions mid-session; cache snapshots do not suppress authoritative updates (O3).
- Deferred tool activation on a large catalog; only authorized matching tools exposed (O4).
- Context pressure with huge tool outputs and partial model error; preserved safety margin and recoverable evidence (O5).

---

### Task O1 — Per-invocation concurrency-safety classifier

**Files:** `crates/davinci-agent/src/{scheduler.rs,turn.rs}`, `src/runtime/{capabilities.rs,registry.rs}`, scheduler unit tests in `src/scheduler.rs`; create `tests/invocation_safety_scheduler.rs` if no equivalent integration tests exist.

**Interfaces:** Existing `RuntimeCapability.concurrency_policy` is the default/fail-closed authority. Any call-specific refinement accepts `tool name + parsed args + authoritative capability` and returns existing `ToolLane`, without permission escalation.

- [ ] **Step 1 — Audit:** Compare `lane_for_capability`, agent-call routing and command effects; identify arguments that genuinely change concurrency safety (do not infer independence from tool name).
- [ ] **Step 2 — Test:** `adjacent_independent_reads_overlap`, `read_write_same_path_is_barrier`, `unsafe_shell_calls_serialize`, `missing_capability_serializes`, `source_order_results_remain_stable`, `abort_skips_not_started_work`, `per_call_refinement_cannot_override_unsafe_registry_policy`.
- [ ] **Step 3 — Red/benchmark:** Run `cargo test -p davinci-agent --lib scheduler --offline --locked` and any new integration tests. Benchmark current width=8/order as control on safe-read and mixed read/write fixtures.
- [ ] **Step 4 — Implement only measured safe refinement:** Add an optional argument-aware classifier at existing admission, never let it mark an authorization-denied or registry-unsafe operation parallel; do not change publication ordering without a separately measured need.
- [ ] **Step 5 — Green/performance:** Rerun tests and benchmark elapsed time, conflicts and cancellation; keep feature disabled if no meaningful gain or any correctness loss.
- [ ] **Step 6 — Record:** Show exact policy table for accepted vs rejected invocation classes; **commit only if authorized**.

### Task O2 — Measured subagent delegation and task ownership

**Files:** `crates/davinci-agent/src/{subagent.rs,delegation.rs}`, `src/runtime/{team.rs,mailbox.rs,worktree.rs,tasks.rs}`, `tests/{delegation_policy,wor_mailbox_durability,wor_worker_control}.rs`; evals `crates/davinci-evals/tests/agent_orchestration.rs`.

**Interfaces:** Existing parent agent task/worktree controls and message bus remain authoritative. No new agent framework or always-on worker.

- [ ] **Step 1 — Audit:** Map parent→child budget/permissions, worktree isolation, cancellation propagation, completion/wakeup and provenance of child reports.
- [ ] **Step 2 — Test:** `delegated_worker_does_not_exceed_parent_permission`, `two_writers_have_distinct_worktrees_or_serialize`, `teammate_message_is_not_user_authorization`, `cancelled_worker_cannot_commit_after_owner_revocation`, `delegation_result_links_to_task_id`.
- [ ] **Step 3 — Red:** `cargo test -p davinci-agent --test delegation_policy --test wor_mailbox_durability --test wor_worker_control --offline --locked` plus new cases.
- [ ] **Step 4 — Minimal repair only if red:** Use existing task, mailbox, owner fence and worktree lease. Do not add forced delegation heuristics until independent eval proves ROI.
- [ ] **Step 5 — Controlled task ablation (only if approved):** Compare no-delegation control to delegated independent investigations on task pass, latency, model calls and usage. Keep solo as default if gains unproven.
- [ ] **Step 6 — Record:** Evidence of ownership and effectiveness; **commit only if authorized**.

### Task O3 — Stable prompt-prefix and cache evidence

**Files:** `crates/davinci-agent/src/{cache_stability.rs,prepared_context.rs,context.rs}`, `src/prompt/{composer.rs,bundle.rs,session.rs}`, `crates/davinci-ai/src/{openai_cache_policy.rs,openai_cache_diagnostics.rs}`, `tests/{cache_runtime_eval,context_vm_prefix_stability,wor37_cache_write_coverage}.rs`; `docs/cache/{codex-backend-probe.md,codex-comparison.md}`.

**Interfaces:** Reuse existing prompt modules, provider route and usage counters. Never substitute local fingerprints for actual backend cache hits.

- [ ] **Step 1 — Audit:** Identify stable tool+instruction prefixes, dynamic project/user context and invalidation triggers; distinguish snapshot version from current authority.
- [ ] **Step 2 — Test:** `equivalent_turns_preserve_stable_prefix`, `project_instruction_change_invalidates_snapshot`, `permission_change_invalidates_tool_exposure`, `compaction_retains_required_context`, `unsupported_cache_field_stays_unknown`.
- [ ] **Step 3 — Red:** `cargo test -p davinci-agent --test context_vm_prefix_stability --test cache_runtime_eval --offline --locked` plus new tests.
- [ ] **Step 4 — Correct if red:** Narrow dynamic prompt changes to existing dynamic modules; preserve current authorized schemas and user context. Do not add provider-specific cache directives unsupported by Codex route.
- [ ] **Step 5 — Green and measurement:** Assert cache-prefix byte stability for unchanged authority and invalidation for changed authority; report provider-observed fresh/cached/output tokens separately. No live probes without user scope approval.
- [ ] **Step 6 — Record:** Mark cache benefits *unmeasured* unless backend usage actually reports them; **commit only if authorized**.

### Task O4 — Progressive tool discovery correctness and schema efficiency

**Files:** `crates/davinci-agent/src/{mcp.rs,skills.rs,tools.rs}`, `src/prompt/tool_strategy.rs`, `tests/{mcp_progressive_disclosure,lean_tool_surface,mcp_execution_policy}.rs`.

**Interfaces:** Current `tool_search` activation + existing tool-spec registry. Unknown/denied capabilities remain inaccessible.

- [ ] **Step 1 — Audit:** Confirm existing 200-tool catalog pagination, activated-tool visibility, denied tool filtering and per-turn exposure/recovery behavior.
- [ ] **Step 2 — Test:** `selected_tool_is_visible_on_next_request`, `denied_tools_do_not_activate`, `activation_survives_safe_resume`, `large_catalog_is_bounded`, `tool_order_does_not_change_without_new_authority`.
- [ ] **Step 3 — Red:** `cargo test -p davinci-agent --test mcp_progressive_disclosure --test lean_tool_surface --test mcp_execution_policy --offline --locked` plus any missing cases.
- [ ] **Step 4 — Minimal fix only if red:** Reuse MCP registry/tool-search state; avoid adding always-visible tools or replacing dynamic discovery.
- [ ] **Step 5 — Green:** Measure schema token count and tool-selection correctness for large fixture; don't optimize tokens at expense of successful discovery.
- [ ] **Step 6 — Record:** Close as existing parity if all constraints hold; **commit only if authorized**.

### Task O5 — Context-pressure protection and fallback

**Files:** `crates/davinci-agent/src/{context_usage.rs,compaction.rs,prepared_context.rs}`, `src/runtime/{root_budget.rs,context_vm/compiler.rs}`, `tests/{auto_compaction_threshold,compaction_local_fallback,context_vm_budget,harness_root_budget}.rs`.

**Interfaces:** Current provider-aware context window and actual usage objects; no global hard-coded Claude 200k/1M arithmetic.

- [ ] **Step 1 — Audit:** Trace reserve, warning, prune/retrieve and compaction selection for each supported route; distinguish local estimates from backend counted tokens.
- [ ] **Step 2 — Test:** `small_window_preserves_output_reserve`, `large_tool_result_retains_retrieval_source`, `failed_compaction_does_not_discard_uncommitted_tool_results`, `unexpected_context_limit_replans_safely`, `missing_usage_is_unknown_not_zero`.
- [ ] **Step 3 — Red:** `cargo test -p davinci-agent --test auto_compaction_threshold --test compaction_local_fallback --test context_vm_budget --test harness_root_budget --offline --locked` plus new cases.
- [ ] **Step 4 — Fix only when red:** Tune adaptive reserve/pruning using current Context VM; preserve exact requirements and side-effect provenance. Do not clone Claude's version-specific 13k threshold blindly.
- [ ] **Step 5 — Green:** Validate deterministic error handling, no unexpected context overrun in fixtures, and correct bounded-retry behavior.
- [ ] **Step 6 — Record:** Report token/correctness tradeoff and retained authority facts; **commit only if authorized**.

## Gate G3

Change defaults only if local safety cases pass and real coding-efficiency measurements are independently favorable. It is acceptable—and often correct—to close O1–O5 as no-change parity audits.
