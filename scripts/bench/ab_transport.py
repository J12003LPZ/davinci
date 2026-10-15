"""Fixed schedule and pre-registered decision rule for the HTTP versus public-WebSocket A/B.

Prepares and evaluates only. It never launches a model run: `commands` prints the PowerShell
lines an approved operator would run, one single-run campaign per line, and `report` reads
campaign result rows that already exist.

Three arms use the same experimental DaVinci build; only the `transport` setting differs:
  http -> "auto"                          (the production default on the public route: HTTP/SSE)
  ws   -> "public-websocket"              one socket per conversation, full input replayed
  wsc  -> "public-websocket-continuation" same socket, plus previous_response_id with new items only

The two effects are separable by construction: ws versus http isolates the persistent
connection, wsc versus ws isolates continuation, wsc versus http is the combined effect.
"""
import argparse
import json
from pathlib import Path
import statistics
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import ab_cache_key as cache_ab  # noqa: E402  (shared row metrics; thresholds below are separate)

ARMS = ("http", "ws", "wsc")
TRANSPORT = {"http": "auto", "ws": "public-websocket", "wsc": "public-websocket-continuation"}
TASKS = cache_ab.TASKS
WARMUP_TASK = cache_ab.WARMUP_TASK
# Warm-ups come first and double as the compatibility probe: a WebSocket failure stops the
# experiment on its first request. Each arm is warmed once so no arm starts cold alone.
WARMUP_ARMS = ("ws", "wsc", "http")
# Arm order per task. Not perfectly balanced with four tasks; the report says so.
ORDERS = (("http", "ws", "wsc"), ("ws", "wsc", "http"), ("wsc", "http", "ws"), ("wsc", "ws", "http"))

# Fixed before any run. Twelve measured runs show a direction, never a default.
MIN_RATIO_GAIN = 0.10         # median gain in cache ratio over requests 2..n
MIN_TASKS = 3                 # of 4: lower uncached input AND higher later ratio
MAX_REQUEST_GROWTH = 1.10     # requests, totals
MAX_WALL_GROWTH = 1.10        # wall seconds, totals
WS_LABELS = {"ws": {"websocket"}, "wsc": {"websocket", "websocket-continuation"}}
COMPARISONS = (("ws", "http"), ("wsc", "http"), ("wsc", "ws"))


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


def commands(root, settings_dir, manifest, bench="scripts/bench/bench.py"):
    lines = []
    for run in schedule():
        name = "ws-%02d-%s-%s" % (run["seq"], run["role"], run["arm"])
        lines.append(
            '$env:BENCH_RUNS = "%s\\%s"; python %s run --harness davinci --task-set large '
            '--large-manifest %s --tasks %s --reps 1 --rep-start %d --variant %s '
            '--settings %s\\%s.json --grading-timeout 120'
            % (root, name, bench, manifest, run["task"], run["rep"], name, settings_dir, run["arm"]))
    return lines


def load_rows(root):
    rows = {}
    for path in sorted(Path(root).glob("ws-*/results.jsonl")):
        seq = int(path.parent.name.split("-")[1])
        for line in path.read_text(encoding="utf-8").splitlines():
            if line.strip():
                rows[seq] = json.loads(line)
    return rows


def arm_errors(plan, rows):
    """Every run must prove, from its own row, that it used the transport its arm asked for."""
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
        else:
            if not set(labels) <= WS_LABELS[arm]:
                errors.append("run %d: arm %s used %s (a fallback or the wrong request shape)" % (seq, arm, sorted(labels)))
            elif arm == "wsc" and "websocket-continuation" not in labels:
                errors.append("run %d: wsc arm never sent a continuation" % seq)
    return errors


MAX_UNKNOWN_ATTEMPTS_RUN = 1   # provider attempts without a complete usage receipt, one run
MAX_UNKNOWN_ATTEMPTS_TOTAL = 3


def stop_reason(run, row, unknown_so_far=0):
    """Predefined stop conditions for one finished run; None means continue. A stop ends the experiment.

    `unknown_so_far` counts attempts without a usage receipt in the earlier runs. The checks are
    the ones in the protocol, in order, and read only fields the harness already wrote.
    """
    if row.get("stop_reason"):
        return "harness stop: %s" % row["stop_reason"]
    errors = arm_errors({run["seq"]: run}, {run["seq"]: row})
    if errors:
        return errors[0]
    unknown = row.get("usage_unknown_attempts")
    if run["arm"] != "http" and (row.get("input_tokens") is None or (type(unknown) is int and unknown > 0)):
        return "run %d: usage incomplete on a WebSocket run" % run["seq"]
    if type(unknown) is int:
        if unknown > MAX_UNKNOWN_ATTEMPTS_RUN:
            return "run %d: %d attempts without usage" % (run["seq"], unknown)
        if unknown_so_far + unknown > MAX_UNKNOWN_ATTEMPTS_TOTAL:
            return "run %d: %d attempts without usage in total" % (run["seq"], unknown_so_far + unknown)
    return None


