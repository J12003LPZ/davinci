"""What each summarizer (compaction) request bought, from recorded DaVinci run logs.

    python scripts/bench/fold_economics.py <run.stdout.jsonl>... [--json]

Offline: reads the given `stdout.jsonl` files only. No model calls. Only the
root actor's provider attempts are read (`attempt_end` with no
`parent_actor_id`): a subagent's requests are not the main prompt. Of those,
only completed ones (`status == "completed"`) are measured, because a failed or
retried attempt has no usable usage. A failed attempt still separates two
compactions: compaction, a refused request, then a recovery compaction are two
rows, not one.

A fold may send several summarizer requests back to back (one per chunk of
unfolded history). Consecutive `purpose == "compaction"` requests with no
coding attempt (completed or not) between them are one compaction row; its cost
is the sum of the completed ones.
For each row it records the cost, how big the coding request just before and
just after it was, and what the reduction is worth if every later coding
request keeps it:

    reduction = input_total of the last coding request before
                - input_total of the first coding request after   (can be < 0)
    est_saved = max(reduction, 0) * coding requests until the next compaction
    net       = est_saved - the summarizer's own input_total

The reduction is a size heuristic, not proof that a compaction was applied: a
prompt also shrinks or grows for other reasons. When the run's final
`harness_stats` carries `contextVm.hybridCompaction` it is reported beside the
rows, and that counter is the authority on what was applied. The "all runs"
section sums the counters of the runs that report them.

A usage figure the log does not carry is `None` (shown as "unknown"), never 0.
A row whose requests are not all known has an unknown cost. Totals add the
known rows only; the summary counts the unknown ones.
"""
import argparse
import json
import math
from pathlib import Path


def events(path):
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        try:
            event = json.loads(line)
        except (ValueError, RecursionError):
            continue  # a truncated final line, or JSON nested too deep, is not evidence
        if isinstance(event, dict):
            yield event


def count(value):
    """A token count, or None when the log does not carry one."""
    return value if type(value) is int and value >= 0 else None


def milliseconds(value):
    if type(value) not in (int, float) or not math.isfinite(value) or value < 0:
        return None
    return round(value)


def dictionary(value):
    return value if isinstance(value, dict) else {}


def usage_figures(observation):
    """(input_total, cache_read, output) from `raw_usage`, or from `usage` when the log has no
    `raw_usage` object. `usage.input` excludes the cached part, so the total adds it back."""
    raw = observation.get("raw_usage")
    if isinstance(raw, dict):
        return count(raw.get("input_total")), count(raw.get("cache_read")), count(raw.get("output"))
    usage = dictionary(observation.get("usage"))
    plain, cache_read = count(usage.get("input")), count(usage.get("cacheRead"))
    cache_write = count(usage.get("cacheWrite")) or 0
    if usage.get("inputUnreported") is True:
        plain = None
    total = None if plain is None or cache_read is None else plain + cache_read + cache_write
    return total, cache_read, count(usage.get("output"))


def request(observation, turn):
    total, cache_read, output = usage_figures(observation)
    return {"purpose": observation.get("purpose"), "turn": turn,
            "completed": observation.get("status") == "completed",
            "input": total, "cache_read": cache_read, "output": output,
            "duration_ms": milliseconds(observation.get("duration_ms"))}


def counted(observation):
    """An attempt of the root actor, whatever its status."""
    return (observation.get("kind") == "attempt_end" and observation.get("parent_actor_id") is None
            and observation.get("purpose") in ("coding", "compaction"))


def requests(path):
    """Root attempts in log order (each says whether it completed), and the final harness stats."""
    found, stats, turn = [], None, 0
    for event in events(path):
        kind = event.get("type")
        if kind == "turn_start":
            turn += 1
        elif kind == "harness_stats":
            stats = event
        elif kind == "provider_observation":
            observation = event.get("observation")
            if isinstance(observation, dict) and counted(observation):
                found.append(request(observation, turn))
    return found, stats


def difference(left, right):
    return None if left is None or right is None else left - right


