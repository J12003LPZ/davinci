# Davinci Self-Improving Learning System Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Add a safe, verifier-backed self-improvement loop that turns settled Davinci turns into durable memory and reusable procedural skills while reusing the existing vector-memory, `SKILL.md`, permission/trust, graph-verification, event, and native-extension systems.

**Architecture:** Add a `LearningController` beside `VectorMemory`, `TokenGovernor`, `GraphController`, and `SecurityScanController` inside the existing native-extension host. It consumes the same settled-turn snapshot Davinci already indexes into vector memory, creates provenance-rich learning candidates, applies deterministic promotion policy, and writes approved procedural knowledge into the same project/global skill roots already consumed by `discover_skills`.

**Tech Stack:** Rust 1.83.0, existing workspace crates, `serde`/`serde_json`, current filesystem/session infrastructure, existing Davinci provider/runtime abstractions, existing vector-memory embedding/lexical fallback, existing graph verification, inline `#[cfg(test)]` tests.

**Spec:** `docs/superpowers/specs/2026-09-03-davinci-self-improving-learning-design.md`

## Global Constraints

- Preserve the repository's TypeScript-`pi` compatibility goal; new learning behavior is a Davinci-only extension and must not change vendor-compatible defaults outside its explicit integration points.
- Never edit `vendor/pi`.
- Keep Rust pinned to `1.83.0`.
- Keep every new dependency exactly pinned; prefer adding no dependency.
- Tests must be fixture-only and must not contact a live provider, Ollama, Qdrant, a browser, or the network.
- Keep tests inline with the module under test, matching repository convention.
- Existing `/skill:name` behavior must remain backward compatible.
- Existing vector-memory records must remain readable.
- Learning failures are fail-open: they may reduce learning, but must not fail the user's normal turn.
- Background review must not receive `bash`, `powershell`, `write`, `edit`, `apply_patch`, or arbitrary write-capable MCP tools.
- Automatic project writes require a trusted project.
- Automatic global writes are disabled by default.
- Autonomous review may directly modify only Davinci-owned learned artifacts.
- A deterministic verification pass requires at least one real command to have run; reuse the graph verifier's existing invariant rather than reimplementing a weaker definition.
- Do not add a new workspace crate for this feature.
- Do not build a new visual TUI screen in the first rollout; expose status/commands through existing surfaces first.
- Use redaction, bounded payloads, and existing evidence/token-governor patterns; never persist entire unbounded tool outputs into learning records.

---

## Existing Davinci capabilities this plan deliberately reuses

Before implementing, the executor should read these exact files:

- `crates/davinci-agent/src/skills.rs` — current `Skill`, `discover_skills`, `/skill:name`, and prompt expansion.
- `crates/davinci-coding-agent/src/native_extensions/vector_memory.rs` — memory records, extraction, promotion, embeddings, lexical fallback, secret redaction, repo id.
- `crates/davinci-coding-agent/src/native_extensions/mod.rs` — `NativeExtensionHost`, native tools/commands, lifecycle methods.
- `crates/davinci-coding-agent/src/extension_host.rs` — extension events and native host ownership.
- `crates/davinci-coding-agent/src/main.rs` — settled-turn sequence and `agent_memory_messages`.
- `crates/davinci-agent/src/events.rs` — structured tool/turn events.
- `crates/davinci-agent/src/stats.rs` — runtime efficiency evidence.
- `crates/davinci-coding-agent/src/native_extensions/graph/verify.rs` — deterministic verification semantics.
- `crates/davinci-coding-agent/src/settings.rs` — persisted product settings.
- `crates/davinci-coding-agent/src/permissions.rs` and `crates/davinci-agent/src/permission.rs` — trust/permission integration.
- `CLAUDE.md` — repository invariants and test conventions.

The implementation should **not** create replacements for any of these systems.

---

## Target file map

### New files

```text
crates/davinci-coding-agent/src/native_extensions/learning/
├── mod.rs
├── config.rs
├── types.rs
├── store.rs
├── evidence.rs
├── policy.rs
├── reviewer.rs
├── skill_manager.rs
├── retrieval.rs
└── prompts.rs
```

### Existing files to modify

```text
crates/davinci-agent/src/skills.rs
crates/davinci-coding-agent/src/native_extensions/mod.rs
crates/davinci-coding-agent/src/native_extensions/vector_memory.rs
crates/davinci-coding-agent/src/extension_host.rs
crates/davinci-coding-agent/src/main.rs
crates/davinci-coding-agent/src/davinci_interactive.rs
crates/davinci-coding-agent/src/settings.rs
crates/davinci-coding-agent/src/rpc.rs
crates/davinci-coding-agent/README.md
README.md
docs/README.md
CLAUDE.md
```

### Persistence owned by the new subsystem

```text
<agent_dir>/learning/candidates.jsonl
<agent_dir>/learning/skills.jsonl
<agent_dir>/learning/state.json

<repo>/.pi/learning/candidates.jsonl
<repo>/.pi/learning/skills.jsonl
<repo>/.pi/learning/state.json
```

The skill content itself remains in the existing roots:

```text
<agent_dir>/skills/<name>/SKILL.md
<repo>/.pi/skills/<name>/SKILL.md
```

---

### Task 1: Add learning configuration, domain types, and durable ledger storage

**Files:**
- Create: `crates/davinci-coding-agent/src/native_extensions/learning/config.rs`
- Create: `crates/davinci-coding-agent/src/native_extensions/learning/types.rs`
- Create: `crates/davinci-coding-agent/src/native_extensions/learning/store.rs`
- Create: `crates/davinci-coding-agent/src/native_extensions/learning/mod.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/mod.rs`
- Modify: `crates/davinci-coding-agent/src/settings.rs`

**Interfaces:**
- Produces:
  - `LearningConfig`
  - `LearningScope`
  - `ArtifactStatus`
  - `SkillOrigin`
  - `VerificationEvidence`
  - `LearningArtifact`
  - `LearningCandidate`
  - `SkillLedgerRecord`
  - `LearningStats`
  - `LearningStore`
  - `LearningController`
- Consumes: existing `Path`, `serde`, `serde_json`, `VectorMemory::repo_id`, and product `Settings`.

- [x] **Step 1: Write failing configuration and serialization tests**

Add inline tests that prove defaults are conservative and old settings still deserialize.

```rust
#[test]
fn learning_defaults_are_safe() {
    let config = LearningConfig::default();
    assert!(config.enabled);
    assert!(config.background_review);
    assert!(config.shadow_mode);
    assert!(!config.auto_apply_global);
    assert_eq!(config.max_candidates_per_review, 3);
    assert_eq!(config.auto_promote_verified_uses, 2);
}

#[test]
fn settings_without_learning_still_deserialize() {
    let settings: Settings = serde_json::from_str(r#"{"theme":"dark"}"#).unwrap();
    assert!(settings.learning.is_none());
}
```

Run:

```bash
cargo test -p davinci-coding-agent learning_defaults_are_safe
cargo test -p davinci-coding-agent settings_without_learning_still_deserialize
```

Expected: FAIL because the types and `Settings.learning` do not exist.

- [x] **Step 2: Define the core enums and structs in `learning/types.rs`**

Use these exact public names so later tasks have stable interfaces:

