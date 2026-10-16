"""Fixed schedule and pre-registered decision rule for the HTTP connection-reuse A/B.

Prepares and evaluates only. It never launches a model run: `commands` prints the PowerShell
lines an approved operator would run, one single-run campaign per line, and `report` reads
campaign result rows that already exist.

Two arms use the same experimental DaVinci build; only the `transport` setting differs:
  http  -> "auto"        (the production default: a new TCP and TLS connection for every request)
  reuse -> "http-reuse"  (one pooled connection per conversation, same requests, no WebSocket)

The question is whether the connection alone explains the cache-reuse gap that the 2026-10
WebSocket A/B measured. Thresholds are the ones of `ab_transport`, so the two experiments are judged alike.
"""
import argparse
import json
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import ab_cache_key as cache_ab  # noqa: E402
import ab_transport as transport_ab  # noqa: E402

ARMS = ("http", "reuse")
TRANSPORT = {"http": "auto", "reuse": "http-reuse"}
TASKS = cache_ab.TASKS
WARMUP_TASK = cache_ab.WARMUP_TASK
# The reuse warm-up comes first and doubles as the compatibility probe.
WARMUP_ARMS = ("reuse", "http")
# First arm alternates per task: two tasks start with each arm.
ORDERS = (("http", "reuse"), ("reuse", "http"), ("reuse", "http"), ("http", "reuse"))
MEASURED = len(TASKS) * len(ARMS)
MAX_RUNS = len(WARMUP_ARMS) + MEASURED
REUSE_LABELS = {"http-reuse", "http-reuse-open"}
# Of the attempts after a conversation's first, at least this share must have expected a pooled connection.
MIN_REUSE_SHARE = 0.5
PREFIX = "ru"
MAX_UNKNOWN_ATTEMPTS_RUN = transport_ab.MAX_UNKNOWN_ATTEMPTS_RUN
MAX_UNKNOWN_ATTEMPTS_TOTAL = transport_ab.MAX_UNKNOWN_ATTEMPTS_TOTAL


def schedule():
    runs = [{"role": "warmup", "arm": arm, "task": WARMUP_TASK, "rep": 90 + i, "task_index": None}
            for i, arm in enumerate(WARMUP_ARMS)]
    for index, task in enumerate(TASKS):
        for arm in ORDERS[index]:
            runs.append({"role": "measured", "arm": arm, "task": task, "rep": 0, "task_index": index})
    for seq, run in enumerate(runs, 1):
        run["seq"] = seq
    return runs


def settings_for(arm, base):
    """Arm settings: the shared base unchanged except `transport`."""
    out = json.loads(json.dumps(base))
    out["transport"] = TRANSPORT[arm]
    return out


def run_name(run):
    return "%s-%02d-%s-%s" % (PREFIX, run["seq"], run["role"], run["arm"])


def commands(root, settings_dir, manifest, bench="scripts/bench/bench.py"):
    lines = []
    for run in schedule():
        name = run_name(run)
        lines.append(
            '$env:BENCH_RUNS = "%s\\%s"; python %s run --harness davinci --task-set large '
            '--large-manifest %s --tasks %s --reps 1 --rep-start %d --variant %s '
            '--settings %s\\%s.json --grading-timeout 120'
            % (root, name, bench, manifest, run["task"], run["rep"], name, settings_dir, run["arm"]))
    return lines


def load_rows(root):
    rows = {}
    for path in sorted(Path(root).glob(PREFIX + "-*/results.jsonl")):
        seq = int(path.parent.name.split("-")[1])
        for line in path.read_text(encoding="utf-8").splitlines():
            if line.strip():
                rows[seq] = json.loads(line)
    return rows


