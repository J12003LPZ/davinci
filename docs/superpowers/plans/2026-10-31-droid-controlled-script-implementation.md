# Droid-Compatible Controlled Script and Background Runtime Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Independently reproduce Droid Script’s authorized Read/Edit/Execute/Test, output, and durable background semantics in DaVinci without weakening its Rust security and subscription route.

**Architecture:** Keep the existing `NodeCodeModeHost`/QuickJS worker as an unprivileged scripting VM and `AgentCodeModeBroker` as the only dispatcher. Add typed/host-owned run state and bound output events, route all mutations through the existing operation journal and permission system, then opt-in after crash recovery gates.

**Tech Stack:** Rust 1.88, Node 24.21.0 QuickJS/WASI, existing `davinci-agent`, `davinci-ai`, `davinci-coding-agent`, JSONL journal and pytest/Rust tests.

**Spec:** `docs/superpowers/specs/2026-10-31-droid-script-controlled-parity-design.md`. This is a *proposed amendment* to the existing owner plan (`docs/superpowers/plans/2026-10-04-codemode.md`); reconcile and approve before product-code implementation.

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

## Dependency graph and ordered releases

```
S01 aliases ──> S03 guarded calls ──> S04 edits / S08 Execute ──> S05 recovery
S02 trusted controlled mode ─┘                            └──> S12 state
S06 raw inputs ─> S07 explicit output ─> S10 limits ─> S11 logs ─> S12 state
S09 concurrency ────────────────────────────────────────────┘
S12 ─> S13 WaitForScript ─> S14 background ─> S15 crash/reconcile
S07 + S12 ─> S16 mode history ─> S17 freeform transport ─> S18 prompt
All ─> S19 reference fixtures ─> S20 opt-in release ─> B03 live pilot
```

**Release cut A** (read-only compatibility): S01/S06/S07/S10/S11/S16/S17 + tests; does not require controlled mode. **Release cut B** (explicit controlled effects): S02–S05/S08/S09 + crash matrix; disabled until gated. **Release cut C** (durable background): S12–S15, separately approved due to contradiction with existing C08 restriction. **Release cut D** (quality): S18–S22. Each cut can be reviewed without shipping the next.

## Workspace/file map

- `crates/davinci-agent/src/codemode/{types,policy,dispatch,execution,projection}.rs`: authority, advertised capabilities, tool results, run lifecycle; do not create another policy engine.
- `crates/davinci-agent/src/runtime/operations/*` and `jobs/*`: **reuse** durable effects and process ownership, not guest-owned files.
- `crates/davinci-coding-agent/codemode-host/{host,bounded-worker,strict-prelude}.mjs`: unprivileged VM, input lexer, bounded events and IPC only.
- `crates/davinci-coding-agent/src/codemode_host/{supervisor,config,protocol}.rs`: parent-owned child supervision and framing.
- `crates/davinci-ai/src/{responses_tools,responses_request}.rs`: provider-specific freeform `custom_tool_call` projection and cache-safe history.
- Focused Rust/Node tests in existing suites; `docs/codemode.md` reflects only behavior actually shipped.

### Task S01: Bind Droid-compatible authorized tool aliases

**Dependencies:** baseline; can be implemented independently where interfaces permit. **Commit gate:** independent reviewer approves each code/test change before it merges; do not accidentally mark this task complete by a green unrelated test.

**Files**
- Modify: `crates/davinci-agent/src/codemode/policy.rs`
- Test: `crates/davinci-agent/tests/codemode_policy.rs`

**Interface:** `pub fn resolve_script_alias(name: &str, mode: CodeModeMode, registry: &CapabilityPolicy) -> Result<String, CodeModeError>`

**Required behavior:** `tools.Read`, `tools.Grep`, `tools.LS`, `tools.Execute`, `tools.ApplyPatch` map to existing declared capabilities, not raw host services. Unsupported/colliding aliases deny, no blanket path fallback. Snapshot alias identities per Script. No new permissions from aliasing.

**Grounding:** Droid reference §03 tool bindings; DaVinci `policy.rs` currently excludes dangerous controls and handles collisions.

**Acceptance:** `tools.Execute` cannot be resolved when normal shell capability is revoked; alias collision yields `DENIED`, and a discovered alias cannot change while a Script is running.

- [ ] **Step 1 — RED:** Create a test named `codemode_aliases_are_policy_bounded` in `crates/davinci-agent/tests/codemode_policy.rs` reproducing the acceptance condition with a disposable fixture. Ensure it fails against the current implementation for the *right reason*.
- [ ] **Step 2 — Record RED:** Run `cargo test -p davinci-agent codemode_aliases_are_policy_bounded -- --nocapture` and capture the expected failed assertion (not a compile or environment error).
- [ ] **Step 3 — GREEN:** Implement the interface and required behavior above in only the named owner files; use existing components and add a focused file if needed, without duplicating the operation journal.
- [ ] **Step 4 — Verify GREEN:** Run `cargo test -p davinci-agent codemode_aliases_are_policy_bounded -- --nocapture` and the relevant neighboring tests. Must pass; on a feature involving effects, perform a fresh subprocess/restart fixture too.
- [ ] **Step 5 — Review/commit:** Run `cargo fmt --all --check` and targeted `cargo clippy -p davinci-agent -p davinci-coding-agent -p davinci-ai --all-targets` for affected Rust crates. Inspect failure and security cases and commit **only if separately authorized**; otherwise leave diff for review.

### Task S02: Enable controlled Codemode through trusted explicit settings

**Dependencies:** baseline only; implement config plumbing first but keep controlled execution disabled until the independent release gate. **Commit gate:** independent reviewer approves each code/test change before it merges; do not accidentally mark this task complete by a green unrelated test.

**Files**
- Modify: `crates/davinci-agent/src/codemode/execution.rs`
- Modify: `crates/davinci-coding-agent/src/codemode_host/config.rs`
- Modify: `crates/davinci-agent/src/codemode/types.rs`
- Test: `crates/davinci-agent/tests/codemode_contract.rs`
- Test: `crates/davinci-coding-agent/tests/codemode_cli.rs`

**Interface:** `pub fn enable_codemode(&mut self, host: Arc<dyn CodeModeHost>, mode: CodeModeMode) -> Result<(), CodeModeError>`

**Required behavior:** Replace hard-coded `ReadOnly` in `run_codemode` with host-resolved immutable mode. Add CLI/user settings `off|read-only|controlled`, no project/guest escalation, no silent default. Controlled must be feature-gated behind S05/S15 acceptance; current `read-only` callers retain behavior.

**Grounding:** Current mode enum already includes Controlled, but `run_codemode` constructs ReadOnly.

**Acceptance:** User controls `controlled`, a JavaScript header does not; sessionless or missing trusted host denies; off mode unchanged.