```rust
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LearningScope {
    Project,
    Global,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactStatus {
    Candidate,
    PendingApproval,
    Active,
    Archived,
    Rejected,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SkillOrigin {
    User,
    Imported,
    LearnedForeground,
    LearnedReview,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct VerificationEvidence {
    pub graph_run_id: Option<String>,
    pub commands_ran: u32,
    pub passed: bool,
    pub user_accepted: bool,
    pub user_corrected: bool,
    pub permission_denied: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum LearningArtifact {
    Memory {
        memory_kind: String,
        text: String,
        importance: f32,
    },
    SkillCreate {
        name: String,
        description: String,
        body: String,
    },
    SkillPatch {
        name: String,
        old_text: String,
        new_text: String,
        expected_hash: String,
    },
    SkillSupportFile {
        name: String,
        relative_path: String,
        content: String,
        expected_hash: Option<String>,
    },
    FailureLesson {
        text: String,
        importance: f32,
    },
}
```

Add `LearningCandidate`, `SkillLedgerRecord`, and `LearningStats` exactly as described in the design spec, with `serde(rename_all = "camelCase")` on persisted records.

- [x] **Step 3: Add `LearningConfig` and product settings integration**

In `learning/config.rs`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct LearningConfig {
    pub enabled: bool,
    pub background_review: bool,
    pub shadow_mode: bool,
    pub auto_apply_project: bool,
    pub auto_apply_global: bool,
    pub max_candidates_per_review: usize,
    pub max_review_input_tokens: usize,
    pub max_review_iterations: usize,
    pub auto_promote_verified_uses: u64,
    pub review_timeout_ms: u64,
}

impl Default for LearningConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            background_review: true,
            shadow_mode: true,
            auto_apply_project: false,
            auto_apply_global: false,
            max_candidates_per_review: 3,
            max_review_input_tokens: 12_000,
            max_review_iterations: 6,
            auto_promote_verified_uses: 2,
            review_timeout_ms: 30_000,
        }
    }
}
```

In `settings.rs` add:

```rust
#[serde(default)]
pub learning: Option<crate::native_extensions::learning::LearningConfig>,
```

If module visibility makes that path undesirable, mirror the settings shape in `settings.rs` and add `From<&LearningSettings> for LearningConfig`; do not create a dependency cycle.

- [x] **Step 4: Write failing store round-trip, append, and corruption tests**

Tests must use `tempfile::tempdir()` and local files only.

```rust
#[test]
fn candidate_round_trips_across_restart() {
    let dir = tempfile::tempdir().unwrap();
    let mut store = LearningStore::open(dir.path().to_path_buf()).unwrap();
    let candidate = fixture_candidate("cand-1");
    store.upsert_candidate(candidate.clone()).unwrap();
    drop(store);

    let store = LearningStore::open(dir.path().to_path_buf()).unwrap();
    assert_eq!(store.candidate("cand-1"), Some(&candidate));
}

#[test]
fn malformed_jsonl_line_does_not_destroy_valid_records() {
    // Write one valid record, one malformed line, then reopen.
    // Assert the valid record survives and diagnostics report one bad line.
}
```

For the malformed-line test, implement the file setup explicitly using `std::fs::write` with a valid serialized first line and `"{broken\n"` as the second line.

- [x] **Step 5: Implement `LearningStore` with atomic state writes and append-safe ledgers**

Required interface:

```rust
impl LearningStore {
    pub fn open(root: PathBuf) -> Result<Self, String>;
    pub fn candidate(&self, id: &str) -> Option<&LearningCandidate>;
    pub fn candidates(&self) -> &[LearningCandidate];
    pub fn upsert_candidate(&mut self, candidate: LearningCandidate) -> Result<(), String>;
    pub fn set_candidate_status(
        &mut self,
        id: &str,
        status: ArtifactStatus,
    ) -> Result<LearningCandidate, String>;
    pub fn skill(&self, name: &str) -> Option<&SkillLedgerRecord>;
    pub fn upsert_skill(&mut self, skill: SkillLedgerRecord) -> Result<(), String>;
    pub fn diagnostics(&self) -> &[String];
}
```

Use a write-to-`*.tmp` + `fs::rename` path for compacted snapshots. Keep in-memory maps keyed by id/name and serialize deterministic ordering.

- [x] **Step 6: Add `LearningController` to `NativeExtensionHost` without changing behavior**

In `learning/mod.rs`:

```rust
#[derive(Debug)]
pub struct LearningController {
    pub config: LearningConfig,
    pub project_store: LearningStore,
    pub global_store: LearningStore,
    pub stats: LearningStats,
}
```

In `native_extensions/mod.rs`, add:

```rust
pub mod learning;
pub use learning::*;
```

and:

```rust
pub learning: LearningController,
```

Construct it from the active `cwd` and `agent_dir`. If initialization fails, construct a disabled in-memory/fallback controller and retain the diagnostic; normal Davinci startup must continue.

- [x] **Step 7: Run crate tests and commit**

```bash
cargo test -p davinci-coding-agent learning
cargo test -p davinci-coding-agent settings
cargo fmt --check
git add crates/davinci-coding-agent/src/native_extensions/learning \
        crates/davinci-coding-agent/src/native_extensions/mod.rs \
        crates/davinci-coding-agent/src/settings.rs
git commit -m "feat: add learning controller storage and config"
```

---

### Task 2: Preserve the current skill loader while adding descriptor/provenance support

**Files:**
- Modify: `crates/davinci-agent/src/skills.rs`
- Create: `crates/davinci-coding-agent/src/native_extensions/learning/retrieval.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/learning/mod.rs`

**Interfaces:**
- Consumes: existing `Skill`, `discover_skills`, project/global roots.
- Produces:
  - `SkillDescriptor`
  - `SkillMatch`
  - `describe_skill(&Skill) -> SkillDescriptor`
  - `rank_skills(query, skills, ledger, limit) -> Vec<SkillMatch>`

- [x] **Step 1: Lock current `/skill:name` behavior with regression tests**

Keep the existing tests and add a project/global duplicate-precedence test.

```rust
#[test]
fn explicit_skill_expansion_remains_backward_compatible() {
    // Create a SKILL.md with current frontmatter and body.
    // Assert expand_skill_command still emits the same XML contract.
}
```

Run:

```bash
cargo test -p davinci-agent skills
```

Expected before changes: PASS. Treat this as a baseline gate.

- [x] **Step 2: Add a lightweight `SkillDescriptor` without changing `Skill.body` semantics**

In `davinci-agent/src/skills.rs`:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillDescriptor {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
    pub base_dir: PathBuf,
}

impl From<&Skill> for SkillDescriptor {
    fn from(skill: &Skill) -> Self {
        Self {
            name: skill.name.clone(),
            description: skill.description.clone(),
            path: skill.path.clone(),
            base_dir: skill.base_dir.clone(),
        }
    }
}
```

Do not remove `body`; explicit skill invocation still uses it.

- [x] **Step 3: Write failing deterministic lexical retrieval tests**

Use a fixture with:

```text
deploy-rust-flyio — Deploy and verify Rust applications on Fly.io
debug-sqlx — Diagnose SQLx compile and offline metadata failures
release-rust-cli — Prepare, verify, and publish a Rust CLI release
```

Assertions:

```rust
let hits = rank_skills("fix sqlx offline compile", &skills, &ledger, 3);
assert_eq!(hits[0].descriptor.name, "debug-sqlx");
```

Also assert:

- project scope beats global at equal textual relevance;
- an active skill beats candidate status;
- a skill with repeated verified failures receives a penalty;
- archived skills are excluded by default.

- [x] **Step 4: Implement lexical-first skill ranking**

Required score components:

```text
+ exact name token overlap
+ description token overlap
+ project-scope boost
+ active-status boost
+ log1p(success_count) boost
- failure-rate penalty
- archived/rejected exclusion
```

Normalize final score to `0.0..=1.0`.

Do not call Ollama/Qdrant in this task.

- [x] **Step 5: Run tests and commit**

```bash
cargo test -p davinci-agent skills
cargo test -p davinci-coding-agent rank_skills
cargo fmt --check
git add crates/davinci-agent/src/skills.rs \
        crates/davinci-coding-agent/src/native_extensions/learning/retrieval.rs \
        crates/davinci-coding-agent/src/native_extensions/learning/mod.rs
git commit -m "feat: add skill descriptors and retrieval ranking"
```

---

### Task 3: Add progressive-disclosure `skill_list` and `skill_view` native tools

**Files:**
- Modify: `crates/davinci-coding-agent/src/native_extensions/mod.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/learning/mod.rs`
- Modify: `crates/davinci-coding-agent/src/extension_host.rs`

**Interfaces:**
- Produces native tools:
  - `skill_list`
  - `skill_view`
- Consumes `Agent.skills`, learning ledger metadata, and the existing project/global skill roots.

- [x] **Step 1: Write failing tool-schema tests**

Expected schemas:

```json
{
  "name": "skill_list",
  "parameters": {
    "type": "object",
    "properties": {
      "query": {"type": "string"},
      "scope": {"type": "string", "enum": ["project", "global", "all"]},
      "status": {"type": "string", "enum": ["candidate", "pending_approval", "active", "archived", "all"]},
      "limit": {"type": "integer", "minimum": 1, "maximum": 20}
    }
  }
}
```

and:

```json
{
  "name": "skill_view",
  "parameters": {
    "type": "object",
    "properties": {
      "name": {"type": "string"},
      "file": {"type": "string"}
    },
    "required": ["name"]
  }
}
```

- [x] **Step 2: Add names to `NATIVE_TOOLS` and tool descriptions**

Descriptions must emphasize progressive disclosure:

```text
skill_list: List compact reusable skill descriptors relevant to a task without loading full skill bodies.
skill_view: Read the current full SKILL.md or an allowed supporting file for one skill.
```

- [x] **Step 3: Implement `skill_list` with a bounded response**

Return at most 20 records and no full skill body:

```json
{
  "skills": [
    {
      "name": "debug-sqlx",
      "description": "Diagnose SQLx compile and offline metadata failures",
      "scope": "project",
      "status": "active",
      "verifiedSuccesses": 3,
      "verifiedFailures": 0,
      "score": 0.91
    }
  ]
}
```

Use current discovered skills plus ledger records. Unknown legacy skills should be represented as `origin=user`, `status=active` for read purposes but must not become autonomously mutable.

- [x] **Step 4: Implement `skill_view` path safety**

Allowed files:

```text
SKILL.md
references/**
templates/**
scripts/**
```

Reject:

```text
../
absolute paths
symlink escape outside skill directory
assets/**  # defer until a later need
```

Return:

```json
{
  "name": "...",
  "file": "SKILL.md",
  "content": "...",
  "contentHash": "<sha256>",
  "origin": "learned_review",
  "scope": "project"
}
```

Record the content hash into the active background-review read set added in Task 4.

- [x] **Step 5: Confirm explicit `/skill:name` still works and commit**

```bash
cargo test -p davinci-agent skills
cargo test -p davinci-coding-agent skill_list
cargo test -p davinci-coding-agent skill_view
cargo fmt --check
git add crates/davinci-coding-agent/src/native_extensions/mod.rs \
        crates/davinci-coding-agent/src/native_extensions/learning \
        crates/davinci-coding-agent/src/extension_host.rs
git commit -m "feat: add progressive skill inspection tools"
```

---

### Task 4: Add safe agent-managed skill writes with ownership and read-before-write

**Files:**
- Create: `crates/davinci-coding-agent/src/native_extensions/learning/skill_manager.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/learning/mod.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/mod.rs`

**Interfaces:**
- Produces:
  - native tool `skill_manage`
  - `ReviewReadSet`
  - `SkillWriteOrigin`
  - `SkillManager::execute(...)`
- Consumes existing project trust result, skill roots, learning stores, secret redaction, and security scanner.

- [x] **Step 1: Write failing policy/safety tests**

Tests must cover:

```text
background review creates a project learned skill in a trusted project -> allowed
background review creates a project skill in an untrusted project -> denied
background review patches user-origin skill -> pending/denied, never direct write
background review patches learned skill without prior skill_view -> denied
background review patches learned skill with stale hash -> denied
foreground /learn patches learned skill -> allowed through normal policy
relative path ../outside -> denied
absolute support-file path -> denied
symlink escape -> denied
```

- [x] **Step 2: Define the write origin and read set**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SkillWriteOrigin {
    ForegroundUserDirected,
    BackgroundReview,
}

#[derive(Debug, Default)]
pub struct ReviewReadSet {
    seen: std::collections::HashMap<PathBuf, String>,
}

impl ReviewReadSet {
    pub fn record(&mut self, path: PathBuf, content_hash: String);
    pub fn matches(&self, path: &Path, content_hash: &str) -> bool;
    pub fn clear(&mut self);
}
```

- [x] **Step 3: Define one constrained `skill_manage` tool**

Supported initial actions:

```text
create
patch
write_file
archive
activate
reject
```

Do not implement physical recursive deletion in the first release.

Required argument shapes:

```json
{"action":"create","name":"debug-sqlx","scope":"project","description":"...","body":"...","candidateId":"cand-..."}
{"action":"patch","name":"debug-sqlx","oldText":"...","newText":"...","expectedHash":"...","candidateId":"cand-..."}
{"action":"write_file","name":"debug-sqlx","filePath":"references/sqlx-offline.md","content":"...","expectedHash":null,"candidateId":"cand-..."}
```

- [x] **Step 4: Implement safe create**

Rules:

- validate skill name as lowercase `[a-z0-9][a-z0-9-]{0,63}`;
- create only inside resolved known project/global skill root;
- project scope requires trust for autonomous writes;
- global background auto-write requires `config.auto_apply_global == true`, which defaults false;
- generate a portable `SKILL.md` with simple frontmatter:

```markdown
---
name: debug-sqlx
description: Diagnose SQLx compile and offline metadata failures
---

# Debug SQLx

## When to Use

...

## Procedure

...

## Pitfalls

...

## Verification

...
```

- [x] **Step 5: Implement safe patch with read-before-write**

Before write:

```rust
let current = fs::read_to_string(&path)?;
let current_hash = content_hash(&current);

if origin == SkillWriteOrigin::BackgroundReview
    && !read_set.matches(&path, &current_hash)
{
    return Err("background review must skill_view the current file before patching".into());
}

if current_hash != expected_hash {
    return Err("skill changed since review; reload with skill_view".into());
}
```

Require `old_text` to match exactly once. Zero or multiple matches are errors.

- [x] **Step 6: Implement safe supporting-file writes**

Allow only:

```text
references/
templates/
scripts/
```

Creation does not execute the file. A future execution still goes through normal Davinci tool permissions.

- [x] **Step 7: Run existing security scanning/redaction before activation**

At minimum:

- call existing secret-redaction helper on generated textual artifacts;
- add an adapter in `learning/skill_manager.rs` that invokes the existing security-scan primitives available without starting a full interactive scan;
- if no reusable primitive currently exists, fail closed for autonomous `scripts/` activation and allow only Markdown `references/`/`templates/` until the scanner adapter is factored.

The implementation must not duplicate the security scanner's pattern database.

- [x] **Step 8: Run tests and commit**

```bash
cargo test -p davinci-coding-agent skill_manage
cargo test -p davinci-coding-agent read_before_write
cargo test -p davinci-coding-agent learning_path
cargo fmt --check
git add crates/davinci-coding-agent/src/native_extensions/learning \
        crates/davinci-coding-agent/src/native_extensions/mod.rs
