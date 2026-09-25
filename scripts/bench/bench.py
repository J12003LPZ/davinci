"""Matched davinci vs Codex CLI benchmark.

    python scripts/bench/make_tasks.py
    python scripts/bench/bench.py validate
    python scripts/bench/bench.py run --harness davinci codex --tasks all --reps 3
    python scripts/bench/bench.py report
    python scripts/bench/bench.py gate

Environment:
    BENCH_MODEL      model for both harnesses (default gpt-6-luna)
    BENCH_EFFORT     reasoning effort for both harnesses (default medium)
    BENCH_TIMEOUT    seconds per run (default 900)
    BENCH_RUNS       results directory (default scripts/bench/runs)
    BENCH_DAVINCI    davinci executable (default: davinci on PATH)
    BENCH_CODEX      codex executable (default: codex on PATH)

This is a maintainer tool, never a test: it starts real harnesses and spends
model usage on both the davinci and the Codex CLI accounts.
"""
import argparse
import json
import os
import re
import shutil
import statistics
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
TASKS = os.path.join(HERE, "tasks")
RUNS = os.environ.get("BENCH_RUNS", os.path.join(HERE, "runs"))
MODEL = os.environ.get("BENCH_MODEL", "gpt-6-luna")
EFFORT = os.environ.get("BENCH_EFFORT", "medium")
TIMEOUT = int(os.environ.get("BENCH_TIMEOUT", "900"))
IGNORED = ("__pycache__", ".pytest_cache", ".git/", ".pi/", ".davinci/", ".codex/",
           ".davinci-transactions/", ".serena/")

# Acceptance gates for davinci, from
# docs/superpowers/plans/2026-09-24-openai-speed-and-token-efficiency.md.
GATES = {
    "min_passes": 20,            # out of 24 runs (8 tasks x 3 repetitions)
    "max_median_wall_s": 45.0,
    "max_uncached_input": 500_000,
    "max_output_tokens": 30_000,
    "max_median_tool_calls": 9.0,
    "max_transaction_leak_runs": 0,
}


def task_ids():
    return sorted(d for d in os.listdir(TASKS) if os.path.isdir(os.path.join(TASKS, d)))


def load(tid):
    with open(os.path.join(TASKS, tid, "task.json"), encoding="utf-8") as f:
        return json.load(f)


def copy_tree(src, dst):
    for base, _, files in os.walk(src):
        for name in files:
            rel = os.path.relpath(os.path.join(base, name), src)
            out = os.path.join(dst, rel)
            os.makedirs(os.path.dirname(out), exist_ok=True)
            shutil.copyfile(os.path.join(base, name), out)


def git(cwd, *args):
    return subprocess.run(["git", *args], cwd=cwd, capture_output=True, text=True, encoding="utf-8")


def _force_remove(func, path, _exc):
    os.chmod(path, 0o700)
    func(path)


def prepare(tid, dest):
    if os.path.exists(dest):
        shutil.rmtree(dest, onexc=_force_remove)
    os.makedirs(dest)
    copy_tree(os.path.join(TASKS, tid, "repo"), dest)
    git(dest, "init", "-q")
    git(dest, "config", "user.email", "bench@example.invalid")
    git(dest, "config", "user.name", "bench")
    git(dest, "config", "core.autocrlf", "false")
    git(dest, "add", "-A")
    git(dest, "commit", "-q", "-m", "fixture", "--no-verify")


def grade(tid, workdir):
    hidden = os.path.join(TASKS, tid, "hidden")
    copy_tree(hidden, workdir)
    files = []
    for base, _, names in os.walk(hidden):
        files += [os.path.relpath(os.path.join(base, n), hidden) for n in names]
    proc = subprocess.run(
        [sys.executable, "-m", "pytest", "-q", "-p", "no:cacheprovider", *files],
        cwd=workdir, capture_output=True, text=True, encoding="utf-8", timeout=120,
    )
    tail = proc.stdout.strip().splitlines()[-1] if proc.stdout.strip() else ""
    passed = int(m.group(1)) if (m := re.search(r"(\d+) passed", tail)) else 0
    failed = sum(int(x) for x in re.findall(r"(\d+) (?:failed|error)", tail))
    return proc.returncode == 0, passed, failed, tail


def changed_files(workdir, ignore=True):
    out = git(workdir, "status", "--porcelain", "-uall").stdout
    paths = []
    for line in out.splitlines():
        path = line[3:].strip().strip('"').replace("\\", "/")
        if " -> " in path:
            path = path.split(" -> ")[1]
        if ignore and any(part in path + "/" for part in IGNORED):
            continue
        paths.append(path)
    return sorted(paths)


