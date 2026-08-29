# Phase 2 — Session types and SQLite writer-leases

## Goal

Port harness session types and the sqlite-node backend, including writer-leases and the shared conformance suite.

## TypeScript sources

- `vendor/pi/packages/agent/src/harness/session/`
- `vendor/pi/packages/session-backends/sqlite-node/`
- Tests: `writer-leases.test.ts`, `conformance.test.ts`, `001_initial.sql`

## Deliverables

- `pi-session`: `Entry`, `LaneRecord`, `SessionStorage`, `SessionRepository`, in-memory backend, conformance runner.
- `pi-session-sqlite`: schema `001_initial.sql`, migrations, writer-lease acquire/renew/release/delete, repository create/open/list/delete/fork, branch cache, facts, stats.
- Lease defaults: `ttlMs = 30000`, `heartbeatIntervalMs = 10000`.
- Exact errors:
  - `SQLite session {id} already has an active writer`
  - `SQLite session {id} writer lease was lost`
  - `writerLease.ttlMs must be positive`
  - `writerLease.heartbeatIntervalMs must be positive and less than ttlMs`
- `list()` must not mutate `writer_leases`.

## Done when

Writer-lease tests and session conformance tests pass under `cargo test -p pi-session -p pi-session-sqlite`.
