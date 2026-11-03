"""P14: scroll, mouse and selection, measured on a live DaVinci.

The reference frames for these rows are resumed fixture sessions, so the frames
are recorded PARTIAL; these tests drive the behaviours live: page and wheel
scroll, the jump-to-bottom hint, click-to-place-caret, tool expansion (a second
click does not collapse), and selection copying in conversation space.
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
ENTER = "\r"
PAGE_UP, PAGE_DOWN = "\x1b[5~", "\x1b[6~"


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


def mouse(action, x, y):
    return {"type": "mouse", "action": action, "x": x, "y": y}


LONG = ("Turn {n}. " + "This is a scrolling fixture sentence. " * 40).strip()


@unittest.skipIf(live_skip_reason(), live_skip_reason() or "")
class LiveMouseTest(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp(prefix="cc2316_p14_"))
        self.addCleanup(shutil.rmtree, self.root, True)
        self.count = 0

    def run_actions(self, actions, turns, expect, files=None):
        self.count += 1
        scenario = {"expect_requests": expect, "wait_ms": 60000, "actions": actions,
                    "turns": turns, "files": files or {},
                    "davinci_settings": SETTINGS}
        return replay.replay(BINARY, scenario, replay.Profile(120, 44, env=ENV),
                             out_dir=self.root / f"run{self.count}")

    def filled(self):
        """Three long answers so the conversation is taller than the screen."""
        actions = []
        turns = []
        for n in (1, 2, 3):
            actions += [key(f"prompt {n}"), wait(700), key(ENTER, 200), wait(3000)]
            turns.append({"kind": "text", "text": LONG.format(n=n)})
        return actions, turns

    def test_page_and_wheel_scroll_and_the_jump_hint(self):
        actions, turns = self.filled()
        actions += [shot("bottom"), mouse("wheel_up", 10, 20), wait(600),
                    shot("wheel"), key(PAGE_UP, 200), wait(600), shot("up"),
                    {"type": "click_text", "text": "Jump to bottom"}, wait(700),
                    shot("jumped")]
        cap = self.run_actions(actions, turns, 3)
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        bottom = screen_text(load(cap.snapshots["bottom"]))
        self.assertIn("Turn 3.", bottom)
        wheel = screen_text(load(cap.snapshots["wheel"]))
        self.assertNotEqual(wheel, bottom, "the wheel did not scroll")
        up = screen_text(load(cap.snapshots["up"]))
        self.assertNotEqual(up, wheel, "PageUp did not scroll")
        self.assertIn("Jump to bottom", up)
        jumped = screen_text(load(cap.snapshots["jumped"]))
        self.assertIn("Turn 3.", jumped)
        self.assertNotIn("Jump to bottom", jumped)

    def test_a_click_places_the_caret_and_typing_lands_there(self):
        actions = [wait(500), {"type": "click_text", "text": "❯", "dx": 10},
                   wait(400), key("z", 100), wait(500), shot("typed")]
        cap = self.run_actions(actions, [], 0)
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        screen = screen_text(load(cap.snapshots["typed"]))
        self.assertIn("z", screen)
        composer = next(row_text(load(cap.snapshots["typed"]), y) for y in range(44)
                        if row_text(load(cap.snapshots["typed"]), y).lstrip().startswith("❯"))
        self.assertIn("z", composer, composer)

    def test_a_tool_row_expands_on_click_and_a_second_click_does_not_collapse(self):
        actions = [key("read it"), wait(700), key(ENTER, 200), wait(3500),
                   shot("collapsed"),
                   {"type": "click_text", "text": "Read 1 file"}, wait(700),
                   shot("expanded"),
                   {"type": "click_text", "text": "Read("}, wait(700), shot("again"),
                   {"type": "click_text", "text": "Read("}, wait(700), shot("twice")]
        cap = self.run_actions(actions,
                               [{"kind": "tool", "tool": "read",
                                 "arguments": {"path": "fixture.txt"}},
                                {"kind": "text", "text": "Done."}],
                               2, files={"fixture.txt": "line one\nline two\n"})
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        collapsed = screen_text(load(cap.snapshots["collapsed"]))
        self.assertIn("Read 1 file", collapsed)
        expanded = screen_text(load(cap.snapshots["expanded"]))
        self.assertIn("line two", expanded, "a click did not expand the tool output")
        again = screen_text(load(cap.snapshots["again"]))
        self.assertIn("line two", again, "a click collapsed tool output")
        twice = screen_text(load(cap.snapshots["twice"]))
        self.assertIn("line two", twice, "a second click collapsed tool output")

    def test_dragging_selects_and_copies(self):
        actions, turns = self.filled()
        actions += [key(PAGE_UP, 200), wait(600), shot("before"),
                    mouse("left", 10, 10), mouse("drag", 10, 16),
                    mouse("release", 10, 16), wait(700), shot("selected")]
        cap = self.run_actions(actions, turns, 3)
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        selected = screen_text(load(cap.snapshots["selected"]))
        self.assertIn("copied", selected, "the selection was not copied")
        self.assertIn("chars to clipboard", selected)

    def test_a_selection_survives_a_resize(self):
        actions, turns = self.filled()
        actions += [key(PAGE_UP, 200), wait(600),
                    mouse("left", 10, 10), mouse("drag", 10, 16),
                    {"type": "resize", "cols": 80, "rows": 24, "settle_ms": 1000},
                    mouse("release", 10, 16), wait(800), shot("resized")]
        cap = self.run_actions(actions, turns, 3)
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        resized = screen_text(load(cap.snapshots["resized"]))
        self.assertIn("copied", resized, "a resize dropped the selection")
        self.assertIn("chars to clipboard", resized)


if __name__ == "__main__":
    unittest.main()
