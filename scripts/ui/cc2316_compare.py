"""Diff styled terminal captures (claude-tui-styled-capture/2) cell by cell.

Runs are expanded into a full grid of ``dimensions`` so two captures that split
the same content into different runs compare equal. Nothing is truncated: a run
past the grid edge, a row beyond ``dimensions``, overlapping runs and a run whose
``w`` disagrees with its text are all reported differences.

Region rules (see the plan, "Region rules, not cell masks") are boxes in the
expected golden. Region coordinates are inclusive: ``{"rows": (r0, r1),
"cols": (c0, c1)}``. Cells outside every region are always compared exactly.

Rules reach a result only through ``compare_golden``, which loads the rules approved for the
golden from ``cc2316_rules.json`` (empty until a task earns an entry) and rejects a
``structure`` rule on a golden that is not tier ``structure`` and any rule on an
``exception`` golden. ``compare`` takes any rules and is for tests.

* ``mark``: must sit inside the header box (rows 1-3, columns 0-9). Cells inside the box
  are not compared at all. Nothing else is
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
* ``dropped``: a surface DaVinci does not draw (decision D3, D6 or D7, cited in the reason).
  The expected cells are ignored and every actual cell in the box must be blank: a space
  that passes the comparison's own blank test (no underline, strike or inverse; the
  invisible fg, bold, dim, italic and blink are not looked at) with the capture's default
  background. Anything drawn there is a ``dropped`` difference, so the rule cannot hide a
  defect. ``compare_golden`` accepts it only inside ``DROPPED_REGIONS`` (D7: the reference's
  underlined link, one row).
* ``copy``: DaVinci's own wording on a ``structure`` golden. Per row ``{"row", "col", "text"}``:
  the capture's cells in the rule's region must be exactly that text at that column and
  blank everywhere else (``copy-text``, ``copy-stray``); the set of foreground and modifier
  combinations on the non-blank cells of each row must equal the golden's set for the same
  columns (``copy-style``); backgrounds are compared cell by cell as always. The golden's
  own text is ignored. The text in a manifest is DaVinci's, never the reference's.
* ``art``: DaVinci's own picture. ``region`` is the rectangle, ``lines`` its exact text (one
  per row, padded with blanks) and ``band`` the zone around it that the reference fills with
  its own art. Rectangle cells must equal the lines (style is ignored); every other cell of
  the band must be a plain blank. The golden's band cells are not compared.
  ``compare_golden`` accepts art only for the goldens and rectangles in ``ART_REGIONS``.
  Rows are 0-based.

``Compact`` (a rule of its own, at most one per golden; decision D6) removes the release
notice and its separator from the *expected* frame before anything is compared: the rows
after the block, within ``transcript``, move up by its height, and the rows vacated at the
bottom of ``transcript`` must be blank in the capture (the ``dropped`` check), or hold the
exact DaVinci text rows in ``exposed``. Rows outside ``transcript`` are untouched. Every other
rule of the golden is written in post-compaction coordinates for the rows below the removed
block. The block is verified in the golden itself: every notice row begins with the notice
bar glyph (``NOTICE_BAR``) in column 0, and the last row (the separator) is blank.
``transcript`` must end at least two rows above the composer's top rule (the first row after
the block that begins with one of ``RULE_GLYPHS``: the rule itself, or the edge of a picker
or panel that replaces the composer), because the effort row and everything below are pinned. A ``transcript`` that reaches there, rows outside the transcript, a block that fails
these checks or a golden with no composer rule raise ``ValueError``.

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
import re
import unicodedata
from dataclasses import dataclass, field
from pathlib import Path

SCHEMA = "claude-tui-styled-capture/2"
STYLE_FIELDS = ("fg", "bg", "bold", "dim", "italic", "underline", "strike", "inverse", "blink")
MODE_FIELDS = ("alternate_screen", "mouse_click", "mouse_drag", "mouse_move", "mouse_sgr",
               "synchronized_output")
_STYLE_DEFAULTS = {"fg": None, "bg": None, "bold": False, "dim": False, "italic": False,
                   "underline": False, "strike": False, "inverse": False, "blink": False}
KINDS = ("mark", "identity", "data", "structure", "dropped", "copy", "art")
MARK_BOX = {"rows": (1, 3), "cols": (0, 9)}  # the header mark of the plan, "Region rules"
# A ``dropped`` rule names the decision that removed a surface. Only the regions listed here
# may be dropped, and ``load_rules`` rejects any other. D7 is the reference's underlined
# security link line: one row of the first-run security notes, its caption and the link,
# columns 4-55 (0-based; the link itself is columns 16-55). D6's notice
# is removed by ``Compact``, which moves the rows instead of leaving a gap, so it is not
# listed. D3's surfaces are not listed because none has a region yet.
DROPPED_REGIONS = {"D7": {"rows": (24, 24), "cols": (4, 55)}}
# Where DaVinci draws its own art over the reference's (0-based rows): the sprite on rows 10-14
# and the dots under it on row 15, inside the band between the two dot rows that the reference
# fills with its star field. The rectangle holds the whole sprite and its dot row.
_ONBOARDING_ART = [{"rect": {"rows": (10, 15), "cols": (0, 57)},
                    "band": {"rows": (2, 15), "cols": (0, 119)}}]
ART_REGIONS = {"onboarding__01_onboarding": _ONBOARDING_ART,
               "onboarding__03_after_enter": _ONBOARDING_ART}
# The glyph that begins every row of the release notice, in column 0 (U+258E, LEFT ONE EIGHTH
# BLOCK), measured on the goldens' notice rows.
NOTICE_BAR = "▎"
# The top edge of the pinned bottom begins with one of these glyphs: the composer's rule
# (U+2500) or, when a picker or panel replaces the composer, the panel's edge (U+2594). The
# transcript must end above it.
RULE_GLYPHS = ("─", "▔")
RULE_PREFIX = 8
_DECISION = re.compile(r"\bD[367]\b")
_COMPACT_DECISION = re.compile(r"\bD6\b")
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
    entries: object = None
    lines: object = None
    band: object = None
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
        if self.kind == "mark" and not (
                MARK_BOX["rows"][0] <= r0 and r1 <= MARK_BOX["rows"][1]
                and MARK_BOX["cols"][0] <= c0 and c1 <= MARK_BOX["cols"][1]):
            raise ValueError(f"a mark must sit inside rows {MARK_BOX['rows']}, "
                             f"columns {MARK_BOX['cols']}, got {self.region!r}")
        if self.kind == "dropped" and not _DECISION.search(self.reason):
            raise ValueError("a dropped rule's reason must cite decision D3, D6 or D7")
        if self.kind == "identity":
            if not self.substitutions or any(not k for k in self.substitutions):
                raise ValueError("an identity rule needs a substitution table with non-empty keys")
        elif self.substitutions is not None:
            raise ValueError("only identity rules take substitutions")
        if self.kind == "copy":
            self._check_copy()
        elif self.entries is not None:
            raise ValueError("only copy rules take entries")
        if self.kind == "art":
            self._check_art()
        elif self.lines is not None or self.band is not None:
            raise ValueError("only art rules take lines or a band")

    def _check_copy(self):
        entries = self.entries
        if not isinstance(entries, (list, tuple)) or not entries:
            raise ValueError("a copy rule needs entries")
        seen = set()
        for entry in entries:
            if not isinstance(entry, dict) or set(entry) != {"row", "col", "text"}:
                raise ValueError(f"a copy entry is a row, a col and a text, got {entry!r:.60}")
            row, col, text = entry["row"], entry["col"], entry["text"]
            if any(isinstance(v, bool) or not isinstance(v, int) for v in (row, col)):
                raise ValueError(f"a copy entry needs integer row and col: {entry!r:.60}")
            width = _plain_width(text, "a copy entry's text")
            if not self.row0 <= row <= self.row1:
                raise ValueError(f"copy row {row} is outside the region {self.region!r}")
            if row in seen:
                raise ValueError(f"two copy entries for row {row}")
            seen.add(row)
            if col < self.col0 or col + width - 1 > self.col1:
                raise ValueError(f"copy text on row {row} does not fit columns "
                                 f"{self.col0}-{self.col1} from column {col}")
        missing = [r for r in range(self.row0, self.row1 + 1) if r not in seen]
        if missing:
            raise ValueError(f"copy region rows without an entry: {missing}")

    def _check_art(self):
        if self.lines is None or self.band is None:
            raise ValueError("an art rule needs lines and a band")
        if (not isinstance(self.lines, (list, tuple)) or not self.lines
                or len(self.lines) != self.row1 - self.row0 + 1):
            raise ValueError("an art rule needs one line per rectangle row")
        width = self.col1 - self.col0 + 1
        for line in self.lines:
            if _plain_width(line, "an art line", allow_empty=True) > width:
                raise ValueError(f"an art line is wider than the {width}-column rectangle: {line!r:.40}")
        try:
            (b0, b1), (d0, d1) = self.band["rows"], self.band["cols"]  # type: ignore[index]
        except (KeyError, TypeError, ValueError) as exc:
            raise ValueError(f"band must be a rows and a cols interval: {exc!r}") from exc
        if not (b0 <= self.row0 and self.row1 <= b1 and d0 <= self.col0 and self.col1 <= d1):
            raise ValueError("the art band must hold the rectangle")
        # The rule owns its whole band, so that is what it must not overlap.
        object.__setattr__(self, "rect", (self.row0, self.row1, self.col0, self.col1))
        for name, value in (("row0", b0), ("row1", b1), ("col0", d0), ("col1", d1)):
            object.__setattr__(self, name, value)

    def decision(self) -> str | None:
        """The decision (``D3``, ``D6`` or ``D7``) a dropped rule's reason cites."""
        found = _DECISION.search(self.reason)
        return found.group(0) if found else None

    def contains(self, row: int, col: int) -> bool:
        return self.row0 <= row <= self.row1 and self.col0 <= col <= self.col1

    def overlaps(self, other: "Rule") -> bool:
        return (self.row0 <= other.row1 and other.row0 <= self.row1
                and self.col0 <= other.col1 and other.col0 <= self.col1)


