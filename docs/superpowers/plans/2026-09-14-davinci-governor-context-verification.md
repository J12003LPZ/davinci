# Governor, Context, Verification, and Recovery Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make DaVinci's existing Governor, root request shaping, verification evidence, and recovery ledger more precise without changing prompt authority or adding parallel systems.

**Architecture:** Extend the current `TokenGovernor`, `RootContextAccount`, mutation-verification state, `RuntimeCapabilityRegistry`, and `ToolCallLedger`. Preserve exact-output recovery and fail-closed recovery semantics; optional context may be deferred, mandatory authority may not.

**Tech Stack:** Rust 1.83, serde/serde_json, std filesystem primitives, existing davinci-agent/davinci-coding-agent test conventions.

**Spec:** `docs/superpowers/specs/2026-09-14-davinci-harness-optimization-design.md`

## Global Constraints

- No model call for compression, context selection, verification, or recovery.
- `LosslessRequired` tools stay lossless.
- Exact Governor originals must be stored before a compressed error view is returned.
- Unknown tools stay conservative.
- Do not change built-in prompt text merely to implement request budgeting.
- Do not run workspace-wide tests after every task.

---

### Task 1: Reversible compression for large error outputs

**Files:**
- Modify: `crates/davinci-coding-agent/src/native_extensions/token_governor.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/content_router.rs`

**Interfaces:**
- Consumes: `tool_may_be_compressed(name: &str) -> bool`, `OutputStore::save`, `compress_with_reference`, `build_specialized_view`.
- Produces: existing `TokenGovernor::after_tool` behavior extended so large error results may be compressed while preserving `ToolResult.is_error = true`.

- [ ] **Step 1: Write the failing regression test**

Add to `token_governor.rs`:

```rust
#[test]
fn large_error_output_is_reversibly_compressed() {
    let dir = tempdir().unwrap();
    let mut governor = TokenGovernor::with_store(
        "test",
        tiny_thresholds(),
        OutputStore::new(dir.path()),
    );
    let original = (0..500)
        .map(|i| format!("error: compile failure {i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let result = governor.after_tool(
        "exec_command",
        &json!({"command":"cargo check"}),
        ToolResult { content: original.clone(), is_error: true, details: None },
    );
    assert!(result.is_error);
    assert!(result.content.len() < original.len());
    let id = result.details.as_ref().unwrap()["tokenGovernor"]["outputId"]
        .as_str().unwrap();
    let recovered = governor.retrieve(&json!({"id": id})).unwrap();
    assert!(recovered.content.contains("compile failure 499"));
}
```

- [ ] **Step 2: Run the test and confirm it fails because `result.is_error` returns before compression**

```bash
cargo test -p davinci-coding-agent large_error_output_is_reversibly_compressed
```

- [ ] **Step 3: Split the early-return conditions**

Change `after_tool` so Governor-originated `skip=true` still returns immediately, but `result.is_error` only bypasses dedupe/anti-loop accounting that is unsafe for failures; compressibility is decided later through `tool_may_be_compressed` and the normal threshold probe.

The production shape should remain equivalent to:

```rust
let governor_skip = result.details.as_ref()
    .and_then(|d| d.get("tokenGovernor"))
    .and_then(|d| d.get("skip"))
    .and_then(Value::as_bool) == Some(true);
if governor_skip {
    return result;
}
```

Do not clear `result.is_error` when replacing `result.content`.

- [ ] **Step 4: Run the regression test and the existing Governor reversibility test**

