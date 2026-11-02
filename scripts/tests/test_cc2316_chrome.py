"""P07: the header, the pinned rows and the geometry, measured on a live DaVinci.

Live tests skip, with a reason, when the binary, pywinpty or the reference package is
missing. They run through ``replay`` with ``PI_DAVINCI_REFERENCE=1`` against the loopback
fixture, and compare each golden with the rules approved in ``cc2316_rules.json``.

What is asserted is every cell except the areas that a later task, or a rule the golden's
tier does not allow, still leaves open. ``owned_areas`` names each one with its owner, and a
test prints the differences by field inside and outside them:

* the startup notice: every golden here carries a ``compact`` rule (D6) with the notice's own
  height (4 rows at 120 and 160 columns, 5 at 100 and 80, 6 at 60), so the conversation closes
  the gap and the vacated rows must be blank. A golden whose notice is scrolled or partly off
  screen would get no rule and leaves the notice rows to no owner, reported to the reviewer;
* the composer row from column 2 on (the caret and the placeholder): P08;
* the footer row (the mode label and the hints beside it): P12b, except in the Ctrl-C golden,
  where the footer is the P07 exit hint and is compared whole.
"""
import json
import os
import shutil
import sys
import tempfile
import unittest
from collections import Counter
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "ui"))
import cc2316_compare as compare  # noqa: E402
import cc2316_replay as replay  # noqa: E402

BINARY = Path(os.environ.get("CC2316_BINARY") or HERE.parents[1] / "target" / "debug" / "davinci.exe")
ENV = {"PI_DAVINCI_REFERENCE": "1"}
SETTINGS = {"defaultThinkingLevel": "medium"}
CTRL_C = "\x03"
ENTER = "\r"
GOLDENS = [
    ("boot__02_settled", 120, 44, []),
    ("layout_sizes__01_120x44", 120, 44, []),
    ("layout_sizes__02_100x30", 100, 30, []),
    ("layout_sizes__03_80x24", 80, 24, []),
    ("layout_sizes__04_60x20", 60, 20, []),
    ("layout_sizes__05_160x50", 160, 50, []),
    ("layout_sizes__06_120x18", 120, 18, []),
    ("keys_panels__08_ctrl_c", 120, 44, [{"type": "key", "text": CTRL_C, "delay_ms": 300}]),
]


def live_skip_reason():
    if not BINARY.is_file():
        return f"no binary at {BINARY}"
    try:
        import winpty  # noqa: F401
    except ImportError:
        return "pywinpty is not installed"
    try:
        replay.load_reference()
    except Exception as exc:  # noqa: BLE001
        return str(exc)
    return ""


def golden(name):
    path = replay.ref_root() / "artifacts" / "frames" / "reference" / f"{name}.json"
    return json.loads(path.read_text(encoding="utf-8")) if path.is_file() else None


def load(path):
    return json.loads(Path(path).read_text(encoding="utf-8"))


def row_text(capture, y):
    return "".join(run["text"] for run in capture["rows"][y])


def notice_rows(expected):
    """The rows of the startup notice: the run of rows after the header that start with the bar."""
    rows = []
    for y in range(5, len(expected["rows"])):
        if not row_text(expected, y).startswith("▎"):
            break
        rows.append(y)
    return rows


def owned_areas(expected, name):
    """(rows, first column, owner, why) for each area this task leaves to others."""
    height = expected["dimensions"]["rows"]
    areas = []
    notice = notice_rows(expected)
    compacted = any(isinstance(rule, compare.Compact) for rule in compare.load_rules(name))
    if notice and not compacted:
        areas.append((set(notice), 0, "none", "startup notice is scrolled or partly off screen"))
    areas.append(({height - 3}, 2, "P08", "caret and placeholder"))
    if name != "keys_panels__08_ctrl_c":
        areas.append(({height - 1}, 0, "P12b", "mode label, hints and context gauge"))
    return areas


def split(report, expected, name):
    """(inside, outside): the differences in an owned area, by owner, and the rest."""
    areas = owned_areas(expected, name)
    inside, outside = {}, []
    for d in report.differences:
        for rows, col, owner, _ in areas:
            if d.row in rows and (d.column is None or d.column >= col):
                inside.setdefault(owner, []).append(d)
                break
        else:
            outside.append(d)
    return inside, outside


def summary(differences):
    return dict(Counter(d.field for d in differences))


