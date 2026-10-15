"""Fixed schedule and pre-registered decision rule for the session cache-key A/B.

Prepares and evaluates only. It never launches a model run: `commands` prints the
PowerShell lines an approved operator would run, one single-run campaign per line,
and `report` reads campaign result rows that already exist.

Arms use the same experimental DaVinci build, so the cache key is the only difference:
  shared  -> settings without experimentalSessionCacheKey (the production default)
  session -> settings with    experimentalSessionCacheKey: true
"""
import argparse
import json
from pathlib import Path
import statistics

ARMS = ("shared", "session")
TASKS = ("m1-config-parser-v2", "m2-public-api-migration-v2",
         "m3-state-persistence-v2", "m4-cli-feature-v2")
WARMUP_TASK = "m2-public-api-migration-v2"
# Fixed before any run. Direction evidence only: 8 pairs cannot show generalization.
MIN_UNCACHED_DROP = 0.25      # median per-pair drop in uncached input, session vs shared
MIN_PAIRS_BETTER = 6          # of 8
MIN_RATIO_GAIN = 0.10         # median gain in cache ratio over requests 2..n
MAX_REQUEST_GROWTH = 1.10     # session requests / shared requests, totals


PROFILES = {
    # profile: (discarded warm-ups, repetitions)
    "full": (4, (0, 1)),
    "pilot": (2, (0,)),
}
# Pilot rule, fixed before any run. It judges whether the larger study is worth approving,
# never whether to change a default. Four pairs cannot show an effect, only a direction.
PILOT_MIN_RATIO_GAIN = 0.10   # median gain in cache ratio over requests 2..n
PILOT_MIN_PAIRS = 3           # of 4: session lower in uncached input AND higher in later ratio


def schedule(profile="full"):
    """Discarded warm-ups, then task pairs with the arm order balanced."""
    warmups, reps = PROFILES[profile]
    runs = [{"role": "warmup", "arm": ARMS[i % 2], "task": WARMUP_TASK, "rep": 90 + i, "pair": None}
            for i in range(warmups)]
    pair = 0
    for rep in reps:
        for position, task in enumerate(TASKS):
            first = ARMS[(position + rep) % 2]
            order = (first, ARMS[1] if first == ARMS[0] else ARMS[0])
            for arm in order:
                runs.append({"role": "measured", "arm": arm, "task": task, "rep": rep, "pair": pair})
            pair += 1
    for seq, run in enumerate(runs, 1):
        run["seq"] = seq
    return runs


def commands(root, settings_dir, manifest, bench="scripts/bench/bench.py", profile="full"):
    lines = []
    for run in schedule(profile):
        name = "ab-%02d-%s-%s" % (run["seq"], run["role"], run["arm"])
        lines.append(
            '$env:BENCH_RUNS = "%s\\%s"; python %s run --harness davinci --task-set large '
            '--large-manifest %s --tasks %s --reps 1 --rep-start %d --variant %s '
            '--settings %s\\%s.json --grading-timeout 120'
            % (root, name, bench, manifest, run["task"], run["rep"], name, settings_dir, run["arm"]))
    return lines


def load_rows(root):
    rows = {}
    for path in sorted(Path(root).glob("ab-*/results.jsonl")):
        seq = int(path.parent.name.split("-")[1])
        for line in path.read_text(encoding="utf-8").splitlines():
            if line.strip():
                rows[seq] = json.loads(line)
    return rows


def metrics(row):
    unc = row["input_tokens"] - row["cached_tokens"]
    later_in, later_cached = row.get("later_request_input_tokens"), row.get("later_request_cached_tokens")
    ratio_later = later_cached / later_in if later_in else None
    settings = row.get("effective_settings")
    mode = None
    if isinstance(settings, dict):
        mode = "session" if settings.get("experimentalSessionCacheKey") is True else "shared"
    return {"uncached": unc, "ratio_later": ratio_later, "requests": row["requests"], "wall": row["wall_s"],
            "cache_write": row.get("cache_write_tokens"), "pass": bool(row["pass"]),
            "first_cached": row.get("first_request_cached_tokens"), "cache_key_mode": mode,
            "stop": row.get("stop_reason")}


