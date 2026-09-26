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
import hashlib
import json
import os
import re
import shutil
import statistics
import subprocess
import sys
import time
from pathlib import Path
from contextlib import nullcontext
from observations import activity
from campaign import LEGACY_TASKS, COMPARABLE, fixture_manifest, schedule, task_success, manifest_errors, paired_metrics, file_hash, digest, metric
from runner import (create_campaign, isolate_settings, controlled_environment, select_tasks,
                    execute, BASE_SETTINGS, agent_source, campaign_identity, stop_reason,
                    pin_model_store, model_stop_reason, validate_large_manifest)
from codex_otel import Collector, request_metrics

HERE = os.path.dirname(os.path.abspath(__file__))
TASKS = os.path.join(HERE, "tasks")
RUNS = os.environ.get("BENCH_RUNS", os.path.join(HERE, "runs"))
MODEL = os.environ.get("BENCH_MODEL", "gpt-6-luna")
EFFORT = os.environ.get("BENCH_EFFORT", "medium")
TIMEOUT = int(os.environ.get("BENCH_TIMEOUT", "900"))
IGNORED = ("__pycache__", ".pytest_cache", ".git/", ".pi/", ".davinci/", ".codex/",
           ".davinci-transactions/", ".serena/")


def task_ids():
    return list(LEGACY_TASKS)


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
    result = git(workdir, "status", "--porcelain=v1", "-z", "-uall")
    if result.returncode != 0:
        raise RuntimeError("cannot establish changed paths: git status failed")
    records = iter(result.stdout.split("\0"))
    paths = set()
    ignored = {part.rstrip("/") for part in IGNORED}
    for record in records:
        if not record:
            continue
        if len(record) < 4 or record[2] != " ":
            raise RuntimeError("invalid git status record")
        members = [record[3:]]
        if "R" in record[:2] or "C" in record[:2]:
            source = next(records, "")
            if not source:
                raise RuntimeError("incomplete git rename record")
            members.append(source)
        for path in members:
            if not ignore or not any(part in ignored for part in path.split("/")[:-1]):
                paths.add(path)
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
    stats = {"input": None, "cached": None, "output": None,
             "tool_calls": 0, "requests": None, "user_turns": 0, "tools": {}}
    usage_seen = False
    missing = set()
    events = []
    completed_usage = {}
    request_usage = []
    usage_conflict = False
    reasoning_values = []

    def add_usage(values):
        nonlocal usage_seen
        usage_seen = True
        if (type(values.get("input")) is int and type(values.get("cached")) is int
                and values["cached"] > values["input"]):
            values = dict(values, input=None, cached=None)
        for key, value in values.items():
            if not isinstance(value, int) or isinstance(value, bool) or value < 0:
                missing.add(key)
                stats[key] = None
            elif key not in missing:
                stats[key] = (stats[key] or 0) + value
    for line in stdout.splitlines():
        line = line.strip()
        if not line.startswith("{"):
            continue
        try:
            event = json.loads(line)
        except ValueError:
            continue
        if not isinstance(event, dict):
            continue
        events.append(event)
        if harness == "codex":
            if event.get("type") == "turn.completed":
                stats["user_turns"] += 1
                u = event.get("usage")
                u = u if isinstance(u, dict) else {}
                add_usage({"input": u.get("input_tokens"),
                           "cached": u.get("cached_input_tokens"),
                           "output": u.get("output_tokens")})
                reasoning_values.append(u.get("reasoning_output_tokens"))
            item = event.get("item") or {}
            item = item if isinstance(item, dict) else {}
            if event.get("type") == "item.completed" and item.get("type") not in (
                None, "agent_message", "reasoning", "error"
            ):
                stats["tool_calls"] += 1
                stats["tools"][item["type"]] = stats["tools"].get(item["type"], 0) + 1
        else:
            update = event.get("assistantMessageEvent") or {}
            update = update if isinstance(update, dict) else {}
            if event.get("type") == "message_update" and update.get("type") == "done":
                u = event.get("usage")
                u = u if isinstance(u, dict) else {}
                message = update.get("message")
                identity = message.get("id") if isinstance(message, dict) else None
                if isinstance(identity, str) and identity:
                    if identity in completed_usage:
                        if completed_usage[identity] != u:
                            add_usage({"input": None, "cached": None, "output": None})
                            usage_conflict = True
                        continue
                    completed_usage[identity] = u
                uncached, cached, written = u.get("input"), u.get("cacheRead"), u.get("cacheWrite", 0)
                valid = all(isinstance(v, int) and not isinstance(v, bool) and v >= 0
                            for v in (uncached, cached, written))
                total = uncached + cached + written if valid else None
                request_usage.append({"input_tokens": total, "cached_tokens": cached if valid else None})
                reasoning_values.append(u.get("reasoning"))
                add_usage({"input": total,
                           "cached": cached, "output": u.get("output")})
                # Completed messages are not authoritative dispatch telemetry.
                stats["completed_assistant_messages"] = (
                    stats.get("completed_assistant_messages", 0) + 1)
            if event.get("type") == "tool_execution_start":
                name = event.get("toolName") or "?"
                name = name if isinstance(name, str) else "?"
                stats["tool_calls"] += 1
                stats["tools"][name] = stats["tools"].get(name, 0) + 1
    stats["usage_available"] = usage_seen and not missing
    if harness == "davinci":
        stats.update(activity(events))
    stats["reasoning_tokens"] = (sum(reasoning_values) if reasoning_values and not usage_conflict
        and all(type(v) is int and v >= 0 for v in reasoning_values) else None)
    # Both benchmark routes use Responses output totals, which include reasoning.
    stats["reasoning_included_in_output"] = True
    groups_available = (harness == "davinci" and request_usage and not usage_conflict
                        and (stats.get("logical_requests") in (None, len(request_usage))))
    for name, values in (("first", request_usage[:1]), ("later", request_usage[1:])):
        for field in ("input_tokens", "cached_tokens"):
            stats[f"{name}_request_{field}"] = aggregate_available(values, field) if groups_available else None
    return stats