git commit -m "feat: add guarded agent-managed skill writes"
```

---

### Task 5: Build a bounded `LearningEvidence` snapshot from existing settled-turn data

**Files:**
- Create: `crates/davinci-coding-agent/src/native_extensions/learning/evidence.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/learning/types.rs`
- Modify: `crates/davinci-coding-agent/src/main.rs`

**Interfaces:**
- Consumes:
  - `Vec<MemoryMessage>` from existing `agent_memory_messages`
  - `Vec<AgentEvent>` already collected by the turn
  - `RunStats`
  - repo/session/turn identifiers
  - optional graph verification summary
- Produces:
  - `LearningEvidence`
  - `ToolEvidence`
  - `build_learning_evidence(...)`

- [x] **Step 1: Write a failing bounded-evidence test**

Create a tool result larger than 100 KB and assert the learning evidence contains only a bounded summary/tail, not the complete output.

```rust
#[test]
fn evidence_never_copies_unbounded_tool_output() {
    let huge = "x".repeat(100_000);
    let events = vec![fixture_tool_end("bash", huge)];
    let evidence = build_learning_evidence(fixture_input(events));
    assert!(evidence.serialized_len() < 20_000);
}
```

- [x] **Step 2: Define the evidence types**

```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ToolEvidence {
    pub name: String,
    pub is_error: bool,
    pub args_summary: String,
    pub result_summary: String,
    pub permission_denied: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LearningEvidence {
    pub session_id: String,
    pub repo_id: String,
    pub turn: u64,
    pub messages: Vec<MemoryMessage>,
    pub tools: Vec<ToolEvidence>,
    pub run_stats: davinci_agent::RunStats,
    pub verification: VerificationEvidence,
}
```

If `MemoryMessage` cannot be reused cleanly because of module ownership, define a small local `LearningMessage { role, content }`.

- [x] **Step 3: Implement sanitization and bounded summaries**

Rules:

- preserve only user and assistant messages;
- run existing secret redaction;
- cap each message at 4,000 chars;
- cap retained messages to the latest context needed for the review;
- tool arg summary max 1,000 chars;
- tool result summary max 2,000 chars;
- identify permission denial from structured details/prefix logic already used elsewhere instead of free-form model judgment.

- [x] **Step 4: Add graph verification adapter**

Add a pure conversion:

```rust
pub fn verification_evidence_from_graph(
    run_id: Option<String>,
    result: Option<&VerificationResult>,
) -> VerificationEvidence
```

The adapter must compute `commands_ran` as non-skipped commands and set:

```rust
passed = result.passed && commands_ran > 0
```

Add tests proving:

- one real passing command -> pass;
- only skipped commands -> fail;
- empty list -> fail;
- one failed command -> fail.

- [x] **Step 5: Build evidence beside the existing settled-turn memory snapshot**

Do not parse JSONL sessions again. Reuse the in-memory snapshot already available at the settled-turn call site.

Initially build and discard/log the evidence behind `learning.enabled` so this task adds no model call and no write behavior.

- [x] **Step 6: Run tests and commit**

```bash
cargo test -p davinci-coding-agent evidence
cargo test -p davinci-coding-agent verification_evidence
cargo fmt --check
git add crates/davinci-coding-agent/src/native_extensions/learning \
        crates/davinci-coding-agent/src/main.rs
git commit -m "feat: capture bounded settled-turn learning evidence"
```

---

### Task 6: Implement deterministic candidate and promotion policy

**Files:**
- Create: `crates/davinci-coding-agent/src/native_extensions/learning/policy.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/learning/types.rs`

**Interfaces:**
- Produces:
  - `CandidateDecision`
  - `evaluate_candidate(...)`
  - `may_auto_apply(...)`
  - `may_patch_skill(...)`
- Consumes learning config, candidate, skill ledger, trust, and verification evidence.

- [x] **Step 1: Write the policy truth-table tests first**

Required cases:

```rust
#[test]
fn graph_pass_with_real_command_can_promote_owned_project_skill() { /* Active */ }

#[test]
fn nothing_ran_can_never_promote() { /* Candidate */ }

#[test]
fn graph_failure_can_never_increment_success() { /* Candidate/Failure */ }

#[test]
fn user_correction_blocks_auto_promotion() { /* PendingApproval */ }

#[test]
fn untrusted_project_blocks_auto_write() { /* PendingApproval */ }

#[test]
fn global_auto_write_is_off_by_default() { /* PendingApproval */ }

#[test]
fn background_review_cannot_patch_user_skill() { /* PendingApproval */ }
```

- [x] **Step 2: Define deterministic decisions**

```rust
pub enum CandidateDecision {
    KeepCandidate,
    StageForApproval,
    AutoApply,
    Reject,
}
```

`AutoApply` requires:

```rust
candidate.confidence >= 0.80
&& !candidate.evidence.permission_denied
&& !candidate.evidence.user_corrected
&& candidate.evidence.passed
&& candidate.evidence.commands_ran > 0
&& scope_is_allowed
&& ownership_is_allowed
```

An explicit foreground `/learn` is allowed to bypass the graph-pass requirement because the user is explicitly authoring durable knowledge, but it still passes trust/permission/security validation.

- [x] **Step 3: Add repeated verified-use promotion**

For candidates/skills without a single graph-verified creation turn:

```rust
pub fn verified_use_threshold_met(
    skill: &SkillLedgerRecord,
    config: &LearningConfig,
) -> bool {
    skill.success_count >= config.auto_promote_verified_uses
        && skill.failure_count == 0
}
```

Do not count neutral/unverified uses as success.

- [x] **Step 4: Run tests and commit**

```bash
cargo test -p davinci-coding-agent policy
cargo fmt --check
git add crates/davinci-coding-agent/src/native_extensions/learning
git commit -m "feat: add verifier-backed learning promotion policy"
```

---

### Task 7: Add a fixture-driven background reviewer in shadow mode

**Files:**
- Create: `crates/davinci-coding-agent/src/native_extensions/learning/reviewer.rs`
- Create: `crates/davinci-coding-agent/src/native_extensions/learning/prompts.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/learning/mod.rs`

**Interfaces:**
- Produces:
  - `ReviewRun`
  - `ReviewResult`
  - `LearningReviewer`
  - `spawn_review(...)`
  - `cancel_review(...)`
- Consumes `LearningEvidence`, current model/runtime info, strict learning tool surface, and `PI_LEARNING_REVIEW_FIXTURE`.

- [x] **Step 1: Write fixture parsing tests**

Fixture environment variable content is JSON:

```json
{
  "candidates": [
    {
      "scope": "project",
      "confidence": 0.92,
      "rationale": "The same SQLx diagnosis sequence solved the compile failure.",
      "artifact": {
        "kind": "skill_create",
        "name": "debug-sqlx",
        "description": "Diagnose SQLx compile and offline metadata failures",
        "body": "# Debug SQLx\n\n## When to Use\n..."
      }
    }
  ]
}
```

Tests assert:

- more than configured candidates is truncated;
- malformed JSON produces an empty review result plus diagnostic;
- secrets are redacted before persistence;
- reviewer failure does not propagate as a turn failure.

- [x] **Step 2: Write the reviewer instruction with explicit decision order**

The prompt must tell the reviewer to choose in this order:

```text
1. Save a compact memory/failure lesson when the durable lesson is a fact.
2. Patch an existing relevant learned skill when the procedure belongs there.
3. Add a support file under an existing learned skill when detail is too large for SKILL.md.
4. Create a new class-level skill only when no existing skill covers the procedure.
5. Save nothing when the lesson is trivial, ephemeral, easily rediscovered, or unverified.
```

It must prohibit names tied only to the current ticket/task/date.

- [x] **Step 3: Implement review cancellation state**

```rust
pub struct ReviewRun {
    pub id: String,
    pub cancelled: Arc<AtomicBool>,
    pub finished: Arc<AtomicBool>,
}

impl ReviewRun {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Relaxed);
    }
}
```

`LearningController` keeps at most one active `ReviewRun`.

- [x] **Step 4: Implement fixture path before live-provider path**

When `PI_LEARNING_REVIEW_FIXTURE` is present:

- no provider call occurs;
- parse the fixture;
- apply max candidate count;
- persist candidates in shadow mode;
- mark stats.

This gives deterministic coverage for the entire orchestration.

- [x] **Step 5: Implement live review with a strict action surface**

Reuse Davinci's existing provider/runtime abstraction rather than shelling out to Hermes or adding a service.

The reviewer must have no direct source-mutation tools. If a separate scoped `Agent` is used, set the active tool list to:

```text
memory_search
skill_list
skill_view
skill_manage
```

If the first implementation uses structured model output without a tool loop, return only `LearningCandidate` JSON and let `LearningController` perform all reads/writes after deterministic checks. This is preferable for the first shadow-mode release because it is smaller and easier to secure.

- [x] **Step 6: Enforce review budgets**

Stop review when any threshold is reached:

```text
max_review_iterations
max_review_input_tokens
review_timeout_ms
cancelled == true
```

Budget exhaustion returns a diagnostic and preserves the foreground turn.

- [x] **Step 7: Run tests and commit**

```bash
PI_LEARNING_REVIEW_FIXTURE='{"candidates":[]}' \
  cargo test -p davinci-coding-agent reviewer
