# Phase 2: Session SQLite & Writer Leases Specification

## Overview
Phase 2 ports the session persistence engine to Rust (`pi-session-sqlite`), featuring robust SQLite-backed session tracking, message histories, metadata indexing, and single-writer lease governance with TTL heartbeats.

## Key Invariants
1. **Writer Leases**: Only one writer process/thread can hold an active lease on a given session.
2. **Lease Expiration & Heartbeats**: Leases expire after a configurable TTL unless renewed.
3. **Transactional Parity**: Transactions mirror TypeScript reference semantics.
4. **Differential Fixtures**: Golden session logs evaluate identically across TS and Rust implementations.