def _plain_width(text, what, allow_empty=False) -> int:
    """Columns of ``text``: a string of printable characters with no zero-width lead."""
    if not isinstance(text, str) or (not text and not allow_empty):
        raise ValueError(f"{what} must be a non-empty string")
    if any(unicodedata.category(ch) == "Cc" for ch in text):
        raise ValueError(f"{what} holds a control character: {text!r:.40}")
    cells, leading = _text_cells(text, ())
    if leading:
        raise ValueError(f"{what} starts with a zero-width character: {text!r:.40}")
    return len(cells)


@dataclass(frozen=True)
class Compact:
    """Remove the release notice (decision D6) from the expected frame. See the module
    docstring. ``rows`` is the notice's rows plus its separator, ``transcript`` the scrolling
    region, both ``(first, last)``, inclusive and 0-based."""
    golden: str
    rows: tuple
    transcript: tuple
    reason: str
    exposed: tuple = ()

    def __post_init__(self):
        if not isinstance(self.reason, str) or not _COMPACT_DECISION.search(self.reason):
            raise ValueError("a compact rule's reason must cite decision D6")
        for name in ("rows", "transcript"):
            value = getattr(self, name)
            if (not isinstance(value, (list, tuple)) or len(value) != 2
                    or any(isinstance(v, bool) or not isinstance(v, int) for v in value)):
                raise ValueError(f"compact {name} must be (first, last) integers, got {value!r}")
            object.__setattr__(self, name, tuple(value))
        if not 0 <= self.rows[0] <= self.rows[1]:
            raise ValueError(f"compact rows {self.rows!r} are not an interval")
        if not self.transcript[0] <= self.rows[0] or self.rows[1] > self.transcript[1]:
            raise ValueError(f"compact rows {self.rows!r} are outside the transcript {self.transcript!r}")
        if not isinstance(self.exposed, (list, tuple)):
            raise ValueError("compact exposed must be a list of text rows")
        object.__setattr__(self, "exposed", tuple(self.exposed))
        for text in self.exposed:
            _plain_width(text, "an exposed row")
        if len(self.exposed) > self.rows[1] - self.rows[0] + 1:
            raise ValueError("compact has more exposed rows than rows it vacates")


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
            if x < 0:
                bad(y, x, f"run starts at negative column {x}")
                continue
            w = item.get("w", 0)
            if isinstance(w, bool) or not isinstance(w, int) or w < 0:
                bad(y, x, f"run w={w!r} is not a non-negative integer")
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


