# davinci vs Codex CLI benchmark

Eight small Python tasks (bug fixes, features, a multi-file rename, a CSV bug
hunt, a CLI flag, a failing test suite). Each run gets a fresh git repository;
hidden tests the agent never sees decide pass or fail.

    python scripts/bench/make_tasks.py         # writes scripts/bench/tasks/
    python scripts/bench/bench.py validate     # each task fails at start, passes with its reference fix
    python scripts/bench/bench.py run --harness davinci codex --tasks all --reps 3
    python scripts/bench/bench.py report       # writes runs/summary.json
    python scripts/bench/bench.py gate         # exit 1 when a davinci gate fails

Both harnesses use the same model and effort (`BENCH_MODEL`, default
`gpt-6-luna`; `BENCH_EFFORT`, default `medium`). Codex runs with
`--ignore-user-config` so a personal `service_tier = "fast"` does not skew wall
time. Approvals are bypassed on both sides inside the throwaway repositories.

This spends real usage on the selected accounts. It is a maintainer tool,
not an offline test. To measure only DaVinci, use `--harness davinci`.

On Windows, set PYTHONUTF8=1 before running these commands so pytest and
the runner agree on subprocess output encoding.

## Cache diagnosis

```powershell
$env:DAVINCI_WIRE_DUMP = "$PWD\scripts\bench\runs\wire-t1"
python scripts/bench/bench.py run --harness davinci --tasks t1-intervals --reps 1
python scripts/bench/cache_report.py scripts/bench/runs/wire-t1
Remove-Item Env:DAVINCI_WIRE_DUMP
```

The dump contains the full conversation, including file contents the agent
read. Keep it local.
