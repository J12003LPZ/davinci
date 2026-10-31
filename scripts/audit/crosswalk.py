#!/usr/bin/env python3
"""Claude finding crosswalk: merge the triage parts, validate, repair.

Subcommands
  merge   concatenate crosswalk-parts/part*.csv into claude-finding-crosswalk.csv
          in evidence-register order.
  check   validate the merged CSV against the register and the repo; exit
          non-zero on any failure. Optional repairs run before validation:
            --apply-closures  rows closed on this branch (CLOSURES below)
            --fix-anchors     IMPLEMENTED_AND_TESTED rows whose test anchor does
                              not resolve are downgraded to EXISTS_UNVERIFIED

Python 3 standard library only. Deterministic: same inputs, same output.
"""
import argparse
import collections
import csv
import os
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
IMPL = ROOT / "docs" / "audit" / "claude-code-parity" / "implementation"
DEFAULT_PARTS = IMPL / "crosswalk-parts"
DEFAULT_MERGED = IMPL / "claude-finding-crosswalk.csv"
DEFAULT_REGISTER = Path(
    os.environ.get(
        "CROSSWALK_REGISTER",
        r"C:\Users\sergi\Desktop\lean-claude\re-claude-code\evidence-register.csv",
    )
)
DEFAULT_CRATES = ROOT / "crates"

HEADER = [
    "finding_id",
    "claude_report",
    "claude_class",
    "claude_build",
    "davinci_owner",
    "disposition",
    "gap_id",
    "plan_task",
    "test_anchor",
    "decision_reason",
]
DISPOSITIONS = [
    "IMPLEMENTED_AND_TESTED",
    "EXISTS_UNVERIFIED",
    "WEAK",
    "MISSING",
    "DIFFERENT_SAFE",
    "EXPERIMENTAL",
    "REJECTED_UNSAFE",
    "BLOCKED",
    "NOT_APPLICABLE",
]
IMPLEMENTED = "IMPLEMENTED_AND_TESTED"
UNVERIFIED = "EXISTS_UNVERIFIED"
OPEN_STATES = {"MISSING", "WEAK", UNVERIFIED}

# Rows closed on this branch (gap-closure-decisions.md). Reviewable on purpose:
# (finding_id, gap_id, test_anchor, commit subject / evidence for the reason).
# A row is closed only if it is currently MISSING / WEAK / EXISTS_UNVERIFIED for
# that gap AND the anchor resolves. Nothing else changes.
#
# Gaps in the closed set with no row below were looked at and left alone:
#   F-11-04 (G-13), F-42-07 (G-14), F-35-05 (X-09): BLOCKED, not open states.
#   F-32-04 (G-11): asks for a finite default memory budget; the queue cap and
#     wait timeout landed but admitted-concurrency defaults are unchanged.
#   F-34-05, F-34-08, F-50-04, F-50-07 (G-05): restart status, durable run id,
#     rejection propagation and an artifact acceptance predicate; the branch
#     closed stale-artifact reruns (source fingerprints), which is not those.
#   F-31-02, F-31-08 (X-07): same-server MCP overlap is not covered by B2.
#   G-01, G-03, G-09, G-10, G-13, G-14, G-15, G-16: no row in an open state.
CLOSURES = [
    (
        "F-06-06",
        "G-12",
        "crates/davinci-mcp/src/lib.rs::g12_the_invocation_id_travels_in_meta_and_never_in_the_arguments",
        "67769a7b fix(mcp): never replay a possibly-committed MCP call (G-01, G-12, G-14, G-16): "
        "the call id is sent as _meta[davinci/invocationId]",
    ),
    (
        "F-20-10",
        "G-12",
        "crates/davinci-mcp/src/lib.rs::g12_the_invocation_id_travels_in_meta_and_never_in_the_arguments",
        "67769a7b fix(mcp): never replay a possibly-committed MCP call (G-01, G-12, G-14, G-16): "
        "the call id is sent as _meta[davinci/invocationId]",
    ),
    (
        "F-07-05",
        "G-04",
        "crates/davinci-agent/src/compaction.rs::repeated_compaction_carries_forward_user_requirements",
        "4ba394c1 feat(agent): carry user requirements through compaction, read SKILL.md bodies "
        "lazily, fingerprint workflow artifacts (default Off path; summaries missing a user "
        "requirement are rejected)",
    ),
    (
        "F-13-03",
        "G-17",
        "crates/davinci-agent/src/skills.rs::invocation_reloads_body_after_file_change",
        "4ba394c1 feat(agent): carry user requirements through compaction, read SKILL.md bodies "
        "lazily, fingerprint workflow artifacts; 5748d80d frontmatter-only skill discovery",
    ),
    (
        "F-10-02",
        "X-07",
        "crates/davinci-agent/src/turn.rs::b2_reverse_completion_pairs_each_result_to_its_call_id_in_call_order",
        "Wave 5 B2 audit tests (turn.rs b2_*): reverse completion, denial, invalid input, "
        "cancel, mutation barrier and output cap keep call-id pairing",
    ),
    (
        "F-10-04",
        "X-07",
        "crates/davinci-agent/src/turn.rs::b2_reverse_completion_pairs_each_result_to_its_call_id_in_call_order",
        "Wave 5 B2 audit tests (turn.rs b2_*): reverse completion, denial, invalid input, "
        "cancel, mutation barrier and output cap keep call-id pairing",
    ),
    (
        "F-17-01",
        "X-07",
        "crates/davinci-agent/src/turn.rs::b2_reverse_completion_pairs_each_result_to_its_call_id_in_call_order",
        "Wave 5 B2 audit tests (turn.rs b2_*): reverse completion, denial, invalid input, "
        "cancel, mutation barrier and output cap keep call-id pairing",
    ),
]