def run_one(harness, tid, rep, campaign=None):
    spec = load(tid)
    workdir = os.path.join(RUNS, harness, f"{tid}-r{rep}")
    if os.path.exists(workdir):
        raise FileExistsError("run directory already exists")
    prepare(tid, workdir)
    env = (controlled_environment(os.environ, campaign["agent_dir"])
           if campaign else dict(os.environ, PI_LEARNING_DISABLE_BACKGROUND="1", PYTHONUTF8="1"))
    with Collector() if harness == "codex" else nullcontext() as collector:
        args = command(harness, spec["prompt"], workdir)
        if campaign:
            args[0] = campaign["executables"][harness]
            if file_hash(args[0]) != campaign["identities"][harness]["binary_sha256"]:
                raise ValueError("binary changed during campaign")
            catalog_hash = campaign["identities"][harness].get("model_catalog_hash")
            if catalog_hash:
                catalog = json.loads(Path(campaign["agent_dir"], "models-store.json").read_text(encoding="utf-8"))
                if digest(catalog) != catalog_hash:
                    raise ValueError("model catalog changed during campaign")
        if collector:
            for value in collector.overrides():
                args[-1:-1] = ["-c", value]
        measured = execute(args, workdir, env, TIMEOUT)
    code, stdout, stderr = measured["exit"], measured["stdout"], measured["stderr"]
    stopped = stop_reason(stdout, stderr)
    if harness == "davinci":
        stopped = stopped or model_stop_reason(stdout, MODEL)
    wall = measured["wall_s"]
    with open(workdir + ".stdout.jsonl", "w", encoding="utf-8") as f:
        f.write(stdout)
    with open(workdir + ".stderr.txt", "w", encoding="utf-8") as f:
        f.write(stderr)
    try:
        all_changed = changed_files(workdir, ignore=False)
        changed = changed_files(workdir)
        unrelated = [p for p in changed if p not in spec["allowed"]]
        transaction_leak = any(p.startswith(".davinci-transactions/") for p in all_changed)
    except (OSError, RuntimeError, subprocess.SubprocessError):
        changed = unrelated = transaction_leak = None
        stopped = stopped or "change_inventory_failed"
    if stopped:
        ok, passed, failed, tail = False, 0, 0, "not run: " + stopped
    else:
        try:
            ok, passed, failed, tail = grade(tid, workdir)
        except (subprocess.TimeoutExpired, OSError) as error:
            ok, passed, failed, tail = False, 0, 0, type(error).__name__
    s = parse_stream(harness, stdout)
    if collector:
        s.update(request_metrics(collector.records, collector.spans, collector.errors))
        Path(workdir + ".otel.json").write_text(json.dumps({
            "records": collector.records, "spans": collector.spans,
            "collection_errors": collector.error_categories}), encoding="utf-8")
    result = {
        "harness": harness, "task": tid, "rep": rep, "exit": code, "grader_pass": ok,
        "tests_passed": passed, "tests_failed": failed, "pytest": tail,
        "wall_s": wall, "input_tokens": s["input"], "cached_tokens": s["cached"],
        "output_tokens": s["output"], "tool_calls": s["tool_calls"], "requests": s["requests"],
        "tools": s["tools"], "changed": changed, "unrelated": unrelated,
        "transaction_leak": transaction_leak, "artifact_leak": transaction_leak,
        "started_at": measured["started_at"], "finished_at": measured["finished_at"],
        "grading_isolation": "diagnostic-only",
        "stop_reason": stopped,
    }
    for key in ("logical_requests", "provider_attempts", "prewarm_attempts", "jev_attempts",
                "gate_reminders", "auto_verify_runs", "requests_after_first_reminder",
                "mutations_after_reminder", "executed_leaf_operations", "batch_children_reported",
                "mutation_metric_scope", "executed_leaf_scope",
                "reasoning_tokens", "reasoning_included_in_output",
                "first_request_input_tokens", "first_request_cached_tokens",
                "later_request_input_tokens", "later_request_cached_tokens",
                "request_metrics_available", "request_metrics_complete", "usage_available"):
        result[key] = s.get(key)
    if collector:
        result["request_metrics_available"] = s.get("logical_requests") is not None
        result["request_metrics_complete"] = s["request_telemetry_complete"]
    if campaign:
        result.update({key: campaign[key] for key in
            ("schema_version", "campaign", "variant", "fixture_hash", "model", "effort_policy", "service_tier")})
        result.update(campaign["identities"][harness])
    result["pass"] = task_success(result)
    with open(os.path.join(RUNS, "results.jsonl"), "a", encoding="utf-8") as f:
        f.write(json.dumps(result) + "\n")
    print(json.dumps(result), flush=True)
    if stopped:
        raise RuntimeError("campaign stopped: " + stopped)
    return result


