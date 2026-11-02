"""Diff styled terminal captures (claude-tui-styled-capture/2) cell by cell.

Runs are expanded into a full grid of ``dimensions`` so two captures that split
the same content into different runs compare equal. Nothing is truncated: a run
past the grid edge, a row beyond ``dimensions``, overlapping runs and a run whose
``w`` disagrees with its text are all reported differences.

Region rules (see the plan, "Region rules, not cell masks") are boxes in the
expected golden. Region coordinates are inclusive: ``{"rows": (r0, r1),
"cols": (c0, c1)}``. Cells outside every region are always compared exactly.

* ``mark``: cells inside the box are not compared at all. Nothing else is
  checked specially: a mark that spills out of its box leaves cells outside the
  box that differ from the golden, and those are compared normally and fail.
* ``identity``: the expected cells in the box are rewritten with the rule's
  substitution table (the replacement takes the style of the first replaced
  cell), re-padded with blanks in the style of the box's last expected cell, and
  then compared with text and style. A replacement wider than the box, a box that
  splits a wide glyph, and a substitution that matches nothing are failures.
* ``data``: text is ignored inside the box; style is compared.
* ``structure``: like ``data``, and the wide-glyph geometry (which columns are the
  second half of a wide glyph) is compared too.

What is compared outside the grid, and how:

* ``dimensions``: exactly.
* ``modes``: only the state fields in ``MODE_FIELDS``. A field missing from one side
  is a difference; every other mode key is ignored. The event counters (``sync_frames``,
  ``cursor_hide``, ``cursor_show``, ``alternate_screen_enters``/``_exits``,
  ``kitty_keyboard_*``, ``modify_other_keys``, ``cursor_shapes``) depend on how many
  repaints happened. ``bracketed_paste`` and ``focus_events`` are left out too: the
  golden tool parses modes only from the bytes after the last ``?1049h``, but a real
  recording enables ``?2004h`` and ``?1004h`` before it, so the goldens say false while
  the feature works. Task P05 checks them from the full raw recording.
* ``cursor``: ``hidden`` always; ``x`` and ``y`` only when the expected or the actual
  cursor is visible.
* ``buffer``: ``type`` only.
* Colours: a cell no run covers, and a run field without ``fg``/``bg``, take the
  capture's own ``defaults`` (``None`` when the capture has none). A text cell in a
  non-default colour therefore still differs from a default one.
* Invisible style: a cell whose text is one space with no underline, strike or inverse
  shows only its ``bg``, so when both sides are such a blank ``fg``, ``bold``, ``dim``,
  ``italic`` and ``blink`` are not compared. If either side is a text cell (or a blank
  with underline, strike or inverse) the full style is compared, so a text cell never
  gains a spurious pass. This applies after any ``identity`` rewrite and inside ``data`` and
  ``structure`` regions.

Deliberately not compared: ``links``, ``sgr_counts``, ``target``, ``source``, mode
counters, cursor ``shape``/``attrs`` (every golden says ``unknown``; the plan records
cursor shape as unverified) and buffer ``length``/``baseY``/``viewportY``.
"""
from __future__ import annotations

import json
import unicodedata
from dataclasses import dataclass, field
from pathlib import Path

SCHEMA = "claude-tui-styled-capture/2"
STYLE_FIELDS = ("fg", "bg", "bold", "dim", "italic", "underline", "strike", "inverse", "blink")
MODE_FIELDS = ("alternate_screen", "mouse_click", "mouse_drag", "mouse_move", "mouse_sgr",
               "synchronized_output")
_STYLE_DEFAULTS = {"fg": None, "bg": None, "bold": False, "dim": False, "italic": False,
                   "underline": False, "strike": False, "inverse": False, "blink": False}
KINDS = ("mark", "identity", "data", "structure")
_MISSING = object()
_CONT = ""  # text of the second column of a wide glyph


@dataclass(frozen=True)
class Difference:
    row: int | None
    column: int | None
    field: str
    expected: object
    actual: object


@dataclass
class DiffReport:
    differences: list[Difference] = field(default_factory=list)

    @property
    def ok(self) -> bool:
        return not self.differences

    def __str__(self) -> str:
        if self.ok:
            return "no differences"
        return "\n".join(f"row {d.row} col {d.column} {d.field}: {d.expected!r} != {d.actual!r}"
                         for d in self.differences)


