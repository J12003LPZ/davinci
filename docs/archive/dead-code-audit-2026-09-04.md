# Dead Code and Uncompiled Source Island Audit (2026-09-04)

## Overview

As part of Ecosystem Release Gate D (Proof + CI + Hygiene), this audit investigates and documents candidate uncompiled source files and orphaned legacy modules in the codebase prior to any deletions.

Per repository invariants:
1. Every candidate must be audited with structural evidence before removal.
2. Code that is uncompiled, unreferenced by active modules, and not required by runtime migrations or historical test harnesses is evaluated for deletion or archival.
3. Cleanup must proceed in independently verified batches where compiler and test suites pass after each batch.

---

## Audit Evidence Matrix

### 1. Archived Non-Workspace Crate: `davinci-core`

- **path**: `crates/davinci-core/`
- **module declared**: no (separate crate directory, not declared in workspace `Cargo.toml`)
- **workspace member**: no
- **runtime/migration reader**: no
- **repository reference count and purpose**: 3 references. Referenced in `CLAUDE.md` gotchas as an uncompiled legacy archive crate, and imported in the uncompiled legacy file `crates/davinci-session-sqlite/src/leases.rs`. Early experimental Phase 0/1 Rust core prototype.
- **decision**: `archive documentation / keep`
  - *Rationale*: Kept as an explicit archive crate root for historical reference; its `crates/davinci-core/README.md` already clearly documents it as an uncompiled historical archive crate. No workspace build or test commands touch it.

---

### 2. Session Island: `crates/davinci-session/src/{backend.rs, conformance.rs, jsonl.rs, memory.rs}`

#### Candidate 2.1: `crates/davinci-session/src/backend.rs`
- **path**: `crates/davinci-session/src/backend.rs`
- **module declared**: no (`crates/davinci-session/src/lib.rs` declares only `codec`, `discovery`, `errors`, `jsonl_repo`, `repo`, `tree`, `types`)
- **workspace member**: yes (file resides inside workspace member `davinci-session`)
- **runtime/migration reader**: no (neither `JsonlSession` nor `JsonlSessionRepo` nor sqlite session backends import or call this file)
- **repository reference count and purpose**: 2 references (referenced only in dead `crates/davinci-session/src/conformance.rs` and dead `crates/davinci-session-sqlite/src/repo.rs`). Contains early prototype `SessionRepository` and `BackendError` trait abstractions superseded by `crates/davinci-session/src/repo.rs`.
- **decision**: `delete`

#### Candidate 2.2: `crates/davinci-session/src/conformance.rs`
- **path**: `crates/davinci-session/src/conformance.rs`
- **module declared**: no (not in `crates/davinci-session/src/lib.rs`)
- **workspace member**: yes
- **runtime/migration reader**: no
- **repository reference count and purpose**: 2 references (referenced only by dead `jsonl.rs` and `backend.rs`). Test harness for the legacy `SessionRepository` trait. Active tests reside in `crates/davinci-session/src/lib.rs`, `jsonl_repo.rs`, and `repo.rs`.
- **decision**: `delete`

#### Candidate 2.3: `crates/davinci-session/src/jsonl.rs`
- **path**: `crates/davinci-session/src/jsonl.rs`
- **module declared**: no (not in `crates/davinci-session/src/lib.rs`)
- **workspace member**: yes
- **runtime/migration reader**: no
- **repository reference count and purpose**: 1 reference (referenced in dead `conformance.rs`). Early standalone JSONL parser superseded by `crates/davinci-session/src/jsonl_repo.rs` and `JsonlSession` in `lib.rs`.
- **decision**: `delete`

#### Candidate 2.4: `crates/davinci-session/src/memory.rs`
- **path**: `crates/davinci-session/src/memory.rs`
- **module declared**: no (not in `crates/davinci-session/src/lib.rs`)
- **workspace member**: yes
- **runtime/migration reader**: no
- **repository reference count and purpose**: 0 external references. Early memory repo superseded by `InMemorySessionRepo` in `crates/davinci-session/src/repo.rs`.
- **decision**: `delete`

---

### 3. Agent Island: `crates/davinci-agent/src/loop_.rs`

