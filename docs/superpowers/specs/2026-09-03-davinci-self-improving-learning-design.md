# Davinci Self-Improving Learning System — Design

**Repository:** `J12003LPZ/davinci`  
**Grounding snapshot:** `main` at `2b5481fb18b12b09ffdcf09a575bb617ab555195`  
**Design date:** 2026-09-03  
**Reference implementation studied:** `NousResearch/hermes-agent`

## 1. Purpose

Davinci already has several pieces that a self-improving agent needs:

- durable, repo-aware vector memory with lexical fallback;
- automatic memory retrieval;
- memory chunk promotion;
- `SKILL.md` discovery from both project and global roots;
- `/skill:name` expansion;
- project trust and permission gates;
- lifecycle events including `TurnEnd`, `AgentEnd`, and `AgentSettled`;
- structured tool events;
- deterministic graph verification;
- token-governor/evidence infrastructure;
- subagents and isolated graph workers.

The missing layer is not another memory store and not another skill loader. The missing layer is a **learning control plane** that decides what a completed turn taught the agent, records that lesson with provenance, validates it, and feeds it back into Davinci's existing memory and skill systems.

The target loop is:

```text
normal Davinci turn
        |
        v
AgentSettled
        |
        +----> existing vector-memory indexing
        |
        v
LearningEvidence snapshot
        |
        v
bounded background review
        |
        v
LearningCandidate(s)
   | memory fact
   | new skill
   | patch existing learned skill
   | support reference/script/template
   | failure lesson
        |
        v
policy + provenance + verification gates
        |
        +----> pending approval
        |
        +----> candidate
        |
        +----> active learned artifact
                    |
          +---------+----------+
          |                    |
          v                    v
 existing memory          existing skill discovery
 retrieval                 + /skill:name
          |                    |
          +---------+----------+
                    |
               future turns
                    |
            usage + outcome evidence
                    |
            refine / promote / demote
```

## 2. Design principles

### 2.1 Extend Davinci; do not fork its architecture

The implementation must preserve the current ownership boundaries:

- `davinci-agent` owns generic agent concepts such as skill parsing and prompt expansion.
- `davinci-coding-agent` owns product/runtime policy and native extensions.
- vector memory remains the durable semantic memory implementation.
- skill files remain ordinary `SKILL.md` resources under the roots Davinci already scans.
- graph verification remains the source of deterministic coding-task verification.

No new workspace crate is needed for the first implementation.

### 2.2 Facts and procedures remain distinct

Use memory for declarative knowledge:

- repository conventions;
- architecture facts;
- corrections;
- constraints;
- durable failure causes;
- completed migration or setup facts.

Use skills for procedural knowledge:

- repeatable debugging workflows;
- release/deployment procedures;
- test-and-verify sequences;
- repository-specific maintenance procedures;
- tool-specific workflows.

A successful session should not automatically become a skill. A skill must describe a repeatable **class of task**, not today's task instance.

### 2.3 Learning is evidence-backed

The language model may propose what was learned, but it must not decide by itself whether the result was objectively successful.

Davinci already has deterministic graph verification. Preserve its invariant:

> A verification result is positive only when at least one real command ran and every non-skipped command passed.

For non-graph turns, "assistant said it worked" is not proof. Those outcomes can create candidates, but automatic promotion must require stronger evidence such as explicit user acceptance or repeated verified usage.

### 2.4 Foreground work always wins

Background review is optional, bounded, cancelable, and fail-open.

A new user turn must never wait indefinitely for learning work. At most one automatic review may exist for a session. A new foreground turn supersedes or cancels the previous review.

### 2.5 Autonomous writes are narrower than foreground writes

The background reviewer may:

- create or patch artifacts Davinci itself previously created as learned artifacts;
- create project-scoped candidates when the project is trusted;
- write only through the learning/skill-management API.

The background reviewer may not directly mutate:

- user-authored skills;
- package/imported skills;
- project skills in an untrusted project;
- arbitrary repository files;
- Davinci's own source code.

A foreground user-directed `/learn` action may target a wider scope, but still passes Davinci's normal permission/trust rules.

