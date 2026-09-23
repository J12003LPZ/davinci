# Operation journal capacity and performance

Task 22 records the bounded-storage contract and one reproducible local
reference measurement. The numbers below are evidence for this worktree and
build profile; they are not a cross-machine performance promise.

## Reference measurement

The ignored benchmark is intentionally small and offline. It uses a temporary
directory on the local Windows filesystem, 32 samples per scenario, and the
debug profile. The baseline only measures JSON encoding, while journaled
scenarios include the SQLite write/read and full durability barriers used by
the operation journal. The batch row measures four caller batches of eight
admissions; it does not claim that those admissions share one transaction.

Command:

```text
cargo test -p davinci-agent --test operation_capacity benchmark_journal_overhead_reference_workload --offline --locked -- --ignored --nocapture
```

Environment and raw output:

```text
benchmark=os-boundary os=windows arch=x86_64 profile=debug fs=local-tempdir
scenario=baseline-encode count=32 p50_us=1 p95_us=2 throughput_ops_s=286481.65
scenario=journaled-admission count=32 p50_us=781 p95_us=903 throughput_ops_s=1258.90
scenario=journaled-read count=32 p50_us=25 p95_us=36 throughput_ops_s=30683.67
scenario=journaled-mutation count=32 p50_us=1404 p95_us=1697 throughput_ops_s=663.02
scenario=journaled-batch count=4 p50_us=5872 p95_us=6205 throughput_ops_s=158.69
scenario=journaled-worker-read count=32 p50_us=79460 p95_us=140427 throughput_ops_s=12.49
```

The worker-read case intentionally exposes the bounded writer/read lane under
parallel pressure. `WriterBusy` is retried by the benchmark; the journal does
not enqueue an unbounded backlog. Release-profile and representative
production filesystem measurements remain release gates.

The initial review threshold for a future comparison is a greater-than-20%
p95 regression against the same workload, hardware, filesystem, and build
profile. That threshold does not permit removing a durable intent, result,
outbox, or recovery barrier to improve a number.

## Capacity and backpressure contract

The journal rejects data before opening a write transaction when a serialized
record is too large. It also uses a single nonblocking writer lane, bounded
outbox reads, and SQLite's page limit. Current limits are:

| Resource | Bound | Failure behavior |
|---|---:|---|
| Operation spec | 1 MiB | `OversizedRecord`; no admission |
| Operation attempt | 64 KiB | `OversizedRecord`; no transition |
| Operation event | 64 KiB | `OversizedRecord`; no transition |
| Result payload | 1 MiB | `OversizedRecord`; result/event/outbox remain unpublished |
| Result reference | 64 KiB | `OversizedRecord`; transaction is rejected |
| One outbox payload | 256 KiB | `OversizedRecord`; no partial publication |
| One outbox batch | 64 items | `OutboxBatchTooLarge` |
| Pending outbox per root | 4,096 items | `Capacity`; caller must drain/acknowledge |
| Snapshot rows per root | 50,000 rows | `SnapshotTooLarge`; durable rows stay intact |
| SQLite journal | 16,384 pages | `Capacity`; recovery remains required |
| Task journal record | 1 MiB | persistence error; recovery is required |
| Task journal file | 64 MiB | persistence error; recovery is required |
| Operation receipts | 4,096 per task registry | persistence error; old receipts are not silently evicted |
| Checkpoint blob | 16 MiB | `BlobTooLarge` |
| Checkpoint bytes per task | 256 MiB | `TaskCapacityExceeded`; correctness data is not evicted |

The operation writer returns `WriterBusy` when another durable read or write
holds the lane. Callers can retry with their own bounded policy; the journal
does not accumulate an internal unbounded queue. Durable transitions are
transactional, so an oversized result or outbox item cannot leave a new event,
result, or publication without its corresponding state transition.

`OperationJournal::capacity()` exposes operation, pending-outbox, and snapshot
occupancy with their configured maxima without materializing the journal. It is
an observation for admission/draining telemetry, not a license to evict rows.

Batch callers may group independent admissions at the orchestration layer, but
each operation still receives its own durable intent acknowledgement. Streamed
output chunks are not operation transitions and are not written as one durable
database transaction per token.

## Retention and evidence safety

Retention is correctness-first. Unresolved operations, pending outbox items,
referenced result artifacts, task receipts, and graph ancestors must remain
available for recovery. The graph run pruner now scans bounded checkpoint JSON
for persisted operation bindings and retains a terminal run when such a
reference is present. An unreadable or over-limit candidate is retained rather
than deleted through uncertainty. The existing ancestor, explicit pin, and
terminal-only rules still apply.

Idempotency receipts and tombstones have no automatic age-based deletion path;
output retention cannot make an old command executable again. Any future
archival process must close the caller scope and record the archive contract
before removing those records.

## Storage-failure behavior

The journal's SQLite trigger-injection tests cover a failed durable write: the
transaction rolls back and the in-memory journal is poisoned until reopen.
Checkpoint tests cover disk-full before blob publication and preserve the
caller-visible `DiskFull` result. Missing/corrupt result artifacts and
inspector behavior under corruption are covered by the publication and runtime
recovery suites. Interrupted graph checkpoint publication uses atomic write
and preserves the previous checkpoint. Physical quota exhaustion after an
external effect and a real filesystem power-loss simulation remain release or
CI evidence; the recovery state stays non-success until a receipt or
postcondition closes that uncertainty.

## Validation

The Task 22 focused checks were:

```text
cargo test -p davinci-agent --test operation_capacity --offline --locked
cargo test -p davinci-agent --test operation_capacity benchmark_journal_overhead_reference_workload --offline --locked -- --ignored --nocapture
cargo test -p davinci-coding-agent operation_referenced_terminal_run_is_retained_until_archived --offline --locked
```

The capacity target passed five tests and kept the benchmark ignored by
default; the explicit benchmark invocation passed and emitted the raw table
above. The graph retention regression passed. Existing warnings in the coding
agent crate were unchanged.