@dataclass(frozen=True)
class Rule:
    golden: str
    region: object
    kind: str
    reason: str
    substitutions: dict | None = None
    row0: int = field(init=False)
    row1: int = field(init=False)
    col0: int = field(init=False)
    col1: int = field(init=False)

    def __post_init__(self):
        if self.kind not in KINDS:
            raise ValueError(f"unknown rule kind {self.kind!r}")
        if not isinstance(self.reason, str) or not self.reason.strip():
            raise ValueError("a rule needs a non-empty reason")
        try:
            (r0, r1), (c0, c1) = self.region["rows"], self.region["cols"]  # type: ignore[index]
        except (KeyError, TypeError, ValueError) as exc:
            raise ValueError(f"region must be {{'rows': (r0, r1), 'cols': (c0, c1)}}: {exc!r}") from exc
        if r0 > r1 or c0 > c1:
            raise ValueError(f"empty region {self.region!r}")
        for name, value in (("row0", r0), ("row1", r1), ("col0", c0), ("col1", c1)):
            object.__setattr__(self, name, value)
        if self.kind == "identity":
            if not self.substitutions or any(not k for k in self.substitutions):
                raise ValueError("an identity rule needs a substitution table with non-empty keys")
        elif self.substitutions is not None:
            raise ValueError("only identity rules take substitutions")

    def contains(self, row: int, col: int) -> bool:
        return self.row0 <= row <= self.row1 and self.col0 <= col <= self.col1

    def overlaps(self, other: "Rule") -> bool:
        return (self.row0 <= other.row1 and other.row0 <= self.row1
                and self.col0 <= other.col1 and other.col0 <= self.col1)


def load_capture(path) -> dict:
    """Load one golden's JSON file. index.json and other non-captures raise ValueError."""
    data = json.loads(Path(path).read_text(encoding="utf-8"))
    if not isinstance(data, dict) or data.get("schema") != SCHEMA or "rows" not in data:
        raise ValueError(f"{path} is not a {SCHEMA} capture")
    return data


# Wide ranges of xterm.js's Unicode 6 width table, the model xterm_dump.js replays
# with. East Asian Width would call emoji such as U+1F642 wide; the goldens store
# them one column wide.
_WIDE = ((0x1100, 0x115F), (0x2329, 0x232A), (0x2E80, 0x303E), (0x3040, 0xA4CF),
         (0xAC00, 0xD7A3), (0xF900, 0xFAFF), (0xFE10, 0xFE19), (0xFE30, 0xFE6F),
         (0xFF00, 0xFF60), (0xFFE0, 0xFFE6), (0x20000, 0x2FFFD), (0x30000, 0x3FFFD))


def _width(ch: str) -> int:
    if unicodedata.combining(ch) or unicodedata.category(ch) in ("Mn", "Me", "Cf"):
        return 0
    code = ord(ch)
    return 2 if any(lo <= code <= hi for lo, hi in _WIDE) else 1


def _defaults(capture: dict) -> dict:
    found = capture.get("defaults") or {}
    return {**_STYLE_DEFAULTS, "fg": found.get("fg"), "bg": found.get("bg")}


def _style(item: dict, defaults: dict) -> tuple:
    return tuple(item.get(k, defaults[k]) for k in STYLE_FIELDS)


def _blank(capture: dict) -> tuple:
    return (" ", _style({}, _defaults(capture)))


def _text_cells(text: str, style: tuple):
    """Split text into cells. Returns (cells, leading): ``leading`` is any
    zero-width text before the first base character, which belongs to the cell
    before this text."""
    cells: list[tuple] = []
    leading = ""
    for ch in text:
        w = _width(ch)
        if w == 0:
            if cells:
                last = len(cells) - 1
                if cells[last][0] == _CONT and last > 0:
                    last -= 1
                cells[last] = (cells[last][0] + ch, style)
            else:
                leading += ch
        else:
            cells.append((ch, style))
            if w == 2:
                cells.append((_CONT, style))
    return cells, leading


