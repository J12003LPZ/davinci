# DaVinci Reliability & Efficiency Program — Luna Execution Plan

This plan is designed for **GPT-5.6 Luna** to execute efficiently: small, reviewable changes; explicit file targets; strict stop conditions; and only the tests needed to prove each invariant.

**Updated baseline:** `main @ 1e0471a4477fb6c05108db1dd0ad240e715d020a` (September 14, 2026), seven commits ahead of the original audited baseline.

Since the first version of this plan, DaVinci added two important systems that must be **preserved, not recreated**:

- **Capability Toolbox v1** for deterministic Graph-worker assembly of already-authorized role tools plus bounded memory/skill context.
- **Content-aware reversible Token Governor routing** for logs, JSON arrays, search results, and plain text, with exact original recovery through the existing `OutputStore`.

DaVinci also already has `ResourceBudget`/`ResourceLedger`, `RuntimeCapabilityRegistry`, stable/dynamic prompt composition, cache identities, verification-related run state, Graph learning, and ecosystem telemetry. The remaining work is therefore **integration, correctness, root-agent efficiency, recovery, and measurement**—not more subsystems.

---

## Plan Header for Luna

When Luna begins, save the working plan as:

`docs/superpowers/plans/2026-09-14-davinci-luna-reliability-efficiency.md`

```markdown
# DaVinci Reliability and Efficiency Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: use
> superpowers:executing-plans. Execute one task at a time.
> Do not spawn implementation subagents by default.
> Use superpowers:test-driven-development for behavioral changes,
> superpowers:systematic-debugging when a test/failure is not understood,
> and superpowers:verification-before-completion before completion claims.

**Goal:** Increase DaVinci's verified software-engineering success rate while
reducing unnecessary context, schemas, tool calls, retries, and resource use.

**Architecture:** Improve existing DaVinci primitives rather than introducing
new orchestration systems. Reuse ResourceLedger, RuntimeCapabilityRegistry,
Capability Toolbox v1, prompt/cache infrastructure, capability evidence,
content-aware Token Governor, scheduler, tool ledger, learning ledger, and eval
harness.

**Tech Stack:** Rust 1.83-compatible DaVinci workspace, existing pinned
dependencies, existing inline Rust test conventions.

**Baseline:** main @ 1e0471a4477fb6c05108db1dd0ad240e715d020a

## Global Constraints

- Do not introduce another task-budget subsystem.
- Do not introduce another capability/tool registry.
- Do not introduce another agent orchestration framework.
- Do not enable LSP globally as part of this program.
- Do not modify unrelated code.
- Do not add dependencies unless the standard library and existing dependencies are demonstrably insufficient.
- Never use a model call for deterministic verification.
- Preserve current permission boundaries and deny-over-allow behavior.
- Unknown tool effects remain conservative.
- Never silently discard authoritative project instructions to save tokens.
- Never replay a mutation whose outcome is uncertain.
- No live provider/network tests without explicit authorization.
- Every optimization must have a measurable before/after result.
- Preserve Capability Toolbox v1 as the single Graph-worker tool/context assembly layer.
- Preserve its current default bounds: <= 2,500 aggregate context tokens, <= 4 memory hits, <= 2 complete skills.
- Preserve duplicate-skill suppression, oversized-skill skipping, complete-skill-only injection, and exact skill version/hash provenance.
- Preserve existing Graph role permissions; Capability Toolbox must never become an authorization layer.
- Preserve content-aware Governor routing as a local, reversible optimization using the existing OutputStore.
- Preserve `content_aware=false` as the generic-Governor fallback.
- Do not add another content classifier, output store, Graph context packet builder, learning database, or coordinator model call.
- Keep protected/system prompt text unchanged; efficiency work may change accounting and provider-visible tool exposure, not silently rewrite prompt policy.
```

---

# Luna-Specific Execution Rules

1. **Read before editing.** Before every phase, inspect only the named files and directly related call sites.
2. **One phase = one concern.** Do not combine context, tools, verification, recovery, etc. in one patch.
3. **Do not improve unrelated code while there.**
4. **Prefer extending existing types over creating parallel abstractions.**
5. **Maximum 1–4 new tests per phase; prefer extending an existing test over adding a new one.**
6. **Do not run `cargo test --workspace` during individual phases.**
7. **Do not run the same passing test repeatedly unless relevant code changed afterward.**
8. **If the expected architecture already exists on current `main`, verify it and skip the implementation instead of recreating it.**
9. **Never treat compilation as proof of behavior.**
10. **Never treat an assistant response saying “fixed” as verification.**
11. **Do not spawn subagents unless the work is genuinely independent and read-only.**
12. **Commit after each independently correct phase.**

The final workspace-wide checks happen **once**, after all phases. The GitHub connector did not expose a combined status check for the latest merge commit during this plan update, so Luna must establish the executable baseline locally rather than assuming CI is green.

---

# Existing Capabilities to Preserve — Do Not Rebuild

Before Luna edits anything, treat these as **implemented architecture**, not plan TODOs:

| Existing capability | Current responsibility | What this plan should do |
|---|---|---|
| `ResourceLedger` / `ResourceBudget` | Whole-task token, retry, cost, concurrency, child-lease, verification/handoff reserves | Reuse; never create another budget manager |
| `RuntimeCapabilityRegistry` | Runtime tool identity, source, class, effects, schema hash/version | Extend only where execution/replay metadata is missing |
| Capability Toolbox v1 | Graph-worker assembly of **already-authorized** tools plus bounded memory/skill context | Preserve; do not use it as a second permission system or root-agent context manager |
| Graph context packet | <= 2,500 total estimated tokens, <= 1,200 memory tokens / 4 hits, <= 1,000 skill tokens / 2 skills | Preserve exact limits unless a separate measured proposal changes them |
| Hardened Graph skill selection | Duplicate names once; oversized skills skipped; skill bodies never partially injected | Reuse; do not add another skill selector |
| Existing learning ledger | Verified worker outcomes influence future skill ranking/version attribution | Reuse; audit only the verified-promotion boundary |
| Content-aware Token Governor | Local log/JSON/search/plain classification; specialized view only when smaller than generic | Preserve and measure |
| Existing `OutputStore` + `retrieve_output` | Exact reversible recovery for compressed/specialized output | Fix continuation correctness; do not add storage |
| Stable/dynamic prompt composer + cache identity | Stable prefix organization and reason-coded invalidation | Measure and connect to actual requests; do not create a second cache system |
| Existing ecosystem telemetry | Governor, cache, context, learning, Graph resource evidence | Extend only for metrics needed by acceptance gates |

## New-feature boundaries that matter to this plan

### Capability Toolbox v1 is Graph-specific

Capability Toolbox packages:

```text
Graph role's already-authorized tools
        +
bounded Vector Memory hits
        +
bounded verified skills
        ↓
CapabilitySelection { tools, context }
```

It does **not** solve root-agent provider tool-schema bloat, global `AGENTS.md` / `CLAUDE.md` request accounting, or normal-turn completion verification. Therefore Phases 2–4 below remain valid, but they must not duplicate Graph Toolbox behavior.

### Content-aware Governor is view selection, not retrieval paging

The new Governor path already:

```text
large successful output
        ↓
save exact original to existing OutputStore
        ↓
classify locally
        ↓
build specialized candidate
        ↓
compare with generic referenced view
        ↓
send smaller view
```

The current `retrieve_output` implementation still lacks intra-line continuation. An oversized first selected line can still return the same line prefix and recommend the same `startLine` again. Phase 1 therefore remains a required correctness fix.

### One small integration gap discovered in the new router

The new log classifier currently recognizes `bash` and `powershell` command output, while DaVinci also exposes `exec_command`. Luna should add `exec_command` to the existing log-like command path with one targeted regression assertion. This is an integration fix to the existing router, **not** a new routing system.

---

# Phase 0 — Establish a Trustworthy Baseline

## Objective

Start from the new post-feature baseline and prove the two newly added systems are present before touching them.

## Inspect

```text
git status --short
git rev-parse HEAD
```

Expected audited baseline:

```text
1e0471a4477fb6c05108db1dd0ad240e715d020a
```

If HEAD differs, Luna must re-locate symbols and inspect commits after this SHA before editing. Do **not** blindly apply line-number-based changes.

## Run four existing feature smoke tests once

These tests already exist. Do not create duplicates.

```bash
cargo test -p davinci-coding-agent capability_selection_preserves_tools_and_context_budget
cargo test -p davinci-coding-agent duplicate_skill_names_are_injected_once
cargo test -p davinci-coding-agent oversized_skill_is_skipped_instead_of_truncated
cargo test -p davinci-coding-agent content_router_classifies_and_reduces_supported_shapes
```

They establish that the current baseline contains:

- Capability Toolbox selection.
- Graph context budgeting.
- Duplicate skill suppression.
- Oversized-skill skipping without partial injection.
- Content-aware classification and specialized-view reduction.

## Reproduce the known current Governor lint/correctness area

Run:

```bash
cargo clippy -p davinci-coding-agent --all-targets -- -D warnings
```

Do not suppress the warning around `TokenGovernor::retrieve`. Repair the retrieval branch in Phase 1 so the lint fix and behavior fix are the same change.

## Do not run yet

Do **not** run:

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Those are final-gate commands, not per-phase development commands.

---

# Phase 1 — Harden the Existing Content-Aware Governor and Fix Retrieval Progress

## Why this is first

The new content-aware routing architecture is useful and should remain unchanged:

- Exact output is stored first.
- Specialized views are used only when smaller than the generic referenced view.
- `content_aware=false` falls back to the prior generic path.
- Lossless tools remain exempt.
- Routing stays local and deterministic.

The unresolved problem is below that layer: `retrieve_output` can still fail to progress through a single oversized line.

## Files

Modify:

```text
crates/davinci-coding-agent/src/native_extensions/token_governor.rs
crates/davinci-coding-agent/src/native_extensions/mod.rs
crates/davinci-coding-agent/src/native_extensions/content_router.rs
```

Do **not** create another router, store, compression format, or retrieval tool.

## Part A — Make retrieval continuation monotonic

Preserve:

```json
{
  "id": "...",
  "startLine": 1,
  "endLine": 20,
  "grep": "needle"
}
```

Add:

```json
{
  "lineByteOffset": 0
}
```

`lineByteOffset` applies only to `startLine`.

Return structured continuation metadata:

```json
{
  "tokenGovernor": {
    "outputId": "out-...",
    "truncated": true,
    "nextCursor": {
      "startLine": 1,
      "lineByteOffset": 47950
    }
  }
}
```

### Required invariant

For every successful truncated retrieval:

```text
next_cursor > current_cursor
```

or retrieval returns an explicit error.

For normal multi-line truncation:

```text
nextCursor.startLine = first omitted line
nextCursor.lineByteOffset = 0
```

For one oversized line:

```text
nextCursor.startLine = same line
nextCursor.lineByteOffset = bytes already returned from that line
```

Move offsets to valid UTF-8 boundaries. Never return the same cursor twice for successful continuation.

The cursor semantics must work identically for output IDs created by either:

```text
generic Governor view
specialized content-aware view
```

because both recover from the same exact `OutputStore`.

## Part B — Make the existing log classifier recognize `exec_command`

Do not add a second classifier.

Update the current `is_log_like` command-tool predicate so:

```text
bash
powershell
exec_command
```

all use the same existing command/output heuristics.

Preserve the current exclusions such as `git diff` / `git show`; do not compact patch/diff output as test logs merely because it came through `exec_command`.

## Minimal tests

Reuse the existing Governor tests rather than adding overlapping ones.

### Existing test 1 — extend oversized-line paging

Extend:

```rust
retrieval_caps_an_oversized_first_line
```

Its current fixture already uses a large multibyte `é` line, so one test can prove both UTF-8 safety and cursor progress.

Add assertions that:

```text
first page is truncated
first nextCursor.startLine == 1
first nextCursor.lineByteOffset > 0
second call uses that exact cursor
second call succeeds with valid UTF-8
second cursor is greater than the first, or retrieval finishes
second page is not identical to the first
```

Two pages are enough. Do not retrieve the entire stored output in the test.

### Existing test 2 — preserve ordinary paging and filtering

Update:

```rust
retrieval_is_paged_and_says_where_to_continue
```

Assert the structured `nextCursor` replaces the old same-line textual continuation contract while preserving:

```text
normal multi-line paging
grep filtering
absent-match behavior
missing-output error behavior
omitted lineByteOffset == zero
```

### Existing content-router test — extend, do not duplicate

Add **one assertion** to:

```rust
content_router_classifies_and_reduces_supported_shapes
```

Prove:

```text
tool = exec_command
command = cargo test ...
→ ContentKind::Log
```

Also keep the existing `git diff` plain-text assertion.

No additional Governor tests are required for this phase unless these existing tests cannot express the new cursor contract.

## Verification

Run only:

```bash
cargo test -p davinci-coding-agent retrieval_caps_an_oversized_first_line
cargo test -p davinci-coding-agent retrieval_is_paged_and_says_where_to_continue
cargo test -p davinci-coding-agent content_router_classifies_and_reduces_supported_shapes
cargo clippy -p davinci-coding-agent --all-targets -- -D warnings
```

## Commit

```bash
git add crates/davinci-coding-agent/src/native_extensions/token_governor.rs \
        crates/davinci-coding-agent/src/native_extensions/mod.rs \
        crates/davinci-coding-agent/src/native_extensions/content_router.rs
git commit -m "fix(governor): harden reversible retrieval and exec routing"
```

---

# Phase 2 — Finish Deferred Provider Tool-Schema Activation Without Replacing Capability Toolbox

This remains one of the highest-value root-agent token-efficiency changes, but its boundary is now explicit.

**Capability Toolbox v1 remains the Graph-worker authority for selecting already-authorized role tools and bounded memory/skill context. Phase 2 must not replace or fork it.**

The missing root/provider flow is:

```text
authorized executable tools
       ↓
small provider-visible schema subset
       ↓
tool_search
       ↓
authorized matches
       ↓
activate matched schema
       ↓
NEXT model request includes schema
```

## Scope

Implement provider-visible schema deferral for the **normal/root agent first**.

Do not modify Graph Toolbox context assembly in this phase.

If Graph workers later adopt schema deferral, their authority must remain:

```text
CapabilitySelection.tools
```

and schema exposure may only form a subset of that already-authorized set.

## Files

Primary:

```text
crates/davinci-agent/src/runtime/capabilities.rs
crates/davinci-agent/src/tools.rs
crates/davinci-agent/src/lib.rs
```

Then locate the single root-agent request-preparation path that converts active tools to provider schemas. Modify only that actual path.

Likely consumers may include:

```text
crates/davinci-agent/src/turn.rs
crates/davinci-agent/src/mcp.rs
crates/davinci-coding-agent/src/main.rs
```

Do **not** edit these automatically.

Do **not** implement schema exposure in:

```text
crates/davinci-coding-agent/src/native_extensions/ecosystem/capability.rs
crates/davinci-coding-agent/src/native_extensions/ecosystem/context.rs
```

Those files serve Graph Capability Toolbox, not root-agent schema deferral.

## Reuse `RuntimeCapabilityRegistry`

Do not create another tool catalog.

Add only discovery information genuinely missing from `RuntimeCapability`, for example:

```rust
pub description: String,
```

If activation requires retaining the schema and there is no authoritative existing lookup, add:

```rust
pub schema: serde_json::Value,
```

Otherwise reference the existing schema source instead of storing duplicates.

## Add provider-visible exposure state

Conceptually:

```rust
#[derive(Debug, Clone, Default)]
pub struct ToolExposureState {
    visible: BTreeSet<String>,
}
```

Required operations:

```rust
pub fn is_visible(&self, name: &str) -> bool;

pub fn activate_authorized(
    &mut self,
    names: impl IntoIterator<Item = String>,
    authorized: &BTreeSet<String>,
) -> Vec<String>;

pub fn retain_authorized(&mut self, authorized: &BTreeSet<String>);
```

Keep these boundaries separate:

```text
RuntimeCapabilityRegistry = what exists
permission / role policy = what may execute
ToolExposureState = which authorized schemas the provider currently sees
Capability Toolbox = Graph-worker assembly of role-authorized tools + bounded context
```

## Initial root-agent visible set

Do not delete tools.

Start with a compact set such as:

```text
read
grep
find
ls
exec_command
apply_patch
update_plan
agent
tool_search
retrieve_output     only when Governor recovery is needed
```

Reuse the existing `CODEX_HOT_TOOLS` concept where practical. Do not create a third independent hot-tool list.

## Improve `tool_search`

Search:

```text
name
description
source / namespace
```

Return no more than five best authorized matches.

Example:

```json
{
  "matches": [
    {
      "name": "code_references",
      "source": "builtin",
      "description": "Find symbol references...",
      "activated": true
    }
  ]
}
```

