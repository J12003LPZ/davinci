import copy
import json
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "ui"))
from cc2316_compare import DiffReport, Rule, compare, compare_golden, load_capture

PACKAGE_FRAMES = Path("C:/Users/sergi/Desktop/lean-claude-ui/artifacts/frames/reference")
FG = "#e6e6e6"
BG = "#000000"


def run(x, text, w=None, **style):
    item = {"x": x, "w": len(text) if w is None else w, "text": text, "fg": FG, "bg": BG,
            "bold": False, "dim": False, "italic": False, "underline": False,
            "strike": False, "inverse": False, "blink": False}
    item.update(style)
    return item


def capture(rows, cols=10, nrows=3, name="g1", cursor=None, modes=None, defaults=None, buffer=None):
    """rows: {row index: [runs]}. Rows not named are one blank run."""
    grid = []
    for y in range(nrows):
        grid.append(rows.get(y, [run(0, " " * cols)]))
    cap = {"schema": "claude-tui-styled-capture/2", "name": name,
           "dimensions": {"cols": cols, "rows": nrows},
           "cursor": cursor or {"x": 0, "y": 0, "hidden": True},
           "modes": modes or {"alternate_screen": True, "mouse_sgr": True},
           "rows": grid}
    if defaults is not None:
        cap["defaults"] = defaults
    if buffer is not None:
        cap["buffer"] = buffer
    return cap


def base():
    return capture({0: [run(0, "hello     ")]})


def fields(report):
    return {(d.row, d.column, d.field) for d in report.differences}


