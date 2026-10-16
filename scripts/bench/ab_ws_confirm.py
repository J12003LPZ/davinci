"""Fixed schedule, stop rules and pre-registered decision rule for the WebSocket confirmation study.

Prepares and evaluates only. It never launches a model run: `commands` prints the PowerShell
lines an approved operator would run, one single-run campaign per line, and `report` reads
campaign result rows that already exist.

Three arms use one frozen DaVinci binary and one model configuration; only `transport` differs:
  http -> "auto"                           the production default on the public route (control)
  ws   -> "public-websocket"               persistent socket, full input replayed
  wsc  -> "public-websocket-continuation"  same socket, plus previous_response_id with new items only

Design: 4 tasks x 3 repetitions x 3 arms (36 measured runs) after 3 discarded warm-ups, 39 runs at most.
A block is one (task, repetition) with all three arms. The 12 blocks are paired but not independent
observations: repetitions of one task share fixtures and prompt prefixes. Effects are therefore decided at
the task level and reported by task and by repetition; the block sign test is informational only.
"""
import argparse
import hashlib
from itertools import permutations
import json
from math import comb
from pathlib import Path
import statistics
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
import ab_cache_key as cache_ab  # noqa: E402  (shared row metrics)
import ab_transport as transport_ab  # noqa: E402  (arm labels and per-run transport proof)

ARMS = transport_ab.ARMS
CANDIDATES = ("ws", "wsc")
TRANSPORT = transport_ab.TRANSPORT
TASKS = cache_ab.TASKS
WARMUP_TASK = cache_ab.WARMUP_TASK
WARMUP_ARMS = transport_ab.WARMUP_ARMS          # ws first: it is the compatibility probe
REPS = 3
PREFIX = "wc"
# The six arm orders, each used on exactly two of the twelve blocks. Block b = rep * 4 + task gets
# ORDERS[b % 6]: every arm sits in every position four times, and each task sees three different orders.
ORDERS = tuple(permutations(ARMS))
BLOCKS = len(TASKS) * REPS
MEASURED = BLOCKS * len(ARMS)
MAX_RUNS = len(WARMUP_ARMS) + MEASURED
STAGES = 3                                       # one fresh sign-in per stage; a stage is one repetition

# Pins. A launcher refuses to start unless the observed values equal these.
SOURCE_SHA = "5ab199e96c674780db59724f38c8c4e7f05b2ae4"
BINARY_SHA256 = "202f539653f051b39c541b8acae4d5e2403d8a6e4620cf2f77ef56854cac9449"
MANIFEST_SHA256 = "49e83faeb7b3091db5ea2952dbc8d180f5acb1b17c44abfd8080ce91e52309e0"
MODEL = "gpt-6-luna"
EFFORT = "high"
SERVICE_TIER = "default"

# Decision rule, fixed before any run. Every threshold compares a candidate with `http`.
MAX_UNCACHED_RATIO = 0.60     # total uncached input, candidate / http
MIN_BLOCKS_LOWER = 9          # of 12 blocks: consistency check, not a significance claim
MIN_TASKS_LOWER = 3           # of 4 tasks, repetition totals
MAX_WALL_RATIO = 0.90         # total wall time
MAX_TASK_WALL_RATIO = 1.10    # no task's median wall time worse than this
MAX_REQUEST_RATIO = 1.10      # total requests
MIN_RATIO_GAIN = 0.10         # median later-request cache-ratio gain (mechanism check)
MAX_TASK_PASS_DEFICIT = 1     # a task may lose at most one repetition against http
# Continuation is adopted over the simpler `ws` only if it clears all of these against `ws`.
CONT_MAX_UNCACHED_RATIO = 0.80
CONT_MAX_WALL_RATIO = 0.90
CONT_MIN_TASKS_BETTER = 3     # of 4: lower uncached AND lower wall
# Stops.
MAX_UNKNOWN_ATTEMPTS_RUN = transport_ab.MAX_UNKNOWN_ATTEMPTS_RUN
MAX_UNKNOWN_ATTEMPTS_TOTAL = transport_ab.MAX_UNKNOWN_ATTEMPTS_TOTAL
SAFETY_STOP_LAG = 2           # cumulative passes behind http, over completed blocks


