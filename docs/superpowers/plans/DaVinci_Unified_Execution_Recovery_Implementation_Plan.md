# DaVinci Unified Execution and Recovery Implementation Plan

> **For agentic workers:** REQUIRED: Use superpowers:subagent-driven-development (if subagents are available) or superpowers:executing-plans to implement this plan. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Preserve DaVinci's capabilities while making every harness-controlled execution traceable, durably recorded, correctly owned, and safe to recover without blindly duplicating side effects.

**Architecture:** Introduce a versioned operation journal and coordinator inside the active `davinci-agent` runtime. Adapt the existing tool ledger, task journal, transaction coordinator, process supervisor, session stores, graph controller, and worker bindings to that execution authority. Keep their domain-specific safety checks and evidence; remove competing execution/retry decisions incrementally.

**Tech Stack:** Existing Rust 2021 workspace; declared Rust floor 1.83; Serde, UUID, SHA-256, synchronous runtime threads, existing JSONL/domain stores. Proposed journal: the workspace-pinned `rusqlite` dependency with bundled SQLite, behind a bounded writer/coordinator API. No hosted database or required external service.

**Status:** Implementation in progress. Tasks 00-05 are complete and their evidence is recorded below; Tasks 06-23 remain unchecked. The implementation branch is `leonardojeziellopez/unified-execution-recovery-01a0ca95`.

**Repository:** `J12003LPZ/davinci`

**Source snapshot requested during inspection:** `main` at `0d3163c30d528bcddaf9a2ee211a4ba052d7f420`.

**Planning date:** September 22, 2026.

**Plan path:** `docs/superpowers/plans/DaVinci_Unified_Execution_Recovery_Implementation_Plan.md`.

**Requirements basis:** The attached `Pasted markdown(20260922-184349).md`, especially operation identity, intent/effect separation, crash consistency, deterministic recovery, incremental migration, and preservation of existing features. This document plans that work; its unchecked tasks do not claim the attachment's implementation deliverables have already been delivered.

---

## Contents

1. Scope and source-audit findings
2. Current ownership and recovery map
3. Proposed architecture and invariants
4. Journal, identities, lifecycle, and publication
5. Recovery and crash-window matrix
6. Migration, dependencies, and execution rules
7. Implementation tasks 00-23
8. Test matrix, performance, and release gates
9. Requirements traceability and completion report
10. Inspected source references

## 1. Scope and source-audit findings

### 1.1 Target the compiled workspace, not stale architecture names

The inspected root `Cargo.toml` lists the active crates as `davinci-protocol`, `davinci-mcp`, `davinci-session`, `davinci-session-sqlite`, `davinci-ai`, `davinci-agent`, `davinci-tui`, `davinci-client`, `davinci-server`, `davinci-telemetry`, `davinci-evals`, `davinci-coding-agent`, `davinci-parity`, and `davinci-voice`. Its product version is `1.0.71`. The `davinci` executable is declared in `crates/davinci-coding-agent/Cargo.toml`. [S01, S03]

The initial planning note incorrectly attributed inactive `davinci-core`, `davinci-app`, and `davinci-cli` names to the checked-out root `AGENTS.md`. Task 00 confirmed that this file contains workflow guidance and none of those crate names; it has no Rust module map. Preserve its instructions and use Cargo metadata plus source declarations for the active layout. This plan targets the inspected `davinci-agent` / `davinci-coding-agent` execution paths, not an assumed `davinci-core/src/runtime/host` architecture. [S02, S04, S05]

### 1.2 DaVinci already has important pieces of this design

This is a convergence project, not a blank-slate runtime rewrite.

| Source-derived finding | What the plan reuses | What must be unified or extended |
|---|---|---|
| `ToolCallLedger` has normalized arguments, argument digests, persisted replay policy, in-flight reservations, `StartedUnknown`, restart reconciliation, and a persistence-failure latch. | Collision checks, fail-closed persistence behavior, duplicate leader/follower behavior, conservative replay decisions. | Universal IDs, separate logical operations and attempts, richer effects/results, immutable history, and cross-subsystem links. |
| `Agent::prepare_tool_call` / `run_prepared_call` already separate preparation from execution to a degree. Dispatch persists `StartedUnknown` before running tools and rechecks contracts/capabilities. | Existing ordering, security checks, scheduler lanes, existing direct and batch tool paths. | A genuinely pure translation step and one coordinator API used by all relevant entry points. |
| `TaskJournal` has bounded checksummed frames, sequence/schema validation, durable operation receipts, a session binding, and an exclusive writer lease. | Its ownership, deduplication, corruption, and receipt semantics as reference behavior and regression tests. | Stop making execution-derived task state an independent recovery authority after migration. |
| Transactions have workspace/owner/schema checks, atomic record replacement, storage bounds, and platform-aware filesystem handling. | `TransactionCoordinator`, its journal, checkpoints, ownership checks, preimages, and verification details. | Connect each actual mutation and transaction phase to operation IDs; reconcile cross-store crash windows. |
| `transactions/commit.rs` explicitly observes an existing Git commit; it does **not** create one. | `observe_commit` as an evidence probe. | Trace real Git mutations through shell/worktree execution; never mistake commit observation for a Git commit executor. |
| `ProcessManager` checks permissions and managed lifetimes; browser leases include process birth identity and socket-ownership checks. | These checks, the process supervisor, and current authorization at resource boundaries. | Durable launch/control receipts and operation linkage; never replace ownership proof with a PID lookup. |
| Session restoration validates worker lineage, uses worker session leases, and restores a separate authoritative task journal. | Existing identity and lineage validation; conversation truth remains in session storage. | Operation recovery must run before fresh dispatch and before independently rescheduling interrupted work. |
| Graph code has typed failure classification, bounded retry decisions, persisted run state, worker/session modules, and retained artifacts. | Topology, worker isolation, artifact validation, existing retry budgets and binding rules. | A graph retry must consult operation-level effect/recovery evidence before relaunching a worker. |
| `RuntimeBus` already distinguishes decision hooks from observations and catches subscriber panics. | Fail-closed decision hooks and observer isolation. | Durable transition events must not depend on best-effort observer delivery; slow observers also need isolation. |
| The cache contract separates request correlation, conversation ownership, transport leases, and cache identity. | All of those distinctions. | New operation IDs and timestamps must remain out of stable model-visible prompt prefixes. |

Evidence: [S06-S18, S20-S22, S27-S28]. These are targeted source observations, not claims that every current path is faulty or that all existing safeguards have been dynamically verified.

### 1.3 Audit boundaries and explicit unknowns

Task 00 reopened the tool ledger/dispatch, runtime task transport/workflow/worktree/session/transaction, process/job, graph controller/process/worker/binding/session/control, browser, MCP/web, native/custom extension, hook, and active workflow entry points. The caller, current gate, adapter, store, result path, and recovery owner are recorded in `docs/runtime/operation-coverage.md`.

The audit did **not** execute the application, reproduce a crash, runtime-validate every platform helper or optional browser/MCP/extension callback, or prove the entire repository is free of bypasses. Arbitrary shell descendants and opaque custom/remote effects remain outside direct observation. These are explicit coverage boundaries, not permission to invent an implementation.

Baseline command outcomes and active workflow job identifiers are recorded in `docs/runtime/execution-recovery-evidence.md` and `docs/runtime/execution-authority-audit.md`. The read-only GitHub repository policy query found no branch protection or repository rulesets, so no required CI contexts were discoverable at this snapshot; workflow job identifiers are not assumed to be required checks.

### 1.4 Scope limits that prevent false guarantees

The journal records harness-controlled requests, processes, mutations, approvals, attempts, results, and evidence. It is not an operating-system syscall tracer. Arbitrary shell programs, custom extensions, and remote servers can perform effects that DaVinci cannot enumerate or reverse. Represent that uncertainty explicitly under the originating operation.

Do not promise universal exactly-once external execution. The achievable contract is durable intent, one admitted local attempt under a valid owner, deduplicated commands/results, and **no automatic re-execution of an ambiguous non-idempotent effect**. A remote idempotency key is useful only where the remote endpoint actually supports and enforces it.

## 2. Current ownership and recovery map

### 2.1 Existing owners and target responsibility

| Domain | Inspected/current surface | Target responsibility |
|---|---|---|
| Tool invocation | `davinci-agent/src/tool_ledger.rs`, `turn.rs`, `batch.rs` | Compatibility facade and wire-call mapping into the operation coordinator; no separate retry authority for migrated operations. |
| Runtime identity | `runtime/mod.rs`, exported `runtime/ids.rs` | Preserve `RunId`, `AgentId`, task identities, parent identity, and cancellation; add operation context rather than replacing these IDs. |
| Task state and control | `runtime/task_store.rs`, `runtime/control.rs`, module `runtime/tasks.rs` | Task definition, dependency/claim policy, generations, and domain receipts remain local responsibilities. Execution-derived status is a versioned projection of operation outcomes. |
| Conversation/session | `davinci-session/src/lib.rs`, `runtime/session.rs` | Authoritative conversation rows and lineage; durable, deduplicated operation-result projection. Never infer successful effects from text. |
| Transaction details | `runtime/transactions/` | Authoritative file transaction records, preimages, apply/rollback details, and workspace checks. Operation journal owns execution/recovery decisions and references these receipts. |
| Process facts | `process_manager.rs`, `jobs::supervisor` | Authoritative live OS ownership and exit observations. Journal records durable attempt identity and observations, not fictitious live handles. |
| Graph orchestration | `native_extensions/graph/{mod,recovery,store}.rs` and declared controller/binding modules | Authoritative topology and policy; execution status/attempt links come from the operation journal. |
| Verification | `transaction_verification` and `runtime/{evidence,evidence_store,completion}` modules; graph verification | Evidence producers/validators. Journal records exact evidence references and which operation/attempt/source revision they verify. |
| Observation | `runtime/bus.rs`, session runtime log, TUI/telemetry | Rebuildable observations and presentation; never the only copy of correctness-critical state. |
| Provider/cache | `davinci-ai`; cache contract | Provider protocol, authenticated continuation, and cache behavior remain separate from execution ownership. |

### 2.2 Overlaps to eliminate without deleting capabilities

A tool can currently have a ledger outcome, a conversation result row, transaction state, a command receipt, and graph/task status. Those describe related facts, but they are not one atomic commit. A failure between stores can leave different descriptions of the same attempt. The work is to establish explicit authority and recovery order across those boundaries, not to erase useful domain evidence.

Retain separate names for **execution result**, **verification result**, **conversation delivery**, and **orchestration completion**. For example:

```text
operation execution: succeeded; result durably recorded
filesystem transaction: applied; postimage recorded
verification: failed on source manifest M
session delivery: pending
parent graph task: blocked by verification
```

That is a coherent state, not corruption. A misleading state would be changing the successful file mutation to “never ran” because session delivery failed, or relaunching it because the graph did not receive the completion notification.

## 3. Proposed architecture and invariants

### 3.1 Keep policy and execution adapters; centralize admission and recovery

```text
Provider tool call / host command / graph control / subagent request
    |
    v
Validate syntax and resolve trusted caller context
    |
    v
Pure OperationPlanner -> immutable operation specification
    |
    v
OperationCoordinator.admit() -> durable intent and stable identity
    |
    v
Existing permission, task-contract, trust, and decision-hook checks
    |
    v
Durable attempt claim + revalidated authority + effect-start latch
    |
    v
Existing resource adapter / transaction coordinator / supervisor
    |
    v
Durable raw result + evidence references + transition event
    |
    v
Verification and result-presentation steps, separately recorded
    |
    v
Durable outbox -> session / graph / task / UI projections
```

Pure planning must not run user hooks, spawn processes, fetch a URL, write checkpoints, or mutate registries. It may receive already-resolved trusted context and normalize input. Permissioned filesystem observations needed for preconditions occur through an admitted preparation/probe step, not hidden I/O in a “pure” translator.