### 2.6 Backward compatibility is a requirement

Existing skills without learning metadata must continue to load and run exactly as they do now.

Existing vector-memory records must continue to deserialize. Any new fields require `serde(default)` or an equivalent migration path.

## 3. What to reuse from Hermes

The useful Hermes concepts are architectural, not code-level ports:

1. **Memory vs skills separation** — facts versus procedures.
2. **Background post-turn review** — inspect a settled conversation for durable lessons.
3. **Progressive skill disclosure** — list metadata first, load full skill only when useful.
4. **Agent-managed skills** — create/patch procedural knowledge through a constrained tool.
5. **`/learn`** — explicitly turn a workflow or source into a skill.
6. **Read-before-write** — autonomous skill patching must first read the exact current version.
7. **Provenance/ownership rules** — autonomous maintenance may only modify owned learned artifacts.
8. **Write staging/approval** — uncertain or user-owned changes remain pending.
9. **Background review budgets** — self-improvement must not consume unbounded model/tool time.

## 4. What not to copy from Hermes

Do not replace Davinci vector memory with small always-injected Markdown files. Davinci already has a stronger repo-scoped semantic store.

Do not introduce a separate Python service or Hermes-compatible runtime.

Do not make all skills global. Davinci already has a natural project/global split:

```text
<repo>/.pi/skills/
<agent_dir>/skills/
```

Do not let background learning run arbitrary shell/edit/write tools.

Do not give the reviewer direct self-source modification capability.

Do not build a Learning Journey/Star Map UI in the first release. Davinci's TUI has strict visual contracts; text commands and status data are enough to validate the subsystem first.

## 5. Proposed module boundaries

```text
crates/davinci-coding-agent/src/native_extensions/
├── mod.rs
├── vector_memory.rs
├── graph/
└── learning/
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

Responsibilities:

- `config.rs` — learning configuration and defaults.
- `types.rs` — candidates, scope, ownership, verification evidence, ledger records, stats.
- `store.rs` — atomic persistence, dedupe, pending/active/archive state.
- `evidence.rs` — bounded settled-turn snapshot built from existing events/messages/stats.
- `policy.rs` — deterministic promotion and write-authorization rules.
- `reviewer.rs` — background-review scheduling, cancellation, fixture path, model output parsing.
- `skill_manager.rs` — safe create/patch/support-file/archive operations.
- `retrieval.rs` — match skill descriptors to a user request, reusing existing dense/lexical machinery where possible.
- `prompts.rs` — stable reviewer and `/learn` instruction text.
- `mod.rs` — `LearningController`, native tool/command dispatch, lifecycle hooks.

`crates/davinci-agent/src/skills.rs` remains the canonical generic `SKILL.md` loader and explicit skill expander.

## 6. Storage layout

Use existing skill locations for actual procedural content.

Add a small operational ledger separate from `SKILL.md` so Davinci-specific provenance does not break skill portability.

```text
<agent_dir>/learning/
├── candidates.jsonl
├── skills.jsonl
└── state.json