- [ ] **Step 1 — RED:** Create a test named `controlled_mode_is_opt_in_and_host_owned` in `crates/davinci-agent/tests/codemode_contract.rs` reproducing the acceptance condition with a disposable fixture. Ensure it fails against the current implementation for the *right reason*.
- [ ] **Step 2 — Record RED:** Run `cargo test -p davinci-agent controlled_mode_is_opt_in_and_host_owned -- --nocapture` and capture the expected failed assertion (not a compile or environment error).
- [ ] **Step 3 — GREEN:** Implement the interface and required behavior above in only the named owner files; use existing components and add a focused file if needed, without duplicating the operation journal.
- [ ] **Step 4 — Verify GREEN:** Run `cargo test -p davinci-agent controlled_mode_is_opt_in_and_host_owned -- --nocapture` and the relevant neighboring tests. Must pass; on a feature involving effects, perform a fresh subprocess/restart fixture too.
- [ ] **Step 5 — Review/commit:** Run `cargo fmt --all --check` and targeted `cargo clippy -p davinci-agent -p davinci-coding-agent -p davinci-ai --all-targets` for affected Rust crates. Inspect failure and security cases and commit **only if separately authorized**; otherwise leave diff for review.

### Task S03: Admit mutating operations with current Rust policy

**Dependencies:** S01; S02. **Commit gate:** independent reviewer approves each code/test change before it merges; do not accidentally mark this task complete by a green unrelated test.

**Files**
- Modify: `crates/davinci-agent/src/codemode/dispatch.rs`
- Modify: `crates/davinci-agent/src/codemode/policy.rs`
- Test: `crates/davinci-agent/tests/codemode_policy.rs`

**Interface:** `fn authorize_script_child(&self, tool: &str, args: &Value, revision: &str) -> Result<AuthorizedChild, CodeModeError>` (internal; avoid duplicating existing `require_authorized` + `dispatch_script_child`)

**Required behavior:** Expose only safe declared-effect classes in controlled; re-check permission, workspace lease, effect type, root deadline and prompt-injection boundary immediately before effect; use the existing operation journal and authorizer. No script-granted `riskLevel` override. Host interaction / account-control tools stay excluded.

**Grounding:** Existing `AgentCodeModeBroker::call()` already serializes conflicting effects and calls `dispatch_script_child`.

**Acceptance:** Queued command denied when policy changes; approved direct command and identical Script command produce equivalent permission verdict and journal entries.

- [ ] **Step 1 — RED:** Create a test named `queued_mutation_revocation_is_denied` in `crates/davinci-agent/tests/codemode_policy.rs` reproducing the acceptance condition with a disposable fixture. Ensure it fails against the current implementation for the *right reason*.
- [ ] **Step 2 — Record RED:** Run `cargo test -p davinci-agent queued_mutation_revocation_is_denied -- --nocapture` and capture the expected failed assertion (not a compile or environment error).
- [ ] **Step 3 — GREEN:** Implement the interface and required behavior above in only the named owner files; use existing components and add a focused file if needed, without duplicating the operation journal.
- [ ] **Step 4 — Verify GREEN:** Run `cargo test -p davinci-agent queued_mutation_revocation_is_denied -- --nocapture` and the relevant neighboring tests. Must pass; on a feature involving effects, perform a fresh subprocess/restart fixture too.
- [ ] **Step 5 — Review/commit:** Run `cargo fmt --all --check` and targeted `cargo clippy -p davinci-agent -p davinci-coding-agent -p davinci-ai --all-targets` for affected Rust crates. Inspect failure and security cases and commit **only if separately authorized**; otherwise leave diff for review.

### Task S04: Add file edit, write and apply-patch Script calls

**Dependencies:** S03. **Commit gate:** independent reviewer approves each code/test change before it merges; do not accidentally mark this task complete by a green unrelated test.

**Files**
- Modify: `crates/davinci-agent/src/codemode/dispatch.rs`
- Modify: `crates/davinci-agent/src/tools.rs`
- Test: `crates/davinci-agent/tests/codemode_contract.rs`

**Interface:** `tools.ApplyPatch({patch})` and `tools.Edit({...})` / `tools.Write({...})` through canonical existing tools; broker returns `operationRef`

**Required behavior:** Preserve atomic edit behavior, symlink and out-of-workspace protections, file writer queue, approval and exact before/after evidence. One failed verification cannot silently revert or replay earlier succeeded edits. Use DaVinci native input schemas where Droid differs; compatibility adapter must be explicit and tested.

**Grounding:** Droid mock `Execute` and `Read` worked inside Script; apply-patch API from reference is static.

**Acceptance:** Patch modifies intended file once, denied path is untouched, concurrent same-path writers serialize, successful earlier edit survives later Script throw with clear failure status.

- [ ] **Step 1 — RED:** Create a test named `controlled_script_applies_patch_once` in `crates/davinci-agent/tests/codemode_contract.rs` reproducing the acceptance condition with a disposable fixture. Ensure it fails against the current implementation for the *right reason*.
- [ ] **Step 2 — Record RED:** Run `cargo test -p davinci-agent controlled_script_applies_patch_once -- --nocapture` and capture the expected failed assertion (not a compile or environment error).
- [ ] **Step 3 — GREEN:** Implement the interface and required behavior above in only the named owner files; use existing components and add a focused file if needed, without duplicating the operation journal.
- [ ] **Step 4 — Verify GREEN:** Run `cargo test -p davinci-agent controlled_script_applies_patch_once -- --nocapture` and the relevant neighboring tests. Must pass; on a feature involving effects, perform a fresh subprocess/restart fixture too.
- [ ] **Step 5 — Review/commit:** Run `cargo fmt --all --check` and targeted `cargo clippy -p davinci-agent -p davinci-coding-agent -p davinci-ai --all-targets` for affected Rust crates. Inspect failure and security cases and commit **only if separately authorized**; otherwise leave diff for review.

### Task S05: Implement crash-safe mutation authorization and reconciliation gate

**Dependencies:** S03; S04. **Commit gate:** independent reviewer approves each code/test change before it merges; do not accidentally mark this task complete by a green unrelated test.

**Files**
- Modify: `crates/davinci-agent/src/codemode/execution.rs`
- Modify: `crates/davinci-agent/src/runtime/operations/recovery.rs`
- Modify: `crates/davinci-agent/src/codemode/dispatch.rs`
- Test: `crates/davinci-agent/tests/codemode_contract.rs`
- Test: `crates/davinci-agent/tests/codemode_policy.rs`

**Interface:** `fn reconcile_script_child_effect(operation_ref: &str) -> ScriptEffectResolution` (proposed; use existing journal recovery types rather than duplicating them)

**Required behavior:** Matrix: process dies before journal; after pending persisted; after effect before resolved marker; after resolution; approval revoked during wait; IPC reply missing; executor disconnect. Mark ambiguous outcomes `RECOVERY_REQUIRED`, close further mutation admission, require independent postcondition before retry.

**Grounding:** C07/C08 existing plan are partial. `dispatch.rs` checks `EffectStatus::{Possible,Unknown,EffectsObserved}`.

**Acceptance:** Never issue second command on ambiguous first effect. Pass native recovery/mutation tests across Windows plus Linux/macOS CI.

