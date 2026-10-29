# 05 — Sessions, operation journal, permissions and local IPC

Status: evidence record. No code changed for this subsystem in this audit.
Research basis: F-09-01…04, F-11-01…04, F-12-01…03, F-20-03, F-36-01…08, F-37-01…06, F-38-01…12.
Research reports: `09-session-persistence.md`, `11-permission-modes.md`,
`36-session-recovery.md`, `37-permission-decisions.md`, `38-ide-ipc.md`.

## 1. Two stores, two jobs

| Store | Path | Role |
|---|---|---|
| Session history | `<session>.jsonl` | conversation, branches, compaction entries |
| Operation journal | `<workspace>/.davinci/operations/operations.sqlite3` | intent, attempts, effect status, results, outbox |

They are not replicas. `davinci-coding-agent` depends on `davinci-session` and **not**
on `davinci-session-sqlite` (`crates/davinci-coding-agent/Cargo.toml:24`); a configured
sqlite session backend is warned about and ignored (`settings.rs:1316-1338`).

## 2. Journal durability, in order

1. `persist_intent` → `admit` in one write transaction (`store.rs:147-160`).
2. Claim dispatch, then `Start` + `MarkEffectPossible` + `dispatch_claims.effect_started = 1`
   in one transaction — the **effect latch** (`coordinator.rs:546-617`).
3. Adapter runs.
4. `transition`: attempt state, event, result payload, outbox row and claim release in
   **one** transaction (`store.rs:243-388`, called from `adapters/tools.rs:290-297`).

Pragmas: `foreign_keys=ON`, `trusted_schema=OFF`, `synchronous=FULL`, WAL
(`store.rs:99-110`). Transactions are `Immediate` with an explicit `commit`
(`store.rs:855-866`). Fsync is delegated to SQLite; no manual `fsync` exists in
`store.rs`.

A write failure poisons the handle until reopen (`store.rs:846-880`; test
`operation_journal.rs:250`). Triggers forbid clearing `effect_started` or deleting the
mapping rows (`migrations.rs:329-343`).

**Guarantee boundary:** this is process-crash durability plus a reopen test
(`operation_journal.rs:171`). There is no power-loss or fault-injection test for the
journal.

## 3. Replay policy and the effect latch

`ReplayPolicy::{SafeToReplay, ReconcileBeforeReplay, NeverAutoReplay}`, default
`NeverAutoReplay` (`runtime/capabilities.rs:51-64`). Assignment is conservative: reads
safe; `write`/`edit`/`apply_patch` reconcile-first; `bash`/`powershell`/`agent`/task
tools never auto-replay; unknown tools never auto-replay (`:113-191`).

Three independent reasons nothing non-idempotent replays automatically:

1. journal replay is **result-only** (`adapters/tools.rs:301-313`);
2. an in-flight admission is refused (`turn.rs:2501`);
3. `StartSafeRetry` requires safe class **and** quiescent owner **and** claimed-unstarted
   **and** `EffectStatus::NotStarted` (`recovery.rs:101-115`). An `Unknown` attempt can
   never satisfy it.

Caveat: the journal stores `replay_policy` on the planned spec (`planner.rs:187`) but
`replay_result` does not branch on it; safety comes from `OperationState::Succeeded`
plus the effect latch.

## 4. Unknown state reaching the model

A failed completion records `EffectStatus::Unknown` (`adapters/tools.rs:255-263`); a
success on a non-read-only capability records `Possible`, a read-only one
`KnownNoEffect` (`:265-269`).

On load, an unresolved call is answered synthetically:

- `Outcome unknown: … It was not re-run.` when the journal cannot prove
  `NotStarted`/`KnownNoEffect`;
- `Not executed: …` otherwise.

`turn.rs:501-552`, including `details.toolRecovery = { outcome: unknown|not_executed,
source: operation_journal|tool_ledger|operation_journal_unreadable }`. An unreadable
journal is treated as unknown (fail-safe). Codemode surfaces the same as
`RECOVERY_REQUIRED` (`codemode/dispatch.rs:468`, `execution.rs:315`).

