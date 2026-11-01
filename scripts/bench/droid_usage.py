"""Droid usage parsing from session settings files and log text.

Totals come from ``~/.factory/sessions/*/<id>.settings.json`` (a ``tokenUsage``
object) and the request count from ``Streaming result`` log lines filtered by
session id. Both functions take text/JSON, never a live process, so they are
unit-testable offline.
"""
import argparse
import json
import os
import re
import sys
from pathlib import Path


def droid_home(home=None):
    """Droid data root: ``home`` argument, else ``BENCH_DROID_HOME``, else ``~/.factory``."""
    return Path(home or os.environ.get("BENCH_DROID_HOME") or Path.home() / ".factory")


def droid_session_files(home=None):
    """Set of ``*.settings.json`` paths under ``<home>/sessions`` (read-only listing)."""
    root = droid_home(home) / "sessions"
    return {str(path) for path in root.rglob("*.settings.json")} if root.is_dir() else set()


def droid_log_text(home=None):
    """Concatenated text of ``droid-log-single.log`` and its dated rotations."""
    logs = droid_home(home) / "logs"
    if not logs.is_dir():
        return ""
    return "\n".join(path.read_text(encoding="utf-8", errors="replace")
                     for path in sorted(logs.glob("droid-log-single*.log*")) if path.is_file())


def _session_json(path):
    try:
        with open(path, encoding="utf-8") as source:
            data = json.load(source)
    except (OSError, ValueError):
        return {}
    return data if isinstance(data, dict) else {}


def _text(value):
    return value if isinstance(value, str) and value else None


def droid_session_mode(path):
    """Raw ``toolExecutionMode`` of a session settings file (for example ``script_only``), else None."""
    return _text(_session_json(path).get("toolExecutionMode"))


def droid_session_fields(path):
    """Raw session-recorded ``mode``, ``reasoning_effort`` and ``model`` (None when absent).

    Observed in real 1.1.0 session files, which spell the mode with underscores
    (``script_only``); compare through ``normalize_mode``.
    """
    data = _session_json(path)
    return {"mode": _text(data.get("toolExecutionMode")),
            "reasoning_effort": _text(data.get("reasoningEffort")),
            "model": _text(data.get("model"))}


def normalize_mode(value):
    """``script_only`` and ``script-only`` name the same mode."""
    return value.strip().lower().replace("_", "-") if isinstance(value, str) else None


def _non_negative_int(value):
    return value if type(value) is int and not isinstance(value, bool) and value >= 0 else None


def _first_int(usage, *names):
    for name in names:
        value = _non_negative_int(usage.get(name))
        if value is not None:
            return value
    return None


def droid_usage_from_session(path):
    """Read ``tokenUsage`` from a Droid session settings file.

    Returns ``{"input": ..., "cached": ..., "output": ..., "reasoning": ...}``.
    Field names are the 1.1.0 ones (``cacheReadTokens``, ``thinkingTokens``),
    falling back to the 1.0.0 names (``cacheReadInputTokens``, ``reasoningTokens``).
    ``input`` is total input (``inputTokens + cached``), ``output`` is
    ``outputTokens``. Any absent or invalid field is ``None``, never zero.
    """
    with open(path, encoding="utf-8") as source:
        data = json.load(source)
    usage = data.get("tokenUsage") if isinstance(data, dict) else None
    usage = usage if isinstance(usage, dict) else {}
    input_tokens = _non_negative_int(usage.get("inputTokens"))
    cached = _first_int(usage, "cacheReadTokens", "cacheReadInputTokens")
    output = _non_negative_int(usage.get("outputTokens"))
    reasoning = _first_int(usage, "thinkingTokens", "reasoningTokens")
    total = input_tokens + cached if input_tokens is not None and cached is not None else None
    return {"input": total, "cached": cached, "output": output, "reasoning": reasoning}


def _count_from_line(line):
    """Extract the numeric ``count`` (uncached prompt size) from a log line.

    Droid log lines carry a JSON context object; ``count`` is read from it when
    present, with a loose ``"count": N`` / ``count=N`` token scan as a fallback.
    Returns ``None`` when the line carries no usable count.
    """
    for token in re.findall(r"\{[^{}]*\}", line):
        try:
            obj = json.loads(token)
        except ValueError:
            continue
        if isinstance(obj, dict) and isinstance(obj.get("count"), int):
            return obj["count"]
    match = re.search(r'"count"\s*:\s*(-?\d+)|(?:^|[\s|,])count\s*=\s*(-?\d+)', line)
    if match:
        value = next((group for group in match.groups() if group is not None), None)
        return int(value)
    return None