- [ ] **Step 1 — RED:** Create a test named `unknown_mutation_never_auto_replays` in `crates/davinci-agent/tests/codemode_contract.rs` reproducing the acceptance condition with a disposable fixture. Ensure it fails against the current implementation for the *right reason*.
- [ ] **Step 2 — Record RED:** Run `cargo test -p davinci-agent unknown_mutation_never_auto_replays -- --nocapture` and capture the expected failed assertion (not a compile or environment error).
- [ ] **Step 3 — GREEN:** Implement the interface and required behavior above in only the named owner files; use existing components and add a focused file if needed, without duplicating the operation journal.
- [ ] **Step 4 — Verify GREEN:** Run `cargo test -p davinci-agent unknown_mutation_never_auto_replays -- --nocapture` and the relevant neighboring tests. Must pass; on a feature involving effects, perform a fresh subprocess/restart fixture too.
- [ ] **Step 5 — Review/commit:** Run `cargo fmt --all --check` and targeted `cargo clippy -p davinci-agent -p davinci-coding-agent -p davinci-ai --all-targets` for affected Rust crates. Inspect failure and security cases and commit **only if separately authorized**; otherwise leave diff for review.

### Task S06: Implement raw multiline input blocks and Script options

**Dependencies:** baseline only. **Commit gate:** independent reviewer approves each code/test change before it merges; do not accidentally mark this task complete by a green unrelated test.

**Files**
- Modify: `crates/davinci-coding-agent/codemode-host/host.mjs`
- Modify: `crates/davinci-coding-agent/codemode-host/strict-prelude.mjs`
- Modify: `crates/davinci-agent/src/codemode/types.rs`
- Test: `crates/davinci-coding-agent/codemode-host/host.test.mjs`

**Interface:** `parseScriptSource(source: string): {code:string, inputs:Record<string,string>, background:boolean}` (host-internal JS, host-side validation mirrored in Rust bounds)

**Required behavior:** Support leading `// @input: name <<'TAG'` and `// @script: {"startInBackground": true}` as observed; duplicate keys, malformed markers, unclosed tags, malicious delimiters or excessive bytes fail before dispatch. Preserve trailing newlines. Legacy JSON `{code}` remains.

**Grounding:** Droid §03.3 and §19 fixture 05; current DaVinci rejects header directives.

**Acceptance:** `inputs.patch` equals original multiline bytes, malformed block runs zero children; JS literal directive after code treated as content.

- [ ] **Step 1 — RED:** Create a test named `script_input_blocks_are_byte_exact` in `crates/davinci-coding-agent/codemode-host/host.test.mjs` reproducing the acceptance condition with a disposable fixture. Ensure it fails against the current implementation for the *right reason*.
- [ ] **Step 2 — Record RED:** Run `node --test crates/davinci-coding-agent/codemode-host/*.test.mjs` and capture the expected failed assertion (not a compile or environment error).
- [ ] **Step 3 — GREEN:** Implement the interface and required behavior above in only the named owner files; use existing components and add a focused file if needed, without duplicating the operation journal.
- [ ] **Step 4 — Verify GREEN:** Run `node --test crates/davinci-coding-agent/codemode-host/*.test.mjs` and the relevant neighboring tests. Must pass; on a feature involving effects, perform a fresh subprocess/restart fixture too.
- [ ] **Step 5 — Review/commit:** Run `cargo fmt --all --check` and targeted `cargo clippy -p davinci-agent -p davinci-coding-agent -p davinci-ai --all-targets` for affected Rust crates. Inspect failure and security cases and commit **only if separately authorized**; otherwise leave diff for review.

### Task S07: Add `text`, `image`, `notify` and console output channels

**Dependencies:** baseline only. **Commit gate:** independent reviewer approves each code/test change before it merges; do not accidentally mark this task complete by a green unrelated test.

**Files**
- Modify: `crates/davinci-coding-agent/codemode-host/host.mjs`
- Modify: `crates/davinci-coding-agent/codemode-host/bounded-worker.mjs`
- Modify: `crates/davinci-agent/src/codemode/projection.rs`
- Test: `crates/davinci-coding-agent/codemode-host/host.test.mjs`
- Test: `crates/davinci-agent/tests/codemode_projection.rs`

**Interface:** `emitScriptEvent(kind: "text"|"image"|"notify", value: unknown): void` (JS internal) and an authenticated typed IPC event

**Required behavior:** Only explicit emits reach model, model-invisible intermediate results stay local. Ensure no unredacted secrets or unauthorized image channels. Implement a typed, size-bounded image asset projection via the existing image policy; if it cannot be implemented safely, mark image parity OPEN and fail `UNSUPPORTED_OUTPUT` deterministically rather than claiming complete parity. Reserve final host facts against truncation. Existing return-value behavior remains for legacy scripts.

**Grounding:** Droid §03 outputs; DaVinci existing host returns JSON and preserves head/tail after PR #264.

**Acceptance:** Large results are processed inside VM and emit a short summary; `notify` is not a free model message or authority grant; truncated text carries omitted marker and complete=false.

- [ ] **Step 1 — RED:** Create a test named `script_explicit_output_only` in `crates/davinci-coding-agent/codemode-host/host.test.mjs` reproducing the acceptance condition with a disposable fixture. Ensure it fails against the current implementation for the *right reason*.
- [ ] **Step 2 — Record RED:** Run `node --test crates/davinci-coding-agent/codemode-host/*.test.mjs` and capture the expected failed assertion (not a compile or environment error).
- [ ] **Step 3 — GREEN:** Implement the interface and required behavior above in only the named owner files; use existing components and add a focused file if needed, without duplicating the operation journal.
- [ ] **Step 4 — Verify GREEN:** Run `node --test crates/davinci-coding-agent/codemode-host/*.test.mjs` and the relevant neighboring tests. Must pass; on a feature involving effects, perform a fresh subprocess/restart fixture too.
- [ ] **Step 5 — Review/commit:** Run `cargo fmt --all --check` and targeted `cargo clippy -p davinci-agent -p davinci-coding-agent -p davinci-ai --all-targets` for affected Rust crates. Inspect failure and security cases and commit **only if separately authorized**; otherwise leave diff for review.

### Task S08: Expose structured Execute/test-run results

**Dependencies:** S03; S04. **Commit gate:** independent reviewer approves each code/test change before it merges; do not accidentally mark this task complete by a green unrelated test.

**Files**
- Modify: `crates/davinci-agent/src/codemode/dispatch.rs`
- Modify: `crates/davinci-agent/src/tools.rs`
- Modify: `crates/davinci-coding-agent/codemode-host/host.mjs`
- Test: `crates/davinci-agent/tests/codemode_contract.rs`

**Interface:** `fn adapt_script_tool_result(tool: &str, result: ToolResult) -> CodeModeToolValue` (internal adapter without losing `complete/operationRef`)

**Required behavior:** Map `tools.Execute` to existing guarded `exec_command`/`bash` depending on actual registered tool, with structured exit code, stdout/stderr metadata, abort and timeout. Do not call Node `child_process`. Tests use same shell tool rather than special test executor.

**Grounding:** Droid §19 `tools.Execute` required `riskLevel` and returned exitCode; DaVinci native risk policy may differ.

**Acceptance:** `cargo test` failure returns nonzero exit and last error lines; disallowed command is denied before process spawn; cancellation kills owned descendants.

