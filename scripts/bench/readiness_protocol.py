"""Offline readiness decisions over frozen campaign rows; never calls a model."""
import argparse
import json
from pathlib import Path
import random
import re
import statistics

from campaign import metric, percentile, schedule, task_success
from readiness_metrics import number, report, summarize


def prepare(tasks, arms, seed=0):
    if not tasks or any(not isinstance(value, str) or not value for value in tasks + arms):
        raise ValueError("nonempty task and arm names required")
    return {"schema_version": 1, "repetitions": [0, 1, 2], "seed": seed,
            "schedule": schedule(tasks, [0, 1, 2], arms, seed),
            "model_calls_performed": False,
            "acceptance": "paired private dev; independent holdout confirmation required"}


def validate_pairs(baseline, candidate):
    if not baseline or not candidate or any(not isinstance(row, dict) for row in baseline + candidate):
        raise ValueError("nonempty campaign objects required")
    indexed = []
    for rows in (baseline, candidate):
        index = {}
        identities = set()
        for row in rows:
            key = row.get("task"), row.get("rep")
            if (not isinstance(key[0], str) or not key[0] or type(key[1]) is not int
                    or key[1] not in (0, 1, 2) or key in index):
                raise ValueError("invalid or duplicate task/repetition")
            for field, length in (("source_sha", 40), ("binary_sha256", 64), ("private_suite_digest", 64)):
                if not isinstance(row.get(field), str) or not re.fullmatch("[0-9a-f]{%d}" % length, row[field]):
                    raise ValueError("missing frozen " + field)
            if row.get("source_clean") is not True or row.get("size_class") not in ("small", "large"):
                raise ValueError("clean source and private size labels required")
            if not row.get("model") or not row.get("effort_policy") or row.get("split") not in ("dev", "holdout"):
                raise ValueError("model, effort and split required")
            identities.add(tuple(row.get(field) for field in
                                 ("source_sha", "binary_sha256", "model", "effort_policy", "private_suite_digest", "split")))
            index[key] = row
        if len(identities) != 1 or any({rep for task_id, rep in index if task_id == task} != {0, 1, 2}
                                       for task in {key[0] for key in index}):
            raise ValueError("campaign must freeze one executable and contain three repetitions per task")
        indexed.append(index)
    left, right = indexed
    if left.keys() != right.keys():
        raise ValueError("paired campaigns require identical membership")
    for key in left:
        for field in ("model", "effort_policy", "service_tier", "private_suite_digest", "split",
                      "size_class", "repository_id", "reference_commit", "visible_tests", "requires_existing_test_changes"):
            if left[key].get(field) != right[key].get(field):
                raise ValueError("paired campaign differs in " + field)
    return left, right


def success_interval(baseline, candidate, samples=2000, seed=0):
    left, right = validate_pairs(baseline, candidate)
    tasks = sorted({task for task, _ in left})
    differences = {task: statistics.mean(int(task_success(right[task, rep])) - int(task_success(left[task, rep]))
                                         for rep in (0, 1, 2)) for task in tasks}
    rng = random.Random(seed)
    values = [statistics.mean(differences[task] for task in rng.choices(tasks, k=len(tasks)))
              for _ in range(samples)]
    bounds = [percentile(values, .025), percentile(values, .975)]
    null = bounds[0] <= 0 <= bounds[1]
    return {"method": "paired task-cluster percentile bootstrap", "clusters": len(tasks),
            "repetitions_per_cluster": 3, "resamples": samples, "seed": seed, "ci95": bounds,
            "difference": statistics.mean(differences.values()), "includes_no_change": null,
            "conclusion": "no measurable difference" if null else "candidate improves" if bounds[0] > 0 else "candidate regresses"}


