# Capability Toolbox, Memory, and Learning Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Improve Capability Toolbox relevance and durable knowledge quality without changing its storage architecture or adding model-dependent routing.

**Architecture:** Reuse Graph's existing `ContextPacket`, vector-memory retrieval, skill ledger, exact `SkillVersionRef`, and verified outcome flow. Build a deterministic node-specific retrieval query, calibrate cross-source context utility, reuse local embeddings when already available, add optional skill applicability metadata, and penalize stale memory deterministically.

**Tech Stack:** Rust 1.83, existing vector-memory/learning code, serde, SHA-256 utilities, offline tests.

**Spec:** `docs/superpowers/specs/2026-09-14-davinci-harness-optimization-design.md`

## Global Constraints

- Keep existing Graph aggregate context cap (`2,500` tokens), memory hit cap (`4`), and skill cap (`2`) unless a later benchmark explicitly changes them.
- Never partially inject a skill body.
- Do not create a second learning ledger, embedding store, or vector database.
- Graph context assembly must not trigger a new remote embedding request.
- Exact skill-version/content-hash attribution remains authoritative.

---

### Task 1: Build node-specific deterministic Capability Toolbox queries

**Files:**
- Modify: `crates/davinci-coding-agent/src/native_extensions/ecosystem/context.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/ecosystem/capability.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/controller.rs`

**Interfaces:**

Add:

```rust
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkerContextQuery {
    pub role: Option<crate::native_extensions::graph::Role>,
    pub node_objective: String,
    pub graph_goal: String,
    pub target_hints: Vec<String>,
    pub failure_hint: Option<String>,
}

impl WorkerContextQuery {
    pub fn render(&self) -> String;
}
```

Rendering order:

```text
role:<role>
objective:<bounded objective>
targets:<sorted unique hints>
goal:<bounded graph goal>
failure:<bounded failure hint, retry only>
```

Each field is whitespace-normalized and individually capped; total rendered query should be capped at 2,000 characters.

- [ ] **Step 1: Add a failing determinism/relevance test**

Two workers under the same graph goal but different objectives must produce different rendered queries while repeated inputs produce byte-identical queries.

```rust
#[test]
fn worker_context_query_is_node_specific_and_deterministic() { ... }
```

- [ ] **Step 2: Run and confirm failure**

```bash
cargo test -p davinci-coding-agent worker_context_query_is_node_specific_and_deterministic
```

- [ ] **Step 3: Implement `WorkerContextQuery::render`**

Sort/deduplicate target hints with `BTreeSet`. Do not include timestamps, run IDs, full transcripts, or output bodies.

- [ ] **Step 4: Wire the controller to build the query from existing task data**

Use, in priority order:
- task briefing/current node objective;
- role;
- target paths/symbol hints already present in typed task/artifact fields when available;
- overall Graph goal as secondary context;
- retry failure class/diagnostic summary only after a failed attempt.

The first-attempt base query must not depend on volatile retry text.

- [ ] **Step 5: Run controller/ecosystem tests**

```bash
cargo test -p davinci-coding-agent worker_context_query_is_node_specific_and_deterministic
cargo test -p davinci-coding-agent ecosystem::
cargo test -p davinci-coding-agent graph::controller
```

- [ ] **Step 6: Commit**

```bash
git add crates/davinci-coding-agent/src/native_extensions/ecosystem/context.rs \
        crates/davinci-coding-agent/src/native_extensions/ecosystem/capability.rs \
        crates/davinci-coding-agent/src/native_extensions/graph/controller.rs
git commit -m "feat(capability-toolbox): use node-specific retrieval queries"
```

---

### Task 2: Calibrate context utility across memory and skill sources

**Files:**
- Modify: `crates/davinci-coding-agent/src/native_extensions/ecosystem/context.rs`
- Modify only the minimal vector-memory/learning types needed to expose quality factors

**Interfaces:**

Add a source-neutral selection record:

```rust
#[derive(Debug, Clone, Copy)]
pub struct ContextUtilityInput {
    pub relevance: f32,
    pub confidence: f32,
    pub applicability: f32,
    pub freshness: f32,
    pub estimated_tokens: usize,
}

pub fn context_utility(input: ContextUtilityInput) -> f32 {
    let quality = input.relevance.clamp(0.0, 1.0)
        * input.confidence.clamp(0.0, 1.0)
        * input.applicability.clamp(0.0, 1.0)
        * input.freshness.clamp(0.0, 1.0);
    let token_cost = (input.estimated_tokens.max(1) as f32).sqrt();
    quality / token_cost
}
```

Defaults for legacy candidates lacking metadata:

```text
confidence=1.0 only when existing evidence already treats source as verified/active;
otherwise confidence=0.75
applicability=1.0 when no applicability metadata exists
freshness=1.0 when freshness cannot yet be assessed (do not invent staleness)
```

- [ ] **Step 1: Write a pure unit test for utility ordering**

Assert a slightly less relevant but much smaller/verified candidate can outrank an enormous low-confidence candidate; equal inputs are deterministic.

- [ ] **Step 2: Run and confirm failure**

