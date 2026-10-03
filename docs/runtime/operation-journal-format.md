# Operation journal format

## Location and identity

A durable root stores SQLite at:

    <durable-root>/.davinci/operations/operations.sqlite3

The opener requires an absolute directory, nonzero workspace binding version,
and matching journal, workspace, and root identities. SQLite application ID is
0x44564f50 and database schema is version 4. Trusted schema is disabled before
validation. A closed backup contains operations.sqlite3 without a live WAL
sidecar and must be written to a new directory.

## Durable records

| Table | Durable fact |
| --- | --- |
| journal_metadata | singleton journal/workspace binding, schema, creation time |
| journal_migrations | applied schema versions |
| journal_roots | root namespace binding |
| operations | immutable spec, caller key, payload and intent digests |
| attempts | owner/generation, revision, state, effect status, outcome refs |
| operation_events | append-only transition sequence and revisions |
| operation_results | immutable result reference, digest, bounded payload |
| operation_outbox | consumer payload and pending/acknowledged state |
| operation_owners | owner-generation history |
| dispatch_claims | dispatch permit and durable effect-start latch |
| operation_call_mappings | stable wire/session/caller mapping |
| operation_recovery_decisions | evidence-bound recovery decisions |
| operation_effect_claims | unresolved resource claims and resolution |
| legacy_observations | append-only observations from old stores |

Specifications, attempts, events, results, recovery decisions, effect claims,
and legacy observations reject unsafe updates/deletes. An outbox acknowledgement
can change only state and acknowledgement time.

## Wire records and reducer

OperationSpec schema version 1 contains operation ID, context, scoped caller
key, payload digest, kind, effect profile, preconditions, and normalized JSON
payload. Context binds journal/root/workspace, session/run, agent/worker/task/
graph lineage, caller type, parent operation, and optional wire call ID.

OperationAttempt contains stable operation ID plus attempt ID/number, owner
ID/generation, revision, state, effect status, authorization, timestamps,
result/failure, verification/publication projections, cancellation, retry
lineage, recovery evidence, and supersession.

OperationEvent includes persist, authorize, queue, start, mark_effect_possible,
complete_success, complete_failure, cancel_before_start, request_cancellation,
interrupt, recover_unstarted_dispatch, require_recovery,
finalize_recovered_success, finalize_recovered_failure, block_recovery,
mark_verification, publish, and supersede. The reducer enforces legal state
transitions, monotonic revisions, and non-not_started effect certainty for any
completed outcome. Retry creates a new attempt and preserves old history.

## Versioned migration

- schema 1 creates core operation, attempt, event, result, and outbox tables;
- schema 2 adds intent digests, owner generations, dispatch claims, and wire
  call mappings;
- schema 3 adds evidence-bound recovery decisions and resource claims;
- schema 4 adds append-only legacy observations.

Migrations are transactional, record their version, and validate application ID
and identity. Failed migration rolls back and can be retried. Changed legacy
bytes under an existing source key are rejected. Model schema versions are
independent from SQLite schema versions; unknown versions fail closed.

## Bounds and inspection

Current writer bounds are recorded in operation-performance.md: 1 MiB specs
and results, 64 KiB attempts/events/result references, 256 KiB outbox payloads,
64-item outbox batches, 4,096 pending outbox rows, 50,000 snapshot rows, and
16,384 SQLite pages. Inspector output is capped at 256 KiB and also bounds rows,
findings, timeline entries, and per-operation reports. It reports unavailable
rather than materializing an unbounded journal.

## Privacy and retention

Payload and result JSON may contain caller data. Durable roots, logs, backups,
and inspector output require the same access control as their workspace and
session. Retention cannot remove unresolved operations, unacknowledged outbox
entries, active claims, referenced artifacts, graph ancestors, or task receipts.
Idempotency tombstones remain until the caller scope is explicitly closed or an
archival contract closes replay. There is no age-only delete operation.
