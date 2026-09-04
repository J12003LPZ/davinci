# Davinci Ecosystem Integration Design

## Status

Approved architecture derived from the September 4, 2026 whole-harness audit. This design turns Davinci's strong but partially independent subsystems into one bounded, token-efficient feedback ecosystem without adding an always-on orchestration model.

## Goal

Make Graph Engineering, Token Governor, Vector Memory, provider prompt caching, context pruning, self-improving learning, security scanning, verification, and observability reinforce one another so the system improves across runs while keeping the harness thin and letting modern models reason rather than being micromanaged by orchestration code.

## Product Philosophy

Davinci must remain an **AI-thin harness**.

The harness should decide only what software can decide cheaply and reliably:

- permissions and capability boundaries;
- hard cost, worker, timeout, and context limits;
- whether deterministic verification passed;
- what persisted artifact belongs to which run;
- whether a changed surface warrants a security gate;
- how much memory/skill context may be injected;
- stable cache-affinity identities;
- provenance, accounting, and lifecycle transitions.

The model should continue to decide the things models are good at:

- understanding the task;
- investigating code;
- planning changes;
- writing code;
- reviewing semantics;
- deciding which supplied context is relevant;
- adapting when evidence changes.

### Explicit non-goal: no always-on Director LLM

This design does **not** introduce a new model call whose job is to coordinate every other model call. Normal agent turns gain zero additional orchestration calls. Graph runs gain zero additional director calls. The existing graph classifier remains the only classification worker required by the graph topology.

## Current State

The existing normal-turn loop is already substantially integrated:

```text
prompt
  -> VectorMemory retrieval
  -> ephemeral context
  -> model reasoning
  -> TokenGovernor around tool execution
  -> pruning for old provider-visible tool outputs
  -> provider prompt caching
  -> deterministic verification evidence
  -> learning review
  -> memory / learned skill persistence
  -> later retrieval
```

The main remaining gaps are:

1. Graph workers can be governed, but their default role allowlists do not guarantee access to `retrieve_output` when the governor compresses output.
2. Graph workers are ephemeral (`--no-session`), so provider prompt-cache affinity is weaker than it needs to be.
3. Declarative memory can flow back into graph workers, but learned procedural skills are intentionally disabled with `--no-skills` and are not selectively reintroduced by the controller.
4. Graph cost/worker budgets and Token Governor telemetry exist independently rather than sharing one run-level resource view.
5. Security scanning is a strong native subsystem but is not a conditional verification gate in Graph Engineering.
6. Graph hardening still has known correctness risks that must be fixed before deeper integration.
7. Cross-subsystem behavior lacks end-to-end contract tests and repository CI.
8. Known dead source islands add search noise for coding agents.

## Target Architecture

```text
                          USER TASK
                              |
                              v
                     Normal Agent / Graph
                              |
                 +------------+------------+
                 |                         |
                 v                         v
        Bounded Context Packet       Resource Envelope
        - relevant memory            - hard cost cap
        - relevant skills            - deadline
        - repository facts           - worker cap
        - provenance                 - context soft cap
                 |                         |
                 +------------+------------+
                              |
                              v
                         MODEL WORK
                              |
                   +----------+----------+
                   |                     |
                   v                     v
             Token Governor        Provider Cache
                   |                     |
                   +----------+----------+
                              |
                              v
                     Context Pruning
                              |
                              v
                         VERIFICATION
                   +----------+----------+
                   |                     |
                   v                     v
              deterministic        conditional
              tests/lint/fmt        security gate
                   |                     |
                   +----------+----------+
                              |
                              v
                           REVIEW
                              |
                              v
                       LEARNING ENGINE
                   +----------+----------+
                   |                     |
                   v                     v
              Vector Memory       Learned Skills
                   |                     |
                   +----------+----------+
                              |
                              v
                          NEXT TASK
```

## Core Design Rule: Integration Through Small Contracts

