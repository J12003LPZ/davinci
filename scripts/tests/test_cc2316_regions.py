"""The ``compact``, ``copy`` and ``art`` rules, and the D7 dropped region.

All captures here are synthetic: DaVinci's own wording and a made-up golden, never the
reference's.
"""
import copy
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1] / "ui"))
import cc2316_compare as cmp  # noqa: E402
from cc2316_compare import Compact, Rule, compare, compare_golden  # noqa: E402

FG, BG = "#e6e6e6", "#000000"
COLS, ROWS = 10, 14


def run(x, text, **style):
    item = {"x": x, "w": len(text), "text": text, "fg": FG, "bg": BG, "bold": False,
            "dim": False, "italic": False, "underline": False, "strike": False,
            "inverse": False, "blink": False}
    item.update(style)
    return item


def cap(rows, name="g1", cols=COLS, nrows=ROWS):
    grid = [rows.get(y, [run(0, " " * cols)]) for y in range(nrows)]
    return {"schema": "claude-tui-styled-capture/2", "name": name, "defaults": {"fg": FG, "bg": BG},
            "dimensions": {"cols": cols, "rows": nrows},
            "cursor": {"x": 0, "y": 0, "hidden": True},
            "modes": {"alternate_screen": True, "mouse_sgr": True}, "rows": grid}


def pad(text, cols=COLS):
    return text + " " * (cols - len(text))


def golden():
    """Header, a notice of two bar rows and a blank separator at rows 3-5, a conversation
    at rows 6-7, the effort row 10, the composer rule 11, the prompt and the footer."""
    return cap({
        1: [run(0, pad(" header"))],
        3: [run(0, pad("▎ notice"))], 4: [run(0, pad("▎ more"))],
        6: [run(0, pad("talk one"))], 7: [run(0, pad("talk two"))],
        10: [run(0, pad("  effort"))], 11: [run(0, "─" * COLS)],
        12: [run(0, pad("❯"))], 13: [run(0, pad("  footer"))],
    })


def compacted(**rows):
    """What DaVinci draws: the same screen without the notice and its separator."""
    base = {
        1: [run(0, pad(" header"))],
        3: [run(0, pad("talk one"))], 4: [run(0, pad("talk two"))],
        10: [run(0, pad("  effort"))], 11: [run(0, "─" * COLS)],
        12: [run(0, pad("❯"))], 13: [run(0, pad("  footer"))],
    }
    base.update({int(k[1:]): v for k, v in rows.items()})
    return cap(base)


def compact_rule(**over):
    spec = {"golden": "g1", "rows": (3, 5), "transcript": (3, 9), "reason": "D6: no release notice"}
    spec.update(over)
    return Compact(**spec)


def fields(report):
    return {(d.row, d.column, d.field) for d in report.differences}


