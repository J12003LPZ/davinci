# 02 — MCP replay safety and unknown outcomes

Status: one fix implemented and tested (`10-IMPLEMENTATION-RECORD.md`).
Research basis: F-20-01…10, F-31-01…11, F-42-01…07, F-06-06…11.
Raw evidence: `re-claude-code/_artifacts/e2s4/` (direct replay, 21 cases),
`e2s5/` (concurrent recovery, 22 cases), `e2s6/` (cancellation).

## 1. Why this document exists

The research corpus shows that "at-most-once" is not a property of the Claude Code
client. It is a property of some paths, under some error shapes, for some servers.
DaVinci must not document or claim a client-wide at-most-once guarantee either.
What it can guarantee is narrower and testable, and this document states it.

## 2. The six events that get conflated

Restated from `00-RESEARCH-IMPORT-AND-EVIDENCE.md` §4 with DaVinci's answer at
`6912f940` plus the fix in this audit.

| Event | DaVinci answer | Mechanism | Test |
|---|---|---|---|
| **Direct replay** — the same call is sent again by the client | Impossible | No retry loop exists in `Client::call_tool` or in `McpRegistry::call` | `crates/davinci-mcp/src/lib.rs` `Client::call_tool`; `a_lost_tool_reply_is_ambiguous_and_a_refused_call_is_not` |
| **Collateral replay** — a peer is re-sent because a shared connection was torn down | Impossible, but the peer still fails | One `Client` per server behind a `Mutex`; an error evicts that server and all of its routes (`McpRegistry::Inner::failed`) | `crates/davinci-agent/src/mcp.rs`; `a_lost_mcp_reply_becomes_may_have_run_not_failed` |
| **Transport reconnection** — a new connection is established, no call resent | Not attempted | A failed server stays evicted for the session; `resolve_tool` then returns `None` | `crates/davinci-agent/src/mcp.rs` `drop_server`/`resolve_tool` |
| **Model-initiated second call** | Allowed and distinguishable | New `tool_use` id; ledger records a new attempt | `tool_ledger.rs` `reserve_call_with_policy` |
| **Idempotent execution** | Never assumed by the client | `idempotentHint` is not consulted for replay; there is no replay to authorise | `crates/davinci-mcp/src/lib.rs` tool annotations |
| **Unknown outcome** | First-class after this audit | `Error::Ambiguous` → `ToolError::MayHaveRun` → model-visible `Outcome unknown` + `details.outcome_unknown` → ledger `AttemptOutcome::StartedUnknown` | `RUN` tests listed in §6 |

Conclusion for DaVinci, stated without borrowing Claude Code's wording:

1. A `tools/call` may execute **zero or one** time, never twice by the client.
2. If the client cannot prove the request never left, the outcome is **unknown**,
   not failed.
3. Only a JSON-RPC error reply, or a write refused before queueing, is a **known**
   outcome. Everything else is unknown.

## 3. DaVinci transport model at the audited revision

```
Caller (turn.rs tool dispatch)
  └── McpRegistry::call_exposed            crates/davinci-agent/src/mcp.rs
        └── McpRegistry::call_full
              └── Client::call_tool        crates/davinci-mcp/src/lib.rs
                    └── Rpc::call          (jsonrpc id allocation, correlation)
                          └── RpcTransport::{Stdio,Http,External}::call
                                stdio:  StdinWriter queue + ack deadline
                                http:   ureq POST with call deadline
```

Both transports are **synchronous and single-flight per client**. There is no
background replay queue, no session cache, no re-dial, and no `_meta` invocation id.

### 3.1 Error classes before this audit

`Error::{Transport, Protocol, Rpc}`. `Protocol` covered both handshake defects and
undecodable replies; `Transport` covered timeouts, closed pipes, spawn failure and
HTTP status errors. Both were reported to the model as a plain failure string, and
the compatibility ledger stored `AttemptOutcome::Failed`.

That is exactly the class-E defect in F-20-03: a stdio crash **after** the server
committed the effect and a crash **before** it are indistinguishable to the caller.

### 3.2 Error classes after this audit