def _row_blank(cells, row, ncols, blank) -> bool:
    return all(_dropped_blank(cells.get((row, c), blank), blank) for c in range(ncols))


def _compacted(expected: dict, rule: Compact):
    """The expected frame without the notice, and the rules for the rows it vacates."""
    golden = expected.get("name")
    if rule.golden != golden:
        raise ValueError(f"compact is for golden {rule.golden!r}, comparing {golden!r}")
    dims = expected.get("dimensions") or {}
    ncols, nrows = dims.get("cols", 0), dims.get("rows", 0)
    first, last = rule.rows
    top, bottom = rule.transcript
    if top < 0 or bottom >= nrows:
        raise ValueError(f"compact transcript {rule.transcript!r} is outside the {nrows}-row grid")
    cells, _, _ = _grid(expected, "expected", [])
    blank = _blank(expected)
    for row in range(first, last):
        if _cell_text(cells, row, 0, blank) != NOTICE_BAR:
            raise ValueError(f"compact row {row} of {golden!r} does not begin with the notice bar "
                             f"in column 0")
    if not _row_blank(cells, last, ncols, blank):
        raise ValueError(f"compact separator row {last} of {golden!r} is not blank")
    if first == last:
        raise ValueError("compact needs at least one notice row and a separator")
    rule_row = next((row for row in range(last + 1, nrows)
                     if any(all(_cell_text(cells, row, c, blank) == glyph
                                for c in range(min(RULE_PREFIX, ncols))) for glyph in RULE_GLYPHS)),
                    None)
    if rule_row is None:
        raise ValueError(f"golden {golden!r} has no composer rule below the notice")
    if bottom > rule_row - 2:
        raise ValueError(f"compact transcript {rule.transcript!r} reaches the pinned bottom, which "
                         f"begins at row {rule_row - 1} (the composer rule is row {rule_row})")
    height = last - first + 1
    rows = list(expected.get("rows", []))
    rows += [[] for _ in range(nrows - len(rows))]
    for row in range(first, bottom - height + 1):
        rows[row] = expected["rows"][row + height] if row + height < len(expected["rows"]) else []
    vacated = bottom - height + 1
    for row in range(vacated, bottom + 1):
        rows[row] = []
    shifted = {**expected, "rows": rows}
    extra = []
    reason = rule.reason
    for index, text in enumerate(rule.exposed):
        row = vacated + index
        width = _plain_width(text, "an exposed row")
        if width > ncols:
            raise ValueError(f"exposed row {text!r:.40} is wider than the {ncols}-column grid")
        extra.append(Rule(golden=golden, region={"rows": (row, row), "cols": (0, width - 1)},
                          kind="art", reason=reason, lines=(text,),
                          band={"rows": (row, row), "cols": (0, ncols - 1)}))
    rest = vacated + len(rule.exposed)
    if rest <= bottom:
        extra.append(Rule(golden=golden, region={"rows": (rest, bottom), "cols": (0, ncols - 1)},
                          kind="dropped", reason=reason))
    return shifted, extra


