# Graph Engineering Hardening Design

## Status

Approved architecture for hardening Davinci's native Graph Engineer runtime.

This design is based on the current `main` branch implementation under
`crates/davinci-coding-agent/src/native_extensions/graph/` and the research report
*Graph Engineering in the Vibe Code Community*. The implementation plan must preserve the
existing product behavior unless this document explicitly changes it.

## Goal

Turn Davinci's existing Graph Engineer from a strong graph-shaped coding pipeline into a
first-class, inspectable, replay-safe, failure-tested execution graph whose topology, state,
contracts, provenance, authority, retries, verification, and observability are explicit and
testable.

The runtime must remain practical for coding work: deterministic where deterministic logic is
enough, model-driven only inside bounded worker nodes, parallel for low-coupling discovery,
single-writer for repository mutation, and externally verifiable before completion.

## Why This Work Exists

Davinci already implements many of the right graph-engineering ideas:

- deterministic Rust control flow outside model workers;
- isolated child-process workers;
- schema-validated artifacts at worker boundaries;
- separate classifier, research, planning, writing, verification, and review responsibilities;
- read-only parallel investigation;
- a single mutation writer;
- deterministic test verification;
- bounded revision and replan loops;
- persisted run state and artifact files;
- resume support;
- per-role tool and shell policies;
- cost, worker, retry, and timeout controls;
- explicit abort behavior.

The remaining problem is not to add more agents. It is to make the execution graph itself the
runtime's source of truth rather than leaving important topology, replay semantics, code-change
provenance, and failure behavior implicit in controller procedure.

## Design Principles

1. **Explicit topology over procedural topology.** If a dependency or transition matters to
   correctness, it must exist as graph data and be inspectable.
2. **Determinism where possible.** Routing, dependency satisfaction, verification pass/fail,
   budgets, retries, and lifecycle transitions are code decisions, not model judgments.
3. **Typed boundaries.** Node inputs and outputs are contracts. A transition may occur only when
   the upstream output satisfies the downstream contract.
4. **State outside transcripts.** Operational state is durable structured data. Worker prose is
   diagnostics, not execution truth.
5. **Parallel reads, single writer.** Discovery may fan out when branches are independent; code
   mutation remains single-threaded.
6. **External verification.** A model never decides that deterministic tests passed.
7. **Replay must prove compatibility.** Previously completed work is reused only when its inputs,
   graph contract, configuration, and repository state are compatible.
8. **Review must cover the graph's actual mutation.** Pre-existing workspace edits must not be
   mistaken for graph-owned changes, and no changed region may disappear behind context
   truncation.
9. **Budgets are semantics, not hints.** A configured wall-clock deadline, worker cap, retry cap,
   or cost cap must be enforceable and testable.
10. **Least privilege is per node.** Capabilities are granted to the smallest role that needs
    them.
11. **Observable execution.** Operators must be able to reconstruct which graph ran, which nodes
    and edges executed, what artifacts moved between them, what was retried, and why the run
    ended.
12. **No complexity without evidence.** New graph machinery must have tests or measurements that
    demonstrate why it exists.

## Scope

This design changes the native Graph Engineer subsystem and its tests, persistence format,
status data, and configuration semantics where required.

Primary implementation area:

```text
crates/davinci-coding-agent/src/native_extensions/graph/
```

Supporting changes may include:

```text
.github/workflows/
crates/davinci-coding-agent/tests/
docs/
```

## Non-Goals

- Replacing the normal single-agent Davinci loop with Graph Engineer.
- Turning every coding task into a multi-agent task.
- Introducing a third-party graph framework such as LangGraph.
- Allowing multiple concurrent writers against one working tree.
- Letting an LLM control retry budgets, dependency resolution, verification pass/fail, or final
  state transitions.
- Automatically committing, pushing, resetting, checking out, or otherwise mutating Git state.
- Building a knowledge graph or GraphRAG subsystem as part of this project.
- Rewriting unrelated Davinci agent-core, memory, governor, security-scan, or TUI architecture.

