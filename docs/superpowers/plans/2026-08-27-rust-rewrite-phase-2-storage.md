# Phase 2: Session SQLite and Writer Leases

## Overview

Port `packages/session-backends/sqlite-node` writer-lease and repository semantics into `crates/pi-session-sqlite`.

## Schema

Apply `001_initial.sql` exactly: `sessions`, `entries`, `session_sequences`, `session_stats`, `branch_entries`, `lanes`, `records`, `lane_moves`, `facts`, `branch_tips`, `writer_leases`. Configure `PRAGMA journal_mode=WAL`, `synchronous=FULL`, `busy_timeout=5000`. Transactions use `BEGIN IMMEDIATE`.

## Writer leases

`WriterLease { ownerId, fence, expiresAtMs }`.

- **Acquire:** insert fence=1, or takeover when `expires_at_ms <= now` incrementing fence. Active row returns no lease.
- **Renew:** update expiry only when session + owner + fence match and lease is unexpired.
- **Release:** delete only matching owner+fence.
- **Delete:** unconditional session-id delete (session deletion).
- Defaults: `ttlMs=30000`, `heartbeatIntervalMs=10000` (must be positive and `< ttlMs`).
- Lost lease after fence takeover: next write fails with `SQLite session {id} writer lease was lost`.
- `list()` never acquires a writer lease.

## Repository

`create`, `open`, `list`, `delete`, `fork` (branch and tree), `repairBranchCache`. Every write renews the lease inside the transaction. Same-process reopen reuses the storage. Cross-process second writer fails until release.

## Tests that must pass

Fence takeover, heartbeat renew, competing writer rejection, list-without-claim, entry sequence assignment, lane isolation, fact latest-wins, branch/tree fork.
