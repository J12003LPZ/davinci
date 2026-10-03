# Davinci Runtime Convergence — Claude Code Gap Integration Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:subagent-driven-development` (recommended) or `superpowers:executing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the areas where Claude Code is currently stronger than Davinci while preserving Davinci's existing strengths: deterministic Graph execution, token-governed tool output, bounded context injection, verifier-backed learning, security gating, parity compatibility, and offline/reproducible tests.

**Architecture:** Do not add a coordinator LLM and do not replace Graph. Add a deterministic shared runtime layer inside `davinci-agent` that provides lifecycle events, stable agent/run/task identities, a runtime registry, context/cache brokerage, messaging, and orchestration contracts. Existing normal turns, subagents, Graph workers, jobs, sessions, plugins, MCP, learning, security, and future workflows progressively become participants in that runtime through adapters and feature-gated migrations.

**Tech Stack:** Rust 1.83.0; current exact-pinned workspace dependencies; `serde`, `serde_json`, `uuid`, `rusqlite`, existing `davinci-*` crates; existing JSONL/session storage; existing Ratatui TUI; existing native/JS extension system; no new network dependency for core runtime behavior.

**Spec:** This document contains the architecture contract in §2 and the implementation plan in §7. Recommended repository destination: `docs/superpowers/plans/2026-09-05-davinci-runtime-convergence.md`.

## Global Constraints

- Preserve Rust `1.83.0`.
- Preserve exact dependency pinning (`=x.y.z`).
- Do not edit `vendor/davinci`; it remains reference-only.
- Do not alter parity-locked prompt strings in `compaction.rs`, `branch.rs`, or system-prompt constants.
- Preserve `~/.davinci` and legacy `~/.pi` compatibility.
- Preserve the current `AgentEvent` parity contract; new lifecycle semantics use a separate versioned `RuntimeEvent`.
- Tests must be offline and fixture-driven; no live provider, OAuth, browser, MCP, or web calls.
- Unit tests remain inline `#[cfg(test)] mod tests`; do not create external Rust test directories.
- Deny rules always win.
- No runtime subsystem may add a hidden coordinator-model call.
- Graph verification stays deterministic; test success remains based on command result/exit status, not model opinion.
- Graph writer exclusivity, replay fingerprints, mutation provenance, review coverage, security gating, and learning provenance remain mandatory.
- Token Governor lossless recovery remains available anywhere output is compacted.
- Every new subsystem needs a kill switch and a fallback to current behavior.
- Migrations must be additive first, then cut over only after equivalence tests pass.
- Each phase below must be independently releasable and revertible.

---

# 1. Current-State Baseline

Davinci already has strong foundations that this plan reuses instead of duplicating:

- `crates/davinci-agent/src/lib.rs`
  - `Agent`
  - `EventSink`
  - `PreToolHook` / `PostToolHook`
  - provider/model state
  - permissions
  - jobs/tool context
  - subagent runner
  - pruning/evidence
  - token estimates
  - abort propagation
- `crates/davinci-agent/src/events.rs`
  - parity-locked low-level `AgentEvent`
- `crates/davinci-agent/src/subagent.rs`
  - one-shot and fan-out read-only subagents
  - scoped tool inheritance
  - model inheritance
  - abort propagation
  - concurrency caps
- `crates/davinci-agent/src/jobs.rs`
  - persistent background shell processes
  - status/output/kill/stdin
  - session-lifetime cleanup
- `crates/davinci-agent/src/scheduler.rs`
  - parallel read lanes and mutation barriers
- `crates/davinci-agent/src/compaction.rs`
  - parity-compatible compaction
- `crates/davinci-agent/src/permission.rs`
  - tool classification and permission policy
- `crates/davinci-coding-agent/src/hooks.rs`
  - `preTool`, `postTool`, `stop`
- `crates/davinci-coding-agent/src/native_extensions/`
  - Graph
  - Token Governor
  - Vector Memory
  - Learning
  - Security
  - ecosystem telemetry
- `crates/davinci-session` / `davinci-session-sqlite`
  - JSONL session persistence and derived SQLite indexing
- `crates/davinci-protocol`, `davinci-client`, `davinci-server`
  - reusable transport/RPC foundations
- `crates/davinci-tui`
  - existing status instruments and session UI

The plan must extend these foundations rather than create competing state machines.

---

# 2. Architecture Contract

## 2.1 Target runtime shape

```text
                                  ┌──────────────────────────┐
                                  │     Davinci Runtime      │
                                  │ deterministic, local     │
                                  └────────────┬─────────────┘
                                               │
             ┌─────────────────────────────────┼────────────────────────────────┐
             │                                 │                                │
      Runtime Event Bus                 Runtime Registry                  Context Broker
             │                                 │                                │
     lifecycle + decisions          agents/runs/tasks/state        budget + provenance + cache
             │                                 │                                │
      ┌──────┼─────────┐             ┌─────────┼─────────┐            ┌─────────┼─────────┐
      │      │         │             │         │         │            │         │         │
    Main   Graph    Workflow      Subagent   Team    Background     Memory    Skills   Governor
    Agent  Worker    Worker         Agent    Mate      Agent
      │      │         │             │         │         │
      └──────┴─────────┴─────────────┴─────────┴─────────┘
                              │
                        Tool / MCP / Plugin
                              │
                Permissions → Execution → Results
                              │
                 Verification / Security / Learning
                              │
                         Outcome Ledger
```

## 2.2 Non-negotiable runtime invariants

1. **No coordinator model**
   - Runtime routing, registration, event delivery, cache-key derivation, context budgeting, task state, worktree ownership, and retry/stop decisions are deterministic local computations.

2. **Graph remains high-assurance execution**
   - The general workflow engine does not replace Graph for mutation-heavy coding work.
   - Graph keeps its explicit DAG and verifier/reviewer/security contracts.

3. **Normal agent remains lightweight**
   - A normal prompt must not pay workflow/team overhead unless those features are used.

4. **One runtime identity model**
   - Main agents, Graph workers, subagents, workflow workers, teammates, and background sessions use the same `AgentId`, `RunId`, `TaskId`, lifecycle states, and event envelope.

5. **One context contract**
   - Every model call can request context from the broker.
   - Context sources are adapters; the broker does not depend directly on `native_extensions`.

6. **One cache identity contract**
   - Compatible retries/workers preserve stable prompt-prefix affinity.
   - Tool schema, system prompt, model/provider, instructions, context-source versions, and permission-visible tools participate in the key.

7. **One task/messaging contract**
   - Teams and persistent workers communicate through runtime-owned task and message ledgers, not ad-hoc process stdout.

8. **One interruption contract**
   - Parent cancellation propagates to descendants unless a child was explicitly detached.
   - Detached agents still obey session/run ownership and cleanup policy.

9. **Session compatibility**
   - Existing session JSONL stays readable.
   - New runtime metadata is initially written to sidecar/runtime stores so old sessions are not corrupted.

10. **Observability without prompt pollution**
    - Telemetry and runtime state are not injected into model context unless explicitly requested by a tool or context source.

---

# 3. Mapping: Claude Code Advantage → Davinci Change

| Claude Code advantage | Davinci implementation target |
|---|---|
| Rich lifecycle hooks | Versioned `RuntimeEventBus` + hook adapter |
| First-class subagents | Persistent `AgentDescriptor` + profiles + resume |
| Agent teams | Shared `TaskRegistry` + messaging + teammate lifecycle |
| Agent-to-agent messaging | `AgentMailbox` and `SendMessage`-style tool |
| Cross-session coordination | Runtime registry persistence + protocol controls |
| Dynamic workflows | Deterministic general `WorkflowEngine` |
| Multiple orchestration modes | normal / subagent / team / workflow / Graph |
| Background agents | persistent runtime-owned agent processes |
| Shared task ownership | task states, dependencies, assignment, wakeups |
| Worktree isolation | runtime `WorktreeManager` with ownership locks |
| Prompt-cache optimization | universal `CacheIdentity` |
| Cache diagnostics | reason-coded cache-affinity diff |
| Compaction maturity | lifecycle events + resumable compaction state |
| Context inspection | `/context` and broker provenance report |
| Skill context-cost analysis | `/skill-doctor` equivalent |
| Universal context budgeting | `ContextBroker` |
| Persistent agent memory | memory source scoped by `AgentId`/profile/repo |
| Resume fidelity | runtime sidecar replay + tool/result provenance |
| Fine-grained permissions | parameter-aware rule matcher |
| Sandbox hardening | shell policy normalization + worktree/path boundary |
| Managed org policy | layered policy source + immutable managed tier |
| Remote control | protocol-level runtime controls |
| IDE integration | SDK/RPC runtime-state API, not TUI-specific logic |
| Plugin maturity | versioned plugin capabilities + namespaces |
| MCP UX | MCP resources represented in runtime capability registry |
| Skill maturity | skill usage/cost/provenance diagnostics |
| Custom agent ecosystem | `.davinci/agents/*.md` profiles |
| Model-aware worker selection | explicit model policy per agent/workflow role |
| Structured outputs | reusable schema-validated worker result contract |
| Resumable background workers | persisted agent/run metadata + wakeups |
| Interrupt propagation | runtime cancellation tree |
| Concurrent sessions | session/run ownership and resource locks |
| Session organization | registry indexes, group/archive metadata |
| Production telemetry | runtime metrics and reason codes |
| Runtime object model | shared Agent/Run/Task/Workflow IDs and state |
| Output virtualization | Governor adapter in all worker execution paths |
| Enterprise auth/policy | policy layer; retain existing provider auth |
| Network/proxy hardening | centralized transport configuration and diagnostics |
| Multi-platform consistency | runtime API reused by TUI/RPC/SDK/server |

---

# 4. File/Module Structure

Create the runtime primitives in `davinci-agent`, because `davinci-coding-agent`, SDK, RPC, server, Graph adapters, and the TUI can all consume `davinci-agent` without creating an inverse dependency.

```text
crates/davinci-agent/src/
├── runtime/
│   ├── mod.rs
│   ├── ids.rs
│   ├── events.rs
│   ├── bus.rs
│   ├── registry.rs
│   ├── cancellation.rs
│   ├── tasks.rs
│   ├── mailbox.rs
│   ├── context.rs
│   ├── cache.rs
│   ├── capabilities.rs
│   ├── worktree.rs
│   └── workflow/
│       ├── mod.rs
│       ├── spec.rs
│       ├── validate.rs
│       ├── state.rs
│       └── executor.rs
├── subagent.rs                 # evolve existing path; preserve old tool contract
├── permission.rs               # evolve matcher and policy
├── compaction.rs               # emit runtime lifecycle, do not rewrite prompts
├── turn.rs                     # runtime event emission points
├── jobs.rs                     # adapt jobs into runtime registry
└── lib.rs                      # Agent owns RuntimeHandle

crates/davinci-coding-agent/src/
├── runtime_host.rs             # host adapters: native extensions, hooks, persistence
├── hooks.rs                    # Hook v2 compatibility adapter
├── agent_profiles.rs           # .davinci/agents discovery/loading
├── policy.rs                   # user/project/managed policy layering
├── extension_host.rs           # runtime capability registration
├── mcp_host.rs                 # runtime capability registration
├── davinci_interactive.rs      # new commands/views wiring
├── rpc.rs                      # runtime control methods
├── sdk.rs                      # runtime control API
└── native_extensions/
    ├── ecosystem/              # context/cache/runtime adapters
    ├── graph/                  # register workers and emit lifecycle
    ├── learning/               # runtime outcome subscriber
    ├── token_governor.rs       # universal output adapter
    ├── vector_memory.rs        # ContextSource adapter
    └── security_scan.rs        # verification subscriber

crates/davinci-session/src/
├── runtime_log.rs              # append-only sidecar event persistence
└── lib.rs

crates/davinci-session-sqlite/src/
├── runtime_index.rs            # derived runtime/agent/task index
└── lib.rs

crates/davinci-protocol/src/
├── runtime.rs                  # read/control DTOs
└── lib.rs

crates/davinci-tui/src/davinci/
├── views/agents.rs
├── views/workflows.rs
├── views/context.rs
└── views/runtime.rs
```

