"""Deterministic campaign identity and integrity; never starts a model."""
import hashlib
import json
import math
import statistics
from pathlib import Path


LEGACY_TASKS = (
    "t1-intervals", "t2-duration", "t3-lru", "t4-rename",
    "t5-csv", "t6-bookings", "t7-cli", "t8-calc",
)
COMPARABLE = ("fixture_hash", "model", "effort_policy", "service_tier")


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
    errors = integrity_errors(rows, manifest["tasks"], manifest["repetitions"], manifest["harnesses"])
    for row in rows:
        if not isinstance(row, dict):
            continue
        for field in ("schema_version", "campaign", "variant", *COMPARABLE):
            if manifest.get(field) is None or row.get(field) != manifest[field]:
                errors.append("row differs from manifest: " + field)
        harness = row.get("harness")
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
