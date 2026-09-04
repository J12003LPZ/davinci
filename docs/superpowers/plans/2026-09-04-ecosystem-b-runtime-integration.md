# Ecosystem Gate B — Runtime Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the Graph ↔ Governor ↔ Memory ↔ Skills ↔ provider-cache loop while preserving worker isolation and hard token bounds.

**Architecture:** Add small deterministic ecosystem contracts under `native_extensions/ecosystem/`; reuse the existing native subsystems rather than introducing a central controller. Graph workers remain ephemeral and isolated, but the parent explicitly supplies bounded context, required governor recovery capability, stable cache affinity, and run-level accounting.

**Tech Stack:** Rust 1.83.0, existing native extensions, `davinci-ai::StreamOptions`, existing vector-memory and learning retrieval code.

**Spec:** `docs/superpowers/specs/2026-09-04-davinci-ecosystem-integration-design.md`

## Global Constraints

- Zero additional model calls.
- Preserve graph `--no-session --no-extensions --no-skills` isolation.
- Default ecosystem context cap: 2,500 tokens per graph worker.
- Default memory cap: 1,200 tokens / 4 hits.
- Default skill cap: 1,000 tokens / 2 full skills.
- No quota filling: weak retrieval returns empty context.
- `retrieve_output` recovery is automatic whenever a role can generate compressible output.
- Cache affinity must not create session files.
- All new behavior must have a configuration kill switch or natural absence fallback.

---

### Task 1: Create Focused Ecosystem Contract Module

**Files:**
- Create: `crates/davinci-coding-agent/src/native_extensions/ecosystem/mod.rs`
- Create: `crates/davinci-coding-agent/src/native_extensions/ecosystem/context.rs`
- Create: `crates/davinci-coding-agent/src/native_extensions/ecosystem/cache_affinity.rs`
- Create: `crates/davinci-coding-agent/src/native_extensions/ecosystem/resource.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/mod.rs`

**Interfaces:**

```rust
pub struct ContextPacketRequest<'a> {
    pub prompt: &'a str,
    pub role: Option<crate::native_extensions::graph::Role>,
    pub token_cap: usize,
    pub include_skills: bool,
}

pub struct ContextPacket {
    pub text: String,
    pub memory_refs: Vec<String>,
    pub skill_refs: Vec<SkillContextRef>,
    pub estimated_tokens: usize,
    pub fingerprint: String,
}

pub struct SkillContextRef {
    pub name: String,
    pub version: u64,
    pub content_hash: String,
}
```

- [ ] **Step 1: Add compile-time/unit tests for default caps**

```rust
#[test]
fn default_graph_context_caps_match_design() {
    assert_eq!(DEFAULT_GRAPH_CONTEXT_TOKENS, 2_500);
    assert_eq!(DEFAULT_GRAPH_MEMORY_TOKENS, 1_200);
    assert_eq!(DEFAULT_GRAPH_MEMORY_HITS, 4);
    assert_eq!(DEFAULT_GRAPH_SKILL_TOKENS, 1_000);
    assert_eq!(DEFAULT_GRAPH_SKILL_COUNT, 2);
}
```

- [ ] **Step 2: Run and verify missing module failure**

```bash
cargo test -p davinci-coding-agent default_graph_context_caps_match_design -- --nocapture
```

- [ ] **Step 3: Add pure data types/constants only**

Do not move subsystem ownership into this module. It contains contracts and pure helpers.

- [ ] **Step 4: Add deterministic fingerprint helper**

Hash canonical text + selected memory/skill identifiers; exclude timestamps/run IDs.

- [ ] **Step 5: Run focused tests and commit**

```bash
cargo test -p davinci-coding-agent native_extensions::ecosystem -- --nocapture
git add crates/davinci-coding-agent/src/native_extensions/ecosystem crates/davinci-coding-agent/src/native_extensions/mod.rs
git commit -m "feat(ecosystem): add bounded integration contracts"
```

---

### Task 2: Guarantee Governor Recovery Capability for Graph Roles