Avoid a new workspace crate until these interfaces prove stable. A separate `davinci-runtime` crate can be extracted later without changing public behavior.

---

# 5. Core Interfaces

These interfaces are the contracts every later phase must use.

## 5.1 IDs and ownership

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentId(Uuid);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RunId(Uuid);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TaskId(Uuid);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct WorkflowId(Uuid);
```

Use UUIDv7 for runtime-created IDs so logs remain time-sortable.

## 5.2 Agent runtime model

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentKind {
    Main,
    Subagent,
    GraphWorker,
    WorkflowWorker,
    Teammate,
    Background,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AgentState {
    Starting,
    Running,
    Waiting,
    Idle,
    Stopping,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRecord {
    pub id: AgentId,
    pub run_id: RunId,
    pub parent: Option<AgentId>,
    pub kind: AgentKind,
    pub name: String,
    pub provider: String,
    pub model_id: String,
    pub cwd: PathBuf,
    pub state: AgentState,
    pub task_id: Option<TaskId>,
    pub worktree: Option<PathBuf>,
    pub started_ms: i64,
    pub updated_ms: i64,
}
```

## 5.3 Runtime events

Keep `AgentEvent` unchanged.

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RuntimeEventEnvelope {
    pub schema_version: u16,       // start at 1
    pub event_id: Uuid,
    pub sequence: u64,
    pub timestamp_ms: i64,
    pub run_id: RunId,
    pub session_id: Option<String>,
    pub agent_id: Option<AgentId>,
    pub parent_agent_id: Option<AgentId>,
    pub payload: RuntimeEvent,
}
```

Minimum event set:

```rust
pub enum RuntimeEvent {
    SessionStarted { resumed: bool },
    SessionEnded { reason: String },
    TurnStarted,
    TurnEnded { success: bool },
    UserPromptSubmitted,
    InstructionsLoaded { paths: Vec<PathBuf> },
    PreToolUse { call_id: String, tool: String, args: Value },
    PermissionRequested { call_id: String, tool: String },
    PermissionDenied { call_id: String, reason: String },
    PostToolUse { call_id: String, tool: String, is_error: bool },
    PostToolBatch { calls: usize, failures: usize },
    AgentStarted { record: AgentRecord },
    AgentStateChanged { from: AgentState, to: AgentState },
    AgentMessageQueued { to: AgentId, message_id: Uuid },
    AgentMessageDelivered { to: AgentId, message_id: Uuid },
    TaskCreated { task_id: TaskId },
    TaskAssigned { task_id: TaskId, agent_id: AgentId },
    TaskCompleted { task_id: TaskId, success: bool },
    WorkflowStarted { workflow_id: WorkflowId },
    WorkflowPhaseChanged { workflow_id: WorkflowId, phase: String },
    WorkflowEnded { workflow_id: WorkflowId, success: bool },
    PreCompact { estimated_tokens: u64 },
    PostCompact { before_tokens: u64, after_tokens: u64 },
    PreModelSwitch { provider: String, model_id: String },
    PostModelSwitch { provider: String, model_id: String },
    WorktreeCreated { path: PathBuf },
    WorktreeRemoved { path: PathBuf },
    ContextBuilt { estimated_tokens: u64, cache_key: Option<String> },
    CacheAffinity { key: String, reason: String },
    RuntimeWarning { code: String, message: String },
}
```

## 5.4 Runtime bus

```rust
pub enum RuntimeDecision {
    Continue,
    Deny { reason: String },
}

pub trait RuntimeSubscriber: Send + Sync {
    fn on_event(&self, event: &RuntimeEventEnvelope) -> RuntimeDecision;
}

#[derive(Clone, Default)]
pub struct RuntimeBus {
    inner: Arc<RuntimeBusInner>,
}

impl RuntimeBus {
    pub fn emit_observe(&self, event: RuntimeEventEnvelope);
    pub fn emit_decision(&self, event: RuntimeEventEnvelope) -> Result<(), String>;
}
```

Rules:
- only documented decision events can block;
- observer failures are logged and fail open;
- security/permission decision subscribers can fail closed;
- event ordering for one agent is monotonic by `sequence`.

## 5.5 Context broker

```rust
#[derive(Debug, Clone)]
pub struct ContextRequest {
    pub run_id: RunId,
    pub agent_id: AgentId,
    pub goal: String,
    pub provider: String,
    pub model_id: String,
    pub tools: Vec<String>,
    pub max_tokens: u64,
    pub kind: AgentKind,
}

#[derive(Debug, Clone)]
pub struct ContextItem {
    pub source: String,
    pub content: String,
    pub estimated_tokens: u64,
    pub priority: i32,
    pub stable_for_cache: bool,
    pub provenance: Value,
}

pub trait ContextSource: Send + Sync {
    fn collect(&self, request: &ContextRequest) -> Vec<ContextItem>;
}

#[derive(Debug, Clone)]
pub struct ContextPacket {
    pub items: Vec<ContextItem>,
    pub estimated_tokens: u64,
    pub cache_key: String,
}
```

The broker sorts deterministically by priority, then source, then stable provenance hash. It rejects/omits items over budget rather than silently exceeding the request.

## 5.6 Task and mailbox contracts

```rust
pub enum TaskState {
    Pending,
    Ready,
    Assigned,
    Running,
    Blocked,
    Completed,
    Failed,
    Cancelled,
}

pub struct RuntimeTask {
    pub id: TaskId,
    pub run_id: RunId,
    pub title: String,
    pub description: String,
    pub state: TaskState,
    pub owner: Option<AgentId>,
    pub depends_on: Vec<TaskId>,
    pub created_ms: i64,
    pub updated_ms: i64,
}

pub struct AgentMessage {
    pub id: Uuid,
    pub run_id: RunId,
    pub from: AgentId,
    pub to: AgentId,
    pub body: String,
    pub created_ms: i64,
    pub delivered_ms: Option<i64>,
}
```

## 5.7 Workflow contract

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowSpec {
    pub schema_version: u16,
    pub name: String,
    pub phases: Vec<WorkflowPhaseSpec>,
    pub max_parallel_agents: usize,
    pub max_total_agents: usize,
    pub max_cost_usd: Option<f64>,
    pub deadline_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowPhaseSpec {
    pub id: String,
    pub depends_on: Vec<String>,
    pub workers: Vec<WorkflowWorkerSpec>,
    pub join: WorkflowJoin,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum WorkflowJoin {
    All,
    Any,
    Quorum { required: usize },
}
```

Do not permit arbitrary shell code as the workflow controller in v1. The workflow engine owns intermediate artifacts in Rust so outputs do not accumulate in the main model context.

---

# 6. Release Strategy

## Release A — Runtime foundation
Phases 0–3. No user-visible behavior change by default.

## Release B — First-class agents
Phases 4–6. Persistent agents, tasks, messages, teams, and worktree isolation behind flags.

## Release C — General workflows
Phases 7–8. Workflow engine and full compaction/resume lifecycle.

## Release D — Security/platform convergence
Phases 9–12. Permissions v2, plugins/MCP runtime integration, remote control, managed policy.

## Release E — Product hardening
Phases 13–15. Observability, evals, battle-hardening, performance and rollout.

Each release has a `DAVINCI_RUNTIME_*` master kill switch and subsystem-specific kill switches.

---

# 7. Detailed Implementation Plan

## Phase 0 — Freeze Baseline and Add Architectural Regression Gates

### Task 0.1: Capture current ecosystem invariants

**Files**
- Modify: `.github/workflows/ci.yml`
- Modify: `crates/davinci-coding-agent/src/native_extensions/ecosystem/mod.rs`
- Modify: `docs/ecosystem.md`

**Produces**
- A regression gate proving runtime work does not break current Graph/Memory/Governor/Learning/Security behavior.

- [ ] Add an offline test named `runtime_migration_preserves_ecosystem_baseline` that invokes the existing ecosystem fixtures and asserts:
  - zero added coordinator model calls;
  - Graph context remains `<= 2500` estimated tokens;
  - Governor recovery stays byte-for-byte;
  - security failure blocks Graph approval;
  - learning outcome attribution still uses exact `(name, version, content_hash)`;
  - cache-affinity retry key stays stable.
- [ ] Run:
  ```bash
  cargo test -p davinci-coding-agent runtime_migration_preserves_ecosystem_baseline -- --nocapture
  ```
  Expected: PASS.
- [ ] Add `Runtime migration baseline` to CI immediately after current ecosystem invariants.
- [ ] Run:
  ```bash
  cargo fmt --check
  cargo clippy --workspace --all-targets -- -D warnings
  cargo test --workspace
  ```
- [ ] Commit:
  ```bash
  git add .github/workflows/ci.yml crates/davinci-coding-agent/src/native_extensions/ecosystem/mod.rs docs/ecosystem.md
  git commit -m "test: freeze ecosystem baseline before runtime migration"
  ```

---

## Phase 1 — Versioned Runtime Event Bus and Hook Lifecycle v2

### Task 1.1: Add runtime IDs and event envelope

**Files**
- Create: `crates/davinci-agent/src/runtime/mod.rs`
- Create: `crates/davinci-agent/src/runtime/ids.rs`
- Create: `crates/davinci-agent/src/runtime/events.rs`
- Modify: `crates/davinci-agent/src/lib.rs`

**Interfaces**
- Produces: `AgentId`, `RunId`, `TaskId`, `WorkflowId`, `RuntimeEventEnvelope`, `RuntimeEvent`.

- [ ] Write inline tests proving UUIDv7 IDs serialize/deserialize and remain distinct.
- [ ] Write `RuntimeEventEnvelope` round-trip tests for `PreToolUse`, `PostCompact`, and `AgentStateChanged`.
- [ ] Implement the exact interfaces in §5.1 and §5.3.
- [ ] Export them from `runtime/mod.rs` and `davinci-agent/src/lib.rs`.
- [ ] Run:
  ```bash
  cargo test -p davinci-agent runtime::ids
  cargo test -p davinci-agent runtime::events
  ```
- [ ] Commit:
  ```bash
  git add crates/davinci-agent/src/runtime crates/davinci-agent/src/lib.rs
  git commit -m "feat(runtime): add versioned runtime identities and events"
  ```

### Task 1.2: Implement deterministic RuntimeBus

**Files**
- Create: `crates/davinci-agent/src/runtime/bus.rs`
- Modify: `crates/davinci-agent/src/runtime/mod.rs`

**Interfaces**
- Consumes: `RuntimeEventEnvelope`.
- Produces: `RuntimeBus`, `RuntimeSubscriber`, `RuntimeDecision`.

- [ ] Write a test registering three subscribers and assert observe events arrive in registration order.
- [ ] Write a test where subscriber two returns `Deny`; assert subscriber three is not called for decision events.
- [ ] Write a test where an observe subscriber panics; catch the panic at the bus boundary and emit/log a warning without failing the caller.
- [ ] Implement `emit_observe` and `emit_decision`.
- [ ] Reject blocking decisions for event kinds not in the explicit decision allowlist:
  `UserPromptSubmitted`, `PreToolUse`, `PermissionRequested`, `PreModelSwitch`, `TaskCompleted`.
- [ ] Run:
  ```bash
  cargo test -p davinci-agent runtime::bus
  ```
- [ ] Commit:
  ```bash
  git add crates/davinci-agent/src/runtime
  git commit -m "feat(runtime): add deterministic lifecycle event bus"
  ```