def mode_errors(plan, rows, require):
    """Every row must record the cache-key mode its arm asked for, or the run measured something else."""
    errors = []
    for seq, row in sorted(rows.items()):
        mode = metrics(row)["cache_key_mode"]
        if mode is None and require:
            errors.append("run %d: effective cache-key mode not recorded" % seq)
        elif mode is not None and mode != plan[seq]["arm"]:
            errors.append("run %d: arm %s ran with cache-key mode %s" % (seq, plan[seq]["arm"], mode))
    return errors


def evaluate(rows, profile="full"):
    if profile == "pilot":
        return evaluate_pilot(rows)
    plan = {run["seq"]: run for run in schedule()}
    errors = mode_errors(plan, rows, require=False)
    if errors:
        return {"verdict": "INVALID", "errors": errors}
    measured = {s: metrics(r) for s, r in rows.items() if plan[s]["role"] == "measured"}
    if len(measured) != 16:
        return {"verdict": "INCOMPLETE", "reason": "%d of 16 measured runs present" % len(measured)}
    pairs = {}
    for seq, m in measured.items():
        pairs.setdefault(plan[seq]["pair"], {})[plan[seq]["arm"]] = m
    drops, gains, better = [], [], 0
    for pair in pairs.values():
        a, b = pair["shared"], pair["session"]
        drops.append((b["uncached"] - a["uncached"]) / a["uncached"])
        better += b["uncached"] < a["uncached"]
        if a["ratio_later"] is not None and b["ratio_later"] is not None:
            gains.append(b["ratio_later"] - a["ratio_later"])
    arm_total = lambda arm, key: sum(p[arm][key] for p in pairs.values())
    summary = {
        "median_uncached_change": statistics.median(drops), "pairs_session_lower": better,
        "median_ratio_gain": statistics.median(gains) if gains else None,
        "passes": {arm: sum(p[arm]["pass"] for p in pairs.values()) for arm in ARMS},
        "requests": {arm: arm_total(arm, "requests") for arm in ARMS},
        "uncached": {arm: arm_total(arm, "uncached") for arm in ARMS},
        "wall_s": {arm: round(arm_total(arm, "wall"), 1) for arm in ARMS},
    }
    conditions = [
        summary["median_uncached_change"] <= -MIN_UNCACHED_DROP,
        better >= MIN_PAIRS_BETTER,
        summary["median_ratio_gain"] is not None and summary["median_ratio_gain"] >= MIN_RATIO_GAIN,
        summary["passes"]["session"] >= summary["passes"]["shared"],
        summary["requests"]["session"] <= MAX_REQUEST_GROWTH * summary["requests"]["shared"],
    ]
    if all(conditions):
        verdict = "SUPPORTS_FURTHER_TESTING"
    elif summary["median_uncached_change"] >= 0:
        verdict = "NO_BENEFIT"
    else:
        verdict = "INCONCLUSIVE"
    return {"verdict": verdict, "conditions_met": conditions, **summary,
            "note": "16 runs show direction only; no production default follows from this alone"}


