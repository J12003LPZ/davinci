#!/usr/bin/env python3
"""Coverage ledger for the Claude Code 2.1.316 parity work (task P04).

``build_coverage(evidence, cases, owners, results=None, out_dir=None) -> Coverage`` lists
every reference item DaVinci has to answer for, with its owner and DaVinci's state. It is
a report: every row starts ``NOT_STARTED`` (features, goldens) or ``UNCLASSIFIED``
(commands, keybindings), and only a result file that carries a passing result and an
evidence path can move a row to ``VERIFIED``.

* ``evidence``: the reference package root (EVIDENCE_MATRIX.json, COMMANDS.md,
  KEYBINDINGS.md, artifacts/frames/reference/index.json).
* ``cases``: the reference ``test_results.json`` (path or loaded dict).
* ``owners``: ``cc2316_owners.json`` (path or loaded dict). Committed; IDs and DaVinci facts only.
* ``results``: optional file/dict mapping a row ID to
  ``{"state", "result", "evidence", "limitation"}``.

The ledger holds Claude-derived data, so it is written only to ``CC2316_EVIDENCE`` (default
``C:/Users/sergi/Desktop/davinci-cc2316-evidence``) and never inside the repository.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import sys
from collections import Counter
from dataclasses import dataclass, field
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_EVIDENCE = "C:/Users/sergi/Desktop/davinci-cc2316-evidence"
DEFAULT_REF = "C:/Users/sergi/Desktop/lean-claude-ui"
TIERS = ("exact", "structure", "exception")
WEAK = ("EXPECTED_FAIL", "BLOCKED", "UNVERIFIED", "PARTIAL", "SOURCE_SUPPORTED")
STATES = ("NOT_STARTED", "UNCLASSIFIED", "IN_PROGRESS", "VERIFIED", "FAILED", "BLOCKED",
          "NO_REFERENCE", "SAME", "EQUIVALENT", "DROPPED")
UNASSIGNED = "UNASSIGNED"
EXCEPTION_REASON = re.compile(r"^D[123]: ")
COMMAND_ROW = re.compile(r"^\| (`/[^|]*?)\s*\|")
KEY_ROW = re.compile(r"^\| `(.+?)` \| `(.+?)` \| ([A-Z_]+) \|\s*$")


@dataclass
class Coverage:
    rows: list = field(default_factory=list)
    errors: list = field(default_factory=list)
    unassigned: list = field(default_factory=list)
    counts: dict = field(default_factory=dict)

    @property
    def ok(self) -> bool:
        return not self.errors


def ref_root() -> Path:
    return Path(os.environ.get("CC2316_REF") or DEFAULT_REF)


def evidence_dir() -> Path:
    return Path(os.environ.get("CC2316_EVIDENCE") or DEFAULT_EVIDENCE)


def inside_repo(path) -> bool:
    return Path(path).resolve().is_relative_to(REPO_ROOT)


def _load(value):
    if value is None or isinstance(value, (dict, list)):
        return value
    return json.loads(Path(value).read_text(encoding="utf-8"))


def expand_test_ids(text: str) -> list[str]:
    """``ONBOARD-01/02`` -> two IDs, ``HELP-01..03`` -> three, ``A-1/B-2`` -> two."""
    ids, prefix = [], ""
    for part in str(text or "").split("/"):
        part = part.strip()
        if not part or part == "none":
            continue
        m = re.fullmatch(r"([A-Z0-9]+(?:-[A-Z]+)*-)?(\d+)(?:\.\.(\d+))?", part)
        if m and m.group(1):
            prefix = m.group(1)
        if m and (m.group(1) or prefix):
            lo, hi = int(m.group(2)), int(m.group(3) or m.group(2))
            width = len(m.group(2))
            ids += [f"{m.group(1) or prefix}{n:0{width}d}" for n in range(lo, hi + 1)]
        else:
            ids.append(part)
    return ids


def _commands(path: Path) -> list[str]:
    seen: Counter = Counter()
    out = []
    for line in path.read_text(encoding="utf-8").splitlines():
        m = COMMAND_ROW.match(line)
        if m:
            name = m.group(1).replace("`", "")
            seen[name] += 1
            out.append(f"cmd:{name}" + (f"#{seen[name]}" if seen[name] > 1 else ""))
    return out


def _keybindings(path: Path) -> list[tuple[str, str]]:
    context, out = "", []
    for line in path.read_text(encoding="utf-8").splitlines():
        if line.startswith("## "):
            context = line[3:].strip()
            continue
        m = KEY_ROW.match(line)
        if m:
            out.append((f"key:{context}:{m.group(1)}", m.group(3)))
    return out


def _check_owner(kind: str, key: str, entry, errors: list):
    where = f"{kind} owner {key}"
    if not isinstance(entry, dict) or not isinstance(entry.get("owner_task"), str):
        errors.append(f"{where}: needs an owner_task")
        return
    if entry.get("tier") not in TIERS:
        errors.append(f"{where}: tier must be one of {', '.join(TIERS)}")
    if entry.get("tier") == "exception" and not EXCEPTION_REASON.match(str(entry.get("reason", ""))):
        errors.append(f"{where}: an exception needs a reason citing D1, D2 or D3")
    if not isinstance(entry.get("source_owners", []), list):
        errors.append(f"{where}: source_owners must be a list")


def _row(rid, kind, label, status, owner, golden, results, errors):
    r = {"id": rid, "kind": kind, "label": label, "reference_status": status,
         "owner_task": owner.get("owner_task", UNASSIGNED) if owner else UNASSIGNED,
         "source_owner": list(owner.get("source_owners", [])) if owner else [],
         "tier": owner.get("tier", "") if owner else "", "golden": golden,
         "state": "UNCLASSIFIED" if kind in ("command", "keybinding") else "NOT_STARTED",
         "limitation": "", "evidence": ""}
    notes = []
    if r["tier"] == "exception":
        notes.append(owner.get("reason", ""))
    if status in WEAK:
        notes.append(f"Reference status {status}: parity cannot be claimed beyond what is exercised.")
    if owner and owner.get("limitation"):
        notes.append(str(owner["limitation"]))
    res = results.get(rid)
    if res is not None:
        state = res.get("state")
        if state not in STATES:
            errors.append(f"result {rid}: state {state!r} is not one of {', '.join(STATES)}")
        else:
            r["state"] = state
        r["evidence"] = str(res.get("evidence") or "")
        if state == "VERIFIED":
            if res.get("result") != "PASS" or not r["evidence"]:
                errors.append(f"{rid}: VERIFIED needs a result of PASS and an evidence path")
            if status in WEAK and not str(res.get("limitation") or "").strip():
                errors.append(f"{rid}: reference status {status} marked VERIFIED needs a limitation")
        if res.get("limitation"):
            notes.append(str(res["limitation"]))
    r["limitation"] = " ".join(n for n in notes if n) if r["tier"] == "exception" else (
        notes[-1] if res and res.get("limitation") else " ".join(notes))
    return r


def build_coverage(evidence, cases, owners, results=None, out_dir=None) -> Coverage:
    root = Path(evidence)
    matrix = json.loads((root / "EVIDENCE_MATRIX.json").read_text(encoding="utf-8"))
    index = json.loads((root / "artifacts/frames/reference/index.json").read_text(encoding="utf-8"))
    commands = _commands(root / "COMMANDS.md")
    keys = _keybindings(root / "KEYBINDINGS.md")
    tests = (_load(cases) or {}).get("results", [])
    owners, results = _load(owners) or {}, _load(results) or {}

    cov = Coverage()
    features = {f["id"]: f for f in matrix["features"]}
    goldens = [e["name"] for e in index]
    by_test: dict[str, list[str]] = {}
    golden_status: dict[str, str] = {}
    for t in tests:
        name = f"{t.get('test')}__{t.get('snapshot')}"
        if name in goldens:
            by_test.setdefault(t["id"], []).append(name)
            golden_status[name] = t.get("status", "")

    for kind, table, known in (("feature", owners.get("features", {}), set(features)),
                               ("golden", owners.get("goldens", {}), set(goldens))):
        for key, entry in table.items():
            if key not in known:
                cov.errors.append(f"{kind} owner {key} does not exist in the reference")
            else:
                _check_owner(kind, key, entry, cov.errors)
    defaults = owners.get("defaults", {})

    def linked(fid, test):
        out = []
        for tid in expand_test_ids(test) or [fid]:
            for known, names in by_test.items():
                if known == tid or re.fullmatch(re.escape(tid) + r"[a-z]+", known):
                    out += [n for n in names if n not in out]
        return out

    rows = cov.rows
    for fid, f in features.items():
        rows.append(_row(fid, "feature", f.get("feature", ""), f.get("status", ""),
                         owners.get("features", {}).get(fid), linked(fid, f.get("test")), results, cov.errors))
    for name in goldens:
        rows.append(_row(name, "golden", name, golden_status.get(name) or "FRAME_ONLY",
                         owners.get("goldens", {}).get(name), [name], results, cov.errors))
    for rid in commands:
        rows.append(_row(rid, "command", rid[4:], "LISTED", defaults.get("commands"), [], results, cov.errors))
    for rid, status in keys:
        rows.append(_row(rid, "keybinding", rid[4:], status, defaults.get("keybindings"), [], results,
                         cov.errors))

    ids = {r["id"] for r in rows}
    for rid in results:
        if rid not in ids:
            cov.errors.append(f"result {rid} has no row in the ledger")
    cov.unassigned = [r["id"] for r in rows if r["owner_task"] == UNASSIGNED]
    cov.counts = {"kinds": dict(Counter(r["kind"] for r in rows)),
                  "states": dict(Counter(r["state"] for r in rows)),
                  "tiers": dict(Counter(r["tier"] or "none" for r in rows)),
                  "unassigned": len(cov.unassigned)}
    if out_dir is not None:
        write_coverage(cov, out_dir)
    return cov


def write_coverage(cov: Coverage, out_dir) -> None:
    out = Path(out_dir)
    if inside_repo(out):
        raise ValueError(f"{out} is inside the repository; the ledger holds Claude-derived data")
    out.mkdir(parents=True, exist_ok=True)
    (out / "coverage.json").write_text(json.dumps({
        "counts": cov.counts, "errors": cov.errors, "unassigned": cov.unassigned, "rows": cov.rows},
        indent=1, ensure_ascii=False), encoding="utf-8")
    cell = lambda v: str(v).replace("|", "\\|").replace("\n", " ")  # noqa: E731
    lines = ["# Coverage ledger", "", "A report only. No row is passing unless a result file says so.", "",
             f"Rows: {cov.counts['kinds']}", f"States: {cov.counts['states']}",
             f"Unassigned: {cov.unassigned or 'none'}", f"Errors: {len(cov.errors)}", ""]
    lines += [f"- {e}" for e in cov.errors]
    lines += ["", "| ID | Kind | Reference | Owner | Source owner | Tier | Golden | DaVinci | Limitation |",
              "|---|---|---|---|---|---|---|---|---|"]
    for r in cov.rows:
        lines.append("| " + " | ".join(cell(x) for x in (
            r["id"], r["kind"], r["reference_status"], r["owner_task"], ", ".join(r["source_owner"]),
            r["tier"], ", ".join(r["golden"]), r["state"], r["limitation"])) + " |")
    (out / "coverage.md").write_text("\n".join(lines) + "\n", encoding="utf-8")


def main(argv=None) -> int:
    here = Path(__file__).resolve().parent
    p = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    p.add_argument("--ref", default=str(ref_root()))
    p.add_argument("--owners", default=str(here / "cc2316_owners.json"))
    p.add_argument("--results")
    p.add_argument("--out", default=str(evidence_dir()))
    a = p.parse_args(argv)
    ref = Path(a.ref)
    try:
        cov = build_coverage(ref, ref / "artifacts/data/test_results.json", a.owners, a.results,
                             out_dir=a.out)
    except (OSError, ValueError) as exc:
        print(f"error: {exc}", file=sys.stderr)
        return 2
    print(f"rows {cov.counts['kinds']}")
    print(f"states {cov.counts['states']}")
    print(f"tiers {cov.counts['tiers']}")
    print(f"unassigned {cov.unassigned}")
    for e in cov.errors:
        print(f"ERROR {e}")
    print(f"written to {a.out}")
    return 0 if cov.ok else 1


if __name__ == "__main__":
    sys.exit(main())