A low-level executor receives a non-serializable, host-minted `ExecutionPermit`, not permission inferred from model JSON. The permit binds the operation/attempt, owner generation, approved payload digest, effective workspace, and relevant policy/contract revision. It is not a replacement for current authorization at the resource boundary.

### 3.2 Location and dependency direction

Add the operation subsystem to **`crates/davinci-agent/src/runtime/operations/`**, exported from the existing runtime. Keep CLI commands in `davinci-coding-agent`; do not put journal code in TUI or create a second top-level agent engine.

Proposed new modules:

```text
operations/
  mod.rs                 public facade and narrowly scoped traits
  model.rs               durable records and schema versions
  identity.rs            caller keys, payload digests, wire-call mapping
  transitions.rs         checked state reducer
  store.rs               SQLite transactions and bounded reads
  migrations.rs          journal schema changes and legacy adapters
  coordinator.rs         admission, claims, execution, finalization
  planner.rs             pure translation to operation specifications
  recovery.rs            deterministic decision reducer
  outbox.rs              durable delivery and consumer receipts
  inspector.rs           read-only query/report model
  failure.rs             typed causal failures and redaction
  fault.rs               injected test fault points
  adapters/              boundary adapters, not duplicate resource engines
```

Add `rusqlite.workspace = true` to `davinci-agent/Cargo.toml`; the dependency is already pinned at the workspace level, but it is not currently a dependency of `davinci-agent`. [S01, S19]

**Alternative considered:** generalize the task journal's append-only framing for all operations. That would reuse proven local mechanisms, but would also require new indexing, atomic multi-record batches, outbox bookkeeping, and schema migration machinery. The proposed SQLite store keeps those concerns together. Reuse task-journal invariants and tests rather than lifting its domain format unchanged. This is a design choice, not a claim that the existing task journal is defective.

### 3.3 Journal ownership and worker access

Use one journal namespace per root conversation/orchestration tree, with multiple run IDs allowed within that namespace. Store namespaces in a shared, private, workspace-bound physical database so conflicting cross-root resource claims can be committed atomically with intent. Sharing a database does not share root authority or conversation ownership. A detached graph run receives an explicit root descriptor; it must not accidentally inherit an unrelated session namespace merely because the working directory matches.

The trusted root host opens the journal and holds an exclusive coordinator lease for its root namespace using the repository's platform-aware ownership pattern. Independent roots have independent leases; schema migration additionally requires exclusive journal-wide migration ownership. In-process children share a coordinator handle. Out-of-process workers submit scoped operation commands through an authenticated, parent-bound transport, extending the existing task-transport boundary after inspection. Do not give model-controlled arguments the journal path or unrestricted database access.

Persist the physical `JournalId`, root namespace identity, workspace identity, root lineage, and binding version in a root descriptor. Workers inherit validated references, not a freshly generated journal per retry. A worker must not become an independent coordinator on resume. The journal location belongs in private runtime/session storage; resolve it using existing directory policy, not a model-supplied path or a blindly trusted repository symlink.

A lease timeout is a liveness hint, **not proof an old process is dead**. Fencing can prevent stale journal writes, but cannot retroactively fence an arbitrary shell command or remote service. Before a replacement attempt, prove the prior owner quiescent or require reconciliation/manual intervention.

### 3.4 Non-negotiable invariants

1. No unsafe side effect before durable intent and the durable effect-start latch.
2. Same scoped idempotency key plus different semantic payload is a hard collision.
3. A logical operation ID survives retries; each physical attempt has its own ID and monotonically increasing attempt number.
4. At most one admitted active attempt under the current owner generation. An expired lease alone does not authorize duplicate execution.
5. A committed execution result is immutable. Verification, delivery failure, and later source changes do not erase it.
6. Replaying a committed result is different from executing its body again. Preserve full result structure and provenance.
7. An ambiguous external/process effect never becomes `SafeToRetry` because a timeout elapsed or an error message contains a familiar phrase.
8. No successful session result without a durable result or explicitly labeled evidence-based recovered outcome.
9. Task/graph retries preserve successful sibling work, authority boundaries, workspace binding, and conversation lineage.
10. Journal corruption, unsupported schema, owner mismatch, or loss of required persistence stops new affected execution.
11. Observation failure does not roll back a persisted result; policy/authorization failure still fails closed.
12. IDs, timestamps, diagnostics, and attempt counters do not change stable provider prompt prefixes or masquerade as conversation/cache identity.

## 4. Journal, identities, lifecycle, and publication

### 4.1 Logical operation, attempts, evidence, and delivery are distinct records

The following is an API/schema sketch, not code already present in the repository:

```rust
struct OperationSpec {
    operation_id: OperationId,
    schema_version: u32,
    context: OperationContext,
    caller_key: ScopedIdempotencyKey,
    payload_digest: PayloadDigest,
    kind: OperationKind,
    effects: EffectProfile,
    preconditions: Vec<Precondition>,
    payload: OperationPayload,
}

struct OperationAttempt {
    attempt_id: AttemptId,
    operation_id: OperationId,
    attempt_number: u32,
    owner: ExecutionOwner,
    expected_revision: u64,
    state: OperationState,
    effect_status: EffectStatus,
    authorization: AuthorizationReceipt,
    started_at: Option<Timestamp>,
    finished_at: Option<Timestamp>,
    result: Option<ResultRef>,
    error: Option<OperationFailure>,
}
```

`OperationContext` contains journal/root identity, session ID, existing runtime run ID, parent operation ID, agent ID, optional worker/task/graph IDs, workspace identity, caller type, and original wire tool-call ID. A graph's string run ID is not silently cast to the runtime's typed `RunId`; store an explicit binding.

Persist timestamps and stable sequence numbers. Use journal sequence for accepted-event ordering and timestamps for human display; wall clocks do not establish distributed causality. Store the requesting component and causal parent separately from the physical executor.

`EffectProfile` combines a classification with capabilities such as `supports_idempotency_key`, `supports_postcondition_probe`, `supports_compensation`, and `requires_live_owner`. The initial classes cover read-only, idempotent mutation, reversible mutation, irreversible mutation, external mutation, and process mutation. They are conservative defaults, not automatic retry permissions.

Required operation kinds include tool invocation, shell command, file write/edit/delete/move, Git mutation, process spawn/stdin/signal/termination, transaction apply/rollback/commit observation, verification, graph-worker/subagent launch, graph/task control, browser action, MCP call, and network/custom external action.

### 4.2 Stable idempotency keys and attempts

For provider calls, use the trusted root/session lineage plus the durable assistant-message identity and provider tool-call ID. For a batch child, include the immutable parent operation ID and child index; preserve the batch's original ordering and payload digest. For host controls, persist a command UUID before submission. For worker launch, bind graph/task identity, requested generation, parent authority, and the stable control command.

Do not use only an argument hash: users can intentionally request the same command twice. Do not generate a fresh key on every retry. Do not reuse a key after the user changes the payload; that is a new operation linked by an explicit replacement or corrective-action relationship.

Enforce uniqueness transactionally on `(journal_id, key_scope, idempotency_key)`. Compare semantic digest and trusted context before returning an existing record. Two concurrent deliveries must receive the same operation/result or a typed collision, not two executable permits.

A retry adds an attempt. A corrective command after a failed verification is normally a new operation with a causal link, not a rewritten payload under the original key.

### 4.3 Lifecycle and orthogonal facts

Use an explicit checked reducer. The proposed initial state set is:

```text
Created (memory only)
  -> Persisted -> Authorized -> Queued -> Running -> EffectPossible
  -> Succeeded | Failed

Persisted / Authorized / Queued -> Cancelled (when execution never started)
Running / EffectPossible -> Interrupted -> RecoveryRequired
RecoveryRequired -> retry admission | evidence-based finalization | terminal block
Terminal operation -> Superseded only through an explicit replacement link
```

This is not an unrestricted list of allowed edges. Task 01 supplies the complete transition table and forbidden-edge tests. An interrupted `Running` attempt is retryable only if the effect latch and adapter evidence establish that execution did not begin. `EffectPossible` is committed **before** the unsafe boundary; setting it after the syscall would miss the dangerous crash window.

`Succeeded` means the adapter outcome and durable result are recorded. It does not mean verification passed or that every consumer received the result. Track verification as `NotRequired / Pending / Running / Passed / Failed / Inconclusive`, and delivery per consumer as `Pending / Acknowledged / Failed`. A compact UI may display “Verified,” but it must preserve the underlying successful execution and verification evidence separately.

`Failed` is not synonymous with “no effects.” A nonzero shell exit can still mutate files. Retry eligibility depends on effect evidence and policy, not the state label alone. Cancellation requested while running is a control request, not proof of process exit, rollback, or absence of effects.

### 4.4 Storage contract

Use a single database transaction for each correctness-critical journal update. Proposed tables:

| Table | Purpose |
|---|---|
| `journal_metadata` | Physical journal identity, workspace binding, schema, and migration state. |
| `journal_roots` | Root namespaces, lineage bindings, per-root coordinator generations and leases. |
| `resource_claims` | Operation-linked conflicting-effect claims, updated atomically with admission/recovery. |
| `operations` | Immutable identity/specification plus current checked revision/projection. |
| `operation_attempts` | Attempt identity, owner, dispatch latch, outcome, timing. |
| `operation_events` | Append-only transitions and recovery decisions with monotonic sequence. |
| `operation_results` | Immutable complete structured result, digest, and artifact references. |
| `operation_evidence` | Verification/reconciliation observations and source/attempt binding. |
| `operation_links` | Domain receipt, session row, graph task, process, transaction, and artifact links. |
| `control_receipts` | Stable command ID, request digest, expected generation/revision, response. |
| `outbox` | Durable consumer-specific publication work. |
| `projection_receipts` | Consumer acknowledgement/deduplication bookkeeping. |

Proposed opening policy: local disk, WAL, `synchronous=FULL`, foreign keys enabled, bounded busy handling, restrictive file permissions, verified private-directory ownership, and explicit schema migration. Validate the actual bundled SQLite behavior and filesystem assumptions in tests. Unsupported/network filesystems must not silently inherit local-disk crash guarantees.

A committed intent/attempt is acknowledged only after its database transaction succeeds. Never hold a database write transaction while a shell command, network call, permission dialog, or user hook runs. Batch independent writes where safe; state transitions can share a commit while retaining distinct events. Do not fsync every stdout chunk.

Large outputs and preimages use existing bounded artifact facilities or a dedicated private blob adapter. Write and verify the blob before committing its reference. A missing blob is an integrity/recovery problem, not a successful empty result. Unreferenced blobs can be collected later; referenced or unresolved evidence cannot.

### 4.5 Result persistence, hooks, and outbox delivery

Persist the factual adapter outcome before a post-tool hook can transform it. Preserve both raw result and user/model-facing presentation with a transformation version/digest. A hook veto may block acceptance or parent completion; it must not rewrite “the file was written” into “nothing happened.” Hooks that can perform side effects must themselves pass through operation admission or remain conservatively opaque and non-auto-replayable.

Use one atomic journal transaction to publish a finalized result reference, transition event, and outbox row. Each consumer must apply a stable event ID idempotently. For JSONL sessions, the projection ID belongs in the **same durable session entry** as the projected result; a separate “already delivered” flag written afterward does not solve the crash window.

The publication protocol is:

```text
commit result + outbox
apply sink update with embedded event/operation ID
commit sink's own durable update
acknowledge outbox
```

A crash between the last two lines re-delivers the same event and the sink recognizes it. It never invokes the original executor. Session, graph, and task stores cannot be atomically committed together with an external filesystem mutation; use stable domain receipts and reconciliation, not a fictional cross-store transaction.

## 5. Recovery and crash-window matrix

### 5.1 Deterministic recovery reducer