def compare(tasks, arm, base):
    a, b = tasks[base], tasks[arm]
    drops, gains, lower, higher = [], [], 0, 0
    for index in sorted(a):
        drops.append((b[index]["uncached"] - a[index]["uncached"]) / a[index]["uncached"])
        lower += b[index]["uncached"] < a[index]["uncached"]
        gain = None
        if a[index]["ratio_later"] is not None and b[index]["ratio_later"] is not None:
            gain = b[index]["ratio_later"] - a[index]["ratio_later"]
            gains.append(gain)
        higher += bool(gain is not None and gain > 0)
    total = lambda side, key: sum(m[key] for m in side.values())
    unc = {base: total(a, "uncached"), arm: total(b, "uncached")}
    net = (unc[arm] - unc[base]) / unc[base]
    median_gain = statistics.median(gains) if gains else None
    passes = {base: sum(m["pass"] for m in a.values()), arm: sum(m["pass"] for m in b.values())}
    requests = {base: total(a, "requests"), arm: total(b, "requests")}
    wall = {base: total(a, "wall"), arm: total(b, "wall")}
    both = [index for index in sorted(a) if a[index]["ratio_later"] is not None and b[index]["ratio_later"] is not None]
    wins = sum(1 for index in both if b[index]["uncached"] < a[index]["uncached"] and b[index]["ratio_later"] > a[index]["ratio_later"])
    promising = all([
        median_gain is not None and median_gain >= MIN_RATIO_GAIN, wins >= MIN_TASKS, net < 0,
        passes[arm] >= passes[base], requests[arm] <= MAX_REQUEST_GROWTH * requests[base],
        wall[arm] <= MAX_WALL_GROWTH * wall[base]])
    if promising:
        verdict = "PROMISING"
    elif net >= 0 or (median_gain is not None and median_gain <= 0):
        verdict = "NOT_PROMISING"
    else:
        verdict = "MIXED"
    return {"verdict": verdict, "median_later_ratio_gain": median_gain, "tasks_lower_and_higher": wins,
            "tasks_uncached_lower": lower, "tasks_ratio_higher": higher, "net_uncached_change": round(net, 4),
            "uncached": unc, "requests": requests, "wall_s": {k: round(v, 1) for k, v in wall.items()}, "passes": passes,
            "median_first_request_cached_lost": statistics.median(
                [(a[i]["first_cached"] or 0) - (b[i]["first_cached"] or 0) for i in sorted(a)])}


def evaluate(rows):
    plan = {run["seq"]: run for run in schedule()}
    errors = arm_errors(plan, rows)
    if errors:
        return {"verdict": "INVALID", "errors": errors}
    stops = ["run %d: %s" % (s, r.get("stop_reason")) for s, r in sorted(rows.items()) if r.get("stop_reason")]
    if stops:
        return {"verdict": "STOPPED", "stops": stops, "note": "a harness stop ends the experiment; no reruns"}
    measured = {s: cache_ab.metrics(r) for s, r in rows.items() if plan[s]["role"] == "measured"}
    if len(measured) != 12:
        return {"verdict": "INCOMPLETE", "reason": "%d of 12 measured runs present" % len(measured)}
    missing = cache_ab.unscorable(measured)
    if missing:
        return {"verdict": "INCOMPLETE", "reason": "%d of 12 measured runs have unknown usage" % len(missing),
                "unscorable_runs": missing,
                "note": "thresholds are not changed and no usage is imputed; report complete tasks descriptively only"}
    tasks = {arm: {} for arm in ARMS}
    for seq, metric in measured.items():
        tasks[plan[seq]["arm"]][plan[seq]["task_index"]] = metric
    results = {"%s_vs_%s" % (arm, base): compare(tasks, arm, base) for arm, base in COMPARISONS}
    transport, combined, continuation = (results["ws_vs_http"]["verdict"], results["wsc_vs_http"]["verdict"],
                                         results["wsc_vs_ws"]["verdict"])
    if "PROMISING" in (transport, combined):
        verdict = "PROMISING"
    elif {transport, combined} == {"NOT_PROMISING"}:
        verdict = "NOT_PROMISING"
    else:
        verdict = "MIXED"
    if verdict != "PROMISING":
        attribution = "none"
    elif transport == "PROMISING" and continuation == "PROMISING":
        attribution = "persistent transport and continuation each contribute"
    elif transport == "PROMISING":
        attribution = "persistent transport"
    elif continuation == "PROMISING":
        attribution = "continuation (given a persistent transport)"
    else:
        attribution = "combined only; the two effects are not separable from this data"
    return {"verdict": verdict, "attribution": attribution, **results,
            "note": "12 runs show direction only. PROMISING means a larger study is worth requesting; "
                    "it never supports changing a default. Arm order is balanced only approximately."}


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
            print("%2d %-8s %-4s %s" % (run["seq"], run["role"], run["arm"], run["task"]))
    elif args.mode == "commands":
        print("\n".join(commands(args.root, args.settings_dir, args.manifest)))
    else:
        print(json.dumps(evaluate(load_rows(args.root)), indent=2))


if __name__ == "__main__":
    main()
