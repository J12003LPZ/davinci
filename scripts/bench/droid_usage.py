"""Droid usage parsing from session settings files and log text.

Totals come from ``~/.factory/sessions/*/<id>.settings.json`` (a ``tokenUsage``
object) and the request count from ``Streaming result`` log lines filtered by
session id. Both functions take text/JSON, never a live process, so they are
unit-testable offline.
"""
import json
import os
import re
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


def _non_negative_int(value):
    return value if type(value) is int and not isinstance(value, bool) and value >= 0 else None


def droid_usage_from_session(path):
    """Read ``tokenUsage`` from a Droid session settings file.

    Returns ``{"input": ..., "cached": ..., "output": ..., "reasoning": ...}``.
    ``cached`` is ``cacheReadInputTokens``; ``input`` is total input
    (``inputTokens + cacheReadInputTokens``); ``output`` is ``outputTokens``;
    ``reasoning`` is ``reasoningTokens``. Any absent or invalid field is
    ``None``, never zero.
    """
    with open(path, encoding="utf-8") as source:
        data = json.load(source)
    usage = data.get("tokenUsage")
    usage = usage if isinstance(usage, dict) else {}
    input_tokens = _non_negative_int(usage.get("inputTokens"))
    cached = _non_negative_int(usage.get("cacheReadInputTokens"))
    output = _non_negative_int(usage.get("outputTokens"))
    reasoning = _non_negative_int(usage.get("reasoningTokens"))
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
