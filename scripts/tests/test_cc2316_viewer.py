"""P14b: the detailed transcript viewer, measured on a live DaVinci.

The reference's detailed transcript is a mode of the conversation (ctrl+o), with
timestamps and the model name beside each answer. DaVinci's ctrl+o opens a
read-only viewer of the same transcript with every tool result expanded; this
checks its header, the model name, verbose content, g/G and close keys. The
frame itself is recorded PARTIAL because the reference's timestamps come from
the resumed fixture session.
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
CTRL_O, ENTER = "\x0f", "\r"
LONG = ("Turn {n}. " + "This is a scrolling fixture sentence. " * 16).strip()


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
class LiveViewerTest(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp(prefix="cc2316_p14b_"))
        self.addCleanup(shutil.rmtree, self.root, True)
        self.count = 0

    def test_the_viewer_has_a_header_model_and_g_G(self):
        self.count += 1
        actions = []
        turns = []
        for n in (1, 2):
            actions += [key(f"prompt {n}"), wait(700), key(ENTER, 200), wait(3000)]
            turns.append({"kind": "text", "text": LONG.format(n=n)})
        actions += [key(CTRL_O, 200), wait(800), shot("top"),
                    key("G", 200), wait(500), shot("bottom"),
                    key("g", 200), wait(500), shot("back"),
                    key("q", 200), wait(500), shot("closed")]
        scenario = {"expect_requests": 2, "wait_ms": 60000, "actions": actions,
                    "turns": turns, "davinci_settings": SETTINGS}
        cap = replay.replay(BINARY, scenario, replay.Profile(120, 44, env=ENV),
                            out_dir=self.root / f"run{self.count}")
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        top = screen_text(load(cap.snapshots["top"]))
        self.assertIn("Transcript", top)
        self.assertIn("Demo", top, "the header lacks the model name")
        self.assertIn("Showing detailed transcript", top)
        self.assertIn("Turn 1.", top)
        bottom = screen_text(load(cap.snapshots["bottom"]))
        self.assertIn("Turn 2.", bottom)
        back = screen_text(load(cap.snapshots["back"]))
        self.assertIn("Turn 1.", back)
        self.assertNotIn("Transcript", screen_text(load(cap.snapshots["closed"])))


if __name__ == "__main__":
    unittest.main()