def _cell_text(cells, row, col, blank) -> str:
    return cells.get((row, col), blank)[0]


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


def _dropped_blank(cell: tuple, blank: tuple) -> bool:
    """A blank cell of the capture: a plain blank (see ``_plain_blank``) on its default bg."""
    named, default = dict(zip(STYLE_FIELDS, cell[1])), dict(zip(STYLE_FIELDS, blank[1]))
    return _plain_blank(cell) and named["bg"] == default["bg"]


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


def _style_set(cells, row, c0, c1, blank_cell) -> set:
    """The foreground and modifier combinations of the non-blank cells of a row segment."""
    found = set()
    for col in range(c0, c1 + 1):
        text, style = cells.get((row, col), blank_cell)
        if text not in (" ", _CONT):
            found.add(tuple(v for name, v in zip(STYLE_FIELDS, style) if name != "bg"))
    return found


def _check_copy(rule: Rule, ecells, eblank, acells, ablank, arows, acols, out):
    for entry in rule.entries:
        row, col = entry["row"], entry["col"]
        wanted, _ = _text_cells(entry["text"], ())
        end = col + len(wanted)
        for c in range(rule.col0, rule.col1 + 1):
            cell = _cell_at(acells, row, c, arows, acols, ablank)
            if cell is None:
                out.append(Difference(row, c, "copy-text", "inside the grid", None))
            elif col <= c < end:
                if cell[0] != wanted[c - col][0]:
                    out.append(Difference(row, c, "copy-text", wanted[c - col][0], cell[0]))
            elif not _plain_blank(cell):
                out.append(Difference(row, c, "copy-stray", "blank", cell))
        have = _style_set(acells, row, rule.col0, rule.col1, ablank)
        want = _style_set(ecells, row, rule.col0, rule.col1, eblank)
        if have != want:
            out.append(Difference(row, None, "copy-style", sorted(map(repr, want)),
                                  sorted(map(repr, have))))