| Class | Produced by | Meaning | Model-visible |
|---|---|---|---|
| `Rpc { code, message }` | server replied with a JSON-RPC error | the server handled the call and reports failure | plain error text |
| `NotDelivered(String)` | line never queued (`WriteError::Backlog`), or the line is incomplete (`write_all` failure, stopped writer, encode failure) | the request never reached the server as a message | plain error text |
| `Ambiguous(String)` | timeout, closed stdout, server exit, HTTP status, undecodable `tools/call` result | delivery cannot be ruled out | `Outcome unknown …` + `details.outcome_unknown=true` |
| `Transport` / `Protocol` | other methods (`tools/list`, `read_resource`, handshake) | no call was attributed, so no unknown-outcome statement is made | plain error text |

The asymmetry is deliberate: only `Client::call_tool` classifies, because only there
is the question "did this call run?" meaningful. `tools/list` failing is a plain
transport problem.

The stdio writer distinguishes *why* a line did not complete, because the answer
changes the class:

| Writer result | Meaning | Class |
|---|---|---|
| line never queued (queue full) | zero bytes offered to the pipe | `NotDelivered` |
| `write_all` failed, writer stopped, or encode failed | the line is incomplete; a line-framed server cannot dispatch it | `NotDelivered` |
| whole line written, `flush` failed | the server may read the complete line | `Ambiguous` |
| writer stopped after accepting the job | it may already have written it | `Ambiguous` |

### 3.3 Classification truth table (implemented)

| Observation | Class | May have run |
|---|---|---|
| Server returns a JSON-RPC `error` | `Rpc` | no |
| `try_send` finds the stdin queue full (line never queued) | `NotDelivered` | no |
| `write_all`/`flush` fails because the pipe is gone (line never entered) | `NotDelivered` | no |
| Write accepted, reply times out (`--hang`) | `Ambiguous` | yes |
| Write accepted, server exits (`--die`) | `Ambiguous` | yes |
| Reply arrives but does not decode | `Ambiguous` | yes |
| Whole line written, `flush` fails | `Ambiguous` | yes |
| Writer stopped after accepting the job | `Ambiguous` | yes |
| HTTP non-2xx, timeout, truncated body | `Ambiguous` | yes |
| Spawn failure / handshake failure | `Transport` (no call) | no call was made |

The fail-safe direction is "if it is not provably undelivered and not an RPC reply,
it is ambiguous". This matches the research's own lesson from F-20-04/F-20-05:
**never derive "did not run" from an error shape that does not prove it.**

## 4. From transport class to durable state

Three consumers had to change together, or the fix would be cosmetic:

| Layer | File | Behaviour |
|---|---|---|
| Registry | `crates/davinci-agent/src/mcp.rs` `Inner::failed` | evicts the server, then returns `ToolError::MayHaveRun` instead of `ToolError::Failed` for an ambiguous error |
| Model-visible result | `crates/davinci-agent/src/tools.rs` `ToolError::into_tool_result` | appends the unknown-outcome sentence and sets `details.outcome_unknown=true`, `details.tool=<name>` |
| Turn + ledger | `turn.rs` catch-all; `ToolCallLedger::record_unknown_outcome` | non-journal path records `AttemptOutcome::StartedUnknown` with terminal status `Failed`, so later recovery reconciles instead of retrying |

The journal route already recorded `EffectStatus::Unknown` on any errored tool
(`runtime/operations/adapters/tools.rs`), and journal retry already required
`EffectStatus::NotStarted`. The gap was the **non-journal** ledger and the text the
model read.

### 4.1 Why `StartedUnknown` with status `Failed`

The ledger needs two independent facts:

- `status`: is anything still in flight? Here, no — the call ended.
- `outcome`: did the effect happen? Here, unknown.

Recording `status=Failed, outcome=Failed` (the old behaviour) made recovery treat the
call as a confirmed failure and, for a replay-safe tool, execute it again.
Recording `status=Executing` would leave a phantom in-flight record until restart.
The chosen pair is terminal-but-unconfirmed, which is exactly what `recover_attempt`
needs: `AttemptOutcome::StartedUnknown` maps to `RecoveryAction::Stop(...)` /
`ReplayBlocked`, never `Retry` and never `Replay`.

## 5. What the fix does **not** do

| Not done | Why | Consequence |
|---|---|---|
| No automatic retry, even for `NotDelivered` | DaVinci has no retry path to reuse; adding one is a separate, larger change with its own acceptance criteria | a refused write costs one model round-trip to re-issue |
| No reconnect | Fail-closed eviction is the existing, tested policy | a dead server needs a session restart |
| No server-side dedupe (`_meta` invocation id) | Not present in DaVinci; would require wire changes in all three transports | a cooperating server cannot dedupe; the client's no-replay rule is the only protection |
| No `read_resource` classification | A resource read has no mutation semantics DaVinci can assert | resource reads stay plain transport errors |
| No change to HTTP status handling | HTTP already routes through `Error::Transport` and is classified ambiguous at the call boundary | correct after the fix, by construction |