def known_sum(values):
    known = [value for value in values if value is not None]
    return sum(known) if known else None


def whole_sum(values):
    """The sum, or None when any part is unknown."""
    values = list(values)
    return None if any(value is None for value in values) else sum(values)


def share(coding):
    """Cached share of input over the requests whose usage is known."""
    pairs = [(item["cache_read"], item["input"]) for item in coding
             if item["cache_read"] is not None and item["input"]]
    total = sum(input_total for _, input_total in pairs)
    return sum(cached for cached, _ in pairs) / total if total else None


def reported(stats):
    """The harness's own hybrid compaction counters, or None when it has none."""
    vm = dictionary(dictionary(stats).get("contextVm"))
    hybrid = vm.get("hybridCompaction")
    if not isinstance(hybrid, dict):
        return None
    runtime = dictionary(dictionary(stats).get("runtime"))
    mode = vm.get("mode")
    return {"mode": mode if isinstance(mode, str) else None, "compactions": count(runtime.get("compactions")),
            **{key: count(value) for key, value in hybrid.items() if isinstance(key, str)}}


def groups(log):
    """Index ranges (start, stop) of consecutive compaction attempts."""
    found, start = [], None
    for index, item in enumerate(log + [{"purpose": "coding"}]):
        if item["purpose"] == "compaction":
            start = index if start is None else start
        elif start is not None:
            found.append((start, index))
            start = None
    return found


def analyze(path):
    log, stats = requests(path)
    # A group with no completed request measured nothing; its failed attempts still
    # separated the compactions around it.
    spans = [span for span in groups(log) if any(item["completed"] for item in log[span[0]:span[1]])]
    rows = []
    for number, (start, stop) in enumerate(spans, 1):
        end = spans[number][0] if number < len(spans) else len(log)
        members = [item for item in log[start:stop] if item["completed"]]
        coding_before = [item for item in log[:start] if item["purpose"] == "coding" and item["completed"]]
        coding_after = [item for item in log[stop:end] if item["purpose"] == "coding" and item["completed"]]
        before = coding_before[-1] if coding_before else {}
        after = coding_after[0] if coding_after else {}
        cost = whole_sum(item["input"] for item in members)
        reduction = difference(before.get("input"), after.get("input"))
        est_saved = None if reduction is None else max(reduction, 0) * len(coding_after)
        rows.append({
            "n": number, "turn": members[0]["turn"], "requests": len(members), "summarizer_input": cost,
            "summarizer_output": whole_sum(item["output"] for item in members),
            "summarizer_ms": whole_sum(item["duration_ms"] for item in members),
            "before_input": before.get("input"), "before_cache_read": before.get("cache_read"),
            "after_input": after.get("input"), "after_cache_read": after.get("cache_read"),
            "reduction": reduction, "subsequent_coding_requests": len(coding_after),
            "est_saved": est_saved, "net": difference(est_saved, cost)})
    coding = [item for item in log if item["purpose"] == "coding" and item["completed"]]
    first = spans[0] if spans else (len(log), len(log))
    positive = [row for row in rows if row["reduction"] is not None and row["reduction"] > 0]
    summary = {
        "compactions": len(rows),
        "compaction_requests": sum(row["requests"] for row in rows),
        "summarizer_input": known_sum(row["summarizer_input"] for row in rows),
        "summarizer_output": known_sum(row["summarizer_output"] for row in rows),
        "summarizer_ms": known_sum(row["summarizer_ms"] for row in rows),
        "summarizer_usage_unknown": sum(row["summarizer_input"] is None for row in rows),
        "compactions_with_positive_reduction": len(positive),
        "est_saved": known_sum(row["est_saved"] for row in rows),
        "net": known_sum(row["net"] for row in rows),
        "coding_requests": len(coding),
        "cached_share_before_first": share([item for item in log[:first[0]] if item in coding]),
        "cached_share_after_first": share([item for item in log[first[1]:] if item in coding])
        if spans else None}
    return {"run": str(path), "rows": rows, "summary": summary, "reported": reported(stats)}


