# OpenAI speed and token efficiency results

Measured on Windows on 2026-09-25 with `gpt-6-luna`, medium base effort.
All three final campaigns used the same release binary and ran sequentially.
Each contains eight tasks with three repetitions. These are real provider
runs, not simulated results. Historical comparisons are not contemporaneous
controls; server load and model sampling can affect the measurements.

## Historical baseline supplied with the plan

| Metric | davinci | Codex CLI 0.156.1 |
|---|---:|---:|
| Hidden-test passes | 20/24 | 17/24 |
| Median wall time | 70 s | 26 s |
| Uncached input tokens | 766,641 | 354,610 |
| Cache ratio | 77% | 88% |
| Output tokens | 38,820 | 19,295 |
| Median tool calls | 14 | 4 |
| Transaction-directory leak runs | 24/24 | 0/24 |

Tool-call and transaction-leak baseline values come from the plan's table;
the supplied historical raw rows did not record those fields. No fresh
Codex CLI benchmark was run.

## Measured checkpoints

| Campaign | Passes | Median seconds | Uncached input | Cache | Output | Median tools | Leak runs |
|---|---:|---:|---:|---:|---:|---:|---:|
| checkpoint-b | 8/8 | 54.7 | 333,473 | 61.4% | 15,144 | 10 | 7 |
| checkpoint-c | 23/24 | 44.8 | 242,460 | 91.2% | 37,193 | 10 | 23 |
| g-default-final | 22/24 | 35.6 | 175,542 | 91.6% | 25,212 | 8 | 0 |
| g-adaptive | 23/24 | 35.7 | 241,645 | 89.4% | 25,194 | 10.5 | 0 |
| g-lean | 21/24 | 30.8 | 196,126 | 74.2% | 24,521 | 9 | 0 |

Checkpoint B is one repetition only. Checkpoint C precedes harness verification
and transaction-ignore changes. Neither is a final acceptance result.
`g-default-final` uses fixed effort and the full tool surface. `g-adaptive`
changes only effort policy. `g-lean` changes only tool surface.

## Final gate output

### g-default-final

```text
PASS  passes             22 >= 20
PASS  median wall        35.6s <= 45.0s
PASS  uncached input     175,542 <= 500,000
PASS  output tokens      25,212 <= 30,000
PASS  median tool calls  8.0 <= 9.0
PASS  transaction leaks  0 <= 0
```

### g-adaptive

```text
PASS  passes             23 >= 20
PASS  median wall        35.7s <= 45.0s
PASS  uncached input     241,645 <= 500,000
PASS  output tokens      25,194 <= 30,000
FAIL  median tool calls  10.5 <= 9.0
PASS  transaction leaks  0 <= 0
```

### g-lean

```text
PASS  passes             21 >= 20
PASS  median wall        30.8s <= 45.0s
PASS  uncached input     196,126 <= 500,000
PASS  output tokens      24,521 <= 30,000
PASS  median tool calls  9.0 <= 9.0
PASS  transaction leaks  0 <= 0
```

`g-adaptive` does not meet the plan's condition for beating default on both wall time and uncached input with at least the same number of passes.

`g-lean` does not meet the plan's condition for beating default on both wall time and uncached input with at least the same number of passes.

Defaults remain fixed effort and full tool surface. A default change is
the owner's decision. No service-tier ablation was requested or performed.

The failed hidden-test runs are retained below. For example, the lean duration
failure in repetition 1 rejects a valid duration with surrounding whitespace
(`  3m  `). The model-generated parser skipped the final whitespace and then
incorrectly required another token. This is a generated-solution failure,
not a change made to the benchmark or its hidden tests.

- `g-default-final`: t4-rename r0, t5-csv r0.
- `g-adaptive`: t2-duration r1.
- `g-lean`: t2-duration r0, t2-duration r1, t2-duration r2.


## Evidence and implementation

Release source: `43e0058`.