def _check_art(rule: Rule, acells, ablank, arows, acols, out):
    r0, r1, c0, c1 = rule.rect  # type: ignore[attr-defined]
    wanted = {}
    for index, line in enumerate(rule.lines):
        cells, _ = _text_cells(line, ())
        for offset, (text, _) in enumerate(cells):
            wanted[(r0 + index, c0 + offset)] = text
    for row in range(rule.row0, rule.row1 + 1):
        for col in range(rule.col0, rule.col1 + 1):
            cell = _cell_at(acells, row, col, arows, acols, ablank)
            inside = r0 <= row <= r1 and c0 <= col <= c1
            if cell is None:
                out.append(Difference(row, col, "art", "inside the grid", None))
            elif inside:
                if cell[0] != wanted.get((row, col), " "):
                    out.append(Difference(row, col, "art", wanted.get((row, col), " "), cell[0]))
            elif not _dropped_blank(cell, ablank):
                out.append(Difference(row, col, "art", "blank", cell))


def compare(expected: dict, actual: dict, rules=()) -> DiffReport:
    rules = list(rules)
    compacts = [r for r in rules if isinstance(r, Compact)]
    if len(compacts) > 1:
        raise ValueError("a golden takes at most one compact rule")
    rules = [r for r in rules if not isinstance(r, Compact)]
    if compacts:
        expected, vacated = _compacted(expected, compacts[0])
        rules += vacated
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
            if rule is not None and rule.kind in ("mark", "art"):
                continue  # art is checked below, against its own lines
            left = _cell_at(ecells, row, col, erows, ecols, eblank)
            right = _cell_at(acells, row, col, arows, acols, ablank)
            if rule is not None and rule.kind == "copy":
                # Text and the foreground are DaVinci's own; only the background is the golden's.
                if left is None or right is None:
                    out.append(Difference(row, col, "outside-grid", left, right))
                elif left[1][STYLE_FIELDS.index("bg")] != right[1][STYLE_FIELDS.index("bg")]:
                    out.append(Difference(row, col, "bg", left[1][STYLE_FIELDS.index("bg")],
                                          right[1][STYLE_FIELDS.index("bg")]))
                continue
            if rule is not None and rule.kind == "dropped":
                if right is None or not _dropped_blank(right, ablank):
                    out.append(Difference(row, col, "dropped", "blank", right))
                continue
            if rule is not None and rule.kind == "identity":
                if (row, col) not in identity:
                    continue  # the identity failure was already reported
                left = identity[(row, col)]
            _compare_pair(row, col, left, right,
                          ignore_text=rule is not None and rule.kind in ("data", "structure"),
                          geometry=rule is not None and rule.kind == "structure", out=out)
    for rule in rules:
        if rule.kind == "copy":
            _check_copy(rule, ecells, eblank, acells, ablank, arows, acols, out)
        elif rule.kind == "art":
            _check_art(rule, acells, ablank, arows, acols, out)
    return DiffReport(out)