def directory_hash(directory):
    """Hash one generated fixture tree without exposing its contents."""
    digest_value = hashlib.sha256()
    directory = Path(directory).resolve()
    for path in sorted(path for path in directory.rglob("*") if path.is_file()):
        digest_value.update(path.relative_to(directory).as_posix().encode("utf-8"))
        digest_value.update(b"\0")
        digest_value.update(path.read_bytes())
        digest_value.update(b"\0")
    return digest_value.hexdigest()


def validate_fixture_manifest(task_ids, large_manifest):
    """Check generated task metadata and hashes before running graders."""
    if large_manifest is None:
        return []
    validate_large_manifest(large_manifest)
    errors = []
    for tid in task_ids:
        metadata = load(tid)
        expected = large_manifest["fixtures"][tid]
        for field in ("task_set", "allowed", "public_verification"):
            if metadata.get(field) != expected.get(field):
                errors.append(f"{tid}: metadata mismatch for {field}")
        for directory, field in (("repo", "public_hash"), ("solution", "reference_solution_hash"),
                                 ("hidden", "hidden_grader_hash")):
            actual = directory_hash(Path(TASKS) / tid / directory)
            if actual != expected.get(field):
                errors.append(f"{tid}: {directory} hash does not match frozen manifest")
        allowed = set(metadata.get("allowed", []))
        repo = Path(TASKS) / tid / "repo"
        missing = sorted(path for path in allowed if not (repo / path).is_file())
        if missing:
            errors.append(f"{tid}: allowlist paths missing from public repo: {missing}")
    return errors