## Current Architecture

The present pipeline is conceptually:

```text
classify -> investigate -> plan -> implement -> verify -> review
```

Investigation may fan out across research workers. Verification and review can send work back to
the writer. A writer can invalidate a plan and trigger replanning. Complex tasks may be decomposed
into milestones.

The current graph state already contains task IDs, roles, expected artifact kinds, dependencies,
status, attempts, usage, timestamps, and artifact paths. The controller enforces a deterministic
state machine, while model calls are isolated in worker children.

That foundation remains. The hardening project makes the missing graph semantics first-class.

## Confirmed Correctness Risks to Fix Before Architectural Expansion

### Schema and validator drift

`artifact_schema()` and the handwritten validator do not currently have identical requirements.
One concrete example is the classification `milestones` field: the advertised schema marks it
required while the runtime validator accepts it missing or null.

**Required behavior:** one contract must define both advertised and runtime acceptance semantics.
Schema/validator drift must fail tests.

### Empty evidence references

Evidence findings require a `refs` array, but an empty array currently satisfies the general
string-array check.

**Required behavior:** each evidence finding must carry at least one valid evidence reference, or
future typed evidence must explicitly declare another source kind that provides equivalent
provenance.

### Numeric budget truncation

Integer budgets are read through floating-point conversion and then cast, allowing values such as
`3.7` to become `3` after validation.

**Required behavior:** integer budgets accept only non-negative whole values within the target
integer range. No silent truncation or overflow.

### Stale task error after retry

A task can fail one attempt and later succeed. Final task state must not retain an error from a
superseded failed attempt.

**Invariant:** `TaskStatus::Succeeded` implies `error == None`.

### Wall-clock deadline semantics

A run deadline checked only before node starts is not a true run deadline when a silent worker can
remain active beyond it.

**Required behavior:** a configured run deadline actively aborts the running worker process tree
when the deadline expires.

### Reviewer diff provenance

Review currently derives changes from the working tree against Git `HEAD`. A dirty workspace can
therefore mix user changes, previous graph changes, and current writer changes.

**Required behavior:** review is based on a run-owned mutation baseline and a deterministic delta
captured around writer execution.

### Large-diff review truncation

A fixed context truncation may prevent the reviewer from seeing part of a large change.

**Required behavior:** no run may be approved unless every graph-owned changed file or patch chunk
has review coverage. Context reduction may summarize or chunk evidence, but may not silently omit
coverage.

## Target Execution Graph

The normal non-trivial graph is:

```text
                          +------------+
                          |  CLASSIFY  |
                          +-----+------+
                                |
                                | classification
                                v
                  +----------------------------+
                  |       INVESTIGATION        |
                  |                            |
                  | code ----+                 |
                  | tests ---+--- parallel     |
                  | docs ----+    read-only    |
                  | history -+                 |
                  +-------------+--------------+
                                |
                                | evidence set
                                v
                          +------------+
                          |    PLAN    |
                          +-----+------+
                                |
                                | plan
                                v
                          +------------+
                    +---->|   WRITER   |
                    |     +-----+------+
                    |           |
                    |           | patch report
                    |           v
                    |     +------------+
                    |     |   VERIFY   | deterministic
                    |     +-----+------+
                    |       fail| |pass
                    +-----------+ v
                            +------------+
                       +--->|   REVIEW   |
                       |    +-----+------+
                       | changes  | approve
                       +----------+
                                  v
                            +------------+
                            |    DONE    |
                            +------------+
```

A plan-invalidation edge returns from writer output to a new plan node. Budget exhaustion, abort,
contract failure, unrecoverable worker failure, or unverifiable changes lead to explicit terminal
`BLOCKED` or `CANCELLED` states.

Trivial mode may use a smaller graph, but its topology must still be explicit and validated.

## First-Class Topology

Create a dedicated topology module. The exact Rust representation may evolve during implementation,
but it must model at least these concepts:

```rust
pub struct GraphDefinition {
    pub graph_id: String,
    pub version: u32,
    pub nodes: Vec<NodeDefinition>,
    pub edges: Vec<EdgeDefinition>,
}

pub struct NodeDefinition {
    pub id: String,
    pub kind: NodeKind,
    pub role: Option<Role>,
    pub input_contract: Option<ArtifactKind>,
    pub output_contract: Option<ArtifactKind>,
    pub side_effect_class: SideEffectClass,
}

pub struct EdgeDefinition {
    pub from: String,
    pub to: String,
    pub condition: EdgeCondition,
    pub retry_policy: RetryPolicy,
}
```

Implementation types may use enums rather than strings where practical.

The graph definition is immutable for one run and is persisted before the first executable node
starts.

### Topology invariants

Validation must reject or block a graph with:

- an edge referencing an unknown node;
- a dependency cycle not explicitly represented as a bounded revision/retry transition;
- an unreachable required node;
- a node whose input contract cannot be produced by its predecessors;
- more than one mutation-capable writer that can be active against the same workspace;
- an ordinary outgoing edge from a terminal node;
- a required approval/review path that can be bypassed;
- a verification-success edge that can be taken without deterministic verification evidence.

### Explicit dependencies

Every runtime task must declare meaningful predecessor IDs. In particular:

- research tasks depend on classification;
- plan depends on the investigation join or classification when no investigation exists;
- implement depends on its plan, except the explicit trivial topology;
- verify depends on the corresponding implementation attempt;
- review depends on successful verification;
- a revision implementation depends on the verification or review result that requested it;
- a replan depends on the writer artifact that invalidated the previous plan.

No node is allowed to run merely because procedural controller order happened to reach it.

## Scheduler and Controller Responsibilities

The controller currently combines topology, scheduling, lifecycle, and policy logic. Hardened
responsibilities are separated conceptually:

### Topology

Defines nodes, edges, contracts, and legal transitions.

### Scheduler

Computes the ready frontier from persisted task/edge state and concurrency limits. It never asks a
model what should run next.

### Controller

Executes ready nodes, records outcomes, applies deterministic transition rules, and advances the
run.

### Worker

Executes one bounded model-backed capability with the role-specific tools and submits one typed
artifact.

### Verifier

Executes deterministic verification commands and emits a structured verification artifact.

This separation should reduce growth in `controller.rs`. It is acceptable to implement it
incrementally rather than in one rewrite.

## Verification as a Normal Graph Node

Verification remains model-free. The behavioral rule stays:

> Tests failed is an exit-code decision, not an LLM judgment.

However, verification becomes a normal persisted task/node instead of special run-level state.

A verification task records:

- node/task ID;
- predecessor implementation ID;
- exact command set and provenance of each command;
- command exit codes;
- duration;
- output evidence references;
- skipped-plan-command reason when applicable;
- timeout/abort status;
- overall deterministic result.

**Invariant:** a non-dry-run graph cannot reach an approved terminal state without at least one
non-skipped verification command having run successfully, unless a future topology explicitly
models another deterministic verification mechanism.

## Artifact Contracts and Envelope

Model-generated content and controller-generated provenance must be separated.

Persisted artifacts move toward an envelope concept:

```json
{
  "artifactId": "...",
  "schemaVersion": 2,
  "kind": "review",
  "producer": {
    "runId": "...",
    "taskId": "review-1",
    "role": "reviewer"
  },
  "execution": {
    "graphVersion": 2,
    "model": "provider/model",
    "promptVersion": "...",
    "toolPolicyVersion": "..."
  },
  "inputs": [
    {"artifactId": "...", "contentHash": "..."}
  ],
  "contentHash": "...",
  "createdAt": 0,
  "content": {}
}
```

The exact serialized shape may be introduced with a versioned migration rather than breaking every
call site at once.

### Provenance authority

The controller, not the model, generates:

- artifact ID;
- schema version;
- producer run/task/role;
- graph version;
- configured model identity;
- prompt/briefing version;
- input artifact hashes;
- content hash;
- timestamps.