DEFAULT_RULES = Path(__file__).with_name("cc2316_rules.json")
DEFAULT_OWNERS = Path(__file__).with_name("cc2316_owners.json")


def _box(region) -> tuple:
    return tuple(region["rows"]), tuple(region["cols"])


SPEC_KEYS = {"kind", "reason", "region", "substitutions", "entries", "lines", "band", "rows",
             "transcript", "exposed"}


def load_rules(name: str, manifest_path=DEFAULT_RULES, owners_path=DEFAULT_OWNERS) -> list:
    """The approved rules for golden ``name``, checked against its tier in the owners file."""
    manifest = json.loads(Path(manifest_path).read_text(encoding="utf-8"))
    specs = (manifest.get("goldens") or {}).get(name) or []
    if not specs:
        return []
    tier = ((json.loads(Path(owners_path).read_text(encoding="utf-8")).get("goldens") or {})
            .get(name) or {}).get("tier")
    rules = []
    for spec in specs:
        unknown = set(spec) - SPEC_KEYS
        if unknown:
            raise ValueError(f"golden {name!r}: unknown rule keys {sorted(unknown)}")
        if tier == "exception":
            raise ValueError(f"golden {name!r} is an exception; it takes no rules")
        if spec.get("kind") == "compact":
            if set(spec) - {"kind", "reason", "rows", "transcript", "exposed"}:
                raise ValueError(f"golden {name!r}: a compact rule takes rows, transcript, exposed "
                                 f"and a reason, nothing else")
            if any(isinstance(r, Compact) for r in rules):
                raise ValueError(f"golden {name!r} takes at most one compact rule")
            rules.append(Compact(golden=name, rows=spec.get("rows"), transcript=spec.get("transcript"),
                                 reason=spec.get("reason"), exposed=spec.get("exposed") or ()))
            continue
        if {"rows", "transcript", "exposed"} & set(spec):
            raise ValueError(f"golden {name!r}: only a compact rule takes rows, transcript or exposed")
        rule = Rule(golden=name, region=spec.get("region"), kind=spec.get("kind"),
                    reason=spec.get("reason"), substitutions=spec.get("substitutions"),
                    entries=spec.get("entries"), lines=spec.get("lines"), band=spec.get("band"))
        if rule.kind in ("structure", "copy", "art") and tier != "structure":
            raise ValueError(f"a {rule.kind} rule needs golden {name!r} to be tier structure, "
                             f"not {tier!r}")
        if rule.kind == "art":
            listed = [(_box(item["rect"]), _box(item["band"])) for item in ART_REGIONS.get(name, [])]
            if (_box(spec["region"]), _box(spec["band"])) not in listed:
                raise ValueError(f"golden {name!r}: art must use a rectangle and band listed in "
                                 f"ART_REGIONS, got {spec['region']!r} in {spec['band']!r}")
        if rule.kind == "dropped":
            listed = DROPPED_REGIONS.get(rule.decision())
            if listed is None or not (listed["rows"][0] <= rule.row0 and rule.row1 <= listed["rows"][1]
                                      and listed["cols"][0] <= rule.col0 and rule.col1 <= listed["cols"][1]):
                raise ValueError(f"golden {name!r}: a dropped rule must lie inside the region listed "
                                 f"for its decision in DROPPED_REGIONS, got {rule.region!r} for "
                                 f"{rule.decision()!r}")
        rules.append(rule)
    return rules


def compare_golden(expected, actual, manifest_path=DEFAULT_RULES, owners_path=DEFAULT_OWNERS) -> DiffReport:
    """Compare with the rules approved for ``expected["name"]``. Callers that produce
    results use this; ``compare`` takes any rules and is for tests."""
    return compare(expected, actual, load_rules(expected.get("name"), manifest_path, owners_path))
