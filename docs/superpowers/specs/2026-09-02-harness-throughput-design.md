# Harness throughput: fewer model turns, less context, real concurrency

**Date:** 2026-09-02
**Scope:** `pi-agent` (loop, tools, subagents), `pi-coding-agent` (prompt, stats, evidence dir)
**Origin:** `CODEX_VS_PI_SUBAGENTS_ARCHITECTURE_REPORT.md` — the same GPT-5.6 Sol task
finished in Codex in 6 model turns / 5 batched tool calls / ~101k peak context, and aborted in
pi after 32 turns / 93 tool calls / ~226k context.

## Diagnosis the design answers

1. `ToolExecutionMode::Parallel` was a second `for` loop; the default was `Sequential`. The
   provider was told `parallel_tool_calls: true` and the runtime then serialized every call.
2. Every tool-bearing response cost another inference, and the prompt never told the model to
   batch, so it did one primitive call per turn.
3. Every primitive result stayed in the provider-visible history until compaction, which for a
   272k-window model does not fire before ~255k tokens.
4. Nothing counted turns, batch width or wall time, so none of this was measurable from inside.

## What changed

### 1. A lane scheduler (`pi-agent/src/scheduler.rs`)

Mirrors TS `executeToolCallsParallel` (real concurrency, results finalized in source order)
with one refinement. Each call is placed in a lane by the class the permission policy gives it:

| lane       | tools                                                                                     |
|------------|-------------------------------------------------------------------------------------------|
| `Parallel` | `read grep find ls web_fetch web_search mcp_read job_output`, MCP tools with `readOnlyHint`, `agent` |
| `Serial`   | `write edit notebook_edit bash powershell todo job_kill batch`, other MCP tools, extension tools |

Consecutive `Parallel` calls form a group that runs on up to `MAX_TOOL_PARALLELISM = 8`
scoped threads; a `Serial` call is a barrier that runs alone after the group before it and
before the group after it. So `edit A` then `read A` still sees the edit, while eight reads
cost one latency. `ToolExecutionMode::Sequential` forces width one. The abort flag is checked
before every group; the remaining calls are skipped exactly as an interrupted sequential run
skipped them.

The loop (`turn.rs`) is now three stages, as in TS: **prepare** (extension hook, unknown-tool
check, permission gate — on the loop thread, in order, so the approver is asked one question
at a time), **run** (`&self` only, so it may run on a worker), **finalize** (post hook, events
emitted live as each call ends and recorded in source order). `Agent::new` defaults to
`Parallel`, which is what TS `runtimeOptions.toolExecution ?? "parallel"` does.

### 2. The `batch` tool (`pi-agent/src/batch.rs`)

`batch { operations: [{ tool, args }, …] }`, up to 16 operations. Each operation passes the
same prepare stage as a direct call (so a `write` inside a batch is refused in `read-only`
mode and asked about in `ask` mode), runs in the same lanes, and reports to the post hook as
`<batch id>#<n>`. The model gets one result:

```
batch: 5/5 operations ran, 1 concurrent group, some failed

[1] read path="a.txt" → ok (12 chars)
…
[4] batch operations=… → error (52 chars)
`batch` cannot run inside a batch; call it directly.
```

Visible output is capped at 12 KB per operation and 64 KB per batch; overflow is written to
the evidence store and the result names the file. `batch` and `agent` cannot nest. The batch
itself is `Read` class and `Serial` lane: it runs on the loop thread so its operations' asks go
through the one approver.

### 3. The evidence store (`pi-agent/src/evidence.rs`)

`~/.pi/agent/evidence/<tag>-<id>.txt` (under `PI_CODING_AGENT_DIR` in tests). Full output the
model was not shown, readable back with `read offset/limit`. Swept at startup: files older than
seven days are removed. Reduce what the model *sees*, never what is *debuggable*.

### 4. Context pruning (`pi-agent/src/pruning.rs`)