SHA256: `2581029412ba2e85fb8c97ab0deadaf2a6c9acf0adb054359e8162e0597190c5`.

Local raw results, JSON transcripts, reports, gate outputs and build identities
are under `scripts/bench/runs/<campaign>/`. These ignored artifacts are not
committed. Private request dumps are not published.

[Cache diagnosis](openai-cache-diagnosis.md) records the missing no-session
WebSocket continuation, the scoped transport fix, and measured delta reuse.
[Remote compaction research](../superpowers/specs/2026-09-24-openai-remote-compaction.md)
is research only. No remote compaction code was added.

An initial `g-default` campaign was interrupted after 12 completed rows
(11 passes). It exposed valid multi-edits rejected when a provider filled
unused legacy fields with empty strings. Parser and real execution tests
failed before the narrow fix and passed afterward. That partial campaign is
preserved as diagnostic evidence and excluded from final comparisons.

The branch also contains repairs needed to satisfy the existing build and
workspace gates: library-split imports, stale graph/TUI fixtures, graph
checkpoint state updates, a session persistence error latch, an OAuth test
client lifetime, isolated JS test resources, narrow permission-warning
rendering, and a concurrent SQLite WAL initialization retry. The WAL retry
handles only SQLITE_BUSY, at most four attempts, and preserves other errors.
SQLite can bypass its busy handler during lock upgrades; see the
[SQLite documentation](https://www.sqlite.org/c3ref/busy_handler.html).
Thirty consecutive concurrent-open repetitions passed after the fix.

## Post-review fix: false verification failures

A review of the final campaign streams found that the completion gate fired
in 24 of 24 runs and that the harness rerun reported "failed" in 9 of them
although pytest passed. Two causes:

- Verification coverage only understood Cargo commands. A passing `pytest`,
  `go test` or `npm test` was classified `Unknown`, so the change stayed
  unverified.
- The rerun was judged failed whenever the evidence was not `Verified`,
  rather than by the rerun's own exit status.

Non-Cargo verifiers are now scoped by their path arguments (none means the
project's suite), and a rerun is failed only when it exits with an error.
A passing rerun that does not cover the change falls back to the plain
reminder. The harness tool call is also excluded from the reply text so a
passing rerun cannot blank the final answer. The campaigns above predate this
fix; they have not been re-run.

## Settings

| Setting | Environment | Behavior |
|---|---|---|
| `autoVerify` | `DAVINCI_AUTO_VERIFY` | Defaults to true; replays the prior verification through normal tool permissions and execution. `0`, `false`, or `off` disables it. |
| `serviceTier` | `DAVINCI_OPENAI_SERVICE_TIER` | Optional Codex route service tier; fast/priority maps to priority, flex maps to flex, unknown values are omitted. |
| `effortPolicy` | `DAVINCI_EFFORT_POLICY` | Fixed by default; adaptive changes per-request effort without changing configured identity. |
| `toolSurface` | `DAVINCI_TOOL_SURFACE` | Full by default; lean initially exposes core schemas and retains authorized discovery. Applied at startup. |
| Request diagnostics | `DAVINCI_WIRE_DUMP` | Opt-in local request, usage, and WebSocket-frame captures. |

## Validation and boundaries

- Full workspace: 4,474 passed, 33 ignored across 123 suites.
- Strict workspace Clippy with all targets: passed.
- Formatting: all 61 changed Rust files passed; untouched files retain
  pre-existing formatting drift, using the plan's Checkpoint C exception.
- Release build and final diff whitespace check: passed.
- All eight fixture pairs failed before their reference solution and passed
  afterward. Supplied benchmark scripts remain byte-identical to the assets.
- Legacy prompt, compaction and branch-summary constants are unchanged.
  No dependency or lockfile changes.
- Formal prompt promotion checks were not run, as specified by this plan.
- No subagents, installation, merge, or deployment. Review was performed
  in the main session; it was not an independent agent review.