def _grid(capture: dict, side: str, out: list):
    """Expand runs into {(row, col): (text, style)} over dimensions. Layout problems
    go to ``out`` as Differences with expected/actual describing the bad side."""
    dims = capture.get("dimensions") or {}
    ncols, nrows = dims.get("cols", 0), dims.get("rows", 0)

    def bad(row, col, message):
        out.append(Difference(row, col, "layout", message if side == "expected" else None,
                              message if side == "actual" else None))

    defaults = _defaults(capture)
    cells: dict[tuple, tuple] = {}
    for y, row in enumerate(capture.get("rows", [])):
        if y >= nrows:
            bad(y, None, f"row {y} is beyond dimensions.rows={nrows}")
            continue
        for item in row:
            x = item.get("x")
            if not isinstance(x, int):
                bad(y, None, f"run without an integer x: {item!r:.60}")
                continue
            run_cells, leading = _text_cells(item.get("text", ""), _style(item, defaults))
            if leading:
                base = x - 1
                if cells.get((y, base), (None,))[0] == _CONT:
                    base -= 1
                if base >= 0 and (y, base) in cells:
                    cells[(y, base)] = (cells[(y, base)][0] + leading, cells[(y, base)][1])
                else:
                    bad(y, x, "run starts with a zero-width character and no base cell")
            if len(run_cells) != item.get("w", len(run_cells)):
                bad(y, x, f"run w={item.get('w')} but its text is {len(run_cells)} columns")
            if x + len(run_cells) > ncols:
                bad(y, x, f"run spans columns {x}..{x + len(run_cells) - 1}, past cols={ncols}")
            for i, cell in enumerate(run_cells):
                if x + i >= ncols:
                    break
                if (y, x + i) in cells:
                    bad(y, x + i, "overlapping runs")
                cells[(y, x + i)] = cell
    return cells, nrows, ncols


def _cell_at(cells, row, col, nrows, ncols, blank):
    if row >= nrows or col >= ncols:
        return None
    return cells.get((row, col), blank)


def _identity_cells(rule: Rule, cells: dict, blank: tuple, out: list) -> dict:
    """Return {(row, col): cell} for the rewritten expected region, or {} on failure."""
    def fail(row, col, message):
        out.append(Difference(row, col, "identity", message, None))
        return {}

    width = rule.col1 - rule.col0 + 1
    rewritten: dict[tuple, tuple] = {}
    used: set = set()
    for row in range(rule.row0, rule.row1 + 1):
        region = [cells.get((row, c), blank) for c in range(rule.col0, rule.col1 + 1)]
        after = cells.get((row, rule.col1 + 1), blank)
        if region[0][0] == _CONT or after[0] == _CONT:
            return fail(row, rule.col0, "identity region splits a wide glyph")
        graphemes = []  # (text, style); a wide glyph is one grapheme
        for text, style in region:
            if text != _CONT:
                graphemes.append((text, style))
        flat = "".join(g[0] for g in graphemes)
        starts, pos = {}, 0
        for index, (text, _) in enumerate(graphemes):
            starts[pos] = index
            pos += len(text)
        starts[pos] = len(graphemes)
        table = rule.substitutions or {}
        keys = sorted(table, key=len, reverse=True)
        out_cells: list[tuple] = []
        spare: list[bool] = []  # True for an unreplaced original blank, which may be dropped
        i = 0
        while i < len(flat):
            for key in keys:
                if flat.startswith(key, i) and i in starts and i + len(key) in starts:
                    style = graphemes[starts[i]][1]
                    repl, _ = _text_cells(table[key], style)
                    out_cells.extend(repl)
                    spare.extend([False] * len(repl))
                    used.add(key)
                    i += len(key)
                    break
            else:
                index = starts[i]
                text, style = graphemes[index]
                cells_for, _ = _text_cells(text, style)
                out_cells.extend(cells_for)
                spare.extend([text == " "] * len(cells_for))
                i += len(text)
        while len(out_cells) > width and spare and spare[-1]:
            out_cells.pop()
            spare.pop()
        if len(out_cells) > width:
            return fail(row, rule.col0, f"substituted text is {len(out_cells)} columns, region is {width}")
        out_cells.extend([(" ", region[-1][1])] * (width - len(out_cells)))
        for offset, cell in enumerate(out_cells):
            rewritten[(row, rule.col0 + offset)] = cell
    unused = [k for k in (rule.substitutions or {}) if k not in used]
    if unused:
        return fail(rule.row0, rule.col0, f"substitutions matched nothing in the region: {unused!r}")
    return rewritten