Do not create a giant `EcosystemController`. Integration is expressed through small immutable data contracts and existing subsystem APIs.

New coordination code lives under:

```text
crates/davinci-coding-agent/src/native_extensions/ecosystem/
  mod.rs
  context.rs
  cache_affinity.rs
  resource.rs
  risk.rs
  telemetry.rs
```

Each module has one responsibility.

## 1. Bounded Context Packet

Graph workers remain isolated with `--no-session --no-extensions --no-skills`. The parent controller may explicitly supply a compact context packet.

### Interface

```rust
pub struct ContextPacketRequest<'a> {
    pub prompt: &'a str,
    pub role: Option<graph::Role>,
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

### Defaults

- Normal turn: preserve existing Vector Memory behavior; do not add a second context mechanism.
- Graph worker total ecosystem packet: **2,500 tokens maximum** by default.
- Memory portion: up to **1,200 tokens**, maximum **4 hits**.
- Skill portion: up to **1,000 tokens**, maximum **2 skill bodies**.
- Metadata/provenance: up to **300 tokens**.
- If no high-quality memory or skill match exists, inject nothing.

These are caps, not quotas. Empty context is valid and preferred over weak context.

### Progressive disclosure

Skill selection happens in two stages without an additional model call:

1. retrieve compact descriptors using existing lexical/vector retrieval;
2. load full `SKILL.md` only for the top role-compatible matches that fit the skill token cap.

The worker is still trusted to reason about whether the supplied material is useful. The harness does not turn skills into a procedural script the model must obey.

## 2. Graph + Token Governor Capability Closure

Any graph role allowed to call a tool that the Token Governor may compress must also be capable of lossless recovery.

Add a deterministic capability rule:

```rust
fn ensure_governor_recovery_tool(tools: &mut Vec<String>)
```

If `tools` contains a governor-compressible tool, add `retrieve_output` exactly once.

No user setting is required. Existing `workerExtraTools` remains supported. A future `roleExtraTools` configuration may be added for role-specific customization, but the recovery capability is automatic and cannot be accidentally omitted.

The worker prompt should mention recovery in one short sentence only when `retrieve_output` is present. Do not add a large governor instruction block.

## 3. Cache Affinity Independent of Session Persistence

Session persistence and provider prompt-cache affinity are separate concerns.

Extend `davinci_ai::StreamOptions` with:

```rust
pub cache_key: Option<String>
```

Provider request builders resolve:

```rust
cache_key.or(session_id)
```

Normal sessions keep current behavior through `session_id` fallback.

Graph workers stay `--no-session`, but the parent passes an internal stable affinity key derived from:

```text
repo_id + graph_definition_version + role + model + toolset_hash + system_contract_hash
```

The key intentionally excludes `run_id` so retries and later compatible graph runs can reuse provider routing/cache affinity. A changed graph contract, model, toolset, or system contract naturally generates a new key.

No conversation/session file is created for graph workers.

## 4. Resource Envelope: Hard Bounds, Not Micromanagement

Graph already owns cost, worker, retry, and deadline budgets. Do not replace them with an intelligent scheduler.

Add one read-only resource contract used for status and enforcement:

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

The graph scheduler may use these only for deterministic hard/soft bounds. It must not add a model call to decide fan-out.

Permitted adaptive behavior:

- do not start an optional ready node after a hard cost/deadline cap is exhausted;
- do not exceed configured parallelism;
- report resource pressure in status;
- preserve current explicit graph topology and role model choices.

Not permitted in this phase:

- heuristic model downgrades;
- dynamically rewriting the plan because cache hit rate is low;
- spawning extra agents because context is cheap;
- a separate AI resource manager.

## 5. Learned Skills Back Into Graph

Graph workers keep `--no-skills`. The controller, not the child discovery path, owns skill admission.

For each model-backed graph node:

1. build a role-scoped `ContextPacket`;
2. inject at most two applicable learned skills;
3. persist the selected skill names, versions, and hashes in task/run metadata;
4. after deterministic verification, attribute success/failure/neutral outcomes to those exact versions.

This closes:

```text
graph work -> verification -> learning -> learned skill -> future graph worker -> verification
```

User-authored/imported skills remain governed by existing ownership rules. This design does not make graph workers mutate skills.

## 6. Learning Token Discipline

Memory indexing remains cheap and automatic. The model-backed learning reviewer should run only when the settled turn has a meaningful durable signal.

Add deterministic `should_review_evidence` gating. Review when at least one is true:

- files were mutated;
- deterministic verification ran;
- a graph run completed/failed;
- an injected skill received a verifiable outcome;
- the user corrected or rejected prior work;
- repeated tool failures produced a failure lesson candidate;
- `/learn` explicitly requested review.

Do not spend a reviewer call on ordinary read-only explanation turns with no durable signal. Vector-memory indexing still occurs for those turns.

Do not immediately lower the existing learning token limit by guesswork. First add measurement. Acceptance target after a fixture benchmark:

- at least **40% lower median learning-review input tokens** than the ungated baseline;
- no more than **5% relative reduction** in accepted high-confidence learning artifacts on the fixture corpus.

## 7. Conditional Security Verification

Security scanning should not run after every edit.

Add a deterministic `ChangeRisk` classifier over graph-owned changed paths and diff metadata. No model call.

Risk surfaces include:

- authentication / authorization;
- permission policy;
- secret or credential handling;
- shell/process execution;
- network/HTTP/TLS;
- cryptography;
- dependency manifests / lockfiles;
- deserialization / protocol boundaries;
- filesystem traversal / archive extraction;
- extension/plugin loading.

`ChangeRisk::High` inserts a security verification step after deterministic tests and before review. `Low` and `None` do not.

The security result becomes part of the same verification bundle used by review and learning.

Configuration may support `securityVerification = off | risk | always`, default `risk`.

## 8. Graph Hardening Is Phase Zero

No deeper ecosystem integration ships before the already-approved Graph Engineering hardening invariants are implemented and tested, including:

- schema/validator consistency;
- non-empty evidence references;
- strict integer budget parsing;
- clearing stale task errors on successful retry;
- active wall-clock worker termination;
- graph-owned mutation provenance;
- complete review coverage for large diffs;
- explicit topology/replay compatibility as described by the hardening spec.

Integration builds on correctness rather than obscuring it.

## 9. Unified Verification Bundle

Introduce a small structured bundle rather than passing ad-hoc booleans between graph, security, and learning.

```rust
pub struct VerificationBundle {
    pub commands_ran: usize,
    pub commands_failed: usize,
    pub deterministic_passed: bool,
    pub security: SecurityVerification,
    pub changed_files: Vec<String>,
    pub graph_run_id: Option<String>,
}

