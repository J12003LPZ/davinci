"""P12: the permission dialog and its decisions, measured on a live DaVinci.

The reference's write dialog offers a session grant for a path outside the project.
DaVinci's policy deliberately does not: a session or project rule for an outside
path is never offered, so the rows that carry those choices are recorded, not
compared. Everything else in the dialog block is compared row for row against
`mock_permission_write__01_perm_dialog`, and each decision is exercised live:
allow once writes, deny and Escape leave the call denied, and "always" reports
the persisted project rule.
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
# The automatic verification turn would add provider requests after a successful
# write; P12 is about the dialog, so it is off and the counts stay about the call.
SETTINGS = {"defaultThinkingLevel": "medium", "autoVerify": False}
ENTER, ESC = "\r", "\x1b"
PROBE = "permission probe"
OUTSIDE = "..\\outside.txt"


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


def wait_for(text, timeout_ms=30000):
    return {"type": "wait_for", "text": text, "timeout_ms": timeout_ms}


def shot(name):
    return {"type": "snapshot", "name": name}


def write_turn(path):
    return {"kind": "tool", "tool": "write",
            "arguments": {"path": path, "content": PROBE}}


TURNS = [{"kind": "text", "text": "First done."},
         {"kind": "text", "text": "Acknowledged."}]


@unittest.skipIf(live_skip_reason(), live_skip_reason() or "")
class LivePermissionTest(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp(prefix="cc2316_p12_"))
        self.addCleanup(shutil.rmtree, self.root, True)
        self.count = 0

    def run_actions(self, actions, path="note.txt", expect=2):
        self.count += 1
        scenario = {"expect_requests": expect, "wait_ms": 45000, "actions": actions,
                    "turns": [write_turn(path), *TURNS],
                    "davinci_settings": SETTINGS}
        return replay.replay(BINARY, scenario, replay.Profile(120, 44, env=ENV),
                             out_dir=self.root / f"run{self.count}")

    def test_the_create_dialog_matches_the_reference_block(self):
        # The reference's own frame is an outside-project write; Esc ends this run.
        actions = [key("write outside"), wait(900), key(ENTER, 300),
                   wait_for("Do you want to create outside.txt?"), shot("dialog"),
                   key(ESC, 100), wait(2500)]
        cap = self.run_actions(actions, path=OUTSIDE, expect=2)
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        actual = load(cap.snapshots["dialog"])
        expected = load(replay.ref_root()
                        / "artifacts/frames/reference/mock_permission_write__01_perm_dialog.json")
        # Title, both dashed rules, the preview and the question.
        for ours_at, theirs_at, field in [
            (29, 21, "title"),
            (31, 23, "rule under the subject"),
            (32, 24, "preview"),
            (33, 25, "rule under the preview"),
            (34, 26, "question"),
        ]:
            self.assertEqual(row_text(actual, ours_at).rstrip(),
                             row_text(expected, theirs_at).rstrip(), field)
        # The subject is the reference's path plus DaVinci's outside-project note.
        self.assertEqual(row_text(actual, 30).rstrip(), f" {OUTSIDE} · outside the project")
        self.assertTrue(row_text(expected, 22).rstrip() == f" {OUTSIDE}")
        # The outside-project write offers only the choices the policy issues.
        self.assertEqual([row_text(actual, y).rstrip() for y in range(35, 38)],
                         [" ❯ 1. Yes", "   2. No",
                          "   3. No, and tell the model what to do differently"])
        # The reference has a session option here (row 28-30) and "Tab to amend";
        # both are recorded in results.json, one as a safety difference and one as
        # DaVinci's own footer copy.
        self.assertIn("accept edits", screen_text(expected))

    def test_allow_once_writes_and_closes_the_dialog(self):
        actions = [key("write the note"), wait(900), key(ENTER, 300),
                   wait_for("Do you want to create note.txt?"), shot("dialog"),
                   key(ENTER, 100), wait(3500), shot("after")]
        cap = self.run_actions(actions, expect=3)
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        after = screen_text(load(cap.snapshots["after"]))
        self.assertNotIn("Do you want to create", after, "the dialog stayed up")
        self.assertIn("Write(note.txt)", after)
        self.assertIn("Added 1 line", after, "the approved write did not run")
        self.assertNotIn("denied", after)
        self.assertEqual(len(cap.requests), 3, "tool call, answer, completion turn")

    def test_deny_leaves_the_call_denied(self):
        actions = [key("write the note"), wait(900), key(ENTER, 300),
                   wait_for("Do you want to create note.txt?"),
                   key("4", 100), key(ENTER, 200), wait(3500), shot("after")]
        cap = self.run_actions(actions, expect=2)
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        after = screen_text(load(cap.snapshots["after"]))
        self.assertIn("denied", after)
        self.assertNotIn("Added 1 line", after)

    def test_escape_leaves_the_call_denied(self):
        actions = [key("write the note"), wait(900), key(ENTER, 300),
                   wait_for("Do you want to create note.txt?"),
                   key(ESC, 200), wait(3500), shot("after")]
        cap = self.run_actions(actions, expect=2)
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        self.assertIn("denied", screen_text(load(cap.snapshots["after"])))

    def test_allow_always_reports_the_persisted_rule(self):
        actions = [key("write the note"), wait(900), key(ENTER, 300),
                   wait_for("Do you want to create note.txt?"),
                   key("3", 100), key(ENTER, 200), wait(3500), shot("after")]
        cap = self.run_actions(actions, expect=3)
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        after = screen_text(load(cap.snapshots["after"]))
        self.assertIn("remembered write(note.txt)", after)
        self.assertIn(".pi/settings.json", after)
        self.assertIn("Added 1 line", after)

    def test_the_dialog_does_not_change_the_permission_mode(self):
        actions = [key("write the note"), wait(1000), shot("before"),
                   key(ENTER, 300), wait_for("Do you want to create note.txt?"),
                   key(ENTER, 100), wait(3500), shot("after")]
        cap = self.run_actions(actions, expect=3)
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        before = screen_text(load(cap.snapshots["before"]))
        after = screen_text(load(cap.snapshots["after"]))
        self.assertIn("manual mode on", before)
        self.assertIn("manual mode on", after, "an approval changed the mode")


if __name__ == "__main__":
    unittest.main()