```bash
cargo test -p davinci-coding-agent large_error_output_is_reversibly_compressed
cargo test -p davinci-coding-agent compression_preserves_notable_lines_and_is_reversible
```

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-coding-agent/src/native_extensions/token_governor.rs
git commit -m "feat(governor): compress large errors reversibly"
```

---

### Task 2: Add deterministic content shapes and minimum specialized-view win

**Files:**
- Modify: `crates/davinci-coding-agent/src/native_extensions/content_router.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/token_governor.rs`

**Interfaces:**
- Extend `ContentKind` with: `CompilerDiagnostics`, `TestOutput`, `Ndjson`, `JsonObject`, `Tree`, `Table`.
- Add config field:

```rust
#[serde(default = "default_specialized_min_reduction_pct")]
pub specialized_min_reduction_pct: u8;
```

Default: `10`.

- [ ] **Step 1: Add one table-driven classifier test**

Use fixtures for:
- `cargo check` diagnostic output;
- `cargo test` output;
- 20-line NDJSON;
- JSON object with a large `items` array and `errors` array;
- indented tree output;
- aligned pipe/tabular output.

Assert each maps to its expected `ContentKind` and ordinary prose remains `PlainText`.

- [ ] **Step 2: Run only the classifier test and confirm the new variants do not exist yet**

```bash
cargo test -p davinci-coding-agent content_router_classifies_extended_shapes
```

- [ ] **Step 3: Implement deterministic classifiers and bounded renderers**

Rules:

```text
CompilerDiagnostics: shell tool + compiler markers (`error[E`, `-->`, `warning:`) with at least two signals.
TestOutput: shell tool + known test command or test-result markers.
Ndjson: at least 8 non-empty lines, >=80% individually parse as JSON values.
JsonObject: top-level object with a large array field or diagnostics-like fields.
Tree: >=12 lines with repeated indentation/tree glyph structure.
Table: >=8 lines with stable delimiter count (`|` or tab) across most rows.
```

Renderers must:
- preserve first/last context;
- prefer diagnostic rows/records;
- include exact `retrieve_output` instructions;
- return `None` when the specialized view is not meaningfully structured.

- [ ] **Step 4: Add the minimum-win test**

```rust
#[test]
fn specialized_view_must_clear_minimum_reduction_threshold() {
    // Configure a specialized renderer whose output is only ~5% smaller.
    // Assert Governor chooses strategy="generic" at default 10%.
}
```

- [ ] **Step 5: Implement the choice rule**

Use integer arithmetic to avoid float drift:

```rust
fn clears_specialized_threshold(specialized: usize, generic: usize, pct: u8) -> bool {
    specialized < generic
        && specialized.saturating_mul(100)
            <= generic.saturating_mul(100u8.saturating_sub(pct) as usize)
}
```

- [ ] **Step 6: Run the focused tests**

```bash
cargo test -p davinci-coding-agent content_router_classifies_extended_shapes
cargo test -p davinci-coding-agent specialized_view_must_clear_minimum_reduction_threshold
cargo test -p davinci-coding-agent content_router_classifies_and_reduces_supported_shapes
```

- [ ] **Step 7: Commit**

```bash
git add crates/davinci-coding-agent/src/native_extensions/content_router.rs \
        crates/davinci-coding-agent/src/native_extensions/token_governor.rs
git commit -m "feat(governor): expand deterministic content routing"
```

---

### Task 3: Track per-kind retrieval feedback

**Files:**
- Modify: `crates/davinci-coding-agent/src/native_extensions/token_governor.rs`

**Interfaces:**
- Extend stored manifest entry with content kind and model-view strategy.
- Extend `ContentRoutingStats` with per-kind retrieval counters.

Use a compact serializable shape rather than a map keyed by arbitrary strings:

```rust
#[derive(Default, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct KindRetrievalStats {
    pub compressed: u64,
    pub specialized: u64,
    pub original_bytes: u64,
    pub model_view_bytes: u64,
    pub retrievals: u64,
}
```

- [ ] **Step 1: Write a failing test**

Compress one log and one JSON array, retrieve only the JSON output, then assert status reports:

```text
log.retrievals == 0
jsonArray.retrievals == 1
```

- [ ] **Step 2: Run it and confirm failure**

```bash
cargo test -p davinci-coding-agent governor_tracks_retrievals_by_content_kind
```

- [ ] **Step 3: Persist kind/strategy in `StoredOutputEntry` and increment the matching counter in `retrieve`**

Do not infer kind by reparsing the stored body on retrieval; use the manifest metadata recorded at compression time.

- [ ] **Step 4: Run the focused test and status serialization test**

```bash
cargo test -p davinci-coding-agent governor_tracks_retrievals_by_content_kind
cargo test -p davinci-coding-agent governor_status
```

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(governor): report retrieval feedback by content kind"
```

---

### Task 4: Make root context budgeting affect provider-visible optional context

**Files:**
- Modify: `crates/davinci-agent/src/context.rs`
- Modify: `crates/davinci-agent/src/lib.rs`
- Test: existing inline context/lib tests

**Interfaces:**
- Add:

```rust
pub struct SelectedRootContext {
    pub report: ContextBudgetReport,
    pub repository_files: Vec<ContextFile>,
    pub ephemeral_messages: Vec<ChatMessage>,
}
```

- Add method:

```rust
pub fn select_root_context(&self, budget: u64) -> SelectedRootContext;
```

