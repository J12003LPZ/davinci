# 04 - G-07 durable status, reload, compaction and branch rewind

Status: implemented and verified offline.
Files: `crates/davinci-agent/src/lib.rs`,
`crates/davinci-agent/src/completion_status_tests.rs`.

## 1. Record shape

The terminal status is stored as one session entry:

```json
{
  "entry_type": "custom",
  "custom_type": "completion_status",
  "extra": {
    "data": {
      "status": "unverified",
      "evidence": "unverified",
      "mutation_generation": 1,
      "verified_generation": null,
      "recorded_at_ms": 1760000000000
    }
  }
}
```

`CompletionStatusRecord` keeps both the terminal status and the gate evidence it
came from, plus the generation pair, so an operator can see why the status is
what it is without recomputing it.

## 2. Write path

`Agent::record_terminal_completion_status()`:

1. calls `persist_mutation_verification()` first, so the branch never contains a
   completion status that refers to an unpersisted mutation state;
2. builds the record from the live state;
3. returns early if the last persisted record has the same status, evidence and
   generation pair (`same_terminal_state`);
4. appends the custom entry through `session.append_entry`, letting the session
   assign id, parent, sequence and timestamp;
5. updates `persisted_completion_status` only when the append succeeded.

A failed append leaves the in-memory marker unchanged, so the next turn boundary
can retry. Recording is local session work; it does not send a provider request
and it does not create an assistant message.

## 3. Read path

`restore_branch_state()` calls `restore_completion_status()` next to the existing
mutation and tool-exposure restores. That function:

- walks `davinci_session::branch_entries(&session.entries, session.leaf_id)`;
- scans in reverse for the newest `custom` entry whose `custom_type` is
  `completion_status`;
- reads `extra["data"]` and deserializes `CompletionStatusRecord`.

Because the search is branch-scoped and reverse-ordered, a rewind or a fork does
not inherit a sibling branch's status, and a later record supersedes an earlier
one. If the entry is absent or malformed, the stored record is `None` and the live
status is re-derived from the restored mutation state.

## 4. Interaction with mutation invalidation

`restore_mutation_verification()` runs first and invalidates any mutation whose
effect marker was left unresolved. The completion status is then derived from
that corrected state. A session interrupted mid-effect therefore cannot reload as
verified, even if a verified record was written before the crash.

## 5. What the tests pin down

Persistence-related cases from `completion_status_tests.rs`:

| test | persisted behaviour |
| --- | --- |
| `session_interruption_before_verification_persists_unverified` | abort records `verification_unavailable`, `ensure_session_persistence` flushes it, and a fresh agent loading the JSONL sees the same record with `mutation_generation == 1` |
| `resume_keeps_a_verified_mutation_verified` | a verified turn emits no notice, persists a verified record, and reload reports `Verified` |
| `resume_with_unverified_changes_never_reports_verified` | mutation state alone survives reload; no terminal record exists, and the resumed live status is `VerificationUnavailable` |
| `compaction_keeps_pending_verification_state` | a real `compact()` keeps the live status and the recorded status at `VerificationUnavailable` |
| `user_stop_without_verification_is_recorded_as_unverified` | explicit stop goes through the same turn-end recorder |

Run:

```text
cargo test -p davinci-agent --lib --offline completion_status
EXIT=0
test result: ok. 13 passed; 0 failed; 0 ignored; 1881 filtered out
```

The full `davinci-agent` library suite is 1894 passed, 0 failed.

## 6. Compatibility and failure behaviour

- Sessions written before this change simply have no entry; there is no
  migration and no invented status.
- Unknown fields in a future record are ignored by serde, and a record that
  cannot be deserialized is treated as absent rather than as truth.
- `record_terminal_completion_status` is `pub(crate)`; hosts read
  `completion_status()` or `last_recorded_completion_status()`.
- The record is written at the turn boundary in `emit_verification_notice`, so a
  model that claims success mid-turn cannot create it.

## 7. Limits

The record describes the latest mutation generation on the current branch. It is
not a repository-wide audit: switching branches, rewinding, or editing outside
the session invalidates the assumption. The mutation state restore remains the
authority for whether the observed generation still applies.
