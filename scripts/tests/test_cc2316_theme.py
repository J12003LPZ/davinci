"""P06 / P06b: the reference palette and the /theme picker, measured on a live DaVinci.

Live tests skip, with a reason, when the binary, pywinpty or the reference package is
missing. They run through ``replay`` with ``PI_DAVINCI_REFERENCE=1`` against the loopback
fixture. Full parity is not expected until P07 lands the layout, so each golden's
differences are reported by field, and only what P06 owns is asserted: the colours of cells
identified by their text, and that no truecolor or 256-colour SGR appears at ANSI depth.
"""
import json
import os
import re
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
DOWN, ENTER, CTRL_T, ESC = "\x1b[B", "\r", "\x14", "\x1b"
# Truecolor and 256-colour SGR, as colon- or semicolon-separated parameters.
EXTENDED_SGR = re.compile(rb"\x1b\[[0-9;:]*\b[34]8[;:][25][;:]")
TRUECOLOR_SGR = re.compile(rb"\x1b\[[0-9;:]*\b[34]8[;:]2[;:]")


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


def runs(capture):
    """(row, run) for every run with visible text."""
    for y, row in enumerate(capture["rows"]):
        for run in row:
            if run["text"].strip():
                yield y, run


def picker_top(capture):
    """The row of the picker's heading. Above it sit the header and the composer, which
    have a ``❯`` and a name of their own."""
    for y, row in enumerate(capture["rows"]):
        if "".join(run["text"] for run in row).strip() == "Theme":
            return y
    raise AssertionError("the picker is not on screen")


def find(capture, text, exact=True):
    """The first run, from the picker's heading down, whose stripped text is ``text``
    (or starts with it)."""
    top = picker_top(capture)
    for y, run in runs(capture):
        if y < top:
            continue
        stripped = run["text"].strip()
        if stripped == text or (not exact and stripped.startswith(text)):
            return run
    raise AssertionError(f"no run with text {text!r}")


def row_of(capture, text):
    top = picker_top(capture)
    for y, row in enumerate(capture["rows"]):
        if y >= top and text in "".join(run["text"] for run in row):
            return row
    raise AssertionError(f"no row containing {text!r}")


def summary(report):
    """Differences by field, for the evidence log."""
    return dict(Counter(d.field for d in report.differences))


def open_picker(*steps):
    return [{"type": "key", "text": "/theme"}, {"type": "key", "text": ENTER, "delay_ms": 300},
            {"type": "wait", "ms": 600}, *steps]


def snap(name):
    return {"type": "snapshot", "name": name}


def press(text):
    return [{"type": "key", "text": text, "delay_ms": 200}, {"type": "wait", "ms": 400}]


class ExtendedSgrTest(unittest.TestCase):
    def test_the_truecolor_detector_ignores_256_colour(self):
        self.assertTrue(TRUECOLOR_SGR.search(b"[1;38;2;1;2;3m"))
        self.assertFalse(TRUECOLOR_SGR.search(b"[38;5;9;49m"))

    def test_the_detector_sees_truecolor_and_256_but_not_named_colours(self):
        self.assertTrue(EXTENDED_SGR.search(b"\x1b[38;2;1;2;3m"))
        self.assertTrue(EXTENDED_SGR.search(b"\x1b[1;48;5;22m"))
        self.assertTrue(EXTENDED_SGR.search(b"\x1b[38:2::1:2:3m"))
        self.assertFalse(EXTENDED_SGR.search(b"\x1b[91m\x1b[2;37m\x1b[49m\x1b[?1049h"))