def _line_has_session(line, session_id):
    """Exact session match: the JSON context's ``sessionId`` when present,
    otherwise the id as a whole token (not a prefix of a longer id)."""
    found_context = False
    for token in re.findall(r"\{[^{}]*\}", line):
        try:
            obj = json.loads(token)
        except ValueError:
            continue
        if isinstance(obj, dict) and "sessionId" in obj:
            found_context = True
            if obj["sessionId"] == session_id:
                return True
    if found_context:
        return False
    return re.search(r"(?<![\w-])" + re.escape(session_id) + r"(?![\w-])", line) is not None


def droid_request_count(log_text, session_id):
    """Count ``Streaming result`` lines for a session and sum their sizes.

    Returns ``{"requests": count, "uncached_input": sum}`` or ``None`` when no
    ``Streaming result`` line matches ``session_id``. A matching line without a
    numeric ``count`` still counts as one request but leaves ``uncached_input``
    ``None`` (its size is unknown), never zero.
    """
    sizes = []
    for line in log_text.splitlines():
        if "Streaming result" not in line or not _line_has_session(line, session_id):
            continue
        sizes.append(_count_from_line(line))
    if not sizes:
        return None
    valid = [_non_negative_int(value) for value in sizes]
    total = sum(valid) if all(value is not None for value in valid) else None
    return {"requests": len(valid), "uncached_input": total}


def rederive(runs_dir, home=None):
    """Recompute Droid usage fields for existing rows from their session files (read-only).

    The session is the row's recorded ``droid_session_id`` when present;
    otherwise the one session id under the Droid sessions root that appears in
    the run's saved ``<harness>/<task>-r<rep>.stdout.jsonl``. Zero or several
    matches give an ``error`` and no values. ``results.jsonl`` is never written.
    """
    runs = Path(runs_dir)
    stems = {Path(p).name[:-len(".settings.json")]: p for p in droid_session_files(home)}
    log = droid_log_text(home)
    out = []
    for line in (runs / "results.jsonl").read_text(encoding="utf-8").splitlines():
        if not line.strip():
            continue
        row = json.loads(line)
        if row.get("harness") not in ("droid", "droid-direct", "droid-script"):
            continue
        item = {"harness": row["harness"], "task": row.get("task"), "rep": row.get("rep"), "error": None}
        sid = row.get("droid_session_id")
        if not sid:
            transcript = runs / row["harness"] / f"{row.get('task')}-r{row.get('rep')}.stdout.jsonl"
            text = transcript.read_text(encoding="utf-8", errors="replace") if transcript.is_file() else ""
            found = sorted(stem for stem in stems if stem in text)
            if len(found) != 1:
                item["error"] = f"expected exactly one session id in the saved stream, found {len(found)}"
                out.append(item)
                continue
            sid = found[0]
        if sid not in stems:
            item["error"] = "session settings file not found: " + sid
            out.append(item)
            continue
        usage = droid_usage_from_session(stems[sid])
        fields = droid_session_fields(stems[sid])
        counted = droid_request_count(log, sid)
        requested = row.get("droid_tool_execution_mode")
        resolved = fields["mode"]
        item.update(
            session_id=sid, input=usage["input"], cached=usage["cached"], output=usage["output"],
            reasoning=usage["reasoning"],
            requests=counted["requests"] if counted else None,
            droid_log_uncached_input=counted["uncached_input"] if counted else None,
            droid_resolved_tool_execution_mode=resolved,
            droid_mode_mismatch=(None if resolved is None or requested is None
                                 else normalize_mode(resolved) != normalize_mode(requested)),
            droid_session_reasoning_effort=fields["reasoning_effort"],
            droid_session_model=fields["model"],
            row={"input_tokens": row.get("input_tokens"), "cached_tokens": row.get("cached_tokens"),
                 "output_tokens": row.get("output_tokens"), "requests": row.get("requests"),
                 "usage_available": row.get("usage_available")})
        out.append(item)
    return out


def main(argv=None):
    ap = argparse.ArgumentParser(description="Droid session helpers")
    sub = ap.add_subparsers(dest="command", required=True)
    cmd = sub.add_parser("rederive", help="print usage recomputed from preserved session files (read-only)")
    cmd.add_argument("runs_dir")
    cmd.add_argument("--home", help="Droid data root (default BENCH_DROID_HOME or ~/.factory)")
    args = ap.parse_args(argv)
    for item in rederive(args.runs_dir, args.home):
        print(json.dumps(item))
    return 0


if __name__ == "__main__":
    sys.exit(main())
