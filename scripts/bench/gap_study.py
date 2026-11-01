"""Stage 0 gap-study analysis (preregistration sections 3-6). Pure stdlib, offline.

Reads ``results.jsonl`` from a runs directory (never ``warmup.jsonl``) and prints,
for each of three arm pairs and five metrics, the geometric mean of the per-task
ratio with a percentile bootstrap interval over tasks, and its GAP / AHEAD /
NO_CLEAR_GAP label, plus the pass guard.

    python gap_study.py RUNS_DIR --seed N [--resamples 10000] [--confidence 0.90]
                        [--causes causes.json] [--droid-reasoning-excluded] [--json]

``--seed`` is required: it is the bootstrap seed written into the approval block.

Reading choices where the preregistration is silent are listed in its section 4
("Clarifications"); this file follows that list.
"""
import argparse
import json
import math
import random
import sys
from pathlib import Path

from campaign import metric

ARMS = ("davinci", "droid-direct", "droid-script")
# (numerator arm, denominator arm); ratio = numerator / denominator.
PAIRS = (("droid-direct", "davinci"), ("droid-script", "davinci"), ("droid-script", "droid-direct"))
METRICS = ("requests", "input_uncached", "input_total", "output_plus_reasoning", "wall_ms")
GAP_AT_MOST = 0.85
AHEAD_AT_LEAST = 1.15
DEFAULT_RESAMPLES = 10_000
DEFAULT_CONFIDENCE = 0.90
SUCCESS_GAP_MIN_TASKS = 3


def _positive(value):
    return (type(value) in (int, float) and math.isfinite(value) and value > 0)


def arm_of(row):
    """Arm name of a row: the harness, or the recorded mode for a plain ``droid`` row."""
    harness = row.get("harness")
    if harness == "droid":
        mode = row.get("droid_tool_execution_mode")
        return {"direct-only": "droid-direct", "script-only": "droid-script"}.get(mode)
    return harness if harness in ARMS else None


def is_incomplete(row):
    """A row without complete usage or a known request count is not scored."""
    return row.get("usage_available") is not True or row.get("logical_requests") is None


def metric_value(row, name, droid_reasoning_excluded=False):
    """One primary metric of a row, or None when missing or invalid."""
    if name == "requests":
        value = row.get("logical_requests")
    elif name == "input_uncached":
        value = metric(row, "uncached_input_tokens")
    elif name == "input_total":
        value = row.get("input_tokens")
    elif name == "wall_ms":
        wall = row.get("wall_s")
        value = round(wall * 1000) if type(wall) in (int, float) and math.isfinite(wall) else None
    elif name == "output_plus_reasoning":
        output, reasoning = row.get("output_tokens"), row.get("reasoning_tokens")
        is_droid = arm_of(row) in ("droid-direct", "droid-script")
        included = row.get("reasoning_included_in_output") is True and not (
            is_droid and droid_reasoning_excluded)
        if included:
            value = output
        else:
            value = (output + reasoning if type(output) is int and type(reasoning) is int else None)
    else:
        raise ValueError("unknown metric " + name)
    return value if _positive(value) else None


def geometric_mean(ratios):
    return math.exp(sum(math.log(value) for value in ratios) / len(ratios))


def bootstrap_interval(ratios, seed, resamples=DEFAULT_RESAMPLES, confidence=DEFAULT_CONFIDENCE):
    """Percentile bootstrap of the geometric mean, resampling tasks with replacement."""
    rng = random.Random(seed)
    n = len(ratios)
    means = sorted(geometric_mean([ratios[rng.randrange(n)] for _ in range(n)])
                   for _ in range(resamples))
    tail = (1 - confidence) / 2
    low = means[math.floor(tail * resamples)]
    high = means[min(resamples - 1, math.ceil((1 - tail) * resamples) - 1)]
    return low, high


def label(low, high):
    """The preregistered gap rule on a (lower, upper) interval."""
    if high <= GAP_AT_MOST:
        return "GAP"
    if low >= AHEAD_AT_LEAST:
        return "AHEAD"
    return "NO_CLEAR_GAP"


def load_study_rows(runs_dir):
    """Rows of ``results.jsonl`` only; ``warmup.jsonl`` and unscored rows are never read."""
    path = Path(runs_dir) / "results.jsonl"
    rows = [json.loads(line) for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]
    return [row for row in rows if row.get("scored", True) is not False]


def index_rows(rows):
    """(arm, task) -> row, rejecting repeated measurements: the study is one repetition."""
    index = {}
    for row in rows:
        arm = arm_of(row)
        if arm is None:
            continue
        key = (arm, row.get("task"))
        if key in index:
            raise ValueError(f"more than one row for {key}: the study is one repetition")
        index[key] = row
    return index


def estimate(index, numerator, denominator, name, seed, resamples, confidence, droid_reasoning_excluded):
    """One pair x metric estimate with every exclusion counted by reason."""
    tasks = sorted({task for (arm, task) in index if arm in (numerator, denominator)})
    ratios, excluded = [], {"missing_arm": 0, "not_both_pass": 0, "incomplete": 0,
                            "mode_mismatch": 0, "missing_or_zero_metric": 0}
    for task in tasks:
        left, right = index.get((numerator, task)), index.get((denominator, task))
        if left is None or right is None:
            excluded["missing_arm"] += 1
        elif not (left.get("pass") is True and right.get("pass") is True):
            excluded["not_both_pass"] += 1
        elif left.get("droid_mode_mismatch") is True or right.get("droid_mode_mismatch") is True:
            excluded["mode_mismatch"] += 1
        elif is_incomplete(left) or is_incomplete(right):
            excluded["incomplete"] += 1
        else:
            a = metric_value(left, name, droid_reasoning_excluded)
            b = metric_value(right, name, droid_reasoning_excluded)
            if a is None or b is None:
                excluded["missing_or_zero_metric"] += 1
            else:
                ratios.append(a / b)
    result = {"pair": f"{numerator} vs {denominator}", "metric": name, "n": len(ratios),
              "excluded": excluded, "geometric_mean": None, "interval": None,
              "label": "NO_CLEAR_GAP"}
    if ratios:
        low, high = bootstrap_interval(ratios, seed, resamples, confidence)
        result.update(geometric_mean=geometric_mean(ratios), interval=[low, high], label=label(low, high))
    return result