pub enum SecurityVerification {
    NotRequired,
    Passed { scan_id: String },
    Failed { scan_id: String, blockers: usize },
    Unavailable { reason: String },
}
```

A run cannot be approved if deterministic verification failed. If security was required and failed, review receives a blocking verification result. If the scanner is unavailable, behavior follows configuration: `risk` defaults fail-open with an explicit warning; `always` defaults fail-closed.

## 10. Observability Without Prompt Cost

All ecosystem accounting is structured runtime data. It is never injected into prompts unless a model explicitly asks via an existing status tool.

Add per-run/session counters for:

- memory hits and injected tokens;
- skill descriptors considered;
- skills injected and their versions;
- context packet tokens and fingerprint;
- governor compressed bytes and recovery calls;
- pruning count;
- cache read/write tokens;
- graph cost and worker count;
- security gate triggered/result;
- learning review skipped/dispatched;
- learned artifact result.

Expose compact summaries through `/status`, `/graph-status`, and the existing TUI sheets. Avoid a new dashboard subsystem.

## 11. End-to-End Ecosystem Tests

Unit tests are insufficient for a 10/10 integration claim.

Add fixture-only integration tests proving these closed loops:

### Loop A: Governor recovery

```text
graph researcher -> oversized grep/bash output -> governor digest -> retrieve_output available -> worker can recover original
```

### Loop B: Cache affinity

```text
ephemeral graph worker -> stable cache key -> retry with same role/contract gets same key -> changed toolset/contract gets different key
```

### Loop C: Learning to Graph

```text
verified turn -> learned skill -> graph context selection -> worker metadata records skill version -> successful verification -> skill outcome increments
```

### Loop D: Memory to Graph

```text
settled turn -> vector index -> later graph context packet retrieves bounded relevant memory
```

### Loop E: Security gate

```text
writer changes auth/shell-sensitive file -> risk classifier triggers security verification -> failure prevents approval
```

### Loop F: Full circle

```text
graph run #1 -> verification -> learning artifact -> persisted memory/skill -> graph run #2 receives it -> run #2 verification succeeds -> outcome ledger updated
```

All tests must use local fixtures and never require provider/network access.

## 12. CI Gate

Add GitHub Actions only after the ecosystem tests exist. Required checks:

```text
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Add a named targeted ecosystem test step so integration failures are visible independently from the workspace suite.