## 5. Idempotency keys and dedupe

`provider_key(session_id, call_id)` is length-prefixed and contains no run id, time or
process-local value (`planner.rs:192-198`), scoped `ProviderCall` (`:80-86`). Batch
children derive `"{parent_operation_id}:{child_index}"` from the durable parent
(`:104-136`).

Backed by `UNIQUE (root_namespace_id, caller_scope, caller_key)` (`migrations.rs:188`)
and `UNIQUE (root_namespace_id, session_id, wire_tool_call_id)` (`:313-314`).
Admission returns `Collision`, `ExistingResult` or `ExistingInFlight`
(`coordinator.rs:60-95`; tests `operation_idempotency.rs:97,156,184,212`).

## 6. Session JSONL

Append is `O_APPEND` + `write_all` + `sync_all` per line
(`davinci-session/src/lib.rs:469-474`). The first write takes an exclusive sidecar
lock and re-reads/adopts on-disk state (`:492-527`), then repairs a torn tail
(`:529-532`, `:647-697`) after backing up the discarded bytes
(`.torn-<uuid>.bak`, `:681-686`). A completed malformed line fails closed; only an
unterminated **last** line is tolerated (`:190-223`). Parent-directory fsync is
Unix-only (`:699-707`), so on Windows the directory entry is not fsynced.

Branches are explicit tree operations over the same file (`set_leaf` `:306-308`,
`branch_with_summary` `:312-334`, `fork` `:358-370`, reads `tree.rs:126-149`).

The compatibility tool ledger has its own `<name>.json.events.jsonl` with snapshot
compaction at `MAX_JOURNAL_BYTES` (`tool_ledger.rs:566-582`) and a torn-tail
truncation that does **not** preserve the discarded bytes (`:197-210`), unlike the
session log. Its dedupe window is bounded at 256 terminal records
(`tool_ledger.rs:17`, `:623-642`), so a very old call id can be re-admitted.

## 7. Load-time repair and pairing

`reconcile_unresolved_tool_calls` inserts answers after the assistant call they belong
to and persists only the newest exchange (`turn.rs:337-384`), then
`validate_loaded_tool_history` rejects orphan results and drops duplicates
(`:386-474`). A journal-known orphan is withheld from the provider; an unknown orphan
fails closed. Tests: `tool_result_pairing_recovery.rs:195,218,270,298,329,367,392`.

Pending results are republished before the provider continues
(`operation_publication.rs:893`); a replayed result whose originating call is absent
from history is withheld (`:862`).

## 8. Permissions

Modes: `ReadOnly`, `Ask`, `Edits`, `Auto`, `AlwaysApprove` (`permission.rs:22-33`),
aliases include `"danger-full-access" | "bypass" | "yolo"` → `AlwaysApprove`
(`:76`). `decide` evaluates in a fixed order: configured deny rules, the
`process_start` shell-deny bridge, `ReadOnly` hard denial, boundary/symlink/secret
checks, then `AlwaysApprove` allow (`:1239-1338`). **Deny always precedes allow**, so
`AlwaysApprove` cannot override a deny rule.

One choke point serves every route (`turn.rs:2569-2652`): direct calls, batch children
`"{id}#{n}"` (`batch.rs:179-193`), codemode children
(`turn.rs:3708-3767`), MCP calls (`:3102-3125`), host operations
(`host_operation.rs:132-170`). Dispatch re-binds the approval to what will run —
payload digest, workspace, root namespace, policy revision, cancellation, owner —
before the latch (`adapters/tools.rs:159-178`).

No approver and no responder means deny (`turn.rs:4448-4461`), not silent allow.

Caveats:

- `ask_user_question` returns `None` from the permission check (`turn.rs:4379`) because
  it is host interaction; the operations it later selects still pass the gates.
- The **compatibility** (non-journal) route does not re-bind a policy revision at
  dispatch; the sequential lock covers the ordering but not the revision comparison.