`RecoveryEngine` is the only component that authorizes another attempt for migrated operations. Resource adapters supply typed observations; graph/task policy can impose additional restrictions but cannot override an unsafe effect decision.

The reducer takes the operation specification, all attempt records, owner/liveness evidence, domain receipts, current precondition observations, authorization status, and the stored recovery-policy version. It emits one of:

- `SafeToRetry`: with the exact evidence and authority required for a new attempt.
- `AlreadyCompleted`: replay a durable result, or finalize a specifically labeled evidence-based outcome.
- `NeedsVerification`: run an authorized probe, not the original mutation.
- `NeedsReconciliation`: gather/compare domain receipts and effects before deciding.
- `NeedsHumanDecision`: stop automatic progress and explain the uncertainty.
- `TerminalFailure`: no automatic retry under this operation.

Persist the decision, reason code, input evidence digests, policy version, and proposed next action. Models can explain the report, but model-generated text must not decide that a side effect is safe to repeat.

### 5.2 Effect-specific default policy

| Action | Default after an ambiguous attempt | Evidence needed for a different decision |
|---|---|---|
| Read a file under current authority | Retry as a new attempt after checking owner/permissions. | Record that the new read may observe a new file revision; do not relabel it as the original observation. |
| Conditional write of exact bytes | Reconcile transaction/preimage/postimage before retry. | Matching operation receipt or a guarded current-state comparison. Never overwrite intervening user edits. |
| Delete/move/rollback | Reconcile both names, identities, and transaction phase. | Proven source/destination/postimage state; reversible does not mean automatically safe. |
| Arbitrary shell command, test, or build | Do not automatically repeat merely because the process ended or timed out. | Proven failure before spawn, an explicitly safe command contract, or human-authorized new action. Tests/builds can mutate state. |
| Managed process spawn | Reconcile supervisor/attempt/owner identity; do not spawn another on a missing response. | Certified not-started result or verified current lifetime with a recoverable receipt. |
| Process stop/stdin/signal | Inspect the exact managed lifetime; no raw-PID action. | Current ownership and typed command receipt. Repeated stdin is potentially mutating. |
| Git commit/push/cherry-pick | No blind replay. | Structured receipt and relevant object/ref/remote evidence. A command string or commit message is not proof. |
| Browser click/type/submit | Reconcile browser/process/page state; otherwise require a decision. | Endpoint-specific evidence. A similar screenshot does not prove a form was not submitted. |
| MCP/network mutation | Block ambiguous replay unless the endpoint's idempotency contract is verified. | Stable supported remote key and response/status lookup, or manual reconciliation. |
| Task/graph control command | Return its durable receipt; do not repeat the transition. | Matching request digest and original generation/revision. A changed command is a collision/new operation. |
| Subagent/worker launch | Reconcile parent-bound launch and child session first. | Valid original binding, quiescent prior owner, and operation recovery clearance. |

Observing the desired file contents can support “postcondition satisfied by observation,” but it does not prove a particular process exited successfully. Use a typed recovered outcome; never invent an exit code, original stdout, transaction owner, or missing successful tool result.

### 5.3 Crash-window matrix

| Crash boundary | Durable state expected | Startup/recovery behavior | Duplicate prevention |
|---|---|---|---|
| Before intent commit | No admitted operation. | Repeat admission from the durable caller command/message. | Stable scoped key; executor has received no permit. |
| After intent, before authorization | Persisted spec, no effect latch. | Reauthorize or leave denied/waiting. | No execution before current authorization. |
| After authorization, before claim | Authorization receipt and intent. | Recheck policy/contract changes, then claim if valid. | CAS revision and coordinator ownership. |
| After claim, before unsafe boundary | Claimed attempt; no effect latch only where that fact is durable. | Prove old owner quiescent; safe new attempt only when boundary was not crossed. | Attempt generation and latch protocol. |
| After effect latch, before syscall | Effect may have started. | Treat conservatively; probe/reconcile. | Never assume the syscall was skipped. |
| After OS spawn, before launch receipt | Intent/latch, potentially live child. | Reconcile supervisor identity and process tree; unresolved means blocked. | No second spawn from a missing acknowledgement. |
| After file effect, before operation result | Intent/latch plus transaction/preimage/domain evidence, if committed. | Compare current identities/hashes and transaction phase. | Receipt/CAS checks; preserve external changes. |
| During multi-file apply or rollback | Operation plus transaction phase and individual path facts. | Use existing coordinator reconciliation; do not claim atomic all-or-nothing across files without evidence. | Stable transaction ID and per-path guarded actions. |
| After external request, before response | Effect-possible attempt. | Query supported status/idempotency endpoint, or ask for a decision. | No generic HTTP/MCP/browser retry. |
| After result commit, before notification | Immutable result and pending outbox. | Replay publication only. | Executor is not called; consumer deduplication. |
| During session append | Journal result plus either valid complete row or rejected/torn session tail. | Validate session; deduplicate valid entry, otherwise explicit repair/recovery. | Embedded event ID and writer ownership. |
| During verification | Execution result remains committed; verification attempt is incomplete. | Retry only an authorized safe probe or reconcile the verification command. | Verification has its own operation/attempt and source binding. |
| During graph-worker completion | Child result/domain receipts, possibly stale graph projection. | Finish validated result publication, then graph projection. | Original launch/binding/generation; preserve siblings. |
| During cancellation/termination | Control intent/receipt; process may remain live. | Retain `Stopping`/unknown until ownership and exit are proven. | Never infer terminated from cancellation request. |
| During migration/schema update | Either pre-migration format or committed new version. | Resume idempotent migration or stop on incompatible/corrupt state. | Exclusive writer, source digests, no mixed-authority execution. |
| During journal write/corruption | Last validated commit or explicit integrity failure. | Stop affected execution; preserve evidence for diagnosis. | No silent truncation, record skipping, or fabricated success. |

## 6. Migration, dependencies, and execution rules

### 6.1 Phases and checkpoints

| Phase | Tasks | Exit condition |
|---|---|---|
| A. Baseline and design contracts | 00-01 | Active paths verified; invariants and compatibility fixtures accepted. |
| B. Durable foundation | 02-04 | Journal, identity, claims, reducer, and initial crash harness pass focused tests. |
| C. Tool dispatch and result publication | 05-06 | One migrated dispatch facade; durable result/outbox/session replay proven. |
| D. Shell/process and file/transaction paths | 07-10 | No unjournaled migrated effect; process and filesystem crash windows covered. |
| E. Verification and orchestration | 11-15 | Evidence-linked completion and parent-bound worker/control recovery. |
| F. Browser/MCP/external paths | 16-17 | Conservative adapters and transport-loss tests; no blanket external replay. |
| G. Observability and inspection | 18-19 | Causal reports and read-only inspectors expose authoritative facts. |
| H. Compatibility and convergence | 20 | Explicit legacy handling; old recovery code is non-owning for migrated families. |
| I. Stress, faults, performance | 21-22 | Cross-platform recovery properties, resource bounds, retention tests. |
| J. Documentation and release | 23 | Required gates pass with evidence; supported capabilities retained. |

Basic inspection and fault hooks begin in Phase B. They are not postponed until the last phase. A small vertical slice must exercise admission -> effect -> durable result -> publication -> restart before additional adapters are migrated.

### 6.2 Migration modes and rollback

Support explicit `legacy`, `shadow`, and `authoritative` modes per operation family during development. In shadow mode, the legacy path is the **only executor and recovery decision maker**; the journal observes and compares. Shadow data must be marked non-authoritative and cannot later be treated as proof of an effect that was not durably admitted.

Once a family has admitted authoritative operations, restarting or disabling a feature flag cannot send those operations to the legacy executor. Persist the ownership mode with the operation/root format. On journal failure, block affected dispatch; do not “fall back for availability.”

Rollback means stopping new admission, preserving journal/domain data, and returning to a compatible reader or a deliberate new legacy session with clearly non-resumable historical records. It does not mean opening an authoritative journal with an old binary and hoping ignored metadata is safe. Test the actual older reader; when a safe downgrade cannot be guaranteed, declare it unsupported and preserve the data.

### 6.3 Implementation discipline

Every task follows a red/green/refactor sequence, adds focused tests, and ends with a small reviewed commit. Listed tests are **new tests to create** unless explicitly described as existing. A test command that discovers zero tests does not satisfy a task. Maintain an evidence ledger with command, platform, commit, test count, exit status, and limitations.

Do not change public tool names, remove capabilities, reset user permissions, disable graph workers, lower budgets, or alter model routing/cache semantics to make the new tests pass. Do not reformat unrelated large modules. Extract narrowly scoped adapters and keep module ownership visible.

After the model/store API stabilizes, browser and MCP adapters can be developed independently; inspector presentation can proceed against fixed query fixtures. Journal, dispatch, session publication, and authority migrations are ordered work and must not be independently reinvented by parallel workers.

## 7. Implementation tasks

### Task 00 — Revalidate the baseline and inventory effect entry points

**Depends on:** Nothing. This is the implementation starting point.

**Existing files to inspect:** Root `Cargo.toml`, `AGENTS.md`, both agent/package manifests, `davinci-agent/src/{lib,turn,batch,tool_ledger,process_manager}.rs`, `runtime/{mod,session,task_store,control}.rs`, `runtime/transactions/`, `davinci-coding-agent/src/{main,args,runtime_host}.rs`, the graph module tree, actual CI workflows, and the public tool registries.

**Create:** `docs/runtime/execution-authority-audit.md`, `docs/runtime/operation-coverage.md`, `docs/runtime/execution-recovery-evidence.md`.

- [x] Fetch the selected branch without modifying user work; record `git status --short`, `git rev-parse HEAD`, and `cargo metadata --no-deps --format-version 1`. Reconcile the compiled crate list with this plan. Record any difference from `0d3163c30d528bcddaf9a2ee211a4ba052d7f420` before rebasing the plan.
- [x] Check the suspected stale crate-layout references in `AGENTS.md` with source evidence. Preserve its instructions; correct this plan because the checked-out file contains no inactive crate names and supplies no Rust module map.
- [x] Enumerate harness-controlled effect families and record caller, gate, adapter, store, result publication, recovery owner, and opaque boundaries in `docs/runtime/operation-coverage.md`.
- [x] Open the declared task transport, workflow, worktree, tools, jobs, graph controller/process/worker/bindings/worker_sessions/control, browser, MCP, and native/custom-tool host adapters. Record the active files and ownership findings in the audit and coverage documents.
- [x] Capture baseline commands, active workflow job identifiers, repository-required-check policy, ignored/flaky/platform-specific test evidence, and regression-selection areas in `docs/runtime/execution-recovery-evidence.md`.
- [x] Add a coverage row for each entry-point family, initially `legacy` or `audit_required`, and mark opaque subprocess/remote effects explicitly.

