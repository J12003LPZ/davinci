#!/usr/bin/env python3
"""Join Jev shadow samples with what the coding model actually did.

Reads `<session>.events.jsonl` files. Each `jev` row marks one submitted turn
(`turnTs`); the `tool` rows between that turn and the next `jev` row are the
tools the coding model ran for it. For each capability question, the report
counts how often Jev's answer agreed with actual use.

"Used" is a proxy label: the model not calling a tool does not prove the tool
would not have helped. Treat missed/false rates as a disagreement signal for
the rollout review, not as ground truth.

Usage:
  python scripts/jev-shadow-report.py [EVENTS.jsonl ...] [--threshold 0.85]
  python scripts/jev-shadow-report.py --self-test

With no paths, scans the DaVinci session directories under the home folder.
"""

import argparse
import json
import sys
from collections import Counter, defaultdict
from pathlib import Path

# Question id -> tool-name prefixes/names of the capability it asks about.
# Mirrors the TOOL_NAMES constants in crates/davinci-coding-agent/src/native_extensions.
CAPABILITY_TOOLS = {
    "browser_relevant": ("browser_",),
    "git_history_relevant": ("git_",),
    "package_intelligence_relevant": ("package_",),
    "test_impact_relevant": ("test_related", "test_impacted", "test_plan"),
    "change_impact_relevant": ("impact_analyze",),
    "verification_planner_relevant": ("verification_plan",),
}


def uses(tool, question):
    return any(tool == name or (name.endswith("_") and tool.startswith(name))
               for name in CAPABILITY_TOOLS[question])


def turns(rows):
    """Yield (jev_row, [tool names]) per turn, ordered by turn start."""
    marks = sorted((r for r in rows if r.get("kind") == "jev"), key=lambda r: r["turnTs"])
    tools = sorted((r for r in rows if r.get("kind") == "tool"), key=lambda r: r["ts"])
    for index, mark in enumerate(marks):
        end = marks[index + 1]["turnTs"] if index + 1 < len(marks) else float("inf")
        yield mark, [t["tool"] for t in tools if mark["turnTs"] <= t["ts"] < end]


def report(files, threshold):
    outcomes = Counter()
    matrix = defaultdict(Counter)  # question -> tp/fp/fn/tn
    latency, input_tokens, samples = [], 0, 0
    for path in files:
        rows = []
        for line in Path(path).read_text(encoding="utf-8").splitlines():
            try:
                rows.append(json.loads(line))
            except json.JSONDecodeError:
                continue
        for mark, used_tools in turns(rows):
            outcomes[mark.get("outcome", "?")] += 1
            if mark.get("outcome") != "ok":
                continue
            samples += 1
            latency.append(mark.get("latencyMs", 0))
            input_tokens += mark.get("inputTokens") or 0
            for answer in mark.get("answers", []):
                question = answer.get("question_id")
                if question not in CAPABILITY_TOOLS or answer.get("value") is None:
                    continue
                said = answer["value"] >= threshold
                did = any(uses(tool, question) for tool in used_tools)
                matrix[question][("tp" if did else "fp") if said else ("fn" if did else "tn")] += 1
    return {
        "files": len(files),
        "turns": sum(outcomes.values()),
        "outcomes": dict(outcomes),
        "samples": samples,
        "latencyMsP50": sorted(latency)[len(latency) // 2] if latency else None,
        "inputTokensTotal": input_tokens,
        "threshold": threshold,
        "questions": {q: dict(c) for q, c in sorted(matrix.items())},
    }


def render(result):
    print(f"files={result['files']} turns={result['turns']} ok_samples={result['samples']} "
          f"outcomes={result['outcomes']} p50_latency_ms={result['latencyMsP50']} "
          f"input_tokens={result['inputTokensTotal']} threshold={result['threshold']}")
    print(f"{'question':32} {'said+used':>9} {'said only':>9} {'used only':>9} {'neither':>8} {'agree':>6}")
    for question, c in result["questions"].items():
        total = sum(c.values())
        agree = (c.get("tp", 0) + c.get("tn", 0)) / total if total else 0
        print(f"{question:32} {c.get('tp', 0):>9} {c.get('fp', 0):>9} {c.get('fn', 0):>9} "
              f"{c.get('tn', 0):>8} {agree:>6.0%}")


def default_files():
    home = Path.home()
    found = []
    for root in (home / ".davinci" / "agent", home / ".pi" / "agent"):
        if root.is_dir():
            found.extend(root.rglob("*.events.jsonl"))
    return found


def self_test():
    rows = [
        {"kind": "jev", "ts": 5, "turnTs": 0, "outcome": "ok", "latencyMs": 140, "inputTokens": 1500,
         "answers": [{"question_id": "browser_relevant", "value": 0.9},
                     {"question_id": "git_history_relevant", "value": 0.1}]},
        {"kind": "tool", "ts": 10, "tool": "browser_open"},
        {"kind": "tool", "ts": 11, "tool": "git_branch_diff"},
        {"kind": "jev", "ts": 25, "turnTs": 20, "outcome": "Busy", "answers": []},
        {"kind": "tool", "ts": 30, "tool": "browser_click"},  # busy turn: not credited to turn 0
    ]
    import tempfile
    with tempfile.TemporaryDirectory() as tmp:
        path = Path(tmp) / "s.events.jsonl"
        path.write_text("\n".join(json.dumps(r) for r in rows) + "\n", encoding="utf-8")
        result = report([path], 0.85)
    assert result["turns"] == 2 and result["samples"] == 1, result
    assert result["outcomes"] == {"ok": 1, "Busy": 1}, result
    assert result["questions"]["browser_relevant"] == {"tp": 1}, result
    assert result["questions"]["git_history_relevant"] == {"fn": 1}, result
    assert uses("git_symbol_history", "git_history_relevant")
    assert not uses("github_search", "git_history_relevant")
    print("self-test ok")


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("paths", nargs="*")
    parser.add_argument("--threshold", type=float, default=0.85,
                        help="Noul value counted as 'Jev says relevant' (policy ADDITIVE_THRESHOLD)")
    parser.add_argument("--json", action="store_true")
    parser.add_argument("--self-test", action="store_true")
    args = parser.parse_args()
    if args.self_test:
        self_test()
        return 0
    files = [Path(p) for p in args.paths] or default_files()
    result = report(files, args.threshold)
    print(json.dumps(result, indent=2)) if args.json else render(result)
    return 0


if __name__ == "__main__":
    sys.exit(main())