def arm_errors(plan, rows):
    """Every run must prove, from its own row, which transport it used and that reuse was exercised."""
    errors = []
    for seq, row in sorted(rows.items()):
        arm = plan[seq]["arm"]
        settings = row.get("effective_settings")
        if not isinstance(settings, dict):
            errors.append("run %d: effective settings not recorded" % seq)
        elif settings.get("transport") != TRANSPORT[arm]:
            errors.append("run %d: arm %s ran with transport %r" % (seq, arm, settings.get("transport")))
        labels = row.get("provider_transports")
        if not isinstance(labels, dict) or not labels:
            errors.append("run %d: provider transports not recorded" % seq)
        elif arm == "http":
            if set(labels) != {"http"}:
                errors.append("run %d: http arm used %s" % (seq, sorted(labels)))
        elif not set(labels) <= REUSE_LABELS:
            errors.append("run %d: reuse arm used %s (the wrong request path)" % (seq, sorted(labels)))
        else:
            total = sum(labels.values())
            if labels.get("http-reuse", 0) < MIN_REUSE_SHARE * (total - 1):
                errors.append("run %d: reuse arm expected a pooled connection on only %d of %d attempts"
                              % (seq, labels.get("http-reuse", 0), total))
    return errors


def stop_reason(run, row, unknown_so_far=0):
    """Predefined stop conditions for one finished run; None means continue. A stop ends the experiment."""
    if row.get("stop_reason"):
        return "harness stop: %s" % row["stop_reason"]
    errors = arm_errors({run["seq"]: run}, {run["seq"]: row})
    if errors:
        return errors[0]
    unknown = row.get("usage_unknown_attempts")
    if type(unknown) is not int and row.get("input_tokens") is None:
        return "run %d: usage incomplete" % run["seq"]
    if type(unknown) is int:
        if unknown > MAX_UNKNOWN_ATTEMPTS_RUN:
            return "run %d: %d attempts without usage" % (run["seq"], unknown)
        if unknown_so_far + unknown > MAX_UNKNOWN_ATTEMPTS_TOTAL:
            return "run %d: %d attempts without usage in total" % (run["seq"], unknown_so_far + unknown)
    return None


def evaluate(rows):
    plan = {run["seq"]: run for run in schedule()}
    errors = arm_errors(plan, rows)
    if errors:
        return {"verdict": "INVALID", "errors": errors}
    stops = ["run %d: %s" % (s, r.get("stop_reason")) for s, r in sorted(rows.items()) if r.get("stop_reason")]
    if stops:
        return {"verdict": "STOPPED", "stops": stops, "note": "a harness stop ends the experiment; no reruns"}
    measured = {s: cache_ab.metrics(r) for s, r in rows.items() if plan[s]["role"] == "measured"}
    if len(measured) != MEASURED:
        return {"verdict": "INCOMPLETE", "reason": "%d of %d measured runs present" % (len(measured), MEASURED)}
    missing = cache_ab.unscorable(measured)
    if missing:
        return {"verdict": "INCOMPLETE", "reason": "%d of %d measured runs have unknown usage" % (len(missing), MEASURED),
                "unscorable_runs": missing,
                "note": "thresholds are not changed and no usage is imputed; report complete tasks descriptively only"}
    tasks = {arm: {} for arm in ARMS}
    for seq, metric in measured.items():
        tasks[plan[seq]["arm"]][plan[seq]["task_index"]] = metric
    result = transport_ab.compare(tasks, "reuse", "http")
    return {"verdict": result["verdict"], "reuse_vs_http": result,
            "note": "%d measured runs show direction only. PROMISING means a larger study is worth requesting; it never "
                    "supports changing a default. The share of the earlier WebSocket gain that this recovers is read "
                    "from the earlier experiment descriptively; the two were not run side by side." % MEASURED}


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = ap.add_subparsers(dest="mode", required=True)
    sub.add_parser("schedule")
    c = sub.add_parser("commands")
    c.add_argument("--root", required=True)
    c.add_argument("--settings-dir", required=True)
    c.add_argument("--manifest", required=True)
    r = sub.add_parser("report")
    r.add_argument("--root", required=True)
    args = ap.parse_args(argv)
    if args.mode == "schedule":
        for run in schedule():
            print("%2d %-8s %-5s %s" % (run["seq"], run["role"], run["arm"], run["task"]))
    elif args.mode == "commands":
        print("\n".join(commands(args.root, args.settings_dir, args.manifest)))
    else:
        print(json.dumps(evaluate(load_rows(args.root)), indent=2))


if __name__ == "__main__":
    main()
