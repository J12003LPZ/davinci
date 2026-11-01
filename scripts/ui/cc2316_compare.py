"""Diff styled terminal captures without concealing visual mismatches."""
from __future__ import annotations

STYLE = ("chars", "fg", "bg", "bold", "dim", "italic", "underline", "strike", "inverse", "blink")


def _cells(capture):
    if "cells" in capture:
        return capture["cells"]
    if "rows" in capture:
        return capture["rows"]
    return []


def compare(expected: dict, actual: dict) -> list[str]:
    differences: list[str] = []
    if expected.get("dimensions") != actual.get("dimensions"):
        differences.append(f"dimensions: {expected.get('dimensions')} != {actual.get('dimensions')}")
    for field in ("cursor", "modes"):
        left, right = expected.get(field, {}), actual.get(field, {})
        for key in sorted(set(left) | set(right)):
            if left.get(key) != right.get(key):
                differences.append(f"{field}.{key}: {left.get(key)!r} != {right.get(key)!r}")
    ea, aa = _cells(expected), _cells(actual)
    if len(ea) != len(aa):
        differences.append(f"rows count: {len(ea)} != {len(aa)}")
    for y, (erow, arow) in enumerate(zip(ea, aa)):
        if len(erow) != len(arow):
            differences.append(f"row {y}: run/cell count {len(erow)} != {len(arow)}")
        for x, (left, right) in enumerate(zip(erow, arow)):
            for key in ("text", "x", "w") if "text" in left or "text" in right else STYLE:
                if left.get(key) != right.get(key):
                    differences.append(f"row {y} item {x} {key}: {left.get(key)!r} != {right.get(key)!r}")
            if "text" in left or "text" in right:
                for key in STYLE[1:]:
                    if left.get(key) != right.get(key):
                        differences.append(f"row {y} item {x} {key}: {left.get(key)!r} != {right.get(key)!r}")
    return differences