`tool_search` should activate matching **authorized** schemas directly so another model turn is not wasted on `activate_tool`.

### Security invariant

```rust
if !authorized_tools.contains(name) {
    do_not_activate(name);
}
```

Permission revocation must remove the tool from provider-visible exposure before the next request.

Discovery never modifies Graph role permissions or Capability Toolbox selection.

## Minimal tests

```rust
tool_search_activates_authorized_deferred_schema
tool_search_cannot_activate_denied_tool
tool_activation_changes_schema_identity_once
```

For the first test, generate a large fake registry programmatically and verify one required non-hot tool is absent initially, discoverable, activated, and present in the next schema projection.

Do not add Graph Toolbox tests here; its authorization/context behavior is already covered by existing tests.

## Verification

```bash
cargo test -p davinci-agent tool_search_activates_authorized_deferred_schema
cargo test -p davinci-agent tool_search_cannot_activate_denied_tool
cargo test -p davinci-agent tool_activation_changes_schema_identity_once
cargo check -p davinci-agent
cargo clippy -p davinci-agent --all-targets -- -D warnings
```

## Commit

```bash
git commit -am "feat(tools): defer root provider schemas behind authorized search"
```

---

# Phase 3 — Account for Root Request Context Without Duplicating Graph Context Toolbox

Capability Toolbox v1 already owns **Graph-worker** memory/skill context and its 2,500-token cap.

This phase is only about the **normal/root provider request**, including large repository instruction files and provider-visible request components.

## Do not modify Graph context behavior

Do not change these as part of root-context budgeting:

```text
crates/davinci-coding-agent/src/native_extensions/ecosystem/capability.rs
crates/davinci-coding-agent/src/native_extensions/ecosystem/context.rs
crates/davinci-coding-agent/src/native_extensions/learning/retrieval.rs
```

Preserve:

```text
<= 2,500 Graph context tokens
<= 4 memory hits
<= 2 complete skills
duplicate names injected once
oversized skills skipped
no partially injected skills
```

## Objective

Make a normal/root provider request explainable by contribution:

```text
stable system prompt
dynamic prompt/runtime state
AGENTS.md / CLAUDE.md
provider-visible tool schemas
normal-turn memory
skills/references outside Graph, if any
conversation
retrieved evidence/tool output
```

For each contribution record:

```text
estimated tokens
mandatory / important / deferred
stable / dynamic
source/provenance
content hash
```

This phase is **accounting first**. Do not change protected prompt strings.

## Add root request-context accounting

Prefer an existing request/context preparation module. Add a small data structure such as:

```rust
pub enum ContextPriority {
    Mandatory,
    Important,
    Deferred,
}

pub struct ContextContribution {
    pub source: String,
    pub stable: bool,
    pub priority: ContextPriority,
    pub estimated_tokens: u64,
    pub content_hash: String,
}

pub struct ContextBudgetReport {
    pub contributions: Vec<ContextContribution>,
    pub total_estimated_tokens: u64,
}
```

Do not add another Graph `ContextPacket`.

## Safe reductions

### Always preserve

```text
system authority
permission/security instructions
authoritative repository instructions
active task contract
latest user request
```

### Remove exact duplication

If two root-context sources have exactly identical content hashes, inject the body once while retaining both provenance labels.

This is a semantic no-op and is allowed by default.

### Measure before changing default behavior

Do not automatically summarize or silently truncate `AGENTS.md` / `CLAUDE.md`.

After accounting exists, measure which root sources dominate context. Only then may a later, separately measured change defer:

```text
low-confidence optional memory
old recoverable tool output
inactive skill/reference material
inactive tool schemas
exact duplicate content
```

Phase 2 should remove most avoidable schema overhead.

## Cache requirement

The existing stable/dynamic prompt composer and cache identity stay authoritative.

Required behavior:

```text
dynamic runtime-only change
→ stable prefix hash unchanged

stable prompt/instruction/tool-schema change
→ appropriate identity changes
```

Do not create another cache key system.

## Minimal tests

### Test 1

```rust
duplicate_root_context_content_is_included_once
```

Exact duplicate bodies only. Retain both provenance entries.

### Test 2

```rust
mandatory_root_context_is_never_dropped
```

Use a synthetic request budget and prove mandatory contributions remain represented.

### Test 3

```rust
dynamic_state_does_not_change_stable_prefix_hash
```

Do not add tests for Graph context limits here; they already exist.

## Verification

```bash
cargo test -p davinci-agent duplicate_root_context_content_is_included_once
cargo test -p davinci-agent mandatory_root_context_is_never_dropped
cargo test -p davinci-agent dynamic_state_does_not_change_stable_prefix_hash
cargo check -p davinci-agent
cargo clippy -p davinci-agent --all-targets -- -D warnings
```

## Commit

```bash
git commit -am "feat(context): account for root request context without prompt rewrites"
```

---

# Phase 4 — Make Normal Coding Completion Evidence-Driven

The goal is not to automatically run huge suites. The goal is to require evidence that appropriate verification occurred.

## Extend existing run state

```rust
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MutationVerificationState {
    pub mutation_generation: u64,
    pub verified_generation: Option<u64>,
    pub last_verification_succeeded: bool,
}
```

On successful `write`, `edit`, `apply_patch`, or `notebook_edit`:

```rust
mutation_generation += 1;
last_verification_succeeded = false;
```

On successful recognized verification:

```rust
verified_generation = Some(mutation_generation);
last_verification_succeeded = true;
```

On failed verification:

```rust
last_verification_succeeded = false;
```

Reuse the existing verification-command classifier.

## Completion states

```rust
pub enum CompletionEvidence {
    Verified,
    Unverified,
    VerificationFailed,
    NotRequired,
}
```

Rules:

```text
Read-only work
    → NotRequired

Mutation + successful verification after latest mutation
    → Verified

Mutation + no verification
    → Unverified

Mutation + verification failed
    → VerificationFailed
```

A later mutation invalidates earlier verification.

After one reminder, allow completion if necessary but classify it as `Unverified`, not `Verified`.

## Minimal tests

```rust
mutation_without_verification_requests_evidence
mutation_then_successful_verification_is_verified
later_mutation_invalidates_previous_verification
read_only_turn_does_not_require_verification
```

## Verification

```bash
cargo test -p davinci-agent mutation_without_verification_requests_evidence
cargo test -p davinci-agent mutation_then_successful_verification_is_verified
cargo test -p davinci-agent later_mutation_invalidates_previous_verification
cargo test -p davinci-agent read_only_turn_does_not_require_verification
cargo check -p davinci-agent
cargo clippy -p davinci-agent --all-targets -- -D warnings
```

## Commit

```bash
git commit -am "feat(agent): distinguish verified completion from normal stop"
```

---

# Phase 5 — Make Existing Runtime Tool Metadata Authoritative

DaVinci now has two differently scoped concepts whose names are similar:

```text
RuntimeCapabilityRegistry
  → global/runtime tool identity and executable metadata

Graph Capability Toolbox
  → per-worker assembly of already-authorized tools + bounded context
```

Do not merge them and do not create a third capability abstraction.

This phase extends `RuntimeCapability` only with execution properties that are still duplicated across scheduler, ledger, and recovery code.

## Files

Primary:

```text
crates/davinci-agent/src/runtime/capabilities.rs
crates/davinci-agent/src/scheduler.rs
crates/davinci-agent/src/tool_ledger.rs
```

Do not move Graph context selection into the registry.

Do not modify `native_extensions/ecosystem/capability.rs` unless a compile-time adapter is strictly required.

## Add only missing execution metadata

Reuse existing:

```text
ToolClass
DeclaredEffect
source
read_only
schema_hash
version
```

Add only properties not already authoritative:

```rust
pub enum ConcurrencyPolicy {
    ParallelSafe,
    SerialBarrier,
}

pub enum ReplayPolicy {
    SafeToReplay,
    ReconcileBeforeReplay,
    NeverAutoReplay,
}

pub enum OutputPolicy {
    Normal,
    Compressible,
    LosslessRequired,
}
```

Then extend the existing `RuntimeCapability`.

## Safe defaults

Unknown capability:

```text
read_only = false
concurrency = SerialBarrier
replay = NeverAutoReplay
output_policy = Normal
```

Never optimize an unknown tool by assuming safety.

## Representative defaults

```text
read / grep / find / ls
  ParallelSafe
  SafeToReplay

web_search / web_fetch
  ParallelSafe
  ReconcileBeforeReplay

edit / write / apply_patch
  SerialBarrier
  ReconcileBeforeReplay

exec_command / bash / powershell
  SerialBarrier
  NeverAutoReplay

tool_search
  ParallelSafe
  SafeToReplay

agent
  preserve explicit existing worker behavior
```

The content-aware Governor remains responsible for output view selection. `OutputPolicy` should tell the Governor whether output is eligible; it must not implement a second compression router.

## Migrate consumers gradually

1. Scheduler lane selection.
2. Tool-ledger/recovery replay policy.
3. Governor eligibility only if this removes an existing duplicated list without changing behavior.

Do not simultaneously rewrite user-facing permission classes.

## Minimal tests

Use one table-driven test:

```rust
representative_tools_have_consistent_execution_metadata
```

Cover:

```text
read
edit
exec_command
web_search
tool_search
agent
read-only MCP
unknown
```

Add one explicit fail-closed test:

```rust
unknown_tool_defaults_to_serial_nonreplayable_mutation
```

Do not duplicate Capability Toolbox tests.

## Verification

```bash
cargo test -p davinci-agent representative_tools_have_consistent_execution_metadata
cargo test -p davinci-agent unknown_tool_defaults_to_serial_nonreplayable_mutation
cargo check -p davinci-agent
cargo clippy -p davinci-agent --all-targets -- -D warnings
```

## Commit

```bash
git commit -am "refactor(runtime): centralize executable tool metadata"
```

---

# Phase 6 — Stream Bounded Text Reads Instead of Loading Entire Files

## File

```text
crates/davinci-agent/src/tools.rs
```

If necessary, extract only the read implementation:

```text
crates/davinci-agent/src/tools/read.rs
```

Do not reorganize all tools.

## Helper

```rust
struct TextWindow {
    content: String,
    first_line: usize,
    lines_returned: usize,
    truncated: bool,
}

fn read_text_window(
    path: &Path,
    offset: usize,
    limit: usize,
    max_bytes: usize,
) -> Result<TextWindow, ToolError>;
```

Use:

```rust
std::fs::File
std::io::BufReader
std::io::BufRead
```

Stop reading when requested lines are satisfied or output byte limit is reached.

Preserve:
- images
- notebooks
- binary handling
- BOM/UTF-8 behavior
- existing line numbering
- existing truncation wording where contractually tested

## Minimal tests

```rust
read_large_text_file_returns_requested_window
read_window_preserves_utf8_at_byte_limit
```

Do not add timing or RSS unit tests.

## Verification

```bash
cargo test -p davinci-agent read_large_text_file_returns_requested_window
cargo test -p davinci-agent read_window_preserves_utf8_at_byte_limit
cargo clippy -p davinci-agent --all-targets -- -D warnings
```

## Commit

```bash
git commit -am "perf(read): stream bounded text windows"
```

---

# Phase 7 — Prevent Unsafe Replay After Crashes or Uncertain Outcomes