```bash
cargo test -p davinci-coding-agent context_utility_prefers_verified_relevant_value_per_token
```

- [ ] **Step 3: Implement utility and use it only for final aggregate packet trimming**

Do not replace each source's internal retrieval rank. First retrieve bounded memory/skill candidate sets using existing systems, then choose the final packet under the aggregate cap using the source-neutral utility.

Preserve the rule: a skill either fits completely or is omitted.

- [ ] **Step 4: Add aggregate-cap regression**

Create one high-value full skill plus several lower-value memories that together exceed the cap; assert the high-utility complete skill survives and total packet stays under cap.

- [ ] **Step 5: Run focused tests and commit**

```bash
cargo test -p davinci-coding-agent context_utility_prefers_verified_relevant_value_per_token
cargo test -p davinci-coding-agent context_packet_uses_cross_source_utility_under_cap
git commit -am "feat(capability-toolbox): calibrate context utility"
```

---

### Task 3: Reuse cached semantic skill embeddings when already available

**Files:**
- Modify: `crates/davinci-coding-agent/src/native_extensions/learning/retrieval.rs`
- Modify the existing learning controller/storage module that owns skill embeddings, if present
- Do not create a new embedding persistence file

**Interfaces:**

Add a retrieval entry point that accepts optional already-resolved embeddings:

```rust
pub fn select_graph_skill_candidates_with_embeddings(
    query: &str,
    query_embedding: Option<&[f32]>,
    skills: &[Skill],
    skill_embeddings: Option<&[Option<Vec<f32>>]>,
    ledger: &[SkillLedgerRecord],
    role: Role,
    max_skills: usize,
    token_cap: usize,
) -> Vec<SkillContextCandidate>;
```

Existing `select_graph_skill_candidates` remains a lexical compatibility wrapper calling the new function with `None` embeddings.

- [ ] **Step 1: Add a failing ranking test**

Create two skills with no lexical overlap; provide a query embedding close to only one skill embedding and assert that skill is selected.

- [ ] **Step 2: Confirm failure**

```bash
cargo test -p davinci-coding-agent graph_skill_selection_uses_cached_embedding_when_supplied
```

- [ ] **Step 3: Implement wrapper around existing `rank_skills_with_embeddings`**

No embedding generation occurs in this function. If query or skill embeddings are absent/incompatible, use lexical ranking immediately.

- [ ] **Step 4: Wire only existing locally cached embeddings**

If the learning/vector-memory layer already has cached embeddings for skill content/query, pass them. If not, leave `None`; do not add an Ollama/network call to Graph assembly.

- [ ] **Step 5: Run lexical-fallback and semantic tests**

```bash
cargo test -p davinci-coding-agent graph_skill_selection_uses_cached_embedding_when_supplied
cargo test -p davinci-coding-agent graph_skill_selection_falls_back_to_lexical_without_embeddings
```

- [ ] **Step 6: Commit**

```bash
git commit -am "feat(learning): reuse cached semantic skill ranking"
```

---

### Task 4: Add optional skill applicability metadata

**Files:**
- Modify: learning skill metadata/types file under `crates/davinci-coding-agent/src/native_extensions/learning/`
- Modify: `crates/davinci-coding-agent/src/native_extensions/learning/retrieval.rs`
- Modify skill serialization/parsing code that persists current skill versions

**Interfaces:**

Add backwards-compatible optional structure:

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SkillApplicability {
    #[serde(default)] pub languages: Vec<String>,
    #[serde(default)] pub task_types: Vec<String>,
    #[serde(default)] pub path_globs: Vec<String>,
    #[serde(default)] pub required_signals: Vec<String>,
    #[serde(default)] pub verification_categories: Vec<String>,
}
```

Add to the persisted skill ledger/version metadata with `#[serde(default)]` so old data loads unchanged.

- [ ] **Step 1: Add backwards-compatibility test**

Deserialize an old skill ledger fixture without `applicability` and assert default empty metadata.

- [ ] **Step 2: Add applicability ranking test**

For equal lexical scores, a Rust/debugging skill whose applicability matches `language=rust`, `task_type=debugging`, and target `crates/davinci-agent/**` should outrank an unrelated frontend skill.

- [ ] **Step 3: Run both tests and confirm the ranking behavior is missing**

```bash
cargo test -p davinci-coding-agent legacy_skill_metadata_defaults_applicability
cargo test -p davinci-coding-agent skill_applicability_boosts_matching_scope
```

- [ ] **Step 4: Implement deterministic applicability score**

Suggested bounded score:

```text
no metadata                => 1.00 neutral
explicit matching language => +0.08
matching task type         => +0.08
matching path glob         => +0.08
missing required signal    => -0.20
explicit incompatible lang => -0.15
clamp final multiplier to [0.5, 1.2]
```

Use existing glob/path helpers if present; do not add a new glob dependency solely for this. A simple normalized prefix/suffix matcher is acceptable for `**`-style applicability hints if already sufficient for the saved format.

- [ ] **Step 5: Run learning tests and commit**

```bash
cargo test -p davinci-coding-agent learning::
git commit -am "feat(learning): add skill applicability metadata"
```