### Task 1.3: Adapt low-level AgentEvent into RuntimeEvent

**Files**
- Modify: `crates/davinci-agent/src/lib.rs`
- Modify: `crates/davinci-agent/src/turn.rs`
- Keep unchanged: `crates/davinci-agent/src/events.rs`

**Interfaces**
- `Agent` gains:
  ```rust
  pub runtime: Option<RuntimeHandle>
  ```
- `RuntimeHandle` owns `RunId`, `AgentId`, sequence counter, bus, cancellation token.

- [ ] Write a test proving existing serialized `AgentEvent` JSON is byte-for-byte unchanged.
- [ ] Write a test proving one tool call emits:
  `PreToolUse -> ToolExecutionStart(existing) -> PostToolUse`.
- [ ] Emit runtime lifecycle events at the same execution boundaries currently used by `turn.rs`; do not derive them later from logs.
- [ ] Ensure `runtime == None` has zero behavioral difference.
- [ ] Run:
  ```bash
  cargo test -p davinci-agent events
  cargo test -p davinci-agent turn
  ```
- [ ] Commit:
  ```bash
  git add crates/davinci-agent/src/lib.rs crates/davinci-agent/src/turn.rs
  git commit -m "feat(runtime): emit lifecycle events from agent loop"
  ```

### Task 1.4: Replace hook-specific lifecycle plumbing with a compatibility adapter

**Files**
- Modify: `crates/davinci-coding-agent/src/hooks.rs`
- Create: `crates/davinci-coding-agent/src/runtime_host.rs`
- Modify: `crates/davinci-coding-agent/src/main.rs`

**Required compatibility**
- Existing `hooks.json` with `preTool`, `postTool`, `stop` keeps working unchanged.

**New hook names**
- `sessionStart`
- `userPromptSubmit`
- `preTool`
- `permissionRequest`
- `postTool`
- `postToolFailure`
- `postToolBatch`
- `subagentStart`
- `subagentStop`
- `taskCreated`
- `taskCompleted`
- `preCompact`
- `postCompact`
- `preModelSwitch`
- `postModelSwitch`
- `sessionEnd`

- [ ] Extend `HooksFile` with optional vectors for the new events while preserving defaults.
- [ ] Add one mapping function:
  ```rust
  fn hook_kind_for(event: &RuntimeEvent) -> Option<&'static str>
  ```
- [ ] Keep existing stdin JSON fields and add:
  `schemaVersion`, `runId`, `agentId`, `sessionId`, `event`.
- [ ] For old `preTool`, preserve non-zero-exit blocking semantics.
- [ ] For observe-only hooks, ignore non-zero status for agent control but record the failure.
- [ ] Add `DAVINCI_RUNTIME_HOOKS_V2=0` kill switch.
- [ ] Add tests for old config compatibility, new event dispatch, trusted-project loading, timeout, and decision blocking.
- [ ] Commit:
  ```bash
  git add crates/davinci-coding-agent/src/hooks.rs crates/davinci-coding-agent/src/runtime_host.rs crates/davinci-coding-agent/src/main.rs
  git commit -m "feat(hooks): bridge rich lifecycle hooks through runtime events"
  ```

**Phase 1 acceptance gate**
```bash
cargo test -p davinci-agent runtime
cargo test -p davinci-coding-agent hooks
cargo test --workspace
```

---

## Phase 2 — Runtime Registry, Cancellation Tree, Agent State

### Task 2.1: Add AgentRecord and in-memory registry

**Files**
- Create: `crates/davinci-agent/src/runtime/registry.rs`
- Modify: `crates/davinci-agent/src/runtime/mod.rs`

**Interfaces**
- Produces:
  ```rust
  pub struct RuntimeRegistry;
  pub fn register_agent(&self, record: AgentRecord) -> Result<(), RegistryError>;
  pub fn transition(&self, id: AgentId, to: AgentState) -> Result<(), RegistryError>;
  pub fn snapshot(&self) -> Vec<AgentRecord>;
  ```

- [ ] Encode allowed transitions explicitly:
  - `Starting -> Running|Failed|Cancelled`
  - `Running -> Waiting|Idle|Stopping|Completed|Failed|Cancelled`
  - `Waiting -> Running|Stopping|Failed|Cancelled`
  - `Idle -> Running|Stopping|Completed|Cancelled`
  - `Stopping -> Completed|Failed|Cancelled`
  - terminal states cannot transition.
- [ ] Write tests rejecting terminal resurrection and duplicate IDs.
- [ ] Emit `AgentStarted` and `AgentStateChanged`.
- [ ] Commit:
  ```bash
  git add crates/davinci-agent/src/runtime/registry.rs crates/davinci-agent/src/runtime/mod.rs
  git commit -m "feat(runtime): track first-class agent lifecycle state"
  ```

### Task 2.2: Add hierarchical cancellation

**Files**
- Create: `crates/davinci-agent/src/runtime/cancellation.rs`
- Modify: `crates/davinci-agent/src/lib.rs`
- Modify: `crates/davinci-agent/src/subagent.rs`
- Modify: `crates/davinci-agent/src/jobs.rs`

**Interface**
```rust
#[derive(Clone)]
pub struct CancellationToken {
    inner: Arc<AtomicBool>,
    children: Arc<Mutex<Vec<Weak<CancellationInner>>>>,
}
```

- [ ] Add tests proving parent cancellation reaches normal subagents, fan-out workers, and jobs.
- [ ] Add a detached-child test; detached means it does not inherit parent-turn cancellation but still inherits session/run cancellation.
- [ ] Adapt existing `abort_signal` to read from the runtime token while preserving the public field until migration completes.
- [ ] Make `JobBook::kill_all()` execute on run/session cancellation.
- [ ] Commit:
  ```bash
  git add crates/davinci-agent/src/runtime/cancellation.rs crates/davinci-agent/src/lib.rs crates/davinci-agent/src/subagent.rs crates/davinci-agent/src/jobs.rs
  git commit -m "feat(runtime): unify cancellation across agents and jobs"
  ```

### Task 2.3: Register Graph workers without changing Graph topology

