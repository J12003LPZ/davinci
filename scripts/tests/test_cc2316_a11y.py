"""P17: accessibility, measured on a live DaVinci.

The reference's screen-reader mode is line-oriented text with a banner line and a
`$` prompt; its reduced-motion toggle is a settings row. DaVinci now has both:
`--screen-reader` renders the plain layout, and the Reduce motion row persists
`reduceMotion` and stops the spinner animation.
"""
import json
import os
import shutil
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "ui"))
import cc2316_replay as replay  # noqa: E402

BINARY = Path(os.environ.get("CC2316_BINARY") or HERE.parents[1] / "target" / "debug" / "davinci.exe")
ENV = {"PI_DAVINCI_REFERENCE": "1"}
SETTINGS = {"defaultThinkingLevel": "medium", "autoVerify": False}
ESC, ENTER = "\x1b", "\r"


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


def load(path):
    return json.loads(Path(path).read_text(encoding="utf-8"))


def row_text(capture, y):
    return "".join(run["text"] for run in capture["rows"][y])


def screen_text(capture):
    return "\n".join(row_text(capture, y) for y in range(len(capture["rows"])))


def key(text, delay=0):
    return {"type": "key", "text": text, "delay_ms": delay}


def wait(ms):
    return {"type": "wait", "ms": ms}


def shot(name):
    return {"type": "snapshot", "name": name}


@unittest.skipIf(live_skip_reason(), live_skip_reason() or "")
class LiveA11yTest(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp(prefix="cc2316_p17_"))
        self.addCleanup(shutil.rmtree, self.root, True)
        self.count = 0

    def run_actions(self, actions, args=None, settings=None):
        self.count += 1
        scenario = {"expect_requests": 0, "wait_ms": 45000, "actions": actions,
                    "args": args or [], "davinci_settings": {**SETTINGS, **(settings or {})}}
        return replay.replay(BINARY, scenario, replay.Profile(120, 44, env=ENV),
                             out_dir=self.root / f"run{self.count}")

    def test_screen_reader_mode_is_plain_lines(self):
        cap = self.run_actions([wait(1200), shot("s")], args=["--screen-reader"])
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        screen = screen_text(load(cap.snapshots["s"]))
        self.assertIn("[Screen Reader Mode: on via flag]", screen)
        self.assertIn("DaVinci v1.1.7", screen)
        self.assertIn("Demo · local", screen)
        self.assertIn("effort: medium · /effort", screen)
        self.assertIn("$", screen)
        self.assertNotIn("▐", screen)
        self.assertNotIn("█", screen)

    def test_reduce_motion_toggles_and_persists(self):
        actions = [key("/config"), wait(800), key(ENTER, 200), wait(1200),
                   key("motion", 100), wait(600), shot("found"),
                   key(ENTER, 200), wait(300), key(" ", 200), wait(800), shot("toggled"),
                   key(ESC, 250), wait(500), shot("closed")]
        cap = self.run_actions(actions)
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        found = screen_text(load(cap.snapshots["found"]))
        self.assertIn("Reduce motion", found)
        self.assertIn("false", found)
        toggled = screen_text(load(cap.snapshots["toggled"]))
        self.assertRegex(toggled, r"Reduce motion\s+true")
        self.assertEqual(cap.agent_settings.get("reduceMotion"), True)


if __name__ == "__main__":
    unittest.main()