def combine_reported(runs):
    """Counters summed over the runs that report them; a key unknown in any run is unknown."""
    found = [run["reported"] for run in runs if run["reported"]]
    if not found:
        return None
    total = {"runs_reporting": len(found), "mode": found[0]["mode"] if len({r["mode"] for r in found}) == 1
             else "mixed"}
    for key in dict.fromkeys(key for item in found for key in item if key != "mode"):
        total[key] = whole_sum(item.get(key) for item in found)
    return total


def combine(runs):
    rows = [row for run in runs for row in run["rows"]]
    keys = ("compactions", "compaction_requests", "compactions_with_positive_reduction",
            "summarizer_usage_unknown", "coding_requests")
    summaries = [run["summary"] for run in runs]
    total = {key: sum(item[key] for item in summaries) for key in keys}
    for key in ("summarizer_input", "summarizer_output", "summarizer_ms", "est_saved", "net"):
        total[key] = known_sum(item[key] for item in summaries)
    return {"run": "all runs", "combined": True, "rows": rows, "summary": total,
            "reported": combine_reported(runs)}


def cell(value, unit=""):
    if value is None:
        return "unknown"
    if isinstance(value, str):
        return value
    if isinstance(value, float):
        return f"{value:.1%}"
    return f"{value:,}{unit}"


COLUMNS = (("#", "n"), ("turn", "turn"), ("reqs", "requests"), ("summarizer in", "summarizer_input"),
           ("summarizer out", "summarizer_output"), ("summarizer ms", "summarizer_ms"),
           ("coding in before", "before_input"), ("cache before", "before_cache_read"),
           ("coding in after", "after_input"), ("cache after", "after_cache_read"),
           ("reduction", "reduction"), ("later coding reqs", "subsequent_coding_requests"),
           ("est saved", "est_saved"), ("net", "net"))


def markdown(run):
    lines = [f"### {run['run']}", ""]
    if run.get("combined"):
        pass  # the rows are above, under their own runs
    elif run["rows"]:
        lines += ["| " + " | ".join(name for name, _ in COLUMNS) + " |",
                  "|" + " --- |" * len(COLUMNS)]
        lines += ["| " + " | ".join(cell(row[key]) for _, key in COLUMNS) + " |" for row in run["rows"]]
    else:
        lines.append("No compaction requests.")
    summary = run["summary"]
    lines += ["", f"- compactions: {cell(summary['compactions'])} in {cell(summary['compaction_requests'])} "
                  f"summarizer requests (cost unknown on {summary['summarizer_usage_unknown']})",
              f"- summarizer input / output tokens: {cell(summary['summarizer_input'])} / "
              f"{cell(summary['summarizer_output'])}",
              f"- summarizer time: {cell(summary['summarizer_ms'], ' ms')}",
              f"- compactions with positive reduction: {summary['compactions_with_positive_reduction']}",
              f"- estimated tokens saved: {cell(summary['est_saved'])}",
              f"- net after summarizer input: {cell(summary['net'])}"]
    if "cached_share_before_first" in summary:
        lines.append(f"- coding requests cached share, before / after the first compaction: "
                     f"{cell(summary['cached_share_before_first'])} / {cell(summary['cached_share_after_first'])}")
    counters = run["reported"]
    lines.append("- harness hybridCompaction counters: " + (
        ", ".join(f"{key}={cell(value)}" for key, value in counters.items()) if counters else "not reported"))
    return "\n".join(lines)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__.split("\n\n")[0])
    parser.add_argument("logs", nargs="+", type=Path, help="run stdout.jsonl files")
    parser.add_argument("--json", action="store_true", help="print JSON instead of markdown")
    args = parser.parse_args(argv)
    runs = [analyze(path) for path in args.logs]
    if len(runs) > 1:
        runs.append(combine(runs))
    if args.json:
        print(json.dumps(runs, indent=2))
    else:
        print("\n\n".join(markdown(run) for run in runs))


if __name__ == "__main__":
    main()