```bash
git rev-parse HEAD
cargo metadata --no-deps --format-version 1
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

**Acceptance:** No unresolved ambiguity about the active crate graph or command targets. Every known side-effect family has an owner and an audit row. Baseline evidence distinguishes failed, passed, skipped, and not run.

**Commit:** `docs(runtime): map active execution and recovery ownership`

### Task 01 — Introduce operation identity, model, and checked transitions

**Depends on:** 00.

**Modify:** `crates/davinci-agent/src/runtime/mod.rs`, `crates/davinci-agent/src/lib.rs` only for narrow exports.

**Create:** `runtime/operations/{mod,model,identity,transitions,failure}.rs`; `crates/davinci-agent/tests/operation_model.rs`.

- [x] Write red tests for complete context serialization, distinct logical/attempt identity, unknown schema rejection, absent required ownership, valid transitions, forbidden terminal regression, and cancellation that does not imply effects were undone.
- [x] Define `OperationId`, `AttemptId`, `JournalId`, scoped caller key, immutable specification, attempt record, structured result/evidence/link references, effect profile, and typed causal failure. Reuse existing run/agent/task types.
- [x] Define explicit schema versions and the complete transition table. Model verification and publication separately from factual execution outcome. Use a checked revision to prevent stale updates.
- [x] Define structured reason codes for persistence, authorization, collision, owner loss, process identity mismatch, unknown effect, verification, publication, corruption, and unsupported schema. Preserve original errors as causes rather than string-matching them for safety decisions.
- [x] Round-trip old-compatible optional metadata without silently defaulting missing legacy effect certainty to success. Property-test that successful execution cannot transition back to not-started.
- [x] Run focused tests and format; commit only the model and its tests, not partial executor rewrites.

**Evidence:** The pre-implementation run failed because the operations module did not yet exist. After implementation, the focused integration target passed 9 tests, the retry-admission unit test passed 1 test, and rustfmt check passed on all touched Rust files.

**Verify:** `cargo test -p davinci-agent --test operation_model` — new tests are discovered and pass; illegal transitions and missing binding data are rejected.

**Commit:** `feat(runtime): define versioned operation identity and lifecycle`

### Task 02 — Build the durable operation journal

**Depends on:** 01.

**Modify:** `crates/davinci-agent/Cargo.toml`, `Cargo.lock` only as needed for the already-pinned dependency.

**Create:** `runtime/operations/{store,migrations}.rs`; `crates/davinci-agent/tests/operation_journal.rs`.

- [x] Write red tests for atomic intent creation; result/event/outbox atomicity; reopen durability; schema mismatch; root/workspace mismatch; injected write failures; bounded record/queue behavior; and a second coordinator trying to own the same root.
- [x] Add the workspace `rusqlite` dependency and implement schema version 1 with uniqueness constraints, foreign keys, revision checks, and a private-directory opening policy. Validate current workspace identity before opening an existing journal.
- [x] Implement short database transactions behind a bounded coordinator writer. A dispatch barrier waits for durable acknowledgement. Reuse existing platform directory/lease patterns after reviewing them; do not weaken symlink, Windows ACL, or owner checks to accommodate SQLite files.
- [x] Implement append-only event history and immutable results; reject oversized payloads in favor of bounded artifact references. Store corruption or failed durable writes poison further affected admission until reopened and reconciled.
- [x] Add consistent read snapshots and safe backup/export primitives. Do not copy a live WAL database as an isolated main file and call it a complete backup.
- [x] Reopen from another process in tests; verify either the complete committed transaction exists or it does not. Add an explicit integrity-failure path rather than a “skip bad record” fallback.

**Evidence:** The initial red run failed because the journal API did not exist. A later integrity-poison regression test also failed before the fix. After implementation, `cargo test -p davinci-agent --test operation_journal --offline --locked` passed 12 tests, the journal and operation-model integration targets passed 21 combined tests, and `cargo test -p davinci-agent operations::transitions::tests --lib --offline --locked` passed 1 test. The changed Rust files passed targeted rustfmt checking and `git diff --check`. The write-failure path is injected through a SQLite trigger; physical disk-full and power-loss behavior were not exercised. Backup was checked for a standalone database before reopening it.

**Verify:** `cargo test -p davinci-agent --test operation_journal`.

**Acceptance:** No API returns a usable mutation permit on a failed intent/dispatch commit. There is one authoritative root binding and one admitted coordinator owner.

**Commit:** `feat(runtime): add durable operation journal and schema guards`

### Task 03 — Implement idempotent admission, attempts, and owner fencing

**Depends on:** 02.

**Create/modify:** `runtime/operations/{coordinator,coordinator_api,migrations,mod,model,retry_safety,store,store_api,transitions}.rs`; create `crates/davinci-agent/tests/operation_idempotency.rs` and `operation_ownership.rs`; update journal integration coverage.

- [x] Write red tests for concurrent same-key deliveries, same-key/different-payload collisions, identical arguments under two intentionally different commands, stale revisions, and stale owner generations.
- [x] Implement one transactional `admit` path returning `New`, `ExistingInFlight`, `ExistingResult`, or `Collision`. Persist wire-call/parent/batch-child mappings once.
- [x] Implement attempt claims with monotonic numbers and host-owned dispatch permits. Claiming an attempt must not itself invoke an adapter or synthesize permission.
- [x] Persist the effect-start latch before calling any unsafe boundary. Permit consumption and record transitions must be checked; no public constructor may manufacture an authorized permit from deserialized JSON.
- [x] Add owner-loss handling. A new generation rejects stale writes but cannot automatically rerun a command until old-owner quiescence and effect policy permit it.
- [x] Verify duplicate terminal requests return the full durable result for matching intent; caller-facing access checks are enforced at the facade introduced in Task 05.

**Evidence:** Red runs exposed missing admission/dispatch/owner-fence APIs, missing coordinator-only recovery, acceptance of a v2 database with its application ID cleared, and retry admission without rechecking the durable effect latch. After fixes, the four focused integration targets passed 33 tests; the populated-v1 migration regression and transition unit target each passed 1 test; targeted rustfmt and `git diff --check` passed. The journal replay test verifies the complete stored success payload for matching intent; live user permission checks are a caller-facade responsibility and will be wired in Task 05.

**Verify:**

```bash
cargo test -p davinci-agent --test operation_idempotency
cargo test -p davinci-agent --test operation_ownership
```

**Acceptance:** Concurrent delivery admits one logical operation; a payload collision admits none; a replacement coordinator cannot convert missing liveness into permission to repeat a mutation.

**Commit:** `feat(runtime): enforce operation idempotency and fenced attempts`

### Task 04 — Add deterministic recovery and the first crash-test harness

**Depends on:** 03.

**Create:** `runtime/operations/{recovery,fault}.rs`; `crates/davinci-agent/tests/{operation_recovery,operation_crash}.rs`; `crates/davinci-agent/tests/support/operation_harness.rs`.

- [x] Write table-driven red tests for each recovery classification and every row in the crash-window matrix. Inputs are durable records plus typed observations, not human-readable error substrings.
- [x] Implement a pure reducer and a separate authorized action runner. Persist each decision and its evidence/policy version before scheduling a retry/probe. Persist unresolved-effect resource claims and check them during admission regardless of tool-call ID, wrapper, or root namespace; known scope allows narrow barriers, while opaque workspace effects require conservative conflict blocking.
- [x] Implement a test-only injected fault interface. Initial points: before/after intent commit, after attempt claim, after effect latch, after side effect, before/after result commit, before publication, after sink commit, and before outbox acknowledgement.
- [x] Build a child-process test harness that opens a real temporary journal, performs a counted durable effect, signals the chosen fault boundary, and is forcibly terminated. A fresh process then loads the journal and recovers. Do not substitute a returned Rust error for every crash test.
- [x] Add a separately stored mutation counter and probeable fake endpoint. Assert mutation count, number of attempts, preserved result, recovery decision, and projection count after restart and duplicate delivery.
- [x] Keep production builds free of model-controlled environment fault switches. Where an environment variable is used by a test helper, scope it to the helper process, not process-global parallel tests.

**Canonical red/green scenario:**

```text
Given an opaque mutation with a durable intent and stable caller key,
when the mutation increments a durable counter and the process dies before result commit,
and the same command is delivered after restart,
then the counter stays at one and recovery requires reconciliation/decision.

