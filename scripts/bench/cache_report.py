"""Explain prompt-cache misses from a DAVINCI_WIRE_DUMP directory.

    python scripts/bench/cache_report.py <dump-dir>

The dump holds, per provider request, `<seq>-<pid>-logical.json` (the full
logical request body), `<seq>-<pid>-wire.json` (the frame actually sent, when
the WebSocket transport was used) and `<seq>-<pid>-usage.json` (the usage the
provider reported). For each request, in order, this prints total and cached
input tokens, whether the wire frame continued a previous response
(`previous_response_id`), the prompt_cache_key, and the first place the
logical body stops extending the previous request's logical body.

An append-only conversation shows `break=-` on every row. Any other value
names the part that changed: `instructions`, `tools`, `shorter`, or
`input[i]` with the two differing items printed underneath.
"""
import glob
import json
import os
import sys


def load(path):
    with open(path, encoding="utf-8") as f:
        return json.load(f)


def first_break(prev, cur):
    if prev.get("instructions") != cur.get("instructions"):
        return "instructions", None
    if prev.get("tools") != cur.get("tools"):
        return "tools", None
    a, b = prev.get("input") or [], cur.get("input") or []
    if len(b) < len(a):
        return "shorter", None
    for i, (x, y) in enumerate(zip(a, b)):
        if x != y:
            return f"input[{i}]", (x, y)
    return None, None


def brief(item):
    kind = item.get("type") or item.get("role") or "?"
    text = json.dumps(item, ensure_ascii=False)
    return f"{kind}: {text[:200]}"


def main(folder):
    groups = {}
    for path in glob.glob(os.path.join(folder, "*-logical.json")):
        seq, pid, _ = os.path.basename(path).split("-", 2)
        groups.setdefault(pid, []).append((int(seq), path))
    if not groups:
        print(f"no *-logical.json files in {folder}")
        return 1
    for pid, items in sorted(groups.items()):
        print(f"process {pid}")
        prev = None
        total_all = cached_all = 0
        for seq, path in sorted(items):
            body = load(path)
            usage_path = path.replace("-logical.json", "-usage.json")
            wire_path = path.replace("-logical.json", "-wire.json")
            usage = load(usage_path) if os.path.exists(usage_path) else {}
            wire = load(wire_path) if os.path.exists(wire_path) else {}
            cached = usage.get("cacheRead", 0) or 0
            total = (usage.get("input", 0) or 0) + cached
            total_all += total
            cached_all += cached
            mode = "delta" if wire.get("previous_response_id") else ("full" if wire else "http")
            key = str(body.get("prompt_cache_key", "-"))
            where, pair = first_break(prev, body) if prev is not None else (None, None)
            print(f"  #{seq:04d} items={len(body.get('input') or []):3d} total={total:7,} "
                  f"cached={cached:7,} {mode:5} key={key[:20]:20} break={where or '-'}")
            if pair:
                print(f"         before: {brief(pair[0])}")
                print(f"         after:  {brief(pair[1])}")
            prev = body
        ratio = 100 * cached_all / total_all if total_all else 0
        print(f"  total input {total_all:,}, cached {cached_all:,} ({ratio:.0f}%)")
    return 0


if __name__ == "__main__":
    sys.stdout.reconfigure(encoding="utf-8")
    if len(sys.argv) != 2:
        print(__doc__)
        sys.exit(2)
    sys.exit(main(sys.argv[1]))