When the estimated context passes 50% of the window, the oldest tool results larger than
1,500 chars — never the newest eight — are replaced in the provider view by

```
[output of grep pruned to save context (4000 chars). Re-run the tool if you need it again.]
```

until the estimate is under 35% or nothing prunable is left. The session JSONL keeps every
byte; only `messages_for_provider` and `estimated_context_tokens` see the placeholder. A pruned
result stays pruned, so the prompt prefix changes once per prune pass, not every turn, and the
provider's cache survives the turns between. Compaction is unchanged (its threshold mirrors TS)
and now rarely fires. `Agent::prune_settings` tunes it; `enabled: false` turns it off.

### 5. Subagent fan-out (`pi-agent/src/subagent.rs`)

`agent { tasks: [{ prompt, description?, tools? }, …] }` runs up to 8 workers, 4 at a time
(the reference extension's `MAX_PARALLEL_TASKS` / `MAX_CONCURRENCY`), and reports each under
`## n — description` in task order; one failed worker does not hide the others. Several `agent`
calls in one response also overlap, because `agent` is in the parallel lane. The worker's prompt
asks for findings with paths, not a narrative, and carries the tool-use strategy below.

### 6. Prompt policy (`pi_agent::TOOL_USE_STRATEGY`)

Appended to `default_system_prompt` and the worker prompt: minimize round trips, issue known
reads together or in a batch, read ranges not files, grep before read, delegate research to
workers and do not wait on them by reflex, keep output small. The runtime and the prompt have
to agree; this is the prompt's half.

### 7. Run counters (`pi-agent/src/stats.rs`)

`RunStats`: model turns, tool calls / batches / max and mean width / parallel groups, batch
operations, workers, model and tool wall time, peak context, pruned results and chars,
compactions, evidence files. `Agent::run_stats()` folds in the counters bumped from inside
tool calls. Exposed as `runtime` in `get_session_stats` (RPC) and as a `Runtime (this run)`
block in `/status`.

### 8. Tool algorithms (`pi-agent/src/tools.rs`)

The native grep (the path taken when ripgrep is not installed) compiled its regex **per line**;
it now builds one `Matcher` per call, skips binary files (NUL in the first 8 KB, as ripgrep
does), and scans files on up to 8 scoped threads once there are 32 or more of them, merging
results in walk order so output is identical to the sequential scan. The walker sorts entries
by name for stable output, uses the entry's own file type instead of a `stat` per path, and no
longer follows symlinked directories. `.gitignore` rules are split into name rules (checked
against the entry name only — the walk already pruned ignored ancestors) and path rules.
`ls` also stops calling `is_dir` per entry, and `read` no longer clones its output.

## Tests

- `scheduler.rs`: overlap (three sleeps of 120/20/60 ms finish under 200 ms), sequential mode,
  barrier ordering, width cap, abort.
- `lib.rs` loop tests: three workers in one message overlap and answer in order, with starts
  before ends; sequential mode does not overlap; `read → edit → read` sees the edit; a batch of
  five operations is one result with per-operation status; a batch `write` is gated like a
  direct call; overflow lands in the evidence store; old output is pruned from the provider
  view and kept in history.
- `subagent.rs`: task fan-out order and partial failure; all-failed is an error.
- `pruning.rs`, `stats.rs`, `evidence.rs`, `batch.rs`: unit tests.

## Not done (and why)

- **Compaction threshold** stays TS-parity. Pruning removes the pressure that made it matter;
  revisit with measurements.
- **Provider-reported usage for the context estimate**: the loop still uses the char estimate.
  `RunStats::peak_context_tokens` is that estimate.
- **Per-file serialization of edits**: edits are a global barrier. The per-file mutation queue
  already exists (`file_mutation_queue.rs`); relaxing the lane to per-path is a follow-up.
- **Benchmark harness**: the counters exist; the A/B run against Codex (same model, tier,
  repo state) is manual.
