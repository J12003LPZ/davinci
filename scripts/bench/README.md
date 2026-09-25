# davinci vs Codex CLI benchmark

Eight small Python tasks (bug fixes, features, a multi-file rename, a CSV bug
hunt, a CLI flag, a failing test suite). Each run gets a fresh git repository;
hidden tests the agent never sees decide pass or fail.

    python scripts/bench/make_tasks.py         # writes scripts/bench/tasks/
    python scripts/bench/bench.py validate     # each task fails at start, passes with its reference fix
    python scripts/bench/bench.py run --harness davinci codex --tasks all --reps 3
    python scripts/bench/bench.py report       # writes runs/summary.json
    python scripts/bench/bench.py gate         # exit 1 when a davinci gate fails

Both harnesses use the same model and effort (
On Windows, set PYTHONUTF8=1 before running these commands so pytest and
the runner agree on subprocess output encoding.