def schedule():
    runs = [{"role": "warmup", "arm": arm, "task": WARMUP_TASK, "rep": 90 + i, "task_index": None,
             "block": None, "stage": 1} for i, arm in enumerate(WARMUP_ARMS)]
    for rep in range(REPS):
        for index, task in enumerate(TASKS):
            block = rep * len(TASKS) + index
            for arm in ORDERS[block % len(ORDERS)]:
                runs.append({"role": "measured", "arm": arm, "task": task, "rep": rep, "task_index": index,
                             "block": block, "stage": rep + 1})
    for seq, run in enumerate(runs, 1):
        run["seq"] = seq
    return runs


def run_name(run):
    return "%s-%02d-%s-%s" % (PREFIX, run["seq"], run["role"], run["arm"])


def settings_for(arm, base):
    """Arm settings: the shared base unchanged except `transport`."""
    out = json.loads(json.dumps(base))
    out["transport"] = TRANSPORT[arm]
    return out


def settings_errors(settings_dir, base):
    """The three arm files must exist and differ from the base only in `transport`."""
    errors = []
    for arm in ARMS:
        path = Path(settings_dir) / ("%s.json" % arm)
        try:
            found = json.loads(path.read_text(encoding="utf-8"))
        except (OSError, ValueError):
            errors.append("%s: settings file missing or unreadable" % arm)
            continue
        if found != settings_for(arm, base):
            errors.append("%s: settings differ from the base beyond transport" % arm)
    return errors


def commands(root, settings_dir, manifest, stage=None, bench="scripts/bench/bench.py"):
    lines = []
    for run in schedule():
        if stage is not None and run["stage"] != stage:
            continue
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


def file_sha256(path):
    digest = hashlib.sha256()
    with open(path, "rb") as handle:
        for chunk in iter(lambda: handle.read(1 << 20), b""):
            digest.update(chunk)
    return digest.hexdigest()


def pin_errors(binary=None, manifest=None, harness_sha=None, expected_harness_sha=None):
    """Observed identities against the registered pins. Anything not supplied is an error, not a pass."""
    errors = []
    for label, path, pin in (("binary", binary, BINARY_SHA256), ("manifest", manifest, MANIFEST_SHA256)):
        try:
            observed = file_sha256(path)
        except (OSError, TypeError):
            errors.append("%s: file unreadable" % label)
            continue
        if observed != pin:
            errors.append("%s hash %s is not the registered %s" % (label, observed[:12], pin[:12]))
    if not expected_harness_sha or harness_sha != expected_harness_sha:
        errors.append("harness commit %s is not the frozen %s" % (harness_sha, expected_harness_sha))
    return errors


def identity_errors(plan, rows):
    """Every row must prove the frozen binary, source, model configuration and the unchanged task fixtures."""
    errors, fixtures, base_settings = [], {}, None
    for seq, row in sorted(rows.items()):
        for field, pin in (("binary_sha256", BINARY_SHA256), ("source_sha", SOURCE_SHA), ("model", MODEL),
                           ("effort_policy", EFFORT), ("service_tier", SERVICE_TIER)):
            if row.get(field) is None:
                errors.append("run %d: %s not recorded" % (seq, field))
            elif row[field] != pin:
                errors.append("run %d: %s %r is not the pinned value" % (seq, field, str(row[field])[:12]))
        fixture = row.get("fixture_hash")
        if fixture is None:
            errors.append("run %d: fixture hash not recorded" % seq)
        elif fixtures.setdefault(plan[seq]["task"], fixture) != fixture:
            errors.append("run %d: fixture of %s changed between runs" % (seq, plan[seq]["task"]))
        settings = row.get("effective_settings")
        if isinstance(settings, dict):
            rest = {k: v for k, v in settings.items() if k != "transport"}
            if base_settings is None:
                base_settings = rest
            elif rest != base_settings:
                errors.append("run %d: settings differ from the other runs beyond transport" % seq)
    return errors


def validity_errors(plan, rows):
    return transport_ab.arm_errors(plan, rows) + identity_errors(plan, rows)


def block_passes(plan, rows):
    """Verified passes per arm and per (arm, task), over blocks where all three arms have a row."""
    blocks = {}
    for seq, row in rows.items():
        run = plan[seq]
        if run["role"] == "measured":
            blocks.setdefault(run["block"], {})[run["arm"]] = (run["task_index"], bool(row.get("pass")))
    totals = {arm: 0 for arm in ARMS}
    per_task = {arm: {i: 0 for i in range(len(TASKS))} for arm in ARMS}
    complete = 0
    for entry in blocks.values():
        if len(entry) == len(ARMS):
            complete += 1
            for arm, (task, ok) in entry.items():
                totals[arm] += ok
                per_task[arm][task] += ok
    return {"complete_blocks": complete, "passes": totals, "per_task": per_task}