cargo test -p davinci-coding-agent review_cancel
cargo fmt --check
git add crates/davinci-coding-agent/src/native_extensions/learning
git commit -m "feat: add bounded background learning reviewer"
```

---

### Task 8: Wire review to the existing `AgentSettled` path and foreground cancellation

**Files:**
- Modify: `crates/davinci-coding-agent/src/main.rs`
- Modify: `crates/davinci-coding-agent/src/extension_host.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/mod.rs`
- Modify: `crates/davinci-coding-agent/src/davinci_interactive.rs`

**Interfaces:**
- Consumes existing settled-turn sequence:
  - `ExtensionEvent::TurnEnd`
  - `ExtensionEvent::AgentEnd`
  - `ExtensionEvent::AgentSettled`
  - `agent_memory_messages`
  - `native_index_messages`
- Produces:
  - `ExtensionHost::native_review_settled_turn(...)`
  - `ExtensionHost::native_cancel_learning_review()`

- [x] **Step 1: Add an integration test proving memory indexing remains first-class**

Fixture the host and assert one settled turn:

```text
indexes vector memory
creates one learning evidence snapshot
starts at most one review
returns foreground result regardless of review error
```

Do not make learning a prerequisite for `native_index_messages`.

- [x] **Step 2: Add host methods**

```rust
pub fn native_review_settled_turn(
    &self,
    evidence: LearningEvidence,
) -> Result<Option<String>, davinci_agent::ToolError>;

pub fn native_cancel_learning_review(&self);
```

`native_review_settled_turn` should return a review id/diagnostic, not block waiting for the entire review when a detached execution path is used.

- [x] **Step 3: Wire immediately after current memory indexing**

Target ordering:

```rust
host.emit(ExtensionEvent::TurnEnd);
host.emit(ExtensionEvent::AgentEnd);
host.emit(ExtensionEvent::AgentSettled);

let memory_messages = agent_memory_messages(agent);
let _ = host.native_index_messages(&memory_messages);

if learning_enabled {
    let evidence = build_learning_evidence(...);
    let _ = host.native_review_settled_turn(evidence);
}
```

Do not move or replace existing vector-memory indexing.

- [x] **Step 4: Cancel stale review at the start of a new live turn**

At the earliest point where a new user prompt is accepted for execution:

```rust
host.native_cancel_learning_review();
```

Cancellation must be best-effort and bounded. Never wait indefinitely for the review thread/request.

- [x] **Step 5: Add lifecycle shutdown cancellation**

On `SessionShutdown`, call both:

```text
graph::abort_all_runs()
learning.cancel_active_review()
```

- [x] **Step 6: Run tests and commit**

```bash
cargo test -p davinci-coding-agent settled_turn
cargo test -p davinci-coding-agent learning_review
cargo test -p davinci-coding-agent session_shutdown
cargo fmt --check
git add crates/davinci-coding-agent/src/main.rs \
        crates/davinci-coding-agent/src/extension_host.rs \
        crates/davinci-coding-agent/src/native_extensions \
        crates/davinci-coding-agent/src/davinci_interactive.rs
git commit -m "feat: review settled turns without blocking foreground work"
```

---

### Task 9: Add pending approval, activation, rejection, and learning status commands

**Files:**
- Modify: `crates/davinci-coding-agent/src/native_extensions/mod.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/learning/mod.rs`
- Modify: `crates/davinci-coding-agent/src/davinci_interactive.rs`
- Modify: `crates/davinci-coding-agent/src/rpc.rs`

**Interfaces:**
- Produces native commands:
  - `learning-status`
  - `learning-pending`
  - `learning-approve`
  - `learning-reject`
  - `skill-list`
  - `skill-view`
- Consumes `LearningStore`, policy, and skill manager.

- [x] **Step 1: Add command-discovery tests**

Every command must appear in `NATIVE_COMMANDS` and `command_specs()` with a description and argument hint where relevant.

- [x] **Step 2: Implement `learning-status`**

Return structured JSON with:

```json
{
  "enabled": true,
  "shadowMode": true,
  "activeReview": false,
  "project": {"candidates": 4, "activeSkills": 2},
  "global": {"candidates": 1, "activeSkills": 3},
  "stats": {
    "reviewsStarted": 8,
    "reviewsCompleted": 8,
    "reviewsCancelled": 1,
    "candidatesCreated": 5
  }
}
```

- [x] **Step 3: Implement pending/approve/reject**

Examples:

```text
/learning-pending
/learning-approve cand-abc123
/learning-approve all
/learning-reject cand-abc123
/learning-reject all
```

Approval re-runs policy/security/path checks at write time. It must not trust a stale decision made when the candidate was first created.

- [x] **Step 4: Add thin RPC exposure**

Return the same structured data through existing native command invocation rather than creating a second learning API.

- [x] **Step 5: Run tests and commit**

```bash
cargo test -p davinci-coding-agent learning_status
cargo test -p davinci-coding-agent learning_approve
cargo test -p davinci-coding-agent native_command
cargo fmt --check
git add crates/davinci-coding-agent/src/native_extensions \
        crates/davinci-coding-agent/src/davinci_interactive.rs \
        crates/davinci-coding-agent/src/rpc.rs