class CompareTests(unittest.TestCase):
    def test_identical(self):
        report = compare(base(), copy.deepcopy(base()))
        self.assertIsInstance(report, DiffReport)
        self.assertTrue(report.ok)
        self.assertEqual(report.differences, [])

    def test_different_run_boundaries_compare_equal(self):
        split = capture({0: [run(0, "he"), run(2, "llo"), run(5, "     ")]})
        self.assertTrue(compare(base(), split).ok)

    def test_uncovered_cells_are_default_blanks(self):
        explicit = capture({0: [run(0, "hi"), run(2, " " * 8, fg=None, bg=None)]})
        sparse = capture({0: [run(0, "hi")]})
        self.assertTrue(compare(explicit, sparse).ok)

    def test_missing_style_field_equals_default(self):
        bare = base()
        for item in bare["rows"][0]:
            del item["bold"], item["blink"]
        self.assertTrue(compare(base(), bare).ok)

    def test_wide_glyph_takes_two_columns(self):
        wide = "\u4f60"
        a = capture({0: [run(0, wide + "ab", w=4), run(4, " " * 6)]})
        b = capture({0: [run(0, wide, w=2), run(2, "ab" + " " * 6)]})
        self.assertTrue(compare(a, b).ok)
        shifted = capture({0: [run(0, "a" + wide + "b", w=4), run(4, " " * 6)]})
        self.assertFalse(compare(a, shifted).ok)

    def test_wide_glyph_style_applies_to_both_columns(self):
        wide = "你"
        a = capture({0: [run(0, wide, w=2), run(2, " " * 8)]})
        b = capture({0: [run(0, wide, w=2, bg="#111111"), run(2, " " * 8)]})
        self.assertEqual(fields(compare(a, b)), {(0, 0, "bg"), (0, 1, "bg")})

    def test_combining_mark_stays_with_base(self):
        a = capture({0: [run(0, "e\u0301x", w=2), run(2, " " * 8)]})
        b = capture({0: [run(0, "e", w=1), run(1, "\u0301x", w=1), run(2, " " * 8)]})
        self.assertTrue(compare(a, b).ok)
        plain = capture({0: [run(0, "ex", w=2), run(2, " " * 8)]})
        self.assertIn((0, 0, "text"), fields(compare(a, plain)))

    def test_one_cell_text_difference(self):
        other = capture({0: [run(0, "hellO     ")]})
        report = compare(base(), other)
        self.assertEqual(fields(report), {(0, 4, "text")})
        d = report.differences[0]
        self.assertEqual((d.expected, d.actual), ("o", "O"))

    def test_one_style_difference_each_field(self):
        for key, value in (("fg", "#e6e6e7"), ("bg", "#000001"), ("bold", True), ("dim", True),
                           ("italic", True), ("underline", True), ("strike", True),
                           ("inverse", True), ("blink", True)):
            other = capture({0: [run(0, "hell"), run(4, "o", **{key: value}), run(5, " " * 5)]})
            self.assertEqual(fields(compare(base(), other)), {(0, 4, key)}, key)

    def test_default_colour_differs_from_explicit_colour(self):
        default = capture({0: [run(0, "hello", fg=None), run(5, " " * 5)]})
        explicit = capture({0: [run(0, "hello", fg=FG), run(5, " " * 5)]})
        self.assertEqual({f for _, _, f in fields(compare(default, explicit))}, {"fg"})

    def test_extra_trailing_run_is_reported(self):
        other = base()
        other["rows"][0].append(run(10, "z"))
        report = compare(base(), other)
        self.assertFalse(report.ok)
        self.assertTrue(any(d.field == "layout" and d.row == 0 for d in report.differences))

    def test_row_beyond_dimensions_is_reported(self):
        other = base()
        other["rows"].append([run(0, "x")])
        report = compare(base(), other)
        self.assertTrue(any(d.field == "layout" and d.row == 3 for d in report.differences))

    def test_dimension_difference_does_not_truncate(self):
        bigger = capture({0: [run(0, "hello     ")], 3: [run(0, "tail      ")]}, nrows=4)
        report = compare(base(), bigger)
        self.assertIn((None, None, "dimensions"), fields(report))
        self.assertTrue(any(d.row == 3 for d in report.differences))

    def test_negative_column_run_is_a_layout_difference(self):
        other = base()
        other["rows"][0].append(run(-1, "z"))
        report = compare(base(), other)
        self.assertTrue(any(d.field == "layout" and d.row == 0 for d in report.differences))
        self.assertFalse(compare(other, other).ok)

    def test_negative_or_non_integer_width_is_a_layout_difference(self):
        for text, w in (("hello", -1), ("hello", 5.0), ("h", True), ("", -0.0)):
            other = base()
            other["rows"][0] = [run(0, text, w=w), run(len(text), "hello"[len(text):] + " " * 5)]
            report = compare(base(), other)
            self.assertTrue(any(d.field == "layout" for d in report.differences), repr(w))

    def test_overlapping_runs_are_reported(self):
        other = base()
        other["rows"][0].append(run(2, "zz"))
        self.assertTrue(any(d.field == "layout" for d in compare(base(), other).differences))

    def test_run_width_mismatch_is_reported(self):
        other = capture({0: [run(0, "hello", w=4), run(5, " " * 5)]})
        self.assertTrue(any(d.field == "layout" for d in compare(base(), other).differences))

    def test_cursor_and_mode_differences(self):
        shown = capture({0: [run(0, "hello     ")]}, cursor={"x": 0, "y": 0, "hidden": False})
        other = copy.deepcopy(shown)
        other["cursor"]["x"] = 3
        other["modes"]["mouse_sgr"] = False
        other["modes"]["extra"] = True
        got = {d.field for d in compare(shown, other).differences}
        self.assertEqual(got, {"cursor.x", "modes.mouse_sgr"})

    def test_missing_mode_is_not_false(self):
        off = base()
        off["modes"]["mouse_sgr"] = False
        gone = base()
        del gone["modes"]["mouse_sgr"]
        self.assertEqual({d.field for d in compare(off, gone).differences}, {"modes.mouse_sgr"})