def stop_reason(run, row, unknown_so_far=0, earlier=None, plan=None):
    """Predefined stop conditions for one finished run; None means continue. A stop ends the whole study.

    `earlier` maps seq to the rows of the runs already finished, for the cumulative safety stop. Identity
    fields are checked with the same rule `evaluate` uses.
    """
    base = transport_ab.stop_reason(run, row, unknown_so_far)
    if base:
        return base
    plan = plan or {r["seq"]: r for r in schedule()}
    identity = identity_errors(plan, {run["seq"]: row})
    if identity:
        return identity[0]
    unknown = row.get("usage_unknown_attempts")
    if not row.get("pass") and type(unknown) is int and unknown > 0:
        return "run %d: failed verification with %d provider attempts without usage (cause not separable)" % (
            run["seq"], unknown)
    if row.get("output_tokens") is None and run["role"] == "measured":
        return "run %d: output tokens not recorded" % run["seq"]
    seen = dict(earlier or {})
    seen[run["seq"]] = row
    tally = block_passes(plan, seen)
    for arm in CANDIDATES:
        if tally["passes"]["http"] - tally["passes"][arm] >= SAFETY_STOP_LAG:
            return "safety: %s is %d verified passes behind http over %d complete blocks" % (
                arm, tally["passes"]["http"] - tally["passes"][arm], tally["complete_blocks"])
    return None


def metric(row):
    """Row metrics with output tokens. A missing output count makes the run unscorable, never zero."""
    m = cache_ab.metrics(row)
    m["output"] = row.get("output_tokens") if m["usage_known"] else None
    if m["usage_known"] and m["output"] is None:
        m["usage_known"], m["uncached_lower_bound"], m["uncached"] = False, m["uncached"], None
    return m


def sign_p(wins, n):
    """Exact one-sided binomial tail under a fair coin. Informational only: blocks are not independent."""
    return sum(comb(n, k) for k in range(wins, n + 1)) / 2 ** n if n else None


def _sum(blocks, arm, key, tasks=None):
    return sum(b[arm][key] for b in blocks if tasks is None or b["task_index"] in tasks)


def analyze(plan, measured, block_ids):
    """Paired numbers over the given complete blocks. No pass or fail here: `decide` applies the rule."""
    blocks = []
    for block in sorted(block_ids):
        entry = {"block": block, "task_index": None, "rep": None}
        for seq, m in measured.items():
            if plan[seq]["block"] == block:
                entry[plan[seq]["arm"]] = m
                entry["task_index"], entry["rep"] = plan[seq]["task_index"], plan[seq]["rep"]
        blocks.append(entry)
    out = {"blocks_analyzed": len(blocks), "per_arm": {}, "vs_http": {}}
    for arm in ARMS:
        out["per_arm"][arm] = {key: _sum(blocks, arm, key) for key in ("uncached", "requests", "output")}
        out["per_arm"][arm]["wall_s"] = round(_sum(blocks, arm, "wall"), 1)
        out["per_arm"][arm]["passes"] = sum(b[arm]["pass"] for b in blocks)

    def pair(arm, base):
        tasks = sorted({b["task_index"] for b in blocks})
        per_task, per_rep = [], []
        for index in tasks:
            sub = [b for b in blocks if b["task_index"] == index]
            per_task.append({
                "task": TASKS[index], "reps": len(sub),
                "uncached": [_sum(sub, base, "uncached"), _sum(sub, arm, "uncached")],
                "wall_s": [round(_sum(sub, base, "wall"), 1), round(_sum(sub, arm, "wall"), 1)],
                "requests": [_sum(sub, base, "requests"), _sum(sub, arm, "requests")],
                "passes": [sum(b[base]["pass"] for b in sub), sum(b[arm]["pass"] for b in sub)],
                "median_wall_ratio": statistics.median(b[arm]["wall"] for b in sub) / statistics.median(
                    b[base]["wall"] for b in sub)})
        for rep in sorted({b["rep"] for b in blocks}):
            sub = [b for b in blocks if b["rep"] == rep]
            per_rep.append({"rep": rep, "uncached": [_sum(sub, base, "uncached"), _sum(sub, arm, "uncached")],
                            "wall_s": [round(_sum(sub, base, "wall"), 1), round(_sum(sub, arm, "wall"), 1)],
                            "passes": [sum(b[base]["pass"] for b in sub), sum(b[arm]["pass"] for b in sub)]})
        gains = [b[arm]["ratio_later"] - b[base]["ratio_later"] for b in blocks
                 if b[arm]["ratio_later"] is not None and b[base]["ratio_later"] is not None]
        lower = sum(b[arm]["uncached"] < b[base]["uncached"] for b in blocks)
        ties = sum(b[arm]["uncached"] == b[base]["uncached"] for b in blocks)
        ratio = lambda key: (_sum(blocks, arm, key) / _sum(blocks, base, key)) if _sum(blocks, base, key) else None
        tokens = lambda who: _sum(blocks, who, "uncached") + _sum(blocks, who, "output")
        return {
            "uncached_ratio": ratio("uncached"), "wall_ratio": ratio("wall"), "request_ratio": ratio("requests"),
            "total_tokens_ratio": tokens(arm) / tokens(base) if tokens(base) else None,
            "blocks_uncached_lower": lower,
            "tasks_uncached_lower": sum(t["uncached"][1] < t["uncached"][0] for t in per_task),
            "tasks_wall_lower": sum(t["wall_s"][1] < t["wall_s"][0] for t in per_task),
            "tasks_better_on_both": sum(t["uncached"][1] < t["uncached"][0] and t["wall_s"][1] < t["wall_s"][0]
                                        for t in per_task),
            "worst_task_median_wall_ratio": max(t["median_wall_ratio"] for t in per_task),
            "median_later_ratio_gain": statistics.median(gains) if gains else None,
            "median_first_request_cached_lost": statistics.median(
                (b[base]["first_cached"] or 0) - (b[arm]["first_cached"] or 0) for b in blocks),
            "passes": [sum(b[base]["pass"] for b in blocks), sum(b[arm]["pass"] for b in blocks)],
            "worst_task_pass_deficit": max(t["passes"][0] - t["passes"][1] for t in per_task),
            "nominal_block_sign_p": sign_p(lower, len(blocks) - ties),
            "nominal_block_sign_p_note": "blocks share fixtures within a task and are not independent; "
                                         "this is not evidence of a generalizable effect",
            "per_task": per_task, "per_rep": per_rep}

    for arm in CANDIDATES:
        out["vs_http"][arm] = pair(arm, "http")
    out["wsc_vs_ws"] = pair("wsc", "ws")
    return out


