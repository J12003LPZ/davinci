"""P09b: the two bindings the plan names, measured on a live DaVinci.

`keys_panels__05_todos` was captured with ctrl+t and no todos, so the frame is the
ordinary screen; the task band must not invent rows. The transcript viewer is a live
surface check: ctrl+o opens it, Escape closes it, and the draft behind it stays.
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
import cc2316_compare as compare  # noqa: E402
import cc2316_replay as replay  # noqa: E402

BINARY = Path(os.environ.get("CC2316_BINARY") or HERE.parents[1] / "target" / "debug" / "davinci.exe")
ENV = {"PI_DAVINCI_REFERENCE": "1"}
SETTINGS = {"defaultThinkingLevel": "medium"}
CTRL_T, CTRL_O, ESC = "\x14", "\x0f", "\x1b"
FOOTER = 43


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


def screen_text(capture):
    return "\n".join(row_text(capture, y) for y in range(len(capture["rows"])))


def key(text, delay=0):
    return {"type": "key", "text": text, "delay_ms": delay}


def wait(ms):
    return {"type": "wait", "ms": ms}


def shot(name):
    return {"type": "snapshot", "name": name}


@unittest.skipIf(live_skip_reason(), live_skip_reason() or "")
class LiveKeysTest(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp(prefix="cc2316_p09b_"))
        self.addCleanup(shutil.rmtree, self.root, True)
        self.count = 0

    def run_actions(self, actions, settings=None, env=None):
        self.count += 1
        scenario = {"expect_requests": 0, "wait_ms": 45000, "actions": actions,
                    "davinci_settings": {**SETTINGS, **(settings or {})}}
        return replay.replay(BINARY, scenario,
                             replay.Profile(120, 44, env={**ENV, **(env or {})}),
                             out_dir=self.root / f"run{self.count}")

    def snapshot(self, actions, **kw):
        cap = self.run_actions([*actions, wait(400), shot("s")], **kw)
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        return load(cap.snapshots["s"])

    def test_ctrl_t_opens_the_task_band_and_the_empty_band_matches_the_reference(self):
        cap = self.run_actions([key(CTRL_T), wait(600), shot("keys_panels__05_todos")])
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        expected = golden("keys_panels__05_todos")
        if expected is None:
            self.skipTest("no golden keys_panels__05_todos")
        report = compare.compare_golden(expected, load(cap.snapshots["keys_panels__05_todos"]))
        unowned = [d for d in report.differences if d.row != FOOTER]
        print(f"\nkeys_panels__05_todos: {len(report.differences)} differences, "
              f"{len(unowned)} outside the footer")
        self.assertEqual(unowned, [], "\n".join(
            f"row {d.row} col {d.column} {d.field}: {d.expected!r} != {d.actual!r}"
            for d in unowned[:20]))

    def test_ctrl_o_opens_the_transcript_and_escape_closes_it(self):
        shown = self.snapshot([key("keep this draft"), wait(300), key(CTRL_O), wait(500)])
        text = screen_text(shown)
        self.assertIn("Transcript", text)
        # The viewer owns the screen while open; Escape gives the shell back.
        shown = self.snapshot([key(CTRL_O), wait(500), key(ESC), wait(400)])
        text = screen_text(shown)
        self.assertNotIn("Transcript", text)
        self.assertIn("DaVinci", text)

    def test_ctrl_t_and_ctrl_o_are_the_same_bindings_off_the_reference(self):
        shown = self.snapshot([key(CTRL_T), wait(400)], env={"PI_DAVINCI_REFERENCE": "0"})
        self.assertNotIn("Tasks", screen_text(shown))
        shown = self.snapshot([key(CTRL_O), wait(500)], env={"PI_DAVINCI_REFERENCE": "0"})
        self.assertIn("Transcript", screen_text(shown))


if __name__ == "__main__":
    unittest.main()