_FN = re.compile(r"\bfn\s+([A-Za-z_][A-Za-z0-9_]*)")


class FnIndex:
    """`fn <name>` lookup over crates/**/*.rs, per file and repo-wide."""

    def __init__(self, repo_root, crates_dir):
        self.repo_root = Path(repo_root)
        self.crates_dir = Path(crates_dir)
        self._files = {}
        self._all = None

    def _names_in(self, path):
        if path not in self._files:
            try:
                text = path.read_text(encoding="utf-8", errors="replace")
            except OSError:
                text = ""
            self._files[path] = set(_FN.findall(text))
        return self._files[path]

    def _all_names(self):
        if self._all is None:
            names = set()
            for path in self.crates_dir.rglob("*.rs"):
                if "target" in path.parts:
                    continue
                names |= self._names_in(path)
            self._all = names
        return self._all

    def resolves(self, anchor):
        """True if one `;`-separated piece of `anchor` names a real test fn.

        `path::name` and `path::module::name` take the last `::` segment and
        require it in that file; a bare `name` may be anywhere under crates/.
        A `WRITE:` prefix marks a planned test that does not exist yet.
        """
        for piece in anchor.split(";"):
            piece = piece.strip()
            if not piece or piece.startswith("WRITE:"):
                continue
            segments = piece.split("::")
            name = segments[-1].strip()
            if not re.fullmatch(r"[A-Za-z_][A-Za-z0-9_]*", name):
                continue
            if len(segments) > 1 and segments[0].endswith(".rs"):
                file = self.repo_root / segments[0].strip()
                if file.is_file() and name in self._names_in(file):
                    return True
            elif len(segments) == 1 and name in self._all_names():
                return True
        return False


def read_rows(path):
    with open(path, encoding="utf-8-sig", newline="") as handle:
        reader = csv.DictReader(handle)
        return list(reader.fieldnames or []), list(reader)


def write_rows(path, rows):
    path = Path(path)
    path.parent.mkdir(parents=True, exist_ok=True)
    with open(path, "w", encoding="utf-8", newline="") as handle:
        writer = csv.DictWriter(handle, fieldnames=HEADER, lineterminator="\n")
        writer.writeheader()
        writer.writerows(rows)


def register_ids(path):
    with open(path, encoding="utf-8-sig", newline="") as handle:
        reader = csv.reader(handle)
        header = next(reader)
        if header[0] != "Finding ID":
            raise SystemExit(f"register first column is {header[0]!r}, expected 'Finding ID'")
        return [row[0] for row in reader if row]


def merge(parts_dir, out, register):
    rows = []
    for part in sorted(Path(parts_dir).glob("part*.csv")):
        header, part_rows = read_rows(part)
        if header != HEADER:
            raise SystemExit(f"{part}: header {header} != {HEADER}")
        rows.extend(part_rows)
    order = {fid: i for i, fid in enumerate(register_ids(register))}
    unknown = [r["finding_id"] for r in rows if r["finding_id"] not in order]
    if unknown:
        # Keep them (check reports them as extras) after the known rows.
        for offset, fid in enumerate(unknown):
            order.setdefault(fid, len(order) + offset)
    rows.sort(key=lambda r: order[r["finding_id"]])
    write_rows(out, rows)
    return len(rows)


