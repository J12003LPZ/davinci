# 10 — Implementation record: unknown MCP outcomes

Status: implemented, tested, not committed.
Scope: `crates/davinci-mcp`, `crates/davinci-agent`, one test fixture.
Trigger: F-20-03 (class E) and the Phase D guarantee "an unknown outcome is not a
failure".

## 1. Defect

Before this change, `Client::call_tool` returned `Error::Transport` for every transport
fault and `Error::Protocol` for an undecodable reply. `McpRegistry` mapped both to
`ToolError::Failed`, and the turn loop turned that into:

```
content = "…transport failed…"
is_error = true
details = None
```

Two consequences:

1. **The model read a definite failure** for a call the server may already have
   executed. F-20-03 is the same defect in Claude Code 2.1.310: a stdio crash after
   commit and before commit produce the same error.
2. **The compatibility ledger stored a confirmed failure**
   (`AttemptOutcome::Failed`), which is the state from which a replay-safe tool may be
   retried. A call whose effect existed could therefore be re-executed by a later
   recovery path.

The journal route was already correct (it records `EffectStatus::Unknown`, and retry
requires `NotStarted`). The defect was reachable only on the non-journal path, but the
model-visible text was wrong on both.

## 2. Design

### 2.1 New error classes

`crates/davinci-mcp/src/lib.rs`:

```rust
Ambiguous(String)    // the request may have reached the server
NotDelivered(String) // the request provably never reached it
```

with `may_have_run()` and `definitely_not_delivered()`. `drops_server()` remains
`!is_rpc()`, so both new classes still evict the server.

### 2.2 One classification point

`Client::call_tool` is the only place that decides, because it is the only place where
"did this call run?" is a meaningful question:

| Outcome | Class |
|---|---|
| JSON-RPC error reply | `Rpc` (known failure) |
| `NotDelivered` from the transport | `NotDelivered` |
| anything else from the transport | `Ambiguous` |
| reply that does not decode into `CallToolResult` | `Ambiguous` |

The default is deliberately the safe one: *if it is not provably undelivered and not an
RPC reply, it is ambiguous.*

`tools/list`, `read_resource` and the handshake keep their old classes. They are not
tool calls, so "may have run" would be meaningless.

### 2.3 Stdio writer: why a line is incomplete

`crates/davinci-mcp/src/stdio.rs` previously reported every write failure as
`WriteError::Io`, which was then mapped to `NotDelivered`. That was too coarse: a
complete line whose `flush` failed may have been read by the server. The writer now
distinguishes:

| Variant | Meaning | Class |
|---|---|---|
| `Backlog` | queue full, nothing offered to the pipe | `NotDelivered` |
| `Incomplete` | `write_all` failed / writer stopped / encode failed → the line cannot be parsed by a line-framed server | `NotDelivered` |
| `Unflushed` | whole line written, `flush` failed | `Ambiguous` |
| `Unconfirmed` | writer stopped after accepting the job | `Ambiguous` |
| `TimedOut` | caller deadline | `Ambiguous` (at the call boundary) |

The argument for `Incomplete` → not delivered: `write_all` only returns an error after a
`write` accepted zero bytes, so at least one byte of the line is missing; MCP stdio is
line-framed, so a conforming server never dispatches a partial line. `02` §7 records the
residual assumption (a non-conforming server).

### 2.4 Agent side

| File | Change |
|---|---|
| `crates/davinci-agent/src/tools.rs` | `ToolError::MayHaveRun(String)`; `ToolError::into_tool_result(tool)` appends `Outcome unknown: … It was not retried. Check the server or target state before calling it again.` and sets `details = { outcome_unknown: true, tool }` |
| `crates/davinci-agent/src/mcp.rs` | `Inner::failed` returns `MayHaveRun` when `err.may_have_run()`, after evicting the server |
| `crates/davinci-agent/src/turn.rs` | the catch-all uses `into_tool_result(name)`; the non-journal ledger branch reads `outcome_unknown` and calls `record_unknown_outcome` instead of `record_failure` |
| `crates/davinci-agent/src/tool_ledger.rs` | `record_unknown_outcome` writes terminal `status=Failed` with `outcome=StartedUnknown` |

`status=Failed` because nothing is still in flight. `outcome=StartedUnknown` because the
effect is unknown. Recovery maps `StartedUnknown` to `Stop`/`ReplayBlocked`, never
`Retry` and never `Replay`; `get_completed_result` returns `None`, so the call cannot be
replayed as a completed result.

### 2.5 What was deliberately not changed

- No retry, no reconnect, no re-dial. DaVinci has no safe non-delivery signal beyond the
  one added here, and `02` §8 lists the preconditions for adding one.
- No `_meta` invocation id (G-12). It is a precondition for a future retry, not part of
  this fix.
- No change to journal `replay_policy` handling, IPC, permissions, worktrees or evals.
- No change to model selection, thinking level or auth.

## 3. Tests written first

The order was: write the failing assertion, watch it fail for the right reason, then
change the code.