Reuse Phase 5 replay policy. Do not create a new session architecture.

## Attempt states

```rust
pub enum AttemptOutcome {
    NotStarted,
    StartedUnknown,
    Succeeded,
    Failed,
}
```

Persist:
- attempt ID
- tool
- argument digest
- replay policy
- outcome
- optional pre/post state hash

## Recovery rules

```text
NotStarted
  → may execute normally

Failed
  → retry only if ordinary retry rules permit

Succeeded
  → never execute merely for recovery

StartedUnknown + SafeToReplay
  → may retry

StartedUnknown + ReconcileBeforeReplay
  → reconcile state first

StartedUnknown + NeverAutoReplay
  → stop and require explicit reconciliation/operator decision
```

## Minimal tests

```rust
uncertain_safe_read_can_be_replayed
uncertain_mutation_requires_reconciliation
confirmed_failure_can_retry_within_budget
```

If persistence format changes, add one round-trip serialization test.

## Verification

```bash
cargo test -p davinci-agent uncertain_safe_read_can_be_replayed
cargo test -p davinci-agent uncertain_mutation_requires_reconciliation
cargo test -p davinci-agent confirmed_failure_can_retry_within_budget
cargo check -p davinci-agent
cargo clippy -p davinci-agent --all-targets -- -D warnings
```

## Commit

```bash
git commit -am "fix(recovery): block replay of uncertain mutations"
```

---

# Phase 8 — Measure Existing Agent Modes; Keep Capability Toolbox as the Single Graph Worker Assembler

Do **not** add more orchestration.

DaVinci already has:

```text
normal agent
subagents
Graph
workflows
Capability Toolbox v1
learning
ResourceLedger
```

Capability Toolbox now provides the Graph-worker handoff boundary:

```text
role policy / authorized tools
          +
bounded memory + complete verified skills
          ↓
CapabilitySelection
          ↓
Graph worker
```

Do not add another Graph worker prompt/context/tool assembler.

## Default execution policy

```text
Simple/local task
    → one root agent

Broad independent read-only investigation
    → optional subagent

Large/risky multi-stage implementation
    → Graph

Explicit user request for Graph/team
    → requested mode
```

## Measure before changing routing

For the internal evaluation corpus, record:

```text
mode
verified outcome
total task tokens/cost
wall time
root turns
worker count
Graph context estimated tokens
memory hits
skills injected
skill_candidates_considered
Governor specialized/generic output counts
```

Use existing ecosystem and resource telemetry where those values already exist.

If a metric already exists, do not add another copy.

## Capability Toolbox preservation checks

No new implementation is required unless one of these existing invariants regresses:

```text
CapabilitySelection.tools == already-authorized role tool set
context <= configured cap
default context <= 2,500 estimated tokens
memory hits <= 4
skills <= 2
skill names unique
skill body complete or omitted
exact skill version/hash refs retained
zero coordinator/preparation model calls
```

Use the existing tests first. Add a new test only if a future change creates an uncovered regression.

## Resource accounting

Use the existing root `ResourceLedger`.

Do not create:

```text
SubagentBudgetManager
GlobalBudgetV2
TeamBudget
GraphBudget2
CapabilityToolboxV2
```

---

# Phase 9 — Audit Learning Promotion Only; Preserve Hardened Skill Selection

Capability Toolbox v1 and the new retrieval hardening already implement:

```text
duplicate skill names injected once
oversized skills skipped
smaller relevant skills can still be considered
skill bodies never partially injected
<= 2 skills under the existing Graph budget
exact version/hash provenance
existing learning ledger reused
```

Do not build another skill selector, database, reviewer, or ranking store.

## Single audit question

> Can a learned procedural skill be promoted or treated as successful without evidence of a verified successful outcome for the exact skill version/hash?

Inspect the existing learning promotion/outcome path only.

If the answer is **no**, make:

```text
zero production code changes
zero new tests
```

Record the existing source/test proving the invariant and move on.

If the answer is **yes**, fix only the promotion boundary.

Required evidence should bind:

```text
task/run
skill name
skill version
content hash
verification outcome
verification evidence reference/hash where available
```

Repeated verified failures may influence existing ranking/demotion policy, but do not invent a second lifecycle system in this phase.

## Minimal test only if a gap exists

```rust
unverified_skill_version_cannot_be_promoted
```

One regression test is enough.

---

# Phase 10 — Measurement and A/B Evaluation

Do not benchmark after every code edit.

Complete Phases 1–7 first, then evaluate the integrated system.

## Primary metric

\[
\text{Cost per verified success}
=
\frac{\text{total cost across all attempts}}
{\text{number of independently verified successful tasks}}
\]

Total task cost includes:

```text
root model calls
provider retries
compaction
subagents
Graph workers
review calls
learning calls attributable to the task
```

Do not exclude failed attempts.

## Existing new-feature metrics to reuse

### Content-aware Governor

Reuse existing Governor/ecosystem telemetry for:

```text
content kind counts
specialized view count
generic view count
original bytes
model-view bytes
estimated byte reduction %
retrieval count
compressed output count
```

Important:

> **Byte reduction is not automatically token reduction.**

Do not report the existing byte-reduction estimate as provider-billed token savings. Correlate it with actual provider usage when available.

### Capability Toolbox

Reuse existing Graph context evidence for:

```text
estimated context tokens
memory token count / hit count
skill token count / injected skill count
skill_candidates_considered
context fingerprint
skill version/hash refs
```

Do not add parallel telemetry fields when equivalent values already exist.

## Also record

```text
verified success rate
first-attempt verified success rate
input tokens
cached-input tokens
output tokens
reasoning tokens where reported
cache-write tokens where reported
provider attempts
model turns
tool calls
subagents/workers
wall time
p50 / p95
files changed
scope violations
permission failures
human interventions
false success claims
```