def apply_closures(rows, index, closures=None):
    """Set IMPLEMENTED_AND_TESTED on rows the mapping closes. Returns (closed, notes)."""
    closed, notes = [], []
    by_id = {r["finding_id"]: r for r in rows}
    for finding, gap, anchor, reason in CLOSURES if closures is None else closures:
        row = by_id.get(finding)
        if row is None:
            notes.append(f"{finding}: not in the crosswalk")
        elif row["gap_id"] != gap:
            notes.append(f"{finding}: gap is {row['gap_id']!r}, mapping says {gap}")
        elif row["disposition"] == IMPLEMENTED and row["test_anchor"] == anchor:
            continue
        elif row["disposition"] not in OPEN_STATES:
            notes.append(f"{finding}: {row['disposition']} is not an open state, left alone")
        elif not index.resolves(anchor):
            notes.append(f"{finding}: anchor does not resolve, left alone: {anchor}")
        else:
            row["disposition"] = IMPLEMENTED
            row["test_anchor"] = anchor
            row["decision_reason"] = f"Closed on this branch ({gap}): {reason}"
            closed.append(finding)
    return closed, notes


def fix_anchors(rows, index):
    """Downgrade IMPLEMENTED_AND_TESTED rows whose anchor is empty or unresolved."""
    downgraded = []
    for row in rows:
        if row["disposition"] != IMPLEMENTED:
            continue
        anchor = row["test_anchor"].strip()
        if anchor and index.resolves(anchor):
            continue
        row["disposition"] = UNVERIFIED
        note = f"anchor not found: {anchor}" if anchor else "anchor not found: (empty)"
        row["decision_reason"] = f"{row['decision_reason']} {note}".strip()
        downgraded.append(row["finding_id"])
    return downgraded


def validate(rows, header, register, index):
    problems = []
    if header != HEADER:
        problems.append(f"header is {header}, expected {HEADER}")
    ids = [r["finding_id"] for r in rows]
    seen = collections.Counter(ids)
    for fid, count in seen.items():
        if count > 1:
            problems.append(f"duplicate finding id {fid} ({count} rows)")
    expected = set(register)
    for fid in register:
        if fid not in seen:
            problems.append(f"missing finding id {fid}")
    for fid in seen:
        if fid not in expected:
            problems.append(f"extra finding id {fid} (not in the register)")
    if not problems and ids != register:
        first = next(i for i, (a, b) in enumerate(zip(ids, register)) if a != b)
        problems.append(f"order differs from the register at row {first + 1}: {ids[first]} vs {register[first]}")
    for row in rows:
        fid = row["finding_id"]
        if row["disposition"] not in DISPOSITIONS:
            problems.append(f"{fid}: bad disposition {row['disposition']!r}")
        elif row["disposition"] == IMPLEMENTED:
            anchor = row["test_anchor"].strip()
            if not anchor:
                problems.append(f"{fid}: {IMPLEMENTED} with an empty test anchor")
            elif not index.resolves(anchor):
                problems.append(f"{fid}: test anchor does not resolve: {anchor}")
    return problems


def cmd_merge(args):
    count = merge(args.parts, args.out, args.register)
    print(f"merged {count} rows -> {args.out}")
    return 0


def cmd_check(args):
    header, rows = read_rows(args.csv)
    index = FnIndex(ROOT, args.crates)
    changed = False
    if args.apply_closures:
        closed, notes = apply_closures(rows, index)
        changed |= bool(closed)
        print(f"closures applied: {len(closed)} {closed}")
        for note in notes:
            print(f"  note: {note}")
    if args.fix_anchors:
        downgraded = fix_anchors(rows, index)
        changed |= bool(downgraded)
        print(f"anchors downgraded: {len(downgraded)} {downgraded}")
    if changed:
        write_rows(args.csv, rows)
    problems = validate(rows, header, register_ids(args.register), index)
    counts = collections.Counter(r["disposition"] for r in rows)
    print(f"rows: {len(rows)}")
    for name in DISPOSITIONS:
        print(f"  {name}: {counts.get(name, 0)}")
    if problems:
        print(f"FAIL: {len(problems)} problem(s)")
        for problem in problems:
            print(f"  - {problem}")
        return 1
    print("OK")
    return 0


def main(argv=None):
    parser = argparse.ArgumentParser(description=(__doc__ or "").splitlines()[0])
    sub = parser.add_subparsers(dest="command", required=True)
    m = sub.add_parser("merge", help="concatenate the parts in register order")
    m.add_argument("--parts", type=Path, default=DEFAULT_PARTS)
    m.add_argument("--out", type=Path, default=DEFAULT_MERGED)
    m.add_argument("--register", type=Path, default=DEFAULT_REGISTER)
    m.set_defaults(func=cmd_merge)
    c = sub.add_parser("check", help="validate the merged CSV; non-zero exit on failure")
    c.add_argument("--csv", type=Path, default=DEFAULT_MERGED)
    c.add_argument("--register", type=Path, default=DEFAULT_REGISTER)
    c.add_argument("--crates", type=Path, default=DEFAULT_CRATES)
    c.add_argument("--apply-closures", action="store_true")
    c.add_argument("--fix-anchors", action="store_true")
    c.set_defaults(func=cmd_check)
    args = parser.parse_args(argv)
    return args.func(args)


if __name__ == "__main__":
    sys.exit(main())
