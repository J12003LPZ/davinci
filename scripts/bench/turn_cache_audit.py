"""Per-request cache reuse and turn purpose from recorded DaVinci run logs.

    python scripts/bench/turn_cache_audit.py <campaign-dir> [--out <dir>]

Offline: reads `<campaign>/davinci/*.stdout.jsonl` only. No model calls, no
grader input. For every provider request of every run it records the provider's
own token report, how much of the previous request's input came back from
cache, what the harness or the model did between the two requests, and what
the response went on to do. It writes `turn-cache-requests.csv` and
`turn-cache-summary.json`.

"Prefix shortfall" is `previous input_total - cache_read`, floored at zero:
the previous request already sent that many input tokens, so an append-only
conversation could have read them all back. It says that cache was not reused,
not why.
"""
import argparse
import csv
import json
from pathlib import Path
import re

SHELLS = {"exec_command", "bash", "shell", "powershell"}
READERS = {"read", "ls", "list_directory", "find", "grep", "repo_map", "retrieve_output", "codemode",
           "lsp_diagnostics", "code_diagnostics", "workspace_packages", "package_info", "git_branch_diff"}
EDITORS = {"edit", "write", "apply_patch"}
CHECK = re.compile(r"\b(pytest|unittest|compileall|py_compile)\b|\b(cargo|go|npm|pnpm)\s+test\b|\btest_[\w.-]+\.py\b")
# A cached count this close to the previous input is a full reuse: the provider
# reports cache hits in coarse blocks, so the tail of the prefix never matches.
FULL_REUSE_SLACK = 2048
STATS = ("modelTurns", "toolCalls", "toolBatches", "batchOperations", "completionRequirementReminders",
         "verificationCommandsRun", "repeatedReads", "repeatedSearches", "providerAttempts", "providerRetries",
         "usageCompleteAttempts", "usageUnknownAttempts", "compactions", "peakContextTokens")


def leaves(name, args):
    name = (name or "").removeprefix("functions.")
    if name == "batch":
        for op in (args or {}).get("operations", []):
            yield from leaves(op.get("tool", ""), op.get("args", {}))
    else:
        yield name, args or {}


def events(path):
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        try:
            event = json.loads(line)
        except ValueError:
            continue  # a truncated final line is not evidence
        if isinstance(event, dict):
            yield event


def action(calls):
    """What a response went on to do, from the tool calls that followed it."""
    if not calls:
        return "final-answer"
    kinds = set()
    for name, args in calls:
        if name in EDITORS:
            kinds.add("edit")
        elif name in SHELLS:
            kinds.add("verify" if CHECK.search(str(args.get("command", ""))) else "shell")
        elif name == "todo":
            kinds.add("todo")
        elif name in READERS:
            kinds.add("explore")
        else:
            kinds.add("other")
    for kind in ("edit", "verify", "shell", "explore", "other"):
        if kind in kinds:
            return kind
    return "todo-only"


def reuse(previous_input, cache_read):
    if previous_input is None:
        return "first"
    if cache_read == 0:
        return "zero"
    return "full" if cache_read + FULL_REUSE_SLACK >= previous_input else "partial"


def audit_run(path):
    """Request rows and run counters for one stdout log."""
    requests, stats, current = [], None, None
    between = []  # what happened since the previous attempt ended
    for event in events(path):
        kind = event.get("type")
        if kind == "provider_observation":
            seen = event.get("observation") or {}
            if seen.get("purpose") not in (None, "coding"):
                continue  # compaction and helper requests are not turns of the task
            if seen.get("kind") == "attempt_start":
                current = {"trigger": trigger(between, seen), "calls": []}
            elif seen.get("kind") == "attempt_end" and current is not None:
                usage = seen.get("raw_usage") or {}
                current.update(
                    logical_request_id=seen.get("logical_request_id"), attempt_id=seen.get("attempt_id"),
                    transport=seen.get("transport"), schema_hash=seen.get("schema_hash"),
                    http_status=seen.get("http_status"), status=seen.get("status"),
                    duration_ms=seen.get("duration_ms"), usage_complete=seen.get("usage_complete"),
                    input_total=usage.get("input_total"), cache_read=usage.get("cache_read"),
                    cache_write=usage.get("cache_write"), output=usage.get("output"),
                    reasoning=usage.get("reasoning"))
                requests.append(current)
                current, between = None, []
        elif kind == "tool_execution_start":
            found = list(leaves(event.get("toolName"), event.get("args")))
            between.append("tool")
            if requests:
                requests[-1]["calls"].extend(found)
        elif kind in ("completion_reminder", "verification_notice", "auto_retry_start"):
            between.append(kind)
        elif kind == "harness_stats":
            stats = event.get("runtime") or {}
    previous = None
    for index, request in enumerate(requests):
        calls = request.pop("calls")
        request.update(index=index, action=action(calls), tools=" ".join(name for name, _ in calls))
        known = isinstance(request["input_total"], int) and isinstance(request["cache_read"], int)
        prior = previous["input_total"] if previous else None
        request["previous_input_total"] = prior
        request["reuse"] = reuse(prior, request["cache_read"]) if known and (previous is None or prior is not None) \
            else "unknown"
        request["prefix_shortfall"] = max(0, prior - request["cache_read"]) if known and prior is not None else None
        previous = request if known else None
    return requests, stats