**Files**
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/worker.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/controller.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/graph/mod.rs`

- [ ] When a Graph worker starts, register `AgentKind::GraphWorker` with parent `AgentId`.
- [ ] Put Graph run ID into runtime metadata while preserving existing Graph run ID/artifact paths.
- [ ] Translate worker exit/timeout/abort into runtime state.
- [ ] Do not route Graph verification through runtime decisions; Graph's current verifier remains authoritative.
- [ ] Add test: Graph dry-run creates runtime worker records but adds zero model calls.
- [ ] Commit:
  ```bash
  git add crates/davinci-coding-agent/src/native_extensions/graph
  git commit -m "feat(graph): register workers in shared runtime"
  ```

---

## Phase 3 — Universal Context Broker and Cache Coordinator

### Task 3.1: Implement ContextBroker with source adapters

**Files**
- Create: `crates/davinci-agent/src/runtime/context.rs`
- Modify: `crates/davinci-agent/src/lib.rs`

**Rules**
- deterministic ordering;
- strict token cap;
- no direct dependency on Vector Memory/Learning;
- sources register through trait objects;
- every item carries provenance;
- untrusted retrieved text is wrapped as data, never instructions.

- [ ] Add tests for priority ordering, deterministic ties, exact cap behavior, and empty-source fallback.
- [ ] Add:
  ```rust
  pub fn register_context_source(&mut self, source: Arc<dyn ContextSource>);
  pub fn build_context(&self, request: &ContextRequest) -> ContextPacket;
  ```
- [ ] Preserve current ephemeral-context behavior as the default sink.
- [ ] Commit:
  ```bash
  git add crates/davinci-agent/src/runtime/context.rs crates/davinci-agent/src/lib.rs
  git commit -m "feat(context): add universal bounded context broker"
  ```

### Task 3.2: Adapt Vector Memory and Learning to ContextSource

**Files**
- Modify: `crates/davinci-coding-agent/src/native_extensions/vector_memory.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/learning/mod.rs`
- Modify: `crates/davinci-coding-agent/src/native_extensions/ecosystem/mod.rs`
- Modify: `crates/davinci-coding-agent/src/runtime_host.rs`

**Compatibility**
- Preserve Graph's existing max:
  - total `<= 2500`
  - memory `<= 1200`
  - skills `<= 1000`
  - memory hits `<= 4`
  - skills `<= 2`

- [ ] Implement `MemoryContextSource`.
- [ ] Implement `SkillContextSource`.
- [ ] Make normal turns use a separate default budget so Graph's current contract is unchanged.
- [ ] Keep `PI_GRAPH_SUPPRESS_MEMORY_INJECT=1` behavior until Graph has fully switched to the broker.
- [ ] Add a test proving no duplicate memory appears in Graph workers.
- [ ] Commit:
  ```bash
  git add crates/davinci-coding-agent/src/native_extensions/vector_memory.rs crates/davinci-coding-agent/src/native_extensions/learning crates/davinci-coding-agent/src/native_extensions/ecosystem crates/davinci-coding-agent/src/runtime_host.rs
  git commit -m "feat(context): feed memory and skills through shared broker"
  ```

### Task 3.3: Add universal CacheIdentity

**Files**
- Create: `crates/davinci-agent/src/runtime/cache.rs`
- Modify: `crates/davinci-ai` stream options at the existing `cache_key` integration point
- Modify: `crates/davinci-coding-agent/src/native_extensions/ecosystem/`
- Modify: Graph worker cache-key adapter

**Cache key inputs**
```text
provider
model_id
stable system prompt hash
visible tool schema hash
permission-visible tool set hash
context packet stable-item hashes
agent profile version/hash
workflow/graph contract hash
role
```

**Do not include**
- random process ID
- wall-clock time
- session ID when compatible requests should share a prefix
- transient runtime state
- current task status

- [ ] Write reason-coded diff:
  ```rust
  pub enum CacheMissReason {
      ProviderChanged,
      ModelChanged,
      SystemPromptChanged,
      ToolSchemaChanged,
      PermissionSurfaceChanged,
      ContextChanged,
      AgentProfileChanged,
      ContractChanged,
      Unknown,
  }
  ```
- [ ] Preserve existing `derive_worker_cache_key` output behavior through an adapter until cutover.
- [ ] Add test proving retry stability and intentional invalidation.
- [ ] Add live telemetry fields only; do not claim provider cache-hit improvements from offline tests.
- [ ] Commit:
  ```bash
  git add crates/davinci-agent/src/runtime/cache.rs crates/davinci-ai crates/davinci-coding-agent/src/native_extensions
  git commit -m "feat(cache): coordinate prompt affinity across agent types"
  ```

### Task 3.4: Make Token Governor universal for worker outputs

**Files**
- Modify: `crates/davinci-coding-agent/src/native_extensions/token_governor.rs`
- Modify: `crates/davinci-coding-agent/src/runtime_host.rs`
- Modify: subagent/Graph/workflow runner adapters

- [ ] Move compression decision to one host adapter invoked after successful tool execution for every runtime-managed agent.
- [ ] Keep `memory_search`, `retrieve_output`, and error outputs exempt exactly as today.
- [ ] Ensure every worker with compactible tools has `retrieve_output`.
- [ ] Add tests for normal agent, subagent, Graph worker, and workflow worker recoverability.
- [ ] Commit:
  ```bash
  git add crates/davinci-coding-agent/src/native_extensions/token_governor.rs crates/davinci-coding-agent/src/runtime_host.rs
  git commit -m "feat(governor): virtualize output across all runtime agents"
  ```

---

## Phase 4 — First-Class Persistent Subagents and Agent Profiles

### Task 4.1: Add `.davinci/agents/*.md` profile format

**Files**
- Create: `crates/davinci-coding-agent/src/agent_profiles.rs`
- Modify: `crates/davinci-coding-agent/src/main.rs`
- Modify: `crates/davinci-coding-agent/src/davinci_interactive.rs`

**Profile format**
```markdown
---
name: code-improver
description: Reviews code for correctness and maintainability.
model: inherit
permission_mode: read-only
tools: [read, grep, find, ls]
memory_scope: project
max_context_tokens: 20000
---

You are a code improvement specialist...
```

**Locations**
- project: `.davinci/agents/*.md` and legacy `.pi/agents/*.md`
- user: `~/.davinci/agent/agents/*.md` and legacy path

- [ ] Reuse existing frontmatter parsing patterns from skills instead of adding YAML dependency.
- [ ] Require unique `name`; project trusted profile overrides user profile with same name.
- [ ] Validate tools against current registry.
- [ ] Add `/agents` list/status output.
- [ ] Add tests for project trust, duplicate precedence, invalid model, invalid tools, and legacy path.
- [ ] Commit:
  ```bash
  git add crates/davinci-coding-agent/src/agent_profiles.rs crates/davinci-coding-agent/src/main.rs crates/davinci-coding-agent/src/davinci_interactive.rs
  git commit -m "feat(agents): add reusable custom agent profiles"
  ```

### Task 4.2: Evolve existing `agent` tool without breaking one-shot calls

**Files**
- Modify: `crates/davinci-agent/src/subagent.rs`
- Modify: `crates/davinci-agent/src/tools.rs`
- Modify: host runner in `davinci-coding-agent`

**Backward compatibility**
Existing:
```json
{"prompt":"...","tools":["read"],"description":"..."}
```
continues as one-shot mode.

New optional fields:
```json
{
  "prompt": "...",
  "agent": "code-improver",
  "mode": "oneshot|background|teammate",
  "model": "provider/model",
  "isolation": "shared|worktree",
  "name": "reviewer-a"
}
```

- [ ] Add `AgentSpawnMode`.
- [ ] Reject mutation-capable profile use when the parent permission mode cannot grant it.
- [ ] Preserve current fan-out limits for one-shot mode.
- [ ] Background/teammate modes return `agent_id` immediately instead of waiting for final text.
- [ ] Register every spawned agent in `RuntimeRegistry`.
- [ ] Commit:
  ```bash
  git add crates/davinci-agent/src/subagent.rs crates/davinci-agent/src/tools.rs crates/davinci-coding-agent
  git commit -m "feat(agents): support persistent and profile-backed subagents"
  ```

### Task 4.3: Add agent-scoped memory

**Files**
- Modify: `vector_memory.rs`
- Modify: `runtime/context.rs`
- Modify: `agent_profiles.rs`

**Scopes**
- `none`
- `project`
- `agent_project`
- `agent_global`

- [ ] Include `agent_profile_name` in memory metadata, not raw runtime UUID, for reusable agent memory.
- [ ] Retrieval for `agent_project` filters by repository + profile.
- [ ] Main-agent memory behavior remains unchanged.
- [ ] Add tests proving profile A cannot retrieve profile B private agent memory.
- [ ] Commit:
  ```bash
  git add crates/davinci-coding-agent/src/native_extensions/vector_memory.rs crates/davinci-agent/src/runtime/context.rs crates/davinci-coding-agent/src/agent_profiles.rs
  git commit -m "feat(memory): add scoped persistent agent memory"
  ```

---

## Phase 5 — Shared Tasks, Messaging, Teams, Wakeups

### Task 5.1: Add TaskRegistry

**Files**
- Create: `crates/davinci-agent/src/runtime/tasks.rs`
- Modify: `runtime/mod.rs`

- [ ] Implement dependency validation; reject self-dependency and cycles.
- [ ] `Ready` is derived only when all dependencies are `Completed`.
- [ ] A failed dependency moves dependents to `Blocked`.
- [ ] Ownership change emits `TaskAssigned`.
- [ ] Completion emits `TaskCompleted` through a decision event so security/verification policy may refuse invalid completion.
- [ ] Add full state-transition tests.
- [ ] Commit:
  ```bash
  git add crates/davinci-agent/src/runtime/tasks.rs crates/davinci-agent/src/runtime/mod.rs
  git commit -m "feat(runtime): add dependency-aware shared task registry"
  ```

### Task 5.2: Add mailboxes and wakeups

**Files**
- Create: `crates/davinci-agent/src/runtime/mailbox.rs`
- Modify: runtime registry
- Modify: persistent agent runner

**Interface**
```rust
pub fn send(&self, message: AgentMessage) -> Result<(), MailboxError>;
pub fn drain(&self, agent_id: AgentId, limit: usize) -> Vec<AgentMessage>;
```

- [ ] Queue to a running/waiting/idle agent.
- [ ] Reject messages to terminal agents.
- [ ] Delivery to idle agent transitions it to `Running` before its next turn.
- [ ] Persist undelivered message metadata before acknowledging success.
- [ ] Add at-most-once delivery ID handling.
- [ ] Commit:
  ```bash
  git add crates/davinci-agent/src/runtime/mailbox.rs crates/davinci-agent/src/runtime/registry.rs
  git commit -m "feat(runtime): add agent messaging and wakeups"
  ```

### Task 5.3: Add model tools for tasks and messaging

**Files**
- Modify: `crates/davinci-agent/src/tools.rs`
- Create focused modules if `tools.rs` grows too large:
  - `crates/davinci-agent/src/runtime/tools_agent.rs`
  - `crates/davinci-agent/src/runtime/tools_task.rs`

**Tools**
- `agent_status`
- `agent_message`
- `agent_stop`
- `task_create`
- `task_update`
- `task_list`

- [ ] Do not expose team tools unless runtime teams are enabled.
- [ ] Tool schemas include IDs as strings and bounded text lengths.
- [ ] `task_update(completed)` routes through completion decision gate.
- [ ] Add permission classes for the new tools.
- [ ] Commit:
  ```bash
  git add crates/davinci-agent/src/tools.rs crates/davinci-agent/src/runtime
  git commit -m "feat(tools): expose agent messaging and shared tasks"
  ```

### Task 5.4: Implement team mode

**Files**
- Create: `crates/davinci-agent/src/runtime/team.rs` or fold into a focused `teams.rs`
- Modify: `davinci_interactive.rs`
- Modify: TUI agent panel

**Rules**
- Team lead is the main agent or explicitly designated parent.
- Teammates share `RunId` but use separate `AgentId`.
- Teammates do not share raw conversation histories.
- Coordination occurs through tasks/messages/context broker.
- Maximum teammates default `4`, hard cap `8`.
- Team mode is disabled by default initially.

- [ ] Add `DAVINCI_EXPERIMENTAL_AGENT_TEAMS=1`.
- [ ] Add test: three teammates claim independent tasks, complete, and lead receives completion notices.
- [ ] Add test: duplicate task claim is rejected deterministically.
- [ ] Add test: interrupting one teammate does not cancel siblings; stopping the run cancels all.
- [ ] Commit:
  ```bash
  git add crates/davinci-agent/src/runtime crates/davinci-coding-agent/src/davinci_interactive.rs crates/davinci-tui
  git commit -m "feat(agents): add shared-task agent teams"
  ```

---

## Phase 6 — Worktree Isolation and Mutation Ownership

### Task 6.1: Implement runtime WorktreeManager

**Files**
- Create: `crates/davinci-agent/src/runtime/worktree.rs`
- Modify: permissions/runtime registry
- Modify: coding-agent host

**Interface**
```rust
pub struct WorktreeLease {
    pub agent_id: AgentId,
    pub path: PathBuf,
    pub branch: String,
    pub base_head: String,
}

pub fn create_lease(...)->Result<WorktreeLease, WorktreeError>;
pub fn release_lease(...)->Result<(), WorktreeError>;
```

**Rules**
- only git repositories;
- no worktree path inside repository tracked paths;
- deterministic naming under runtime temp/state root;
- branch prefix `davinci/agent/<short-agent-id>`;
- never delete a worktree with unpushed/unmerged commits without explicit force;
- emit create/remove events;
- cleanup on normal terminal state; preserve failed worktree for inspection by default.

- [ ] Add fixture tests with a temporary git repo.
- [ ] Add conflict test where two agents request the same explicit branch.
- [ ] Add dirty-worktree preservation test.
- [ ] Commit:
  ```bash
  git add crates/davinci-agent/src/runtime/worktree.rs
  git commit -m "feat(runtime): isolate mutation agents with worktree leases"
  ```

### Task 6.2: Connect worktree isolation to agents and permissions

**Files**
- Modify: `subagent.rs`
- Modify: `permission.rs`
- Modify: runtime registry
- Modify: Graph adapter only where explicitly safe

- [ ] `Agent(isolation:worktree)` must require permission evaluation.
- [ ] An isolated worker's `cwd` becomes its lease path.
- [ ] Add worktree path to `AgentRecord`.
- [ ] Never let a worker mutate the parent's checkout when `isolation=worktree`.
- [ ] Graph behavior stays unchanged unless Graph explicitly opts into leases later.
- [ ] Commit:
  ```bash
  git add crates/davinci-agent/src/subagent.rs crates/davinci-agent/src/permission.rs crates/davinci-agent/src/runtime
  git commit -m "feat(agents): enforce worktree ownership for isolated writers"
  ```

---

## Phase 7 — Dynamic General Workflow Engine

### Task 7.1: Add workflow schema and validator

**Files**
- Create: `runtime/workflow/mod.rs`
- Create: `runtime/workflow/spec.rs`
- Create: `runtime/workflow/validate.rs`

**Validation**
- phase IDs unique;
- dependency DAG acyclic;
- referenced phase exists;
- `max_parallel_agents` `1..=8`;
- `max_total_agents` `1..=64`;
- quorum `1..=worker_count`;
- mutating workers require explicit isolation or single-writer guarantee;
- workflow cannot bypass runtime permissions;
- workflow cannot declare hidden tools.

- [ ] Add one valid 3-phase fixture in inline test constants.
- [ ] Add tests for cycle, missing dependency, invalid quorum, parallel writer violation.
- [ ] Commit:
  ```bash
  git add crates/davinci-agent/src/runtime/workflow
  git commit -m "feat(workflow): define and validate deterministic workflows"
  ```

### Task 7.2: Add artifact/state store

**Files**
- Create: `runtime/workflow/state.rs`

**Purpose**
Keep intermediate results outside the main model context.

```rust
pub struct WorkflowArtifact {
    pub phase_id: String,
    pub worker_id: AgentId,
    pub schema: Option<Value>,
    pub value: Value,
    pub created_ms: i64,
}
```

- [ ] Store artifacts by workflow/phase/worker.
- [ ] Let later worker prompts request selected artifact references only.
- [ ] Add maximum artifact byte size with Governor-backed overflow storage.
- [ ] Add tests proving a 1 MB intermediate artifact does not become a 1 MB parent prompt.
- [ ] Commit:
  ```bash
  git add crates/davinci-agent/src/runtime/workflow/state.rs
  git commit -m "feat(workflow): keep intermediate artifacts outside model context"
  ```

### Task 7.3: Add workflow executor

**Files**
- Create: `runtime/workflow/executor.rs`
- Modify: runtime registry/tasks/mailbox/context

**Execution**
```text
validate
→ register workflow
→ create tasks per phase
→ spawn eligible workers
→ gather schema-validated artifacts
→ evaluate deterministic join
→ unlock next phase
→ finish or fail
```

- [ ] Use scheduler concurrency but enforce workflow-specific caps.
- [ ] Use RuntimeRegistry for every worker.
- [ ] Use ContextBroker and CacheIdentity for every worker.
- [ ] Use Token Governor adapter for every tool result.
- [ ] Use cancellation tree.
- [ ] Add pause/resume/stop state.
- [ ] Add deterministic retry budget per worker.
- [ ] Add tests for all/any/quorum joins and cancellation.
- [ ] Commit:
  ```bash
  git add crates/davinci-agent/src/runtime/workflow
  git commit -m "feat(workflow): execute multi-phase agent workflows"
  ```

### Task 7.4: Add workflow tools and commands

**Files**
- Modify: `tools.rs`
- Modify: `davinci_interactive.rs`
- Create: `crates/davinci-tui/src/davinci/views/workflows.rs`

**Tools**
- `workflow_run`
- `workflow_status`

**Commands**
- `/workflow <goal>`
- `/workflows`
- `/workflow-stop <id>`
- `/workflow-resume <id>`

- [ ] `workflow_run` accepts schema-validated JSON spec, not arbitrary code.
- [ ] Main model can construct a workflow in one tool call; no separate coordinator model.
- [ ] Save operator-approved reusable workflows under `.davinci/workflows/<name>.json`.
- [ ] Project workflow files require trust.
- [ ] Commit:
  ```bash
  git add crates/davinci-agent/src/tools.rs crates/davinci-coding-agent/src/davinci_interactive.rs crates/davinci-tui/src/davinci/views/workflows.rs
  git commit -m "feat(workflow): expose background workflow orchestration"
  ```

### Task 7.5: Define Graph-vs-Workflow routing policy

**Files**
- Create: `docs/runtime-orchestration.md`
- Modify: system/tool descriptions, not core system prompt parity constants.

**Policy**
- Normal agent: default.
- One-shot subagent: bounded independent research.
- Team: interactive multi-perspective or shared-task collaboration.
- Workflow: multi-phase repeatable orchestration with intermediate artifacts.
- Graph: code mutation requiring deterministic plan/verify/review/security/revision guarantees.

- [ ] Add tests ensuring a workflow cannot call Graph internals directly unless `graph_run` is explicitly exposed.
- [ ] Commit:
  ```bash
  git add docs/runtime-orchestration.md
  git commit -m "docs: define orchestration mode boundaries"
  ```

---

## Phase 8 — Compaction Lifecycle, Resume Fidelity, Persistent Runtime Log

### Task 8.1: Emit PreCompact/PostCompact without changing compaction prompt text

**Files**
- Modify: `compaction.rs`
- Modify: `lib.rs`
- Modify: runtime host

- [ ] Emit `PreCompact` with estimated context count.
- [ ] Run current compaction unchanged.
- [ ] Emit `PostCompact` with before/after counts.
- [ ] Notify Token Governor/Memory through runtime subscribers instead of direct special-case calls after equivalence is proven.
- [ ] Add kill switch to retain current direct calls during rollout.
- [ ] Commit:
  ```bash
  git add crates/davinci-agent/src/compaction.rs crates/davinci-agent/src/lib.rs crates/davinci-coding-agent/src/runtime_host.rs
  git commit -m "feat(compaction): publish lifecycle through runtime"
  ```

### Task 8.2: Persist runtime sidecar log

**Files**
- Create: `crates/davinci-session/src/runtime_log.rs`
- Modify: `davinci-session/src/lib.rs`
- Modify: runtime host

**Format**
`<session>.runtime.jsonl`, each row is `RuntimeEventEnvelope`.

- [ ] Append using one JSON object per line.
- [ ] On corrupt final partial line, ignore only the incomplete tail and retain earlier rows.
- [ ] Add schema-version check.
- [ ] Existing session JSONL stays untouched.
- [ ] Add tests for append/replay/partial-tail recovery.
- [ ] Commit:
  ```bash
  git add crates/davinci-session/src/runtime_log.rs crates/davinci-session/src/lib.rs crates/davinci-coding-agent/src/runtime_host.rs
  git commit -m "feat(session): persist versioned runtime lifecycle sidecar"
  ```

### Task 8.3: Rehydrate registry/tasks/messages on resume

**Files**
- Modify: runtime registry/tasks/mailbox
- Modify: session runtime log
- Modify: interactive/RPC resume path

**Rules**
- Terminal agents stay terminal.
- Agents that were running when the process died rehydrate as `Failed` with reason `process_terminated`, unless their execution backend is reconnectable.
- Undelivered mailbox messages remain queued.
- Workflow state can resume only from persisted completed phase artifacts and unambiguous task state.
- Never automatically rerun a mutation worker without fingerprint validation.

- [ ] Add crash fixture: write runtime log ending with a running worker; resume and assert failed state.
- [ ] Add workflow resume fixture reusing completed read-only phases.
- [ ] Commit:
  ```bash
  git add crates/davinci-agent/src/runtime crates/davinci-session crates/davinci-coding-agent/src
  git commit -m "feat(session): restore runtime state safely on resume"
  ```

---

## Phase 9 — Permission Policy v2 and Sandbox Hardening

### Task 9.1: Add parameter-aware permission matcher

**Files**
- Modify: `crates/davinci-agent/src/permission.rs`

**Supported syntax**
```text
Tool
Tool(specifier)
Tool(param:value)
```

Examples:
```text
Agent(model:anthropic/*)
Agent(isolation:worktree)
Bash(run_in_background:true)
Read(./secrets/**)
WebFetch(domain:example.com)
```

**Safety rule**
Never permit generic parameter matching for primary content fields whose interpretation is command/path-specific; keep dedicated matchers for shell command, filesystem path, and URL domain.

- [ ] Add parser AST rather than ad-hoc string splitting.
- [ ] Deny wins at every layer.
- [ ] Add invalid-rule diagnostics rather than silently accepting malformed patterns.
- [ ] Add tests for wildcard, omitted parameter, exact value, deny precedence, malformed rules.
- [ ] Commit:
  ```bash
  git add crates/davinci-agent/src/permission.rs
  git commit -m "feat(permission): add parameter-aware deterministic rules"
  ```

### Task 9.2: Centralize shell command normalization

**Files**
- Create: `crates/davinci-agent/src/shell_policy.rs`
- Modify: `permission.rs`
- Modify: Graph `worker_hooks.rs`
- Modify: security command checks

**Purpose**
Eliminate multiple divergent shell-risk parsers.

- [ ] Parse/normalize shell command families conservatively.
- [ ] Detect command substitutions, nested shells (`sh -c`, `bash -c`, PowerShell `-Command`), redirections, destructive filesystem operations, privilege escalation, package-install commands, and git state mutation.
- [ ] Preserve Graph's stricter writer/read-only policies as policy profiles on the same analyzer.
- [ ] Unknown parse constructs return `NeedsApproval` or `Denied` in restricted workers, never `Allowed`.
- [ ] Add cross-platform fixture corpus for Bash and PowerShell.
- [ ] Commit:
  ```bash
  git add crates/davinci-agent/src/shell_policy.rs crates/davinci-agent/src/permission.rs crates/davinci-coding-agent/src/native_extensions/graph
  git commit -m "feat(security): unify shell policy across agent types"
  ```

### Task 9.3: Add filesystem boundary policy

**Files**
- Modify: permission/runtime worktree/tools

- [ ] Resolve paths without following untrusted symlink escapes for mutation checks.
- [ ] Enforce worktree root for isolated agents.
- [ ] Add explicit read-outside-root policy.
- [ ] Keep git metadata access required for normal git operations.
- [ ] Add Windows drive/UNC path tests and Unix absolute/symlink tests.
- [ ] Commit:
  ```bash
  git add crates/davinci-agent/src/permission.rs crates/davinci-agent/src/runtime/worktree.rs crates/davinci-agent/src/tools.rs
  git commit -m "feat(security): enforce runtime filesystem boundaries"
  ```

---

## Phase 10 — Plugins, MCP, Skills, Custom Agents on the Same Runtime

### Task 10.1: Add RuntimeCapabilityRegistry

**Files**
- Create: `runtime/capabilities.rs`
- Modify: `extension_host.rs`
- Modify: `mcp.rs` / MCP host
- Modify: tools discovery

**Capability record**
```rust
pub struct RuntimeCapability {
    pub name: String,
    pub source: CapabilitySource,
    pub tool_class: ToolClass,
    pub read_only: bool,
    pub schema_hash: String,
    pub version: Option<String>,
}
```

- [ ] Built-ins, JS extensions, native extensions, and MCP tools register the same metadata.
- [ ] Context/cache hashing uses capability schema/version.
- [ ] Subagent/workflow tool scoping reads the registry instead of guessing by name.
- [ ] Unknown capability remains mutation-capable by default.
- [ ] Commit:
  ```bash
  git add crates/davinci-agent/src/runtime/capabilities.rs crates/davinci-coding-agent/src/extension_host.rs crates/davinci-agent/src/mcp.rs
  git commit -m "feat(runtime): unify tool capabilities across builtins plugins and MCP"
  ```

### Task 10.2: Namespace/version plugin resources

**Files**
- Modify: extension manifests/discovery
- Modify: skill loader
- Modify: agent profile loader

- [ ] Add stable plugin namespace.
- [ ] Skills exposed as `<plugin>:<skill>`.
- [ ] Plugin agents exposed as `<plugin>:<agent>`.
- [ ] Record manifest version in cache identity and provenance.
- [ ] Reject namespace collisions deterministically.
- [ ] Add `/reload-plugins` equivalent that rebuilds capability registry and reports cache-invalidating changes.
- [ ] Commit:
  ```bash
  git add crates/davinci-coding-agent/src/extensions.rs crates/davinci-coding-agent/src/extension_host.rs crates/davinci-agent/src/skills.rs crates/davinci-coding-agent/src/agent_profiles.rs
  git commit -m "feat(plugins): namespace and version agent resources"
  ```

### Task 10.3: Add skill doctor/context-cost diagnostics

**Files**
- Modify: skills loader
- Modify: context broker
- Modify: interactive commands

**Report per skill**
- source
- version/hash
- description tokens
- body tokens when loaded
- invocation count
- automatic injection count
- accepted learning outcome count
- last used timestamp

- [ ] Add `/skill-doctor`.
- [ ] Flag:
  - loaded but unused;
  - high context cost;
  - shadowed duplicate;
  - stale failed skill version.
- [ ] Do not auto-delete skills.
- [ ] Commit:
  ```bash
  git add crates/davinci-agent/src/skills.rs crates/davinci-agent/src/runtime/context.rs crates/davinci-coding-agent/src/davinci_interactive.rs
  git commit -m "feat(skills): expose context cost and usage diagnostics"
  ```

---

## Phase 11 — Cross-Session Runtime Control, Remote Control, SDK/RPC/IDE Surface

### Task 11.1: Add runtime protocol DTOs

**Files**
- Create: `crates/davinci-protocol/src/runtime.rs`
- Modify: `crates/davinci-protocol/src/lib.rs`
- Modify: client/server dispatch

**Read operations**
- list sessions
- list agents
- get agent
- list tasks
- list workflows
- get workflow
- read agent transcript pointer/status

**Control operations**
- send agent message
- stop agent
- stop workflow
- pause/resume workflow
- set permission mode
- interrupt turn

- [ ] Every control request carries target session/run/agent ID.
- [ ] Server validates ownership and current state before execution.
- [ ] Add protocol round-trip tests.
- [ ] Commit:
  ```bash
  git add crates/davinci-protocol crates/davinci-client crates/davinci-server
  git commit -m "feat(protocol): expose runtime state and controls"
  ```

### Task 11.2: Extend RPC and SDK

**Files**
- Modify: `crates/davinci-coding-agent/src/rpc.rs`
- Modify: `crates/davinci-coding-agent/src/sdk.rs`

- [ ] Add read/control methods matching protocol DTOs.
- [ ] Keep existing RPC methods unchanged.
- [ ] Interrupt uses runtime cancellation tree.
- [ ] Add SDK tests with a fake in-process runtime.
- [ ] Commit:
  ```bash
  git add crates/davinci-coding-agent/src/rpc.rs crates/davinci-coding-agent/src/sdk.rs
  git commit -m "feat(sdk): control agents workflows and sessions through runtime"
  ```

### Task 11.3: Add derived SQLite runtime index

**Files**
- Create: `crates/davinci-session-sqlite/src/runtime_index.rs`
- Modify: `davinci-session-sqlite/src/lib.rs`

**Tables**
- sessions
- runtime_runs
- runtime_agents
- runtime_tasks
- runtime_workflows

Treat SQLite as rebuildable derived state; JSONL/sidecar remains source of truth.

- [ ] Add rebuild-from-runtime-log test.
- [ ] Add index version/migration test.
- [ ] Commit:
  ```bash
  git add crates/davinci-session-sqlite
  git commit -m "feat(session): index runtime agents tasks and workflows"
  ```

### Task 11.4: TUI runtime panels

**Files**
- Create: `views/agents.rs`
- Create: `views/runtime.rs`
- Modify: interactive keybindings/commands

- [ ] Show agent state, task, model, elapsed, worktree.
- [ ] Enter opens agent detail/transcript.
- [ ] Esc from detail does not kill; explicit stop command does.
- [ ] Interrupt focused agent uses targeted cancellation.
- [ ] Keep `NO_COLOR` and existing Davinci visual-language invariants.
- [ ] Commit:
  ```bash
  git add crates/davinci-tui crates/davinci-coding-agent/src/davinci_interactive.rs
  git commit -m "feat(tui): inspect and control runtime agents"
  ```

This runtime API becomes the foundation for future VS Code/Desktop/web clients. Do not put product logic into the TUI.

---

## Phase 12 — Managed Policy and Governance

### Task 12.1: Add layered policy model

**Files**
- Create: `crates/davinci-coding-agent/src/policy.rs`
- Modify: settings/trust/permissions/plugins

**Precedence**
```text
compiled safety invariants
> managed organization policy
> user policy
> trusted project policy
> session grants
```

Deny at a higher layer cannot be relaxed below it.

- [ ] Define managed policy path/env source without requiring a network service.
- [ ] Policy covers:
  - allowed providers/models;
  - permission ceiling;
  - plugins allowed/denied;
  - MCP servers allowed/denied;
  - agent teams/workflows allowed;
  - worktree policy;
  - remote-control policy.
- [ ] Add `/status` policy source and active restrictions.
- [ ] Commit:
  ```bash
  git add crates/davinci-coding-agent/src/policy.rs crates/davinci-coding-agent/src/settings.rs crates/davinci-agent/src/permission.rs
  git commit -m "feat(policy): add immutable managed configuration layer"
  ```

### Task 12.2: Plugin governance

- [ ] Add allow/deny by plugin namespace and source.
- [ ] Refuse project plugin loading before trust decision.
- [ ] Managed force-disabled plugin cannot be re-enabled by project config.
- [ ] Log policy reason without exposing secrets.
- [ ] Commit:
  ```bash
  git add crates/davinci-coding-agent/src/extension_host.rs crates/davinci-coding-agent/src/policy.rs
  git commit -m "feat(policy): govern plugin and MCP capability loading"
  ```

---

## Phase 13 — Observability, Cache Diagnostics, Context Inspection

### Task 13.1: RuntimeStats

**Files**
- Create: `crates/davinci-agent/src/runtime/stats.rs`
- Modify: existing `stats.rs`
- Modify: ecosystem telemetry

**Counters**
- active agents by kind/state
- spawned/completed/failed agents
- messages queued/delivered
- tasks created/completed/blocked
- workflow phases
- context items/tokens by source
- cache identity reuse/invalidation reason
- Governor compact/retrieve counts by agent kind
- permission denials by class
- worktree create/preserve/remove
- cancellation propagation depth

- [ ] Use atomics/locks without model-visible state.
- [ ] Add serialization compatibility tests.
- [ ] Commit:
  ```bash
  git add crates/davinci-agent/src/runtime/stats.rs crates/davinci-agent/src/stats.rs crates/davinci-coding-agent/src/native_extensions/ecosystem
  git commit -m "feat(telemetry): measure shared runtime participation"
  ```

### Task 13.2: `/context` and cache-miss diagnosis

**Files**
- Modify: interactive commands
- Create: `views/context.rs`

`/context` shows:
- current estimated context
- system/tool overhead
- broker items by source
- compacted/pruned output count
- skill/memory tokens
- current cache key short hash
- last cache invalidation reason

- [ ] Never dump full secret-bearing context by default.
- [ ] Add `--provenance` to show source identifiers/hashes, not raw hidden content.
- [ ] Commit:
  ```bash
  git add crates/davinci-coding-agent/src/davinci_interactive.rs crates/davinci-tui/src/davinci/views/context.rs
  git commit -m "feat(context): explain token use and cache affinity"
  ```

---

## Phase 14 — Evals, Reliability and Battle-Hardening

### Task 14.1: Add orchestration eval corpus

**Files**
- Add fixtures under existing `crates/davinci-parity/fixtures` or `davinci-evals` according to current project convention.
- Modify eval runner.

**Scenarios**
1. normal single-agent bug fix;
2. parallel read-only research;
3. team task claim;
4. message wakeup;
5. workflow all-join;
6. workflow quorum;
7. workflow cancellation;
8. isolated writer worktree;
9. resume after process interruption;
10. cache-stable retry;
11. denied mutation;
12. Governor retrieval in worker;
13. Graph run unchanged.

**Metrics**
- total input/output tokens
- tool calls
- model calls
- wall time
- max concurrent agents
- failed/retried workers
- cache read/write tokens when provider supplies them
- verification result
- stale/duplicate work

- [ ] Preserve existing `median_tools <= 10.0` no-worsening gate for baseline tasks.
- [ ] Add no-regression thresholds comparing normal mode with runtime disabled/enabled.
- [ ] Commit:
  ```bash
  git add crates/davinci-evals crates/davinci-parity
  git commit -m "test(evals): benchmark runtime orchestration and efficiency"
  ```

### Task 14.2: Concurrency/race corpus

- [ ] Add deterministic stress tests:
  - 8 parallel read workers;
  - simultaneous mailbox sends;
  - cancel during provider retry;
  - cancel during tool execution;
  - workflow pause during phase transition;
  - task completion racing cancellation;
  - two isolated worktree writers;
  - session shutdown with jobs + agents + workflow active.
- [ ] Assert no orphan process remains.
- [ ] Assert no double-delivered message.
- [ ] Assert registry has no `Running` child after parent run terminal state.
- [ ] Commit:
  ```bash
  git commit -am "test(runtime): harden concurrency and shutdown behavior"
  ```

### Task 14.3: Fault-injection corpus

Inject:
- corrupt runtime sidecar tail;
- missing worktree;
- plugin load failure;
- MCP unavailable;
- memory unavailable;
- Governor store write failure;
- worker exits before artifact;
- workflow artifact schema failure;
- permission hook timeout;
- runtime observer panic.

Expected:
- memory/Governor/observability failures fail open where current semantics require;
- security/permission/verification failures fail closed;
- state always lands in an explicit terminal or retryable state.

- [ ] Commit:
  ```bash
  git commit -am "test(runtime): add deterministic fault injection coverage"
  ```

---

## Phase 15 — Product Rollout and Default-On Criteria

### Task 15.1: Feature flags

Add:
```text
DAVINCI_RUNTIME=0|1
DAVINCI_RUNTIME_HOOKS_V2=0|1
DAVINCI_RUNTIME_CONTEXT_BROKER=0|1
DAVINCI_RUNTIME_CACHE=0|1
DAVINCI_RUNTIME_PERSISTENT_AGENTS=0|1
DAVINCI_EXPERIMENTAL_AGENT_TEAMS=0|1
DAVINCI_RUNTIME_WORKFLOWS=0|1
DAVINCI_RUNTIME_WORKTREES=0|1
DAVINCI_RUNTIME_PERMISSION_V2=0|1
DAVINCI_RUNTIME_REMOTE_CONTROL=0|1
```

- [ ] Every feature flag has a test proving disabled behavior follows the previous path.
- [ ] `/status` reports enabled runtime capabilities.
- [ ] Commit:
  ```bash
  git commit -am "feat(runtime): add staged rollout controls"
  ```

### Task 15.2: Default-on criteria

A subsystem becomes default-on only when:

- workspace tests pass on Linux and Windows;
- no baseline normal-turn token regression > 3%;
- no baseline tool-call regression > 5%;
- no additional model call in normal mode;
- zero orphan process in stress corpus;
- resume fixtures pass;
- permission/security bypass corpus passes;
- Graph ecosystem suite passes unchanged;
- cache-affinity structural tests pass;
- user can disable the subsystem with one documented switch.

### Task 15.3: Remove temporary dual paths only after two release cycles

Do not delete:
- old hook path,
- direct Graph ecosystem context builder,
- legacy abort field,
- old permission parser,

until the replacement has been default-on for two tagged releases with CI and eval evidence.

When removing a dual path:
- add a migration note;
- keep settings compatibility;
- preserve old session reading;
- add one test for the retired legacy format.

---

# 8. Integration Details by Existing Davinci Subsystem

## Graph Engineer

**Must consume**
- RuntimeRegistry
- RuntimeEventBus
- Cancellation tree
- ContextBroker
- CacheIdentity
- universal Governor adapter

**Must remain authoritative for**
- Graph topology
- role/tool least privilege
- writer exclusivity
- replay fingerprints
- mutation provenance
- deterministic verification
- review coverage
- revision/replan loops
- security gate
- learning outcome attribution

Graph is a runtime participant, not a workflow-engine implementation detail.

## Token Governor

Move from "native extension that happens to wrap some paths" toward one runtime output middleware contract.

Do not change:
- digest algorithm until separate benchmarking justifies it;
- lossless `retrieve_output`;
- error/memory/retrieve exemptions;
- freshness reset semantics.

## Vector Memory

Become a `ContextSource`.

Add scope:
- repository
- profile/repository
- optional global profile

Do not make memory a permission bypass or instruction source.

## Learning

Subscribe to verified runtime outcomes.

Accepted learning evidence priority:
1. deterministic Graph verified result;
2. workflow result with deterministic verifier;
3. explicit user-approved foreground learning;
4. normal-turn background review.

Never upgrade a normal-turn heuristic success to Graph-level confidence.

## Security

Subscribe to mutation/verification events.

High-risk Graph mutations still block approval exactly as now.

For general workflows:
- any mutation worker must generate a changed-surface record;
- high-risk change triggers the same deterministic security entry point;
- workflow completion cannot claim verified success when required security failed.

## Sessions

Current JSONL remains source of conversation truth.

New runtime sidecar stores orchestration truth.

Derived SQLite indexes may be deleted/rebuilt.

## Plugins/MCP

Everything advertises capability metadata.

No extension may mark itself read-only without host verification/config classification.

Unclassified tools remain mutation-capable.

---

# 9. CLI / UX Additions

Recommended commands:

```text
/agents
/agent <id>
/agent-stop <id>
/tasks
/workflows
/workflow <goal>
/workflow-stop <id>
/workflow-resume <id>
/context
/skill-doctor
/runtime-status
```

Existing commands remain unchanged.

Recommended status additions:

```text
agents  3 active · 1 idle
tasks   4/6 complete · 1 blocked
flow    deep-research · phase 2/4 · 6.2k tok
cache   hit-prefix · 18.4k read
ctx     47k/200k · broker 2.1k
```

Avoid showing all rows when zero; preserve current compact status design.

---

# 10. Security Model for Multi-Agent Operation

1. A child cannot gain more permission than its parent + profile + managed policy intersection.
2. A worktree isolates filesystem mutation but does not grant new tools.
3. A message cannot alter another agent's permission policy.
4. Task ownership does not imply filesystem ownership.
5. Mutation workers require either:
   - exclusive shared-checkout writer lease, or
   - worktree isolation.
6. Background agents cannot outlive run/session ownership unless explicitly promoted to a reconnectable background session.
7. Project-defined profiles/workflows/plugins/hooks require trust.
8. Managed policy can globally disable teams, workflows, remote control, plugins, or MCP.
9. Runtime sidecars redact secrets using the same redaction utilities used by security/learning.
10. Runtime telemetry stores IDs/hashes/metrics, not full prompts by default.

---

# 11. Token-Efficiency Requirements

This project is specifically meant to close Claude Code's orchestration/context advantages without losing Davinci's Governor strength.

Required budgets:

- Runtime metadata injected into a model call: `0` tokens unless requested by a context source/tool.
- Runtime event bus: `0` model calls.
- Registry/task/mailbox updates: `0` model calls.
- Cache-key derivation: `0` model calls.
- Default normal-turn broker augmentation: cap separately and benchmark before default-on.
- Graph broker augmentation: preserve existing `<=2500`.
- Workflow artifact transfer: inject only selected fields; do not paste entire upstream worker transcripts.
- Team notifications: compact machine message first; full transcript retrieved on demand.
- Agent status tools: bounded response; default max 20 rows.
- Workflow progress: bounded summary; full worker details fetched by ID.
- Skill doctor: compute token estimates locally.

Acceptance:
- runtime enabled but unused must produce statistically indistinguishable token usage from runtime disabled on the normal-turn eval corpus.

---

# 12. Cache-Efficiency Requirements

A compatible retry should preserve the cache key when:
- same provider/model;
- same stable system prompt;
- same visible tools/schema;
- same agent profile;
- same stable context sources;
- same role/contract.

Invalidate when:
- model/provider changes;
- tool schema changes;
- permissions remove/add a visible tool;
- profile content changes;
- stable memory/skill injection changes;
- Graph/workflow contract changes.

Do not invalidate for:
- runtime sequence number;
- task progress counters;
- timestamps;
- agent UUID when the worker role/prefix is otherwise identical;
- UI state.

Record one reason code for the first differing component and expose it in `/context` or `/cost`-style diagnostics.

---

# 13. Testing Matrix

| Area | Unit | Integration | Offline fixture | E2E |
|---|---:|---:|---:|---:|
| Runtime events | yes | yes | yes | yes |
| Registry | yes | yes | yes | yes |
| Cancellation | yes | yes | yes | yes |
| Context broker | yes | yes | yes | yes |
| Cache identity | yes | yes | yes | live telemetry optional |
| Profiles | yes | yes | yes | yes |
| Tasks/mailbox | yes | yes | yes | yes |
| Teams | yes | yes | yes | yes |
| Worktrees | yes | yes | temp git repo | yes |
| Workflows | yes | yes | yes | yes |
| Compaction/resume | yes | yes | yes | yes |
| Permission v2 | yes | yes | corpus | yes |
| Shell policy | yes | yes | corpus | yes |
| Plugins/MCP | yes | yes | fake hosts | yes |
| Remote protocol | yes | yes | loopback | yes |
| Graph regression | yes | yes | existing | yes |
| Governor recovery | yes | yes | existing | yes |
| Learning/security | yes | yes | existing | yes |

---

# 14. CI Gate Order

Recommended `.github/workflows/ci.yml` order:

```text
1. fmt
2. clippy
3. davinci-agent runtime unit tests
4. runtime event/registry/context/cache invariants
5. permission + shell-policy corpus
6. ecosystem integration
7. ecosystem invariants
8. Graph regression suite
9. workflow/team offline suite
10. session resume/recovery suite
11. workspace tests
12. eval no-regression smoke gate
```

Fail fast before expensive workspace/eval stages.

---

# 15. Documentation Deliverables

Create/update during implementation:

```text
docs/runtime.md
docs/runtime-orchestration.md
docs/agents.md
docs/agent-teams.md
docs/workflows.md
docs/context-and-cache.md
docs/permissions.md
docs/hooks.md
docs/plugins.md
docs/remote-control.md
docs/ecosystem.md
AGENTS.md
CLAUDE.md
README.md
```

Every doc must state:
- default state;
- feature flag;
- trust implications;
- permission implications;
- persistence location;
- kill switch;
- recovery behavior.

---

# 16. What Not to Build Yet

To avoid feature bloat and preserve token efficiency:

1. Do not add another LLM whose only job is orchestration.
2. Do not replace Graph with the workflow engine.
3. Do not add arbitrary user-authored controller code in workflow v1.
4. Do not add distributed multi-machine agents before local runtime registry/protocol is stable.
5. Do not add a marketplace backend service before plugin namespaces/versioning/governance are stable.
6. Do not rewrite compaction prompts.
7. Do not migrate session JSONL format in place.
8. Do not make every normal prompt auto-create a workflow.
9. Do not inject live teammate transcripts into the lead automatically.
10. Do not claim cache-performance gains from structural/offline tests alone.

---

# 17. Completion Definition

Davinci is considered architecturally converged on the major Claude Code advantages when all of the following are true:

- [ ] Main agent, Graph worker, subagent, teammate, background agent, and workflow worker appear in one registry.
- [ ] All model calls can use one bounded context/cache contract.
- [ ] All worker tool output can use Governor lossless virtualization.
- [ ] Lifecycle events cover sessions, turns, tools, permissions, agents, tasks, workflows, compaction, model switches, worktrees, and shutdown.
- [ ] Existing hook configs remain compatible.
- [ ] Custom agent profiles support model/tools/permissions/memory scope.
- [ ] Persistent agents can receive messages and wake.
- [ ] Shared tasks support dependencies, ownership, completion, and failure propagation.
- [ ] Agent teams can collaborate without raw transcript sharing.
- [ ] Mutation agents can use exclusive worktrees.
- [ ] General workflows run phases in the background and keep intermediate artifacts outside main context.
- [ ] Graph retains all current deterministic safety/verification guarantees.
- [ ] Compaction and session resume rebuild runtime state safely.
- [ ] Permission rules support tool-specific and parameter-aware restrictions.
- [ ] Shell security policy is shared across normal agents, Graph, workflows, and subagents.
- [ ] Plugins/MCP advertise capability metadata through one registry.
- [ ] `/skill-doctor` and `/context` expose context waste and cache causes.
- [ ] RPC/SDK can inspect/control runtime agents and workflows.
- [ ] Managed policy can cap permissions/features/plugins.
- [ ] Normal-turn runtime-enabled overhead adds zero model calls.
- [ ] Normal-turn token regression stays within the release threshold.
- [ ] No orphan processes remain after cancellation/session shutdown.
- [ ] Current ecosystem CI suite remains green.
- [ ] Workspace tests, clippy, fmt, orchestration evals, and fault-injection suites are green.

---

# 18. Recommended Execution Order

Use one implementation branch/worktree per phase.

```text
Phase 0
  ↓
Phase 1 ─ Runtime events/hooks
  ↓
Phase 2 ─ Registry/cancellation
  ↓
Phase 3 ─ Context/cache/Governor
  ↓
Phase 4 ─ Persistent agents/profiles
  ↓
Phase 5 ─ Tasks/messages/teams
  ↓
Phase 6 ─ Worktree isolation
  ↓
Phase 7 ─ Workflow engine
  ↓
Phase 8 ─ Compaction/resume
  ↓
Phase 9 ─ Permissions/sandbox v2
  ↓
Phase 10 ─ Plugins/MCP/skills convergence
  ↓
Phase 11 ─ Protocol/RPC/SDK/remote control
  ↓
Phase 12 ─ Managed policy
  ↓
Phase 13 ─ Observability/context diagnostics
  ↓
Phase 14 ─ Evals/fault injection
  ↓
Phase 15 ─ Staged rollout
```

Do not parallelize Phases 1–3 because they define contracts consumed by every later phase.

After Phase 3 is merged, these can proceed in parallel with separate worktrees:
- Phase 4 agent profiles;
- Phase 9 permission matcher;
- Phase 10 capability registry;
- Phase 13 observability scaffolding.

Phase 5 depends on Phase 4.
Phase 6 depends on Phase 2 + Phase 5.
Phase 7 depends on Phases 2–6.
Phase 8 depends on Phases 1–3 + 7.
Phase 11 depends on Phases 2 + 5 + 7 + 8.
Phase 12 depends on Phases 9–11.
Phase 14 begins as soon as Phase 1 lands and expands continuously.

---

# 19. Per-Phase Review Checklist

Before merging each phase:

```text
[ ] Does runtime-disabled behavior still match current behavior?
[ ] Did this phase add any model call? If yes, is it user-requested rather than orchestration overhead?
[ ] Does Graph still pass ecosystem invariants?
[ ] Are permissions equal or stricter?
[ ] Is context bounded?
[ ] Is output recoverable when compacted?
[ ] Are cancellation and shutdown deterministic?
[ ] Is persisted state versioned?
[ ] Are corrupt/incomplete persisted tails recoverable?
[ ] Are all tests offline?
[ ] Does Windows path/process behavior have coverage?
[ ] Does clippy pass with -D warnings?
[ ] Does cargo fmt --check pass?
[ ] Does cargo test --workspace pass?
```

---

# 20. Final Verification Command Set

```bash
cargo fmt --check

cargo clippy --workspace --all-targets -- -D warnings

cargo test -p davinci-agent runtime -- --nocapture

cargo test -p davinci-agent permission -- --nocapture

cargo test -p davinci-coding-agent ecosystem_loop_ -- --nocapture

cargo test -p davinci-coding-agent ecosystem_invariants_ -- --nocapture

cargo test -p davinci-coding-agent runtime_migration_preserves_ecosystem_baseline -- --nocapture

cargo test --workspace

cargo run -p davinci-parity
```

Then run the orchestration eval corpus with runtime off and on, compare:
- tokens;
- tool calls;
- model calls;
- wall time;
- task completion;
- verification;
- orphan-process count;
- cache telemetry where available.

No subsystem should be declared complete based only on unit tests.

---

# 21. Target End State

After this plan, Davinci should no longer be best described as:

```text
Graph + Governor + Memory + Learning + Security + Subagents + Jobs + Plugins
```

It should be:

```text
Davinci Runtime
    ├── normal agent
    ├── Graph high-assurance execution
    ├── subagents
    ├── teams
    ├── workflows
    ├── background agents/jobs
    ├── sessions
    ├── context/cache broker
    ├── Token Governor
    ├── memory
    ├── learning
    ├── security
    ├── permissions
    ├── MCP/plugins
    └── TUI/RPC/SDK/remote clients
```

The important competitive improvement is not the raw feature count. It is that every execution mode participates in the same identity, lifecycle, context, cache, permission, messaging, persistence, observability, verification, and cancellation contracts while Davinci keeps its deterministic Graph and closed-loop learning/security advantages.


---

# 22. Extended Competitive Parity Phases

The core runtime phases above close the architectural gap. The following phases close the remaining product/platform gaps that are not safe to mix into the runtime refactor itself.

## Phase 16 — First-Party IDE Client and Workspace Visualization

Treat this as a separate implementation branch after Phase 11's RPC/runtime-control API is stable.

### Task 16.1: Define editor-neutral session API acceptance contract

**Files**
- Modify: `crates/davinci-protocol/src/runtime.rs`
- Modify: `crates/davinci-coding-agent/src/sdk.rs`
- Create: `docs/editor-client-api.md`

**Required editor capabilities**
- create/open/resume/archive session;
- stream transcript events;
- list models;
- change model/thinking level;
- show permission request and submit decision;
- list agents/tasks/workflows;
- inspect agent transcript;
- targeted interrupt;
- show context/cache metrics;
- show changed files/diff;
- send follow-up while work is active.

- [ ] Add protocol fixture snapshots for every request/response/event.
- [ ] Version the editor protocol independently from internal runtime structs.
- [ ] Ensure an old editor client can ignore unknown event fields.
- [ ] Commit:
  ```bash
  git add crates/davinci-protocol/src/runtime.rs crates/davinci-coding-agent/src/sdk.rs docs/editor-client-api.md
  git commit -m "feat(protocol): stabilize editor-neutral runtime API"
  ```

### Task 16.2: Add first-party VS Code client

**Files**
- Create a new non-Cargo client root: `clients/vscode/`
- Do not reuse stale `packages/*` implementation stubs.
- Create:
  - `clients/vscode/package.json`
  - `clients/vscode/src/extension.ts`
  - `clients/vscode/src/runtimeClient.ts`
  - `clients/vscode/src/sessionTree.ts`
  - `clients/vscode/src/chatView.ts`
  - `clients/vscode/src/diffView.ts`
  - `clients/vscode/src/agentView.ts`
  - `clients/vscode/src/permissionView.ts`

**Architecture**
- VS Code client contains no agent logic.
- Spawn/connect to Davinci through the runtime protocol.
- Rust runtime remains source of truth for permissions, agents, tasks, cache, and workflows.

- [ ] Session tree displays open/archived/running sessions.
- [ ] Agent tree displays state and task owner.
- [ ] Diff view reads Git/workspace state through a bounded runtime endpoint.
- [ ] Permission UI sends explicit decisions back to Rust.
- [ ] Interrupt maps to targeted runtime cancellation.
- [ ] Add fixture-driven TypeScript tests with a fake protocol server.
- [ ] Add CI job for the client only after its lockfile/toolchain is pinned.
- [ ] Commit:
  ```bash
  git add clients/vscode .github/workflows
  git commit -m "feat(vscode): add first-party Davinci runtime client"
  ```

### Task 16.3: Make diff/workspace state a reusable runtime service

**Files**
- Create: `crates/davinci-agent/src/runtime/workspace.rs`
- Modify: TUI diff view
- Modify: protocol runtime DTOs

**Interface**
```rust
pub struct WorkspaceDelta {
    pub branch: Option<String>,
    pub files: Vec<FileDelta>,
    pub added: u64,
    pub removed: u64,
    pub generated_ms: i64,
}
```

- [ ] TUI and VS Code consume the same service.
- [ ] Exclude binary contents; return path/status/stat only unless a specific diff is requested.
- [ ] Preserve Graph mutation provenance as a separate, stricter concept.
- [ ] Commit:
  ```bash
  git add crates/davinci-agent/src/runtime/workspace.rs crates/davinci-protocol crates/davinci-tui
  git commit -m "feat(workspace): share live diff state across clients"
  ```

---

## Phase 17 — Plugin Marketplace, Distribution, and Ecosystem Management

Start only after Phase 10 namespace/version/capability contracts are stable.

### Task 17.1: Add marketplace index format

**Files**
- Create: `crates/davinci-coding-agent/src/plugin_marketplace.rs`
- Create: `docs/plugin-marketplaces.md`

**Index**
```json
{
  "schemaVersion": 1,
  "name": "example-marketplace",
  "plugins": [
    {
      "name": "review-tools",
      "version": "1.2.0",
      "source": "git",
      "url": "https://github.com/example/review-tools",
      "rev": "immutable-commit-sha",
      "sha256": "..."
    }
  ]
}
```

**Rules**
- immutable revision required for installed resolution;
- hash verified before activation;
- user/project/managed policy can allow/deny marketplace or plugin;
- project marketplace config requires trust;
- installation never executes plugin code before manifest/policy validation.

- [ ] Add malformed-index, hash-mismatch, duplicate-version, and denied-source tests.
- [ ] Commit:
  ```bash
  git add crates/davinci-coding-agent/src/plugin_marketplace.rs docs/plugin-marketplaces.md
  git commit -m "feat(plugins): add verified marketplace index format"
  ```

### Task 17.2: Add plugin lock state and atomic updates

**Files**
- Create: `crates/davinci-coding-agent/src/plugin_lock.rs`
- Modify: extension installer/config

**Lock record**
```rust
pub struct LockedPlugin {
    pub namespace: String,
    pub version: String,
    pub source: String,
    pub revision: String,
    pub sha256: String,
}
```

- [ ] Write new plugin into staging directory.
- [ ] Validate manifest/resources/capabilities.
- [ ] Atomically swap active pointer/directory.
- [ ] Keep previous version for one rollback.
- [ ] Rebuild RuntimeCapabilityRegistry.
- [ ] Emit `ConfigChange`/runtime warning event with cache invalidation reason.
- [ ] Commit:
  ```bash
  git add crates/davinci-coding-agent/src/plugin_lock.rs crates/davinci-coding-agent/src/extension_host.rs
  git commit -m "feat(plugins): lock verify and atomically update plugins"
  ```

### Task 17.3: Add plugin management UX

**Commands**
```text
/plugin list
/plugin search <query>
/plugin install <namespace>
/plugin update <namespace>
/plugin rollback <namespace>
/plugin remove <namespace>
/reload-plugins
```

- [ ] Commands render policy reason when action is blocked.
- [ ] Search/install network calls obey centralized transport/proxy configuration from Phase 18.
- [ ] Never expose marketplace data as model instructions.
- [ ] Commit:
  ```bash
  git commit -am "feat(plugins): add marketplace management commands"
  ```

---

## Phase 18 — Enterprise Transport, Proxy Reliability, and Authentication Policy

This phase improves the corporate-network and enterprise-management gap without replacing existing provider authentication.

### Task 18.1: Centralize HTTP/TLS/proxy configuration

**Files**
- Inspect existing `davinci-ai` HTTP/provider clients and create one shared transport configuration module at the narrowest common layer, for example:
  `crates/davinci-ai/src/transport_config.rs`.
- Modify provider/MCP/update/plugin-marketplace clients to consume it.

**Interface**
```rust
pub struct TransportConfig {
    pub connect_timeout_ms: u64,
    pub request_timeout_ms: u64,
    pub proxy: Option<String>,
    pub no_proxy: Vec<String>,
    pub extra_ca_file: Option<PathBuf>,
    pub offline: bool,
}
```

- [ ] Environment precedence is deterministic and documented.
- [ ] `offline=true` blocks network before DNS/connect.
- [ ] TLS errors identify which trust source was used without printing credentials.
- [ ] Add local fake-proxy and self-signed test fixtures; never call public internet in tests.
- [ ] Commit:
  ```bash
  git add crates/davinci-ai/src/transport_config.rs crates/davinci-ai crates/davinci-mcp crates/davinci-coding-agent
  git commit -m "feat(network): centralize proxy timeout and TLS policy"
  ```

### Task 18.2: Add credential-source diagnostics

**Files**
- Modify provider auth/status modules.
- Modify `/status` and `/doctor`-style diagnostics.

**Report**
- active provider;
- active credential source category (`api_key`, `oauth`, `device_code`, `adc`, `managed_gateway`);
- expiry/refresh state when known;
- inactive conflicting credential sources;
- proxy/TLS policy source;
- managed-policy restrictions.

Do not display tokens, client secrets, refresh tokens, or full credential paths containing secrets.

- [ ] Add conflict fixture: API key + OAuth both present; report which one wins.
- [ ] Add expired-refresh fixture.
- [ ] Add unreachable credential-helper timeout fixture.
- [ ] Commit:
  ```bash
  git commit -am "feat(auth): explain active credential and transport policy"
  ```

### Task 18.3: Managed authentication ceiling

**Files**
- Modify: `policy.rs`
- Modify: provider login/selection

Managed policy may require:
- a specific provider family;
- a managed gateway;
- no raw API-key login;
- specific models;
- remote control disabled;
- plugin marketplace restrictions.

- [ ] A user setting cannot bypass managed authentication restrictions.
- [ ] Existing Bedrock/Vertex/provider flows remain available when policy permits.
- [ ] Add tests for forced provider and forbidden API-key login.
- [ ] Commit:
  ```bash
  git add crates/davinci-coding-agent/src/policy.rs crates/davinci-ai
  git commit -m "feat(policy): enforce managed authentication constraints"
  ```

---

# 23. Competitive Gap Coverage Audit

This table is the final scope audit against the previously identified Claude Code advantages.

| Gap | Covered by |
|---|---|
| lifecycle/event system | Phases 1, 8 |
| first-class subagents | Phase 4 |
| agent teams | Phase 5 |
| agent messaging | Phase 5 |
| cross-session coordination | Phases 8, 11 |
| dynamic workflows | Phase 7 |
| orchestration strategy breadth | Phases 4, 5, 7 + existing Graph |
| universal orchestration fabric | Phases 1–3 |
| background agents | Phases 2, 4, 5 |
| task management | Phase 5 |
| worktree parallelism | Phase 6 |
| prompt-cache optimization | Phase 3 |
| cache diagnostics | Phase 13 |
| cache stability on runtime changes | Phase 3 |
| compaction maturity | Phase 8 |
| compaction lifecycle | Phases 1, 8 |
| context inspection | Phase 13 |
| skill context-cost analysis | Phase 10 |
| universal context budgeting | Phase 3 |
| persistent agent memory | Phase 4 |
| resume/tool-result fidelity | Phase 8 |
| fine-grained permissions | Phase 9 |
| sandbox maturity | Phase 9 |
| dangerous-command analysis | Phase 9 |
| managed enterprise policy | Phase 12 |
| remote control | Phase 11 |
| cloud/cross-client session primitives | Phase 11 |
| IDE depth | Phase 16 |
| diff/workspace visualization | Phase 16 |
| plugin ecosystem maturity | Phases 10, 17 |
| plugin governance | Phases 12, 17 |
| MCP UX/runtime integration | Phase 10 |
| skill ecosystem maturity | Phase 10 |
| custom-agent ecosystem | Phase 4 |
| model-aware worker selection | Phases 3, 4, 7 |
| structured worker output | Phase 7 |
| resumable background workers | Phases 5, 8 |
| interrupt propagation | Phase 2 |
| concurrent-session handling | Phases 8, 11 |
| session grouping/indexing | Phase 11 |
| production telemetry | Phases 13, 14 |
| battle hardening | Phase 14 |
| multi-platform consistency | Phases 11, 16 |
| enterprise authentication policy | Phase 18 |
| network/proxy robustness | Phase 18 |
| agent-state observability | Phases 2, 13 |
| first-class runtime object model | Phase 2 |
| generalized output virtualization | Phase 3 |
| developer ecosystem/distribution | Phases 10, 17 |

No competitive gap in the audit is intentionally left without an implementation phase.

---

# 24. Research Baseline Used for This Plan

Davinci source baseline reviewed:
- `AGENTS.md`
- `Cargo.toml`
- `crates/davinci-agent/src/lib.rs`
- `crates/davinci-agent/src/events.rs`
- `crates/davinci-agent/src/subagent.rs`
- `crates/davinci-agent/src/jobs.rs`
- `crates/davinci-agent/src/compaction.rs` entry points
- `crates/davinci-coding-agent/src/hooks.rs`
- `crates/davinci-coding-agent/src/native_extensions/*`
- `docs/ecosystem.md`
- current `main` ecosystem CI evidence

Claude Code official reference areas:
- Hooks lifecycle: `https://code.claude.com/docs/en/hooks`
- Subagents: `https://code.claude.com/docs/en/sub-agents`
- Agent teams: `https://code.claude.com/docs/en/agent-teams`
- Dynamic workflows: `https://code.claude.com/docs/en/workflows`
- Permissions: `https://code.claude.com/docs/en/permissions`
- Plugins: `https://code.claude.com/docs/en/plugins`

Implementation should re-check those references before each relevant phase because Claude Code behavior continues to evolve.