@unittest.skipIf(live_skip_reason(), live_skip_reason() or "")
class LiveThemeTest(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.mkdtemp(prefix="cc2316_p06_")
        self.addCleanup(shutil.rmtree, self.root, True)
        self.count = 0

    def run_scenario(self, actions, settings=None, env=None):
        self.count += 1
        scenario = {"expect_requests": 0, "wait_ms": 6000, "actions": actions}
        if settings:
            scenario["davinci_settings"] = settings
        cap = replay.replay(BINARY, scenario, replay.Profile(120, 46, env=env or ENV),
                            out_dir=Path(self.root) / f"run{self.count}")
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        return cap

    def compare(self, cap, snapshot, name):
        expected = golden(name)
        if expected is None:
            self.skipTest(f"no golden {name}")
        report = compare.compare_golden(expected, load(cap.snapshots[snapshot]))
        print(f"\n{name}: {len(report.differences)} differences {summary(report)}")
        return report

    def test_nav_dark_picker_inks(self):
        cap = self.run_scenario(open_picker(snap("01_theme")))
        self.compare(cap, "01_theme", "theme_nav__01_theme")
        shot = load(cap.snapshots["01_theme"])
        self.assertEqual(find(shot, "❯")["fg"], "#b1b9f9")
        self.assertEqual(find(shot, "✔")["fg"], "#4eba65")
        self.assertTrue(find(shot, "Theme")["bold"])
        self.assertEqual(find(shot, "Theme")["fg"], "#b1b9f9")
        self.assertEqual(find(shot, "╌", exact=False)["fg"], "#505050")
        self.assertTrue(find(shot, "Enter to select", exact=False)["italic"])
        # Syntax on: the dark tints with their word-level shade.
        plus = {run["bg"] for run in row_of(shot, "total")}
        minus = {run["bg"] for run in row_of(shot, "count")}
        self.assertTrue({"#022800", "#044700"} <= plus, plus)
        self.assertTrue({"#3d0100", "#5c0200"} <= minus, minus)

    def test_nav_colorblind_preview_and_syntax_toggle(self):
        steps = [*press(DOWN) * 3, snap("02_down2"), *press(CTRL_T), snap("03_syntax_toggle")]
        cap = self.run_scenario(open_picker(*steps))
        self.compare(cap, "02_down2", "theme_nav__02_down2")
        self.compare(cap, "03_syntax_toggle", "theme_nav__03_syntax_toggle")
        down = load(cap.snapshots["02_down2"])
        self.assertEqual(find(down, "❯")["fg"], "#99ccff")
        self.assertEqual(find(down, "✔")["fg"], "#3399ff")
        plus = {run["bg"] for run in row_of(down, "total")}
        self.assertTrue({"#001b29", "#003047"} <= plus, plus)
        toggled = load(cap.snapshots["03_syntax_toggle"])
        plus = {run["bg"] for run in row_of(toggled, "total")}
        minus = {run["bg"] for run in row_of(toggled, "count")}
        self.assertTrue({"#004466", "#0077b3"} <= plus, plus)
        self.assertTrue({"#660000", "#b30000"} <= minus, minus)
        self.assertIn("disabled", "".join(run["text"] for run in row_of(toggled, "ctrl+t")))

    def test_apply_light(self):
        steps = [*press(DOWN), snap("01_light_selected"), *press(ENTER), snap("02_light_main")]
        cap = self.run_scenario(open_picker(*steps))
        self.compare(cap, "01_light_selected", "theme_apply__01_light_selected")
        self.compare(cap, "02_light_main", "theme_apply__02_light_main")
        selected = load(cap.snapshots["01_light_selected"])
        self.assertEqual(find(selected, "❯")["fg"], "#5769f7")
        self.assertEqual(find(selected, "✔")["fg"], "#2c7a39")
        self.assertEqual(find(selected, "╌", exact=False)["fg"], "#afafaf")
        main = load(cap.snapshots["02_light_main"])
        text = "\n".join("".join(run["text"] for run in row) for row in main["rows"])
        self.assertNotIn("Enter to select", text, "the picker is still open after Enter")
        self.assertEqual(cap.agent_settings.get("theme"), "light", "Enter did not save the theme")

    def test_escape_leaves_the_saved_theme_alone(self):
        steps = [*press(DOWN), snap("01_light_selected"), *press(ESC), snap("02_closed")]
        cap = self.run_scenario(open_picker(*steps), settings={"theme": "dark"})
        closed = load(cap.snapshots["02_closed"])
        text = "\n".join("".join(run["text"] for run in row) for row in closed["rows"])
        self.assertNotIn("Enter to select", text, "the picker is still open after Esc")
        self.assertEqual(cap.agent_settings.get("theme"), "dark", "Esc changed the saved theme")

    def test_ansi_depth_emits_no_extended_colour(self):
        cap = self.run_scenario([snap("settled")], settings={"theme": "ansi"})
        self.compare(cap, "settled", "ansi_theme__02_settled")
        raw = cap.raw_path.read_bytes()
        self.assertEqual(TRUECOLOR_SGR.findall(raw), [], "truecolor SGR at ANSI depth")

    def test_ansi_depth_emits_no_256_colour_sgr(self):
        # ANSI-01: the sixteen named colours are written as 30-37, 90-97, 40-47 and
        # 100-107, as the reference writes them, not as ``38;5;N`` / ``48;5;N``.
        cap = self.run_scenario([snap("settled")], settings={"theme": "ansi"})
        self.assertEqual(EXTENDED_SGR.findall(cap.raw_path.read_bytes()), [])


if __name__ == "__main__":
    unittest.main()