def evaluate_pilot(rows):
    """Four pairs: what abandoning the shared prefix costs on request 1, what a private key gains later."""
    plan = {run["seq"]: run for run in schedule("pilot")}
    errors = mode_errors(plan, rows, require=True)
    if errors:
        return {"verdict": "INVALID", "errors": errors}
    stops = ["run %d: %s" % (s, metrics(r)["stop"]) for s, r in sorted(rows.items()) if metrics(r)["stop"]]
    if stops:
        return {"verdict": "STOPPED", "stops": stops, "note": "a harness stop ends the experiment; no reruns"}
    measured = {s: metrics(r) for s, r in rows.items() if plan[s]["role"] == "measured"}
    if len(measured) != 8:
        return {"verdict": "INCOMPLETE", "reason": "%d of 8 measured runs present" % len(measured)}
    pairs = {}
    for seq, m in measured.items():
        pairs.setdefault(plan[seq]["pair"], {})[plan[seq]["arm"]] = m
    rows_out, first_loss, gains, lower, higher = [], [], [], 0, 0
    for index in sorted(pairs):
        a, b = pairs[index]["shared"], pairs[index]["session"]
        first_loss.append((a["first_cached"] or 0) - (b["first_cached"] or 0))
        gain = None if a["ratio_later"] is None or b["ratio_later"] is None else b["ratio_later"] - a["ratio_later"]
        if gain is not None:
            gains.append(gain)
            higher += gain > 0
        lower += b["uncached"] < a["uncached"]
        rows_out.append({"pair": index,
                         "first_request_cached": {"shared": a["first_cached"], "session": b["first_cached"]},
                         "later_ratio": {"shared": a["ratio_later"], "session": b["ratio_later"]},
                         "uncached": {"shared": a["uncached"], "session": b["uncached"]},
                         "requests": {"shared": a["requests"], "session": b["requests"]},
                         "wall_s": {"shared": a["wall"], "session": b["wall"]},
                         "cache_write": {"shared": a["cache_write"], "session": b["cache_write"]},
                         "pass": {"shared": a["pass"], "session": b["pass"]}})
    total = lambda arm, key: sum(p[arm][key] for p in pairs.values())
    unc = {arm: total(arm, "uncached") for arm in ARMS}
    req = {arm: total(arm, "requests") for arm in ARMS}
    passes = {arm: sum(p[arm]["pass"] for p in pairs.values()) for arm in ARMS}
    median_gain = statistics.median(gains) if gains else None
    net_change = (unc["session"] - unc["shared"]) / unc["shared"]
    promising = all([
        median_gain is not None and median_gain >= PILOT_MIN_RATIO_GAIN,
        higher >= PILOT_MIN_PAIRS, lower >= PILOT_MIN_PAIRS, net_change < 0,
        passes["session"] >= passes["shared"], req["session"] <= MAX_REQUEST_GROWTH * req["shared"]])
    if promising:
        verdict = "PROMISING"
    elif net_change >= 0 or (median_gain is not None and median_gain <= 0):
        verdict = "NOT_PROMISING"
    else:
        verdict = "MIXED"
    return {"verdict": verdict, "pairs": rows_out,
            "median_first_request_cached_lost": statistics.median(first_loss),
            "median_later_ratio_gain": median_gain, "pairs_later_ratio_higher": higher,
            "pairs_uncached_lower": lower, "net_uncached_change": round(net_change, 4),
            "uncached": unc, "requests": req, "passes": passes,
            "note": "4 pairs show direction only. PROMISING means the 20-run study is worth requesting; "
                    "it never supports changing a default"}


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = ap.add_subparsers(dest="mode", required=True)
    sc = sub.add_parser("schedule")
    sc.add_argument("--profile", choices=sorted(PROFILES), default="full")
    c = sub.add_parser("commands")
    c.add_argument("--profile", choices=sorted(PROFILES), default="full")
    c.add_argument("--root", required=True)
    c.add_argument("--settings-dir", required=True)
    c.add_argument("--manifest", required=True)
    r = sub.add_parser("report")
    r.add_argument("root")
    r.add_argument("--profile", choices=sorted(PROFILES), default="full")
    args = ap.parse_args(argv)
    if args.mode == "schedule":
        print(json.dumps(schedule(args.profile), indent=1))
    elif args.mode == "commands":
        print("\n".join(commands(args.root, args.settings_dir, args.manifest, profile=args.profile)))
    else:
        print(json.dumps(evaluate(load_rows(args.root), args.profile), indent=1))


if __name__ == "__main__":
    main()