- [ ] **Step 1 — RED:** Create a test named `script_execute_reports_exit_and_test_failures` in `crates/davinci-agent/tests/codemode_contract.rs` reproducing the acceptance condition with a disposable fixture. Ensure it fails against the current implementation for the *right reason*.
- [ ] **Step 2 — Record RED:** Run `cargo test -p davinci-agent script_execute_reports_exit_and_test_failures -- --nocapture` and capture the expected failed assertion (not a compile or environment error).
- [ ] **Step 3 — GREEN:** Implement the interface and required behavior above in only the named owner files; use existing components and add a focused file if needed, without duplicating the operation journal.
- [ ] **Step 4 — Verify GREEN:** Run `cargo test -p davinci-agent script_execute_reports_exit_and_test_failures -- --nocapture` and the relevant neighboring tests. Must pass; on a feature involving effects, perform a fresh subprocess/restart fixture too.
- [ ] **Step 5 — Review/commit:** Run `cargo fmt --all --check` and targeted `cargo clippy -p davinci-agent -p davinci-coding-agent -p davinci-ai --all-targets` for affected Rust crates. Inspect failure and security cases and commit **only if separately authorized**; otherwise leave diff for review.

### Task S09: Implement deterministic concurrency and effect-aware scheduling

**Dependencies:** S03. **Commit gate:** independent reviewer approves each code/test change before it merges; do not accidentally mark this task complete by a green unrelated test.

**Files**
- Modify: `crates/davinci-agent/src/codemode/dispatch.rs`
- Modify: `crates/davinci-agent/src/runtime/worker_admission.rs`
- Test: `crates/davinci-agent/tests/codemode_policy.rs`

**Interface:** `fn shared_limit(mode: &CodeModeMode, policy: ConcurrencyPolicy, max_parallel: usize) -> Option<usize>` (existing; extend tests, avoid new scheduler)

**Required behavior:** Parallel independent reads with `Promise.allSettled`. Preserve exclusive lane for ambiguous writes; disjoint writes only when lease and effect class permit; tests must await edits. Root budgets bound number of in-flight and pending calls.

**Grounding:** Existing broker has exclusive lane and `parallelism=4`, root capacity.

**Acceptance:** Parallel read result order matches script’s promise order; same-file writes never overlap; failed sibling does not erase successful sibling evidence.

- [ ] **Step 1 — RED:** Create a test named `parallel_reads_overlap_mutations_serialize` in `crates/davinci-agent/tests/codemode_policy.rs` reproducing the acceptance condition with a disposable fixture. Ensure it fails against the current implementation for the *right reason*.
- [ ] **Step 2 — Record RED:** Run `cargo test -p davinci-agent parallel_reads_overlap_mutations_serialize -- --nocapture` and capture the expected failed assertion (not a compile or environment error).
- [ ] **Step 3 — GREEN:** Implement the interface and required behavior above in only the named owner files; use existing components and add a focused file if needed, without duplicating the operation journal.
- [ ] **Step 4 — Verify GREEN:** Run `cargo test -p davinci-agent parallel_reads_overlap_mutations_serialize -- --nocapture` and the relevant neighboring tests. Must pass; on a feature involving effects, perform a fresh subprocess/restart fixture too.
- [ ] **Step 5 — Review/commit:** Run `cargo fmt --all --check` and targeted `cargo clippy -p davinci-agent -p davinci-coding-agent -p davinci-ai --all-targets` for affected Rust crates. Inspect failure and security cases and commit **only if separately authorized**; otherwise leave diff for review.

### Task S10: Bound Script resource usage, guest and transport

**Dependencies:** S07; S09. **Commit gate:** independent reviewer approves each code/test change before it merges; do not accidentally mark this task complete by a green unrelated test.

**Files**
- Modify: `crates/davinci-agent/src/codemode/mod.rs`
- Modify: `crates/davinci-agent/src/codemode/types.rs`
- Modify: `crates/davinci-coding-agent/codemode-host/bounded-worker.mjs`
- Modify: `crates/davinci-coding-agent/src/codemode_host/supervisor.rs`
- Test: `crates/davinci-coding-agent/tests/codemode_supervisor_budget.rs`

**Interface:** `pub struct CodeModeLimits` (existing fields; add explicit output events, nested input aggregate, background caps if justified)

**Required behavior:** Keep current safe defaults until memory/stress tests; implement versioned Droid-compatible profile with fixed hard ceilings only after validation. Input aggregate, frames, pending requests, guest heap, elapsed root wall, output events and logs must have independent caps; failures cannot allocate unbounded buffered guest output.

**Grounding:** Droid reference: runtime-configurable caps; broad examples include 128 in-flight, 1 MiB nested inputs, 5 MiB output, 1,024 output events.

**Acceptance:** 10MB accidental guest emit is bounded and does not crash app; UTF-8 and status survive; no hidden automatic cap increase under `controlled`.

- [ ] **Step 1 — RED:** Create a test named `oversized_script_fails_closed_and_preserves_tail` in `crates/davinci-coding-agent/tests/codemode_supervisor_budget.rs` reproducing the acceptance condition with a disposable fixture. Ensure it fails against the current implementation for the *right reason*.
- [ ] **Step 2 — Record RED:** Run `cargo test -p davinci-coding-agent oversized_script_fails_closed_and_preserves_tail -- --nocapture` and capture the expected failed assertion (not a compile or environment error).
- [ ] **Step 3 — GREEN:** Implement the interface and required behavior above in only the named owner files; use existing components and add a focused file if needed, without duplicating the operation journal.
- [ ] **Step 4 — Verify GREEN:** Run `cargo test -p davinci-coding-agent oversized_script_fails_closed_and_preserves_tail -- --nocapture` and the relevant neighboring tests. Must pass; on a feature involving effects, perform a fresh subprocess/restart fixture too.
- [ ] **Step 5 — Review/commit:** Run `cargo fmt --all --check` and targeted `cargo clippy -p davinci-agent -p davinci-coding-agent -p davinci-ai --all-targets` for affected Rust crates. Inspect failure and security cases and commit **only if separately authorized**; otherwise leave diff for review.

### Task S11: Persist bounded Script outputs and child evidence without reruns

**Dependencies:** S07; S08. **Commit gate:** independent reviewer approves each code/test change before it merges; do not accidentally mark this task complete by a green unrelated test.

**Files**
- Modify: `crates/davinci-agent/src/codemode/execution.rs`
- Modify: `crates/davinci-agent/src/evidence.rs`
- Modify: `crates/davinci-agent/src/codemode/types.rs`
- Test: `crates/davinci-agent/tests/codemode_contract.rs`

**Interface:** `fn persist_script_log(run_id: &str, events: &[ScriptLogEvent]) -> Result<ScriptLogRef, ScriptLogError>` (new, host-owned and bounded)

**Required behavior:** Store append-only status, per-child command output and outcomes with retention caps and redaction; avoid retaining sensitive full file Reads by default. Scripts can request bounded slices through a host-authorized read path. Artifacts are not arbitrary file paths the model can forge.

**Grounding:** Droid §03 logs retain results except text Reads; DaVinci already has `EvidenceStore`.

**Acceptance:** Script that runs a slow test once can inspect last 200 lines later without issuing the command again; log identifiers are session-bound; secrets scrubbed.

