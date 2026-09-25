# OpenAI cache diagnosis

Measured 2026-09-25 on Windows, model `gpt-6-luna`, thinking `medium`.
Release binary includes Parts A/B and C1. Both tasks passed their hidden tests.
Wire dumps remain in ignored local run directories and are not published.

## Reproduction

Run from the task worktree after the release build:

```powershell
rtk cargo build -p davinci-coding-agent --release --offline --locked
$env:PYTHONUTF8 = "1"
$env:BENCH_DAVINCI = "$PWD\target\release\davinci.exe"
$env:BENCH_RUNS = "$PWD\scripts\bench\runs\diagnosis"
$env:DAVINCI_WIRE_DUMP = "$env:BENCH_RUNS\wire-t1"
rtk proxy python scripts/bench/bench.py run --harness davinci --tasks t1-intervals --reps 1
$env:DAVINCI_WIRE_DUMP = "$env:BENCH_RUNS\wire-t4"
rtk proxy python scripts/bench/bench.py run --harness davinci --tasks t4-rename --reps 1
Remove-Item Env:DAVINCI_WIRE_DUMP
rtk proxy python scripts/bench/cache_report.py scripts/bench/runs/diagnosis/wire-t1
rtk proxy python scripts/bench/cache_report.py scripts/bench/runs/diagnosis/wire-t4
```

The executed Windows driver used Python `os.environ` and `subprocess.run` to
set these same values and invoke these commands sequentially. This avoids
nested PowerShell environment expansion. Full report outputs are below;
neither contains conversation content.

### wire-t1

```text
process 33028
  #0001 items=  2 total=  9,902 cached=  9,728 full  key=ci-root-32db24c31195 break=-
  #0002 items=  4 total= 10,072 cached=  8,704 full  key=ci-root-32db24c31195 break=-
  #0003 items=  6 total= 10,112 cached=  9,728 full  key=ci-root-32db24c31195 break=-
  #0004 items=  8 total= 10,341 cached=  9,728 full  key=ci-root-32db24c31195 break=-
  #0005 items= 10 total= 10,652 cached=  8,704 full  key=ci-root-32db24c31195 break=-
  #0006 items= 12 total= 10,858 cached=  8,704 full  key=ci-root-32db24c31195 break=-
  #0007 items= 14 total= 11,064 cached=  9,728 full  key=ci-root-32db24c31195 break=-
  #0008 items= 16 total= 11,283 cached=  9,728 full  key=ci-root-32db24c31195 break=-
  #0009 items= 18 total= 11,502 cached=  9,728 full  key=ci-root-32db24c31195 break=-
  #0010 items= 20 total= 11,706 cached=  8,704 full  key=ci-root-32db24c31195 break=-
  #0011 items= 22 total= 11,932 cached=  8,704 full  key=ci-root-32db24c31195 break=-
  #0012 items= 24 total= 12,083 cached=  8,704 full  key=ci-root-32db24c31195 break=-
  #0013 items= 26 total= 12,157 cached= 10,752 full  key=ci-root-32db24c31195 break=-
  #0014 items= 28 total= 12,215 cached=  8,704 full  key=ci-root-32db24c31195 break=-
  total input 155,879, cached 130,048 (83%)
```

### wire-t4

```text
process 39592
  #0001 items=  2 total=  9,919 cached=  8,704 full  key=ci-root-32db24c31195 break=-
  #0002 items=  8 total= 10,509 cached=  8,704 full  key=ci-root-32db24c31195 break=-
  #0003 items= 18 total= 10,886 cached=  8,704 full  key=ci-root-32db24c31195 break=-
  #0004 items= 20 total= 10,978 cached=  8,704 full  key=ci-root-32db24c31195 break=-
  #0005 items= 22 total= 11,538 cached=  8,704 full  key=ci-root-32db24c31195 break=-
  #0006 items= 28 total= 12,114 cached=  8,704 full  key=ci-root-32db24c31195 break=-
  #0007 items= 30 total= 12,171 cached= 10,752 full  key=ci-root-32db24c31195 break=-
  #0008 items= 32 total= 12,302 cached=  9,728 full  key=ci-root-32db24c31195 break=-
  #0009 items= 34 total= 12,364 cached= 10,752 full  key=ci-root-32db24c31195 break=-
  total input 102,781, cached 83,456 (81%)
```

## Classification and selected fix

Both logical histories are append-only (`break=-` throughout), and every
non-input request field stays fixed in the inspected t1 sequence. The cache
key is constant, but every wire request is `full`. This extends the plan's
WebSocket-continuation row: continuation is absent throughout, rather than
alternating between full and delta. It does not match the server-only row,
which requires stable delta mode.

The benchmark passes `--no-session`. The CLI provider callback currently
derives `StreamOptions.session_id` only from a persisted session, producing
`None`. `codex::cache_session_id` then returns `None`;
`acquire_cached_continuation` cannot load a continuation and
`store_cached_continuation` stores nothing. The live socket likewise has no
session lease to reuse. A stable prompt cache key is only a routing affinity;
it does not supply the missing conversation-specific transport identity.

Selected C3 fix: give the CLI provider loop a unique in-memory transport
session when persistence is disabled, retain it across requests in the loop,
and close its sockets on scope exit. Keep existing persisted session IDs and
strict cache-disable behavior. Do not reuse the prompt cache key as a session
ID: unrelated conversations can deliberately share that key.

No evidence justifies changing ephemeral-context placement, tool ordering or
history pruning in these measured runs. The offline C1 regression also passed.

## Verification status

Before-fix t1: 7/7 tests, 60.6s, 12 tool calls, 14 requests, 83% cached.
Before-fix t4: 6/6 tests, 51.9s, 15 tool calls, 9 requests, 81% cached.
Checkpoint B: 8/8 tasks, median 55s and 10 tool calls, cache 61%, seven
transaction-directory leaks. These are measurements, not final acceptance.

After-fix validation is pending.