Mandatory project instruction files remain selected. Ephemeral context is selectable/deferable according to provenance supplied by the host; default existing ephemeral entries remain `Important`, not silently `Mandatory`.

- [ ] **Step 1: Add a failing provider-view test**

Build an agent with:
- mandatory system prompt;
- mandatory `AGENTS.md`;
- one large optional ephemeral context message.

Apply a tiny root budget and assert `provider_system_prompt()` still contains `AGENTS.md` content while `messages_for_provider()` omits the optional message selected=false.

- [ ] **Step 2: Confirm failure**

```bash
cargo test -p davinci-agent root_budget_shapes_provider_view_without_dropping_authority
```

- [ ] **Step 3: Wire selection into provider projection**

Do not mutate persisted `self.messages`. Store request-local selected ephemeral indexes or produce a selected projection directly. Persisted conversation/session evidence stays complete.

- [ ] **Step 4: Run context tests**

```bash
cargo test -p davinci-agent context::
cargo test -p davinci-agent root_budget_shapes_provider_view_without_dropping_authority
```

- [ ] **Step 5: Commit**

```bash
git add crates/davinci-agent/src/context.rs crates/davinci-agent/src/lib.rs
git commit -m "feat(context): enforce optional root request budget"
```

---

### Task 5: Add deterministic scoped repository instructions

**Files:**
- Modify: `crates/davinci-agent/src/context.rs`
- Modify only actual caller(s) that populate `Agent.context_files`

**Interfaces:**
- Add:

```rust
pub fn load_context_files_for_targets(
    cwd: &Path,
    enabled: bool,
    targets: &[PathBuf],
) -> Vec<ContextFile>;
```

Behavior:
- root `AGENTS.md` and `CLAUDE.md` preserve existing behavior;
- for each target under `cwd`, walk ancestors from target parent toward `cwd` and include nearest `AGENTS.md` / `CLAUDE.md` files;
- canonicalize/deduplicate paths;
- exact duplicate bodies remain deduplicated at prompt rendering while provenance retains all paths;
- never traverse above `cwd`.

- [ ] **Step 1: Write one filesystem fixture test**

Fixture:

```text
repo/AGENTS.md            -> "root"
repo/crates/AGENTS.md     -> "crates"
repo/crates/a/src/lib.rs
repo/crates/b/src/lib.rs
```

For target `crates/a/src/lib.rs`, assert root + `crates/AGENTS.md` are loaded and unrelated deeper paths are not.

- [ ] **Step 2: Run and confirm failure**

```bash
cargo test -p davinci-agent scoped_context_files_follow_target_ancestors
```

- [ ] **Step 3: Implement bounded ancestor discovery**

Use `Path::ancestors()` and stop at canonical `cwd`. Do not recursively scan the repository.

- [ ] **Step 4: Run context tests and commit**

```bash
cargo test -p davinci-agent scoped_context_files_follow_target_ancestors
cargo test -p davinci-agent repository_prompt_retains_duplicate_provenance_without_duplicate_body
git commit -am "feat(context): load scoped repository instructions"
```

---

### Task 6: Make verification evidence mutation-scope-aware

**Files:**
- Modify: `crates/davinci-agent/src/lib.rs`
- Modify: `crates/davinci-agent/src/turn.rs`
- Modify the existing verification-command classifier module/function, not a parallel list

**Interfaces:**