def trigger(between, seen):
    if (seen.get("attempt_id") or 1) > 1 or "auto_retry_start" in between:
        return "retry"
    if "completion_reminder" in between:
        return "completion-reminder"
    if "verification_notice" in between:
        return "verification-notice"
    return "tool-results" if "tool" in between else "start"


def summarize(runs):
    later = [r for rows, _ in runs.values() for r in rows if r["reuse"] not in ("first", "unknown")]
    every = [r for rows, _ in runs.values() for r in rows]
    counted = [r for r in every if r["reuse"] != "unknown"]

    def count(rows, key):
        result = {}
        for row in rows:
            result[row[key]] = result.get(row[key], 0) + 1
        return dict(sorted(result.items()))

    prefix = sum(r["previous_input_total"] for r in later)
    shortfall = sum(r["prefix_shortfall"] for r in later)
    later_input = sum(r["input_total"] for r in later)
    stats = [s for _, s in runs.values() if s]

    def uncached(rows):
        return sum(r["input_total"] - r["cache_read"] for r in rows if r["reuse"] != "unknown")

    # Everything from the first completion reminder to the end of the run is
    # work the harness asked for after the model had already answered.
    tails = [rows[next(i for i, r in enumerate(rows) if r["trigger"] == "completion-reminder"):]
             for rows, _ in runs.values() if any(r["trigger"] == "completion-reminder" for r in rows)]
    return {
        "requests_from_first_completion_reminder": sum(len(tail) for tail in tails),
        "uncached_input": uncached(every),
        "uncached_input_from_first_completion_reminder": sum(uncached(tail) for tail in tails),
        "runs": len(runs), "requests": len(every), "requests_with_usage": len(counted), "later_requests": len(later),
        "reuse": count(later, "reuse"), "transport": count(every, "transport"),
        "trigger": count(every, "trigger"), "action": count(every, "action"),
        "later_request_cache_hit": sum(r["cache_read"] for r in later) / later_input if later_input else None,
        "prefix_tokens": prefix, "prefix_shortfall": shortfall,
        "prefix_shortfall_fraction": shortfall / prefix if prefix else None,
        "cache_read_lower_than_previous": sum(
            1 for rows, _ in runs.values() for a, b in zip(rows, rows[1:])
            if b["reuse"] not in ("first", "unknown") and a["cache_read"] is not None and b["cache_read"] < a["cache_read"]),
        "runs_with_stats": len(stats),
        "per_run_mean": {key: sum(s.get(key) or 0 for s in stats) / len(stats) for key in STATS} if stats else {},
        "runs_with_completion_reminder": sum(
            any(r["trigger"] == "completion-reminder" for r in rows) for rows, _ in runs.values()),
        "requests_per_run": len(every) / len(runs) if runs else None,
    }


FIELDS = ("run", "index", "trigger", "action", "tools", "reuse", "input_total", "cache_read", "cache_write", "output",
          "reasoning", "previous_input_total", "prefix_shortfall", "transport", "schema_hash", "http_status",
          "status", "attempt_id", "duration_ms", "usage_complete", "logical_request_id")


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("campaign", type=Path)
    parser.add_argument("--out", type=Path)
    args = parser.parse_args()
    logs = sorted((args.campaign / "davinci").glob("*.stdout.jsonl"))
    if not logs:
        parser.error("no davinci/*.stdout.jsonl under the campaign directory")
    runs = {log.name.removesuffix(".stdout.jsonl"): audit_run(log) for log in logs}
    summary = summarize(runs)
    out = args.out or args.campaign
    out.mkdir(parents=True, exist_ok=True)
    with (out / "turn-cache-requests.csv").open("w", newline="", encoding="utf-8") as stream:
        writer = csv.DictWriter(stream, FIELDS, extrasaction="ignore")
        writer.writeheader()
        for run, (rows, _) in runs.items():
            for row in rows:
                writer.writerow({**row, "run": run})
    (out / "turn-cache-summary.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