def command(harness, prompt, workdir):
    if harness == "davinci":
        return [
            os.environ.get("BENCH_DAVINCI", "davinci"), "--mode", "json", "--no-session",
            "--provider", "openai-codex", "--model", MODEL, "--thinking", EFFORT,
            "--permission-mode", "always-approve", "-p", prompt,
        ]
    # --ignore-user-config drops a user's service_tier = "fast" (priority
    # processing davinci does not use), a max effort and MCP servers; auth is kept.
    return [
        os.environ.get("BENCH_CODEX", "codex"), "exec", "--json", "--ephemeral",
        "--ignore-user-config", "--skip-git-repo-check",
        "--dangerously-bypass-approvals-and-sandbox",
        "-m", MODEL, "-c", f'model_reasoning_effort="{EFFORT}"', "-C", workdir, prompt,
    ]


def parse_stream(harness, stdout):
    """Usage and activity from the JSON event stream.

    Returns dict(input, cached, output, tool_calls, requests, tools).
    `input` is total input tokens (cached plus uncached).
    """
    stats = {"input": 0, "cached": 0, "output": 0, "tool_calls": 0, "requests": 0, "tools": {}}
    for line in stdout.splitlines():
        line = line.strip()
        if not line.startswith("{"):
            continue
        try:
            event = json.loads(line)
        except ValueError:
            continue
        if harness == "codex":
            if event.get("type") == "turn.completed" and isinstance(event.get("usage"), dict):
                u = event["usage"]
                stats["input"] += u.get("input_tokens", 0)
                stats["cached"] += u.get("cached_input_tokens", 0)
                stats["output"] += u.get("output_tokens", 0)
                stats["requests"] += 1
            item = event.get("item") or {}
            if event.get("type") == "item.completed" and item.get("type") not in (
                None, "agent_message", "reasoning", "error"
            ):
                stats["tool_calls"] += 1
                stats["tools"][item["type"]] = stats["tools"].get(item["type"], 0) + 1
        else:
            update = event.get("assistantMessageEvent") or {}
            if event.get("type") == "message_update" and update.get("type") == "done":
                u = event.get("usage") or {}
                stats["input"] += u.get("input", 0) + u.get("cacheRead", 0)
                stats["cached"] += u.get("cacheRead", 0)
                stats["output"] += u.get("output", 0)
                stats["requests"] += 1
            if event.get("type") == "tool_execution_start":
                name = event.get("toolName") or "?"
                stats["tool_calls"] += 1
                stats["tools"][name] = stats["tools"].get(name, 0) + 1
    return stats


def run_one(harness, tid, rep):
    spec = load(tid)
    workdir = os.path.join(RUNS, harness, f"{tid}-r{rep}")
    prepare(tid, workdir)
    env = dict(os.environ)
    env["PI_LEARNING_DISABLE_BACKGROUND"] = "1"
    start = time.time()
    try:
        proc = subprocess.run(
            command(harness, spec["prompt"], workdir), cwd=workdir, env=env,
            capture_output=True, text=True, encoding="utf-8", errors="replace", timeout=TIMEOUT,
        )
        code, stdout, stderr = proc.returncode, proc.stdout, proc.stderr
    except subprocess.TimeoutExpired as err:
        code, stdout, stderr = "timeout", err.stdout or "", err.stderr or ""
        if isinstance(stdout, bytes):
            stdout = stdout.decode("utf-8", "replace")
        if isinstance(stderr, bytes):
            stderr = stderr.decode("utf-8", "replace")
    wall = time.time() - start
    with open(workdir + ".stdout.jsonl", "w", encoding="utf-8") as f:
        f.write(stdout)
    with open(workdir + ".stderr.txt", "w", encoding="utf-8") as f:
        f.write(stderr)
    all_changed = changed_files(workdir, ignore=False)
    changed = changed_files(workdir)
    unrelated = [p for p in changed if p not in spec["allowed"]]
    transaction_leak = any(p.startswith(".davinci-transactions/") for p in all_changed)
    ok, passed, failed, tail = grade(tid, workdir)
    s = parse_stream(harness, stdout)
    result = {
        "harness": harness, "task": tid, "rep": rep, "exit": code, "pass": ok,
        "tests_passed": passed, "tests_failed": failed, "pytest": tail,
        "wall_s": round(wall, 1), "input_tokens": s["input"], "cached_tokens": s["cached"],
        "output_tokens": s["output"], "tool_calls": s["tool_calls"], "requests": s["requests"],
        "tools": s["tools"], "changed": changed, "unrelated": unrelated,
        "transaction_leak": transaction_leak,
    }
    with open(os.path.join(RUNS, "results.jsonl"), "a", encoding="utf-8") as f:
        f.write(json.dumps(result) + "\n")
    print(json.dumps(result), flush=True)
    return result


def validate():
    bad = 0
    for tid in task_ids():
        d = os.path.join(RUNS, "_validate", tid)
        prepare(tid, d)
        before = grade(tid, d)
        prepare(tid, d)
        copy_tree(os.path.join(TASKS, tid, "solution"), d)
        after = grade(tid, d)
        good = (not before[0]) and after[0]
        bad += not good
        print(f"{tid:14} start={'PASS' if before[0] else 'fail'} "
              f"solution={'PASS' if after[0] else 'FAIL'}  {after[3]}")
    sys.exit(1 if bad else 0)