STATE = ("alternate_screen", "mouse_click", "mouse_drag", "mouse_move", "mouse_sgr", "synchronized_output")
COUNTERS = ("sync_frames", "cursor_hide", "cursor_show", "alternate_screen_enters",
            "alternate_screen_exits", "kitty_keyboard_query", "kitty_keyboard_push",
            "modify_other_keys", "cursor_shapes")


def with_modes(modes):
    return capture({0: [run(0, "hello     ")]}, modes=modes)


class ModeTests(unittest.TestCase):
    def test_event_counters_are_ignored(self):
        a = with_modes({"alternate_screen": True, **{k: 1 for k in COUNTERS}})
        b = with_modes({"alternate_screen": True, **{k: 9 for k in COUNTERS}})
        self.assertTrue(compare(a, b).ok)

    def test_each_state_field_is_compared(self):
        for name in STATE:
            with self.subTest(mode=name):
                a, b = with_modes({name: True}), with_modes({name: False})
                self.assertEqual({d.field for d in compare(a, b).differences}, {f"modes.{name}"})

    def test_state_field_missing_on_one_side_differs(self):
        a = with_modes({"synchronized_output": False, "mouse_sgr": True})
        b = with_modes({"mouse_sgr": True})
        self.assertEqual({d.field for d in compare(a, b).differences}, {"modes.synchronized_output"})

    def test_paste_focus_and_unknown_modes_are_ignored(self):
        a = with_modes({"bracketed_paste": True, "focus_events": True, "other": 1})
        b = with_modes({"bracketed_paste": False, "focus_events": False})
        self.assertTrue(compare(a, b).ok)


class InvisibleStyleTests(unittest.TestCase):
    HIDDEN = dict(fg="#111111", bold=True, dim=True, italic=True, blink=True)

    def pair(self, style, text=" "):
        a = capture({0: [run(0, "hello"), run(5, " " * 5)]})
        b = capture({0: [run(0, "hello"), run(5, text, **style), run(6, " " * 4)]})
        return a, b

    def test_blank_ignores_invisible_style(self):
        self.assertTrue(compare(*self.pair(self.HIDDEN)).ok)

    def test_blank_still_compares_bg(self):
        self.assertEqual(fields(compare(*self.pair({"bg": "#222222"}))), {(0, 5, "bg")})

    def test_blank_with_visible_attribute_compares_fully(self):
        for attr in ("underline", "strike", "inverse"):
            with self.subTest(attr=attr):
                got = fields(compare(*self.pair({attr: True, "fg": "#111111"})))
                self.assertEqual(got, {(0, 5, attr), (0, 5, "fg")})

    def test_text_cell_keeps_full_comparison(self):
        got = fields(compare(*self.pair(self.HIDDEN, text="x")))
        self.assertIn((0, 5, "fg"), got)
        self.assertIn((0, 5, "bold"), got)

    def test_applies_inside_data_and_structure_regions(self):
        for kind in ("data", "structure"):
            with self.subTest(kind=kind):
                a, b = self.pair(self.HIDDEN)
                self.assertTrue(compare(a, b, [rule(kind, box(0, 0, 5, 9))]).ok)
                a, b = self.pair({"bg": "#222222"})
                self.assertEqual(fields(compare(a, b, [rule(kind, box(0, 0, 5, 9))])), {(0, 5, "bg")})

    def test_applies_after_identity_rewrite(self):
        exp = capture({0: [run(0, "Claude"), run(6, "    ", fg="#111111", bold=True)]})
        act = capture({0: [run(0, "DaVinci"), run(7, "   ")]})
        r = rule("identity", box(0, 0, 0, 9), substitutions={"Claude": "DaVinci"})
        self.assertTrue(compare(exp, act, [r]).ok)