def _validate_rules(rules, expected, ncols, nrows):
    rules = list(rules)
    golden = expected.get("name")
    for rule in rules:
        if not isinstance(rule, Rule):
            raise ValueError(f"not a Rule: {rule!r}")
        if rule.golden != golden:
            raise ValueError(f"rule is for golden {rule.golden!r}, comparing {golden!r}")
        if rule.row0 < 0 or rule.col0 < 0 or rule.row1 >= nrows or rule.col1 >= ncols:
            raise ValueError(f"rule region {rule.region!r} is outside the {ncols}x{nrows} grid")
    for i, a in enumerate(rules):
        for b in rules[i + 1:]:
            if a.overlaps(b):
                raise ValueError(f"rule regions overlap: {a.region!r} and {b.region!r}")
    return rules


_INVISIBLE = {"fg": None, "bold": False, "dim": False, "italic": False, "blink": False}


def _plain_blank(cell: tuple) -> bool:
    text, style = cell
    named = dict(zip(STYLE_FIELDS, style))
    return text == " " and not (named["underline"] or named["strike"] or named["inverse"])


def _hide_invisible(cell: tuple) -> tuple:
    named = dict(zip(STYLE_FIELDS, cell[1]))
    named.update(_INVISIBLE)
    return cell[0], tuple(named[k] for k in STYLE_FIELDS)


def _compare_pair(row, col, left, right, ignore_text, geometry, out):
    if left is None or right is None:
        out.append(Difference(row, col, "outside-grid", left, right))
        return
    if _plain_blank(left) and _plain_blank(right):  # only the bg of a plain blank shows
        left, right = _hide_invisible(left), _hide_invisible(right)
    if not ignore_text:
        if left[0] != right[0]:
            out.append(Difference(row, col, "text", left[0], right[0]))
    elif geometry and (left[0] == _CONT) != (right[0] == _CONT):
        out.append(Difference(row, col, "wide-geometry", left[0] == _CONT, right[0] == _CONT))
    for name, lv, rv in zip(STYLE_FIELDS, left[1], right[1]):
        if lv != rv:
            out.append(Difference(row, col, name, lv, rv))


def compare(expected: dict, actual: dict, rules=()) -> DiffReport:
    out: list[Difference] = []
    edims, adims = expected.get("dimensions"), actual.get("dimensions")
    if edims != adims:
        out.append(Difference(None, None, "dimensions", edims, adims))
    ecur, acur = expected.get("cursor") or {}, actual.get("cursor") or {}
    names = ["hidden"]
    if ecur.get("hidden") is not True or acur.get("hidden") is not True:
        names += ["x", "y"]
    for key, source_names in (("cursor", names), ("modes", MODE_FIELDS), ("buffer", ("type",))):
        left, right = expected.get(key) or {}, actual.get(key) or {}
        for name in source_names:
            lv, rv = left.get(name, _MISSING), right.get(name, _MISSING)
            if lv != rv:
                out.append(Difference(None, None, f"{key}.{name}",
                                      None if lv is _MISSING else lv, None if rv is _MISSING else rv))
    ecells, erows, ecols = _grid(expected, "expected", out)
    acells, arows, acols = _grid(actual, "actual", out)
    rules = _validate_rules(rules, expected, ecols, erows)

    eblank, ablank = _blank(expected), _blank(actual)
    owner = {}
    identity: dict = {}
    for rule in rules:
        for r in range(rule.row0, rule.row1 + 1):
            for c in range(rule.col0, rule.col1 + 1):
                owner[(r, c)] = rule
        if rule.kind == "identity":
            identity.update(_identity_cells(rule, ecells, eblank, out))

    for row in range(max(erows, arows)):
        for col in range(max(ecols, acols)):
            rule = owner.get((row, col))
            if rule is not None and rule.kind == "mark":
                continue
            left = _cell_at(ecells, row, col, erows, ecols, eblank)
            right = _cell_at(acells, row, col, arows, acols, ablank)
            if rule is not None and rule.kind == "identity":
                if (row, col) not in identity:
                    continue  # the identity failure was already reported
                left = identity[(row, col)]
            _compare_pair(row, col, left, right,
                          ignore_text=rule is not None and rule.kind in ("data", "structure"),
                          geometry=rule is not None and rule.kind == "structure", out=out)
    return DiffReport(out)
