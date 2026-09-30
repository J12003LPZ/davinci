"""Comparable readiness metrics with unavailable values kept explicit."""
import json
from datetime import date
import math
from pathlib import Path
import statistics

from campaign import digest, metric, percentile, task_success


def number(value):
    return type(value) in (int, float) and math.isfinite(value) and value >= 0


def load_prices(path):
    table = json.loads(Path(path).read_text(encoding="utf-8"))
    validate_prices(table)
    return table, digest(table)


def validate_prices(table):
    if (not isinstance(table, dict) or table.get("schema_version") != 1 or table.get("currency") != "USD"
            or not isinstance(table.get("as_of"), str) or not isinstance(table.get("source"), str) or not table["source"].strip()
            or not isinstance(table.get("models"), dict)):
        raise ValueError("price table needs schema, currency, date, source and model rates")
    try:
        if date.fromisoformat(table["as_of"]).isoformat() != table["as_of"]:
            raise ValueError("noncanonical date")
    except ValueError as error:
        raise ValueError("price table as_of must be an ISO calendar date") from error
    for model, rates in table["models"].items():
        if not isinstance(model, str) or not isinstance(rates, dict):
            raise ValueError("invalid model price entry")
        for field in ("input_per_million", "cached_input_per_million", "output_per_million"):
            if not number(rates.get(field)):
                raise ValueError("price rates must be finite nonnegative numbers")
        if "cache_write_per_million" in rates and not number(rates["cache_write_per_million"]):
            raise ValueError("invalid cache-write price")


def available_sum(values):
    return sum(values) if values and all(number(value) for value in values) else None


def estimate(row, prices):
    if prices is None:
        return None
    rates = prices["models"].get(row.get("model"))
    uncached, cached, output = metric(row, "uncached_input_tokens"), row.get("cached_tokens"), row.get("output_tokens")
    if rates is None or not all(number(value) for value in (uncached, cached, output)):
        return None
    # Responses has no separate cache-write charge. Other providers must supply
    # cache-write tokens and a pinned rate rather than price them as plain input.
    written = row.get("cache_write_tokens")
    if not number(written) or written > uncached:
        return None
    write_rate = rates.get("cache_write_per_million")
    if written and not number(write_rate):
        return None
    return ((uncached - written) * rates["input_per_million"]
            + cached * rates["cached_input_per_million"]
            + output * rates["output_per_million"]
            + written * (write_rate or 0)) / 1_000_000


def background_summary(rows):
    modes = {row.get("execution_mode", "print") for row in rows}
    if modes == {"print"}:
        return None, "unmeasured in print mode"
    if "print" in modes or not modes.issubset({"interactive", "rpc"}):
        return None, "mixed or unknown execution modes"
    result = {}
    for source in ("learning", "securityWatch"):
        entries = [row.get("background_usage", {}).get(source) for row in rows]
        if any(not isinstance(entry, dict) or not isinstance(entry.get("tokens"), dict)
               or entry.get("pendingRequests") != 0 or entry.get("unknownTokenRequests") != 0 for entry in entries):
            result[source] = None
        else:
            result[source] = {field: available_sum([entry["tokens"].get(field) for entry in entries])
                              for field in ("input", "output", "cacheRead", "cacheWrite")}
    return result, "separate measured interactive/RPC background calls"


def summarize(rows, prices=None):
    if not rows:
        raise ValueError("readiness summary needs at least one run")
    if prices is not None:
        validate_prices(prices)
    successes = sum(task_success(row) for row in rows)
    def per_success(value):
        return value / successes if successes and value is not None else None
    uncached = available_sum([metric(row, "uncached_input_tokens") for row in rows])
    cached = available_sum([row.get("cached_tokens") for row in rows])
    output = available_sum([row.get("output_tokens") for row in rows])
    costs = available_sum([estimate(row, prices) for row in rows])
    regression = [row.get("regression_pass") for row in rows]
    unrelated = [row.get("unrelated") for row in rows]
    walls = [row.get("wall_s") for row in rows]
    requests = [row.get("logical_requests") if row.get("request_metrics_complete") is True else None for row in rows]
    tools = [row.get("tool_calls") for row in rows]
    background, scope = background_summary(rows)
    return {"runs": len(rows), "composite_successes": successes, "composite_success_rate": successes / len(rows),
            "regression_free_successes": sum(task_success(row) and row.get("regression_pass") is True for row in rows)
                if all(type(value) is bool for value in regression) else None,
            "regression_observed_runs": sum(type(value) is bool for value in regression),
            "unrelated_edit_rate": sum(bool(value) for value in unrelated) / len(rows)
                if all(isinstance(value, list) for value in unrelated) else None,
            "uncached_input_tokens": uncached, "cached_input_tokens": cached, "output_tokens": output,
            "uncached_input_per_verified_success": per_success(uncached),
            "cached_input_per_verified_success": per_success(cached),
            "output_per_verified_success": per_success(output),
            "estimated_usd": costs, "estimated_usd_per_verified_success": per_success(costs),
            "median_wall_s": statistics.median(walls) if all(number(value) for value in walls) else None,
            "p90_wall_s": percentile(walls, .9) if all(number(value) for value in walls) else None,
            "model_requests_per_task": available_sum(requests) / len(rows) if available_sum(requests) is not None else None,
            "tool_calls_per_task": available_sum(tools) / len(rows) if available_sum(tools) is not None else None,
            "background_tokens": background, "background_scope": scope,
            "spend_scope": "all attempted runs, including failures, divided by verified composite successes"}


def report(rows, prices=None):
    arms = {}
    for harness in sorted({row["harness"] for row in rows}):
        selected = [row for row in rows if row["harness"] == harness]
        arms[harness] = {"all": summarize(selected, prices)}
        for size in ("small", "large"):
            sized = [row for row in selected if row.get("size_class", "small" if row.get("task_set") == "legacy" else "large") == size]
            if sized:
                arms[harness][size] = summarize(sized, prices)
    return {"arms": arms, "price_table_hash": digest(prices) if prices is not None else None,
            "price_label": "estimate from pinned price table", "measured_competitor_claim": False}
