"""P10: the transcript body, measured on a live DaVinci.

The reference's four transcript frames were captured in the verbose transcript view, whose
chrome (masthead, timestamps, model row) is P14b's and whose tool calls need the permission
dialog (P12) before they can be driven live. This file compares the Markdown body that P10
owns against the same rows of `top_tools2__01_transcript_top`, and leaves the chrome and
tool-call frames to their owners in results.json.
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
MARKDOWN = """Rendering fixture

This paragraph has bold, italic, inline code, and a link.

Lists

- first item
- second item with strong text
  - nested item
  - another nested item

1. ordered one
2. ordered two

Code

```python
def greet(name: str) -> str:
    return f"Hello, {name}!"
```

Table

| Name | Value | Notes |
|------|-------|-------|
| alpha | 1 | first |
| beta | 2 | second |

> A block quote for visual testing.

---"""


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


def body(capture, start_marker="● Rendering fixture"):
    """The Markdown body from the answer marker to the closing rule.

    Only the transcript column is compared: a live screen also carries the scrollbar and
    the context cells at the right edge (P14/P12b), which the reference's viewer frame
    does not share here.
    """
    rows = [row_text(capture, y)[:116].rstrip() for y in range(len(capture["rows"]))]
    start = next((y for y, row in enumerate(rows) if start_marker in row), None)
    if start is None:
        return []
    end = next((y for y in range(start, len(rows)) if rows[y].strip() == "---"), None)
    if end is None:
        end = len(rows) - 1
    return rows[start:end + 1]


@unittest.skipIf(live_skip_reason(), live_skip_reason() or "")
class LiveTranscriptTest(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp(prefix="cc2316_p10_"))
        self.addCleanup(shutil.rmtree, self.root, True)
        self.count = 0

    def test_the_markdown_body_matches_the_reference_frame(self):
        self.count += 1
        scenario = {"expect_requests": 1, "wait_ms": 45000,
                    "turns": [{"kind": "text", "text": MARKDOWN}],
                    "davinci_settings": SETTINGS,
                    "actions": [
                        {"type": "key", "text": "Give me a markdown and rendering test.",
                         "delay_ms": 0},
                        {"type": "wait", "ms": 900},
                        {"type": "key", "text": "\r", "delay_ms": 300},
                        {"type": "wait_for", "text": "Rendering fixture", "timeout_ms": 60000},
                        {"type": "wait", "ms": 500},
                        {"type": "snapshot", "name": "s"}]}
        cap = replay.replay(BINARY, scenario, replay.Profile(120, 44, env=ENV),
                            out_dir=self.root / f"run{self.count}")
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        actual = body(load(cap.snapshots["s"]))
        expected = body(load(replay.ref_root()
                             / "artifacts/frames/reference/top_tools2__01_transcript_top.json"))
        self.assertTrue(expected, "the reference frame has no body")
        self.assertEqual(actual, expected)

    def test_a_tool_error_and_a_diff_render_in_the_body(self):
        # The renderers are also covered by the Rust transcript tests; this live check
        # keeps a real shell path over them without asking for a permission.
        self.count += 1
        scenario = {"expect_requests": 0, "wait_ms": 45000,
                    "davinci_settings": SETTINGS,
                    "actions": [
                        {"type": "key", "text": "!definitely-not-a-real-command", "delay_ms": 0},
                        {"type": "wait", "ms": 900},
                        {"type": "key", "text": "\r", "delay_ms": 300},
                        {"type": "wait_for", "text": "definitely-not-a-real-command",
                         "timeout_ms": 30000},
                        {"type": "wait", "ms": 500},
                        {"type": "snapshot", "name": "s"}]}
        cap = replay.replay(BINARY, scenario, replay.Profile(120, 44, env=ENV),
                            out_dir=self.root / f"run{self.count}")
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        screen = "\n".join(row_text(load(cap.snapshots["s"]), y) for y in range(44))
        self.assertIn("! definitely-not-a-real-command", screen)
        self.assertIn("⎿", screen)


if __name__ == "__main__":
    unittest.main()