Given the same mutation with a committed complete result,
when the process dies before publication and the command is delivered again,
then the counter stays at one and only publication is retried.
```

**Verify:** `cargo test -p davinci-agent --test operation_recovery` and `cargo test -p davinci-agent --test operation_crash`.

**Commit:** `test(runtime): add deterministic recovery reducer and crash harness`

### Task 05 — Route direct and batch tool admission through a single facade

**Depends on:** 04.

**Modify:** `davinci-agent/src/{turn,batch,tool_ledger,lib}.rs`, `runtime/mod.rs`; inspect/modify the existing capability registry in `runtime/capabilities` before changing its metadata contract.

**Create:** `runtime/operations/planner.rs`, `runtime/operations/adapters/tools.rs`; `crates/davinci-agent/tests/operation_dispatch.rs`.

- [x] Write red tests proving direct and batch calls use the same admission path, stable child keys survive replay, a planning-only call causes no process/file/network effects, and unknown/custom tools default conservatively.
- [x] Extract pure translation without bypassing the existing `prepare_tool_call`, permission, contract, capability, pre-hook, and lane semantics. Keep source-order mutation barriers and parallel read lanes intact.
- [x] Introduce a compatibility facade for `ToolCallLedger`. For migrated families it delegates execution truth to the journal; for legacy/shadow families its current semantics remain authoritative.
- [x] Pass trusted operation context through `RuntimeHandle` and the tool dispatch context. Preserve context across session continuation and correctly scope it for child agents.
- [x] Recheck the approved payload, workspace, permission revision, task contract, cancellation, and owner at the actual resource boundary. A cached grant or restored permit cannot silently authorize changed work.
- [x] Test that intent/attempt persistence failure invokes no adapter and that a post-admission denial records a truthful non-executed outcome. Keep active public tool names and provider schemas unchanged.

**Verify:** `cargo test -p davinci-agent --test operation_dispatch`, plus existing ledger, scheduler, permission, batch, and contract tests selected in Task 00.

**Commit:** `refactor(agent): unify operation admission for direct and batch tools`

### Task 06 — Persist full results and make session publication replay-safe

**Depends on:** 05.

**Modify:** `davinci-agent/src/{turn,batch,tool_ledger,lib}.rs`, `runtime/session.rs`, `davinci-session/src/lib.rs`. Inspect the already-declared `davinci-session/src/{repo,jsonl_repo,types,codec}.rs` and SQLite session implementation before applying the equivalent deduplication contract to their actual append APIs.

**Create:** `runtime/operations/outbox.rs`; `crates/davinci-agent/tests/operation_publication.rs`; `crates/davinci-session/tests/operation_projection.rs`.

- [ ] Write red tests for crash after result commit, crash after session append/before acknowledgement, duplicate batch-child publication, missing result artifacts, and post-tool-hook failure after a successful mutation.
- [ ] Persist complete raw `ToolResult` content, error status, structured details, artifact/image references, and digest. Do not reduce it to an output string that loses evidence on replay.
- [ ] Record the post-hook/presentation outcome separately. A failing hook or compression step can block delivery but cannot erase the recorded resource effect.
- [ ] Atomically create result-ready events and consumer outbox entries. Add idempotent sink append/application with embedded event IDs and current lineage validation. JSONL and SQLite-backed conversations must agree on the semantics even if their implementation differs.
- [ ] Gate provider continuation on the required conversation projection being durable. A session write failure may pause that conversation, but successful effects remain recorded and cannot be re-executed to regenerate text.
- [ ] Test recovery with a real reopened session. A duplicate event produces one logical result row; missing or corrupt evidence produces an explicit blocked state rather than empty success.

**Verify:** `cargo test -p davinci-agent --test operation_publication`, `cargo test -p davinci-session --test operation_projection`, and equivalent SQLite session tests located during implementation.

**Commit:** `feat(runtime): persist operation results and deduplicate completion delivery`

### Task 07 — Journal foreground shell and supervised process launches

**Depends on:** 06.

**Modify:** The inspected `davinci-agent/src/turn.rs` and `jobs/supervisor/mod.rs`; the source-search-located `command_receipt.rs` and `tools/foreground.rs` after reading them in full. Inspect and modify the supervisor's declared `client`, `helper`, `platform`, and `wire` modules at their actual launch/exit boundaries.

**Create:** `runtime/operations/adapters/process.rs`; `crates/davinci-agent/tests/operation_process.rs`.

- [ ] Write red tests for bash/PowerShell/native exec admission, failed launch before a child exists, crash after spawn/before receipt, partial output, exit observation, and replay after result commit.
- [ ] Bind resolved executable/argv, effective cwd, safe environment references/digest, operation ID, attempt ID, owner generation, and process lifetime into the supervisor protocol. Do not log credential-bearing environment values.
- [ ] Persist the dispatch latch before contacting the spawning helper. Correlate launch acknowledgement and exit evidence to the exact lifetime; a missing acknowledgement is an unknown launch, not a certified non-launch.
- [ ] Attach the existing command receipt to the operation result. Keep shell stdout streaming noncritical while preserving the final output-completeness flag and durable artifact references.
- [ ] Keep arbitrary shell commands and test/build commands conservatively effectful. Do not allow string heuristics such as a `cargo test` prefix to bypass mutation/recovery rules.
- [ ] Run hard-kill/restart tests on each supported OS. Distinguish application-process crash coverage from power-loss/storage-controller guarantees.

**Verify:** `cargo test -p davinci-agent --test operation_process`, plus current foreground, command-receipt, shell-policy, and supervisor tests.

**Commit:** `feat(runtime): journal shell attempts and supervised process launches`

### Task 08 — Journal process control, stdin, cancellation, and termination proof

**Depends on:** 07.

**Modify:** `davinci-agent/src/process_manager.rs`, `runtime/control.rs`, the actual JobBook/managed-process modules identified in Task 00, and supervisor wire/control handling.

**Create:** `crates/davinci-agent/tests/operation_process_control.rs`.

- [ ] Write red tests for duplicate stop commands, repeated stdin, stale process lifetime, PID reuse, revoked permissions, cancellation races, and children that fail to exit.
- [ ] Route process start/write/signal/stop through operation admission and durable control receipts; preserve the exact current managed owner, workspace, session, generation, and lifetime binding.
- [ ] Separate control acceptance from effect outcome: `Accepted` or `Stopping` is not `Stopped`. Only record termination when the supervisor/OS observation supports it.
- [ ] Reject controls against a stale lifetime even when the numerical PID matches. Never replay stdin automatically after a lost response; it may already have triggered a mutation.
- [ ] Reconcile outstanding controls on resume and expose unresolved descendant ownership. Do not clean up unrelated processes to make recovery appear successful.
- [ ] Verify that bounded shutdown waiting reports uncertainty instead of manufacturing safe termination.

**Verify:** `cargo test -p davinci-agent --test operation_process_control`, plus the existing product process-manager integration tests.

**Commit:** `feat(runtime): make process controls durable and ownership-checked`

### Task 09 — Connect filesystem mutations to operation-backed transactions

**Depends on:** 06; integrate with 07 when a file action delegates to a process.

**Modify:** The inspected `davinci-agent/src/turn.rs` and discovered transaction files `runtime/transactions/{mod,model,store,files,tools}.rs`. **Resolve concrete files in Task 00 before editing:** the declared `crate::apply_patch`, `crate::notebook`, `crate::file_mutation_queue`, and `crate::tools` modules; a module declaration alone does not establish whether its file is `name.rs` or `name/mod.rs`.

**Create:** `runtime/operations/adapters/filesystem.rs`; `crates/davinci-agent/tests/operation_filesystem.rs`.

- [ ] Write red tests for write/edit/apply-patch/notebook edits, delete/move, same-path serialization, crash after mutation, same-key delivery, and intervening user changes.
- [ ] Add operation/attempt references to transaction records and per-path receipts. Reuse the coordinator's preimage, metadata, workspace identity, boundary checks, and mutation queue rather than adding a second filesystem engine.
- [ ] Capture/read authorized preconditions and persist required checkpoint references before the first unsafe write. Preserve platform metadata behavior, symlink protections, and existing transaction capacity limits.
- [ ] Implement reconciliation that distinguishes matching preimage, matching applied postimage, partial multi-path effects, and third-party modification. Matching desired bytes may satisfy a postcondition but cannot invent original command history.
- [ ] Make any compensation or rollback an explicit linked operation with its own checks. Compensation failure is retained as a failure; it does not make the parent successful.
- [ ] Test Windows case/path aliases, Unix permissions, rename/delete boundaries, and checkpoint/artifact corruption on applicable platforms.

**Verify:** `cargo test -p davinci-agent --test operation_filesystem`, plus existing transactional-edit, mutation queue, patch, notebook, and boundary tests.

**Commit:** `feat(runtime): bind filesystem effects to durable operations`

### Task 10 — Reconcile transaction phases and distinguish real Git mutations

**Depends on:** 09.

**Modify:** `runtime/transactions/{mod,model,store,verification,commit}.rs`; inspect `runtime/worktree` and actual Git helpers discovered in Task 00. Do not add a Git write to `observe_commit`.

**Create:** `runtime/operations/adapters/transactions.rs`; `crates/davinci-agent/tests/{operation_transactions,operation_git}.rs`.

- [ ] Write red tests for crash during apply/rollback, transaction receipt persisted before operation result, operation result before session projection, and commit observation interrupted between reading Git objects and saving evidence.
- [ ] Preserve the transaction journal as the authority for detailed file state. The recovery engine consults its stable receipt and sequence; it does not re-run a transaction because the operation result is missing.
- [ ] Add idempotent phase receipts and explicit source/owner checks to the operation link. Avoid pretending SQLite plus filesystem/JSON transaction records share one atomic commit.
- [ ] Keep `TransactionCoordinator::observe_commit` a read-only Git-object observation with an evidence outcome. Trace actual `git commit`, push, cherry-pick, and worktree changes through the relevant shell/process or structured host mutation operation.
- [ ] For structured Git mutations, use expected ref/worktree state and durable command identity where available. For opaque shell Git commands, block ambiguous replay; do not infer safety from matching commit text or a moved HEAD.
- [ ] Test partial effects and failed compensation without overwriting unrelated working-tree changes. Keep old transaction readers explicit about unsupported new metadata/schema.

**Verify:** `cargo test -p davinci-agent --test operation_transactions` and `cargo test -p davinci-agent --test operation_git`.

**Commit:** `feat(runtime): reconcile transaction receipts and Git effect evidence`

### Task 11 — Attach verification evidence to the exact operation and source state

**Depends on:** 10.

**Modify:** The inspected `davinci-coding-agent/src/runtime_host.rs` and graph `operations.rs`, plus the discovered `runtime/transactions/verification.rs`. **Resolve concrete files in Task 00:** the declared agent `transaction_verification` and runtime `evidence`, `evidence_store`, `completion` modules and graph verification adapters, then read their complete relevant paths before modification.

**Create:** `runtime/operations/adapters/verification.rs`; `crates/davinci-agent/tests/operation_verification.rs`.

- [ ] Write red tests for stale source manifests, wrong operation/attempt evidence, missing artifacts, verification failure after a successful write, and crash during a verification command.
- [ ] Give every verification command/probe its own operation ID and causal link. Record executable/arguments where applicable, source-manifest digest, contract/evidence version, exit outcome, output completeness, and artifact digests.
- [ ] Reuse current manifest/currentness and completion gates. Evidence for one worker generation or source revision cannot automatically approve another.
- [ ] Preserve successful execution when verification fails or becomes stale. Parent task completion may remain blocked; the original operation is not reset to executable.
- [ ] Treat arbitrary verification commands as effectful unless a constrained probe contract proves otherwise. Reconciliation probes must still pass current read/process permissions.
- [ ] Test honest recovered outcomes: successful postcondition observation cannot fabricate an unavailable original test exit code or stdout.

**Verify:** `cargo test -p davinci-agent --test operation_verification`, plus existing completion/evidence and graph-verification tests.

**Commit:** `feat(runtime): bind verification evidence to operation attempts`

### Task 12 — Bind graph workers, launches, and completion to operation identities

**Depends on:** 07, 11.

**Modify:** `davinci-coding-agent/src/native_extensions/graph/{mod,store}.rs`; inspect the declared `controller`, `process`, `worker`, `bindings`, `worker_sessions`, and `types` modules and modify their actual launch/binding/completion files. Their existence as modules was located during this audit; their full implementations must be reviewed in Task 00.

**Create:** `graph/operation_bridge.rs`; add the module declaration; create `crates/davinci-coding-agent/src/native_extensions/graph/operation_bridge_tests.rs` and register it as a test module.

- [ ] Write red tests for duplicate launch delivery, crash after worker creation, child result persisted before graph checkpoint, mismatched parent binding, and an attempted independent child coordinator.
- [ ] Persist explicit links among graph string run ID, runtime `RunId`, graph task, launch operation, child agent/session, attempt, generation, workspace, and contract digest before worker dispatch.
- [ ] Pass a parent-issued scoped journal transport binding to the worker. Never derive authority from the graph artifact, model output, user-editable invocation arguments, or a supplied session path alone.
- [ ] Publish worker completion through the operation result/outbox protocol. Retain artifact validation and existing graph policies before accepting an orchestration result.
- [ ] When graph projection persistence fails, preserve the child operation result and replay only the projection. Keep a failed worker from invalidating already-successful sibling operations.
- [ ] Test restored worker sessions against their original parent-bound lineage and operation IDs; no retry may create an unrelated conversation that merely shares a filename.

**Verify:** `cargo test -p davinci-coding-agent graph::operation_bridge_tests`, with nonzero test discovery, plus existing worker/session/binding/graph persistence tests.

**Commit:** `feat(graph): bind worker launch and completion to operation journal`

### Task 13 — Unify task and worker-control receipts without weakening domain policy

**Depends on:** 12.

**Modify:** `davinci-agent/src/runtime/{task_store,control,session}.rs`; inspect and modify the concrete task registry/transport/mailbox files declared by `runtime/mod.rs`; inspect the graph's control and coordinator-handler implementations.

**Create:** `runtime/operations/adapters/control.rs`; `crates/davinci-agent/tests/operation_control.rs`.

- [ ] Write red tests for duplicate task create/claim/status changes, stop/retry/steer delivery, stale generation/revision, a command-ID collision, and crash after the task journal commits but before the operation response persists.
- [ ] Reuse existing `TaskOperationReceipt`, `WorkerControlCommand`, and domain generation/revision checks. Map their identity into the operation journal rather than inventing an unrelated control ID for every delivery.
- [ ] Persist the command admission before dispatch, then execute the domain change under its existing authority and stable domain receipt. On a missing outer result, reconcile the exact receipt instead of applying the transition again.
- [ ] Make deduplication survive process restart, not just an in-memory `HashMap` or `HashSet`. Retain request digests so repeated IDs with changed actions are rejected.
- [ ] Separate policy-owned task metadata from execution-derived status. A graph/task projection can restrict scheduling, but it cannot authorize another effect when operation recovery is unresolved.
- [ ] Test interruption between mailbox acceptance and conversation insertion; a steering message must not be silently lost or inserted twice under the same command.

**Verify:** `cargo test -p davinci-agent --test operation_control`, plus existing task-journal/transport/mailbox and graph-control tests.

**Commit:** `feat(runtime): coordinate durable task and worker-control receipts`

### Task 14 — Make graph retry depend on effect recovery, not failure labels

**Depends on:** 13.

**Modify:** `graph/recovery.rs`, `graph/store.rs`, and the concrete controller/retry/continuation files resolved in Task 00.

**Create:** `graph/operation_retry_tests.rs` and register the test module.

- [ ] Write red tests where a worker times out after a committed file mutation, where an artifact is missing but effects are unknown, and where one sibling succeeded before another failed.
- [ ] Keep `classify_worker_failure` and `retry_decision` as orchestration/budget recommendations. Add a mandatory operation-recovery gate; a textual timeout/process/artifact label cannot overrule unresolved child effects.
- [ ] Require prior-owner quiescence, original binding, current authority, source reconciliation, and available retry budget before a replacement attempt. Use the existing retry condition semantics as the lower bound, not a looser replacement.
- [ ] Keep the original operation and attempt history. A corrective writer revision/replan uses an explicit new operation relationship; it does not silently mutate the old payload or broaden worker permission.
- [ ] Preserve successful siblings, original context ownership, and validated conversation ancestry. Block only affected dependencies/resources where evidence allows; an opaque workspace mutation may require a broader pause.
- [ ] Persist why a retry was permitted or denied, and test replayed graph controls after restart. Do not infer current run ownership from diagnostic log text.

**Verify:** `cargo test -p davinci-coding-agent graph::operation_retry_tests`, plus current retry, continuation, sibling-isolation, and parent-seed tests discovered in Task 00.

**Commit:** `fix(graph): gate worker retries on operation recovery evidence`

### Task 15 — Cover subagents, workflows, background jobs, and alternate hosts

**Depends on:** 13; validate graph-related behavior with 14.

**Modify:** `davinci-agent/src/lib.rs`, `runtime/mod.rs`, `davinci-coding-agent/src/runtime_host.rs`; resolve and inspect the declared `subagent`, `jobs`, `runtime/workflow`, `runtime/tools_agent`, and native-tool/extension-host modules before changes.

**Create:** `runtime/operations/adapters/agents.rs`; `crates/davinci-agent/tests/operation_agent_launch.rs`.

- [ ] Write red tests for direct subagent launch, batched subagent requests, workflow phase launch, background job completion, and launch through alternate host entry points.
- [ ] Give each launch a durable logical identity and explicit child context; inherit the parent-bound coordinator/transport rather than creating independent execution ownership.
- [ ] Carry the context through direct library execution, text/JSON/RPC paths, foreground UI, and applicable server/client/native-extension hosts. Close any bypass discovered by the coverage manifest.
- [ ] Link job notices and workflow phase completion to durable child results. A missing notice cannot trigger another launch or erase a successful job.
- [ ] Preserve current tool scoping, concurrency caps, budget checks, cancellation, worktree isolation, and provider selection. Do not serialize unrelated read-only workers solely to simplify the journal.
- [ ] Test parent shutdown while a child is active and reopening the parent session; unresolved child ownership remains visible and blocks unsafe replacement.

**Verify:** `cargo test -p davinci-agent --test operation_agent_launch`, plus existing workflow/job/subagent and alternate-host integration tests.

**Commit:** `feat(runtime): journal subagent workflow and background-job execution`

### Task 16 — Add browser operation adapters around existing lifetime checks

**Depends on:** 08, 11, 15.

**Modify:** `davinci-agent/src/process_manager.rs`; inspect the actual native browser tool/adapters and `davinci-coding-agent/src/browser_integration_tests.rs` identified in `main.rs`.

**Create:** `runtime/operations/adapters/browser.rs`; new browser-operation test cases in the existing product integration suite or a registered `browser_operation_tests.rs` test module.

- [ ] Write red tests for browser open/click/type/select/close, browser-process crash, response loss, changed dev-server lifetime, current permission revocation, and unverifiable listener ownership.
- [ ] Bind action identity to the existing process/session/owner/lifetime lease plus browser/page identity. Keep `with_verified_browser_dev_server` and current authorization checks before and after I/O.
- [ ] Persist action intent and effect latch before interactive browser effects. Record durable result/evidence references without logging secrets typed into forms or credential-bearing network details.
- [ ] Classify navigation and interactions conservatively; “open URL” can cause external activity. Reconcile supported browser state rather than automatically resubmitting a click or form.
- [ ] Keep screenshots/accessibility/network evidence source-bound and scoped. Similar visual output is not proof an external action ran only once.
- [ ] Ensure a browser adapter failure blocks/reconciles its affected operations while unrelated successful filesystem or sibling work stays intact.

**Verify:** `cargo test -p davinci-coding-agent browser_operation`, with fixture-backed tests requiring no live user website or credentials; run existing browser integration tests as applicable.

**Commit:** `feat(browser): journal actions and preserve managed-lifetime authority`

### Task 17 — Cover MCP, network requests, custom tools, and side-effecting hooks

**Depends on:** 06, 15; can proceed independently of 16 after shared APIs stabilize.

**Modify:** The actual `davinci-agent::mcp`/web/tool capability implementations, `davinci-mcp` transport dispatch, and coding-agent MCP/native/custom/JS/hook adapters discovered in Task 00. `davinci-coding-agent/src/runtime_host.rs` is the inspected hook/runtime integration anchor.

**Create:** `runtime/operations/adapters/external.rs`; `crates/davinci-agent/tests/operation_external.rs`; product-level adapter tests at the concrete host boundaries.

- [ ] Write red tests using local fixture servers for success, connection loss before send, loss after a remote mutation, duplicate delivery, expired/changed endpoint identity, and mismatched remote idempotency keys.
- [ ] Record server/endpoint identity, operation kind, schema/capability version, request digest, authorization context, and supported remote receipt references. Do not persist credentials in correlation metadata.
- [ ] Treat unknown MCP tools and custom hooks as potentially mutating unless a trusted registered contract establishes a narrower effect profile. Advisory names or model-supplied “read only” flags are not proof.
- [ ] Pass a stable remote key only through a verified endpoint contract. Without that contract, a response-loss window produces reconciliation/human-decision status rather than automatic replay.
- [ ] Ensure custom tools/hooks cannot silently escape the operation facade through a direct host callback. Where an extension is intentionally opaque, record the entire invocation as such and conservatively gate recovery.
- [ ] Test that receipt/result replay does not call the fixture endpoint again; separately test that an intentional new user-authorized operation is not deduplicated merely because arguments match.

**Verify:** `cargo test -p davinci-agent --test operation_external`, plus fixture-backed MCP/custom-tool/extension-host integration tests.

**Commit:** `feat(runtime): journal external adapters with conservative replay policy`

### Task 18 — Add causal reports and isolate observation failures

**Depends on:** 06; exercise all integrated adapters through 17 before completion.

**Modify:** `davinci-agent/src/runtime/{bus,mod}.rs`, `runtime/operations/failure.rs`, `davinci-coding-agent/src/runtime_host.rs`; inspect runtime event definitions and active telemetry/TUI event adapters before adding fields.

**Create:** `crates/davinci-agent/tests/operation_failure_domains.rs` and redaction fixtures.

- [ ] Write red tests for observer panic/slowdown/disconnect, provider failure after tool success, post-hook failure, context-compaction failure, language-service failure, browser failure, and a failed child with a successful sibling.
- [ ] Emit durable transition/recovery events with operation/root/run/session/agent/worker IDs, attempt, causal parent, from/to state, revision, reason, timestamps, and artifact references. Derive observation events from committed journal facts.
- [ ] Keep allowlisted decision hooks synchronous and fail-closed. Isolate noncritical subscribers with bounded queues or equivalent non-owning dispatch; a slow observer must not indefinitely block execution or exhaust memory.
- [ ] Produce a structured causal report distinguishing failed subsystem, effect certainty, durable result, verification status, publication state, and allowed recovery action. Do not reduce every failure to `Graph worker failed`.
- [ ] Keep dynamic correlation outside stable prompts, provider cache partitions, and conversation/transport ownership. Run the existing OpenAI cache identity and wire fixture regressions selected in Task 00.
- [ ] Test terminal escaping and redaction for logs, shell output, endpoint arguments, paths, and secrets. A TUI failure must preserve durable runtime facts; a whole-process abort still requires normal crash recovery rather than an impossible promise that an in-process engine remains alive.

**Verify:** `cargo test -p davinci-agent --test operation_failure_domains`, plus event bus, hook, cache, and UI forwarding regressions.

**Commit:** `feat(runtime): report causal failures and isolate observation channels`

### Task 19 — Add read-only runtime inspectors and doctor diagnostics

**Depends on:** 04 for basic query support; 18 for final integration.

**Modify:** `davinci-coding-agent/src/{main,args}.rs`; inspect active TUI recovery/graph-detail wiring, including the discovered `davinci-tui/src/davinci/views/recovery.rs`.

**Create:** `runtime/operations/inspector.rs`, `davinci-coding-agent/src/runtime_inspect.rs`, `crates/davinci-coding-agent/tests/runtime_inspect.rs`.

- [ ] Write red CLI tests for the command grammar below, invalid IDs, nonexistent roots, unknown schema, corrupt/missing artifacts, JSON output, output bounds, and no unintended writes or provider initialization.
- [ ] Add explicit inspector dispatch before normal agent/provider startup. Preserve literal prompt handling with `--` so ordinary user text is not accidentally interpreted as a maintenance command.
- [ ] Implement operation/run/session views with identity, parent/requester, attempts, timeline, side effects, owner, domain links, result, evidence, and recovery reasons. Resolve graph/runtime ID distinctions explicitly.
- [ ] Implement `doctor runtime` as a diagnostic reader: detect incomplete operations, missing projections, bad bindings, broken evidence links, and inconsistent revisions. It must not launch a worker, stop a process, reauthorize, migrate, truncate, or repair by default.
- [ ] Open storage in a mode proven not to create or mutate journal/session/sidecar files. If a safe consistent read is unavailable, report the limitation instead of opening a writable recovery path. Do not use immutable-database shortcuts against a live changing WAL.
- [ ] Link TUI failure details to the same report model without redesigning the whole interface or disabling input during ongoing graph execution. Snapshot-test compact and expanded renderings.

```bash
davinci inspect run <run-id>
davinci inspect operation <operation-id>
davinci inspect session <session-id>
davinci inspect operation <operation-id> --json
davinci doctor runtime
```

Proposed diagnostic exits: `0` for healthy/readable result, `2` for detected recovery/inconsistency findings, `3` for unavailable/corrupt/unsupported source. Keep machine-readable schema versions explicit. Any future repair command must be separate, authorized, journaled, and dry-run capable.

**Verify:** `cargo test -p davinci-coding-agent --test runtime_inspect`; then run `cargo run -p davinci-coding-agent --bin davinci -- inspect operation <fixture-operation-id> --json` against a generated fixture, not an invented ID.

**Commit:** `feat(cli): inspect runtime operations and diagnose recovery state`

### Task 20 — Migrate legacy roots and retire competing recovery decisions

**Depends on:** 14-19.

**Modify:** `runtime/operations/migrations.rs`, `davinci-agent/src/{tool_ledger,lib}.rs`, `runtime/{session,task_store}.rs`, `graph/{store,recovery}.rs`, and the actual session/graph startup paths discovered earlier.

**Create:** `crates/davinci-agent/tests/operation_migration.rs`; fixtures under `crates/davinci-agent/tests/fixtures/operations/`; update the coverage and authority audit documents.

- [ ] Write red tests for legacy tool ledgers; existing session/task journal versions; partial transaction state; `.davinci` and legacy `.pi` graph roots; corrupted records; repeated migration; and interruption during schema change.
- [ ] Bind imported records to their original source digest and identity. Import known results as legacy observations with explicit evidence provenance; mark unknown start/effect/timing/owner fields unknown instead of filling in convenient defaults.
- [ ] Preserve original data and perform versioned, exclusive, idempotent migration. Do not reinterpret a legacy “running” string as proof that a process is live or that a completed mutation is retryable.
- [ ] Run operation reconciliation before legacy startup paths independently fail/reschedule work. For migrated families, replace old retry branches with adapters/projections and retain only their domain-specific validation.
- [ ] Cover sessionless and embedding entry points explicitly. Proposed durable sessionless mode creates a private **execution-only** root, not a conversation transcript; verify existing `--no-session` retention expectations before enabling it. Do not silently retain excluded payloads. An explicitly memory-only embedding must report that crash recovery is unavailable and cannot be advertised as durable mode.
- [ ] Prove authoritative-to-legacy flag fallback is blocked for existing roots. Test downgrade behavior against the actual supported previous reader; document unsupported downgrades instead of claiming old binaries obey new markers they do not understand.

**Verify:** `cargo test -p davinci-agent --test operation_migration`, plus product session/graph restore tests and a coverage audit showing no authoritative operation family still invokes a legacy retry authority.

**Commit:** `refactor(runtime): migrate legacy execution records and converge recovery`

### Task 21 — Expand hard-crash, property, and fuzz coverage

**Depends on:** All adapter tasks and 20. Fault tests already exist from Task 04 onward.

**Modify:** The shared fault harness and all operation-focused test suites.

**Create:** `crates/davinci-agent/tests/operation_properties.rs`, `crates/davinci-coding-agent/tests/runtime_recovery_e2e.rs`; proposed separate fuzz harness `fuzz/fuzz_targets/operation_record.rs` and `operation_reducer.rs`, with its own manifest/toolchain isolation if the repository has no existing fuzz workspace.

- [ ] Generate deterministic sequences of admit, approve/deny, claim, effect, result, delivery, duplicate control, cancellation, owner loss, verification, source change, corruption, and restart. Persist the failing seed and minimized event sequence.
- [ ] Assert the seven requested core properties: no automatic duplicate irreversible replay; successful sibling preservation; termination requires proof; resume never invents successful output; retries preserve authority/lineage; corruption fails closed; duplicate controls transition at most once.
- [ ] Add properties for same-key collisions, stale generation rejection, unknown schema, bounded allocations, full-result replay, outbox deduplication, and evidence-currentness. Unknown outcomes must remain unknown until justified by evidence.
- [ ] Add a retry-bypass regression: after an unresolved effect, supplying a fresh tool-call ID, switching to batch, or relaunching through graph cannot evade the affected resource's unresolved-effect barrier. A deliberate human-authorized new action is separately recorded with the duplication risk acknowledged.
- [ ] Run process-kill crash tests for real temporary files, the real transaction coordinator, supervised processes, worker completion, and independently surviving local fixture servers. Use error injection for storage failures in addition to hard kills, not instead of them.
- [ ] Add bounded parser/reducer fuzzing. Reuse an existing fuzz setup where present; otherwise keep optional nightly/libFuzzer tooling outside the stable workspace gates and pin dependencies only after checking current MSRV compatibility. Commit useful crash corpus inputs as regressions.

**Verify:**

```bash
cargo test -p davinci-agent --test operation_properties
cargo test -p davinci-agent --test operation_crash
cargo test -p davinci-coding-agent --test runtime_recovery_e2e
```

**Acceptance:** Mandatory deterministic coverage runs on supported Linux, Windows, and macOS configurations. Platform exclusions state the missing guarantee. Optional fuzz campaign duration/seed/corpus and failures are recorded; a short successful fuzz run is not a proof of correctness.

**Commit:** `test(runtime): exercise crash recovery and execution invariants`

### Task 22 — Bound overhead, retention, and storage failure behavior

**Depends on:** 20-21.

**Modify:** Journal writer/query/GC code, result artifact adapters, graph retention in `graph/store.rs`, task receipt retention, and relevant runtime limits.

**Create:** `crates/davinci-agent/tests/operation_capacity.rs`; `docs/runtime/operation-performance.md`; use the repository's benchmark convention after discovery, or add an explicit ignored local benchmark test rather than assuming a benchmark framework is installed.

- [ ] Benchmark baseline versus journaled no-op/read-heavy/mutation-heavy/batch/worker workloads on named hardware, OS, filesystem, and build profile. Measure p50/p95 admission, durable barriers, throughput, memory, journal growth, and startup replay.
- [ ] Batch journal transactions without acknowledging durability early. Coalesce state events where semantics allow; never turn each streamed token/output chunk into a durable database transaction.
- [ ] Bound queues, record sizes, artifact sizes, indexes, query pages, and recovery scans. Add backpressure, observable storage capacity, and enough reserved capacity for in-flight terminal/recovery records; never evict correctness data to admit new work.
- [ ] Protect unresolved operations, unacknowledged outbox entries, referenced artifacts/transactions, and ancestor graph evidence from retention cleanup. Existing graph pruning must consult operation references before deleting a finished run's directory.
- [ ] Retain idempotency tombstones until the caller scope is permanently closed or an explicit archival contract makes replay impossible. Deleting output retention data must not make an old command executable again.
- [ ] Test disk-full after effect, quota-full before effect, artifact disappearance, interrupted archival/checkpointing, and inspector access under capacity pressure. Preserve uncertainty and provide recovery guidance instead of success with missing evidence.

**Proposed performance gate:** Before tuning, agree and record a reference workload and hardware budget. An initial review threshold is more than 20% p95 regression in a representative whole-run workload, not a promise of current performance; investigate measured regressions. Correctness-critical barriers are never removed to meet the threshold. Report raw measurements, not only percentages.

**Verify:** `cargo test -p davinci-agent --test operation_capacity`, measured benchmark commands recorded verbatim, and graph-retention regressions.

**Commit:** `perf(runtime): bound journal overhead and preserve recovery evidence`

### Task 23 — Document, integrate CI, and verify the full product

**Depends on:** 00-22.

**Modify:** Actual CI workflows discovered in Task 00, `AGENTS.md`, applicable user/developer runtime/session/graph docs, and the requirements coverage document. Do not assume `.github/workflows/test.yml` is the active workflow path.

**Create:** `docs/runtime/unified-execution.md`, `docs/runtime/recovery-playbook.md`, `docs/runtime/operation-journal-format.md`, `docs/runtime/execution-recovery-verification.md`; proposed new `.github/workflows/runtime-recovery.yml` only after checking existing workflow conventions and required-check policy.

- [ ] Document ownership, the operation/attempt distinction, side-effect uncertainty, result versus verification versus delivery, supported legacy modes, journal retention/privacy, recovery classifications, and inspector usage with fixture-generated examples.
- [ ] Provide incident playbooks for failed result persistence after effect, unknown process ownership, stale worker binding, incomplete transaction, missing evidence, corrupt journal, duplicate command collision, and blocked downgrade. Destructive repair is never an implicit startup action.
- [ ] Wire deterministic recovery/property suites into required CI; add platform-specific process/filesystem tests and optional fuzz schedules without weakening existing checks. Verify the current required status-check configuration rather than relying on old documentation.
- [ ] Run the complete command set below on the final branch. Fix regressions introduced by this work and re-run affected focused tests after every fix. Capture any unrelated baseline failure separately; do not call the complete product verified while required CI is red.
- [ ] Run the end-to-end acceptance scenarios and compare the capability/coverage manifest against baseline: tool names, providers/cache, permissions, graphs, subagents, processes, transactions, browser/MCP, context, alternate hosts, and usable TUI input are retained.
- [ ] Produce the final evidence report with commit, platform, commands/test counts, fault scenarios, measured overhead, migration fixtures, remaining unsupported guarantees, and links to CI runs. Check off tasks only when their specific evidence exists.

```bash
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
```

**Commit:** `docs(runtime): document unified recovery and record release verification`

## 8. Test matrix, performance, and release gates

### 8.1 Required acceptance scenarios

| Scenario | Required observation |
|---|---|
| Mutation; crash before response; duplicate delivery | One original mutation; reconciliation or saved-result replay, never an unexamined second execution. |
| Result committed; parent notification lost | Result survives; outbox finishes delivery; executor call count is unchanged. |
| Writer succeeds; verifier fails | Successful write remains recorded; verification failure blocks appropriate parent completion. |
| One graph worker fails after siblings succeed | Successful sibling outputs/operations/lineage survive retry or replan. |
| Spawn acknowledgement lost | Managed lifetime reconciled or action blocked; no automatic second process. |
| Stop requested but ownership/exit unknown | Inspector reports stopping/unknown, not safely terminated. |
| Duplicate control with changed payload/generation | Collision or stale-command rejection; no new transition/effect. |
| Same intended retry with a new tool-call ID | Unresolved-effect barrier still protects affected resources; a new key is not a safety bypass. |
| TUI/telemetry observer fails | Durable operation result remains available; no required execution state lives only in that observer. |
| Journal/schema/lineage is invalid | Affected dispatch stops; old evidence is preserved; no silent repair. |
| Legacy session/graph opened | Explicit migration or legacy handling; no invented successful effects or broadened authority. |
| Inspector/doctor reads an inconsistent root | Bounded causal output and non-success diagnostic exit; no mutations, provider calls, or process actions. |
| Retention removes old output | Duplicate-protection tombstone remains or caller scope is closed; unresolved evidence is not deleted. |
| Correlation IDs change between equivalent requests | Stable provider prompt/cache-sensitive content is unchanged; conversation identity stays correctly scoped. |

### 8.2 Required fault points

Use typed injected points, not user/model-controlled production switches:

```text
BeforeIntentCommit       AfterIntentCommit
BeforeAttemptClaim       AfterAttemptClaim
BeforeEffectLatch        AfterEffectLatch
AfterProcessSpawn        BeforeLaunchReceiptCommit
AfterSideEffect          BeforeResultCommit
AfterResultCommit        BeforeProjectionApply
AfterProjectionCommit    BeforeOutboxAck
BeforeVerification       AfterVerificationEffect
DuringTransactionApply   DuringTransactionRollback
DuringWorkerCompletion   DuringSessionAppend
DuringMigrationCommit    DuringArtifactArchive
```

Cover each relevant point for read-only, filesystem, process, graph/control, and external operations. Not every point applies to every adapter; the test matrix must say why, not simply mark all boxes green.

### 8.3 Scope of safety guarantees

The automated suite must distinguish: normal errors, task/host process termination, concurrent delivery, storage-write failure, corrupted persisted data, remote response loss, and hardware/power failure. Child-process kill tests plus durability checks provide application-crash evidence; they do not alone prove all possible storage hardware behavior.

Likewise, a remote fixture demonstrates the adapter protocol, not that every live MCP/browser/network endpoint supports idempotency. Endpoint-specific capabilities remain explicit. Recovery can safely stop and require human input; that is a valid outcome, not a test failure to remove by broadening retries.

### 8.4 Privacy, access, and unresolved-effect barriers

The journal must not become a central plaintext leak of credentials, environment values, form inputs, MCP secrets, or arbitrary command output. Keep identity metadata separate from protected payload/result blobs, use existing secret-reference and directory controls, and make redaction policy explicit. A missing secret during restart can legitimately prevent automatic replay. Inspection/export must honor current access policy without revealing another session's data.

An unresolved effect establishes a barrier on its affected resource/domain, not just its original call ID. Within the workspace-bound physical journal, root namespaces consult the same durable conflicting-effect claims without gaining access to one another's conversations. This prevents a provider, graph retry, batch wrapper, or custom tool from escaping recovery by asking for the same risky action under a fresh key. Narrow the barrier when trustworthy effect scope is known. For opaque shell actions, conservatively pause potentially conflicting workspace mutations while allowing unrelated safe inspection where possible.

A human decision must be a durable, authenticated control action tied to the operation, evidence, and specific proposed next step. “Retry anyway” creates an explicit risk-accepted action; it must not rewrite the original attempt as known-not-started or claim duplication is impossible.

### 8.5 Definition of implementation done

Implementation is ready only when all required tasks and focused tests have evidence, the full workspace gates and current required CI are green, supported capability parity is demonstrated, and the inspector can answer the attachment's questions from persisted records: what was requested, who requested it, what started, what may have changed, what completed, what was saved, what failed, what was retried, whether duplication is possible, and whether resumption is permitted.

A passing build without crash tests is insufficient. A new journal with old independent retry paths still active is insufficient. A command that prints attractive log summaries without durable authority/evidence is insufficient.

## 9. Requirements traceability and completion report

### 9.1 Requirement-to-task mapping

| Attached requirement | Planned coverage |
|---|---|
| Current architecture analysis and duplicated ownership | 00, sections 1-2. |
| Universal operation identity/model/lifecycle | 01-03, 05; sections 3-4. |
| Intent separated from execution | 02-05, each adapter task. |
| Idempotency and crash-before-response protection | 03-04, 06-10, 12-17, 21. |
| Crash-window audit | Section 5.3; 04 and adapter-specific tests. |
| One deterministic recovery authority | 04, 13-15, 20. |
| Side-effect classification and evidence-based retry | 01, 04, 07-11, 16-17. |
| Authority hierarchy and clear projections | Sections 2-4; 05-06, 12-15, 20. |
| Logs/correlation/timeline/causal failure reports | 18-19. |
| Isolated provider/TUI/telemetry/context/browser/worker failures | 06, 14-18, 21. |
| Fault injection | 04 throughout development; 21 and section 8.2. |
| Seven requested properties and fuzzing | 21, section 8.1. |
| Inspector and doctor commands | 19. |
| Performance and asynchronous noncritical telemetry | 18, 22. |
| Incremental migration and reuse | 00, 05-17, 20; section 6. |
| Backward compatibility and versioned storage | 01-02, 20. |
| Unit, crash, idempotency, recovery, and integration tests | Every task; 21 and 23. |
| Documentation and final verification report | 23. |
| Preserve all existing capabilities | Baseline/coverage manifest in 00; parity gates in 23. |

### 9.2 Final implementation report template

```text
Implementation commit / branch:
Source baseline and subsequent rebase:
Operation families authoritative / legacy / excluded:
Known remaining bypasses:
Schema and migration versions:
Supported journal filesystem / platforms:
Focused tests (command, count, result):
Hard-crash scenarios (fault, durable facts, recovery, mutation count):
Property/fuzz seeds and corpus:
Process-ownership/termination evidence:
Session/graph/transaction migration results:
Performance measurements and environment:
Full fmt/clippy/workspace test results:
Required CI run links and statuses:
Capability parity and cache regressions:
Unresolved limitations / human-decision cases:
```

For this planning delivery, all implementation fields above remain unverified. The source audit and Markdown artifact are the completed work; the application implementation is not.

## 10. Inspected source references

All repository links below are pinned to the source snapshot used for this plan. “Module located” or “directory listing” does not mean its implementation was fully reviewed. Future implementers must revalidate the checkout and open the exact relevant implementations before edits. The attached requirements remain the requested product/architecture brief; source-derived findings and proposed changes are distinguished throughout this document.

| ID | Source | Inspected scope | Relevance |
|---|---|---|---|
| S01 | [Cargo.toml](https://github.com/J12003LPZ/davinci/blob/0d3163c30d528bcddaf9a2ee211a4ba052d7f420/Cargo.toml) | Full manifest | Active workspace, product version, Rust floor, pinned SQLite dependency. |
| S02 | [AGENTS.md](https://github.com/J12003LPZ/davinci/blob/0d3163c30d528bcddaf9a2ee211a4ba052d7f420/AGENTS.md) | Full file supplied by connector | Repository guidance and the documented crate-layout mismatch; not a build inventory. |
| S03 | [crates/davinci-coding-agent/Cargo.toml](https://github.com/J12003LPZ/davinci/blob/0d3163c30d528bcddaf9a2ee211a4ba052d7f420/crates/davinci-coding-agent/Cargo.toml) | Full manifest | Actual CLI package, executable target, host dependencies. |
| S04 | [crates/davinci-agent/src/lib.rs](https://github.com/J12003LPZ/davinci/blob/0d3163c30d528bcddaf9a2ee211a4ba052d7f420/crates/davinci-agent/src/lib.rs) | Lines 1-230 | Active agent modules, runtime exports, hook/executor boundary definitions. |
| S05 | [crates/davinci-coding-agent/src/main.rs](https://github.com/J12003LPZ/davinci/blob/0d3163c30d528bcddaf9a2ee211a4ba052d7f420/crates/davinci-coding-agent/src/main.rs) | Lines 1-190 | Host module declarations and integration-test entry points. |
| S06 | [crates/davinci-agent/src/tool_ledger.rs](https://github.com/J12003LPZ/davinci/blob/0d3163c30d528bcddaf9a2ee211a4ba052d7f420/crates/davinci-agent/src/tool_ledger.rs) | Lines 1-540 | Existing records, normalization, durability, reservation, restart and replay semantics. |
| S07 | [crates/davinci-agent/src/turn.rs](https://github.com/J12003LPZ/davinci/blob/0d3163c30d528bcddaf9a2ee211a4ba052d7f420/crates/davinci-agent/src/turn.rs) | Lines 430-750 and 1250-1530 | Session-result handling, provider retry boundary, checked tool dispatch and effect-start persistence. |
| S08 | [crates/davinci-agent/src/batch.rs](https://github.com/J12003LPZ/davinci/blob/0d3163c30d528bcddaf9a2ee211a4ba052d7f420/crates/davinci-agent/src/batch.rs) | Lines 1-280 | Batch child IDs, shared preparation/dispatch, lanes, post hooks and receipts. |
| S09 | [crates/davinci-agent/src/runtime/task_store.rs](https://github.com/J12003LPZ/davinci/blob/0d3163c30d528bcddaf9a2ee211a4ba052d7f420/crates/davinci-agent/src/runtime/task_store.rs) | Lines 1-220 | Task journal authority, receipts, bounded frames, schema/checksum/sequence and lease design. |
| S10 | [crates/davinci-agent/src/runtime/transactions/store.rs](https://github.com/J12003LPZ/davinci/blob/0d3163c30d528bcddaf9a2ee211a4ba052d7f420/crates/davinci-agent/src/runtime/transactions/store.rs) | Lines 1-180 | Transaction owner/workspace/schema checks, bounds and atomic writes. |
| S11 | [crates/davinci-agent/src/runtime/transactions/commit.rs](https://github.com/J12003LPZ/davinci/blob/0d3163c30d528bcddaf9a2ee211a4ba052d7f420/crates/davinci-agent/src/runtime/transactions/commit.rs) | Full file | Read-only existing-commit observation and evidence validation; not Git commit creation. |
| S12 | [crates/davinci-agent/src/process_manager.rs](https://github.com/J12003LPZ/davinci/blob/0d3163c30d528bcddaf9a2ee211a4ba052d7f420/crates/davinci-agent/src/process_manager.rs) | Lines 1-260 | Managed process and browser lifetime, current authorization, socket identity checks. |
| S13 | [crates/davinci-agent/src/jobs/supervisor/mod.rs](https://github.com/J12003LPZ/davinci/blob/0d3163c30d528bcddaf9a2ee211a4ba052d7f420/crates/davinci-agent/src/jobs/supervisor/mod.rs) | Full file | Trusted supervisor types, platform/helper/client/wire module boundaries. |
| S14 | [crates/davinci-agent/src/runtime/session.rs](https://github.com/J12003LPZ/davinci/blob/0d3163c30d528bcddaf9a2ee211a4ba052d7f420/crates/davinci-agent/src/runtime/session.rs) | Lines 1-220 | Root/worker restore paths, task recovery, parent-bound worker identity and session lease. |
| S15 | [crates/davinci-coding-agent/src/native_extensions/graph/recovery.rs](https://github.com/J12003LPZ/davinci/blob/0d3163c30d528bcddaf9a2ee211a4ba052d7f420/crates/davinci-coding-agent/src/native_extensions/graph/recovery.rs) | Lines 1-220 | Typed failure classes, compatibility text markers and bounded graph retry recommendations. |
| S16 | [crates/davinci-coding-agent/src/native_extensions/graph/store.rs](https://github.com/J12003LPZ/davinci/blob/0d3163c30d528bcddaf9a2ee211a4ba052d7f420/crates/davinci-coding-agent/src/native_extensions/graph/store.rs) | Lines 1-200 | Graph roots, atomic state helper, legacy directory selection and retention/pruning. |
| S17 | [crates/davinci-agent/src/runtime/bus.rs](https://github.com/J12003LPZ/davinci/blob/0d3163c30d528bcddaf9a2ee211a4ba052d7f420/crates/davinci-agent/src/runtime/bus.rs) | Lines 1-170 | Observation and allowlisted decision semantics; panic behavior. |
| S18 | [crates/davinci-agent/src/runtime/mod.rs](https://github.com/J12003LPZ/davinci/blob/0d3163c30d528bcddaf9a2ee211a4ba052d7f420/crates/davinci-agent/src/runtime/mod.rs) | Lines 1-250 | Shared runtime handles, IDs, state owners, module declarations and continuation handling. |
| S19 | [crates/davinci-agent/Cargo.toml](https://github.com/J12003LPZ/davinci/blob/0d3163c30d528bcddaf9a2ee211a4ba052d7f420/crates/davinci-agent/Cargo.toml) | Full manifest | Agent dependency boundary; proposed SQLite dependency is not currently present here. |
| S20 | [crates/davinci-agent/src/runtime/control.rs](https://github.com/J12003LPZ/davinci/blob/0d3163c30d528bcddaf9a2ee211a4ba052d7f420/crates/davinci-agent/src/runtime/control.rs) | Lines 1-190 | Control commands/receipts, generation/revision predicates and termination-state distinctions. |
| S21 | [crates/davinci-coding-agent/src/native_extensions/graph/mod.rs](https://github.com/J12003LPZ/davinci/blob/0d3163c30d528bcddaf9a2ee211a4ba052d7f420/crates/davinci-coding-agent/src/native_extensions/graph/mod.rs) | Lines 1-200 | Graph module map, worker model identity, active-run bookkeeping and lifecycle boundaries. |
| S22 | [crates/davinci-session/src/lib.rs](https://github.com/J12003LPZ/davinci/blob/0d3163c30d528bcddaf9a2ee211a4ba052d7f420/crates/davinci-session/src/lib.rs) | Lines 1-180 | Actual JSONL session types/open/create behavior and repository abstraction modules. |
| S23 | [crates/davinci-coding-agent/src/args.rs](https://github.com/J12003LPZ/davinci/blob/0d3163c30d528bcddaf9a2ee211a4ba052d7f420/crates/davinci-coding-agent/src/args.rs) | Lines 1-150 | Current CLI argument structure, sessionless/alternate modes and literal -- behavior. |
| S24 | [crates/davinci-coding-agent/src/runtime_host.rs](https://github.com/J12003LPZ/davinci/blob/0d3163c30d528bcddaf9a2ee211a4ba052d7f420/crates/davinci-coding-agent/src/runtime_host.rs) | Lines 1-160 | Session/workflow host restore and budget/evidence/hook adapters. |
| S25 | [crates/davinci-coding-agent/src/native_extensions/graph/operations.rs](https://github.com/J12003LPZ/davinci/blob/0d3163c30d528bcddaf9a2ee211a4ba052d7f420/crates/davinci-coding-agent/src/native_extensions/graph/operations.rs) | Lines 1-190 | Graph verification source-manifest binding and revisioned budget updates. |
| S26 | [crates/davinci-agent/src/runtime/transactions](https://github.com/J12003LPZ/davinci/tree/0d3163c30d528bcddaf9a2ee211a4ba052d7f420/crates/davinci-agent/src/runtime/transactions) | Directory listing, not every implementation | Located transaction model/files/tools/verification/platform modules; read before editing. |
| S27 | [docs/cache/openai-cache-contract.md](https://github.com/J12003LPZ/davinci/blob/0d3163c30d528bcddaf9a2ee211a4ba052d7f420/docs/cache/openai-cache-contract.md) | Full document | Repo-defined separation of correlation, cache, conversation and transport; used here for local invariants, not independent live-provider capability claims. |
| S28 | [Pinned commit](https://github.com/J12003LPZ/davinci/commit/0d3163c30d528bcddaf9a2ee211a4ba052d7f420) | Commit metadata | Pinned source identity and merge metadata; not a replacement for the compiled workspace manifest. |

### Planning verification record

The Markdown deliverable was checked for sequential task numbering, unchecked implementation steps, balanced code fences, source-reference coverage, requirement mapping, and explicit separation of proposed work from verified source observations. These document checks are not Rust compilation, application test execution, a full repository audit, or CI verification.