- [ ] **Step 1 — RED:** Create a test named `script_log_is_readable_without_reexecuting_commands` in `crates/davinci-agent/tests/codemode_contract.rs` reproducing the acceptance condition with a disposable fixture. Ensure it fails against the current implementation for the *right reason*.
- [ ] **Step 2 — Record RED:** Run `cargo test -p davinci-agent script_log_is_readable_without_reexecuting_commands -- --nocapture` and capture the expected failed assertion (not a compile or environment error).
- [ ] **Step 3 — GREEN:** Implement the interface and required behavior above in only the named owner files; use existing components and add a focused file if needed, without duplicating the operation journal.
- [ ] **Step 4 — Verify GREEN:** Run `cargo test -p davinci-agent script_log_is_readable_without_reexecuting_commands -- --nocapture` and the relevant neighboring tests. Must pass; on a feature involving effects, perform a fresh subprocess/restart fixture too.
- [ ] **Step 5 — Review/commit:** Run `cargo fmt --all --check` and targeted `cargo clippy -p davinci-agent -p davinci-coding-agent -p davinci-ai --all-targets` for affected Rust crates. Inspect failure and security cases and commit **only if separately authorized**; otherwise leave diff for review.

### Task S12: Add durable Script-run registry and state transitions

**Dependencies:** S05; S11. **Commit gate:** independent reviewer approves each code/test change before it merges; do not accidentally mark this task complete by a green unrelated test.

**Files**
- Modify: `crates/davinci-agent/src/codemode/types.rs`
- Modify: `crates/davinci-agent/src/codemode/execution.rs`
- Modify: `crates/davinci-agent/src/runtime/operations/mod.rs`
- Test: `crates/davinci-agent/tests/codemode_contract.rs`

**Interface:** `ScriptRunRecord { run_id, outer_call_id, session_id, workspace_binding, permission_revision, started_at, state, log_ref }` (new host-owned, never guest-deserialized)

**Required behavior:** Durable `created/running/completed/failed/cancelled/stalled/recovery_required` monotonic transitions with write-ahead and fsync per existing journal standards. Restrict by session/workspace/branch/host; no writable guest state fields. Recovery always reconciles operations before interpreting status.

**Grounding:** Droid §03.6 `state.json`+backup+lock; DaVinci must reuse its existing operation journal rather than blindly copying files.

**Acceptance:** State identical after cold reload; forged handle and identity mismatch denied; duplicate completion notification suppressed.

- [ ] **Step 1 — RED:** Create a test named `script_state_persists_and_reconciles` in `crates/davinci-agent/tests/codemode_contract.rs` reproducing the acceptance condition with a disposable fixture. Ensure it fails against the current implementation for the *right reason*.
- [ ] **Step 2 — Record RED:** Run `cargo test -p davinci-agent script_state_persists_and_reconciles -- --nocapture` and capture the expected failed assertion (not a compile or environment error).
- [ ] **Step 3 — GREEN:** Implement the interface and required behavior above in only the named owner files; use existing components and add a focused file if needed, without duplicating the operation journal.
- [ ] **Step 4 — Verify GREEN:** Run `cargo test -p davinci-agent script_state_persists_and_reconciles -- --nocapture` and the relevant neighboring tests. Must pass; on a feature involving effects, perform a fresh subprocess/restart fixture too.
- [ ] **Step 5 — Review/commit:** Run `cargo fmt --all --check` and targeted `cargo clippy -p davinci-agent -p davinci-coding-agent -p davinci-ai --all-targets` for affected Rust crates. Inspect failure and security cases and commit **only if separately authorized**; otherwise leave diff for review.

### Task S13: Add `WaitForScript` status/wait/kill API

**Dependencies:** S12. **Commit gate:** independent reviewer approves each code/test change before it merges; do not accidentally mark this task complete by a green unrelated test.

**Files**
- Modify: `crates/davinci-agent/src/codemode/execution.rs`
- Modify: `crates/davinci-agent/src/codemode/types.rs`
- Modify: `crates/davinci-agent/src/tools.rs`
- Test: `crates/davinci-agent/tests/codemode_contract.rs`

**Interface:** `WaitForScriptRequest { tool_call_id: String, action: WaitAction }`, `WaitAction::{Status,Wait,Kill}` (host-owned parsing); `fn wait_for_script(req: WaitForScriptRequest, ctx: &ToolContext) -> ToolResult`

**Required behavior:** Opaque handles, session authorization, bounded `wait` <= root deadline; `status` nonblocking and idempotent; `kill` returns actual observed child-tree state or `RECOVERY_REQUIRED`, never promises cancellation before effect ownership confirmed.

**Grounding:** Droid §19 `WaitForScript({toolCallId,action})`; user must not be allowed to poll other sessions.

**Acceptance:** Different session cannot inspect same id; status unchanged across restart; repeated kill safe; running status truthfully returned.

- [ ] **Step 1 — RED:** Create a test named `wait_for_script_scoped_handle_and_terminal_status` in `crates/davinci-agent/tests/codemode_contract.rs` reproducing the acceptance condition with a disposable fixture. Ensure it fails against the current implementation for the *right reason*.
- [ ] **Step 2 — Record RED:** Run `cargo test -p davinci-agent wait_for_script_scoped_handle_and_terminal_status -- --nocapture` and capture the expected failed assertion (not a compile or environment error).
- [ ] **Step 3 — GREEN:** Implement the interface and required behavior above in only the named owner files; use existing components and add a focused file if needed, without duplicating the operation journal.
- [ ] **Step 4 — Verify GREEN:** Run `cargo test -p davinci-agent wait_for_script_scoped_handle_and_terminal_status -- --nocapture` and the relevant neighboring tests. Must pass; on a feature involving effects, perform a fresh subprocess/restart fixture too.
- [ ] **Step 5 — Review/commit:** Run `cargo fmt --all --check` and targeted `cargo clippy -p davinci-agent -p davinci-coding-agent -p davinci-ai --all-targets` for affected Rust crates. Inspect failure and security cases and commit **only if separately authorized**; otherwise leave diff for review.

### Task S14: Support detached/background Scripts and notification delivery

**Dependencies:** S12; S13; explicit owner approval to supersede C08 resident/background prohibition. **Commit gate:** independent reviewer approves each code/test change before it merges; do not accidentally mark this task complete by a green unrelated test.

**Files**
- Modify: `crates/davinci-agent/src/codemode/execution.rs`
- Modify: `crates/davinci-agent/src/jobs/managed.rs`
- Modify: `crates/davinci-coding-agent/src/codemode_host/supervisor.rs`
- Test: `crates/davinci-agent/tests/codemode_contract.rs`
- Test: `crates/davinci-coding-agent/tests/codemode_supervisor.rs`

**Interface:** `fn start_background_script(ctx: CodeModeRunContext, source: ScriptSource) -> Result<ScriptRunHandle, CodeModeError>` (new; route through existing process manager)

**Required behavior:** `// @script` starts background only with trusted `controlled` opt-in; timeout may return running without killing; process/job owner supervises child tree and root budget. Persist notification and re-wake once; no polling-only model turns when notification arrives. Define shutdown, session close and reconnect ownership.

**Grounding:** Conflicts with prior C08 plan restriction on persistent JS. Owner approval required before this task.

**Acceptance:** Long test returns handle, `wait` later finds same run; host quit kills or reconciles owned process; no zombie workers; notification delivered once.