The model generates only the artifact content allowed by its contract.

### Evidence references

The initial hardening may preserve string references while requiring non-empty arrays. The target
shape should support typed evidence without forcing every source into `path:line` strings:

```rust
pub enum EvidenceRefKind {
    FileLine,
    TestOutput,
    CommandOutput,
    Commit,
    Url,
}
```

Typed evidence is a later task inside this project, not a prerequisite for fixing empty references.

## Persistence and Replay Compatibility

Persistence continues under:

```text
.pi/graph/runs/<runId>/
```

The run directory should contain at least:

```text
state.json
artifacts/
logs/
events.jsonl
```

Additional version/fingerprint files may be embedded in `state.json` rather than stored separately.

### Run fingerprint

A replay-compatible run fingerprint must cover inputs that can change the meaning of a cached node:

- graph definition ID/version;
- artifact schema version;
- Davinci runtime/build version where available;
- repository `HEAD` when in Git;
- relevant dirty-worktree baseline hash;
- graph configuration hash;
- role-to-model mapping;
- role-to-tool/permission mapping;
- prompt/briefing version;
- verification command configuration;
- node-specific input artifact hashes.

The implementation may compute a node-specific compatibility key from this data rather than requiring
one monolithic run hash for every reuse decision.

### Resume rule

A cached artifact is reused only when the runtime proves its compatibility predicate. Otherwise the
node reruns and the rejection reason is recorded.

Examples of rejection reasons:

```text
repository_head_changed
graph_version_changed
schema_version_changed
prompt_version_changed
model_mapping_changed
permissions_changed
verification_config_changed
input_artifact_changed
artifact_corrupt
```

Resume must never silently reuse an artifact merely because its task ID matches.

Existing safeguards that avoid replaying superseded plan/write/review artifacts after revision or
replanning must remain unless the new compatibility model provides a strictly safer equivalent.

## Repository Mutation Provenance

Graph Engineer must know what its writer changed independently of `git diff HEAD`.

### Baseline

Before the first writer for a run or milestone mutates the workspace, capture a non-destructive
baseline manifest containing enough information to distinguish:

- clean tracked files;
- pre-existing tracked modifications;
- pre-existing untracked files;
- later graph-created files;
- later graph-modified files;
- later graph-deleted files.

For Git repositories include `HEAD` and status data. File hashes are preferred for files relevant to
the mutation delta.

### Post-writer delta

After each writer attempt, capture a post-write manifest and compute the graph-owned delta against
the immediately preceding graph baseline.

The resulting patch manifest must identify at least:

```text
created_by_graph
modified_by_graph
deleted_by_graph
pre_existing_dirty
unchanged
```

No automatic reset, checkout, stash, add, or commit is allowed.

### Writer report reconciliation

`PatchReport.changedFiles` is a model claim. The controller compares it with the observed mutation
manifest.

- An observed changed file missing from the patch report is added to controller provenance and
  flagged for review.
- A reported changed file with no observed mutation is recorded as a report mismatch.
- The model's report never overrides observed filesystem/Git evidence.

## Complete Review Coverage

The reviewer must evaluate graph-owned mutations, not the entire dirty workspace and not a truncated
prefix of a large diff.

### Small changes

For patches within the direct review context budget, one reviewer may receive the complete graph-owned
patch plus plan and verification evidence.

### Large changes

Large patches are deterministically partitioned by file and, when necessary, by stable chunk. Each
changed file/chunk receives review coverage. Read-only review workers may run in parallel because they
do not mutate the workspace.

A final deterministic aggregation step checks that:

- all required chunks were reviewed;
- no blocker or major issue is hidden by an `approve` verdict;
- all review artifacts correspond to the current patch hashes;
- no changed file lacks coverage.

Only then can the graph take an approval edge.

Context compression is allowed. Coverage loss is not.

## Budget and Deadline Semantics

