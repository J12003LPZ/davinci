# 11 — Verification record

Status: executed record. Every command below was run in this session against
`C:\Users\sergi\Desktop\davinci` with the patch from `10-IMPLEMENTATION-RECORD.md`
applied to the working tree (nothing committed).

## 1. Environment

| Property | Value |
|---|---|
| OS | Windows, `x86_64-pc-windows-msvc` |
| `rustc` | 1.88.0 (6b00bc388 2025-06-23) |
| `cargo` | 1.88.0 (873a06493 2025-05-10) |
| Branch | `J12003LPZ/cc-tui-parity` |
| HEAD | `6912f940` |
| Build profile | `test` (debug), `--offline` for clippy |
| Network | none used; no provider, model or subscription call was made |

## 2. Commands and results

All exit codes are from `$LASTEXITCODE` in PowerShell after the pipeline. Durations are
the test binary's own reported time (single run, debug profile); they are not
benchmarks and are not compared to anything.

| # | Command | Exit | Result |
|---|---|---|---|
| 1 | `cargo test -p davinci-mcp` | 0 | 52 lib + 15 integration, 0 failed |
| 2 | `cargo test -p davinci-agent --lib` | 0 | 1863 passed, 0 failed, 46.99 s |
| 3 | `cargo test -p davinci-mcp --test audit_capabilities` | 0 | 3 passed |
| 4 | `cargo test -p davinci-mcp --test codemode_structured` | 0 | 9 passed |
| 5 | `cargo test -p davinci-mcp --test stdio_write_timeout` | 0 | 3 passed, 0.63 s |
| 6 | `cargo test -p davinci-agent --test codemode_contract` | 0 | 6 passed |
| 7 | `cargo test -p davinci-agent --test codemode_policy` | 0 | 5 passed |
| 8 | `cargo test -p davinci-agent --test codemode_projection` | 0 | 5 passed |
| 9 | `cargo test -p davinci-agent --test mcp_execution_policy` | 0 | 5 passed |
| 10 | `cargo test -p davinci-agent --test mcp_progressive_disclosure` | 0 | 7 passed |
| 11 | `cargo test -p davinci-agent --test agent_tool_resume_integration` | 0 | 3 passed |
| 12 | `cargo test -p davinci-agent --test tool_result_pairing_recovery` | 0 | 8 passed |
| 13 | `cargo test -p davinci-agent --test luna_reliability_followups` | 0 | 2 passed |
| 14 | `cargo fmt -p davinci-mcp -p davinci-agent --check` | 0 | no diff |
| 15 | `cargo clippy -p davinci-mcp -p davinci-agent --all-targets --offline` | 0 | no warning, no error |

Totals: 52 + 15 + 1863 + 41 targeted integration = **1971 test cases executed, 0 failed**.

`cargo test -p davinci-mcp` breakdown (from the run output):

| Binary | Tests |
|---|---|
| `davinci_mcp` (lib) | 52 (6.70 s) |
| `tests/audit_capabilities.rs` | 3 |
| `tests/codemode_structured.rs` | 9 |
| `tests/stdio_write_timeout.rs` | 3 (0.63 s) |

## 3. Fix-specific regressions (all green)

| Test | File:line (post-patch) | Asserts |
|---|---|---|
| `tests::a_lost_tool_reply_is_ambiguous_and_a_refused_call_is_not` | `davinci-mcp/src/lib.rs:783` | hang → `Ambiguous`; die → `Ambiguous`; RPC error → known, not ambiguous |
| `tests::malformed_correlated_reply_is_an_ambiguous_outcome` | `davinci-mcp/src/lib.rs:726` | undecodable `tools/call` reply → `Ambiguous`, `may_have_run`, still `drops_server` |
| `stdio::tests::a_line_refused_before_queueing_is_provably_not_delivered` | `davinci-mcp/src/stdio.rs:718` | full stdin queue → `NotDelivered` |
| `stdio::tests::a_half_written_line_cannot_have_run` | `davinci-mcp/src/stdio.rs:802` | `write_all` fails mid-line → `NotDelivered` |
| `stdio::tests::a_line_written_but_not_flushed_is_an_unknown_outcome` | `davinci-mcp/src/stdio.rs:810` | complete line, flush fails → `Ambiguous` |
| `tests::a_lost_mcp_reply_becomes_may_have_run_not_failed` | `davinci-agent/src/mcp.rs:1096` | registry → `MayHaveRun`; route evicted; later call is `Unknown` |
| `tests::an_error_that_may_have_reached_the_tool_is_reported_as_unknown` | `davinci-agent/src/tools.rs:4568` | model text says unknown + `details.outcome_unknown`; definite failure unchanged |
| `tests::an_unknown_outcome_is_not_recorded_as_a_confirmed_failure` | `davinci-agent/src/tool_ledger.rs:1268` | `status=Failed`, `outcome=StartedUnknown`; recovery never retries/replays |

Two expectations were deliberately changed (justified in `10` §4):

| Test | Change |
|---|---|
| `malformed_correlated_reply_is_a_protocol_error` → `…is_an_ambiguous_outcome` | expected `Protocol`; now expects `Ambiguous` + `drops_server` |
| `delayed_response_loss_is_not_no_effect_or_an_automatic_retry` (`davinci-agent/tests/common/codemode.rs`) | expected `Err(Error::Transport)`; now asserts `may_have_run` and effect count still 1 |

## 4. Tests removed, moved or not run