## Required feature ablations

These are experiments, not new production subsystems.

### A. Generic vs content-aware Governor

Run the same large-output tasks with:

```text
content_aware = false
content_aware = true
```

Keep all other settings identical.

Compare:

```text
verified success
provider input tokens where reported
model-view bytes
retrieval frequency
wall time
tool calls
```

The specialized router stays default only if it does not reduce correctness and demonstrates useful efficiency on relevant workloads.

### B. Root tool-schema deferral

After Phase 2:

```text
full authorized root schemas
vs
hot/deferred root schemas
```

Measure schema bytes/tokens, tool discovery success, provider input, latency, and verified outcome.

Do not use this ablation to modify Graph role authorization.

### C. Capability Toolbox context contribution

For selected Graph tasks only, compare the existing context path with its existing context-disable control when safe:

```text
Graph context enabled
Graph context disabled
```

Measure verified success and total cost.

This is only to quantify value. Do not create a new context mechanism.

---

# Small Development Evaluation Suite

Use only **12–16 controlled tasks**:

| Category | Tasks | What it stresses |
|---|---:|---|
| Small bug fixes | 4 | normal coding loop + verification |
| Navigation/refactor | 4 | search/read/tool efficiency |
| Large context/tool catalog | 4 | deferred schemas + context budget |
| Recovery/failure | 0–4 | retry/replay/false-success behavior |

Every task needs a deterministic oracle. No model judges whether the task passed.

---

# Required A/B Comparisons

Compare:

```text
baseline DaVinci
new DaVinci
```

Run once initially.

If results are promising, run three times per task before release decisions.

Do not immediately run 10 repetitions.

---

# Competitor Comparison After Internal Acceptance

Then compare:

```text
DaVinci
OpenAI Codex
Claude Code
Hermes Agent
OpenCode
```

Keep separate:

## Harness comparison

Use the same model where technically possible.

Question:

```text
Which harness architecture is stronger?
```

## Product comparison

Use each product's intended configuration.

Question:

```text
Which complete system performs best?
```

Never combine those claims.

---

# Acceptance Gates

## New-feature preservation gate

Capability Toolbox v1 must continue to satisfy:

```text
no new permissions granted
authorized role tools preserved
default Graph context <= 2,500 estimated tokens
memory hits <= 4
skills <= 2
duplicate skill names injected once
oversized skills skipped instead of truncated
skill bodies complete or omitted
exact skill version/hash provenance retained
zero added coordinator/preparation model calls
```

Content-aware Governor must continue to satisfy:

```text
exact original stored before model-view reduction
specialized view selected only when smaller than generic view
content_aware=false returns to generic referenced view
lossless tools remain lossless
retrieve_output remains available for compressed Graph workers
prompts, memory packets, permissions, and provider request policy are not rewritten by routing
routing telemetry remains local/deterministic
```

## Reliability gate

Must have:

```text
0 known unsafe replay cases in controlled recovery suite
0 false verified-success results in controlled verification cases
0 permission elevation through tool_search
token-governor continuation always progresses
UTF-8 retrieval cursor never splits a character
exec_command test/build output follows existing log-routing policy
```

## Tool efficiency gate

On a synthetic large-tool root-agent fixture:

```text
initial provider-visible schema bytes reduced by at least 30%
```

while the required deferred tool remains discoverable, authorized, and callable.

This gate is for Phase 2 root-schema deferral. It does **not** require replacing Capability Toolbox.

## Context gate

Root-agent context work must show:

```text
no mandatory instruction loss
protected prompt text unchanged
stable prefix unchanged when only dynamic state changes
exact duplicate root context not repeatedly injected
Graph Capability Toolbox context limits unchanged
```

## Governor efficiency gate

For content-aware A/B tasks:

```text
specialized path never loses exact recoverability
verified success does not regress
model-view bytes decrease on supported large-output shapes
actual token/cost claims use provider usage, not byte estimates alone
```

Do not require every output to use a specialized view. Generic fallback is correct whenever it is smaller or classification is plain text.

## Success gate

Never trade correctness for token savings.

If:

```text
tokens ↓ 30%
verified success ↓ 10%
```

reject the optimization.

## Multi-agent gate

Keep additional delegation only if:

```text
verified success improvement justifies increased total cost/latency
```

Do not judge it by trace complexity or number of workers.

---

# Testing Policy for Luna

During each phase:

```text
1. Add the smallest regression test.
2. Run exactly that test and confirm failure.
3. Implement the smallest change.
4. Run exactly that test again.
5. Run the other 1–3 tests for that phase.
6. cargo check only if interfaces/types changed.
7. crate-level clippy once.
8. commit.
```

Do **not** do this after every phase:

```bash
cargo test --workspace
cargo clippy --workspace --all-targets
cargo build --workspace
```

---

# Final Verification — Once