---

### Task 5: Improve exact-version outcome credit

**Files:**
- Modify existing learning outcome record/update code under `crates/davinci-coding-agent/src/native_extensions/learning/`
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/controller.rs`

**Interfaces:**

Extend exact skill outcome evidence:

```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SkillUsageSignal {
    Injected,
    ScopeRelevant,
    VerifiedHelpful,
    VerifiedFailureRelevant,
    Neutral,
}
```

Do not require the model to say whether it used a skill. Derive only deterministic evidence available from target scope, task kind, exact injected skill version, and verified result.

- [ ] **Step 1: Add failing credit test**

Inject two skills into a successful task; only one has path/task applicability matching the mutated/verified scope. Assert the irrelevant injected skill does not receive the same positive success increment as the relevant one.

- [ ] **Step 2: Confirm failure**

```bash
cargo test -p davinci-coding-agent injected_irrelevant_skill_is_not_credited_as_helpful
```

- [ ] **Step 3: Implement conservative credit**

Rules:
- exact version/hash remains mandatory;
- injected + no deterministic relevance => neutral usage count only;
- relevant + verified success => positive success credit;
- relevant + verified failure => failure credit;
- never credit a version not in the task's `skill_refs`.

- [ ] **Step 4: Run exact-version and new credit tests**

```bash
cargo test -p davinci-coding-agent ecosystem_loop_learning_to_graph
cargo test -p davinci-coding-agent injected_irrelevant_skill_is_not_credited_as_helpful
```

- [ ] **Step 5: Commit**

```bash
git commit -am "feat(learning): improve verified skill outcome credit"
```

---

### Task 6: Add memory freshness and compatibility metadata

**Files:**
- Modify: `crates/davinci-coding-agent/src/native_extensions/vector_memory.rs`
- Modify memory ingestion call sites only where deterministic source references already exist

**Interfaces:**

Extend `MemoryRecord` backwards-compatibly:

```rust
#[serde(default)]
pub source_paths: Vec<String>,
#[serde(default, skip_serializing_if = "Option::is_none")]
pub source_state_hash: Option<String>,
#[serde(default, skip_serializing_if = "Option::is_none")]
pub verified_at_revision: Option<String>,
```

Add:

```rust
pub fn memory_freshness(record: &MemoryRecord, cwd: &Path) -> f32;
```

Rules:
- no freshness metadata => `1.0` neutral;
- all referenced paths exist and optional state hash matches => `1.0`;
- paths exist but state hash changed => bounded penalty, e.g. `0.65`;
- referenced path removed/renamed => stronger penalty, e.g. `0.35`;
- user-decision/explicit constraint provenance may have a floor so source-file churn does not erase human authority.

- [ ] **Step 1: Add legacy-record compatibility test**

Old JSONL records without new fields deserialize and retain neutral freshness.

- [ ] **Step 2: Add stale-source test**

Create a record referencing `src/auth.rs` with a saved state hash, mutate the file, and assert freshness decreases deterministically.

- [ ] **Step 3: Run tests**

```bash
cargo test -p davinci-coding-agent legacy_memory_record_has_neutral_freshness
cargo test -p davinci-coding-agent memory_freshness_penalizes_changed_source_state
```

- [ ] **Step 4: Integrate freshness into final retrieval score**

Keep existing dense/lexical/importance base; apply freshness/compatibility as a multiplier or bounded term after base relevance, not a replacement for relevance.

Example:

```rust
let base = (dense * 0.6 + lexical * 0.3 + record.importance * 0.1).clamp(0.0, 1.0);
let freshness = memory_freshness(record, &self.cwd);
let score = (base * freshness).clamp(0.0, 1.0);
```

If `VectorMemory` does not currently retain `cwd`, pass the repository path already owned by the struct rather than introducing global state.

- [ ] **Step 5: Run vector-memory tests and commit**

```bash
cargo test -p davinci-coding-agent vector_memory
cargo clippy -p davinci-coding-agent --all-targets -- -D warnings
git commit -am "feat(memory): penalize stale source-bound memories"
```

---

### Task 7: Add deterministic Capability Toolbox ablation

**Files:**
- Modify: existing ecosystem/eval test module, preferably `crates/davinci-coding-agent/src/native_extensions/ecosystem/mod.rs` or `davinci-evals` if already used for feature ablations

**Interfaces:**

Create an offline comparison fixture:

```text
baseline query = global graph goal only
candidate query = node-specific query
oracle = expected relevant memory/skill IDs for each node
```

- [ ] **Step 1: Implement 6–10 fixture cases covering researcher, writer, reviewer, and test-analyzer roles**

- [ ] **Step 2: Assert candidate does not increase context-token cap and improves or ties relevant-candidate precision**

- [ ] **Step 3: Run**

```bash
cargo test -p davinci-coding-agent capability_toolbox_node_query_ablation
```

- [ ] **Step 4: Commit subsystem boundary**

```bash
cargo clippy -p davinci-coding-agent --all-targets -- -D warnings
git commit -am "test(capability-toolbox): add node retrieval ablation"
```
