"""P05: terminal modes and restore, measured on a live DaVinci through the replay.

Live tests skip, with a reason, when the binary, pywinpty or the reference package is
missing. Synthetic prompts only. The reference's goldens and raw recording are read in
place and never copied.

Set ``CC2316_BASE_BINARY`` to a davinci built before P05 to also compare the setting-off
mode sequences with it.
"""
import json
import os
import re
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
BASE_BINARY = Path(os.environ["CC2316_BASE_BINARY"]) if os.environ.get("CC2316_BASE_BINARY") else None
MODE_RE = re.compile(rb"\x1b\[\?([0-9;]+)([hl])")
# The modes the reference holds at fullscreen, and the ones P05 negotiates.
HELD = (1049, 1000, 1002, 1003, 1006, 1004, 2004)
# ConPTY sets these itself and puts them in the stream, also after DaVinci turned focus
# reporting off (the pre-P05 binary writes ``?1004l`` and ConPTY answers ``?1004h``).
# They are not DaVinci's to restore; ``?1004l`` is checked separately.
CONPTY_OWNED = {1004, 9001}
SETTLED = {"expect_requests": 0, "wait_ms": 8000, "actions": [{"type": "snapshot", "name": "settled"}]}
# ctrl+c at rest clears the draft and arms the exit; a second one within 2 s leaves.
QUIT = [{"type": "key", "text": "\x03"}, {"type": "key", "text": "\x03", "delay_ms": 300},
        {"type": "wait", "ms": 1500}]


def mode_events(raw: bytes):
    """(mode, on) for every DEC private mode sequence in order. ``?1003;1006h`` is two."""
    for params, final in MODE_RE.findall(raw):
        for mode in params.split(b";"):
            yield int(mode), final == b"h"


def final_modes(raw: bytes) -> dict:
    state = {}
    for mode, on in mode_events(raw):
        state[mode] = on
    return state


def ever_on(raw: bytes) -> set:
    return {mode for mode, on in mode_events(raw) if on}


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


class ModeParsingTest(unittest.TestCase):
    def test_a_combined_sequence_sets_each_mode(self):
        raw = b"\x1b[?1003;1006h\x1b[?1000h\x1b[?1003l"
        self.assertEqual(final_modes(raw), {1003: False, 1006: True, 1000: True})

    def test_the_last_sequence_wins(self):
        self.assertEqual(final_modes(b"\x1b[?2004h\x1b[?2004l\x1b[?2004h"), {2004: True})
        self.assertEqual(ever_on(b"\x1b[?2004h\x1b[?2004l"), {2004})