- [ ] **Step 1 — RED:** Create a test named `background_script_returns_running_and_can_finish` in `crates/davinci-agent/tests/codemode_contract.rs` reproducing the acceptance condition with a disposable fixture. Ensure it fails against the current implementation for the *right reason*.
- [ ] **Step 2 — Record RED:** Run `cargo test -p davinci-agent background_script_returns_running_and_can_finish -- --nocapture` and capture the expected failed assertion (not a compile or environment error).
- [ ] **Step 3 — GREEN:** Implement the interface and required behavior above in only the named owner files; use existing components and add a focused file if needed, without duplicating the operation journal.
- [ ] **Step 4 — Verify GREEN:** Run `cargo test -p davinci-agent background_script_returns_running_and_can_finish -- --nocapture` and the relevant neighboring tests. Must pass; on a feature involving effects, perform a fresh subprocess/restart fixture too.
- [ ] **Step 5 — Review/commit:** Run `cargo fmt --all --check` and targeted `cargo clippy -p davinci-agent -p davinci-coding-agent -p davinci-ai --all-targets` for affected Rust crates. Inspect failure and security cases and commit **only if separately authorized**; otherwise leave diff for review.

### Task S15: Reconcile crash, restart, cancellation and ambiguous effects

**Dependencies:** S05; S12; S14. **Commit gate:** independent reviewer approves each code/test change before it merges; do not accidentally mark this task complete by a green unrelated test.

**Files**
- Modify: `crates/davinci-agent/src/codemode/execution.rs`
- Modify: `crates/davinci-agent/src/runtime/operations/recovery.rs`
- Modify: `crates/davinci-coding-agent/src/codemode_host/supervisor.rs`
- Test: `crates/davinci-agent/tests/codemode_contract.rs`
- Test: `crates/davinci-coding-agent/tests/codemode_supervisor.rs`

**Interface:** `fn reconcile_script_run(run_id: &str) -> Result<ReconciledScriptRun, RecoveryError>` (new integration; existing journal authoritative)

**Required behavior:** Crash matrix: guest throws, Node killed after edit, parent killed mid-effect, double restart, pending tool approval, lost IPC frame, detached process timeout. Never resume prior JS from mid-instruction; preserve already durable success; block unresolved effects for explicit postcondition.

**Grounding:** Droid claims durable reconcile; DaVinci existing journal/receipts are the security source of truth.

**Acceptance:** No duplicate write/command; orphan worker eventually reaped; post-crash record reports `recovery_required` accurately.

- [ ] **Step 1 — RED:** Create a test named `crashed_script_never_replays_mutation` in `crates/davinci-agent/tests/codemode_contract.rs` reproducing the acceptance condition with a disposable fixture. Ensure it fails against the current implementation for the *right reason*.
- [ ] **Step 2 — Record RED:** Run `cargo test -p davinci-agent crashed_script_never_replays_mutation -- --nocapture` and capture the expected failed assertion (not a compile or environment error).
- [ ] **Step 3 — GREEN:** Implement the interface and required behavior above in only the named owner files; use existing components and add a focused file if needed, without duplicating the operation journal.
- [ ] **Step 4 — Verify GREEN:** Run `cargo test -p davinci-agent crashed_script_never_replays_mutation -- --nocapture` and the relevant neighboring tests. Must pass; on a feature involving effects, perform a fresh subprocess/restart fixture too.
- [ ] **Step 5 — Review/commit:** Run `cargo fmt --all --check` and targeted `cargo clippy -p davinci-agent -p davinci-coding-agent -p davinci-ai --all-targets` for affected Rust crates. Inspect failure and security cases and commit **only if separately authorized**; otherwise leave diff for review.

### Task S16: Preserve append-only history through mode switches

**Dependencies:** S07; S12. **Commit gate:** independent reviewer approves each code/test change before it merges; do not accidentally mark this task complete by a green unrelated test.

**Files**
- Modify: `crates/davinci-agent/src/codemode/execution.rs`
- Modify: `crates/davinci-ai/src/responses_tools.rs`
- Modify: `crates/davinci-ai/src/responses_request.rs`
- Test: `crates/davinci-agent/tests/codemode_contract.rs`
- Test: `crates/davinci-ai/tests/openai_cache_request_manifest.rs`

**Interface:** `fn project_script_history(script: &CodeModeOutcome, mode: CodeModeMode) -> Vec<ChatMessage>` (versioned, effect-safe presentation only)

**Required behavior:** Switch script/direct, model, or availability mid-session without rewriting previous tool-call wire identities or regenerating historical content. Provider request ordering remains stable; if provider cannot represent mixed old modes, transparently start new branch with explicit cache-cost warning instead of silent mutation.

**Grounding:** Droid §03.1 mode-change reminder/reprojection; DaVinci tool wire kinds are persisted in `responses_tools.rs`.

**Acceptance:** Record/replay after mode flip has no orphan tool_call_output; prefixes before the change remain byte-identical.

- [ ] **Step 1 — RED:** Create a test named `script_to_direct_switch_preserves_history_and_wire_kind` in `crates/davinci-agent/tests/codemode_contract.rs` reproducing the acceptance condition with a disposable fixture. Ensure it fails against the current implementation for the *right reason*.
- [ ] **Step 2 — Record RED:** Run `cargo test -p davinci-agent script_to_direct_switch_preserves_history_and_wire_kind -- --nocapture` and capture the expected failed assertion (not a compile or environment error).
- [ ] **Step 3 — GREEN:** Implement the interface and required behavior above in only the named owner files; use existing components and add a focused file if needed, without duplicating the operation journal.
- [ ] **Step 4 — Verify GREEN:** Run `cargo test -p davinci-agent script_to_direct_switch_preserves_history_and_wire_kind -- --nocapture` and the relevant neighboring tests. Must pass; on a feature involving effects, perform a fresh subprocess/restart fixture too.
- [ ] **Step 5 — Review/commit:** Run `cargo fmt --all --check` and targeted `cargo clippy -p davinci-agent -p davinci-coding-agent -p davinci-ai --all-targets` for affected Rust crates. Inspect failure and security cases and commit **only if separately authorized**; otherwise leave diff for review.

### Task S17: Gate freeform custom Script and hosted PTC by actual Codex support

**Dependencies:** S07; S16; Codex route capability check before live wire use. **Commit gate:** independent reviewer approves each code/test change before it merges; do not accidentally mark this task complete by a green unrelated test.

**Files**
- Modify: `crates/davinci-ai/src/responses_tools.rs`
- Modify: `crates/davinci-ai/src/responses_request.rs`
- Modify: `crates/davinci-agent/src/codemode/execution.rs`
- Test: `crates/davinci-ai/tests/openai_cache_request_manifest.rs`
- Test: `crates/davinci-agent/tests/codemode_contract.rs`

**Interface:** `enum ScriptWireMode { FunctionJson, CustomFreeform, HostedProgrammatic }` (new provider-resolved preference, non-authoritative)

**Required behavior:** Offline mock tests for `custom_tool_call` JS `input`; `custom_tool_call_output` pairing, partial generation, retry, and function JSON fallback. Probe backend **only with separately approved budget**; public OpenAI PTC tool is not proof of subscription support. Preserve stable cached prefix and legacy consumers.