git commit -m "feat: add learning review and approval commands"
```

---

### Task 10: Add explicit `/learn` that feeds the same skill manager

**Files:**
- Modify: `crates/davinci-coding-agent/src/native_extensions/mod.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/learning/prompts.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/learning/mod.rs`
- Modify: `crates/davinci-coding-agent/src/davinci_interactive.rs`

**Interfaces:**
- Produces native command:
  - `/learn <instruction>`
  - `/learn --global <instruction>`
- Consumes existing agent tools/context and the same `skill_list`, `skill_view`, `skill_manage` path.

- [x] **Step 1: Write parse tests**

```rust
assert_eq!(
    parse_learn_args("--global release Rust crates"),
    LearnRequest {
        scope: LearningScope::Global,
        instruction: "release Rust crates".into(),
    }
);

assert_eq!(
    parse_learn_args("how we fixed SQLx offline mode"),
    LearnRequest {
        scope: LearningScope::Project,
        instruction: "how we fixed SQLx offline mode".into(),
    }
);
```

Empty instruction must return a usage error.

- [x] **Step 2: Build the foreground learning prompt**

Required instructions:

```text
Inspect existing skills before creating a new one.
Prefer updating an existing relevant skill.
Write a reusable procedure, not a transcript summary.
Include When to Use, Procedure, Pitfalls, and Verification.
Default project scope; global scope is explicit.
Use skill_manage for persistence.
```

- [x] **Step 3: Reuse the normal agent turn instead of creating a second ingestion engine**

`/learn` should transform the input into a normal foreground instruction that has access to normal read/search/web tools plus `skill_list`, `skill_view`, and `skill_manage`.

Because the user explicitly invoked `/learn`, skill persistence is user-directed, but project trust and normal permissions still apply.

- [x] **Step 4: Add duplicate-prevention behavior**

Before `create`, the agent or manager must search by name/description. If an active skill scores above the configured merge threshold, respond with the existing skill and require patch/update instead of creating a near duplicate.

Use deterministic manager-side enforcement as the final gate, not only prompt wording.

- [x] **Step 5: Run tests and commit**

```bash
cargo test -p davinci-coding-agent parse_learn
cargo test -p davinci-coding-agent learn
cargo fmt --check
git add crates/davinci-coding-agent/src/native_extensions \
        crates/davinci-coding-agent/src/davinci_interactive.rs
git commit -m "feat: add explicit reusable skill learning command"
```

---

### Task 11: Record skill usage and feed verified outcomes back into skill quality

**Files:**
- Modify: `crates/davinci-agent/src/skills.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/learning/types.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/learning/store.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/learning/retrieval.rs`
- Modify: `crates/davinci-coding-agent/src/main.rs`

**Interfaces:**
- Produces:
  - `SkillUse`
  - `record_skill_retrieved(name, turn)`
  - `record_skill_outcome(name, outcome)`
  - `SkillOutcome::{VerifiedSuccess, VerifiedFailure, Neutral}`
- Consumes explicit `/skill:name` expansion and automatic skill retrieval.

- [x] **Step 1: Add a way to identify which skill was injected**

Do not parse the final prompt text. Change expansion plumbing to optionally return metadata:

```rust
pub struct ExpandedUserText {
    pub text: String,
    pub skills: Vec<String>,
}

pub fn expand_user_text_with_metadata(
    text: &str,
    skills: &[Skill],
    templates: &[crate::PromptTemplate],
) -> ExpandedUserText;
```

Keep existing `expand_user_text(...) -> String` as a wrapper for backward compatibility.

- [x] **Step 2: Write outcome accounting tests**

Required rules:

```text
graph verified pass -> success_count + 1
graph verified fail -> failure_count + 1
no deterministic verification -> neutral_count + 1
only skipped verification commands -> neutral_count + 1, never success
```

- [x] **Step 3: Persist use/outcome metrics in the ledger**

Update:

```text
last_used_at_ms
success_count
failure_count
neutral_count
updated_at_ms
```

Do not append unbounded per-use history in `SkillLedgerRecord`.

- [x] **Step 4: Use metrics in retrieval ranking**

Verified success increases ranking modestly; high verified failure ratio decreases it. Textual relevance must remain the dominant signal so an unrelated successful skill never outranks a relevant one.

- [x] **Step 5: Run tests and commit**

```bash
cargo test -p davinci-agent expand_user_text
cargo test -p davinci-coding-agent skill_outcome
cargo test -p davinci-coding-agent rank_skills
cargo fmt --check
git add crates/davinci-agent/src/skills.rs \
        crates/davinci-coding-agent/src/native_extensions/learning \
        crates/davinci-coding-agent/src/main.rs
git commit -m "feat: learn from verified skill usage outcomes"
```

---

### Task 12: Upgrade vector-memory promotion with learning provenance instead of replacing it

**Files:**
- Modify: `crates/davinci-coding-agent/src/native_extensions/vector_memory.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/learning/mod.rs`

**Interfaces:**
- Consumes existing `MemoryRecord`, `MemoryChunk`, `promote_chunk`, `index_messages`.
- Produces backward-compatible optional provenance fields and `index_learning_memory(...)`.

- [x] **Step 1: Add backward-compatibility deserialization test**

Serialize an old-shape record without any new fields and assert it still deserializes.

```rust
#[test]
fn old_memory_record_without_learning_fields_still_loads() {
    let value = serde_json::json!({
        "id":"1",
        "repoId":"repo",
        "kind":"fact",
        "text":"uses pnpm",
        "source":"turn 1",
        "contentHash":"abc",
        "importance":0.8,
        "createdAt":1
    });
    let record: MemoryRecord = serde_json::from_value(value).unwrap();
    assert_eq!(record.use_count, 0);
    assert!(record.verification.is_none());
}
```

- [x] **Step 2: Add optional learning fields with defaults**

```rust
#[serde(default)]
pub confidence: Option<f32>,
#[serde(default)]
pub source_session_id: Option<String>,
#[serde(default)]
pub source_turn: Option<u64>,
#[serde(default)]
pub verification: Option<String>,
#[serde(default)]
pub use_count: u64,
#[serde(default)]
pub last_used_at: Option<u64>,
```

- [x] **Step 3: Add a dedicated reviewed-memory indexing path**

```rust
pub fn index_learning_memory(
    &mut self,
    text: &str,
    kind: MemoryKind,
    importance: f32,
    confidence: f32,
    source_session_id: &str,
    source_turn: u64,
    verification: Option<&str>,
) -> Result<String, ToolError>;
```

It must reuse:

```text
redact_secrets
content_hash
repo_id
existing local/Qdrant persistence
existing dense fallback
```

- [x] **Step 4: Keep heuristic promotion but lower its authority**

`promote_chunk` may continue generating high-importance memory chunks cheaply, but only reviewed/verified memory records receive verification provenance.

Do not delete or rewrite existing memory indexes.

- [x] **Step 5: Run tests and commit**

```bash
cargo test -p davinci-coding-agent vector_memory
cargo test -p davinci-coding-agent old_memory_record
cargo fmt --check
git add crates/davinci-coding-agent/src/native_extensions/vector_memory.rs \
        crates/davinci-coding-agent/src/native_extensions/learning/mod.rs