| # | Test | First result | After the fix |
|---|---|---|---|
| 1 | `a_lost_tool_reply_is_ambiguous_and_a_refused_call_is_not` (`davinci-mcp/src/lib.rs`) | compile error: no `may_have_run` | pass |
| 2 | `a_line_refused_before_queueing_is_provably_not_delivered` (`stdio.rs`) | pass only after the writer split, with a deterministic gated pipe | pass |
| 3 | `a_half_written_line_cannot_have_run` (`stdio.rs`) | — | pass |
| 4 | `a_line_written_but_not_flushed_is_an_unknown_outcome` (`stdio.rs`) | — | pass |
| 5 | `a_lost_mcp_reply_becomes_may_have_run_not_failed` (`agent/src/mcp.rs`) | failed: registry returned `Failed` | pass |
| 6 | `an_error_that_may_have_reached_the_tool_is_reported_as_unknown` (`agent/src/tools.rs`) | compile error: no `into_tool_result` | pass |
| 7 | `an_unknown_outcome_is_not_recorded_as_a_confirmed_failure` (`tool_ledger.rs`) | compile error: no `record_unknown_outcome` | pass |

Two attempts were discarded during the cycle and are recorded here for honesty:

- a first version of test 2 flooded the real `ping` child with 8 MB payloads and waited
  for the queue to fill. It was non-deterministic (needed ~60 calls, and the writer
  drained abandoned jobs), so it was replaced by the gated-pipe unit test;
- an early assertion that a follow-up call to a blocked stdin is `NotDelivered` was
  wrong: that call times out while its line is being written, so it is `Ambiguous`. The
  final test asserts `Ambiguous` behaviour only through the real-child timing tests
  (`wor51_*`), which already existed.

## 4. Existing expectations that had to change

Both changes make the test match the new, safer contract. They are called out so a
reviewer can reject them independently.

| Test | Before | After | Why |
|---|---|---|---|
| `malformed_correlated_reply_is_a_protocol_error` (`davinci-mcp/src/lib.rs`) | expects `Error::Protocol` | renamed `…is_an_ambiguous_outcome`; expects `Ambiguous`, `may_have_run`, `drops_server` | the server replied to a `tools/call` with an undecodable envelope; it may have executed the call |
| `delayed_response_loss_is_not_no_effect_or_an_automatic_retry` (`davinci-agent/tests/common/codemode.rs`) | expects `Err(Error::Transport)` | expects `may_have_run` and not `definitely_not_delivered`, effect count still 1 | same reason: the fixture's effect ran and the reply was lost |

## 5. Risks and trade-offs

| Risk | Severity | Mitigation / note |
|---|---|---|
| The model may now under-retry a call that could have been retried safely | low | the text says "check the state before calling again", not "do not call"; a refused write (`NotDelivered`) keeps the ordinary failure text so the model is free to re-issue |
| A non-conforming server that acts on partial lines breaks the `Incomplete` argument | low | documented in `02` §7; a conforming server cannot |
| Classification is one layer deep: `call_full` callers that ignore `MayHaveRun` would lose the signal | low | the only production caller is the guarded dispatcher; the codemode fixture now asserts it |
| The ledger now has a terminal record with a non-terminal-looking outcome | low | `recover_attempt` handles `StartedUnknown` explicitly; a test asserts `Stop`/`ReplayBlocked` |
| `NotDelivered` evicts the server like any other transport error | acceptable | fail-closed is the existing policy; the message tells the model the call was not sent |

## 6. Diff size

Final `git diff --numstat` (test code included; docs excluded):

```
crates/davinci-mcp/src/lib.rs                   +72  −5
crates/davinci-mcp/src/stdio.rs                +148 −14
crates/davinci-agent/src/mcp.rs                 +59  −0
crates/davinci-agent/src/tools.rs               +55  −0
crates/davinci-agent/src/tool_ledger.rs         +61  −0
crates/davinci-agent/src/turn.rs                +12  −6
crates/davinci-agent/tests/common/codemode.rs    +5  −1
                                               ───────────
                                               +412 −26
```

`crates/davinci-mcp/tests/stdio_write_timeout.rs` is byte-identical to HEAD
(`git diff --quiet` → 0) after the non-deterministic test was moved into
`stdio.rs` as a unit test; see `11-VERIFICATION-RECORD.md` §4.

## 7. Rollback

The change is additive except for the two expectations in §4:

1. revert `tools.rs` (`MayHaveRun`, `into_tool_result`) and `turn.rs` to the previous
   catch-all — the model text returns to a plain failure;
2. revert `mcp.rs` `Inner::failed` — the registry returns `Failed`;
3. revert `tool_ledger.rs` — no `record_unknown_outcome` call remains;
4. the MCP crate can keep the new classes (they are inert without consumers) or revert
   them together with the two tests.

No data migration is involved: the ledger JSONL accepts the outcome value it already
had (`StartedUnknown`), and the session format is unchanged.

## 8. Review checklist

- [ ] Is "unknown" the right default for a transport error at the `call_tool` boundary?
- [ ] Is `Incomplete` → `NotDelivered` acceptable given line framing?
- [ ] Should `NotDelivered` keep the server alive instead of evicting it?
- [ ] Is `status=Failed, outcome=StartedUnknown` the right terminal pair for the ledger?
- [ ] Are the two changed expectations in §4 acceptable?
