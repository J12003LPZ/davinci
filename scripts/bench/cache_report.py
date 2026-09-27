"""Explain prompt-cache misses from a DAVINCI_WIRE_DUMP directory.

    python scripts/bench/cache_report.py <dump-dir>

The dump holds, per provider request, `<seq>-<pid>-logical.json` (the full
logical request body), `<seq>-<pid>-wire.json` (the frame actually sent, when
the WebSocket transport was used) and `<seq>-<pid>-usage.json` (the usage the
provider reported). For each request, in order, this prints total and cached
input tokens, whether the wire frame continued a previous response
(`previous_response_id`), the prompt_cache_key, and the first place the
logical body stops extending the previous request's logical body.
Each row also includes stable digests for the complete logical/wire bodies
and the instructions, tools, and input components. The per-process summary
records whether the first request had zero cached input.

An append-only conversation shows `break=-` on every row. Any other value
names the part that changed: `instructions`, `tools`, `shorter`, or
`input[i]` with the two differing items printed underneath.
"""
import glob
import hashlib
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


def token_pair(usage):
    if not isinstance(usage, dict):
        return None, None
    values = [usage.get("input"), usage.get("cacheRead"), usage.get("cacheWrite", 0)]
    if any(type(value) is not int or value < 0 for value in values):
        return None, None
    return sum(values), values[1]


def digest(value):
    """Return a stable short digest for a complete JSON value."""
    encoded = json.dumps(
        value,
        ensure_ascii=False,
        sort_keys=True,
        separators=(",", ":"),
    ).encode("utf-8")
    return hashlib.sha256(encoded).hexdigest()[:16]


def request_fingerprints(body, wire=None):
    """Hash the complete request and cache-sensitive logical components."""
    return {
        "body": digest(body),
        "instructions": digest(body.get("instructions")),
        "tools": digest(body.get("tools")),
        "input": digest(body.get("input") or []),
        "wire": digest(wire) if wire else "-",
    }


def number(value):
    return "unavailable" if value is None else f"{value:,}"


def group_report(name, pairs):
    available = pairs and all(total is not None and cached is not None for total, cached in pairs)
    total = sum(pair[0] for pair in pairs) if available else None
    cached = sum(pair[1] for pair in pairs) if available else None
    ratio = f"{100 * cached / total:.1f}%" if total else "unavailable"
    print(f"  {name}: input {number(total)}, cached {number(cached)} ({ratio})")


def first_request_cache_state(pairs):
    if not pairs or pairs[0][1] is None:
        return "unavailable"
    return "yes" if pairs[0][1] == 0 else "no"


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
        pairs = []
        for seq, path in sorted(items):
            body = load(path)
            usage_path = path.replace("-logical.json", "-usage.json")
            wire_path = path.replace("-logical.json", "-wire.json")
            usage = load(usage_path) if os.path.exists(usage_path) else {}
            wire = load(wire_path) if os.path.exists(wire_path) else {}
            total, cached = token_pair(usage)
            pairs.append((total, cached))
            mode = "delta" if wire.get("previous_response_id") else ("full" if wire else "http")
            key = str(body.get("prompt_cache_key", "-"))
            fingerprints = request_fingerprints(body, wire)
            where, pair = first_break(prev, body) if prev is not None else (None, None)
            print(f"  #{seq:04d} items={len(body.get('input') or []):3d} total={number(total):>11} "
                  f"cached={number(cached):>11} {mode:5} key={key[:20]:20} break={where or '-'} "
                  f"body={fingerprints['body']} wire={fingerprints['wire']} "
                  f"instructions={fingerprints['instructions']} tools={fingerprints['tools']} "
                  f"input={fingerprints['input']}")
            if pair:
                print(f"         before: {brief(pair[0])}")
                print(f"         after:  {brief(pair[1])}")
            prev = body
        print(f"  first_request_zero_cached={first_request_cache_state(pairs)}")
        group_report("all", pairs)
        group_report("first", pairs[:1])
        group_report("later", pairs[1:])
    return 0


if __name__ == "__main__":
    sys.stdout.reconfigure(encoding="utf-8")
    if len(sys.argv) != 2:
        print(__doc__)
        sys.exit(2)
    sys.exit(main(sys.argv[1]))