## 6. Regression evidence

`RUN` in this session — `cargo test -p davinci-mcp`, `cargo test -p davinci-agent --lib`,
and the eight integration suites in `11` §2 (the codemode fixture is compiled into them,
not into `--lib`):

| Test | File | Asserts |
|---|---|---|
| `a_lost_tool_reply_is_ambiguous_and_a_refused_call_is_not` | `davinci-mcp/src/lib.rs` | hang → ambiguous; die → ambiguous; RPC error → known, not ambiguous |
| `a_line_refused_before_queueing_is_provably_not_delivered` | `davinci-mcp/src/stdio.rs` | a full stdin queue yields `NotDelivered`, not ambiguous |
| `a_half_written_line_cannot_have_run` | `davinci-mcp/src/stdio.rs` | an incomplete line is `NotDelivered` |
| `a_line_written_but_not_flushed_is_an_unknown_outcome` | `davinci-mcp/src/stdio.rs` | a complete line whose flush failed is ambiguous |
| `malformed_correlated_reply_is_an_ambiguous_outcome` | `davinci-mcp/src/lib.rs` | undecodable reply → ambiguous and still evicts the server |
| `a_lost_mcp_reply_becomes_may_have_run_not_failed` | `davinci-agent/src/mcp.rs` | registry maps to `MayHaveRun`; the route is still evicted, so a later call is `Unknown` |
| `an_error_that_may_have_reached_the_tool_is_reported_as_unknown` | `davinci-agent/src/tools.rs` | model text + `outcome_unknown` details; a definite failure is unchanged |
| `an_unknown_outcome_is_not_recorded_as_a_confirmed_failure` | `davinci-agent/src/tool_ledger.rs` | ledger outcome; no completed-result replay; second delivery is reconciled |
| `delayed_response_loss_is_not_no_effect_or_an_automatic_retry` | `davinci-agent/tests/common/codemode.rs` | codemode fixture: effect ran once, reply lost → `may_have_run` |
| `wor51_*` (3 tests) | `davinci-mcp/tests/stdio_write_timeout.rs` | an unread stdin still times out without blocking call, notify or teardown |

## 7. Residual risks and open questions

1. **Cancellation vs teardown.** `call_timeout` sends `notifications/cancelled` best
   effort after the deadline. The research did not establish a server-side guarantee
   for that path (F-42-07), and DaVinci has no test for a server that commits after
   receiving the cancellation. The unknown-outcome report remains correct either way,
   because it never claims the effect is absent.
2. **Definition of "incomplete".** `NotDelivered` relies on MCP stdio being
   line-framed: a server dispatches only a complete line. A non-conforming server
   that acts on partial input would break the argument, and DaVinci cannot detect
   that from the client side.
3. **HTTP retries by `ureq`/the agent.** No retry is configured, but an
   intermediary proxy can replay a POST. Nothing in DaVinci can detect that; it is a
   deployment property, not a client guarantee.
4. **No `_meta` invocation id (M7).** If DaVinci ever adds a retry or a reconnect, a
   stable invocation id must be added **first**, because the server currently has no
   way to recognise a repeat.
5. **Flush on Windows pipes.** The `flush`-failed path is covered by a unit test with
   a failing `Write`, not by a real broken pipe. On Windows and Unix a `ChildStdin`
   flush is a no-op, so the real-world path is expected to be the `write_all`
   failure; the unit test pins the classification either way.

## 8. Acceptance criteria for any future retry/reconnect work

Any proposal to add a retry must satisfy all of the following before it is merged:

1. It can prove non-delivery at the transport, not infer it from an error string.
2. It reuses the same ledger attempt (no second `tool_use` id) and records the
   attempt count.
3. It never retries a `StartedUnknown` outcome.
4. It sends a stable invocation id (`_meta` or equivalent) so a cooperating server can
   dedupe.
5. It has a fixture proving the effect count is 1 for a lost reply and 0 for a
   refused write.
6. It does not change the model-visible wording from "unknown" to "failed" for any
   path where delivery cannot be excluded.