class CursorBufferTests(unittest.TestCase):
    def cur(self, **kw):
        return capture({0: [run(0, "hello     ")]}, cursor={"x": 1, "y": 1, "hidden": True, **kw})

    def test_hidden_cursor_position_shape_and_attrs_ignored(self):
        a = self.cur(shape="unknown", attrs=[])
        b = capture({0: [run(0, "hello     ")]},
                    cursor={"x": 7, "y": 2, "hidden": True, "shape": "bar", "attrs": [5]})
        self.assertTrue(compare(a, b).ok)

    def test_hidden_flag_is_always_compared(self):
        a = self.cur()
        b = self.cur(hidden=False)
        self.assertEqual({d.field for d in compare(a, b).differences}, {"cursor.hidden"})

    def test_visible_cursor_position_compared_on_either_side(self):
        for ea, ac in ((False, False), (False, True), (True, False)):
            with self.subTest(expected_hidden=ea, actual_hidden=ac):
                a = capture({0: [run(0, "hello     ")]}, cursor={"x": 1, "y": 1, "hidden": ea})
                b = capture({0: [run(0, "hello     ")]}, cursor={"x": 2, "y": 2, "hidden": ac})
                got = {d.field for d in compare(a, b).differences}
                self.assertTrue({"cursor.x", "cursor.y"} <= got)

    def test_visible_cursor_shape_ignored(self):
        a = self.cur(hidden=False, shape="unknown")
        b = self.cur(hidden=False, shape="block", attrs=[2])
        self.assertTrue(compare(a, b).ok)

    def test_buffer_compares_type_only(self):
        rows = {0: [run(0, "hello     ")]}
        a = capture(rows, buffer={"type": "alternate", "length": 3, "baseY": 0, "viewportY": 0})
        b = capture(rows, buffer={"type": "alternate", "length": 90, "baseY": 7, "viewportY": 7})
        self.assertTrue(compare(a, b).ok)
        c = capture(rows, buffer={"type": "normal", "length": 3, "baseY": 0, "viewportY": 0})
        self.assertEqual({d.field for d in compare(a, c).differences}, {"buffer.type"})

    def test_buffer_type_missing_on_one_side_differs(self):
        rows = {0: [run(0, "hello     ")]}
        a = capture(rows, buffer={"type": "alternate"})
        for b in (capture(rows, buffer={"length": 3}), capture(rows)):
            self.assertEqual({d.field for d in compare(a, b).differences}, {"buffer.type"})
        self.assertTrue(compare(capture(rows), capture(rows)).ok)


class DefaultsTests(unittest.TestCase):
    DEFAULTS = {"fg": FG, "bg": BG}

    def test_uncovered_cells_take_the_capture_defaults(self):
        sparse = capture({0: [run(0, "hi")]}, defaults=self.DEFAULTS)
        explicit = capture({0: [run(0, "hi"), run(2, " " * 8)]}, defaults=self.DEFAULTS)
        self.assertTrue(compare(sparse, explicit).ok)

    def test_missing_colour_fields_take_the_capture_defaults(self):
        bare = capture({0: [run(0, "hello"), run(5, " " * 5)]}, defaults=self.DEFAULTS)
        for item in bare["rows"][0]:
            del item["fg"], item["bg"]
        self.assertTrue(compare(base(), bare).ok)

    def test_each_side_uses_its_own_defaults(self):
        exp = capture({0: [run(0, "hi")]}, defaults=self.DEFAULTS)
        act = capture({0: [run(0, "hi")]}, defaults={"fg": FG, "bg": "#101010"})
        self.assertEqual(fields(compare(exp, act)), {(0, c, "bg") for c in range(2, 10)})

    def test_non_default_text_colour_still_differs(self):
        plain = capture({0: [run(0, "hello"), run(5, " " * 5)]}, defaults=self.DEFAULTS)
        red = capture({0: [run(0, "hello", fg="#ff0000"), run(5, " " * 5)]}, defaults=self.DEFAULTS)
        self.assertEqual(fields(compare(plain, red)), {(0, c, "fg") for c in range(5)})

    def test_without_defaults_missing_colours_are_none(self):
        none_blank = capture({0: [run(0, "hi"), run(2, " " * 8, bg=None)]})
        self.assertFalse(compare(none_blank, capture({0: [run(0, "hi"), run(2, " " * 8)]})).ok)