#### Candidate 3.1: `crates/davinci-agent/src/loop_.rs`
- **path**: `crates/davinci-agent/src/loop_.rs`
- **module declared**: no (`crates/davinci-agent/src/lib.rs` declares `apply_patch`, `batch`, `branch`, `compaction`, `context`, `edit_diff`, `events`, `evidence`, `file_mutation_queue`, `images`, `jobs`, `mcp`, `notebook`, `permission`, `pruning`, `queues`, `scheduler`, `skills`, `stats`, `subagent`, `templates`, `todo`, `tool_ledger`, `tools`, `turn`, `web`; `loop_` is not declared)
- **workspace member**: yes (inside workspace member `davinci-agent`)
- **runtime/migration reader**: no
- **repository reference count and purpose**: 1 reference (mentioned as dead code in `CLAUDE.md`). Monolithic event loop prototype superseded by `crates/davinci-agent/src/turn.rs` (`Agent::run_loop_inner`).
- **decision**: `delete`

---

### 4. SQLite Island: `crates/davinci-session-sqlite/src/{leases.rs, repo.rs, schema.rs}`

#### Candidate 4.1: `crates/davinci-session-sqlite/src/leases.rs`
- **path**: `crates/davinci-session-sqlite/src/leases.rs`
- **module declared**: no (`crates/davinci-session-sqlite/src/lib.rs` declares only `branch_cache`)
- **workspace member**: yes
- **runtime/migration reader**: no
- **repository reference count and purpose**: 1 reference (in dead `repo.rs`). Threaded background lease renewal prototype using `davinci-core`. Active non-blocking lease management is implemented directly in `crates/davinci-session-sqlite/src/lib.rs` (`SqliteSessionStore::acquire_writer_lease`, `renew_writer_lease`, `release_writer_lease`).
- **decision**: `delete`

#### Candidate 4.2: `crates/davinci-session-sqlite/src/repo.rs`
- **path**: `crates/davinci-session-sqlite/src/repo.rs`
- **module declared**: no (not declared in `lib.rs`)
- **workspace member**: yes
- **runtime/migration reader**: no
- **repository reference count and purpose**: 0 external references. Implementation of obsolete `davinci_session::backend::SessionRepository` trait. Superseded by `SqliteSessionStore` methods in `lib.rs`.
- **decision**: `delete`

#### Candidate 4.3: `crates/davinci-session-sqlite/src/schema.rs`
- **path**: `crates/davinci-session-sqlite/src/schema.rs`
- **module declared**: no (not declared in `lib.rs`)
- **workspace member**: yes
- **runtime/migration reader**: no
- **repository reference count and purpose**: 0 external references. Hardcoded DDL string `INITIAL_SCHEMA`. Active migrations are managed via `migrations/001_initial.sql` included in `lib.rs`.
- **decision**: `delete`

---

## Summary of Decisions

| Path | Crate | Module Declared | Workspace Member | Runtime Reader | Decision |
|---|---|---|---|---|---|
| `crates/davinci-core/` | `davinci-core` | No | No | No | `archive documentation / keep` |
| `crates/davinci-session/src/backend.rs` | `davinci-session` | No | Yes | No | `delete` |
| `crates/davinci-session/src/conformance.rs` | `davinci-session` | No | Yes | No | `delete` |
| `crates/davinci-session/src/jsonl.rs` | `davinci-session` | No | Yes | No | `delete` |
| `crates/davinci-session/src/memory.rs` | `davinci-session` | No | Yes | No | `delete` |
| `crates/davinci-agent/src/loop_.rs` | `davinci-agent` | No | Yes | No | `delete` |
| `crates/davinci-session-sqlite/src/leases.rs` | `davinci-session-sqlite` | No | Yes | No | `delete` |
| `crates/davinci-session-sqlite/src/repo.rs` | `davinci-session-sqlite` | No | Yes | No | `delete` |
| `crates/davinci-session-sqlite/src/schema.rs` | `davinci-session-sqlite` | No | Yes | No | `delete` |

---

## Independent Deletion Batches for Task 8

1. **Batch 1 (Session Island)**:
   - Remove `crates/davinci-session/src/{backend.rs, conformance.rs, jsonl.rs, memory.rs}`
   - Verify with `cargo check --workspace` and `cargo test -p davinci-session`
   - Commit: `chore(session): remove audited uncompiled legacy sources`

2. **Batch 2 (Agent Island)**:
   - Remove `crates/davinci-agent/src/loop_.rs`
   - Verify with `cargo check --workspace` and `cargo test -p davinci-agent`
   - Commit: `chore(agent): remove audited uncompiled legacy loop source`

3. **Batch 3 (SQLite Session Island)**:
   - Remove `crates/davinci-session-sqlite/src/{leases.rs, repo.rs, schema.rs}`
   - Verify with `cargo check --workspace` and `cargo test -p davinci-session-sqlite`
   - Commit: `chore(session-sqlite): remove audited uncompiled legacy sources`