After all selected implementation phases are complete:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
```

Then run the repository's canonical required workspace/behavioral command **once**.

If current CI requires the ecosystem filters as separate release gates, run each required filter once:

```bash
cargo test -p davinci-coding-agent ecosystem_loop_ -- --nocapture
cargo test -p davinci-coding-agent ecosystem_invariants_ -- --nocapture
cargo test -p davinci-coding-agent runtime_migration_preserves_ecosystem_baseline -- --nocapture
cargo test -p davinci-coding-agent ecosystem_telemetry -- --nocapture
```

Do not repeat these locally merely because `cargo test --workspace` already ran them unless the separate invocation is needed as explicit release evidence.

Finally:

```bash
git diff --check
git status --short
```

Then run the small internal A/B evaluation.

Do **not** rerun the full workspace because only documentation or benchmark reports changed afterward.

---

# Stop Conditions for Luna

Stop a phase rather than expanding scope when:

```text
The required subsystem already exists and satisfies the invariant.
The failure is unrelated to files modified in the phase.
The change requires changing a public compatibility contract not included here.
A proposed optimization would weaken permissions or verification.
A new dependency seems necessary.
The change starts requiring a second independent subsystem.
```

Report the blocker instead of “fixing everything around it.”

---

# Explicit Things Luna Should NOT Build

```text
another Graph system
another generic agent planner
another task-budget manager
another vector-memory database
another capability registry
another cache identity system
another Capability Toolbox
another Graph context packet builder
another content-aware output router
another output storage/recovery system
another global prompt
another LLM verification agent
a universal always-on LSP layer
automatic subagents for every task
automatic learning review for every task
LLM-powered context summarization just to reduce AGENTS.md
```

The existing architecture already has enough machinery. The goal is to **connect it correctly and prove it helps**.

---

# Recommended Execution Order

```text
0. Confirm latest baseline + preserve new feature smoke tests
        ↓
1. Governor retrieval cursor + exec_command routing compatibility
        ↓
2. Deferred root provider tool-schema activation
        ↓
3. Root request accounting + cache stability
        ↓
4. Evidence-driven normal completion
        ↓
5. Central runtime execution/replay metadata
        ↓
6. Bounded large-file reads
        ↓
7. Safe uncertain-outcome recovery
        ↓
8. Internal A/B benchmark, including Governor/tool-schema ablations
        ↓
9. Audit Graph mode value + learning promotion boundary
        ↓
10. Competitor benchmark
```

Do **not** begin benchmark/optimization conclusions until Phases 1–7 pass.

Capability Toolbox v1 and content-aware Governor routing are **inputs to this program now**, not TODOs to rebuild.

---

# Core Instruction for Luna

> **Do not make DaVinci larger unless necessary. Capability Toolbox v1 and content-aware Token Governor routing already exist—preserve them and do not recreate their responsibilities. Prefer wiring, simplifying, and measuring existing systems. For every change, identify one concrete invariant, reuse an existing test when possible, add only the minimum missing regression test, implement the smallest change, and stop when the invariant passes. Never run a broad test suite when a targeted test can prove the current step. A feature is complete only when the real execution path uses it and deterministic evidence proves the behavior.**

---

# Execution Record — 2026-09-14

The plan was executed in the isolated worktree
`C:\Users\sergi\.claude-worktrees\pi-rust-9416e5cee6\codex-luna-reliability-20260914`
from baseline `1e0471a4477fb6c05108db1dd0ad240e715d020a`. No implementation
subagents were used, per the task instruction. The shared checkout was not
edited.

## Phase commits

1. `bf012ac` — `fix(governor): harden reversible retrieval and exec routing`
2. `06156c7` — `feat(tools): defer root provider schemas behind authorized search`
3. `268555e` — `feat(context): account for root request context without prompt rewrites`
4. `896df36` — `feat(agent): distinguish verified completion from normal stop`
5. `c969104` — `refactor(runtime): centralize executable tool metadata`
6. `f536294` — `perf(read): stream bounded text windows`
7. `0101ec9` — `fix(recovery): block replay of uncertain mutations`
8. `2172b92` — `fix(learning): bind promotion to verified skill versions`
9. `d25ed22` — `test(eval): record deterministic reliability ablations`
10. `b89ce9c` — `test(graph): align skill cap fixture with budget`

The learning audit found a real gap: the normal caller could record an
outcome by skill name alone and promotion could target the latest version.
That path now resolves and records the exact content hash/version, and
promotion occurs only after the exact version accepts the verified outcome.

## Deterministic validation

- Governor retrieval, UTF-8 continuation, `exec_command` routing, tool-schema
  activation/denial, root context accounting, completion evidence, execution
  policy, tool-ledger recovery, bounded reads, and exact learning promotion
  regression tests passed in their phase-specific runs.
- Capability Toolbox preservation filters passed: ecosystem loop `12`,
  invariants `2`, runtime migration `2`, telemetry `8`, capability selection
  `2`, duplicate skill names `2`, and oversized-skill omission `2` tests.
- `cargo run -p davinci-evals -- corpus verify` passed.
- `cargo test -p davinci-evals` passed with `151` tests across its suites.
- The deterministic Governor ablation ran `12` identical large-output tasks
  in generic and content-aware modes. Generic views totaled `18,140` bytes;
  specialized views totaled `9,438` bytes; all `12` outputs were exactly
  recoverable; the generic path selected `12/12` generic views and the
  content-aware path selected `12/12` specialized views.
- The root schema ablation exposed `9` deferred tools initially versus `30`
  when fully exposed. Serialized provider schemas were `5,274` versus
  `17,163` bytes, satisfying the `>=30%` reduction gate while preserving
  authorized discovery.
- A complete `davinci-agent` package sweep passed `847` tests. The first
  `davinci-coding-agent` sweep exposed four missing-debug-binary prerequisites
  and a stale Graph-skill-cap fixture. The required debug binary was built,
  the security CLI/RPC checks passed (`6` and `2`), and the fixture was
  corrected without changing production Graph selection. The final package
  sweep passed `2,040` tests with `6` ignored.

## Provider and competitor boundary

The canonical provider-backed behavior command was attempted and failed
closed before execution because no provider was authorized/configured:
`provider is required (--provider or DAVINCI_PROVIDER/PI_PROVIDER)`. Therefore
provider input/output tokens, billed cost per verified success, wall-time
comparisons, three-run task A/Bs, and the external Codex/Claude
Code/Hermes/OpenCode comparison remain unmeasured. The byte measurements above
are not token or cost claims. No external credentials, networked provider, or
competitor harness was assumed.