def box(r0, r1, c0, c1):
    return {"rows": (r0, r1), "cols": (c0, c1)}


def rule(kind, region, golden="g1", reason="test", **extra):
    return Rule(golden=golden, region=region, kind=kind, reason=reason, **extra)


class RuleTests(unittest.TestCase):
    def test_mark_ignores_inside_and_compares_outside(self):
        r = rule("mark", box(1, 1, 0, 4))
        one = capture({1: [run(0, "hello     ")]})
        other = capture({1: [run(0, "XYZ", bold=True), run(3, "lo     ")]})
        self.assertTrue(compare(one, other, [r]).ok)
        spill = capture({1: [run(0, "hello", bold=True), run(5, "Q    ")]})
        self.assertEqual(fields(compare(one, spill, [r])), {(1, 5, "text")})

    def test_mark_corruption_just_outside_still_fails(self):
        r = rule("mark", box(1, 1, 0, 4))
        one = capture({1: [run(0, "hello     ")]})
        other = capture({1: [run(0, "hello"), run(5, "     ", bg="#010101")]})
        self.assertEqual(fields(compare(one, other, [r])), {(1, c, "bg") for c in range(5, 10)})
        below = capture({1: [run(0, "hello     ")], 2: [run(0, "a" + " " * 9)]})
        self.assertEqual(fields(compare(one, below, [r])), {(2, 0, "text")})

    def test_data_ignores_text_but_not_style(self):
        r = rule("data", box(0, 0, 0, 4))
        other = capture({0: [run(0, "12345"), run(5, " " * 5)]})
        self.assertTrue(compare(base(), other, [r]).ok)
        styled = capture({0: [run(0, "12345", fg="#ff0000"), run(5, " " * 5)]})
        self.assertEqual({f for _, _, f in fields(compare(base(), styled, [r]))}, {"fg"})

    def test_data_still_fails_one_cell_outside(self):
        r = rule("data", box(0, 0, 0, 3))
        other = capture({0: [run(0, "12345"), run(5, " " * 5)]})
        self.assertEqual(fields(compare(base(), other, [r])), {(0, 4, "text")})

    def test_structure_ignores_blank_vs_nonblank_but_not_style(self):
        exp = capture({0: [run(0, "abc", bold=True), run(3, " " * 7)]})
        r = rule("structure", box(0, 0, 0, 2))
        blank = capture({0: [run(0, "   ", bold=True), run(3, " " * 7)]})
        self.assertTrue(compare(exp, blank, [r]).ok)
        plain = capture({0: [run(0, "abc"), run(3, " " * 7)]})
        self.assertEqual({f for _, _, f in fields(compare(exp, plain, [r]))}, {"bold"})

    def test_structure_compares_wide_geometry(self):
        wide = "\u4f60"
        exp = capture({0: [run(0, wide + "a", w=3), run(3, " " * 7)]})
        r = rule("structure", box(0, 0, 0, 2))
        narrow = capture({0: [run(0, "bca"), run(3, " " * 7)]})
        self.assertFalse(compare(exp, narrow, [r]).ok)
        other_wide = capture({0: [run(0, "\u597d" + "z", w=3), run(3, " " * 7)]})
        self.assertTrue(compare(exp, other_wide, [r]).ok)

    def test_identity_substitutes_repads_and_keeps_style(self):
        exp = capture({0: [run(0, "Claude", fg="#aa0000"), run(6, "    ")]})
        r = rule("identity", box(0, 0, 0, 9), substitutions={"Claude": "DaVinci"})
        ok = capture({0: [run(0, "DaVinci", fg="#aa0000"), run(7, "   ")]})
        self.assertTrue(compare(exp, ok, [r]).ok)
        wrong_style = capture({0: [run(0, "DaVinci"), run(7, "   ")]})
        self.assertIn((0, 0, "fg"), fields(compare(exp, wrong_style, [r])))
        wrong_text = capture({0: [run(0, "DaVinc!", fg="#aa0000"), run(7, "   ")]})
        self.assertEqual(fields(compare(exp, wrong_text, [r])), {(0, 6, "text")})

    def test_identity_replacement_that_does_not_fit_fails_without_truncation(self):
        exp = capture({0: [run(0, "Claude"), run(6, "    ")]})
        r = rule("identity", box(0, 0, 0, 5), substitutions={"Claude": "DaVinci"})
        other = capture({0: [run(0, "DaVinc"), run(6, "    ")]})
        report = compare(exp, other, [r])
        self.assertFalse(report.ok)
        self.assertTrue(any(d.field == "identity" for d in report.differences))

    def test_identity_unused_substitution_fails(self):
        r = rule("identity", box(0, 0, 0, 9), substitutions={"Nothing": "X"})
        report = compare(base(), base(), [r])
        self.assertTrue(any(d.field == "identity" for d in report.differences))

    def test_identity_outside_region_still_compared(self):
        exp = capture({0: [run(0, "Claude  XY")]})
        r = rule("identity", box(0, 0, 0, 7), substitutions={"Claude": "Dav"})
        other = capture({0: [run(0, "Dav     XZ")]})
        self.assertEqual(fields(compare(exp, other, [r])), {(0, 9, "text")})

    def test_invalid_rules_raise(self):
        with self.assertRaises(ValueError):
            rule("bogus", box(0, 0, 0, 1))
        with self.assertRaises(ValueError):
            rule("data", box(0, 0, 0, 1), reason="   ")
        with self.assertRaises(ValueError):
            rule("data", box(0, 0, 0, 1), reason="")
        with self.assertRaises(ValueError):
            rule("data", box(2, 1, 0, 1))
        with self.assertRaises(ValueError):
            rule("identity", box(0, 0, 0, 1))
        with self.assertRaises(ValueError):
            rule("data", box(0, 0, 0, 1), substitutions={"a": "b"})

    def test_rule_outside_grid_raises(self):
        for region in (box(0, 3, 0, 1), box(0, 0, 0, 10), box(-1, 0, 0, 1)):
            with self.assertRaises(ValueError, msg=str(region)):
                compare(base(), base(), [rule("data", region)])

    def test_overlapping_rules_raise(self):
        with self.assertRaises(ValueError):
            compare(base(), base(), [rule("data", box(0, 1, 0, 3)), rule("data", box(1, 2, 3, 5))])

    def test_adjacent_rules_are_allowed(self):
        self.assertTrue(compare(base(), base(), [rule("data", box(0, 0, 0, 3)),
                                                 rule("data", box(0, 0, 4, 5))]).ok)

    def test_rule_for_another_golden_raises(self):
        with self.assertRaises(ValueError):
            compare(base(), base(), [rule("data", box(0, 0, 0, 1), golden="other")])