class CompactTests(unittest.TestCase):
    def test_the_conversation_moves_up_and_the_vacated_rows_are_blank(self):
        report = compare(golden(), compacted(), [compact_rule()])
        self.assertTrue(report.ok, report)

    def test_without_the_compaction_the_notice_is_a_difference(self):
        self.assertFalse(compare(golden(), golden(), [compact_rule()]).ok)

    def test_the_bar_glyph_is_the_measured_one(self):
        self.assertEqual(cmp.NOTICE_BAR, "▎")

    def test_text_in_a_vacated_row_fails(self):
        drawn = compacted(r8=[run(0, pad("stray"))])
        found = compare(golden(), drawn, [compact_rule()])
        self.assertEqual({(d.row, d.field) for d in found.differences if d.column == 0},
                         {(8, "dropped")})

    def test_a_blank_with_a_visible_style_in_a_vacated_row_fails(self):
        drawn = compacted(r9=[run(0, " ", underline=True), run(1, " " * 9)])
        self.assertEqual(fields(compare(golden(), drawn, [compact_rule()])), {(9, 0, "dropped")})

    def test_exposed_rows_are_exact_text_in_the_first_vacated_rows(self):
        rule = compact_rule(exposed=("more here",))
        shown = compacted(r7=[run(0, pad("more here"))])
        self.assertTrue(compare(golden(), shown, [rule]).ok)
        for wrong in ("more her", "more heres", "More here"):
            with self.subTest(wrong=wrong):
                bad = compacted(r7=[run(0, pad(wrong))])
                self.assertFalse(compare(golden(), bad, [rule]).ok)
        self.assertFalse(compare(golden(), compacted(), [rule]).ok, "an exposed row must be drawn")

    def test_more_exposed_rows_than_vacated_rows_raises(self):
        with self.assertRaises(ValueError):
            compare(golden(), compacted(), [compact_rule(exposed=("a",) * 4)])

    def test_rows_outside_the_transcript_are_untouched(self):
        for row in (1, 10, 11, 12, 13):
            with self.subTest(row=row):
                bent = compacted(**{f"r{row}": [run(0, pad("X" * 5))]})
                self.assertIn((row, 0, "text"), fields(compare(golden(), bent, [compact_rule()])))

    def test_a_row_above_the_notice_stays_where_it_is(self):
        drawn = compacted(r1=[run(0, pad(" header"), bold=True)])
        self.assertTrue(fields(compare(golden(), drawn, [compact_rule()])))

    def test_other_rules_use_post_compaction_rows(self):
        data = Rule(golden="g1", region={"rows": (3, 3), "cols": (0, 9)}, kind="data", reason="x")
        moved = compacted(r3=[run(0, pad("other"))])
        self.assertTrue(compare(golden(), moved, [compact_rule(), data]).ok)
        # In the golden's own rows it would have been the notice.
        self.assertFalse(compare(golden(), compacted(r6=[run(0, pad("XXXX"))]),
                                 [compact_rule(), data]).ok)

    def test_a_rule_over_the_vacated_rows_raises(self):
        data = Rule(golden="g1", region={"rows": (8, 8), "cols": (0, 9)}, kind="data", reason="x")
        with self.assertRaises(ValueError):
            compare(golden(), compacted(), [compact_rule(), data])

    def test_a_compact_is_validated_against_the_golden(self):
        bad = {
            "rows outside the transcript": dict(rows=(3, 5), transcript=(4, 9)),
            "transcript below the block": dict(rows=(3, 5), transcript=(3, 4)),
            "transcript into the pinned bottom": dict(transcript=(3, 10)),
            "transcript past the composer rule": dict(transcript=(3, 12)),
            "block beyond the grid": dict(rows=(3, 20), transcript=(3, 20)),
            "block that is not an interval": dict(rows=(5, 3)),
        }
        for label, over in bad.items():
            with self.subTest(label):
                with self.assertRaises(ValueError):
                    compare(golden(), compacted(), [compact_rule(**over)])

    def test_the_transcript_may_end_two_rows_above_the_composer_rule(self):
        self.assertTrue(compare(golden(), compacted(), [compact_rule(transcript=(3, 9))]).ok)

    def test_more_than_one_compact_raises(self):
        with self.assertRaises(ValueError):
            compare(golden(), compacted(), [compact_rule(), compact_rule()])

    def test_a_compact_for_another_golden_raises(self):
        with self.assertRaises(ValueError):
            compare(golden(), compacted(), [compact_rule(golden="other")])

    def test_the_reason_must_cite_d6(self):
        for reason in ("tidier", "D3: no survey", "D7: no link", " "):
            with self.subTest(reason=reason):
                with self.assertRaises(ValueError):
                    compact_rule(reason=reason)

    def test_the_block_is_verified_in_the_golden(self):
        def regolden(**rows):
            base = {1: [run(0, pad(" header"))], 6: [run(0, pad("talk one"))],
                    10: [run(0, pad("  effort"))], 11: [run(0, "─" * COLS)],
                    12: [run(0, pad("❯"))], 13: [run(0, pad("  footer"))]}
            base.update({int(k[1:]): v for k, v in rows.items()})
            return cap(base)

        notice = {"r3": [run(0, pad("▎ notice"))], "r4": [run(0, pad("▎ more"))]}
        self.assertTrue(compare(regolden(**notice), compacted(r3=[run(0, pad("talk one"))],
                                                              r4=[run(0, " " * COLS)]),
                                [compact_rule()]).ok, "the control with a good block")
        cases = {
            "no bar on the first row": regolden(r3=[run(0, pad("  notice"))], r4=notice["r4"]),
            "the bar is not in column 0": regolden(r3=[run(0, pad(" ▎ notice"))], r4=notice["r4"]),
            "a later notice row without the bar": regolden(r3=notice["r3"], r4=[run(0, pad("  more"))]),
            "a separator that is not blank": regolden(**notice, r5=[run(0, pad("not blank"))]),
            "a separator with a visible style": regolden(
                **notice, r5=[run(0, " ", underline=True), run(1, " " * 9)]),
            "arbitrary content": regolden(r3=[run(0, pad("a conversation"))],
                                          r4=[run(0, pad("more of it"))]),
        }
        for label, golden_ in cases.items():
            with self.subTest(label):
                with self.assertRaises(ValueError):
                    compare(golden_, compacted(), [compact_rule()])

    def test_a_golden_without_a_composer_rule_cannot_be_compacted(self):
        no_rule = golden()
        no_rule["rows"][11] = [run(0, pad("not a rule"))]
        with self.assertRaises(ValueError):
            compare(no_rule, compacted(), [compact_rule()])

    def test_a_compaction_that_moves_nothing_is_rejected(self):
        with self.assertRaises(ValueError):
            compact_rule(rows=(3, 2))


