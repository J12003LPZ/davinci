"""P11: streaming, interrupt and queue, measured on a live DaVinci.

The mock holds a turn open so the spinner and the body can be read mid-turn. Interrupt
(Esc) keeps the reference's behaviour of sending the queue; Ctrl+C is the exit gesture,
so the queued line waits instead of starting a request the user may be leaving from.
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
SETTINGS = {"defaultThinkingLevel": "medium"}
ESC, ENTER, CTRL_C = "\x1b", "\r", "\x03"
CTRL_X, CTRL_S = "\x18", "\x13"
HELD = "Slow line one.\nSlow line two."


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
class LiveStreamTest(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp(prefix="cc2316_p11_"))
        self.addCleanup(shutil.rmtree, self.root, True)
        self.count = 0

    def run_scenario(self, actions, turns, expect, timeout=45000):
        self.count += 1
        scenario = {"expect_requests": expect, "wait_ms": timeout, "actions": actions,
                    "turns": turns, "davinci_settings": SETTINGS}
        return replay.replay(BINARY, scenario, replay.Profile(120, 44, env=ENV),
                             out_dir=self.root / f"run{self.count}")

    def test_a_held_turn_shows_a_spinner_with_tokens_and_the_streamed_body(self):
        actions = [key("say something long"), wait(900), key(ENTER, 300), wait(2000),
                   shot("s"), {"type": "release"}, wait(2500), shot("done")]
        cap = self.run_scenario(actions, [{"kind": "text", "text": HELD, "hold": True}], 1)
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        live = screen_text(load(cap.snapshots["s"]))
        self.assertIn("Slow line one.", live, "the body did not stream live")
        self.assertIn("tokens", live, "the spinner has no token counter")
        self.assertIn("interrupt", live)
        done = screen_text(load(cap.snapshots["done"]))
        self.assertNotIn("tokens)", done.split("done")[0], "the spinner outlived the turn")

    def test_enter_queues_and_up_takes_the_line_back_for_editing(self):
        actions = [key("first"), wait(900), key(ENTER, 300), wait(1200),
                   key("queued prompt"), wait(900), key(ENTER, 300), wait(800),
                   shot("queued"), key("\x1b[A", 200), wait(500), shot("up"),
                   {"type": "release"}, wait(3000)]
        cap = self.run_scenario(actions, [{"kind": "text", "text": HELD, "hold": True}], 1)
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        queued = screen_text(load(cap.snapshots["queued"]))
        self.assertIn("queued prompt", queued)
        self.assertNotIn("Press up", queued)  # the hint is P12b's footer
        up = screen_text(load(cap.snapshots["up"]))
        # Up moves the newest queued line into the draft and out of the queue.
        self.assertEqual(up.count("queued prompt"), 1, up)

    def test_ctrl_x_ctrl_s_sends_now_and_preempts_the_turn(self):
        actions = [key("first"), wait(900), key(ENTER, 300), wait(1200),
                   key("send now line"), wait(900), key(CTRL_X, 100), key(CTRL_S, 200),
                   wait(1500), shot("s"), {"type": "release"}, wait(4000), shot("done")]
        cap = self.run_scenario(actions, [{"kind": "text", "text": HELD, "hold": True},
                                 {"kind": "text", "text": "Quick answer."}], 2)
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        done = screen_text(load(cap.snapshots["done"]))
        self.assertIn("Slow line one.", done, "the partial answer was lost")
        self.assertIn("send now line", done)
        self.assertIn("Quick answer.", done, "the preempting turn did not run")

    def test_two_ctrl_c_exits_without_sending_the_queued_line(self):
        actions = [key("first"), wait(900), key(ENTER, 300), wait(1200),
                   key("queued line"), wait(900), key(ENTER, 300), wait(800),
                   key(CTRL_C, 200), wait(300), key(CTRL_C, 200), wait(2000)]
        cap = self.run_scenario(actions, [{"kind": "text", "text": HELD, "hold": True},
                                 {"kind": "text", "text": "Queued answer."}], 1)
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        self.assertEqual(cap.exit, "exited", "the second Ctrl+C did not leave")
        self.assertEqual(len(cap.requests), 1, "the exit started a queued request")

    def test_escape_interrupt_still_sends_the_queue_as_the_reference_does(self):
        actions = [key("first"), wait(900), key(ENTER, 300), wait(1200),
                   key("queued line"), wait(900), key(ENTER, 300), wait(800),
                   key(ESC, 200), wait(3000)]
        cap = self.run_scenario(actions, [{"kind": "text", "text": HELD, "hold": True},
                                 {"kind": "text", "text": "Queued answer."}], 2)
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        self.assertEqual(len(cap.requests), 2)


if __name__ == "__main__":
    unittest.main()
