"""Deterministic campaign identity and integrity; never starts a model."""
import hashlib
import json
import math
import statistics
import random
from datetime import datetime
from pathlib import Path


LEGACY_TASKS = (
    "t1-intervals", "t2-duration", "t3-lru", "t4-rename",
    "t5-csv", "t6-bookings", "t7-cli", "t8-calc",
)
COMPARABLE = ("fixture_hash", "model", "effort_policy", "service_tier")
GRADING_ISOLATIONS = ("diagnostic-only", "container")


def digest(value):
    return hashlib.sha256(json.dumps(
        value, sort_keys=True, separators=(",", ":"), ensure_ascii=True
    ).encode("utf-8")).hexdigest()


def file_hash(path):
    with open(path, "rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def fixture_manifest(root, tasks):
    """Hash frozen bytes without loading private fixture contents into reports."""
    root = Path(root).resolve()
    members = {}
    if len(set(tasks)) != len(tasks) or not tasks:
        raise ValueError("fixture membership must be nonempty and unique")
    for task in tasks:
        if Path(task).name != task or task in (".", ".."):
            raise ValueError("invalid task id")
        directory = root / task
        if not directory.is_dir() or directory.is_symlink():
            raise ValueError("missing or linked fixture: " + task)
        files = {}
        for path in sorted(directory.rglob("*")):
            if path.is_symlink() or not path.resolve().is_relative_to(root):
                raise ValueError("linked fixture path")
            if path.is_file():
                # Generated interpreter/cache files are never fixture inputs.
                if any(part in ("__pycache__", ".pytest_cache", ".git")
                       for part in path.relative_to(directory).parts):
                    continue
                files[path.relative_to(directory).as_posix()] = file_hash(path)
        if not files:
            raise ValueError("empty fixture: " + task)
        members[task] = files
    return {"schema_version": 2, "tasks": list(tasks), "files": members,
            "fixture_hash": digest(members)}


def schedule(tasks, repetitions, harnesses, seed=0):
    """Rotate first harness across tasks and repetitions; preserve every pair."""
    if not harnesses or len(set(harnesses)) != len(harnesses):
        raise ValueError("harnesses must be nonempty and unique")
    tasks, repetitions = list(tasks), list(repetitions)
    if len(set(tasks)) != len(tasks) or len(set(repetitions)) != len(repetitions):
        raise ValueError("duplicate task or repetition")
    offset = int(digest(seed)[:8], 16) % len(harnesses)
    result = []
    for rep in repetitions:
        for index, task in enumerate(tasks):
            first = (offset + index + rep) % len(harnesses)
            order = harnesses[first:] + harnesses[:first]
            result.extend((harness, task, rep) for harness in order)
    return result


def task_success(row):
    """Each component must be present; missing evidence cannot establish success."""
    return (type(row.get("exit")) is int and row["exit"] == 0
            and row.get("grader_pass") is True
            and row.get("unrelated") == []
            and row.get("cleanup_complete") is True
            and row.get("transaction_leak") is False
            and row.get("artifact_leak") is False)


def integrity_errors(rows, tasks, repetitions, harnesses=("davinci", "codex")):
    """Validate a complete single-variant campaign without dropping failed runs."""
    errors, seen = [], set()
    if not rows:
        return ["empty campaign"]
    if any(not isinstance(row, dict) for row in rows):
        return ["campaign rows must be objects"]
    campaign, variant = rows[0].get("campaign"), rows[0].get("variant")
    if not campaign or not variant:
        errors.append("missing campaign or variant identity")
    expected = {(h, t, r) for h in harnesses for t in tasks for r in repetitions}
    fixed = {field: rows[0].get(field) for field in COMPARABLE}
    per_harness = {}
    per_harness_isolation = {}
    for row in rows:
        key = (row.get("harness"), row.get("task"), row.get("rep"))
        if (not isinstance(key[0], str) or not isinstance(key[1], str)
                or type(key[2]) is not int):
            errors.append("invalid harness/task/repetition identity")
            continue
        if key in seen:
            errors.append("duplicate row: " + repr(key))
        seen.add(key)
        if key not in expected:
            errors.append("unexpected row: " + repr(key))
        if row.get("schema_version") != 2:
            errors.append("unsupported row schema")
        if row.get("campaign") != campaign or row.get("variant") != variant:
            errors.append("mixed campaign or variant")
        for field, value in fixed.items():
            if value is None or row.get(field) != value:
                errors.append("incompatible or missing " + field)
        identity = (row.get("binary_sha256"), row.get("effective_settings"))
        if any(value is None for value in identity):
            errors.append("missing binary/settings identity")
        previous = per_harness.setdefault(row.get("harness"), identity)
        if previous != identity:
            errors.append("binary/settings changed within harness")
        isolation = row.get("grading_isolation")
        if isolation not in GRADING_ISOLATIONS:
            errors.append("invalid or missing grading isolation")
        previous_isolation = per_harness_isolation.setdefault(row.get("harness"), isolation)
        if previous_isolation != isolation:
            errors.append("grading isolation changed within harness")
        wall = row.get("wall_s")
        if not isinstance(wall, (int, float)) or isinstance(wall, bool) or not math.isfinite(wall) or wall < 0:
            errors.append("invalid monotonic duration")
        if type(row.get("pass")) is not bool or row["pass"] != task_success(row):
            errors.append("invalid composite success: " + repr(key))
    errors.extend("missing row: " + repr(key) for key in sorted(expected - seen))
    return errors


def manifest_errors(manifest, rows):
    """Rows must match the pinned manifest, not just agree with one another."""
    if not isinstance(manifest, dict):
        return ["invalid campaign manifest"]
    for field in ("tasks", "repetitions", "harnesses"):
        values = manifest.get(field)
        expected_type = int if field == "repetitions" else str
        if (not isinstance(values, list) or not values
                or any(type(value) is not expected_type for value in values)
                or len(set(values)) != len(values)):
            return ["invalid manifest " + field]
    identities = manifest.get("identities")
    if not isinstance(identities, dict):
        return ["missing manifest executable identities"]
    isolations = manifest.get("grading_isolation")
    if (not isinstance(isolations, dict)
            or any(harness not in isolations for harness in manifest["harnesses"])
            or any(isolations[harness] not in GRADING_ISOLATIONS for harness in manifest["harnesses"])):
        return ["missing or invalid manifest grading isolation"]
    errors = integrity_errors(rows, manifest["tasks"], manifest["repetitions"], manifest["harnesses"])
    for row in rows:
        if not isinstance(row, dict):
            continue
        for field in ("schema_version", "campaign", "variant", *COMPARABLE):
            if manifest.get(field) is None or row.get(field) != manifest[field]:
                errors.append("row differs from manifest: " + field)
        harness = row.get("harness")
        if isinstance(harness, str) and row.get("grading_isolation") != isolations.get(harness):
            errors.append("row differs from manifest: grading_isolation")
        identity = identities.get(harness) if isinstance(harness, str) else None
        if not isinstance(identity, dict) or not identity:
            errors.append("missing manifest harness identity")
            continue
        for field, value in identity.items():
            if field not in row or row[field] != value:
                errors.append("row differs from manifest identity: " + field)
    return errors


def metric(row, field):
    if field == "uncached_input_tokens":
        total, cached = row.get("input_tokens"), row.get("cached_tokens")
        return total - cached if type(total) is int and type(cached) is int and 0 <= cached <= total else None
    if field == "gate_reminder_count":
        reasons = row.get("gate_reminders")
        return sum(reasons.values()) if isinstance(reasons, dict) and all(
            type(value) is int and value >= 0 for value in reasons.values()) else None
    return row.get(field)


def paired_metrics(baseline, candidate):
    """Candidate / baseline ratios over exact task/repetition pairs, failures included."""
    left = {(row["task"], row["rep"]): row for row in baseline}
    right = {(row["task"], row["rep"]): row for row in candidate}
    if not left or left.keys() != right.keys() or len(left) != len(baseline) or len(right) != len(candidate):
        raise ValueError("paired metrics require identical, unique task/repetition membership")
    result = {"pairs": len(left), "ratio_direction": "candidate / baseline",
              "baseline_passes": sum(row["pass"] for row in baseline),
              "candidate_passes": sum(row["pass"] for row in candidate)}
    for field in ("wall_s", "logical_requests", "provider_attempts", "tool_calls",
                  "input_tokens", "cached_tokens", "uncached_input_tokens", "output_tokens",
                  "reasoning_tokens", "auto_verify_runs", "gate_reminder_count",
                  "requests_after_first_reminder", "mutations_after_reminder", "executed_leaf_operations"):
        pairs = [(metric(left[key], field), metric(right[key], field)) for key in sorted(left)]
        valid = lambda value: type(value) in (int, float) and math.isfinite(value) and value >= 0
        unavailable = sum(not (valid(a) and valid(b)) for a, b in pairs)
        ratios = None if unavailable or any(a == 0 for a, _ in pairs) else [b / a for a, b in pairs]
        result[field] = {
            "unavailable_pairs": unavailable,
            "median_ratio": statistics.median(ratios) if ratios is not None else None,
            "median_delta": None if unavailable else statistics.median(b - a for a, b in pairs),
            "total_delta": None if unavailable else sum(b - a for a, b in pairs),
        }
    return result


def percentile(values, fraction):
    ordered = sorted(values)
    position = (len(ordered) - 1) * fraction
    lower = int(position)
    upper = min(lower + 1, len(ordered) - 1)
    return ordered[lower] + (ordered[upper] - ordered[lower]) * (position - lower)


def latency_uncertainty(baseline, candidate, samples=2000, seed=0):
    """Paired task-cluster percentile bootstrap; repetitions stay in their task."""
    paired_metrics(baseline, candidate)  # Reject duplicates and mismatched pairs.
    left = {(row["task"], row["rep"]): row for row in baseline}
    right = {(row["task"], row["rep"]): row for row in candidate}
    tasks = sorted({key[0] for key in left})
    groups = {task: [(left[key]["wall_s"], right[key]["wall_s"])
                     for key in sorted(left) if key[0] == task] for task in tasks}
    result = {"method": "paired task-cluster percentile bootstrap", "confidence": 0.95,
              "resamples": samples, "seed": seed, "clusters": len(tasks),
              "scope": "frozen task families; repeated runs are not independent tasks"}
    if len(tasks) < 2 or any(a <= 0 or b < 0 for group in groups.values() for a, b in group):
        return dict(result, available=False, reason="at least two tasks and positive baseline durations required")
    rng = random.Random(seed)
    ratios, deltas, tail_ratios, paired_ratios = [], [], [], []
    for _ in range(samples):
        pairs = [pair for task in rng.choices(tasks, k=len(tasks)) for pair in groups[task]]
        a, b = zip(*pairs)
        ratios.append(statistics.median(b) / statistics.median(a))
        deltas.append(statistics.median(y - x for x, y in pairs))
        tail_ratios.append(percentile(b, 0.9) / percentile(a, 0.9))
        paired_ratios.append(statistics.median(y / x for x, y in pairs))
    for field, values, null in (("ratio_of_medians", ratios, 1), ("median_paired_delta_s", deltas, 0),
                                ("p90_ratio", tail_ratios, 1), ("median_paired_ratio", paired_ratios, 1)):
        bounds = [percentile(values, 0.025), percentile(values, 0.975)]
        result[field] = {"ci95": bounds, "includes_no_change": bounds[0] <= null <= bounds[1]}
    return dict(result, available=True)


def serial_campaigns(baseline, candidate):
    """A campaign spans its entire schedule, including time between rows."""
    try:
        windows = []
        for rows in (baseline, candidate):
            starts = [datetime.fromisoformat(row["started_at"]) for row in rows]
            ends = [datetime.fromisoformat(row["finished_at"]) for row in rows]
            if not starts or any(start.tzinfo is None or end.tzinfo is None or end < start
                                 for start, end in zip(starts, ends)):
                return None
            windows.append((min(starts), max(ends)))
        return windows[0][1] <= windows[1][0] or windows[1][1] <= windows[0][0]
    except (KeyError, ValueError, TypeError):
        return None


def promotion_gates(baseline, candidate, baseline_manifest, candidate_manifest, uncertainty, provenance,
                    *, baseline_campaign=None, candidate_campaign=None):
    """Expose each rule. One paired diagnostic window cannot certify promotion."""
    rules = {}
    def rule(name, passed, evidence):
        rules[name] = {"status": "unavailable" if passed is None else "pass" if passed else "fail",
                       "evidence": evidence}
    rule("committed_source_and_actual_merge_base", provenance.get("verified"), provenance)
    complete_campaigns = baseline_campaign is not None and candidate_campaign is not None
    rule("sequential_campaigns", serial_campaigns(baseline_campaign, candidate_campaign)
         if complete_campaigns else None,
         {"scope": "complete campaign envelopes, including every Codex control row",
          "parent_rows": len(baseline_campaign) if complete_campaigns else None,
          "candidate_rows": len(candidate_campaign) if complete_campaigns else None})
    toolings = [manifest.get("tooling_preflight", {}).get("davinci")
                for manifest in (baseline_manifest, candidate_manifest)]
    rule("usable_comparable_tooling", None if not all(toolings) else
         all(entry.get("available") is True for entry in toolings) and toolings[0] == toolings[1], toolings)
    protected = all(row.get("grading_assurance") == "independent-hidden-grader"
                    for row in baseline + candidate)
    rule("independent_grading_boundary", protected,
         "current container and native arms are diagnostic; tools can read auth and public fixture generators")
    memberships = [{(row["task"], row["rep"]) for row in rows} for rows in (baseline, candidate)]
    repetitions = [{task: len({rep for tid, rep in membership if tid == task})
                    for task in {tid for tid, _ in membership}} for membership in memberships]
    rule("ten_repetitions_per_task", all(count >= 10 for arm in repetitions for count in arm.values()), repetitions)
    rule("legacy_and_large_strata", all(set(LEGACY_TASKS).issubset(arm) and
         len(set(arm) - set(LEGACY_TASKS)) >= 4 for arm in repetitions),
         "all eight legacy and at least four frozen larger tasks are required")
    rule("two_independent_windows", None, "this report evaluates one paired window; cross-window review is required")
    rule("untouched_holdout", False, "bundled tasks, graders and solutions are public; fresh repetitions are not an untouched holdout")
    rule("deterministic_regressions", None, "requires the affected code's test/eval evidence outside this performance report")
    parent_passes = sum(row["pass"] for row in baseline)
    child_passes = sum(row["pass"] for row in candidate)
    rule("candidate_passes_at_least_parent", child_passes >= parent_passes,
         {"parent": parent_passes, "candidate": child_passes})
    prior = {(row["task"], row["rep"]): row for row in baseline}
    regressions = [{"task": row["task"], "rep": row["rep"]} for row in candidate
                   if prior[(row["task"], row["rep"])]["pass"] and not row["pass"]]
    rule("every_correctness_regression_reviewed", not regressions,
         {"unreviewed_parent_pass_candidate_fail": regressions})
    a, b = [row["wall_s"] for row in baseline], [row["wall_s"] for row in candidate]
    median_ratio = statistics.median(b) / statistics.median(a) if statistics.median(a) else None
    p90_ratio = percentile(b, 0.9) / percentile(a, 0.9) if percentile(a, 0.9) else None
    rule("median_at_least_ten_percent_faster", None if median_ratio is None else median_ratio <= 0.9,
         {"candidate_divided_by_parent_medians": median_ratio, "maximum": 0.9})
    rule("p90_regression_at_most_ten_percent", None if p90_ratio is None else p90_ratio <= 1.1,
         {"candidate_divided_by_parent_p90": p90_ratio, "maximum": 1.1})
    bounds = uncertainty.get("ratio_of_medians", {}).get("ci95")
    rule("latency_improvement_with_uncertainty", None if not bounds else bounds[1] < 1,
         {"ratio_of_medians_ci95": bounds, "method": uncertainty["method"]})
    complete_requests = all(row.get("request_metrics_complete") is True for row in baseline + candidate)
    for field, name in (("logical_requests", "logical_request_reduction"),
                        ("requests_after_first_reminder", "gate_continuation_reduction")):
        values = [[metric(row, field) for row in rows] for rows in (baseline, candidate)]
        available = complete_requests and all(type(value) is int and value >= 0 for arm in values for value in arm)
        totals = [sum(arm) for arm in values] if available else None
        rule(name, totals[1] < totals[0] if totals else None,
             {"totals_parent_candidate": totals, "complete_request_telemetry": complete_requests})
    for field in ("uncached_input_tokens", "output_tokens"):
        changes = {}
        for task in repetitions[0]:
            arms = [[metric(row, field) for row in rows if row["task"] == task] for rows in (baseline, candidate)]
            changes[task] = ([sum(arm) for arm in arms] if all(type(v) is int and v >= 0 for arm in arms for v in arm) else None)
        rule(field + "_no_task_regression", None if any(v is None for v in changes.values()) else
             all(v[1] <= v[0] for v in changes.values()), {"totals_parent_candidate_by_task": changes})
    return {"accepted": all(value["status"] == "pass" for value in rules.values()),
            "scope": "one paired window; fail or unavailable rules block promotion", "rules": rules}