class ManifestFiles(unittest.TestCase):
    """A disposable manifest and owners file for golden ``g1``."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)

    def files(self, tier, specs):
        owners = Path(self.tmp.name) / "owners.json"
        owners.write_text(json.dumps({"version": 1, "goldens": {"g1": {"owner_task": "P00",
                                      "source_owners": [], "tier": tier}}}), encoding="utf-8")
        manifest = Path(self.tmp.name) / "rules.json"
        manifest.write_text(json.dumps({"version": 1, "goldens": {"g1": specs}}), encoding="utf-8")
        return manifest, owners


class CompactManifestTests(ManifestFiles):

    SPEC = {"kind": "compact", "rows": [3, 5], "transcript": [3, 9], "reason": "D6: no release notice"}

    def test_it_loads_from_the_manifest_on_an_exact_and_a_structure_golden(self):
        for tier in ("exact", "structure"):
            manifest, owners = self.files(tier, [self.SPEC])
            self.assertTrue(compare_golden(golden(), compacted(), manifest, owners).ok, tier)

    def test_exposed_loads_from_the_manifest(self):
        manifest, owners = self.files("exact", [dict(self.SPEC, exposed=["more here"])])
        shown = compacted(r7=[run(0, pad("more here"))])
        self.assertTrue(compare_golden(golden(), shown, manifest, owners).ok)

    def test_an_exception_golden_takes_none(self):
        manifest, owners = self.files("exception", [self.SPEC])
        with self.assertRaises(ValueError):
            compare_golden(golden(), compacted(), manifest, owners)

    def test_a_compact_with_a_region_is_rejected(self):
        manifest, owners = self.files("exact", [dict(self.SPEC, region={"rows": [3, 5], "cols": [0, 9]})])
        with self.assertRaises(ValueError):
            compare_golden(golden(), compacted(), manifest, owners)

    def test_the_committed_manifest_has_at_most_one_compact_per_golden_and_cites_d6(self):
        data = json.loads(Path(cmp.DEFAULT_RULES).read_text(encoding="utf-8"))
        for name, specs in data["goldens"].items():
            found = [s for s in specs if s["kind"] == "compact"]
            self.assertLessEqual(len(found), 1, name)
            for spec in found:
                self.assertIn("D6", spec["reason"], name)
                self.assertNotIn("dropped", {s["kind"] for s in specs
                                             if "D6" in s.get("reason", "")}, name)


# ---- copy -------------------------------------------------------------------

DIM = "#aaaaaa"


def copy_golden():
    return cap({2: [run(0, "Alpha", bold=True), run(5, " "), run(6, "beta", fg=DIM)]})


def copy_rule(entries=None, region=None, **over):
    spec = {"golden": "g1", "region": region or {"rows": (2, 2), "cols": (0, 9)}, "kind": "copy",
            "reason": "DaVinci's own wording",
            "entries": [{"row": 2, "col": 0, "text": "Omega zeta"}] if entries is None else entries}
    spec.update(over)
    return Rule(**spec)


def copy_actual(*runs, row=2):
    return cap({row: list(runs)})


class CopyTests(unittest.TestCase):
    def test_other_words_in_the_same_styles_pass(self):
        shown = copy_actual(run(0, "Omega", bold=True), run(5, " "), run(6, "zeta", fg=DIM))
        report = compare(copy_golden(), shown, [copy_rule()])
        self.assertTrue(report.ok, report)

    def test_the_words_may_be_longer_or_shorter_than_the_golden_ones(self):
        rule = copy_rule([{"row": 2, "col": 0, "text": "Om zetas!"}])
        shown = copy_actual(run(0, "Om", bold=True), run(2, " "), run(3, "zetas!", fg=DIM))
        self.assertTrue(compare(copy_golden(), shown, [rule]).ok)

    def test_a_row_that_differs_from_the_manifest_text_fails(self):
        shown = copy_actual(run(0, "Omega", bold=True), run(5, " "), run(6, "zetb", fg=DIM))
        found = compare(copy_golden(), shown, [copy_rule()])
        self.assertTrue(found.ok is False and {d.field for d in found.differences} == {"copy-text"})

    def test_a_one_character_row_fails(self):
        for shown in (copy_actual(run(0, "O", bold=True)),
                      copy_actual(run(0, "O", bold=True), run(1, " "), run(2, "z", fg=DIM))):
            self.assertFalse(compare(copy_golden(), shown, [copy_rule()]).ok)

    def test_a_missing_row_fails(self):
        self.assertFalse(compare(copy_golden(), cap({}), [copy_rule()]).ok)

    def test_a_dropped_style_fails(self):
        plain = copy_actual(run(0, "Omega zeta"))
        found = compare(copy_golden(), plain, [copy_rule()])
        self.assertEqual({d.field for d in found.differences}, {"copy-style"})

    def test_an_extra_style_fails(self):
        extra = copy_actual(run(0, "Omega", bold=True), run(5, " "), run(6, "zeta", fg=DIM, italic=True))
        self.assertEqual({d.field for d in compare(copy_golden(), extra, [copy_rule()]).differences},
                         {"copy-style"})

    def test_the_modifier_must_go_with_the_colour(self):
        # A set of combinations: where they sit does not matter, but a bold in the other
        # colour is a combination the golden lacks.
        moved = copy_actual(run(0, "Omega", fg=DIM), run(5, " "), run(6, "zeta", bold=True))
        self.assertTrue(compare(copy_golden(), moved, [copy_rule()]).ok)
        mixed = copy_actual(run(0, "Omega", fg=DIM, bold=True), run(5, " "), run(6, "zeta"))
        self.assertEqual({d.field for d in compare(copy_golden(), mixed, [copy_rule()]).differences},
                         {"copy-style"})

    def test_a_stray_character_elsewhere_in_the_region_fails(self):
        rule = copy_rule([{"row": 2, "col": 0, "text": "Omega"}])
        golden_ = cap({2: [run(0, "Alpha", bold=True)]})
        shown = copy_actual(run(0, "Omega", bold=True), run(8, "x", bold=True))
        found = compare(golden_, shown, [rule])
        self.assertEqual(fields(found), {(2, 8, "copy-stray")})
        self.assertTrue(compare(golden_, copy_actual(run(0, "Omega", bold=True)), [rule]).ok)

    def test_text_that_starts_in_the_wrong_column_fails(self):
        shown = copy_actual(run(1, "Omega zet", bold=True))
        self.assertFalse(compare(copy_golden(), shown, [copy_rule()]).ok)

    def test_backgrounds_are_compared_cell_by_cell(self):
        shown = copy_actual(run(0, "Omega", bold=True), run(5, " "), run(6, "zeta", fg=DIM, bg="#101010"))
        found = compare(copy_golden(), shown, [copy_rule()])
        self.assertEqual({d.field for d in found.differences}, {"bg"})
        self.assertEqual({d.column for d in found.differences}, set(range(6, 10)))

    def test_a_blank_with_a_different_background_fails(self):
        shown = copy_actual(run(0, "Omega", bold=True), run(5, " ", bg="#202020"),
                            run(6, "zeta", fg=DIM))
        self.assertEqual(fields(compare(copy_golden(), shown, [copy_rule()])), {(2, 5, "bg")})

    def test_cells_outside_the_region_are_still_compared_exactly(self):
        shown = cap({2: [run(0, "Omega", bold=True), run(5, " "), run(6, "zeta", fg=DIM)],
                     3: [run(0, pad("stray"))]})
        self.assertEqual(fields(compare(copy_golden(), shown, [copy_rule()])),
                         {(3, c, "text") for c in range(5)})

    def test_wide_glyphs_are_two_columns(self):
        golden_ = cap({2: [run(0, "ab", bold=True), run(2, " "), run(3, "cd", fg=DIM)]})
        rule = copy_rule([{"row": 2, "col": 0, "text": "中 x"}])
        shown = copy_actual(run(0, "中", bold=True, w=2), run(2, " "), run(3, "x", fg=DIM))
        shown["rows"][2][0]["w"] = 2
        self.assertTrue(compare(golden_, shown, [rule]).ok)

    def test_the_rule_is_validated(self):
        bad = {
            "no entries": dict(entries=[]),
            "a row outside the region": dict(entries=[{"row": 3, "col": 0, "text": "x"}]),
            "two entries for a row": dict(entries=[{"row": 2, "col": 0, "text": "a"},
                                                   {"row": 2, "col": 4, "text": "b"}]),
            "text past the region": dict(entries=[{"row": 2, "col": 5, "text": "toolong"}]),
            "an empty text": dict(entries=[{"row": 2, "col": 0, "text": ""}]),
            "a column before the region": dict(entries=[{"row": 2, "col": -1, "text": "x"}]),
            "a region row without an entry": dict(region={"rows": (1, 2), "cols": (0, 9)}),
            "an entry that is not an object": dict(entries=["text"]),
            "a missing key": dict(entries=[{"row": 2, "text": "x"}]),
            "a control character": dict(entries=[{"row": 2, "col": 0, "text": "a\x1bb"}]),
        }
        for label, over in bad.items():
            with self.subTest(label):
                with self.assertRaises(ValueError):
                    rule = copy_rule(**over)
                    compare(copy_golden(), copy_actual(run(0, "x")), [rule])

    def test_only_a_copy_rule_takes_entries(self):
        with self.assertRaises(ValueError):
            Rule(golden="g1", region={"rows": (2, 2), "cols": (0, 9)}, kind="data", reason="x",
                 entries=[{"row": 2, "col": 0, "text": "x"}])


class CopyManifestTests(ManifestFiles):
    def spec(self, **over):
        return dict({"kind": "copy", "region": {"rows": [2, 2], "cols": [0, 9]},
                     "reason": "DaVinci's own wording",
                     "entries": [{"row": 2, "col": 0, "text": "Omega zeta"}]}, **over)

    def good(self):
        return copy_actual(run(0, "Omega", bold=True), run(5, " "), run(6, "zeta", fg=DIM))

    def test_a_copy_rule_needs_a_structure_tier(self):
        manifest, owners = self.files("structure", [self.spec()])
        self.assertTrue(compare_golden(copy_golden(), self.good(), manifest, owners).ok)
        for tier in ("exact", "exception"):
            manifest, owners = self.files(tier, [self.spec()])
            with self.assertRaises(ValueError):
                compare_golden(copy_golden(), self.good(), manifest, owners)

    def test_the_committed_manifest_carries_no_reference_text(self):
        data = json.loads(Path(cmp.DEFAULT_RULES).read_text(encoding="utf-8"))
        for name, specs in data["goldens"].items():
            for spec in specs:
                if spec["kind"] == "copy":
                    self.assertTrue(spec["entries"], name)



# ---- art --------------------------------------------------------------------

ART_COLS, ART_ROWS = 12, 10
BAND = {"rows": (1, 7), "cols": (0, 11)}
RECT = {"rows": (4, 6), "cols": (2, 7)}
LINES = ("  /\\  ", " /__\\ ", "  ||  ")


def art_cap(rows):
    return cap(rows, cols=ART_COLS, nrows=ART_ROWS)


def art_golden():
    stars = {y: [run(0, pad("* . *  . *", ART_COLS), fg="#777777")] for y in range(1, 8)}
    stars[0] = [run(0, pad("title", ART_COLS))]
    return art_cap(stars)


def art_rule(**over):
    spec = {"golden": "g1", "region": RECT, "band": BAND, "lines": LINES, "kind": "art",
            "reason": "DaVinci's own art"}
    spec.update(over)
    return Rule(**spec)


def art_actual(lines=LINES, at=(4, 2), extra=None, style=None):
    rows = {0: [run(0, pad("title", ART_COLS))]}
    for i, line in enumerate(lines):
        rows[at[0] + i] = [run(0, " " * at[1]), run(at[1], line, **(style or {}))]
    rows.update(extra or {})
    return art_cap(rows)


class ArtTests(unittest.TestCase):
    def test_the_art_in_the_rectangle_with_a_blank_band_passes_whatever_its_style(self):
        for style in ({}, {"fg": "#ff0000", "bold": True}):
            with self.subTest(style=style):
                report = compare(art_golden(), art_actual(style=style), [art_rule()])
                self.assertTrue(report.ok, report)

    def test_missing_art_fails(self):
        found = compare(art_golden(), art_actual(lines=()), [art_rule()])
        self.assertEqual({d.field for d in found.differences}, {"art"})
        self.assertTrue(found.differences)

    def test_art_shifted_by_one_cell_fails(self):
        for at in ((4, 3), (4, 1), (5, 2), (3, 2)):
            with self.subTest(at=at):
                self.assertFalse(compare(art_golden(), art_actual(at=at), [art_rule()]).ok)

    def test_one_wrong_character_fails_and_only_that_cell(self):
        wrong = ("  /\\  ", " /_\\\\ ", "  ||  ")
        found = compare(art_golden(), art_actual(lines=wrong), [art_rule()])
        self.assertTrue(found.differences)
        self.assertTrue(all(d.field == "art" for d in found.differences))

    def test_a_cell_drawn_in_the_band_outside_the_rectangle_fails(self):
        for row, col in ((2, 5), (4, 0), (4, 9), (7, 3), (1, 11), (6, 8)):
            with self.subTest(at=(row, col)):
                shown = art_actual()
                kept = [r for r in shown["rows"][row] if r["text"].strip()]
                shown["rows"][row] = kept + [run(col, "*")]
                found = compare(art_golden(), shown, [art_rule()])
                self.assertEqual({(d.row, d.column, d.field) for d in found.differences},
                                 {(row, col, "art")})

    def test_a_blank_with_a_visible_style_in_the_band_fails(self):
        stray = art_actual(extra={2: [run(0, " ", underline=True), run(1, " " * 11)]})
        self.assertIn((2, 0, "art"), fields(compare(art_golden(), stray, [art_rule()])))

    def test_the_golden_stars_are_not_compared_inside_the_band(self):
        report = compare(art_golden(), art_actual(), [art_rule()])
        self.assertTrue(report.ok)

    def test_cells_outside_the_band_are_compared_exactly(self):
        title = art_actual(extra={0: [run(0, pad("TITLE", ART_COLS))]})
        self.assertEqual({d.row for d in compare(art_golden(), title, [art_rule()]).differences}, {0})
        below = art_actual(extra={8: [run(0, pad("below", ART_COLS))]})
        self.assertEqual({d.row for d in compare(art_golden(), below, [art_rule()]).differences}, {8})

    def test_the_rule_is_validated(self):
        bad = {
            "the lines do not fill the rows": dict(lines=LINES[:2]),
            "a line wider than the rectangle": dict(lines=("0123456789", *LINES[1:])),
            "no lines": dict(lines=()),
            "a rectangle outside the band": dict(region={"rows": (4, 8), "cols": (2, 7)}),
            "a band that does not hold the rectangle": dict(band={"rows": (5, 7), "cols": (0, 11)}),
            "no band": dict(band=None),
            "a control character": dict(lines=("\x1b[1m  ", *LINES[1:])),
        }
        for label, over in bad.items():
            with self.subTest(label):
                with self.assertRaises(ValueError):
                    rule = art_rule(**over)
                    compare(art_golden(), art_actual(), [rule])

    def test_only_an_art_rule_takes_lines_or_a_band(self):
        with self.assertRaises(ValueError):
            Rule(golden="g1", region=RECT, kind="data", reason="x", lines=LINES)
        with self.assertRaises(ValueError):
            Rule(golden="g1", region=RECT, kind="data", reason="x", band=BAND)

    def test_overlapping_another_rule_raises(self):
        data = Rule(golden="g1", region={"rows": (5, 5), "cols": (0, 3)}, kind="data", reason="x")
        with self.assertRaises(ValueError):
            compare(art_golden(), art_actual(), [art_rule(), data])


class ArtManifestTests(ManifestFiles):
    def spec(self, **over):
        return dict({"kind": "art", "region": {"rows": list(RECT["rows"]), "cols": list(RECT["cols"])},
                     "band": {"rows": list(BAND["rows"]), "cols": list(BAND["cols"])},
                     "lines": list(LINES), "reason": "DaVinci's own art"}, **over)

    def listed(self, rect=RECT, band=BAND):
        return mock.patch.dict(cmp.ART_REGIONS, {"g1": [{"rect": rect, "band": band}]}, clear=True)

    def test_an_unlisted_golden_cannot_carry_art(self):
        manifest, owners = self.files("structure", [self.spec()])
        with mock.patch.dict(cmp.ART_REGIONS, {}, clear=True):
            with self.assertRaises(ValueError):
                compare_golden(art_golden(), art_actual(), manifest, owners)

    def test_a_listed_rectangle_passes_and_any_other_one_raises(self):
        manifest, owners = self.files("structure", [self.spec()])
        with self.listed():
            self.assertTrue(compare_golden(art_golden(), art_actual(), manifest, owners).ok)
        for rect in ({"rows": (4, 6), "cols": (2, 8)}, {"rows": (3, 6), "cols": (2, 7)},
                     {"rows": (4, 5), "cols": (2, 7)}):
            with self.subTest(rect=rect), self.listed(rect=rect):
                with self.assertRaises(ValueError):
                    compare_golden(art_golden(), art_actual(), manifest, owners)
        with self.listed(band={"rows": (1, 8), "cols": (0, 11)}):
            with self.assertRaises(ValueError):
                compare_golden(art_golden(), art_actual(), manifest, owners)

    def test_art_needs_a_structure_tier(self):
        for tier in ("exact", "exception"):
            manifest, owners = self.files(tier, [self.spec()])
            with self.listed(), self.assertRaises(ValueError):
                compare_golden(art_golden(), art_actual(), manifest, owners)

    def test_the_committed_art_regions_are_the_two_onboarding_goldens(self):
        self.assertEqual(set(cmp.ART_REGIONS), {"onboarding__01_onboarding", "onboarding__03_after_enter"})
        for name, found in cmp.ART_REGIONS.items():
            for item in found:
                (r0, r1), (c0, c1) = item["rect"]["rows"], item["rect"]["cols"]
                # 0-based rows: DaVinci's welcome draws the sprite on rows 10-14 and its dots on 15.
                self.assertEqual((r0, r1), (10, 15), name)
                self.assertTrue(item["band"]["rows"][0] <= r0 and r1 <= item["band"]["rows"][1], name)



class D7Tests(unittest.TestCase):
    def test_the_dropped_regions_are_d7_only(self):
        self.assertEqual(set(cmp.DROPPED_REGIONS), {"D7"})
        box_ = cmp.DROPPED_REGIONS["D7"]
        # The reference's link line, its caption and underlined link: one row, columns 4-55
        # (0-based).
        self.assertEqual((box_["rows"], box_["cols"]), ((24, 24), (4, 55)))

    def test_a_dropped_rule_may_cite_d7(self):
        Rule(golden="g1", region={"rows": (24, 24), "cols": (4, 55)}, kind="dropped",
             reason="D7: no external Claude link")

    def test_the_link_cells_must_be_blank(self):
        link = cap({2: [run(0, "More: "), run(6, "link", underline=True)]})
        rule = Rule(golden="g1", region={"rows": (2, 2), "cols": (6, 9)}, kind="dropped",
                    reason="D7: no external Claude link")
        self.assertTrue(compare(link, cap({2: [run(0, "More: ")]}), [rule]).ok)
        shown = cap({2: [run(0, "More: "), run(6, "x")]})
        self.assertEqual(fields(compare(link, shown, [rule])), {(2, 6, "dropped")})

    def test_the_committed_manifest_drops_only_the_d7_cells(self):
        data = json.loads(Path(cmp.DEFAULT_RULES).read_text(encoding="utf-8"))
        box_, seen = cmp.DROPPED_REGIONS["D7"], 0
        for name, specs in data["goldens"].items():
            for spec in specs:
                if spec["kind"] != "dropped":
                    continue
                seen += 1
                self.assertIn("D7", spec["reason"], name)
                (r0, r1), (c0, c1) = spec["region"]["rows"], spec["region"]["cols"]
                self.assertTrue(box_["rows"][0] <= r0 and r1 <= box_["rows"][1]
                                and box_["cols"][0] <= c0 and c1 <= box_["cols"][1], name)
        self.assertGreater(seen, 0)


if __name__ == "__main__":
    unittest.main()