**Files:**
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/roles.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/token_governor.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/briefings.rs`

**Interfaces:**

```rust
pub fn tool_may_be_compressed(name: &str) -> bool;
pub fn ensure_governor_recovery_tool(tools: &mut Vec<String>);
```

- [ ] **Step 1: Write failing role tests**

```rust
#[test]
fn researcher_with_compressible_tools_always_gets_retrieve_output() {
    let tools = role_tools(Role::Researcher, &GraphConfig::default());
    assert!(tools.contains(&"grep".into()));
    assert!(tools.contains(&"retrieve_output".into()));
}
```

Add equivalent coverage for test-analyzer, historian/reviewer/writer roles with compressible shell/search tools. Classifier/planner should receive `retrieve_output` only when their allowed tool set can actually generate compressed output.

- [ ] **Step 2: Run and verify failure**

```bash
cargo test -p davinci-coding-agent governor_recovery_tool -- --nocapture
```

- [ ] **Step 3: Export Governor capability knowledge through a tiny predicate**

Avoid duplicating `LOSSLESS_TOOLS` policy in graph code. `token_governor.rs` owns the answer to whether a tool result may be compressed.

- [ ] **Step 4: Apply capability closure after normal role + configured extra tool assembly**

Add `retrieve_output` exactly once when needed.

- [ ] **Step 5: Add one compact worker-prompt sentence only when recovery is present**

Example intent: `Large tool output may be compacted; use retrieve_output only when the omitted detail is needed.`

- [ ] **Step 6: Add an end-to-end fixture test**

Worker fixture emits oversized grep/bash output; assert returned governor digest names a retrievable ID and the role toolset includes `retrieve_output`.

- [ ] **Step 7: Run tests and commit**

```bash
cargo test -p davinci-coding-agent governor_recovery -- --nocapture
git add crates/davinci-coding-agent/src/native_extensions/graph/roles.rs crates/davinci-coding-agent/src/native_extensions/token_governor.rs crates/davinci-coding-agent/src/native_extensions/graph/briefings.rs
git commit -m "fix(graph): guarantee governor output recovery"
```

---

### Task 3: Separate Provider Cache Affinity from Session Persistence

**Files:**
- Modify: `crates/davinci-ai/src/stream.rs`
- Modify: `crates/davinci-ai/src/request.rs` if it constructs equivalent request bodies
- Modify: `crates/davinci-ai/src/cache.rs`
- Modify: call sites constructing `davinci_ai::StreamOptions` across workspace as required by compiler

**Interfaces:**

```rust
pub struct StreamOptions {
    // existing fields...
    pub session_id: Option<String>,
    pub cache_key: Option<String>,
}

pub fn effective_prompt_cache_key(options: &StreamOptions) -> Option<&str> {
    options.cache_key.as_deref().or(options.session_id.as_deref())
}
```

- [ ] **Step 1: Add request-body tests**

```rust
#[test]
fn explicit_cache_key_wins_over_session_id() {
    let options = StreamOptions {
        session_id: Some("session-a".into()),
        cache_key: Some("graph-role-a".into()),
        ..StreamOptions::default()
    };
    assert_eq!(effective_prompt_cache_key(&options), Some("graph-role-a"));
}

#[test]
fn session_id_remains_fallback_cache_key() {
    let options = StreamOptions {
        session_id: Some("session-a".into()),
        cache_key: None,
        ..StreamOptions::default()
    };
    assert_eq!(effective_prompt_cache_key(&options), Some("session-a"));
}
```

Also pin OpenAI/Codex/Azure request bodies to the effective key.

- [ ] **Step 2: Run focused AI tests and verify missing field failure**

```bash
cargo test -p davinci-ai cache_key -- --nocapture
```

- [ ] **Step 3: Implement fallback without altering normal-session behavior**

All existing normal call sites set `cache_key: None` unless they explicitly need affinity separate from session persistence.

- [ ] **Step 4: Ensure retention semantics remain unchanged**

This task changes identity, not whether provider caching is enabled/retained.

- [ ] **Step 5: Run `davinci-ai` tests and commit**

```bash
cargo test -p davinci-ai
git add crates/davinci-ai
git commit -m "feat(ai): decouple prompt cache affinity from sessions"
```

---

### Task 4: Derive Stable Graph Worker Cache Keys

**Files:**
- Modify: `crates/davinci-coding-agent/src/native_extensions/ecosystem/cache_affinity.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/worker.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/process.rs` if internal env/arg handoff is required
- Modify: `crates/davinci-coding-agent/src/main.rs` only at the internal worker request path if needed

**Interfaces:**

```rust
pub struct GraphCacheIdentity<'a> {
    pub repo_id: &'a str,
    pub graph_version: u32,
    pub role: graph::Role,
    pub model: &'a str,
    pub toolset_hash: &'a str,
    pub system_contract_hash: &'a str,
}