- No end-to-end test drives a deny rule through a batch child or a codemode child.

## 9. Subagent permissions

`worker_child_mode(parent, worktree)`: unknown parent → `ReadOnly`; `Ask` + worktree →
`Edits`; otherwise the parent mode (`main.rs:2826-2837`). Workers get no approver and
no responder (`:2746-2748`), so anything not allowed fails closed. Profile modes are
contained by a rank comparison `ReadOnly < Ask < Edits < Auto < AlwaysApprove`, and a
`ReadOnly` parent cannot grant mutation tools (`agent_profiles.rs:409-423`). Tool
scoping complements the mode (`subagent.rs:366`, `:438-440`).

## 10. Local IPC

Transport is a Unix-domain socket, compiled out unless the non-default
`experimental-ipc` feature is enabled (`main.rs:16-23`; `Cargo.toml:65`). The socket
lives in a private `0700` directory with mode `0600`, published by hard link with
dev/ino/mode verification (`davinci-server/src/unix.rs:16,161,210,252-255`), and a
stale socket is detected by connecting before reuse (`:389-446`).

Authentication is a **single connection-level preamble**:

```rust
let Some(expected) = expected.filter(|token| !token.is_empty()) else {
    return Ok(());
};
```
`davinci-server/src/lib.rs:1036-1046`

Consequences:

- an empty or absent token **disables** auth;
- the comparison is plain `==`, not constant-time;
- the token authenticates a stream, not each request;
- there is no peer-credential check (`SO_PEERCRED`/`getpeereid` appears nowhere);
- the server accepts exactly one connection (`experimental.rs:384-387`), and
  `--listen` binds an address into a static that nothing accepts on
  (`experimental.rs:12,256-277`; `main.rs:529-531`);
- there is no Windows named-pipe transport (grep found no `CreateNamedPipe`).

The `0600` socket is the effective boundary for other local users; the token adds
nothing when unset.

## 11. Cleanup on exit and signal

Covered by `Drop` on normal unwinding: the bound listener removes its paths
(`unix.rs:340-344`, `:364`), the job book kills all jobs (`jobs.rs:892-896`), the
supervisor stops (`jobs/supervisor/client.rs:377-381`), and job objects set
kill-on-close (`jobs/supervisor/platform.rs:98`, `sandbox/appcontainer.rs:793-798`).

Not covered:

- the SIGTERM/SIGHUP watcher restores the terminal, calls `dispose`, then
  `std::process::exit(143)` — destructors do not run (`shutdown.rs:37-45`), so a Unix
  socket inode and live jobs can survive (the socket is recoverable as stale; jobs
  are not reaped by this process);
- the foreground spawn job has no kernel kill-on-close; termination is only the
  explicit `job.terminate()` (`davinci-sys/src/process.rs:212-239`), so abrupt parent
  death leaves descendants running;
- `panic = "abort"` is not set in the workspace profile (`Cargo.toml:69-77`), so
  unwinding panics do run destructors; an abort or hard kill does not;
- torn session backups (`.torn-*.bak`) are never garbage-collected;
- the compatibility ledger truncates its torn tail without a backup.

## 12. Residual caveats

1. Journal durability is process-crash durability, not media durability.
2. The journal is append-only with hard caps (`MAX_OPERATIONS_PER_ROOT = 100_000`,
   `MAX_DATABASE_PAGES = 16_384`, `MAX_RESULT_BYTES = 1 MiB`,
   `store_api.rs:13-19`) and no pruning.
3. `execution_authority(id, journal_configured, journal_configured)` passes the same
   flag twice (`turn.rs:2327`), so the builtin-capability distinction is lost at the
   only production call site.
4. Empty IPC token means no auth; see §10.
5. The compatibility ledger's 256-record dedupe window is not a permanent guarantee.

## 13. Non-claims

No crash-consistency or durability measurement was run here. All statements above are
source plus existing tests; the tests named were not re-executed for this subsystem in
this session.