<repo>/.pi/learning/
├── candidates.jsonl
├── skills.jsonl
└── state.json
```

Actual skills stay here:

```text
<agent_dir>/skills/<skill-name>/SKILL.md
<repo>/.pi/skills/<skill-name>/SKILL.md
```

This avoids expanding Davinci's current frontmatter parser into a nested-YAML configuration system just to support internal bookkeeping.

## 7. Core types

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

A candidate always records provenance:

```rust
pub struct LearningCandidate {
    pub id: String,
    pub scope: LearningScope,
    pub status: ArtifactStatus,
    pub artifact: LearningArtifact,
    pub confidence: f32,
    pub source_session_id: String,
    pub source_repo_id: String,
    pub source_turn: u64,
    pub created_at_ms: u64,
    pub evidence: VerificationEvidence,
    pub rationale: String,
}
```

Skill operational metadata:

```rust
pub struct SkillLedgerRecord {
    pub skill_id: String,
    pub name: String,
    pub scope: LearningScope,
    pub origin: SkillOrigin,
    pub status: ArtifactStatus,
    pub path: PathBuf,
    pub content_hash: String,
    pub version: u32,
    pub success_count: u64,
    pub failure_count: u64,
    pub neutral_count: u64,
    pub last_used_at_ms: Option<u64>,
    pub created_at_ms: u64,
    pub updated_at_ms: u64,
    pub pinned: bool,
}
```

## 8. Evidence model

`LearningEvidence` is generated only after a normal turn settles.

It should contain:

- sanitized user/assistant messages;
- compact tool-call records from `AgentEvent`;
- error/success flags;
- permission-denial signals;
- `RunStats`;
- repository id;
- session id;
- turn number;
- optional graph `VerificationResult`;
- optional skill usage records from the turn.

Do not embed complete large tool outputs. Preserve only:

- tool name;
- short arguments after secret redaction;
- `is_error`;
- a bounded result tail or evidence-store id;
- deterministic verification summary.

## 9. Promotion policy

### 9.1 Strong verified success

A candidate may become active automatically when all are true:

- it targets a Davinci-owned learned artifact or creates a new project candidate;
- project scope is trusted;
- deterministic verification passed;
- at least one verification command actually ran;
- no permission denial occurred;
- no explicit user correction contradicts the lesson;
- security/path validation passed.

### 9.2 Unverified success

A normal chat/code turn without deterministic verification may create a candidate but should not immediately become an active learned workflow.

Promotion paths:

- user explicitly approves;
- same candidate/skill succeeds in two independently verified future uses;
- an explicit foreground `/learn` creates it with user intent.

### 9.3 Failure

A failed verification must never increment skill success.

The reviewer may still create a durable failure lesson such as:

> This repository's SQLx macros require `DATABASE_URL` or offline metadata at compile time.

It may also propose a patch to a learned debugging skill, but that patch follows the same ownership/read-before-write rules.

## 10. Skill ownership and read-before-write

For a background review to patch an existing skill:

1. the skill must be Davinci-owned (`LearnedReview`, or an explicitly opted-in `LearnedForeground`);
2. the review must call `skill_view` for the exact target;
3. `skill_view` records `(resolved_path, content_hash)` in the active review's read set;
4. the patch includes `expected_hash`;
5. the manager re-hashes the file immediately before the write;
6. mismatch means the patch is rejected as stale.

This prevents autonomous review from patching content it only inferred from the transcript.

## 11. Progressive disclosure

Keep explicit `/skill:name` unchanged.

Add model-facing tools:

```text
skill_list(query?, scope?, status?, limit?)
skill_view(name, file?)
skill_manage(...)
```

Normal automatic selection should expose only compact descriptors:

```text
name
description
scope
status
success/failure counters
```

Full `SKILL.md` content is loaded only when:

- the user explicitly invokes the skill;
- the agent calls `skill_view`;
- retrieval selects the skill above the configured threshold.

## 12. Skill retrieval

Do not create another vector service.

Phase 1:

- lexical matching over name, description, headings, and optional trigger lines;
- deterministic boost from project scope;
- deterministic boost from prior verified success;
- penalty from failure rate and archived/candidate status.

Phase 2:

- factor reusable embedding/scoring primitives from `vector_memory.rs`;
- embed skill descriptors with the same configured embedding model;
- use the same dense-backoff/fail-open behavior;
- merge lexical+dense scores.

The user experience must remain functional when Ollama and Qdrant are unavailable.

## 13. Manual `/learn`

`/learn` is foreground, explicit learning.

Examples:

```text
/learn how we just fixed the SQLx offline build
/learn this deployment workflow
/learn --global the Rust release checklist we just used
```

It should:

1. gather context with normal agent tools;
2. inspect existing skills first;
3. prefer patching a relevant skill over creating a duplicate;
4. create a class-level reusable procedure;
5. call `skill_manage`;
6. pass normal trust/permission policy;
7. default to project scope unless `--global` is explicit.

## 14. Background review execution

The review should run after:

```text
TurnEnd
AgentEnd
AgentSettled
existing memory indexing
```

The reviewer gets a cloned, bounded snapshot and a strict tool surface:

```text
memory_search
skill_list
skill_view
skill_manage
```

No direct:

```text
bash
powershell
write
edit
apply_patch
mcp write-capable tools
graph_run
```

The review has:

- one active run per session;
- an atomic cancellation flag;
- a request/token budget;
- a max tool/model iteration count;
- a fixture-driven offline test path;
- no ability to fail the foreground response.

A new foreground turn cancels/supersedes an old review.

## 15. Memory integration

Keep vector memory as the main long-term fact store.

Improve it incrementally with backward-compatible metadata:

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

Use `serde(default)` so existing stored records remain valid.

Replace heuristic-only promotion with a combined rule:

```text
heuristic candidate extraction
        +