def validate(task_set="legacy", large_manifest=None):
    task_ids = select_tasks(task_set, ["all"], large_manifest)
    manifest_errors = validate_fixture_manifest(task_ids, large_manifest) if task_set != "legacy" else []
    if manifest_errors:
        for error in manifest_errors:
            print("FAIL " + error)
        sys.exit(1)
    bad = 0
    for tid in task_ids:
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


def aggregate_available(rows, field, operation=sum):
    values = [r.get(field) for r in rows]
    return operation(values) if values and all(v is not None for v in values) else None


def display(value, spec=""):
    return "unavailable" if value is None else format(value, spec)


def summarize(rows):
    out = {}
    for harness in sorted({r["harness"] for r in rows}):
        rs = [r for r in rows if r["harness"] == harness]
        total_in = aggregate_available(rs, "input_tokens")
        cached = aggregate_available(rs, "cached_tokens")
        out[harness] = {
            "runs": len(rs),
            "passes": sum(bool(r["pass"]) for r in rs),
            "median_wall_s": statistics.median(r["wall_s"] for r in rs),
            "p90_wall_s": percentile([r["wall_s"] for r in rs], 0.9),
            "total_wall_s": sum(r["wall_s"] for r in rs),
            "input_tokens": total_in,
            "cached_tokens": cached,
            "uncached_input": total_in - cached if total_in is not None and cached is not None else None,
            "cache_ratio": cached / total_in if total_in and cached is not None else None,
            "output_tokens": aggregate_available(rs, "output_tokens"),
            "reasoning_tokens": aggregate_available(rs, "reasoning_tokens"),
            "median_tool_calls": statistics.median(r.get("tool_calls", 0) for r in rs),
            "median_requests": aggregate_available(rs, "requests", statistics.median),
            "logical_requests": aggregate_available(rs, "logical_requests"),
            "provider_attempts": aggregate_available(rs, "provider_attempts"),
            "prewarm_attempts": aggregate_available(rs, "prewarm_attempts"),
            "auto_verify_runs": aggregate_available(rs, "auto_verify_runs"),
            "unrelated_runs": sum(bool(r["unrelated"]) for r in rs),
            "transaction_leak_runs": sum(bool(r.get("transaction_leak")) for r in rs),
        }
        for name in ("first", "later"):
            total = aggregate_available(rs, f"{name}_request_input_tokens")
            hits = aggregate_available(rs, f"{name}_request_cached_tokens")
            out[harness][f"{name}_request_cache_ratio"] = hits / total if total and hits is not None else None
        for field in ("gate_reminder_count", "tool_calls", "executed_leaf_operations",
                      "requests_after_first_reminder", "mutations_after_reminder"):
            values = [metric(row, field) for row in rs]
            out[harness][field] = sum(values) if all(value is not None for value in values) else None
        reasons = [row.get("gate_reminders") for row in rs]
        out[harness]["gate_reminders"] = ({reason: sum(counts.get(reason, 0) for counts in reasons)
            for reason in sorted({key for counts in reasons for key in counts})}
            if all(isinstance(counts, dict) for counts in reasons) else None)
    return out