def pass_guard(index, causes=None):
    """Pass counts, discordant tasks per pair, and the three-task same-cause rule."""
    counts = {arm: {"passes": sum(1 for (a, _), row in index.items() if a == arm and row.get("pass") is True),
                    "runs": sum(1 for (a, _) in index if a == arm)} for arm in ARMS}
    discordant = []
    for numerator, denominator in PAIRS:
        for task in sorted({task for (arm, task) in index if arm in (numerator, denominator)}):
            left, right = index.get((numerator, task)), index.get((denominator, task))
            if left is None or right is None or (left.get("pass") is True) == (right.get("pass") is True):
                continue
            passer, failer = (numerator, denominator) if left.get("pass") is True else (denominator, numerator)
            failed_row = index[(failer, task)]
            discordant.append({
                "pair": f"{numerator} vs {denominator}", "task": task, "passes": passer, "fails": failer,
                "stop_reason": failed_row.get("stop_reason"),
                "transcript": f"{failed_row.get('harness')}/{task}-r{failed_row.get('rep')}.stdout.jsonl",
                "cause": (causes or {}).get(f"{task}|{failer}")})
    # Success gap: tasks a Droid arm passes and DaVinci fails, grouped by the cause
    # a human read from the transcript. Without a causes file nothing can qualify.
    by_cause = {}
    for item in discordant:
        if item["fails"] == "davinci" and item["passes"] != "davinci" and item["cause"]:
            by_cause.setdefault(item["cause"], set()).add(item["task"])
    shared = {cause: sorted(tasks) for cause, tasks in by_cause.items() if len(tasks) >= SUCCESS_GAP_MIN_TASKS}
    return {"pass_counts": counts, "discordant": discordant,
            "success_gap_causes": shared, "success_gap": bool(shared),
            "note": "qualitative, not a significance test"}


def analyse(rows, seed, resamples=DEFAULT_RESAMPLES, confidence=DEFAULT_CONFIDENCE,
            causes=None, droid_reasoning_excluded=False):
    index = index_rows(rows)
    estimates = [estimate(index, n, d, m, seed, resamples, confidence, droid_reasoning_excluded)
                 for (n, d) in PAIRS for m in METRICS]
    incomplete = sorted(f"{arm}|{task}" for (arm, task), row in index.items() if is_incomplete(row))
    return {"seed": seed, "resamples": resamples, "confidence": confidence,
            "estimates": estimates, "incomplete_rows": incomplete,
            "pass_guard": pass_guard(index, causes)}


def format_table(report):
    lines = [f"{'pair':32} {'metric':22} {'n':>3} {'geo-mean':>9} {'interval':>19}  label"]
    for item in report["estimates"]:
        if item["interval"] is None:
            mean = interval = "-"
        else:
            mean = f"{item['geometric_mean']:.3f}"
            interval = f"[{item['interval'][0]:.3f}, {item['interval'][1]:.3f}]"
        lines.append(f"{item['pair']:32} {item['metric']:22} {item['n']:>3} {mean:>9} {interval:>19}  {item['label']}")
    guard = report["pass_guard"]
    lines.append("")
    lines.append("pass counts: " + ", ".join(f"{arm} {v['passes']}/{v['runs']}" for arm, v in guard["pass_counts"].items()))
    for item in guard["discordant"]:
        lines.append(f"discordant {item['pair']}: {item['task']} passes={item['passes']} fails={item['fails']} "
                     f"stop_reason={item['stop_reason']} cause={item['cause'] or '(read transcript)'}")
    lines.append(f"success gap (>= {SUCCESS_GAP_MIN_TASKS} same-cause tasks, qualitative): {guard['success_gap']}")
    if report["incomplete_rows"]:
        lines.append("INCOMPLETE rows (not in ratios): " + ", ".join(report["incomplete_rows"]))
    return "\n".join(lines)


def main(argv=None):
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("runs_dir")
    ap.add_argument("--seed", type=int, required=True, help="bootstrap seed from the approval block")
    ap.add_argument("--resamples", type=int, default=DEFAULT_RESAMPLES)
    ap.add_argument("--confidence", type=float, default=DEFAULT_CONFIDENCE)
    ap.add_argument("--causes", help='JSON {"<task>|<failing arm>": "<cause read from the transcript>"}')
    ap.add_argument("--droid-reasoning-excluded", action="store_true",
                    help="Droid outputTokens excludes reasoning (smoke-row check G3): add reasoning_tokens")
    ap.add_argument("--json", action="store_true")
    args = ap.parse_args(argv)
    causes = json.loads(Path(args.causes).read_text(encoding="utf-8")) if args.causes else None
    report = analyse(load_study_rows(args.runs_dir), args.seed, args.resamples, args.confidence,
                     causes, args.droid_reasoning_excluded)
    print(json.dumps(report, indent=2) if args.json else format_table(report))
    return 0


if __name__ == "__main__":
    sys.exit(main())