def _ok(value, op, limit):
    return value is not None and (value <= limit if op == "<=" else value >= limit)


def decide(analysis):
    """Apply the registered rule to a complete analysis. Returns per-arm criteria and the study verdict."""
    arms = {}
    for arm in CANDIDATES:
        a = analysis["vs_http"][arm]
        checks = {
            "safety_passes_not_below_http": a["passes"][1] >= a["passes"][0],
            "safety_no_task_loses_more_than_one": a["worst_task_pass_deficit"] <= MAX_TASK_PASS_DEFICIT,
            "uncached_total": _ok(a["uncached_ratio"], "<=", MAX_UNCACHED_RATIO),
            "uncached_blocks_lower": a["blocks_uncached_lower"] >= MIN_BLOCKS_LOWER,
            "uncached_tasks_lower": a["tasks_uncached_lower"] >= MIN_TASKS_LOWER,
            "wall_total": _ok(a["wall_ratio"], "<=", MAX_WALL_RATIO),
            "wall_worst_task": _ok(a["worst_task_median_wall_ratio"], "<=", MAX_TASK_WALL_RATIO),
            "requests": _ok(a["request_ratio"], "<=", MAX_REQUEST_RATIO),
            "later_ratio_gain": _ok(a["median_later_ratio_gain"], ">=", MIN_RATIO_GAIN),
            "total_tokens_lower": a["total_tokens_ratio"] is not None and a["total_tokens_ratio"] < 1.0}
        arms[arm] = {"verdict": "PASS" if all(checks.values()) else "FAIL", "checks": checks,
                     "failed": sorted(k for k, v in checks.items() if not v)}
    c = analysis["wsc_vs_ws"]
    continuation = {
        "uncached_vs_ws": _ok(c["uncached_ratio"], "<=", CONT_MAX_UNCACHED_RATIO),
        "wall_vs_ws": _ok(c["wall_ratio"], "<=", CONT_MAX_WALL_RATIO),
        "tasks_better_on_both": c["tasks_better_on_both"] >= CONT_MIN_TASKS_BETTER,
        "passes_not_below_ws": c["passes"][1] >= c["passes"][0]}
    continuation_earns_its_keep = all(continuation.values())
    ws_ok, wsc_ok = arms["ws"]["verdict"] == "PASS", arms["wsc"]["verdict"] == "PASS"
    if wsc_ok and (continuation_earns_its_keep or not ws_ok):
        verdict = "GO_WSC"
    elif ws_ok:
        verdict = "GO_WS"
    else:
        verdict = "NO_GO"
    return {"verdict": verdict, "arms": arms,
            "continuation_vs_ws": {"checks": continuation, "earns_its_keep": continuation_earns_its_keep}}