def load_rows(path=None):
    path = path or os.path.join(RUNS, "results.jsonl")
    with open(path, encoding="utf-8") as f:
        return [json.loads(line) for line in f if line.strip()]


def summarize(rows):
    out = {}
    for harness in sorted({r["harness"] for r in rows}):
        rs = [r for r in rows if r["harness"] == harness]
        total_in = sum(r["input_tokens"] for r in rs)
        cached = sum(r["cached_tokens"] for r in rs)
        out[harness] = {
            "runs": len(rs),
            "passes": sum(bool(r["pass"]) for r in rs),
            "median_wall_s": statistics.median(r["wall_s"] for r in rs),
            "total_wall_s": round(sum(r["wall_s"] for r in rs), 1),
            "input_tokens": total_in,
            "cached_tokens": cached,
            "uncached_input": total_in - cached,
            "cache_ratio": round(cached / total_in, 3) if total_in else 0.0,
            "output_tokens": sum(r["output_tokens"] for r in rs),
            "median_tool_calls": statistics.median(r.get("tool_calls", 0) for r in rs),
            "median_requests": statistics.median(r.get("requests", 0) for r in rs),
            "unrelated_runs": sum(bool(r["unrelated"]) for r in rs),
            "transaction_leak_runs": sum(bool(r.get("transaction_leak")) for r in rs),
        }
    return out


def report():
    rows = load_rows()
    summary = summarize(rows)
    with open(os.path.join(RUNS, "summary.json"), "w", encoding="utf-8") as f:
        json.dump(summary, f, indent=2)
    for harness, s in summary.items():
        print(f"{harness:8} pass={s['passes']}/{s['runs']} median_wall={s['median_wall_s']:.0f}s "
              f"uncached_in={s['uncached_input']:,} cache={100 * s['cache_ratio']:.0f}% "
              f"out={s['output_tokens']:,} tool_calls(median)={s['median_tool_calls']} "
              f"requests(median)={s['median_requests']} unrelated={s['unrelated_runs']} "
              f"txn_leak={s['transaction_leak_runs']}")
    print()
    for r in rows:
        print(f"{r['harness']:8} {r['task']:14} r{r['rep']} {'PASS' if r['pass'] else 'FAIL'} "
              f"{r['wall_s']:>6}s in={r['input_tokens']:>8,} cached={r['cached_tokens']:>8,} "
              f"out={r['output_tokens']:>6,} calls={r.get('tool_calls', 0):>3} "
              f"tests={r['tests_passed']}/{r['tests_passed'] + r['tests_failed']}")


def gate():
    s = summarize(load_rows()).get("davinci")
    if not s:
        print("no davinci rows")
        sys.exit(2)
    checks = [
        ("passes", s["passes"] >= GATES["min_passes"], f"{s['passes']} >= {GATES['min_passes']}"),
        ("median wall", s["median_wall_s"] <= GATES["max_median_wall_s"],
         f"{s['median_wall_s']:.1f}s <= {GATES['max_median_wall_s']}s"),
        ("uncached input", s["uncached_input"] <= GATES["max_uncached_input"],
         f"{s['uncached_input']:,} <= {GATES['max_uncached_input']:,}"),
        ("output tokens", s["output_tokens"] <= GATES["max_output_tokens"],
         f"{s['output_tokens']:,} <= {GATES['max_output_tokens']:,}"),
        ("median tool calls", s["median_tool_calls"] <= GATES["max_median_tool_calls"],
         f"{s['median_tool_calls']} <= {GATES['max_median_tool_calls']}"),
        ("transaction leaks", s["transaction_leak_runs"] <= GATES["max_transaction_leak_runs"],
         f"{s['transaction_leak_runs']} <= {GATES['max_transaction_leak_runs']}"),
    ]
    failed = 0
    for name, ok, detail in checks:
        failed += not ok
        print(f"{'PASS' if ok else 'FAIL'}  {name:18} {detail}")
    sys.exit(1 if failed else 0)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("mode", choices=["validate", "run", "report", "gate"])
    ap.add_argument("--harness", nargs="+", default=["davinci", "codex"])
    ap.add_argument("--tasks", nargs="+", default=["all"])
    ap.add_argument("--reps", type=int, default=1)
    ap.add_argument("--rep-start", type=int, default=0)
    args = ap.parse_args()
    os.makedirs(RUNS, exist_ok=True)
    if args.mode == "validate":
        validate()
    elif args.mode == "report":
        report()
    elif args.mode == "gate":
        gate()
    else:
        tids = task_ids() if args.tasks == ["all"] else args.tasks
        for rep in range(args.rep_start, args.rep_start + args.reps):
            for tid in tids:
                for harness in args.harness:
                    run_one(harness, tid, rep)


if __name__ == "__main__":
    sys.stdout.reconfigure(encoding="utf-8")
    main()
