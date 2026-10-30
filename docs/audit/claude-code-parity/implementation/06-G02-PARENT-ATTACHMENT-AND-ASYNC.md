# 06 - G-02 parent attachment, async delivery and what the parent sees

Status: implemented and verified offline.
Files: `crates/davinci-agent/src/subagent.rs`,
`crates/davinci-agent/src/runtime/workflow/executor.rs`,
`crates/davinci-coding-agent/src/main.rs`,
`crates/davinci-agent/src/lib_tests.rs`.

## 1. Host-owned slot on the request

`SubagentRequest` gained:

```rust
/// Host-owned slot the worker fills with a receipt after its turn. Never
/// accepted from model-authored agent tool JSON.
pub receipt_slot: Option<crate::subagent_receipt::SubagentReceiptSlot>,
```

The field is populated by `run_tool_with_spawner`, not by deserializing model
arguments. The model-facing schema only exposes `prompt`, `tasks`, `tools`,
`mode`, `name` and the other pre-existing fields; there is no path from tool JSON
to this slot. A worker therefore cannot inject `status: verified`.

`run_tool_with_spawner` creates one `receipt_slot()` per spec:

```rust
for (position, spec) in specs.iter().enumerate() {
    let child_agent_id = AgentId::new();
    let receipt_slot = crate::subagent_receipt::receipt_slot();
    // ...
    receipt_slot: Some(receipt_slot.clone()),
    // ...
    receipt_slots.push(receipt_slot);
}
```

Parallel workers therefore never share a slot, and the order of the slot vector
matches the order of results.

## 2. Who writes the receipt

For the real CLI host, `davinci-coding-agent/src/main.rs` wraps every worker
turn:

```rust
let workspace = req.worktree_path.clone().unwrap_or_else(|| cwd.to_path_buf());
let receipt_slot = req.receipt_slot.clone();
let mut turn = |prompt: &str| {
    if teammate { refresh_worker_settings(&mut child, profile_allows_codemode); }
    let before = davinci_agent::WorkspaceSnapshot::capture(&workspace).ok();
    let result = run_worker_turn(parsed, &mut child, prompt);
    let after = davinci_agent::WorkspaceSnapshot::capture(&workspace).ok();
    if let Some(slot) = &receipt_slot {
        let receipt = davinci_agent::VerificationReceipt::from_worker(
            child.completion_status(),
            &child.mutation_verification_state(),
            before.as_ref(),
            after.as_ref(),
            Some(&workspace),
        );
        davinci_agent::publish_subagent_receipt(slot, receipt);
    }
    result
};
```

The snapshot is taken after the final mutation and after the worker's last
verification attempt, and the receipt is derived from the same worker agent that
ran the turn. The host is the only writer.

`crates/davinci-agent/src/runtime/workflow/executor.rs` constructs an internal
`SubagentRequest` with `receipt_slot: None` because no workspace snapshot is
taken on that path. It cannot fabricate a receipt; if a receipt is consumed from
such a request, `take(None)` returns `Unavailable`.

## 3. Synchronous single worker

After `run_journaled_subagent` and before the result is returned:

```rust
let receipt = VerificationReceipt::take(receipt_slots.first());
```

The host block is appended to `content`, and the JSON form is added to
`details.verificationReceipt`. `take` removes the receipt, so a second read does
not repeat a stale observation.

## 4. Several synchronous workers

Each result keeps its own receipt:

```rust
let receipts = receipt_slots.iter()
    .map(|slot| VerificationReceipt::take(Some(slot)))
    .collect::<Vec<_>>();
```

Each section gets its own `[verification receipt]` block, and
`details.verificationReceipts` is an array in request order. A worker with no
receipt is `Unavailable`, not silently omitted.

## 5. Async and background workers

The async closure wraps the outcome before reporting:

```rust
let outcome = run_journaled_subagent(&req_clone, &runner_clone, operation.as_ref());
let receipt = VerificationReceipt::take(req_clone.receipt_slot.as_ref());
let outcome = with_receipt_outcome(outcome, &receipt);
// progress.finish(...)
// report_async_outcome(...)
```

`report_async_outcome` passes the wrapped text to `report_to_lead`, which calls
`mailbox.record_worker_result(&message)` before `mailbox.send(message)`.
`record_worker_result` emits `RuntimeEvent::AgentResultRecorded` with the full
message. The receipt is therefore inside the durable result record before the
lead is notified, and the existing at-most-once delivery behaviour is unchanged.
The same wrapper applies to a teammate's `report_status_to_lead` path.

## 6. What the parent sees

Example for a worker that changed a file but produced no observed check:

```text
<worker answer>

[verification receipt] status: unverified
detail: the worker's verification is stale or incomplete
changed files (1): src/lib.rs
mutation generation: 1 (unverified)
```

Example for a worker that wrote no receipt at all:

```text
[verification receipt] status: unavailable
detail: the host did not observe the worker's completion state
changed files: unknown
mutation generation: 0 (unverified)
```

The block is host text appended by the parent, not worker-authored text. A worker
whose prose says "all tests pass" but whose slot is empty is reported
`unavailable`; a worker whose slot says `unverified` is reported `unverified`
even if its prose claims success.

## 7. Tests

```text
cargo test -p davinci-agent --lib --offline subagent
EXIT=0
test result: ok. 74 passed; 0 failed; 0 ignored
```

New integration tests in `subagent.rs`:

| test | property |
| --- | --- |
| `worker_text_cannot_replace_the_host_receipt` | a verified host receipt survives false/true prose and is exposed in details |
| `missing_host_receipt_is_unavailable_despite_success_text` | no slot means `unavailable`, and `verified` never appears |
| `parent_keeps_an_unverified_receipt_when_the_worker_claims_success` | host `unverified` beats worker prose |
| `each_parallel_worker_gets_its_own_receipt` | per-task slots and per-task blocks |

Existing exact-output tests were updated, not weakened, because every worker
result now ends with a host receipt block:

- `a_canned_runner_returns_the_reply`
- `parallel_workers_in_one_message_overlap_and_answer_in_order`
- `notice_preserves_final_answer_provider_history_and_saved_session`

The first two now assert the reply prefix and the `status: unavailable` block
instead of equality with the raw reply.

## 8. Limits

- `WorkspaceSnapshot::capture` is best-effort per turn. A snapshot error becomes
  `Unavailable`; it never becomes `Verified`.
- The receipt describes one worker turn, not a multi-turn team history.
- The workflow executor path has no snapshots in this wave, so it reports
  `Unavailable` rather than inventing observation.
- Session interruption before the host publishes a receipt yields no receipt,
  which the parent reports as `Unavailable`.