class MarkBoxTests(unittest.TestCase):
    def test_mark_must_sit_inside_the_header_box(self):
        for region in (box(0, 0, 0, 4), box(1, 4, 0, 4), box(1, 3, 0, 10), box(0, 43, 0, 119)):
            with self.assertRaises(ValueError, msg=str(region)):
                rule("mark", region)
        rule("mark", box(1, 3, 0, 9))

    def test_whole_screen_mark_raises(self):
        with self.assertRaises(ValueError):
            compare(base(), capture({}), [rule("mark", box(0, 2, 0, 9))])

    def test_the_real_mark_fits_the_box(self):
        path = PACKAGE_FRAMES / "boot__02_settled.json"
        if not path.is_file():
            self.skipTest("reference package not present")
        cap = load_capture(path)
        cells = {(y, x + i) for y, row in enumerate(cap["rows"][:5]) for run_ in row
                 if run_.get("fg") == "#d77757" and run_["x"] < 11
                 for i, ch in enumerate(run_["text"]) if ch != " " for x in [run_["x"]]}
        self.assertTrue(cells)
        self.assertTrue(all(1 <= y <= 3 and 0 <= x <= 9 for y, x in cells), sorted(cells))


class CompareGoldenTests(unittest.TestCase):
    def setUp(self):
        import tempfile
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)

    def files(self, tier, rules, golden="g1", reason=None):
        owner = {"owner_task": "P00", "source_owners": [], "tier": tier}
        if reason:
            owner["reason"] = reason
        owners = Path(self.tmp.name) / "owners.json"
        owners.write_text(json.dumps({"version": 1, "goldens": {golden: owner}}), encoding="utf-8")
        manifest = Path(self.tmp.name) / "rules.json"
        manifest.write_text(json.dumps({"version": 1, "goldens": {golden: rules}}), encoding="utf-8")
        return manifest, owners

    def spec(self, kind, region=None, **extra):
        return dict({"region": region or {"rows": [0, 0], "cols": [0, 4]}, "kind": kind,
                     "reason": "test"}, **extra)

    def test_empty_manifest_means_exact_comparison(self):
        manifest, owners = self.files("exact", [])
        other = capture({0: [run(0, "HELLO     ")]})
        self.assertFalse(compare_golden(base(), other, manifest, owners).ok)
        self.assertTrue(compare_golden(base(), copy.deepcopy(base()), manifest, owners).ok)

    def test_approved_rule_is_applied(self):
        manifest, owners = self.files("structure", [self.spec("data")])
        other = capture({0: [run(0, "12345"), run(5, " " * 5)]})
        self.assertTrue(compare_golden(base(), other, manifest, owners).ok)

    def test_structure_rule_needs_a_structure_tier(self):
        manifest, owners = self.files("exact", [self.spec("structure")])
        with self.assertRaises(ValueError):
            compare_golden(base(), base(), manifest, owners)

    def test_any_rule_on_an_exception_golden_is_rejected(self):
        manifest, owners = self.files("exception", [self.spec("data")], reason="D1: test")
        with self.assertRaises(ValueError):
            compare_golden(base(), base(), manifest, owners)

    def test_whole_screen_mark_in_the_manifest_raises(self):
        manifest, owners = self.files("exact", [self.spec("mark", {"rows": [0, 2], "cols": [0, 9]})])
        with self.assertRaises(ValueError):
            compare_golden(base(), capture({}), manifest, owners)

    def test_committed_manifest_loads_and_is_empty(self):
        from cc2316_compare import DEFAULT_RULES
        data = json.loads(Path(DEFAULT_RULES).read_text(encoding="utf-8"))
        self.assertEqual(data, {"version": 1, "goldens": {}})


class LoaderTests(unittest.TestCase):
    def test_load_capture_round_trip_and_rejects_non_capture(self):
        import tempfile
        with tempfile.TemporaryDirectory() as tmp:
            good = Path(tmp) / "g1.json"
            good.write_text(json.dumps(base()), encoding="utf-8")
            self.assertTrue(compare(base(), load_capture(good)).ok)
            index = Path(tmp) / "index.json"
            index.write_text(json.dumps({"frames": []}), encoding="utf-8")
            with self.assertRaises(ValueError):
                load_capture(index)

    @unittest.skipUnless(PACKAGE_FRAMES.is_dir(), "reference package not present")
    def test_every_real_golden_equals_itself(self):
        paths = sorted(p for p in PACKAGE_FRAMES.glob("*.json") if p.name != "index.json")
        self.assertTrue(paths)
        for path in paths:
            with self.subTest(golden=path.name):
                cap = load_capture(path)
                report = compare(cap, copy.deepcopy(cap))
                self.assertTrue(report.ok, report.differences[:3])


if __name__ == "__main__":
    unittest.main()
