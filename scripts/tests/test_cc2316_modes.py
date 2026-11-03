"""P12b: the permission-mode cycle and its footer, measured on a live DaVinci.

The reference cycles manual, accept edits, plan and auto with shift+tab, and its
footer always ends with the agents hint. DaVinci's always-approve is not one of
those stops. The four `keys_panels__0*_mode*` frames compare outside the release
notice (D6) and the mark/identity rules.
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
SHIFT_TAB = "\x1b[Z"
MODES = [
    ("keys_panels__01_mode1", 0, "⏸ manual mode on", " · ? for shortcuts · ← for agents"),
    ("keys_panels__02_mode2", 1, "⏵⏵ accept edits on", " (shift+tab to cycle) · ← for agents"),
    ("keys_panels__03_mode3", 2, "⏸ plan mode on", " (shift+tab to cycle) · ← for agents"),
    ("keys_panels__04_mode4", 3, "⏵⏵ auto mode on", " (shift+tab to cycle) · ← for agents"),
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
class LiveModeTest(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp(prefix="cc2316_p12b_"))
        self.addCleanup(shutil.rmtree, self.root, True)
        self.count = 0

    def run_actions(self, actions, env=None):
        self.count += 1
        scenario = {"expect_requests": 0, "wait_ms": 45000, "actions": actions,
                    "davinci_settings": SETTINGS}
        return replay.replay(BINARY, scenario,
                             replay.Profile(120, 44, env={**ENV, **(env or {})}),
                             out_dir=self.root / f"run{self.count}")

    def test_shift_tab_cycles_the_four_reference_modes(self):
        for golden, presses, label, hint in MODES:
            actions = [wait(500)]
            for _ in range(presses):
                actions += [key(SHIFT_TAB, 250), wait(450)]
            actions.append(shot(golden))
            cap = self.run_actions(actions)
            self.assertEqual(cap.status, replay.PASS, cap.reason)
            footer = row_text(load(cap.snapshots[golden]), 43).rstrip()
            self.assertTrue(footer.startswith(f"  {label}"), f"{golden}: {footer!r}")
            self.assertTrue(footer.endswith(hint), f"{golden}: {footer!r}")
            self.assertNotIn("always approve", footer)

    def test_the_four_mode_goldens_match_outside_the_notice(self):
        for golden, presses, _, _ in MODES:
            actions = [wait(500)]
            for _ in range(presses):
                actions += [key(SHIFT_TAB, 250), wait(450)]
            actions.append(shot(golden))
            cap = self.run_actions(actions)
            self.assertEqual(cap.status, replay.PASS, cap.reason)
            expected = load(replay.ref_root()
                            / "artifacts/frames/reference" / f"{golden}.json")
            report = compare.compare_golden(expected, load(cap.snapshots[golden]))
            print(f"\n{golden}: {len(report.differences)} differences")
            self.assertEqual(report.differences, [], "\n".join(
                f"row {d.row} col {d.column} {d.field}: {d.expected!r} != {d.actual!r}"
                for d in report.differences[:20]))

    def test_always_approve_is_never_a_reference_stop(self):
        actions = [wait(500)]
        for i in range(8):
            actions += [key(SHIFT_TAB, 120), wait(300), shot(f"s{i}")]
        cap = self.run_actions(actions)
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        for name in cap.snapshots:
            self.assertNotIn("always approve", screen_text(load(cap.snapshots[name])))

    def test_off_mode_keeps_the_five_mode_cycle(self):
        actions = [wait(500)]
        for i in range(4):
            actions += [key(SHIFT_TAB, 120), wait(300)]
        actions.append(shot("s"))
        cap = self.run_actions(actions, env={"PI_DAVINCI_REFERENCE": "0"})
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        footer = row_text(load(cap.snapshots["s"]), 43).rstrip()
        self.assertIn("always approve on", footer, footer)

    def test_the_draft_survives_every_mode_change(self):
        actions = [key("keep me"), wait(400)]
        for i in range(4):
            actions += [key(SHIFT_TAB, 150), wait(350), shot(f"s{i}")]
        cap = self.run_actions(actions)
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        for name in cap.snapshots:
            screen = screen_text(load(cap.snapshots[name]))
            self.assertIn("keep me", screen, f"{name}: the draft was lost")


if __name__ == "__main__":
    unittest.main()
