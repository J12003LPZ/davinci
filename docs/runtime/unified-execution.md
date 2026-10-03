# Unified execution and recovery

## Scope

Tasks 00-23 establish one durable execution authority for the active
 davinci-agent and davinci-coding-agent paths. A logical operation is one
caller intent and scoped idempotency key. An attempt is one owner generation
trying that operation. Effect evidence, execution result, verification result,
and consumer delivery remain separate facts.

## Ownership contract

| Fact | Authority |
| --- | --- |
| Intent, operation/attempt identity, revisions, owners, transitions, recovery decisions, result refs, outbox | Operation journal |
| Live process identity, listener ownership, and exit observation | ProcessManager and supervisor |
| File preimages, apply/rollback, workspace binding | Transaction coordinator |
| Graph topology, worker binding, attempts, artifacts, retention | Graph store/controller |
| Conversation rows and session lineage | davinci-session/runtime session |
| Verification and evidence verdicts | Verification/evidence producers |
| TUI, telemetry, and runtime-bus events | Rebuildable observers |

The normal durable path is:

    Created -> Persisted -> Authorized -> Queued -> Running -> Succeeded/Failed

An adapter records EffectPossible before an uncertain effect. Restart moves an
interrupted attempt to RecoveryRequired; evidence must be recorded before
final success, final failure, or blocking. A retry keeps the operation ID,
creates a new attempt ID, and advances the owner generation. It never rewrites
prior history. Cancellation before dispatch is different from termination when
ownership or effect is unknown.

## Admission and idempotency

Migrated callers first validate the known/enabled capability and create an
immutable OperationSpec with root/workspace and session lineage, operation kind,
effect profile, preconditions, normalized payload, and digest. The immutable
intent is durably persisted before runtime decision hooks, contract/effect
policy, or permission/approval evaluation. A denied operation transitions from
Persisted/Authorized/Queued to Cancelled with EffectStatus::NotStarted; only an
approved intent receives an AuthorizationReceipt and advances to Queued.
Admission is unique by root, caller scope, and caller key. A matching digest
reuses the operation; a changed payload is a collision. A duplicate delivery
can replay a stored result or outbox publication and cannot silently execute a
second mutation. Unresolved resource claims block fresh keys. Owner fencing,
revisions, and dispatch permits reject stale commands.

The coordinator is used by direct and batch tools, host controls,
process/browser lifetimes, external providers, verification, graph workers, and
task/agent boundaries. Existing tool-ledger, task, transaction, graph, and
session records remain domain evidence. Arbitrary shell descendants, custom
extensions, hooks, remote MCP/HTTP effects, and browser-side effects remain
conservative opaque boundaries. Sessionless workers remain ephemeral without a
durable root. Git commit observation is evidence, not a Git mutation.

## Migration, privacy, and retention

SQLite schema version 4 imports legacy tool-ledger and task-runtime records as
append-only legacy_observations keyed by source digest and record identity.
Changed source bytes are rejected; original files remain. Legacy .pi graph
roots are checked beside .davinci roots and conflicting checkpoints fail
closed. Startup never performs destructive repair or invents owner/effect/
success facts.

Payloads, results, evidence references, outbox messages, query pages, and
recovery scans are bounded before writes. OperationJournal::capacity() reports
occupancy for admission/draining telemetry; it never authorizes eviction.
Unresolved attempts, claims, pending outbox rows, referenced artifacts, task
receipts, graph ancestors, and idempotency tombstones are retained until an
explicit archival contract closes the caller scope.

## Read-only inspection

    davinci inspect operation <operation-uuid> --json
    davinci inspect run <run-uuid> --json
    davinci inspect session <session-id> --json
    davinci doctor runtime --json

The inspector discovers existing roots only; it never creates or repairs one,
starts a provider, mutates a graph, or runs a process. Healthy output exits 0.
Missing, corrupt, unsupported, inconsistent, or bounded-out output exits 3 and
preserves evidence. The fixture in
crates/davinci-coding-agent/tests/runtime_recovery_e2e.rs creates a real journal,
backs it up, reopens it through the CLI, and verifies restart/read-only
stability.

See operation-journal-format.md, recovery-playbook.md,
execution-recovery-verification.md, operation-performance.md, and
operation-coverage.md for the format, incident actions, measured gates, and
coverage boundaries.
