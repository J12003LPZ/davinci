# OpenAI-Codex Prompt Cache and Context Stability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Determine and fix preventable DaVinci cache misses and preserve OpenAI-compatible, prefix-stable Script history without making unsupported OAuth backend API calls.

**Architecture:** Extend existing provider observations and wire manifests with exact bounded segment identity diagnostics. Reuse existing session-key and native item replay, separate provider-side misses from local mutations, and capability-gate public Responses features.

**Tech Stack:** Rust 1.88; existing `davinci-ai` cache/replay, DaVinci session persistence and CI fixtures; no new paid API integration.

**Spec:** `docs/superpowers/specs/2026-10-31-droid-script-controlled-parity-design.md`. This is a *proposed amendment* to the existing owner plan (`docs/superpowers/plans/2026-10-22-codex-efficiency.md`); reconcile and approve before product-code implementation.

**Pinned baseline:** [`ecc8485`](https://github.com/J12003LPZ/davinci/commit/ecc8485361dc335aec8b82e7014b94904f347c71), Stage 0 PR #264, `gpt-6.1-sol` comparisons on 17 tasks. **Plan-only deliverable; no code changes, permissions, credentials, paid probes, or live experiments performed.**

## Global Constraints

- Preserve authenticated ChatGPT `openai-codex` without requiring an API key or paid API fallback.
- Root `AGENTS.md` and the existing Codemode C01–C12 security, journal, cancellation and model/effort contracts apply. No product write, commit, install or live subscription probe is authorized by this planning request.
- Keep current default/direct behavior until safe promotion; no speculative model calls or subagents to optimize heuristically.
- Independent behavioral imitation only: no proprietary binary/runtime code or exact private prompt copying.
- Rust toolchain **1.88.0** at inspected repo baseline; use pinned project dependencies and approved optional Node **24.21.0** Codemode sandbox, not an unverified interpreter.
- The public OpenAI API is **not** the ChatGPT Codex OAuth route; use a per-route *observed* capability matrix, not documentation alone.
- Separate full behavioral parity from performance parity and actual subscription-quota benefit; use independent oracles and include failures in total resource calculations.
- Do not change the `2026-10-04` plan's background-service prohibition without explicit owner approval and updated C08 recovery gates.

## Review Focus

1. Tool permission revoked while a Script call waits in a queue: deny before dispatch; test in the dispatch/permission task.
2. Mutation completed but reply lost: no automatic replay, later Script does not falsely imply rollback; test in the recovery task.
3. Process tree survives Script kill: must be owned/reaped or surfaced as unresolved; test in the job lifecycle task.
4. `toolCallId` from another session/branch: reject `status/wait/kill`; test in WaitForScript task.
5. Provider cache drops without any changed local prefix: classify UNKNOWN/provider-side instead of fabricating local cause; test in diagnostics task.

---

## Observed basis

Stage 0 (PR #264) showed **zero pruning and compactions** and 16 mid-run cache-read drops covering 60,435 of 185,252 uncached input tokens (~32.6%). That is a measurement target, not an established prefix bug. Initial prompt tokens were not worse than Droid's. Preserve Full and Context VM Off until route-specific evidence supports a change. OpenAI's public 30m/12h TTL write rates are real API product facts but are not automatically usable or economical on the subscription route.

### Task K01: Reproduce and attribute mid-run cache drops

**Dependencies:** Stage 0 evidence. **Commit gate:** independent reviewer approves each code/test change before it merges; do not accidentally mark this task complete by a green unrelated test.

**Files**
- Modify: `crates/davinci-ai/src/openai_cache_diagnostics.rs`
- Modify: `crates/davinci-agent/src/cache_stability.rs`
- Test: `crates/davinci-ai/tests/openai_cache_diagnostics.rs`

**Interface:** `CacheDropRecord { request_index, prev_cache_read, next_cache_read, wire_prefix_digest, route, reason }` (new telemetry DTO)

**Required behavior:** Recreate 16 drops / 60,435 uncached tokens in 17 PR #264 tasks when evidence available. Produce exact request segment manifests and provider usage pairing; classify local prefix change vs provider-side eviction/routing vs unknown. No raw prompts or secrets in artifacts.

**Grounding:** 32.6% uncached input in reported run, cause unknown; no compaction or pruning occurred.

**Acceptance:** A drop with identical prefix has UNKNOWN/provider class, never invented local cause; missing metrics stay null.

- [ ] **Step 1 — RED:** Create a test named `cache_drop_classification_requires_evidence` in `crates/davinci-ai/tests/openai_cache_diagnostics.rs` reproducing the acceptance condition with a disposable fixture. Ensure it fails against the current implementation for the *right reason*.
- [ ] **Step 2 — Record RED:** Run `cargo test -p davinci-ai cache_drop_classification_requires_evidence -- --nocapture` and capture the expected failed assertion (not a compile or environment error).
- [ ] **Step 3 — GREEN:** Implement the interface and required behavior above in only the named owner files; use existing components and add a focused file if needed, without duplicating the operation journal.
- [ ] **Step 4 — Verify GREEN:** Run `cargo test -p davinci-ai cache_drop_classification_requires_evidence -- --nocapture` and the relevant neighboring tests. Must pass; on a feature involving effects, perform a fresh subprocess/restart fixture too.
- [ ] **Step 5 — Review/commit:** Run `cargo fmt --all --check` and targeted `cargo clippy -p davinci-agent -p davinci-coding-agent -p davinci-ai --all-targets` for affected Rust crates. Inspect failure and security cases and commit **only if separately authorized**; otherwise leave diff for review.

### Task K02: Keep schema and instructions byte-stable under Script and tool changes

**Dependencies:** K01. **Commit gate:** independent reviewer approves each code/test change before it merges; do not accidentally mark this task complete by a green unrelated test.

**Files**
- Modify: `crates/davinci-ai/src/responses_tools.rs`
- Modify: `crates/davinci-agent/src/turn_context.rs`
- Modify: `crates/davinci-agent/src/prompt/manifest.rs`
- Test: `crates/davinci-ai/tests/openai_cache_request_manifest.rs`

**Interface:** `fn cache_prefix_manifest(req: &PreparedProviderRequest) -> Vec<WireSegmentDigest>` (existing manifest extension)

**Required behavior:** Stable tools order and exact advertised schemas by mode/session; append dynamic state near tail; preserve historical wire kind across direct/Script and retries; compare digest before/after tool discovery, Git status changes, reconnect.

**Grounding:** PR #264 initial 10,802 DaVinci tokens is not a proven problem; avoid indiscriminate prompt slimming.

**Acceptance:** Unchanged prefix bytes identical; new bindings do not silently rewrite cached prior tools.

- [ ] **Step 1 — RED:** Create a test named `script_mode_prefix_is_stable` in `crates/davinci-ai/tests/openai_cache_request_manifest.rs` reproducing the acceptance condition with a disposable fixture. Ensure it fails against the current implementation for the *right reason*.
- [ ] **Step 2 — Record RED:** Run `cargo test -p davinci-ai script_mode_prefix_is_stable -- --nocapture` and capture the expected failed assertion (not a compile or environment error).
- [ ] **Step 3 — GREEN:** Implement the interface and required behavior above in only the named owner files; use existing components and add a focused file if needed, without duplicating the operation journal.
- [ ] **Step 4 — Verify GREEN:** Run `cargo test -p davinci-ai script_mode_prefix_is_stable -- --nocapture` and the relevant neighboring tests. Must pass; on a feature involving effects, perform a fresh subprocess/restart fixture too.
- [ ] **Step 5 — Review/commit:** Run `cargo fmt --all --check` and targeted `cargo clippy -p davinci-agent -p davinci-coding-agent -p davinci-ai --all-targets` for affected Rust crates. Inspect failure and security cases and commit **only if separately authorized**; otherwise leave diff for review.

### Task K03: Probe subscription-route tool discovery and freeform support

**Dependencies:** K02; S17 mock integration. **Commit gate:** independent reviewer approves each code/test change before it merges; do not accidentally mark this task complete by a green unrelated test.

**Files**
- Modify: `crates/davinci-ai/src/codex_capabilities.rs`
- Modify: `crates/davinci-ai/src/responses_tools.rs`
- Test: `crates/davinci-ai/tests/openai_cache_request_manifest.rs`

**Interface:** `CodexCapabilities` scoped flags for `tool_search`, `additional_tools`, `programmatic_tool_calling`, and `custom_script`

**Required behavior:** Offline shape/parser tests first; public OpenAI APIs support deferred tools and programmatic, but DaVinci Codex backend previously rejected several APIs. If separately authorized, <=4 model requests test specific new wire shapes. Do not write unsupported shapes by default.

**Grounding:** Existing Stage 0 W3 probe authorized only as proposed; not yet run.

**Acceptance:** Unsupported shape retains function fallback; no active subscriptions required for offline regression.

- [ ] **Step 1 — RED:** Create a test named `unsupported_hosted_tools_fail_closed` in `crates/davinci-ai/tests/openai_cache_request_manifest.rs` reproducing the acceptance condition with a disposable fixture. Ensure it fails against the current implementation for the *right reason*.
- [ ] **Step 2 — Record RED:** Run `cargo test -p davinci-ai unsupported_hosted_tools_fail_closed -- --nocapture` and capture the expected failed assertion (not a compile or environment error).
- [ ] **Step 3 — GREEN:** Implement the interface and required behavior above in only the named owner files; use existing components and add a focused file if needed, without duplicating the operation journal.
- [ ] **Step 4 — Verify GREEN:** Run `cargo test -p davinci-ai unsupported_hosted_tools_fail_closed -- --nocapture` and the relevant neighboring tests. Must pass; on a feature involving effects, perform a fresh subprocess/restart fixture too.
- [ ] **Step 5 — Review/commit:** Run `cargo fmt --all --check` and targeted `cargo clippy -p davinci-agent -p davinci-coding-agent -p davinci-ai --all-targets` for affected Rust crates. Inspect failure and security cases and commit **only if separately authorized**; otherwise leave diff for review.

### Task K04: Compare stable manual discovery to native deferred tool search

**Dependencies:** K03. **Commit gate:** independent reviewer approves each code/test change before it merges; do not accidentally mark this task complete by a green unrelated test.

**Files**
- Modify: `crates/davinci-agent/src/tools.rs`
- Modify: `crates/davinci-agent/src/runtime/capabilities.rs`
- Modify: `crates/davinci-ai/src/responses_tools.rs`
- Test: `crates/davinci-agent/tests/lean_tool_surface.rs`

**Interface:** `ToolExposureDelta { snapshot_revision, added_names, stable_prefix_digest }` (host-authored, append only)

**Required behavior:** Use existing `tool_search` where possible and test `additional_tools` only if backend supports it. Scope binding discovery to next Script and keep frozen catalog current Script. Report possible extra model turn and cache cost.

**Grounding:** Droid dynamically loads `ToolSearch` and freezes Script bindings; live initial prompt DaVinci is not bigger.

**Acceptance:** Discover a new tool mid-session; it is callable in next Script only, permission revocation invalidates it, old request prefix unaffected if route supports append.

- [ ] **Step 1 — RED:** Create a test named `deferred_tool_load_does_not_rewrite_history` in `crates/davinci-agent/tests/lean_tool_surface.rs` reproducing the acceptance condition with a disposable fixture. Ensure it fails against the current implementation for the *right reason*.
- [ ] **Step 2 — Record RED:** Run `cargo test -p davinci-agent deferred_tool_load_does_not_rewrite_history -- --nocapture` and capture the expected failed assertion (not a compile or environment error).
- [ ] **Step 3 — GREEN:** Implement the interface and required behavior above in only the named owner files; use existing components and add a focused file if needed, without duplicating the operation journal.
- [ ] **Step 4 — Verify GREEN:** Run `cargo test -p davinci-agent deferred_tool_load_does_not_rewrite_history -- --nocapture` and the relevant neighboring tests. Must pass; on a feature involving effects, perform a fresh subprocess/restart fixture too.
- [ ] **Step 5 — Review/commit:** Run `cargo fmt --all --check` and targeted `cargo clippy -p davinci-agent -p davinci-coding-agent -p davinci-ai --all-targets` for affected Rust crates. Inspect failure and security cases and commit **only if separately authorized**; otherwise leave diff for review.

### Task K05: Bound compaction and preserve native append-only replay

**Dependencies:** K01. **Commit gate:** independent reviewer approves each code/test change before it merges; do not accidentally mark this task complete by a green unrelated test.

**Files**
- Modify: `crates/davinci-agent/src/compaction.rs`
- Modify: `crates/davinci-agent/src/runtime/context_vm/fold.rs`
- Modify: `docs/context-vm.md`
- Test: `crates/davinci-agent/tests/context_vm_cache.rs`

**Interface:** `ContextVmMode::{Off,Hybrid,Active}` existing; no new default

**Required behavior:** Long-context test with same model/effort/task across off/hybrid/active; account for compaction model calls, exact preserved user requirements and permission metadata, provider hit rate and net verified cost. Stage 0 zero prunes/folds, so no premature W4 rewrite.

**Grounding:** Repo stage0-report: no pruning or compaction in 17 tasks.

**Acceptance:** No claimed improvement absent quality/cost benefit; no forged source requirements or orphan native tool calls.

- [ ] **Step 1 — RED:** Create a test named `context_mode_comparison_does_not_lose_requirements` in `crates/davinci-agent/tests/context_vm_cache.rs` reproducing the acceptance condition with a disposable fixture. Ensure it fails against the current implementation for the *right reason*.
- [ ] **Step 2 — Record RED:** Run `cargo test -p davinci-agent context_mode_comparison_does_not_lose_requirements -- --nocapture` and capture the expected failed assertion (not a compile or environment error).
- [ ] **Step 3 — GREEN:** Implement the interface and required behavior above in only the named owner files; use existing components and add a focused file if needed, without duplicating the operation journal.
- [ ] **Step 4 — Verify GREEN:** Run `cargo test -p davinci-agent context_mode_comparison_does_not_lose_requirements -- --nocapture` and the relevant neighboring tests. Must pass; on a feature involving effects, perform a fresh subprocess/restart fixture too.
- [ ] **Step 5 — Review/commit:** Run `cargo fmt --all --check` and targeted `cargo clippy -p davinci-agent -p davinci-coding-agent -p davinci-ai --all-targets` for affected Rust crates. Inspect failure and security cases and commit **only if separately authorized**; otherwise leave diff for review.

### Task K06: Measure route-specific cache lifetime and write cost safely

**Dependencies:** K01. **Commit gate:** independent reviewer approves each code/test change before it merges; do not accidentally mark this task complete by a green unrelated test.

**Files**
- Modify: `crates/davinci-ai/src/openai_cache_policy.rs`
- Modify: `crates/davinci-ai/src/provider_observation.rs`
- Test: `crates/davinci-ai/tests/openai_cache_accounting.rs`

**Interface:** `EffectiveOpenAiCachePolicy` existing; add evidence-level capability table with exact route/model

**Required behavior:** For direct public API, model-specific 30m/12h cache-write economics may matter. For ChatGPT Codex route, preserve current authenticated supported policy; no unsupported TTL/breakpoint injection. Track cached, fresh, written and reasoning usage independently.

**Grounding:** OpenAI prompt caching docs, model/endpoint dependent; subscription quota != dollar price.

**Acceptance:** No API-only optimization is deployed to Codex; invalid cache shape caught offline; missing usage unknown.

- [ ] **Step 1 — RED:** Create a test named `cache_ttl_not_sent_to_unverified_codex_route` in `crates/davinci-ai/tests/openai_cache_accounting.rs` reproducing the acceptance condition with a disposable fixture. Ensure it fails against the current implementation for the *right reason*.
- [ ] **Step 2 — Record RED:** Run `cargo test -p davinci-ai cache_ttl_not_sent_to_unverified_codex_route -- --nocapture` and capture the expected failed assertion (not a compile or environment error).
- [ ] **Step 3 — GREEN:** Implement the interface and required behavior above in only the named owner files; use existing components and add a focused file if needed, without duplicating the operation journal.
- [ ] **Step 4 — Verify GREEN:** Run `cargo test -p davinci-ai cache_ttl_not_sent_to_unverified_codex_route -- --nocapture` and the relevant neighboring tests. Must pass; on a feature involving effects, perform a fresh subprocess/restart fixture too.
- [ ] **Step 5 — Review/commit:** Run `cargo fmt --all --check` and targeted `cargo clippy -p davinci-agent -p davinci-coding-agent -p davinci-ai --all-targets` for affected Rust crates. Inspect failure and security cases and commit **only if separately authorized**; otherwise leave diff for review.

### Task K07: Protect session rollback, edit/test errors and cold resume prefixes

**Dependencies:** K02; K05. **Commit gate:** independent reviewer approves each code/test change before it merges; do not accidentally mark this task complete by a green unrelated test.

**Files**
- Modify: `crates/davinci-agent/src/cache_stability.rs`
- Modify: `crates/davinci-agent/src/runtime/session.rs`
- Test: `crates/davinci-agent/tests/cache_runtime.rs`

**Interface:** `CacheMissReason` existing enriched with segment provenance where possible

**Required behavior:** Fixture matrix for branch/fork, tail change, mode flip, retry, tool error, uncertain-effect recovery, cold session resume and interleaved subagents. Pair exact canonical wire bytes with reported cache reuse; provider cache miss is allowed despite byte equality.

**Grounding:** Droid's prompt prefix design is append-only, but route behavior varies.

**Acceptance:** No false cached-token claim from digest equality; recovered session history matches previous durable journal.

- [ ] **Step 1 — RED:** Create a test named `rollback_and_resume_cache_manifests_are_truthful` in `crates/davinci-agent/tests/cache_runtime.rs` reproducing the acceptance condition with a disposable fixture. Ensure it fails against the current implementation for the *right reason*.
- [ ] **Step 2 — Record RED:** Run `cargo test -p davinci-agent rollback_and_resume_cache_manifests_are_truthful -- --nocapture` and capture the expected failed assertion (not a compile or environment error).
- [ ] **Step 3 — GREEN:** Implement the interface and required behavior above in only the named owner files; use existing components and add a focused file if needed, without duplicating the operation journal.
- [ ] **Step 4 — Verify GREEN:** Run `cargo test -p davinci-agent rollback_and_resume_cache_manifests_are_truthful -- --nocapture` and the relevant neighboring tests. Must pass; on a feature involving effects, perform a fresh subprocess/restart fixture too.
- [ ] **Step 5 — Review/commit:** Run `cargo fmt --all --check` and targeted `cargo clippy -p davinci-agent -p davinci-coding-agent -p davinci-ai --all-targets` for affected Rust crates. Inspect failure and security cases and commit **only if separately authorized**; otherwise leave diff for review.

## Cache rollout gates

- [ ] Provider request manifests and usage telemetry contain no credentials, paths, raw proprietary content or model-private data.
- [ ] Each proposed change has a deterministic offline prefix test and real-model measurement authorization when applicable.
- [ ] Request-count, total input, fresh input, cache reads, output+reasoning, cache-write and correctness reported separately.
- [ ] Do not change default Context VM, Full/Lean, TTL or hosted tool-search without an independently graded success/cost result.