@unittest.skipIf(live_skip_reason(), live_skip_reason() or "")
class LiveTerminalTest(unittest.TestCase):
    def setUp(self):
        self.root = tempfile.mkdtemp(prefix="cc2316_p05_")
        self.addCleanup(shutil.rmtree, self.root, True)
        self.count = 0

    def run_scenario(self, scenario, *, reference, binary=BINARY, env=None, check=True):
        """Run once with ``tui.reference`` off or on (through the stored setting)."""
        self.count += 1
        scenario = dict(scenario, davinci_settings={"tui": {"reference": reference}})
        cap = replay.replay(binary, scenario, replay.Profile(120, 44, env=env or {}),
                            out_dir=Path(self.root) / f"run{self.count}")
        if check:
            self.assertEqual(cap.status, replay.PASS, cap.reason)
        return cap

    def raw(self, cap) -> bytes:
        return cap.raw_path.read_bytes()

    def settled_raw(self, cap) -> bytes:
        return (cap.out_dir / "settled.full.raw").read_bytes()

    def assert_modes_off(self, raw: bytes, reached: set):
        """Everything the run turned on is off at the end, and the cursor is back."""
        state = final_modes(raw)
        still_on = sorted(mode for mode in reached - {25} - CONPTY_OWNED if state.get(mode))
        self.assertEqual(still_on, [], f"modes left on at the end: {still_on}")
        self.assertTrue(state.get(25, True), "the cursor was left hidden")
        self.assertFalse(state.get(2026), "a synchronized frame was left open")
        if 1004 in reached:
            # ConPTY answers ``?1004l`` with its own ``?1004h``, so only the order is DaVinci's.
            self.assertGreater(raw.rfind(b"\x1b[?1004l"), raw.rfind(b"\x1b[?2004h"),
                               "focus reporting was not turned off at exit")

    # ------------------------------------------------------------ setting on
    def test_setting_on_settled_boot_matches_the_reference_modes(self):
        golden = replay.ref_root() / "artifacts" / "frames" / "reference" / "boot__02_settled.json"
        if not golden.is_file():
            self.skipTest(f"no golden at {golden}")
        expected = json.loads(golden.read_text(encoding="utf-8"))
        cap = self.run_scenario(SETTLED, reference=True)
        actual = json.loads(cap.snapshots["settled"].read_text(encoding="utf-8"))
        report = compare.compare_golden(expected, actual)
        # The cell differences are expected until P07; the terminal state is not.
        terminal = [d for d in report.differences if d.field.split(".")[0] in ("modes", "buffer", "cursor")]
        self.assertEqual(terminal, [], str(compare.DiffReport(terminal)))
        self.assertEqual(actual["buffer"]["type"], "alternate")
        self.assertIs(actual["cursor"]["hidden"], True)

    def test_paste_and_focus_come_from_the_full_recording_like_the_reference(self):
        reference = replay.ref_root() / "artifacts" / "tests" / "boot" / "raw.ansi"
        if not reference.is_file():
            self.skipTest(f"no recording at {reference}")
        want = final_modes(reference.read_bytes())
        cap = self.run_scenario(SETTLED, reference=True)
        got = final_modes(self.settled_raw(cap))
        for mode in (2004, 1004):
            self.assertTrue(want[mode], f"the reference left {mode} off")
            self.assertIs(got.get(mode), True, f"?{mode}h is not on in the full recording")
        for mode in HELD:
            self.assertEqual(got.get(mode), want.get(mode), f"mode {mode}")
        self.assertFalse(got.get(2026), "a frame was left open when the screen settled")

    # ----------------------------------------------------------- setting off
    def test_setting_off_asks_for_no_reference_modes(self):
        cap = self.run_scenario(SETTLED, reference=False)
        raw = self.settled_raw(cap)
        singles = [m for m in re.findall(rb"\x1b\[\?(1000|1002|1003|1006)[hl]", raw)]
        self.assertEqual(singles, [], "the reference's separate mouse sequences leaked")

    @unittest.skipUnless(BASE_BINARY and BASE_BINARY.is_file(), "CC2316_BASE_BINARY is not set")
    def test_setting_off_writes_the_same_mode_sequences_as_before_p05(self):
        def sequences(raw):
            return [(m, f) for m, f in MODE_RE.findall(raw) if m != b"25"]

        now = self.run_scenario(SETTLED, reference=False)
        before = self.run_scenario(SETTLED, reference=False, binary=BASE_BINARY)
        self.assertEqual(sequences(self.settled_raw(now))[:12], sequences(self.settled_raw(before))[:12])
        self.assertEqual(final_modes(self.settled_raw(now)), final_modes(self.settled_raw(before)))

    # --------------------------------------------------------------- restore
    def test_normal_exit_turns_every_mode_off(self):
        for reference in (True, False):
            with self.subTest(reference=reference):
                cap = self.run_scenario(dict(SETTLED, actions=QUIT), reference=reference)
                self.assertEqual(cap.exit, "exited")
                raw = self.raw(cap)
                self.assertTrue(set(HELD) & ever_on(raw) >= {1049, 2004, 1004})
                self.assert_modes_off(raw, ever_on(raw))
                if reference:
                    self.assertLessEqual({1000, 1002, 1003, 1006}, ever_on(raw))

    def test_a_panic_turns_every_mode_off(self):
        env = {"PI_DAVINCI_PANIC_FIXTURE": "1"}
        for reference in (True, False):
            with self.subTest(reference=reference):
                cap = self.run_scenario({"expect_requests": 0, "wait_ms": 6000, "actions": []},
                                        reference=reference, env=env)
                raw = self.raw(cap)
                self.assertIn(b"PI_DAVINCI_PANIC_FIXTURE", raw, "the shell did not panic")
                self.assertEqual(cap.exit, "exited")
                self.assertIn(1049, ever_on(raw))
                self.assert_modes_off(raw, ever_on(raw))
                if reference:
                    self.assertLessEqual({1000, 1002, 1003, 1006}, ever_on(raw))

    def test_the_environment_alone_turns_the_reference_on_and_restore_follows_it(self):
        # The stored setting says off; PI_DAVINCI_REFERENCE=1 wins, and restore does not
        # look at the setting either way.
        cap = self.run_scenario(dict(SETTLED, actions=QUIT), reference=False,
                                env={"PI_DAVINCI_REFERENCE": "1"})
        raw = self.raw(cap)
        self.assertLessEqual({1000, 1002, 1003, 1006}, ever_on(raw))
        self.assert_modes_off(raw, ever_on(raw))

    # --------------------------------------------------------------- adverse
    def test_exit_while_a_turn_is_queued_sends_no_duplicate_request_and_restores(self):
        scenario = {
            "expect_requests": 1, "wait_ms": 8000,
            "turns": [{"kind": "text", "text": "Synthetic held reply.", "hold": True}],
            "actions": [{"type": "key", "text": "first"}, {"type": "key", "text": "\r", "delay_ms": 400},
                        {"type": "wait", "ms": 1200},
                        {"type": "key", "text": "second"}, {"type": "key", "text": "\r", "delay_ms": 400},
                        {"type": "wait", "ms": 800},
                        {"type": "snapshot", "name": "queued"},
                        # A second interrupt while the first is still unanswered gives the
                        # terminal back and leaves (ctrl+c and ctrl+d do nothing mid-turn).
                        {"type": "key", "text": "\x1b"}, {"type": "key", "text": "\x1b"},
                        {"type": "wait", "ms": 2000}]}
        for reference in (True, False):
            with self.subTest(reference=reference):
                cap = self.run_scenario(scenario, reference=reference)
                self.assertEqual(cap.request_count, 1, "the queued line must not be sent on the way out")
                raw = self.raw(cap)
                self.assertEqual(cap.exit, "exited")
                self.assertIn(b"the turn would not stop", raw, "the interrupt exit path did not run")
                self.assertIn(1049, ever_on(raw))
                self.assert_modes_off(raw, ever_on(raw))
                if reference:
                    self.assertLessEqual({1000, 1002, 1003, 1006}, ever_on(raw))


if __name__ == "__main__":
    unittest.main()