**Grounding:** Droid §19 custom freeform Script; current DaVinci `responses_tools.rs` special-cases only `apply_patch` custom grammar.

**Acceptance:** Rejected shapes cause graceful fallback with no duplicate tool effects; no freeform shape sent to unverified Codex route.

- [ ] **Step 1 — RED:** Create a test named `codex_freeform_fallback_is_safe` in `crates/davinci-ai/tests/openai_cache_request_manifest.rs` reproducing the acceptance condition with a disposable fixture. Ensure it fails against the current implementation for the *right reason*.
- [ ] **Step 2 — Record RED:** Run `cargo test -p davinci-ai codex_freeform_fallback_is_safe -- --nocapture` and capture the expected failed assertion (not a compile or environment error).
- [ ] **Step 3 — GREEN:** Implement the interface and required behavior above in only the named owner files; use existing components and add a focused file if needed, without duplicating the operation journal.
- [ ] **Step 4 — Verify GREEN:** Run `cargo test -p davinci-ai codex_freeform_fallback_is_safe -- --nocapture` and the relevant neighboring tests. Must pass; on a feature involving effects, perform a fresh subprocess/restart fixture too.
- [ ] **Step 5 — Review/commit:** Run `cargo fmt --all --check` and targeted `cargo clippy -p davinci-agent -p davinci-coding-agent -p davinci-ai --all-targets` for affected Rust crates. Inspect failure and security cases and commit **only if separately authorized**; otherwise leave diff for review.

### Task S18: Provide model-facing Script-first prompt and sensible direct fallback

**Dependencies:** S01; S06; S07; S13; S17. **Commit gate:** independent reviewer approves each code/test change before it merges; do not accidentally mark this task complete by a green unrelated test.

**Files**
- Modify: `crates/davinci-agent/src/prompt/tool_strategy.rs`
- Modify: `crates/davinci-agent/src/codemode/execution.rs`
- Modify: `crates/davinci-agent/src/prompt/manifest.rs`
- Test: `crates/davinci-agent/tests/default_prompt_baseline.rs`
- Test: `crates/davinci-agent/tests/codemode_projection.rs`

**Interface:** `ScriptModePromptProfile { mode, visible_tool_names, version }` (configuration contract; implementation in existing composer)

**Required behavior:** When controlled Script enabled, teach `tools.*`, raw input blocks, `text()/notify`, sequential edit→test, parallel reads, evidence preservation and one-Script mixed tool use. Do not hide tools needed for a single direct lookup. No prompt change when off; no falsely advertised `WaitForScript` until S13 passes.

**Grounding:** Droid §03 prompt guidance; current DaVinci read-only prompt says not for edits or shell.

**Acceptance:** Model-facing reference matches actual binding schemas, no phantom `riskLevel`, no hidden prompts on off route.

- [ ] **Step 1 — RED:** Create a test named `script_prompt_only_in_script_mode` in `crates/davinci-agent/tests/default_prompt_baseline.rs` reproducing the acceptance condition with a disposable fixture. Ensure it fails against the current implementation for the *right reason*.
- [ ] **Step 2 — Record RED:** Run `cargo test -p davinci-agent script_prompt_only_in_script_mode -- --nocapture` and capture the expected failed assertion (not a compile or environment error).
- [ ] **Step 3 — GREEN:** Implement the interface and required behavior above in only the named owner files; use existing components and add a focused file if needed, without duplicating the operation journal.
- [ ] **Step 4 — Verify GREEN:** Run `cargo test -p davinci-agent script_prompt_only_in_script_mode -- --nocapture` and the relevant neighboring tests. Must pass; on a feature involving effects, perform a fresh subprocess/restart fixture too.
- [ ] **Step 5 — Review/commit:** Run `cargo fmt --all --check` and targeted `cargo clippy -p davinci-agent -p davinci-coding-agent -p davinci-ai --all-targets` for affected Rust crates. Inspect failure and security cases and commit **only if separately authorized**; otherwise leave diff for review.

### Task S19: Exercise 12 Droid reference fixtures plus fault and security matrix

**Dependencies:** S04; S08; S10; S13; S14; S15. **Commit gate:** independent reviewer approves each code/test change before it merges; do not accidentally mark this task complete by a green unrelated test.

**Files**
- Modify: `crates/davinci-agent/tests/codemode_contract.rs`
- Modify: `crates/davinci-coding-agent/tests/codemode_supervisor.rs`
- Modify: `scripts/bench/tests/test_droid.py`
- Test: `crates/davinci-agent/tests/codemode_contract.rs`
- Test: `scripts/bench/tests/test_droid.py`

**Interface:** `tests/fixtures/droid_script/*.json` (new deterministic scripted-response scenarios; **do not reuse proprietary source**)

**Required behavior:** Test real nested read, command, parallel read, error, syntax, hostile input, head/tail output, sandbox escapes, long job, background notify, WaitForScript wait/kill, repeated polling, restart reconciliation and direct baseline. Extend with permission-race and symlink/junction cases.

**Grounding:** Droid supplied fixtures 01–12 and §19 actual CLI mock results.

**Acceptance:** All applicable tests pass offline on Windows; Linux/macOS CI or explicit unsupported declaration; recovery matrix independent of happy-path tests.

- [ ] **Step 1 — RED:** Create a test named `reference_script_fixture_suite_passes` in `crates/davinci-agent/tests/codemode_contract.rs` reproducing the acceptance condition with a disposable fixture. Ensure it fails against the current implementation for the *right reason*.
- [ ] **Step 2 — Record RED:** Run `cargo test -p davinci-agent reference_script_fixture_suite_passes -- --nocapture` and capture the expected failed assertion (not a compile or environment error).
- [ ] **Step 3 — GREEN:** Implement the interface and required behavior above in only the named owner files; use existing components and add a focused file if needed, without duplicating the operation journal.
- [ ] **Step 4 — Verify GREEN:** Run `cargo test -p davinci-agent reference_script_fixture_suite_passes -- --nocapture` and the relevant neighboring tests. Must pass; on a feature involving effects, perform a fresh subprocess/restart fixture too.
- [ ] **Step 5 — Review/commit:** Run `cargo fmt --all --check` and targeted `cargo clippy -p davinci-agent -p davinci-coding-agent -p davinci-ai --all-targets` for affected Rust crates. Inspect failure and security cases and commit **only if separately authorized**; otherwise leave diff for review.

### Task S20: Release opt-in mode with feature flags, docs and rollback

**Dependencies:** S19; S21; S22; B01–B02 offline safety/oracle evidence; live pilot follows opt-in rollout. **Commit gate:** independent reviewer approves each code/test change before it merges; do not accidentally mark this task complete by a green unrelated test.

**Files**
- Modify: `docs/codemode.md`
- Modify: `crates/davinci-coding-agent/src/codemode_host/config.rs`
- Modify: `crates/davinci-agent/src/codemode/execution.rs`
- Test: `crates/davinci-coding-agent/tests/codemode_cli.rs`

**Interface:** `CodemodeMode` user config roundtrip with `off|read-only|controlled`; provider preference set separately

**Required behavior:** Do not default-enable. Validate clean native Windows install, optional Node/missing binary behavior, telemetry, CLI help, setting migration, cancellation, versioned run state. Disable mode must not erase/replay unresolved actions. Link to benchmark plan for later promotion.