NOTE_COMPLETE = ("Complete result for four public task families on one model. Repetitions are not independent "
                 "samples of the world: a GO supports production hardening and independent holdout validation, "
                 "never a default change, a superiority claim or a generalization claim.")
NOTE_PARTIAL = ("Partial directional evidence only. No verdict can be GO or NO_GO from an incomplete study. "
                "Unknown usage is never converted to zero and no block was dropped to improve the result.")


def evaluate(rows):
    plan = {run["seq"]: run for run in schedule()}
    unknown_seq = sorted(seq for seq in rows if seq not in plan)
    if unknown_seq:
        return {"verdict": "INVALID", "errors": ["row for run %s is outside the schedule" % unknown_seq]}
    errors = validity_errors(plan, rows)
    if errors:
        return {"verdict": "INVALID", "errors": errors}
    stops = ["run %d: %s" % (s, r.get("stop_reason")) for s, r in sorted(rows.items()) if r.get("stop_reason")]
    if stops:
        return {"verdict": "STOPPED", "stops": stops, "note": "a harness stop ends the study; no reruns"}
    measured = {s: metric(r) for s, r in rows.items() if plan[s]["role"] == "measured"}
    missing_runs = sorted(s for s, run in plan.items() if run["role"] == "measured" and s not in measured)
    unscorable = cache_ab.unscorable(measured)
    safety = block_passes(plan, rows)
    if missing_runs or unscorable:
        complete_ids = [b for b in range(BLOCKS) if all(
            s in measured and measured[s]["usage_known"] for s, r in plan.items() if r["block"] == b)]
        excluded = sorted(set(plan[s]["block"] for s in list(unscorable) + missing_runs))
        result = {"verdict": "INCOMPLETE", "evidence": "partial_directional",
                  "measured_runs_present": len(measured), "of": MEASURED, "missing_runs": missing_runs,
                  "unscorable_runs": unscorable, "blocks_excluded_from_directional": excluded,
                  "safety_observed": {"complete_blocks": safety["complete_blocks"], "passes": safety["passes"]},
                  "note": NOTE_PARTIAL}
        if complete_ids:
            result["directional"] = analyze(plan, measured, complete_ids)
        return result
    analysis = analyze(plan, measured, range(BLOCKS))
    return {"evidence": "complete", **decide(analysis), "analysis": analysis, "note": NOTE_COMPLETE}


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    sub = ap.add_subparsers(dest="mode", required=True)
    sub.add_parser("schedule")
    c = sub.add_parser("commands")
    c.add_argument("--root", required=True)
    c.add_argument("--settings-dir", required=True)
    c.add_argument("--manifest", required=True)
    c.add_argument("--stage", type=int, choices=range(1, STAGES + 1))
    r = sub.add_parser("report")
    r.add_argument("--root", required=True)
    p = sub.add_parser("verify-pins")
    p.add_argument("--binary", required=True)
    p.add_argument("--manifest", required=True)
    p.add_argument("--harness-sha", required=True, help="git rev-parse HEAD of the harness checkout")
    p.add_argument("--expected-harness-sha", required=True, help="the frozen harness commit")
    s = sub.add_parser("verify-settings")
    s.add_argument("--settings-dir", required=True)
    args = ap.parse_args(argv)
    if args.mode == "schedule":
        for run in schedule():
            print("%2d stage %d %-8s %-4s rep %2d %s" % (run["seq"], run["stage"], run["role"], run["arm"],
                                                       run["rep"], run["task"]))
    elif args.mode == "commands":
        print("\n".join(commands(args.root, args.settings_dir, args.manifest, args.stage)))
    elif args.mode == "verify-pins":
        errors = pin_errors(args.binary, args.manifest, args.harness_sha, args.expected_harness_sha)
        print(json.dumps({"ok": not errors, "errors": errors}, indent=2))
        sys.exit(1 if errors else 0)
    elif args.mode == "verify-settings":
        import runner
        errors = settings_errors(args.settings_dir, runner.BASE_SETTINGS)
        print(json.dumps({"ok": not errors, "errors": errors}, indent=2))
        sys.exit(1 if errors else 0)
    else:
        print(json.dumps(evaluate(load_rows(args.root)), indent=2))


if __name__ == "__main__":
    main()
