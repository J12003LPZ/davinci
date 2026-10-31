# 03 - G-07 terminal completion status

Status: implemented and verified offline.
Files: `crates/davinci-agent/src/lib.rs`,
`crates/davinci-agent/src/verification_notice.rs`,
`crates/davinci-agent/src/events.rs`,
`crates/davinci-agent/src/completion_status_tests.rs`.

## 1. The problem

`CompletionEvidence` is the harness gate signal: it decides whether the turn gets
one verification reminder. It is advisory and it is not persisted. A host that
wanted to show the honest end state had two bad options: repeat the reminder text,
or trust the model's claim that the work was verified.

The gate had a second ambiguity. `CompletionEvidence::Unverified` meant both
"verification ran and is now stale" and "verification was never observed at all".
Those are operationally different: the first suggests re-running the check, the
second suggests the worker never had a usable verification path.

## 2. Two types, two jobs

| type | job | persisted |
| --- | --- | --- |
| `CompletionEvidence` | internal gate/reminder signal | mutation state only |
| `CompletionStatus` | terminal, user-visible status | yes, as a session custom entry |

`CompletionStatus` has six values:

| value | meaning |
| --- | --- |
| `Verified` | a passing check covers the latest mutation generation |
| `Partial` | passing checks cover only part of the changed paths |
| `Unverified` | changes exist and the check is stale or incomplete |
| `VerificationFailed` | the latest verification attempt failed |
| `VerificationUnavailable` | changes exist and no verification was ever observed |
| `NoChanges` | the turn made no repository changes |

`is_verified()` is true only for `Verified`.

## 3. Derivation

`CompletionStatus::from_evidence(evidence, state)`:

```text
Verified            -> Verified
Partial             -> Partial
VerificationFailed  -> VerificationFailed
NotRequired         -> NoChanges
Unverified          -> VerificationUnavailable  if state.latest_evidence.is_none()
                                                  and state.last_verification.is_none()
                       Unverified                otherwise
```

`Agent::completion_status()` calls `completion_evidence()` and then this mapping.
It therefore uses the existing mutation accounting: generation increments,
scoped coverage, background-shell invalidation, failed paths, unresolved effects
and late results are all handled by the same code that already drives the gate.

Examples that follow from the existing rules:

- edit, then `cargo test` passes -> `Verified`;
- edit, then `cargo test` fails -> `VerificationFailed`;
- edit, no command -> `VerificationUnavailable`;
- pass, then another edit -> `Unverified` (stale, not unavailable);
- a verification call that started but never returned (timeout/unavailable) ->
  `Unverified`, because `last_verification` exists;
- a late result from an earlier generation is ignored;
- no mutation -> `NoChanges`.

## 4. Turn-end recording

`emit_verification_notice` now starts with:

```rust
self.record_terminal_completion_status();
let status = self.completion_status();
```

Recording happens before the match, so `Verified` and `NoChanges` are persisted
even though they emit no warning. The notice text adds one case:

```text
Verification was not observed: changes were made but no verification command ran.
```

`AgentEvent::VerificationNotice.status` changed from `CompletionEvidence` to
`CompletionStatus`. This is a compile-time break for exhaustive consumers, by
design: a host can no longer accidentally treat a gate signal as the terminal
state. The serialized names for `verified`, `partial`, `unverified` and
`verification_failed` are unchanged; `verification_unavailable` is new.

## 5. Tests

```text
cargo test -p davinci-agent --lib --offline completion_status
EXIT=0
test result: ok. 13 passed; 0 failed; 0 ignored
```

The 13 tests are:

| test | property |
| --- | --- |
| `edit_then_passing_verification_is_verified` | only a covering pass verifies |
| `edit_then_failing_verification_is_failed` | failure is its own state |
| `edit_without_verification_is_not_verified` | absence is unavailable |
| `verification_then_another_edit_is_stale` | later edit downgrades to unverified |
| `verification_of_an_earlier_generation_does_not_cover_a_later_edit` | late results are ignored |
| `verification_timeout_is_not_verified` | started but unfinished is unverified |
| `verification_command_unavailable_is_not_verified` | missing runner is unverified |
| `multiple_independent_edits_require_fresh_verification` | each generation needs a fresh pass |
| `session_interruption_before_verification_persists_unverified` | abort path records and restores |
| `resume_keeps_a_verified_mutation_verified` | verified survives reload |
| `resume_with_unverified_changes_never_reports_verified` | mutation state survives reload |
| `compaction_keeps_pending_verification_state` | compaction preserves the status |
| `user_stop_without_verification_is_recorded_as_unverified` | explicit stop records |

No test in this file contains a model claim. Every transition is driven by
`record_successful_mutation_paths`, `remember_verification_command`,
`record_verification_command` or `record_verification_assessment`.

The full `davinci-agent` library suite is 1894 passed, 0 failed.

## 6. Compatibility

- Older sessions have no `completion_status` entry;
  `last_recorded_completion_status()` returns `None` and the live state is
  derived from the persisted mutation state.
- Old `VerificationNotice` JSON still decodes because the overlapping status
  names are unchanged.
- `CompletionEvidence` itself, its gate, and the one-reminder budget are
  untouched.

## 7. Limits

`CompletionStatus` reports the evidence DaVinci actually has. It is not a proof
that the code is correct: a passing check can be insufficient, and a scenario
without a meaningful oracle is handled by G-06, not here. The status is scoped to
the latest mutation generation on the current session branch.