git commit -m "feat: add learning provenance to vector memory"
```

---

### Task 13: Reuse vector-memory retrieval primitives for dense skill matching

**Files:**
- Modify: `crates/davinci-coding-agent/src/native_extensions/vector_memory.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/learning/retrieval.rs`

**Interfaces:**
- Produces reusable non-memory-specific helpers:
  - `embed_document_text(...)`
  - `embed_query_text(...)`
  - `cosine_similarity(...)`
- Consumes current `VectorMemoryConfig` and dense-backoff behavior.

- [x] **Step 1: Extract pure similarity helpers with no behavior change**

Move only generic operations; do not alter current memory ranking results.

Add a regression test that current memory search fixture ordering is identical before/after extraction.

- [x] **Step 2: Add a dense-skill-ranking fixture path**

Use deterministic fixture vectors in tests; never call Ollama.

Example:

```rust
let query = vec![1.0, 0.0];
let sqlx = vec![0.99, 0.01];
let deploy = vec![0.0, 1.0];
assert!(cosine_similarity(&query, &sqlx) > cosine_similarity(&query, &deploy));
```

- [x] **Step 3: Merge lexical and dense scores**

Use:

```text
final = 0.65 * lexical + 0.35 * dense
```

when dense is available. When dense is unavailable:

```text
final = lexical
```

Then apply small scope/status/usage adjustments.

- [x] **Step 4: Preserve fail-open/backoff behavior**

A dead Ollama/Qdrant instance must not make a prompt wait repeatedly or fail skill retrieval. Use the same backoff principle already present in vector memory.

- [x] **Step 5: Run tests and commit**

```bash
cargo test -p davinci-coding-agent vector_memory
cargo test -p davinci-coding-agent dense_skill
cargo fmt --check
git add crates/davinci-coding-agent/src/native_extensions/vector_memory.rs \
        crates/davinci-coding-agent/src/native_extensions/learning/retrieval.rs
git commit -m "feat: reuse memory embeddings for skill retrieval"
```

---

### Task 14: Move from shadow mode to safe project auto-application behind configuration

**Files:**
- Modify: `crates/davinci-coding-agent/src/native_extensions/learning/mod.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/learning/policy.rs`
- Modify: `crates/davinci-coding-agent/src/settings.rs`

**Interfaces:**
- Consumes all prior candidate/policy/manager functionality.
- Produces controlled rollout states:
  - shadow only;
  - project auto-apply;
  - global auto-apply, still off by default.

- [x] **Step 1: Add end-to-end fixture test for shadow mode**

Given a review fixture proposing a valid project skill:

```text
candidate exists
SKILL.md does not exist
status == Candidate or PendingApproval
```

- [x] **Step 2: Add end-to-end fixture test for trusted project auto-apply**

Configure:

```json
{
  "learning": {
    "enabled": true,
    "backgroundReview": true,
    "shadowMode": false,
    "autoApplyProject": true,
    "autoApplyGlobal": false
  }
}
```

Provide strong verification evidence. Assert:

```text
SKILL.md created under <repo>/.pi/skills/
ledger origin == learned_review
status == active
```

- [x] **Step 3: Add negative end-to-end tests**

Assert no autonomous write for:

```text
untrusted project
global candidate while autoApplyGlobal=false
user-origin target skill
failed verification
nothing-ran verification
stale expected hash
security rejection
```

- [x] **Step 4: Implement auto-application only by routing through the same manager**

There must be exactly one write implementation. Background review, approval command, and `/learn` all call `SkillManager`; no special filesystem write path is allowed.

- [x] **Step 5: Run tests and commit**

```bash
cargo test -p davinci-coding-agent learning_e2e
cargo test -p davinci-coding-agent skill_manage
cargo fmt --check
git add crates/davinci-coding-agent/src/native_extensions/learning \
        crates/davinci-coding-agent/src/settings.rs
git commit -m "feat: enable verifier-gated project learning"
```

---

### Task 15: Add observability without destabilizing the existing TUI contracts

**Files:**
- Modify: `crates/davinci-coding-agent/src/native_extensions/learning/types.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/learning/mod.rs`
- Modify: `crates/davinci-coding-agent/src/davinci_surfaces.rs`
- Modify: `crates/davinci-coding-agent/src/rpc.rs`
- Modify: `crates/davinci-coding-agent/src/davinci_interactive.rs`

**Interfaces:**
- Consumes `LearningStats` and store counts.
- Produces text/structured status through existing command/RPC surfaces.

- [x] **Step 1: Add counter tests**

Counters:

```text
reviews_started
reviews_completed
reviews_cancelled
reviews_failed
candidates_created
candidates_approved
candidates_rejected
skills_created
skills_patched
skills_retrieved
verified_skill_successes
verified_skill_failures
```

- [x] **Step 2: Add concise transcript notifications**

Default messages should be low-noise:

```text
learning · candidate saved: debug-sqlx
learning · skill activated: debug-sqlx
learning · review cancelled by new turn
```

Do not print from a background thread directly into raw TUI stdout. Route through the same hosted/event mechanism already used by the product.

- [x] **Step 3: Expose structured status to RPC**

Reuse the JSON returned by `/learning-status`. Do not create a second independent stats representation.

- [x] **Step 4: Do not add a new visual instrument yet**

If a future TUI sheet is desired, write a separate design/spec because current Davinci screens are governed by strict artboard contracts.

- [x] **Step 5: Run tests and commit**

```bash
cargo test -p davinci-coding-agent learning_status
cargo test -p davinci-coding-agent rpc
cargo test -p davinci-tui
cargo fmt --check
git add crates/davinci-coding-agent/src/native_extensions/learning \
        crates/davinci-coding-agent/src/davinci_surfaces.rs \
        crates/davinci-coding-agent/src/rpc.rs \
        crates/davinci-coding-agent/src/davinci_interactive.rs