def promotion(baseline, candidate, holdout_baseline, holdout_candidate, *, target, prices=None, token_ceiling=.25):
    validate_pairs(baseline, candidate)
    if target not in ("composite_success_rate", "median_wall_s", "uncached_input_per_verified_success", "estimated_usd_per_verified_success"):
        raise ValueError("unsupported target metric")
    if not number(token_ceiling):
        raise ValueError("nonnegative agreed token ceiling required")
    rules = {}
    def rule(name, passed, evidence):
        rules[name] = {"status": "unavailable" if passed is None else "pass" if passed else "fail", "evidence": evidence}
    tasks = {row["task"] for row in baseline}
    repos = {row.get("repository_id") for row in baseline} - {None}
    rule("private_dev_sizes", 40 <= len(tasks) <= 60 and 3 <= len(repos) <= 5 and all(row["split"] == "dev" for row in baseline),
         {"tasks": len(tasks), "repositories": len(repos)})
    rule("independent_grading", all(row.get("grading_assurance") == "independent" for row in baseline + candidate),
         "native lifecycle/container visibility alone cannot certify independent grading")
    before, after = summarize(baseline, prices), summarize(candidate, prices)
    classes = {}
    for size in ("small", "large"):
        a, b = ([row for row in rows if row["size_class"] == size] for rows in (baseline, candidate))
        classes[size] = None if not a else {"baseline": summarize(a)["composite_success_rate"], "candidate": summarize(b)["composite_success_rate"]}
    rule("no_size_class_success_regression", all(value is not None and value["candidate"] >= value["baseline"] for value in classes.values()), classes)
    a, b = before[target], after[target]
    rule("target_improves", None if a is None or b is None else b > a if target == "composite_success_rate" else b < a,
         {"metric": target, "baseline": a, "candidate": b})
    intervals = success_interval(baseline, candidate)
    rule("success_uncertainty", intervals["ci95"][0] > 0 if target == "composite_success_rate" else intervals["ci95"][0] >= 0, intervals)
    values = [[metric(row, "uncached_input_tokens") for row in rows] for rows in (baseline, candidate)]
    medians = [statistics.median(value) if all(number(v) for v in value) else None for value in values]
    rule("uncached_token_ceiling", None if None in medians else medians[1] <= medians[0] * (1 + token_ceiling),
         {"baseline_median": medians[0], "candidate_median": medians[1], "ceiling_fraction": token_ceiling})
    a, b = before["unrelated_edit_rate"], after["unrelated_edit_rate"]
    rule("unrelated_edits_nonincrease", None if a is None or b is None else b <= a, {"baseline": a, "candidate": b})
    a, b = before["median_wall_s"], after["median_wall_s"]
    rule("wall_time_ceiling", None if a is None or b is None else b <= a * 1.15, {"baseline": a, "candidate": b, "ceiling_fraction": .15})
    confirmation = None
    if holdout_baseline is not None and holdout_candidate is not None:
        validate_pairs(holdout_baseline, holdout_candidate)
        holdout_tasks = {row["task"] for row in holdout_baseline}
        if tasks & holdout_tasks:
            raise ValueError("dev and holdout task identities overlap")
        dev_revisions = {(row.get("repository_id"), row.get("reference_commit")) for row in baseline}
        holdout_revisions = {(row.get("repository_id"), row.get("reference_commit")) for row in holdout_baseline}
        if (dev_revisions & holdout_revisions) - {(None, None)}:
            raise ValueError("dev and holdout source revisions overlap")
        for dev, held in ((baseline, holdout_baseline), (candidate, holdout_candidate)):
            for field in ("source_sha", "binary_sha256", "model", "effort_policy", "private_suite_digest"):
                if dev[0][field] != held[0][field]:
                    raise ValueError("holdout differs from frozen dev arm: " + field)
        hold_before, hold_after = summarize(holdout_baseline, prices), summarize(holdout_candidate, prices)
        confirmation = success_interval(holdout_baseline, holdout_candidate)
        a, b = hold_before[target], hold_after[target]
        agrees = a is not None and b is not None and (b > a if target == "composite_success_rate" else b < a)
        classes_ok = all(any(row["size_class"] == size for row in holdout_baseline)
                         and sum(task_success(row) for row in holdout_candidate if row["size_class"] == size)
                         >= sum(task_success(row) for row in holdout_baseline if row["size_class"] == size) for size in ("small", "large"))
        hold_unrelated = (hold_before["unrelated_edit_rate"], hold_after["unrelated_edit_rate"])
        hold_wall = (hold_before["median_wall_s"], hold_after["median_wall_s"])
        valid = (len(holdout_tasks) >= 150 and all(row["split"] == "holdout" for row in holdout_baseline)
                 and all(row.get("grading_assurance") == "independent" for row in holdout_baseline + holdout_candidate)
                 and agrees and classes_ok and None not in hold_unrelated and hold_unrelated[1] <= hold_unrelated[0]
                 and None not in hold_wall and hold_wall[1] <= hold_wall[0] * 1.15
                 and (confirmation["ci95"][0] > 0 if target == "composite_success_rate" else confirmation["ci95"][0] >= 0))
        rule("holdout_confirmation", valid, {"tasks": len(holdout_tasks), "interval": confirmation, "target_agrees": agrees})
    else:
        rule("holdout_confirmation", None, "at least 150 untouched tasks and matching frozen arms required")
    rule("estimated_cost_reported", before["estimated_usd_per_verified_success"] is not None and after["estimated_usd_per_verified_success"] is not None,
         "pin a dated USD price table; estimates are not measured charges")
    return {"accepted": all(value["status"] == "pass" for value in rules.values()), "rules": rules,
            "baseline": report(baseline, prices), "candidate": report(candidate, prices),
            "holdout_interval": confirmation, "model_calls_performed": False}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", required=True)
    parser.add_argument("--candidate", required=True)
    parser.add_argument("--holdout-baseline")
    parser.add_argument("--holdout-candidate")
    parser.add_argument("--target", default="composite_success_rate")
    parser.add_argument("--price-table")
    parser.add_argument("--token-ceiling", type=float, default=.25)
    args = parser.parse_args()
    def read(path):
        return [json.loads(line) for line in Path(path).read_text().splitlines() if line.strip()] if path else None
    from readiness_metrics import load_prices
    prices = load_prices(args.price_table)[0] if args.price_table else None
    result = promotion(read(args.baseline), read(args.candidate), read(args.holdout_baseline), read(args.holdout_candidate),
                       target=args.target, prices=prices, token_ceiling=args.token_ceiling)
    print(json.dumps(result, indent=2))
    return 0 if result["accepted"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