def settled_text(capture):
    return "\n".join(row_text(capture, y) for y in range(len(capture["rows"])))


@unittest.skipIf(live_skip_reason(), live_skip_reason() or "")
class LiveChromeTest(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.mkdtemp(prefix="cc2316_p07_")
        self.addCleanup(shutil.rmtree, self.root, True)
        self.count = 0

    def run_scenario(self, cols, rows, actions, settings=SETTINGS, wait_ms=6000):
        self.count += 1
        scenario = {"expect_requests": 0, "wait_ms": wait_ms, "actions": actions,
                    "davinci_settings": settings}
        cap = replay.replay(BINARY, scenario, replay.Profile(cols, rows, env=ENV),
                            out_dir=Path(self.root) / f"run{self.count}")
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        return cap

    def shot(self, cols, rows, steps):
        cap = self.run_scenario(cols, rows, [*steps, {"type": "wait", "ms": 500},
                                             {"type": "snapshot", "name": "s"}])
        return load(cap.snapshots["s"])

    def test_every_golden_matches_outside_the_areas_left_to_others(self):
        for name, cols, rows, steps in GOLDENS:
            with self.subTest(golden=name):
                expected = golden(name)
                if expected is None:
                    self.skipTest(f"no golden {name}")
                actual = self.shot(cols, rows, steps)
                report = compare.compare_golden(expected, actual)
                inside, outside = split(report, expected, name)
                held = {owner: summary(found) for owner, found in inside.items()}
                print(f"\n{name}: {len(report.differences)} differences, "
                      f"{len(outside)} outside the owned areas; owned {held}")
                self.assertEqual(outside, [], "\n" + "\n".join(
                    f"row {d.row} col {d.column} {d.field}: {d.expected!r} != {d.actual!r}"
                    for d in outside[:20]))

    def test_the_header_and_the_footer_cannot_move_by_one_cell_unnoticed(self):
        expected = golden("boot__02_settled")
        if expected is None:
            self.skipTest("no golden boot__02_settled")
        actual = self.shot(120, 44, [])

        def moved(row_from, row_to=None, shift=0):
            copy = json.loads(json.dumps(actual))
            runs = copy["rows"][row_from]
            if shift:
                for run in runs:
                    run["x"] += shift
                runs[:] = [run for run in runs if run["x"] + run["w"] <= 120]
            if row_to is not None:
                copy["rows"][row_to], copy["rows"][row_from] = runs, copy["rows"][row_to]
            return copy

        for label, copy in (
            ("header one column right", moved(1, shift=1)),
            ("header one row down", moved(1, 2)),
            ("top rule one row up", moved(40, 39)),
            ("bottom rule one row down", moved(42, 43)),
        ):
            report = compare.compare_golden(expected, copy)
            _, outside = split(report, expected, "boot__02_settled")
            self.assertTrue(outside, f"{label} was not caught")

    def test_the_mark_is_static_and_the_screen_settles(self):
        first = self.shot(120, 44, [{"type": "wait", "ms": 1500}])
        later = self.shot(120, 44, [{"type": "wait", "ms": 4000}])
        for y in range(0, 6):
            self.assertEqual(first["rows"][y], later["rows"][y], f"row {y} moved")

    def test_a_draft_survives_resizes_with_the_picker_open(self):
        draft = "keep this draft"
        steps = [{"type": "key", "text": draft, "delay_ms": 200},
                 {"type": "resize", "cols": 80, "rows": 24}, {"type": "wait", "ms": 500},
                 {"type": "resize", "cols": 30, "rows": 8}, {"type": "wait", "ms": 500},
                 {"type": "resize", "cols": 120, "rows": 44}, {"type": "wait", "ms": 500}]
        actual = self.shot(120, 44, steps)
        self.assertIn(draft, settled_text(actual))
        self.assertTrue(row_text(actual, 3).startswith(" ")
                        and "DaVinci" in row_text(actual, 1))
        opened = self.shot(120, 44, [{"type": "key", "text": "/theme", "delay_ms": 200},
                                     {"type": "key", "text": ENTER, "delay_ms": 300},
                                     {"type": "wait", "ms": 600},
                                     {"type": "resize", "cols": 60, "rows": 20},
                                     {"type": "wait", "ms": 500},
                                     {"type": "resize", "cols": 20, "rows": 4},
                                     {"type": "wait", "ms": 500},
                                     {"type": "resize", "cols": 100, "rows": 30},
                                     {"type": "wait", "ms": 500}])
        self.assertIn("Theme", settled_text(opened))


ONBOARDING = [
    # The reference frames show syntax highlighting off, yet no key in their scenarios toggled
    # it; the cause is unknown. DaVinci stays syntax-on by default, so for the comparison only
    # the first golden's scenario presses ctrl+t, to reach the state the frame shows.
    ("onboarding__01_onboarding", [{"type": "key", "text": "\x14", "delay_ms": 200}]),
    ("onboarding__03_after_enter", [{"type": "key", "text": ENTER, "delay_ms": 200}]),
]
DOWN = "\x1b[B"


@unittest.skipIf(live_skip_reason(), live_skip_reason() or "")
class LiveOnboardingTest(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.mkdtemp(prefix="cc2316_p07b_")
        self.addCleanup(shutil.rmtree, self.root, True)
        self.count = 0

    def first_run(self, steps, cols=120, rows=44):
        self.count += 1
        scenario = {"expect_requests": 0, "wait_ms": 6000, "first_run": True,
                    "actions": [{"type": "wait", "ms": 1500}, *steps, {"type": "wait", "ms": 500},
                                {"type": "snapshot", "name": "s"}]}
        cap = replay.replay(BINARY, scenario, replay.Profile(cols, rows, env=ENV),
                            out_dir=Path(self.root) / f"run{self.count}")
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        return cap, load(cap.snapshots["s"])

    def test_goldens_report_their_differences(self):
        for name, steps in ONBOARDING:
            with self.subTest(golden=name):
                expected = golden(name)
                if expected is None:
                    self.skipTest(f"no golden {name}")
                _, actual = self.first_run(steps)
                report = compare.compare_golden(expected, actual)
                print(f"\n{name}: {len(report.differences)} differences {summary(report.differences)}")
                print("modes", {k: v for k, v in actual["modes"].items()
                                if k.startswith("mouse") or k == "alternate_screen"})

    def test_the_theme_is_saved_with_the_analytics_answer_and_not_before(self):
        cap, _ = self.first_run([{"type": "key", "text": DOWN, "delay_ms": 200},
                                 {"type": "key", "text": ENTER, "delay_ms": 300},
                                 {"type": "key", "text": ENTER, "delay_ms": 300}])
        self.assertEqual(cap.agent_settings, {}, "a half-answered first run stored something")
        cap, _ = self.first_run([{"type": "key", "text": DOWN, "delay_ms": 200},
                                 {"type": "key", "text": ENTER, "delay_ms": 300},
                                 {"type": "key", "text": ENTER, "delay_ms": 300},
                                 {"type": "key", "text": ENTER, "delay_ms": 300}])
        self.assertEqual(cap.agent_settings.get("theme"), "light")
        self.assertIn("enableAnalytics", cap.agent_settings)

    def test_escape_and_a_resize_partway_store_nothing(self):
        cap, shot = self.first_run([{"type": "key", "text": ENTER, "delay_ms": 300},
                                    {"type": "resize", "cols": 60, "rows": 20},
                                    {"type": "wait", "ms": 300},
                                    {"type": "key", "text": "\x1b", "delay_ms": 300}])
        self.assertEqual(cap.agent_settings, {})
        self.assertNotIn("Security notes", settled_text(shot))


class OwnedAreasTest(unittest.TestCase):
    def capture(self, rows, text_rows):
        return {"dimensions": {"cols": 20, "rows": rows},
                "rows": [[{"x": 0, "w": len(t), "text": t}] if t else [] for t in text_rows]}

    def test_the_notice_is_the_bar_rows_after_the_header(self):
        expected = self.capture(12, ["", "", "", "", "", "▎ a", "▎ b", "", "", "", "", ""])
        self.assertEqual(notice_rows(expected), [5, 6])

    def test_owned_areas_leave_the_prompt_glyph_and_the_exit_footer_compared(self):
        expected = self.capture(12, [""] * 12)
        by_owner = {owner: (rows, col) for rows, col, owner, _ in owned_areas(expected, "boot__02_settled")}
        self.assertEqual(by_owner["P08"], ({9}, 2))
        self.assertEqual(by_owner["P12b"], ({11}, 0))
        exit_owners = {o for _, _, o, _ in owned_areas(expected, "keys_panels__08_ctrl_c")}
        self.assertNotIn("P12b", exit_owners)


if __name__ == "__main__":
    unittest.main()