git commit -m "feat: expose learning status and lifecycle notifications"
```

---

### Task 16: Add failure-learning and safe skill refinement

**Files:**
- Modify: `crates/davinci-coding-agent/src/native_extensions/learning/policy.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/learning/reviewer.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/learning/skill_manager.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/learning/store.rs`

**Interfaces:**
- Consumes verified failures and previously used learned skills.
- Produces:
  - failure lessons;
  - candidate skill patches;
  - version increments after approved patch.

- [x] **Step 1: Add verified-failure test**

Scenario:

```text
skill debug-sqlx was loaded
verification ran cargo test
cargo test failed
```

Assert:

```text
failure_count increments
success_count does not
active skill is not silently overwritten
review may propose SkillPatch or FailureLesson candidate
```

- [x] **Step 2: Version learned skills on every accepted patch**

On successful patch:

```text
version += 1
content_hash = new hash
updated_at_ms = now
```

Do not put the version into `SKILL.md` unless the user already uses such metadata; keep Davinci's operational version in the ledger.

- [x] **Step 3: Prefer patch over duplicate skill**

When a new candidate matches an existing active learned skill above the merge threshold, convert:

```text
SkillCreate -> SkillPatch candidate
```

or stage it for review if a deterministic patch cannot be constructed safely.

- [x] **Step 4: Add rollback data**

Before patching a Davinci-owned learned skill, persist the previous `SKILL.md` content in a bounded history:

```text
<learning-root>/history/<skill-id>/<version>.md
```

Retain the latest 5 versions per learned skill. Archive rotation is deterministic and limited to files owned by the learning subsystem.

- [x] **Step 5: Run tests and commit**

```bash
cargo test -p davinci-coding-agent skill_refine
cargo test -p davinci-coding-agent failure_lesson
cargo fmt --check
git add crates/davinci-coding-agent/src/native_extensions/learning
git commit -m "feat: refine learned skills from verified outcomes"
```

---

### Task 17: Add full offline integration fixtures and rollout evaluation

**Files:**
- Modify: relevant inline test modules in all learning files
- Modify: `crates/davinci-evals/` only if its current public interfaces make this feature easy to measure without broad refactoring
- Modify: `CLAUDE.md`

**Interfaces:**
- Produces deterministic evaluation scenarios and documented fixture variables.
- Consumes `PI_LEARNING_REVIEW_FIXTURE`.

- [x] **Step 1: Standardize test-only environment hooks**

Document and implement:

```text
PI_LEARNING_REVIEW_FIXTURE
PI_LEARNING_DISABLE_BACKGROUND
PI_LEARNING_CLOCK_MS
```

Use a process-global mutex in tests that mutate these variables, matching existing repository patterns.

- [x] **Step 2: Add scenario: successful reusable workflow**

Fixture:

```text
user asks to diagnose SQLx compile failure
agent uses repeatable sequence
verification passes with real command
review proposes debug-sqlx
```

Assert candidate quality fields, policy decision, and eventual skill file.

- [x] **Step 3: Add scenario: one-off task is not a skill**

Fixture review returns no candidate for:

```text
rename one local variable
read one file
answer a general Rust syntax question
temporary path discovered during one run
```

- [x] **Step 4: Add scenario: correction overrides prior lesson**

Conversation includes an explicit user correction. Assert automatic promotion is blocked and the candidate is pending/rejected according to policy.

- [x] **Step 5: Add scenario: restart persistence**

Create learned candidate/skill, destroy controller, recreate from same temp roots, then verify retrieval and counters survive.

- [x] **Step 6: Add scenario: provider/memory accelerator unavailable**

Disable network and omit Ollama/Qdrant fixtures. Assert:

```text
normal turn works
lexical skill retrieval works
learning store works
background reviewer fixture works
```

- [x] **Step 7: Run the complete quality gate**

```bash
make test
make fmt
make clippy
cargo run -p davinci-parity
```

Expected:

```text
all workspace tests pass
format check passes
clippy passes with -D warnings
parity fixtures show no unintended regressions
```

- [x] **Step 8: Commit**

```bash
git add crates CLAUDE.md
git commit -m "test: cover self-improving learning lifecycle"
```

---

### Task 18: Document the user-facing learning model and safe defaults

**Files:**
- Modify: `README.md`
- Modify: `crates/davinci-coding-agent/README.md`
- Modify: `docs/README.md`
- Modify: `CLAUDE.md`
- Create: `docs/learning.md`

**Interfaces:**
- Documents the stable behavior implemented by Tasks 1–17.

- [x] **Step 1: Document the conceptual split**

`docs/learning.md` must explain:

```text
Memory = durable facts
Skills = reusable procedures
Candidates = proposed learning not yet trusted enough to activate
Verification = deterministic evidence that may promote learning
```

- [x] **Step 2: Document storage and ownership**

Include exact project/global paths and explain which artifacts autonomous review may modify.

- [x] **Step 3: Document commands**

```text
/learn [--global] <instruction>
/learning-status
/learning-pending
/learning-approve <id|all>
/learning-reject <id|all>
/skill-list [query]
/skill-view <name>
```

Keep existing `/skill:name` documentation.

- [x] **Step 4: Document settings with safe defaults**

```json
{
  "learning": {
    "enabled": true,
    "backgroundReview": true,
    "shadowMode": true,
    "autoApplyProject": false,
    "autoApplyGlobal": false,
    "maxCandidatesPerReview": 3,
    "maxReviewInputTokens": 12000,
    "maxReviewIterations": 6,
    "autoPromoteVerifiedUses": 2,
    "reviewTimeoutMs": 30000
  }
}
```

- [x] **Step 5: Document failure behavior**

State explicitly:

```text
learning failures do not fail normal turns
review can be disabled
dense retrieval failure falls back to lexical retrieval
untrusted projects do not receive autonomous project writes
global autonomous writes are off by default
```

- [x] **Step 6: Run docs-related code quality gates and commit**

```bash
make fmt
make clippy
git add README.md \
        crates/davinci-coding-agent/README.md \
        docs/README.md \
        docs/learning.md \
        CLAUDE.md
git commit -m "docs: describe Davinci self-improving learning"
```

---

## Post-MVP extension: explicit self-code-improvement through the graph engine

Do **not** include this capability in automatic background review.

After Tasks 1–18 are stable, create a separate spec/plan for an explicit command such as:

```text
/self-improve <goal>
```

Its implementation should reuse the current graph engine:

```text
explicit command
    -> isolated branch/worktree
    -> graph research/planning
    -> implementation workers
    -> deterministic graph verification
    -> review node
    -> final diff
    -> user-controlled merge/PR
```

The learning subsystem may provide the graph with relevant memories/skills, and the resulting verified work may produce new learning candidates, but it must never merge its own source changes autonomously.

---

## Recommended rollout order

Do not enable full autonomous application at once.

### Milestone 1 — useful with zero autonomous mutation

Complete Tasks 1–9.

Result:

```text
settled turns -> background review -> candidates -> status/pending UI
```

Existing memory and skills continue unchanged.

### Milestone 2 — explicit learning

Complete Task 10.

Result:

```text
/learn -> reusable skill through the same safe manager
```

This is immediately useful even before background auto-apply.

### Milestone 3 — compounding quality

Complete Tasks 11–13 and 16.

Result:

```text
skill retrieval -> use -> deterministic outcome -> quality metrics -> safe refinement
```

This is the point where Davinci actually "gets better" from repeated work.

### Milestone 4 — guarded autonomy

Complete Tasks 14–15.

Initially enable:

```text
shadowMode=false
autoApplyProject=true
autoApplyGlobal=false
```

only after fixture/eval quality is acceptable.

### Milestone 5 — stabilize and document

Complete Tasks 17–18, then decide whether global promotion or a visual learning UI deserves a separate design.

---

## Acceptance checklist

Before calling the feature complete, verify every item:

- [x] Existing `SKILL.md` files still discover without metadata migration.
- [x] Existing `/skill:name` expansion remains unchanged.
- [x] Existing vector-memory data still loads.
- [x] Existing memory indexing still executes after a settled turn.
- [x] Background review is launched only after the normal turn is settled.
- [x] At most one background review runs per session.
- [x] A new foreground turn can cancel/supersede review.
- [x] Review timeout/budget exhaustion cannot fail the foreground turn.
- [x] `skill_list` returns descriptors, not full bodies.
- [x] `skill_view` is required before autonomous patch.
- [x] Stale `expected_hash` blocks autonomous patch.
- [x] Path traversal and symlink escape are blocked.
- [x] User/imported skills are not autonomously mutated.
- [x] Untrusted projects receive no autonomous project writes.
- [x] Global autonomous writes are disabled by default.
- [x] "Nothing ran" graph verification is never considered success.
- [x] Failed verification never increments skill success.
- [x] Unverified turns increment neutral usage only.
- [x] Reviewed memory reuses the current vector store.
- [x] Skill dense retrieval reuses existing embedding infrastructure.
- [x] Lexical retrieval works with Ollama/Qdrant absent.
- [x] Secret redaction happens before learned persistence.
- [x] Newly written skill scripts are never auto-executed.
- [x] Background review has no direct source-mutation tool.
- [x] Learned-skill patches create bounded rollback history.
- [x] Learning can be disabled without affecting normal Davinci behavior.
- [x] `make test`, `make fmt`, `make clippy`, and parity checks pass.