pub fn graph_worker_cache_key(input: &GraphCacheIdentity<'_>) -> String;
```

- [ ] **Step 1: Add stability/sensitivity tests**

Same repo/graph-version/role/model/toolset/contract => same key across different run IDs. Changing model, role, toolset, graph version, or system contract => different key.

- [ ] **Step 2: Run tests and verify missing implementation**

- [ ] **Step 3: Generate a bounded provider-safe key**

Use the existing OpenAI cache-key clamp helper after prefixing a short human-readable role marker plus hash.

- [ ] **Step 4: Hand cache identity to ephemeral child without creating a session**

Prefer an internal environment variable or hidden internal flag that only sets `StreamOptions.cache_key`; it must not imply session persistence.

- [ ] **Step 5: Add print-worker integration fixture**

Assert child request options have `session_id == None`, `cache_key == Some(...)`.

- [ ] **Step 6: Commit**

```bash
git add crates/davinci-coding-agent/src/native_extensions/ecosystem/cache_affinity.rs crates/davinci-coding-agent/src/native_extensions/graph/worker.rs crates/davinci-coding-agent/src/native_extensions/graph/process.rs crates/davinci-coding-agent/src/main.rs
git commit -m "feat(graph): add stable ephemeral worker cache affinity"
```

---

### Task 5: Add Bounded Memory Retrieval for Graph Context Packets

**Files:**
- Modify: `crates/davinci-coding-agent/src/native_extensions/vector_memory.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/ecosystem/context.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/mod.rs`

**Interfaces:**

```rust
pub struct MemoryContextHit {
    pub id: String,
    pub text: String,
    pub score: f32,
    pub estimated_tokens: usize,
}

pub fn context_hits(&self, query: &str, max_hits: usize, token_cap: usize) -> Vec<MemoryContextHit>;
```

- [ ] **Step 1: Add tests for hard caps and weak-match emptiness**

Fixture with 10 relevant records must return no more than 4 and fit 1,200 tokens. Fixture below minimum relevance must return empty.

- [ ] **Step 2: Ensure this API reuses existing hybrid retrieval ranking**

Do not introduce a second search implementation.

- [ ] **Step 3: Build a compact memory section**

Use concise provenance markers, not full memory-store metadata. Preserve secret redaction rules.

- [ ] **Step 4: Keep normal `native_memory_inject` unchanged**

This API exists for explicit parent-built graph packets only.

- [ ] **Step 5: Test and commit**

```bash
cargo test -p davinci-coding-agent graph_memory_context -- --nocapture
git add crates/davinci-coding-agent/src/native_extensions/vector_memory.rs crates/davinci-coding-agent/src/native_extensions/ecosystem/context.rs crates/davinci-coding-agent/src/native_extensions/mod.rs
git commit -m "feat(memory): provide bounded graph context hits"
```

---

### Task 6: Add Role-Scoped Learned Skill Retrieval Without Enabling Child Skill Discovery

**Files:**
- Modify: `crates/davinci-coding-agent/src/native_extensions/learning/retrieval.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/learning/types.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/ecosystem/context.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/mod.rs`

**Interfaces:**

```rust
pub struct SkillContextCandidate {
    pub name: String,
    pub version: u64,
    pub content_hash: String,
    pub body: String,
    pub score: f32,
    pub estimated_tokens: usize,
}

pub fn graph_skill_candidates(
    &self,
    query: &str,
    role: graph::Role,
    max_skills: usize,
    token_cap: usize,
) -> Vec<SkillContextCandidate>;
```

- [ ] **Step 1: Add fixture skills with different relevance and scopes**

- [ ] **Step 2: Write tests proving max 2 / 1,000 token behavior**

Also assert irrelevant skills are omitted rather than selected to reach 2.

- [ ] **Step 3: Prefer compact descriptor ranking before full body load**

Use existing progressive-disclosure retrieval. Read full `SKILL.md` only for finalists.

- [ ] **Step 4: Add simple role-compatibility hints without a brittle rule engine**

Role compatibility should be metadata/keywords and ranking bias, not a mandatory workflow mapping. A relevant general skill may still be supplied to multiple roles.

- [ ] **Step 5: Never add skill directories to child discovery paths**

The worker process must still receive `--no-skills`.

- [ ] **Step 6: Test and commit**

```bash
cargo test -p davinci-coding-agent graph_skill_context -- --nocapture
git add crates/davinci-coding-agent/src/native_extensions/learning crates/davinci-coding-agent/src/native_extensions/ecosystem/context.rs crates/davinci-coding-agent/src/native_extensions/mod.rs
git commit -m "feat(learning): provide bounded role-scoped graph skills"
```

---

### Task 7: Assemble and Persist the Graph Context Packet

**Files:**
- Modify: `crates/davinci-coding-agent/src/native_extensions/ecosystem/context.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/worker.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/types.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/store.rs`

**Interfaces:**

```rust
pub fn build_context_packet(
    memory: &VectorMemory,
    learning: &LearningController,
    request: ContextPacketRequest<'_>,
) -> ContextPacket;
```

Graph task/run metadata adds:

```rust
pub context_fingerprint: Option<String>,
pub context_tokens: usize,
pub memory_refs: Vec<String>,
pub skill_refs: Vec<SkillContextRef>,
```

- [ ] **Step 1: Add aggregate-cap tests**

Memory + skills + metadata must never exceed request `token_cap`; trimming order is lowest-ranked context first.

- [ ] **Step 2: Define packet text format**

Keep it compact:

```text
<context source="davinci" untrusted="true">
<memory>...</memory>
<skill name="..." version="...">...</skill>
</context>
```

No verbose explanation. Treat retrieved content as untrusted context, not higher-priority instructions.

- [ ] **Step 3: Insert packet into worker input/system context through one explicit channel**

Do not duplicate the normal foreground automatic-memory injection inside the child for graph workers. Add an internal graph-worker flag/environment signal to suppress automatic child memory injection when parent context packet is present, preventing double injection.

- [ ] **Step 4: Persist provenance before worker execution**

This enables later learning outcome attribution even if the worker fails.

- [ ] **Step 5: Add fixture test for empty packet**

No matching memory/skill => no context block and zero token overhead.

- [ ] **Step 6: Add fixture test for strict child isolation**

Assert worker args still include `--no-session --no-extensions --no-skills`.

- [ ] **Step 7: Commit**

```bash
git add crates/davinci-coding-agent/src/native_extensions/ecosystem/context.rs crates/davinci-coding-agent/src/native_extensions/graph
git commit -m "feat(graph): inject bounded ecosystem context packets"
```

---

### Task 8: Add Read-Only Resource Envelope and Snapshot

**Files:**
- Modify: `crates/davinci-coding-agent/src/native_extensions/ecosystem/resource.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/token_governor.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/controller.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/types.rs`

**Interfaces:**

```rust
pub struct ResourceEnvelope {
    pub max_cost_usd: Option<f64>,
    pub run_deadline_ms: Option<u64>,
    pub max_parallel_workers: usize,
    pub context_soft_limit_tokens: Option<u64>,
}

pub struct ResourceSnapshot {
    pub cost_usd: f64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub cache_write_tokens: u64,
    pub governor_bytes_omitted: u64,
    pub governor_retrievals: u64,
    pub prunings: u64,
}
```

- [ ] **Step 1: Add snapshot aggregation tests**

Use fixture worker usage + governor stats + run stats.

- [ ] **Step 2: Expose Governor counters through read-only stats**

Do not allow Graph to mutate Governor thresholds dynamically.

- [ ] **Step 3: Populate snapshot during graph status/checkpoints**

- [ ] **Step 4: Keep scheduler adaptation strictly bounded**

Only existing hard limits and explicit optional-node suppression at exhausted hard budgets are permitted. Do not alter model choice or topology based on cache/token heuristics.

- [ ] **Step 5: Test no extra worker/model invocation occurs when snapshot is collected**

- [ ] **Step 6: Commit**

```bash
git add crates/davinci-coding-agent/src/native_extensions/ecosystem/resource.rs crates/davinci-coding-agent/src/native_extensions/token_governor.rs crates/davinci-coding-agent/src/native_extensions/graph
git commit -m "feat(ecosystem): expose deterministic resource snapshots"
```

---

### Task 9: Gate B End-to-End Fixture Tests and Benchmark

**Files:**
- Create or extend inline test module near graph/ecosystem implementation; follow repository convention of inline `#[cfg(test)] mod tests`, not a new tests directory unless Gate A introduced one intentionally.
- Modify: `CLAUDE.md`

**Interfaces:** release proof.

- [ ] **Step 1: Add Governor recovery loop fixture**

Prove graph role -> oversized output -> digest -> retrievable original.

- [ ] **Step 2: Add cache-affinity loop fixture**

Compatible retry => same key. Changed model/toolset/contract => different key. `session_id` remains `None`.

- [ ] **Step 3: Add memory/skill packet cap fixture**

Assert packet `estimated_tokens <= 2_500`, memory hits <= 4, skills <= 2.

- [ ] **Step 4: Assert zero extra model calls**

Use the existing fixture completer invocation counter. Compare graph execution node count before/after integration for the same topology; context construction must not invoke completer.

- [ ] **Step 5: Run focused suite**

```bash
cargo test -p davinci-coding-agent ecosystem -- --nocapture
```

- [ ] **Step 6: Run workspace quality gates**

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

- [ ] **Step 7: Document the actual runtime integration path**

Update `CLAUDE.md` with context packet, cache affinity, governor closure, and hard caps.

- [ ] **Step 8: Commit**

```bash
git add crates/davinci-coding-agent CLAUDE.md
git commit -m "test(ecosystem): prove bounded graph runtime integration"
```

## Gate B Exit Checklist

- [ ] Graph workers remain isolated and ephemeral.
- [ ] Graph workers never lose access to Governor-compressed data.
- [ ] Compatible workers have stable cache affinity without saved sessions.
- [ ] Memory/skills return to Graph under hard caps.
- [ ] Empty/weak retrieval produces no prompt overhead.
- [ ] Context provenance is persisted.
- [ ] Resource accounting is read-only/deterministic.
- [ ] No additional model call exists solely for integration.
- [ ] Workspace fmt/clippy/tests are green.