The existing rule that time/money budgets default to unlimited remains unless configuration says
otherwise. Loop bounds such as revision/replan limits remain positive safeguards.

### Integer budgets

`maxResearchers`, `maxParallelWorkers`, `maxWorkers`, `maxRevisionCycles`, `maxReplans`, timeouts,
and deadlines accept only non-negative whole values in range.

### Cost budget

Cost is monotonically accumulated from worker usage. A configured cost cap may abort an active worker
as soon as reported usage proves the cap is exceeded.

### Run deadline

A dedicated watchdog enforces `runDeadlineMs` even when a worker emits no progress.

At deadline:

1. set the shared execution abort flag;
2. terminate the active worker process tree using existing process-abort mechanisms;
3. record the current task terminal state;
4. finish the run as blocked with a deadline-specific reason unless the operator abort semantics
   require `cancelled`;
5. emit an event and checkpoint.

### Worker accounting

If `maxWorkers` is defined per deliverable/milestone, track that count explicitly rather than deriving
one global allowance by multiplication. Global cost and run-deadline budgets remain global.

## Concurrency Rules

- At most one writer may be active for a project working tree.
- Investigation/review fan-out is bounded by `maxParallelWorkers` and topology-specific limits.
- Parallel workers may share immutable evidence or references.
- Parallel writes to shared graph state must use existing synchronization or explicit reducer/ownership
  semantics.
- Scheduler readiness and worker-budget reservation happen atomically so multiple threads cannot all
  pass a stale capacity check.
- Abort must eventually make every active task terminal.

## Security and Authority

The current two-layer least-privilege model remains the base:

1. per-role tool allowlist;
2. command-text policy for shell access.

Hardening adds finer-grained policy rather than weakening that model.

### Per-role extension tools

Global extra worker tools are too broad for strict least privilege. Configuration should support
role-scoped additional tools. A compatibility path may preserve the existing global setting while
marking it as intentionally broad.

Example target configuration:

```json
{
  "roleExtraTools": {
    "researcher": ["retrieve_output"],
    "writer": ["retrieve_output"]
  }
}
```

### Workspace write boundary

The graph writer may mutate only the project workspace and explicitly approved graph temporary/output
locations. The design does not require a full OS sandbox in the first implementation phase, but tests
must prevent obvious path escape through graph-native write/edit enforcement where the underlying tool
APIs expose path checks.

### Network authority

Network-capable tools or shell paths should be representable as an explicit permission class. The
initial implementation may leave existing shell networking behavior unchanged if changing it would
require broader agent-core work, but graph role policy must not silently expand network authority.

### Trust/provenance labels

Artifacts and evidence should distinguish trusted local deterministic evidence from untrusted or
model-derived material. Candidate source classes include:

```text
repository
deterministic_test
external_tool
external_mcp
web
user_input
model_generated
```

A later security task can attach policy to these labels. This project must at least preserve the
metadata necessary to do so.

## Append-Only Event Log

Add:

```text
.pi/graph/runs/<runId>/events.jsonl
```

Events are append-only diagnostics/audit data; `state.json` remains the current materialized snapshot.

Representative events:

```json
{"seq":1,"event":"run_started","graphVersion":2}
{"seq":2,"event":"node_started","node":"classify","attempt":1}
{"seq":3,"event":"artifact_accepted","node":"classify","contentHash":"..."}
{"seq":4,"event":"edge_taken","from":"classify","to":"research-1"}
{"seq":5,"event":"node_failed","node":"research-1","reason":"timeout"}
{"seq":6,"event":"node_retry","node":"research-1","attempt":2}
```

Each event must have a stable sequence within a run and enough identifiers to correlate run, node,
artifact, and transition.

Logging failure must not corrupt successful execution state. If event append fails, status should make
that observability degradation visible when practical.

## Observability Requirements

For a completed or active run, the runtime should be able to answer:

- Which graph ID/version ran?
- Which nodes were instantiated?
- Which nodes ran, in what order, and with how many attempts?
- Which edges were taken and why?
- Which tasks were reused on resume and why were they considered compatible?
- Which cached tasks were rejected and why?
- Which artifacts entered/exited each node?
- Which model and role/tool policy ran for each model-backed node?
- What verification commands ran and what were their exit codes?
- Which graph-owned files changed?
- Was every change reviewed?
- Which retries/replans/revisions occurred?
- What did each node cost and how long did it take?
- Why did the run reach done, blocked, or cancelled?

Existing TUI/rendering work may consume this data later; UI redesign is not required by this spec.

## Testing Strategy

The project adopts layered tests rather than relying on end-to-end model runs.

### Unit tests

Test deterministic modules directly:

- artifact/config validation;
- topology validation;
- dependency satisfaction;
- budget parsing;
- fingerprint compatibility;
- provenance delta calculation;
- verification collection and pass/fail logic;
- shell/tool policies;
- event serialization;
- review coverage aggregation.

### Contract tests

Assert that:

- advertised schemas and runtime validators agree;
- artifact envelopes decode under the correct schema version;
- edge output/input contracts are compatible;
- task success implies a valid artifact and clean terminal state;
- persisted state round-trips.

### Failure-injection tests

Use fake workers and fake verification executors. These tests must not require paid model calls.

Required worker cases:

- process exits zero without an artifact;
- process exits non-zero before artifact;
- valid artifact is written then process exits non-zero;
- malformed artifact;
- duplicate submission;
- timeout before submission;
- silent/hung worker;
- abort during work;
- usage progress followed by crash.

Required controller cases:

- classifier fails twice;
- one investigation branch fails;
- all investigation branches fail;
- planner fails;
- writer invalidates plan;
- replan budget exhausted;
- verification fails then revision passes;
- verification continuously fails;
- review requests changes then approves;
- review says approve while carrying blocker/major issue;
- abort during every phase.

Required persistence/replay cases:

- interrupted run after task start;
- artifact written before checkpoint;
- malformed/truncated state;
- missing artifact;
- corrupted artifact;
- unknown graph/schema version;
- repository `HEAD` changed;
- dirty file changed;
- graph config changed;
- role model changed;
- prompt version changed;
- permission mapping changed;
- verification command configuration changed;
- previous revision/replan run;
- previous dry run.

Required repository cases:

- clean Git repository;
- dirty repository before run;
- pre-existing untracked file;
- writer creates untracked file;
- writer modifies tracked file;
- writer deletes file;
- binary file;
- patch larger than direct reviewer context;
- multiple milestones touching one file;
- non-Git project.

Required budget cases:

- exact worker cap;
- exact cost boundary and overage;
- silent worker exceeds run deadline;
- revision cap;
- replan cap;
- parallel workers racing for the last worker slot.

## State-Machine / Property Tests

Introduce randomized deterministic state-machine tests once the first-class topology and scheduler
exist.

Generate combinations of worker success/failure, verification failure, review rejection, timeout,
abort, revision, replan, and resume. Assert these invariants continuously:

1. never more than one active writer;
2. no node starts before required predecessors are satisfied;
3. succeeded task has a valid artifact and no final error;
4. failed/cancelled task cannot satisfy a dependency;
5. done non-dry-run graph has valid deterministic verification evidence;
6. required review cannot be bypassed for non-trivial topology;
7. approve plus blocker/major issue cannot reach done;
8. blocked/cancelled state never returns to running;
9. incompatible cached artifacts are never accepted;
10. every graph-owned mutation has review coverage;
11. worker count equals reserved/spawned attempts under the documented accounting model;
12. cost never decreases;
13. revision/replan counters never exceed their limits;
14. abort eventually makes every active task terminal;
15. every accepted artifact identifies its producer and compatible inputs.

## Evaluation Harness

Mechanics can pass while the topology is still inefficient. Build a benchmark suite using isolated
fixture repositories.

Task categories should include:

- one-line bug;
- multi-file bug;
- missing test coverage;
- refactor;
- ambiguous root cause;
- configuration/dependency bug;
- large repository search task;
- documentation-dependent task;
- repository with failing baseline tests;
- repository with irrelevant pre-existing dirty changes.

Compare at least:

```text
normal single-agent loop
/graph --simple
/graph
/graph --complex
```

Measure:

```text
task success
test success/regressions
unrelated-file mutations
input/output tokens
estimated cost
wall-clock latency
workers spawned
parallel width
revision cycles
replans
review issues caught
false review issues
resume reuse/rejection counts
```

A graph topology is not considered better merely because it spends more tokens or workers. Evaluation
must report quality together with cost, latency, and reliability.

## File Responsibility Target

The implementation should move toward this decomposition without forcing a single large refactor:

```text
graph/
├── mod.rs
├── types.rs
├── topology.rs       # graph, node, edge definitions + graph validation
├── scheduler.rs      # ready frontier, dependency/concurrency scheduling
├── controller.rs     # lifecycle orchestration and transition application
├── fingerprint.rs    # replay compatibility
├── provenance.rs     # repository mutation baseline and delta
├── events.rs         # append-only run event log
├── store.rs          # snapshot/artifact persistence
├── validate.rs       # artifact/config validation
├── verify.rs         # deterministic verification
├── config.rs
├── roles.rs
├── worker.rs
├── worker_hooks.rs
├── process.rs
├── briefings.rs
└── render.rs
```

`controller.rs` should not absorb every new responsibility. New modules are justified only when they
own one independently testable concern from the list above.

## Compatibility and Migration

The current run format uses version `1`. First-class topology/artifact-envelope changes require an
explicit version strategy.

Rules:

- New runtime code must never interpret an incompatible old state as current state silently.
- Existing version-1 run history may remain readable for status if practical.
- Resume from version-1 runs may be refused with a clear compatibility reason rather than attempting
  unsafe replay.
- Configuration changes should preserve existing defaults unless this design explicitly changes
  semantics.
- Existing `/graph`, `/graph-status`, `/graph-view`, `/graph-resume`, `/graph-abort`, `graph_run`, and
  `graph_status` interfaces remain available.
- Existing single-writer and no-Git-mutation guarantees remain.

## Delivery Sequence

The implementation should be split into independently reviewable changes.

### Change 1: correctness baseline

- add CI for graph-focused tests;
- enforce schema/validator parity;
- reject empty evidence refs;
- reject fractional/overflow integer budgets;
- clear stale errors on successful retry;
- add regression tests for each issue.

### Change 2: first-class topology

- add topology types;
- persist graph definition/version;
- make dependencies explicit;
- add topology validation;
- represent verification as a task/node;
- add graph invariant tests.

### Change 3: replay-safe persistence

- introduce schema/version metadata;
- introduce controller-generated artifact provenance;
- implement compatibility fingerprint/key;
- reject stale/corrupt replay with recorded reasons;
- add resume fault tests.

### Change 4: precise mutation provenance and review coverage

- capture pre-writer baseline;
- compute graph-owned post-writer delta;
- reconcile observed changes with patch report;
- review graph-owned patch rather than `HEAD` diff;
- chunk large review inputs deterministically;
- require complete coverage before approval.

### Change 5: deadline, scheduling, and concurrency hardening

- enforce wall-clock deadline with watchdog;
- formalize worker reservation/accounting;
- add scheduler module as justified by topology execution;
- add abort/concurrency/race failure-injection tests.

### Change 6: authority and observability

- add role-scoped extra tools;
- preserve workspace write boundaries;
- add trust/provenance labels needed by future policy;
- add append-only event log;
- expose richer graph status fields.

### Change 7: evaluation suite

- add fixture repositories or generated fixture projects;
- compare loop/simple/default/complex graph topologies;
- report task quality, cost, latency, and reliability together.

Each change must leave the workspace buildable and its graph tests passing.

## CI Requirements

At minimum, automated CI should run:

```text
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

Graph-specific test filters may be added for faster feedback. Include Windows coverage for persistence,
process, path, and shell-policy behavior that is platform-sensitive. Linux remains the primary full
workspace lane unless repository CI constraints require a different split.

## Acceptance Criteria

The Graph Engineer hardening project is complete when all of the following are true:

- [ ] Executable topology is persisted before graph execution starts.
- [ ] Every meaningful runtime transition is represented as an explicit edge or deterministic terminal transition.
- [ ] Every meaningful task dependency is explicit and validated.
- [ ] Verification is represented as a persisted deterministic graph node/task.
- [ ] Malformed graph topology is rejected deterministically.
- [ ] Advertised artifact schema and runtime validation cannot drift unnoticed.
- [ ] Evidence findings cannot claim evidentiary status with an empty provenance set.
- [ ] Integer budget fields cannot silently truncate, overflow, or accept negative values.
- [ ] Successful task state cannot retain a superseded attempt error.
- [ ] A configured run deadline terminates a silent/hung worker within bounded shutdown grace.
- [ ] Resume reuses artifacts only after deterministic compatibility checks.
- [ ] Resume records why an artifact was reused or rejected.
- [ ] Every accepted artifact can identify its producer, schema/graph version, and content hash.
- [ ] Pre-existing workspace changes are distinguishable from graph-owned mutations.
- [ ] `PatchReport.changedFiles` is reconciled against observed mutation evidence.
- [ ] Reviewer input is derived from graph-owned changes, not the entire `HEAD` diff.
- [ ] Every graph-owned changed file/chunk receives review coverage before approval.
- [ ] Large-patch context limiting cannot silently hide unreviewed code.
- [ ] At most one writer can be active per project working tree.
- [ ] Abort/deadline leaves no live graph worker process tree after shutdown grace.
- [ ] Fault-injection tests cover worker, controller, persistence, replay, repository, and budget failures.
- [ ] State-machine/property tests enforce the listed graph invariants.
- [ ] Run state plus event log can reconstruct nodes, attempts, edges, artifacts, retries, costs, and termination reason.
- [ ] Linux CI passes formatting, clippy, and workspace tests.
- [ ] Windows CI exercises graph persistence/process/path-sensitive behavior.
- [ ] Evaluation compares quality together with tokens/cost/latency/reliability.
- [ ] Every graph bug discovered during hardening receives a permanent regression test.

## First Seven Regression Tests

Before broad refactoring, write these tests because they probe the highest-risk contract and state
boundaries:

1. Classification schema and runtime validation agree on missing `milestones`.
2. Evidence with `refs: []` is rejected.
3. Failed attempt followed by successful retry ends as `Succeeded` with no final error.
4. Silent worker is terminated by `runDeadlineMs`.
5. Pre-existing dirty file is not attributed to the graph writer.
6. A graph-owned change beyond the current direct-review context limit still receives review coverage.
7. Repository/config/prompt compatibility changes prevent stale artifact reuse.

These tests establish the behavioral baseline that the later topology refactor must preserve.

## Implementation Constraints

- Rust remains the graph runtime implementation language.
- Do not introduce a graph framework dependency unless a later benchmark proves a concrete need.
- Reuse existing worker, process-tree abort, store, verification, and role-policy primitives where they
  satisfy this design.
- Prefer deterministic fake workers in tests over real model calls.
- Do not weaken permission checks to simplify tests.
- Do not mutate Git state automatically.
- Do not conflate execution-graph work with vector memory or knowledge-graph features.
- Keep every intermediate change independently testable and reviewable.

## Final Architectural Invariant

Davinci Graph Engineer should be explainable as:

> A deterministic, durable execution graph whose bounded model workers exchange validated artifacts,
> whose read-only intelligence may parallelize while repository mutation stays single-threaded, whose
> verification is external and deterministic, and whose replay, authority, provenance, and state
> transitions are explicit enough to test and reconstruct.

If an implementation change makes that statement less true, it does not belong in this hardening
project.