**Moved.** `wor51_a_refused_queued_write_is_provably_not_delivered` was first written as an
integration test that flooded a real `ping` child until its stdin queue refused a line.
It passed but was non-deterministic (59 timeout iterations before one refusal) and took
7 s. It was replaced by the deterministic gated-pipe unit test
`a_line_refused_before_queueing_is_provably_not_delivered`. The integration file
`crates/davinci-mcp/tests/stdio_write_timeout.rs` is now **byte-identical to HEAD**
(`git diff --quiet` → exit 0; `git hash-object` equals `git rev-parse HEAD:…`). It still
appears as `M` in `git status` because Git's stat cache was invalidated by a line-ending
normalisation warning; there is no content change.

**Not run** (and therefore not evidence):

| Not run | Why |
|---|---|
| `cargo test --workspace` | not executed in this session; only the two changed crates and the eight affected suites were run |
| `davinci-coding-agent`, `davinci-session`, `davinci-evals`, `davinci-server` suites | no changes in those crates; findings about them are `CODE`/`TEST` from `01`–`06` |
| the full `davinci-agent` integration set (≈90 binaries) | only the eight suites that consume MCP results or codemode were run |
| any live model, provider or subscription call | forbidden without explicit authorisation; none made |
| any evaluation campaign, benchmark or cache measurement | no valid corpus (`06` §5) and no authorisation |

## 5. Line-number drift

`01`–`09` cite `file:line` at the **audited revision**, before this patch. Files not
touched by the patch keep exact line numbers (`compaction.rs`, `worktree.rs`,
`store.rs`, `permission.rs`, `subagent.rs`, `executor.rs`, …). Files that grew:

| File | Change |
|---|---|
| `davinci-mcp/src/lib.rs` | +72 −5 |
| `davinci-mcp/src/stdio.rs` | +148 −14 |
| `davinci-agent/src/mcp.rs` | +59 −0 |
| `davinci-agent/src/tools.rs` | +55 −0 |
| `davinci-agent/src/tool_ledger.rs` | +61 −0 |
| `davinci-agent/src/turn.rs` | −4 inside 3192–3475, +6 after 3475 |
| `davinci-agent/tests/common/codemode.rs` | +5 −1 |

References in `01`–`09` that point *into* those files after an insertion point are
therefore lower than the current line by the accumulated delta. They were not renumbered
because re-deriving ~40 refs by hand would introduce new errors; everything the fix
touched has a post-patch anchor in §3 and in `10`.

`turn.rs:501-552` (`unresolved_call_result`) and all `turn.rs` refs before 3192 are
unaffected and were re-checked.

## 6. Working-tree state at the end of the session

```
branch  J12003LPZ/cc-tui-parity
HEAD    6912f940
modified by this audit:
  crates/davinci-mcp/src/lib.rs
  crates/davinci-mcp/src/stdio.rs
  crates/davinci-agent/src/mcp.rs
  crates/davinci-agent/src/tools.rs
  crates/davinci-agent/src/turn.rs
  crates/davinci-agent/src/tool_ledger.rs
  crates/davinci-agent/tests/common/codemode.rs
untracked by this audit:
  docs/audit/claude-code-parity/   (00–12)
pre-existing, left untouched:
  AGENTS.md
  crates/davinci-coding-agent/src/browser_integration_tests.rs
  CLAUDE.md (untracked)
```

No commit, push, merge or deploy was performed.

## 7. What these results do and do not establish

**Established (structurally):**

1. A `tools/call` whose reply is lost is classified `Ambiguous` and reaches the model as
   an unknown outcome with `outcome_unknown=true`.
2. A provably unqueued or half-written line is `NotDelivered`, not ambiguous.
3. A JSON-RPC error reply remains a known failure.
4. The compatibility ledger stores such a call as terminal-but-unconfirmed, and the
   recovery function does not retry or replay it.
5. The change compiles clean under `clippy --all-targets` and is `rustfmt`-clean.

**Not established by these results:**

1. That a user retries less, duplicates fewer mutations, spends less quota or completes
   more tasks. None of that was measured (`08`).
2. That a *real* non-DaVinci MCP server behaves like the fixtures. The fixtures implement
   the MCP surface DaVinci uses; they are not third-party servers.
3. That cancellation-after-commit is handled (`02` §7.1): untested.
4. That the `Incomplete` → `NotDelivered` argument holds for a non-conforming server
   (`02` §7.2): documented assumption, not a test.
5. Anything about the other four subsystems: their rows in `01` remain `CODE`/`TEST`
   and were not re-executed here.

## 8. Reproducing this record

```powershell
cd C:\Users\sergi\Desktop\davinci
cargo test -p davinci-mcp
cargo test -p davinci-agent --lib
foreach ($t in @("codemode_contract","codemode_policy","codemode_projection",
                 "mcp_execution_policy","mcp_progressive_disclosure",
                 "agent_tool_resume_integration","tool_result_pairing_recovery",
                 "luna_reliability_followups")) {
    cargo test -p davinci-agent --test $t
}
cargo fmt -p davinci-mcp -p davinci-agent --check
cargo clippy -p davinci-mcp -p davinci-agent --all-targets --offline
```

Expected: exit 0 for every command; 52 + 15 + 1863 + 41 passing cases; no clippy
warning; no format diff.

Two tests are timing-sensitive by design and were given wide margins:
`wor51_*` (500 ms deadline, 5 s assertion ceiling) and
`a_line_refused_before_queueing_is_provably_not_delivered` (50 ms deadline, 200 ms
settle). They passed on every run in this session; they have not been run on a loaded CI
machine.