def percentile(values, fraction):
    ordered = sorted(values)
    position = (len(ordered) - 1) * fraction
    lower = int(position)
    upper = min(lower + 1, len(ordered) - 1)
    return ordered[lower] + (ordered[upper] - ordered[lower]) * (position - lower)


def campaign_errors(root, rows):
    with open(Path(root) / "campaign.json", encoding="utf-8") as source:
        manifest = json.load(source)
    return manifest_errors(manifest, rows)


def compare(baseline, candidate):
    parents = load_rows(str(Path(baseline) / "results.jsonl"))
    children = load_rows(str(Path(candidate) / "results.jsonl"))
    errors = campaign_errors(baseline, parents) + campaign_errors(candidate, children)
    if parents and children:
        for field in COMPARABLE:
            if parents[0].get(field) != children[0].get(field):
                errors.append("baseline and candidate differ: " + field)
    if errors:
        print(json.dumps({"integrity_errors": errors, "comparison_available": False}, indent=2))
        return 1
    prior = {(r["task"], r["rep"]): r for r in parents if r["harness"] == "davinci"}
    regressions = [{"task": r["task"], "rep": r["rep"], "cause": "requires investigation"}
                   for r in children if r["harness"] == "davinci"
                   and prior.get((r["task"], r["rep"]), {}).get("pass") and not r["pass"]]
    paired = {}
    try:
        paired["candidate_vs_parent"] = paired_metrics(
            [r for r in parents if r["harness"] == "davinci"],
            [r for r in children if r["harness"] == "davinci"])
        for name, rows in (("parent_vs_codex", parents), ("candidate_vs_codex", children)):
            paired[name] = paired_metrics([r for r in rows if r["harness"] == "codex"],
                                          [r for r in rows if r["harness"] == "davinci"])
    except ValueError as error:
        errors.append(str(error))
    result = {"baseline": summarize(parents), "candidate": summarize(children), "paired": paired,
              "integrity_errors": errors, "parent_pass_candidate_fail": regressions,
              "scope": "diagnostic screening; no general parity claim",
              "acceptance": "requires checkpoint review; exit zero establishes no integrity or observed pass regression"}
    print(json.dumps(result, indent=2))
    return 1 if errors or regressions else 0


def report():
    rows = load_rows()
    summary = summarize(rows)
    with open(os.path.join(RUNS, "summary.json"), "w", encoding="utf-8") as f:
        json.dump(summary, f, indent=2)
    for harness, s in summary.items():
        print(f"{harness:8} pass={s['passes']}/{s['runs']} median_wall={s['median_wall_s']:.0f}s "
              f"uncached_in={display(s['uncached_input'], ',')} cache_ratio={display(s['cache_ratio'], '.3f')} "
              f"out={display(s['output_tokens'], ',')} tool_calls(median)={s['median_tool_calls']} "
              f"requests(median)={s['median_requests']} unrelated={s['unrelated_runs']} "
              f"txn_leak={s['transaction_leak_runs']}")
    print()
    for r in rows:
        print(f"{r['harness']:8} {r['task']:14} r{r['rep']} {'PASS' if r['pass'] else 'FAIL'} "
              f"{r['wall_s']:>6}s in={display(r['input_tokens'], '>8,')} cached={display(r['cached_tokens'], '>8,')} "
              f"out={display(r['output_tokens'], '>6,')} calls={r.get('tool_calls', 0):>3} "
              f"tests={r['tests_passed']}/{r['tests_passed'] + r['tests_failed']}")


