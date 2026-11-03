"""P15: the agents dashboard, measured on a live DaVinci.

The reference's dashboard groups sessions into Needs input, Working, Completed
and Pinned, with counters in the header and rename/pin/find/delete on the
selected row. DaVinci's `/agents` now renders the same groups and counters; the
replay has no worker fixture, so the live test covers the empty state, the
counters, the find field and the guarantee that closing the dashboard never
stops work. Row grouping, rename, pin and the two-step delete are covered
deterministically in `crates/davinci-tui/tests/cc2316/panels.rs`.
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
class LiveAgentsTest(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp(prefix="cc2316_p15_"))
        self.addCleanup(shutil.rmtree, self.root, True)
        self.count = 0

    def run_actions(self, actions, settings=None, env=None):
        self.count += 1
        scenario = {"expect_requests": 0, "wait_ms": 45000, "actions": actions,
                    "davinci_settings": {**SETTINGS, **(settings or {})}}
        return replay.replay(BINARY, scenario,
                             replay.Profile(120, 44, env={**ENV, **(env or {})}),
                             out_dir=self.root / f"run{self.count}")

    def test_the_empty_dashboard_names_its_groups_and_counters(self):
        actions = [key("/agents"), wait(800), key(ENTER, 200), wait(1400), shot("open"),
                   key(ESC, 250), wait(600), shot("closed")]
        cap = self.run_actions(actions)
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        opened = screen_text(load(cap.snapshots["open"]))
        self.assertIn("Needs input", opened)
        self.assertIn("Working", opened)
        self.assertIn("Completed", opened)
        self.assertIn("closing this view never stops them", opened)
        self.assertIn("0 awaiting input · 0 working · 0 completed", opened)
        closed = screen_text(load(cap.snapshots["closed"]))
        self.assertNotIn("0 awaiting input", closed)
        self.assertIn("manual mode on", closed)

    def test_the_dashboard_keys_are_safe_without_a_selected_row(self):
        # Ctrl+R, Ctrl+T, Ctrl+X with no row must not crash or delete anything.
        actions = [key("/agents"), wait(800), key(ENTER, 200), wait(1400),
                   key("\x12", 200), wait(400),          # ctrl+r
                   key("\x14", 200), wait(400),          # ctrl+t
                   key("\x18", 200), wait(400),          # ctrl+x
                   shot("after"), key(ESC, 250), wait(600), shot("closed")]
        cap = self.run_actions(actions)
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        after = screen_text(load(cap.snapshots["after"]))
        self.assertIn("0 awaiting input · 0 working · 0 completed", after)
        self.assertIn("manual mode on", screen_text(load(cap.snapshots["closed"])))

    def test_find_filters_and_esc_clears_before_closing(self):
        actions = [key("/agents"), wait(800), key(ENTER, 200), wait(1400),
                   key("\x06", 200), wait(400),          # ctrl+f
                   key("nope", 100), wait(500), shot("filtered"),
                   key(ESC, 200), wait(400), shot("cleared"),
                   key(ESC, 250), wait(600), shot("closed")]
        cap = self.run_actions(actions)
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        filtered = screen_text(load(cap.snapshots["filtered"]))
        self.assertIn("Find: nope", filtered)
        self.assertIn("No agents match", filtered)
        cleared = screen_text(load(cap.snapshots["cleared"]))
        self.assertIn("0 awaiting input", cleared)
        self.assertNotIn("Find: nope", cleared)
        self.assertNotIn("0 awaiting input", screen_text(load(cap.snapshots["closed"])))


    def test_agents_off_the_reference_is_still_the_team_summary(self):
        actions = [key("/agents"), wait(800), key(ENTER, 200), wait(1200), shot("summary")]
        cap = self.run_actions(actions, env={"PI_DAVINCI_REFERENCE": "0"})
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        screen = screen_text(load(cap.snapshots["summary"]))
        self.assertNotIn("0 awaiting input", screen)
        self.assertIn("No agents defined", screen)


if __name__ == "__main__":
    unittest.main()