Add:

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum VerificationCoverage {
    Targeted,
    Broad,
    Unrelated,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VerificationEvidence {
    pub generation: u64,
    pub command: String,
    pub succeeded: bool,
    pub mutation_paths: Vec<PathBuf>,
    pub verification_targets: Vec<String>,
    pub coverage: VerificationCoverage,
}
```

Extend `MutationVerificationState` with latest evidence while retaining existing public fields for compatibility.

- [ ] **Step 1: Add the critical failing test**

Scenario:

```text
mutation path = crates/davinci-agent/src/lib.rs
verification command = cargo test -p unrelated-crate
exit code = 0
```

Assert completion is not `Verified`.

- [ ] **Step 2: Run and confirm current behavior is too permissive**

```bash
cargo test -p davinci-agent unrelated_successful_test_does_not_verify_mutation
```

- [ ] **Step 3: Implement deterministic coverage classification**

Rules should be conservative:
- workspace-wide verifier such as `cargo test --workspace`, `cargo clippy --workspace`, repository canonical check => `Broad`;
- package/path-specific verifier covering the changed package/path => `Targeted`;
- explicitly different package/path => `Unrelated`;
- commands recognized as verification but with unparseable target => `Unknown`.

`CompletionEvidence::Verified` requires successful `Broad` or `Targeted` evidence for the current generation.

- [ ] **Step 4: Add/extend mutation path capture**

Use actual successful mutation tool arguments/effect metadata already available in the execution path. Do not infer changed files from assistant prose.

- [ ] **Step 5: Run four focused tests**

```bash
cargo test -p davinci-agent unrelated_successful_test_does_not_verify_mutation
cargo test -p davinci-agent mutation_then_successful_verification_is_verified
cargo test -p davinci-agent later_mutation_invalidates_previous_verification
cargo test -p davinci-agent read_only_turn_does_not_require_verification
```

- [ ] **Step 6: Commit**

```bash
git commit -am "feat(verification): require mutation-scope coverage"
```

---

### Task 7: Make `ToolCallLedger` persistence atomic and fail closed on corruption

**Files:**
- Modify: `crates/davinci-agent/src/tool_ledger.rs`
- Test: inline tool ledger tests

**Interfaces:**
- Add private helper:

```rust
fn atomic_write_json(path: &Path, bytes: &[u8]) -> Result<(), String>;
```

Required write protocol:

```text
same-directory temporary file
→ write_all
→ sync_all(temp)
→ rename(temp, destination)
→ sync parent directory where supported
```

- [ ] **Step 1: Add an atomic replacement test**

Persist a ledger twice and assert the destination parses to the second complete state and no stale temporary file remains.

- [ ] **Step 2: Add corruption fail-closed test**

Write invalid JSON at the ledger path and assert `load_bound` returns a distinct corruption error; the caller must not replace it with an empty ledger and proceed with mutation replay.

- [ ] **Step 3: Run both tests and confirm current direct-write behavior does not satisfy the contract**

```bash
cargo test -p davinci-agent tool_ledger_atomic_persist_replaces_complete_state
cargo test -p davinci-agent corrupt_tool_ledger_fails_closed
```

- [ ] **Step 4: Implement atomic write**

Use only stdlib. On platforms where directory fsync cannot be opened/synced, retain file sync + atomic rename and document the platform limitation in code comments; do not add a dependency just for this step.

- [ ] **Step 5: Run tool-ledger tests**

```bash
cargo test -p davinci-agent tool_ledger
cargo test -p davinci-agent restart_restores_uncertain_mutation_and_blocks_replay
```

- [ ] **Step 6: Commit**

```bash
git commit -am "fix(recovery): persist tool ledger atomically"
```

---

### Task 8: Remove duplicated side-effect classification from the ledger

**Files:**
- Modify: `crates/davinci-agent/src/tool_ledger.rs`
- Modify actual call sites that reserve/start tool calls if necessary
- Use: `crates/davinci-agent/src/runtime/capabilities.rs`

**Interfaces:**
- Replace hard-coded `classify_side_effect(tool_name)` decisions with a value derived from the runtime capability record when available:

```rust
fn side_effect_from_capability(cap: Option<&RuntimeCapability>) -> ToolSideEffect {
    match cap {
        Some(cap) if cap.read_only => ToolSideEffect::ReadOnly,
        _ => ToolSideEffect::Mutating,
    }
}
```

- [ ] **Step 1: Add a regression test using a custom registered read-only MCP capability**

Assert a name not present in the old hard-coded list is still recorded `ReadOnly` when the registry says it is read-only.

- [ ] **Step 2: Confirm failure against hard-coded classification**

```bash
cargo test -p davinci-agent tool_ledger_uses_runtime_capability_side_effect
```

- [ ] **Step 3: Thread registry-derived side effect into reservation/record creation**

Do not make `ToolCallLedger` own another capability registry. Pass the classification/replay policy from the existing execution preparation path.

- [ ] **Step 4: Run focused metadata/recovery tests**

```bash
cargo test -p davinci-agent tool_ledger_uses_runtime_capability_side_effect
cargo test -p davinci-agent representative_tools_have_consistent_execution_metadata
cargo test -p davinci-agent unknown_tool_defaults_to_serial_nonreplayable_mutation
```

- [ ] **Step 5: Run subsystem boundary checks and commit**

```bash
cargo check -p davinci-agent
cargo check -p davinci-coding-agent
cargo clippy -p davinci-agent --all-targets -- -D warnings
cargo clippy -p davinci-coding-agent --all-targets -- -D warnings
git commit -am "refactor(runtime): derive ledger side effects from capabilities"
```