def gate():
    manifest_path = Path(RUNS) / "campaign.json"
    if not manifest_path.exists():
        print("FAIL campaign completeness: missing campaign.json")
        sys.exit(1)
    rows = load_rows()
    errors = campaign_errors(RUNS, rows)
    manifest = json.loads(manifest_path.read_text(encoding="utf-8"))
    if set(manifest.get("harnesses", [])) != {"davinci", "codex"}:
        errors.append("paired screening requires both DaVinci and Codex")
    if errors:
        print("FAIL campaign completeness: " + "; ".join(errors))
        sys.exit(1)
    print(f"PASS campaign completeness: {len(rows)} unique pinned rows; not checkpoint acceptance")
    print("Use compare and review regressions, metric availability, targeted tests, and replay before acceptance.")
    sys.exit(0)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("mode", choices=["validate", "run", "report", "gate", "compare"])
    ap.add_argument("--harness", nargs="+", choices=["davinci", "codex"], default=["davinci", "codex"])
    ap.add_argument("--tasks", nargs="+", default=["all"])
    ap.add_argument("--reps", type=int, default=1)
    ap.add_argument("--rep-start", type=int, default=0)
    ap.add_argument("--order", choices=["counterbalanced", "input"], default="counterbalanced")
    ap.add_argument("--order-seed", type=int, default=0)
    ap.add_argument("--task-set", choices=["legacy", "large", "all"], default="legacy")
    ap.add_argument("--large-manifest")
    ap.add_argument("--variant", default="baseline")
    ap.add_argument("--settings", help="explicit DaVinci settings JSON")
    ap.add_argument("--model-store", help="existing public model catalog to pin for DaVinci")
    ap.add_argument("--baseline")
    ap.add_argument("--candidate")
    args = ap.parse_args()
    if args.mode == "validate":
        large = json.loads(Path(args.large_manifest).read_text(encoding="utf-8")) if args.large_manifest else None
        try:
            validate(args.task_set, large)
        except ValueError as error:
            ap.error(str(error))
    elif args.mode == "report":
        report()
    elif args.mode == "gate":
        gate()
    elif args.mode == "compare":
        if not args.baseline or not args.candidate:
            ap.error("compare requires --baseline and --candidate")
        sys.exit(compare(args.baseline, args.candidate))
    else:
        if args.reps < 1 or args.rep_start < 0:
            ap.error("reps must be positive and rep-start nonnegative")
        large = json.loads(Path(args.large_manifest).read_text(encoding="utf-8")) if args.large_manifest else None
        tids = select_tasks(args.task_set, args.tasks, large)
        fixtures = fixture_manifest(TASKS, tids)
        settings = json.loads(Path(args.settings).read_text(encoding="utf-8")) if args.settings else BASE_SETTINGS
        manifest = campaign_identity(RUNS, args.variant, fixtures, args.harness,
                                     MODEL, EFFORT, settings, Path(HERE).parents[1])
        repetitions = list(range(args.rep_start, args.rep_start + args.reps))
        order = (schedule(tids, repetitions, args.harness, args.order_seed)
                 if args.order == "counterbalanced" else
                 [(h, t, r) for r in repetitions for t in tids for h in args.harness])
        manifest.update(tasks=tids, repetitions=repetitions, harnesses=args.harness,
                        order=args.order, order_seed=args.order_seed, schedule=order)
        pinned_models = pin_model_store(args.model_store, MODEL) if args.model_store else None
        if pinned_models:
            manifest["identities"]["davinci"]["model_catalog_hash"] = digest(pinned_models)
        create_campaign(RUNS, manifest)
        Path(RUNS, "fixtures.json").write_text(json.dumps(fixtures, indent=2), encoding="utf-8")
        if "davinci" in args.harness:
            isolate_settings(agent_source(), Path(manifest["agent_dir"]), settings)
            if pinned_models:
                Path(manifest["agent_dir"], "models-store.json").write_text(
                    json.dumps(pinned_models, indent=2), encoding="utf-8")
        for harness, tid, rep in order:
            if fixture_manifest(TASKS, tids)["fixture_hash"] != manifest["fixture_hash"]:
                raise ValueError("fixture changed during campaign")
            run_one(harness, tid, rep, manifest)


if __name__ == "__main__":
    sys.stdout.reconfigure(encoding="utf-8")
    main()