The workflow uses the repository-pinned Rust 1.83.0 toolchain and no network-backed model fixtures.

## 13. Dead-Code Hygiene

After integration tests are green, remove or archive only source islands already confirmed as uncompiled/unreferenced.

The cleanup is deliberately last because deleting historical code before integration is proven creates review noise.

Rules:

- prove no workspace member/dependent references the file/crate;
- run workspace tests before and after each cleanup batch;
- do not mix dead-code removal with functional ecosystem commits;
- preserve relevant migration/history documentation under `docs/archive/` rather than leaving executable-looking dead Rust beside production code.

## Token and Model-Call Budget Invariants

These are release requirements:

1. **Normal foreground turn:** zero additional orchestration model calls.
2. **Graph run:** zero additional director/orchestrator model calls.
3. **Graph ecosystem packet:** default maximum 2,500 injected tokens per worker; empty is valid.
4. **Skills:** maximum 2 full skill bodies per graph worker by default.
5. **Memory:** maximum 4 graph memory hits by default.
6. **Governor recovery:** on demand only; no eager retrieval of compressed output.
7. **Security:** risk-triggered by default, not always-on.
8. **Learning:** asynchronous and meaningfully gated; no reviewer call on low-signal read-only turns.
9. **Observability:** structured data, not prompt text.
10. **Resource coordination:** deterministic; no management LLM.

## Success Definition: What “10/10” Means

Davinci may call ecosystem integration 10/10 only when all of the following are true:

- every native subsystem has a documented producer/consumer relationship or is intentionally standalone;
- Graph workers can losslessly recover every governor-compressed output they are allowed to generate;
- ephemeral workers receive stable provider cache affinity without session persistence;
- relevant learned skills and memory can return to future graph workers under hard token caps;
- skill/memory provenance is persisted and attributable to verification outcomes;
- risk-sensitive graph mutations pass through the security gate;
- the known Graph hardening correctness risks are fixed;
- closed-loop integration tests pass offline;
- CI runs fmt, clippy, workspace tests, and ecosystem tests on every change;
- no new always-on model call exists purely to coordinate the system;
- telemetry can explain, after a run, what context was injected, what was saved, what was verified, and what was learned;
- low-signal turns are not paying unnecessary learning-review token cost;
- known dead production-looking source islands are removed or clearly archived after functional work is complete.

## Rollout Order

1. Graph correctness hardening.
2. Governor recovery + cache affinity + bounded graph context packets.
3. Learned-skill feedback + conditional security verification.
4. Unified telemetry + end-to-end ecosystem tests + CI.
5. Dead-code hygiene and documentation finalization.

Each phase must be independently releasable and may not depend on unfinished later phases.