**Grounding:** Existing `docs/codemode.md` says read-only off by default; update only after code gate.

**Acceptance:** Off/read-only regression unchanged; controlled opt-in works with explicit host; rollback leaves valid sessions recoverable.

- [ ] **Step 1 — RED:** Create a test named `controlled_script_rollout_can_disable_without_history_loss` in `crates/davinci-coding-agent/tests/codemode_cli.rs` reproducing the acceptance condition with a disposable fixture. Ensure it fails against the current implementation for the *right reason*.
- [ ] **Step 2 — Record RED:** Run `cargo test -p davinci-coding-agent controlled_script_rollout_can_disable_without_history_loss -- --nocapture` and capture the expected failed assertion (not a compile or environment error).
- [ ] **Step 3 — GREEN:** Implement the interface and required behavior above in only the named owner files; use existing components and add a focused file if needed, without duplicating the operation journal.
- [ ] **Step 4 — Verify GREEN:** Run `cargo test -p davinci-coding-agent controlled_script_rollout_can_disable_without_history_loss -- --nocapture` and the relevant neighboring tests. Must pass; on a feature involving effects, perform a fresh subprocess/restart fixture too.
- [ ] **Step 5 — Review/commit:** Run `cargo fmt --all --check` and targeted `cargo clippy -p davinci-agent -p davinci-coding-agent -p davinci-ai --all-targets` for affected Rust crates. Inspect failure and security cases and commit **only if separately authorized**; otherwise leave diff for review.

### Task S21: Support explicitly permitted subagent/Task calls inside Script

**Dependencies:** S03; S09; explicit feature approval for child agents. **Commit gate:** independent reviewer approves each code/test change before it merges; do not accidentally mark this task complete by a green unrelated test.

**Files**
- Modify: `crates/davinci-agent/src/codemode/policy.rs`
- Modify: `crates/davinci-agent/src/codemode/dispatch.rs`
- Modify: `crates/davinci-agent/src/subagent.rs`
- Test: `crates/davinci-agent/tests/codemode_policy.rs`

**Interface:** `fn admit_script_subagent(request: &Value, parent: &CodeModeRunContext) -> Result<AuthorizedChild, CodeModeError>` (wrapper over existing subagent/worker budget)

**Required behavior:** Droid supports detached `Task` from Script behind an `allow-subagents-in-scripts` gate. Recreate as off by default: require explicit user/host authorization, depth/root budget and workspace ownership, host verification receipts and cancellation. Do not automatically fan out; keep `agent`, session control, or unrestricted team tools excluded.

**Grounding:** Droid source original 07 §4 and observed CLI flag; DaVinci parent-worker infrastructure exists.

**Acceptance:** Script cannot spawn workers when flag disabled, parent limit reached or session wrong. One authorized child receives bounded scope, verifies, and stops with the parent.

- [ ] **Step 1 — RED:** Create a test named `script_subagent_requires_user_authority_and_budget` in `crates/davinci-agent/tests/codemode_policy.rs` reproducing the acceptance condition with a disposable fixture. Ensure it fails against the current implementation for the *right reason*.
- [ ] **Step 2 — Record RED:** Run `cargo test -p davinci-agent script_subagent_requires_user_authority_and_budget -- --nocapture` and capture the expected failed assertion (not a compile or environment error).
- [ ] **Step 3 — GREEN:** Implement the interface and required behavior above in only the named owner files; use existing components and add a focused file if needed, without duplicating the operation journal.
- [ ] **Step 4 — Verify GREEN:** Run `cargo test -p davinci-agent script_subagent_requires_user_authority_and_budget -- --nocapture` and the relevant neighboring tests. Must pass; on a feature involving effects, perform a fresh subprocess/restart fixture too.
- [ ] **Step 5 — Review/commit:** Run `cargo fmt --all --check` and targeted `cargo clippy -p davinci-agent -p davinci-coding-agent -p davinci-ai --all-targets` for affected Rust crates. Inspect failure and security cases and commit **only if separately authorized**; otherwise leave diff for review.

### Task S22: Support low-overhead Todo/Plan bookkeeping within a real Script

**Dependencies:** S01; S03. **Commit gate:** independent reviewer approves each code/test change before it merges; do not accidentally mark this task complete by a green unrelated test.

**Files**
- Modify: `crates/davinci-agent/src/codemode/policy.rs`
- Modify: `crates/davinci-agent/src/codemode/dispatch.rs`
- Modify: `crates/davinci-agent/src/todo.rs`
- Test: `crates/davinci-agent/tests/codemode_policy.rs`

**Interface:** `tools.TodoWrite` alias maps to existing host-owned todo/update-plan action only if the tool is otherwise authorized and safe under the script owner policy.

**Required behavior:** Droid Script guidance pairs TodoWrite with real work to avoid a standalone bookkeeping turn. Allow an explicitly scoped, journalled todo update without granting session config or account controls. Preserve truthful progress and no forced extra model call.

**Grounding:** Droid runtime reference original 03 §1.1 and original 06 TodoWrite completion discipline.

**Acceptance:** A Script can update todo and read a relevant file together, but cannot forge user approval, erase required work, or spawn a planning-model review.

- [ ] **Step 1 — RED:** Create a test named `script_todo_update_shares_work_without_extra_turn` in `crates/davinci-agent/tests/codemode_policy.rs` reproducing the acceptance condition with a disposable fixture. Ensure it fails against the current implementation for the *right reason*.
- [ ] **Step 2 — Record RED:** Run `cargo test -p davinci-agent script_todo_update_shares_work_without_extra_turn -- --nocapture` and capture the expected failed assertion (not a compile or environment error).
- [ ] **Step 3 — GREEN:** Implement the interface and required behavior above in only the named owner files; use existing components and add a focused file if needed, without duplicating the operation journal.
- [ ] **Step 4 — Verify GREEN:** Run `cargo test -p davinci-agent script_todo_update_shares_work_without_extra_turn -- --nocapture` and the relevant neighboring tests. Must pass; on a feature involving effects, perform a fresh subprocess/restart fixture too.
- [ ] **Step 5 — Review/commit:** Run `cargo fmt --all --check` and targeted `cargo clippy -p davinci-agent -p davinci-coding-agent -p davinci-ai --all-targets` for affected Rust crates. Inspect failure and security cases and commit **only if separately authorized**; otherwise leave diff for review.

## End-of-subplan verification (not evidence that the plan already passed)

- [ ] Native Windows CLI run with managed Node host; Linux/macOS where supported; absent Node should not affect normal DaVinci.
- [ ] `cargo test -p davinci-agent codemode` plus `cargo test -p davinci-coding-agent codemode` plus `cargo test -p davinci-ai responses`; `node --test crates/davinci-coding-agent/codemode-host/*.test.mjs`.
- [ ] Full safety review including denial, symlink/junction escape, approval revocation, cancellation, ambiguous effect, process tree and output leak.
- [ ] Log actual CodeMode limits, processes and failures. Do not claim runtime parity from a passing JSON parser alone.
- [ ] Review/approve amendment to original C08 constraints before any detached persistent Script release. Output status reports unresolved work honestly.