learning-review candidate
        +
verification evidence
        =
promotion decision
```

The old heuristic can remain as a low-cost candidate generator; it should no longer be treated as equivalent to verified learning.

## 16. Security

Every autonomous persistence path must:

- call the existing secret redaction before storage;
- reject path traversal;
- reject writes outside known skill roots;
- reject symlink/junction redirection before destructive operations;
- use atomic temp-file + rename;
- scan learned skill content with Davinci's existing security scanner before activation when practical;
- treat skill text as untrusted retrieved context, not privileged system instructions;
- never auto-execute a newly written `scripts/` file;
- preserve the normal tool permission gate when that script is later executed.

## 17. Observability

Add `LearningStats` rather than overloading parity-sensitive `RunStats` initially:

```rust
pub struct LearningStats {
    pub reviews_started: u64,
    pub reviews_completed: u64,
    pub reviews_cancelled: u64,
    pub reviews_failed: u64,
    pub candidates_created: u64,
    pub candidates_approved: u64,
    pub candidates_rejected: u64,
    pub skills_created: u64,
    pub skills_patched: u64,
    pub skills_retrieved: u64,
    pub verified_skill_successes: u64,
    pub verified_skill_failures: u64,
}
```

Expose these through:

```text
/learning-status
/learning-pending
```

Do not build a new complex TUI sheet until the behavior is stable.

## 18. Rollout strategy

### Stage A — shadow learning

- reviewer runs;
- candidates are persisted;
- no autonomous skill/memory writes are applied;
- inspect candidate quality and costs.

### Stage B — project learned-artifact writes

- trusted project only;
- background reviewer may create/patch Davinci-owned project learned artifacts;
- global writes remain staged;
- user/imported skills remain staged.

### Stage C — verified auto-promotion

- activate project skills after deterministic verification or repeated verified success;
- record success/failure usage;
- refine learned skills with read-before-write.

### Stage D — optional cross-repository learning

- promote a procedure to global scope only after explicit user action or evidence from multiple repositories;
- global automatic writes remain off by default.

## 19. Self-code-improvement boundary

"Self-improving" should mean Davinci learns better memory and procedures automatically.

It should **not** mean a background thread silently rewrites Davinci's own Rust code.

A later explicit `/self-improve` flow can safely use Davinci's existing graph engine:

```text
explicit user request
      |
      v
new git branch/worktree
      |
      v
research -> plan -> implement
      |
      v
deterministic verification
      |
      v
reviewer node
      |
      v
diff / PR for human merge
```

That is the appropriate place to let Davinci improve its own implementation.

## 20. Success criteria

The feature is ready to enable by default for project-scoped learning when all are true:

1. Existing skills still discover and `/skill:name` still expands unchanged.
2. Existing vector-memory records still load.
3. A successful graph-verified workflow can produce a candidate and later an active skill.
4. "Nothing ran" verification can never promote a skill.
5. A failed graph verification can never count as skill success.
6. An untrusted project cannot receive autonomous project skill writes.
7. Background review cannot mutate user/imported skills.
8. A background skill patch fails if it did not read the exact current hash first.
9. A new user turn can cancel/supersede review without waiting indefinitely.
10. Ollama/Qdrant outages do not break skill or memory use.
11. Tests use fixtures only and never require a live provider.
12. Learning can be fully disabled without affecting normal Davinci behavior.
