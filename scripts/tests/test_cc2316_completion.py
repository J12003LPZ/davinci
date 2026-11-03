"""P09: completion and shell mode, measured on a live DaVinci through the replay.

The slash list names DaVinci's own commands (D2: longer copy is DaVinci's own), so the
reference's list rows cannot be compared cell for cell; the rows are recorded as a named
finding and the geometry they must share with the reference is asserted directly below.
The composer, header and rules outside that band are compared.

Live tests skip, with a reason, when the binary, pywinpty or the reference package is
missing. They run with ``PI_DAVINCI_REFERENCE=1`` against the loopback fixture, never read
the machine's clipboard (the replay sets ``PI_CLIPBOARD_DRY_RUN``) and use synthetic files.
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
TAB, ESC, ENTER = "\t", "\x1b", "\r"
FOOTER = 43
LIST_ROWS = range(35, 40)


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


def wait_for(text, timeout_ms=30000):
    return {"type": "wait_for", "text": text, "timeout_ms": timeout_ms}


def shot(name):
    return {"type": "snapshot", "name": name}


# (actions, extra settings, extra env, files)
GOLDENS = {
    "slash": ([key("/th"), wait_for("❯ /theme"), shot("slash_only__01_slash")], {}, {}, {}),
    "tab": ([key("/th"), wait_for("❯ /theme"), key(TAB), wait(300),
             shot("slash_only__02_tab")], {}, {}, {}),
}
# The list rows carry DaVinci's own command set and its own match highlight (D2); the
# geometry they must share with the reference is asserted in test_the_list_sits_where_the
# _reference_puts_it below. Anything outside these rows must match.
OPEN = {
    "slash_only__01_slash": {
        row: "the list names DaVinci's own commands (D2), and its highlight follows "
             "DaVinci's own matches; position, indent, description column and the hidden "
             "effort row are asserted by the live geometry test"
        for row in LIST_ROWS
    },
}


@unittest.skipIf(live_skip_reason(), live_skip_reason() or "")
class LiveCompletionTest(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp(prefix="cc2316_p09_"))
        self.addCleanup(shutil.rmtree, self.root, True)
        self.count = 0

    def run_actions(self, actions, settings=None, env=None, files=None, cols=120, rows=44):
        self.count += 1
        scenario = {"expect_requests": 0, "wait_ms": 45000, "actions": actions,
                    "files": files or {},
                    "davinci_settings": {**SETTINGS, **(settings or {})}}
        return replay.replay(BINARY, scenario,
                             replay.Profile(cols, rows, env={**ENV, **(env or {})}),
                             out_dir=self.root / f"run{self.count}")

    def snapshot(self, actions, **kw):
        cap = self.run_actions([*actions, wait(400), shot("s")], **kw)
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        return load(cap.snapshots["s"])

    # ------------------------------------------------------------- goldens
    def test_every_golden_matches_outside_the_list_rows_left_to_davinci(self):
        for name_, (actions, settings, env, files) in GOLDENS.items():
            cap = self.run_actions(actions, settings=settings, env=env, files=files)
            self.assertEqual(cap.status, replay.PASS, cap.reason)
            for snapshot, path in cap.snapshots.items():
                with self.subTest(golden=snapshot):
                    expected = golden(snapshot)
                    if expected is None:
                        self.skipTest(f"no golden {snapshot}")
                    report = compare.compare_golden(expected, load(path))
                    open_rows = OPEN.get(snapshot, {})
                    by = {"footer (P12b)": [], "list rows (D2)": [], "unowned": []}
                    for d in report.differences:
                        if d.row == FOOTER:
                            by["footer (P12b)"].append(d)
                        elif d.row in open_rows:
                            by["list rows (D2)"].append(d)
                        else:
                            by["unowned"].append(d)
                    print(f"\n{snapshot}: {len(report.differences)} differences; "
                          + ", ".join(f"{k} {len(v)}" for k, v in by.items()))
                    self.assertEqual(by["unowned"], [], "\n" + "\n".join(
                        f"row {d.row} col {d.column} {d.field}: {d.expected!r} != {d.actual!r}"
                        for d in by["unowned"][:20]))

    # ------------------------------------------------------ list geometry
    def test_the_list_sits_where_the_reference_puts_it(self):
        shown = self.snapshot([key("/th"), wait_for("❯ /theme")])
        rows = [row_text(shown, y) for y in LIST_ROWS]
        # Five rows, the last one where the effort row sits; the effort row is not drawn.
        self.assertTrue(rows[0].startswith("  ❯ /theme"), repr(rows[0]))
        self.assertEqual(rows[1][:4], "    ", repr(rows[1]))
        self.assertTrue(all(row.strip() for row in rows), "a list row is blank")
        self.assertEqual(rows[0][:32], "  ❯ /theme".ljust(32), "the name ends at column 32")
        for row in rows[1:]:
            self.assertEqual(row[:4], "    ", "an unselected row is indented four cells")
            name = row[4:].split("  ", 1)[0]
            self.assertGreaterEqual(len(name), 2, repr(row))
            self.assertTrue(row[32:].strip(), "a list row has a description at column 32")
        self.assertNotIn("/effort", screen_text(shown).split("\n")[39])
        # The description column is the same on every row.
        for row in rows:
            self.assertEqual(len(row), 120)
        # The effort row comes back once the list closes.
        shown = self.snapshot([key("/th"), wait_for("❯ /theme"), key(ESC), wait(300)])
        self.assertIn("/effort", row_text(shown, 39))
        self.assertNotIn("❯ /theme", screen_text(shown))

    def test_tab_fills_the_marked_command_and_closes_the_list(self):
        shown = self.snapshot([key("/th"), wait_for("❯ /theme"), key(TAB), wait(300)])
        self.assertIn("/theme", row_text(shown, 41))
        self.assertNotIn("❯ /theme", screen_text(shown))
        self.assertIn("/effort", row_text(shown, 39), "the effort row is back")

    def test_enter_takes_the_marked_command_and_runs_it(self):
        shown = self.snapshot([key("/th"), wait_for("❯ /theme"), key(ENTER), wait(700)])
        # `/theme` opens the reference picker: its theme names are on screen.
        self.assertIn("Dark mode", screen_text(shown))

    def test_escape_keeps_the_typed_text_and_closes_the_list(self):
        shown = self.snapshot([key("/th"), wait_for("❯ /theme"), key(ESC), wait(300)])
        self.assertIn("/th", row_text(shown, 41))
        self.assertNotIn("❯ /theme", screen_text(shown))

    def test_a_click_on_a_row_fills_that_command(self):
        shown = self.snapshot([key("/th"), wait_for("❯ /theme"),
                               {"type": "click_text", "text": "/init"}, wait(500)])
        self.assertIn("/init", row_text(shown, 41))

    # --------------------------------------------------------- @ mentions
    def test_at_mentions_sit_in_the_same_band_and_tab_fills_one(self):
        files = {"fixture.txt": "hello\n"}
        shown = self.snapshot([key("@fixture"), wait_for("+ fixture.txt")], files=files)
        self.assertEqual(row_text(shown, 39).rstrip(), "    + fixture.txt")
        self.assertNotIn("/effort", row_text(shown, 39))
        shown = self.snapshot([key("@fixture"), wait_for("+ fixture.txt"), key(TAB), wait(300)],
                              files=files)
        self.assertIn("@fixture.txt", row_text(shown, 41))
        self.assertIn("/effort", row_text(shown, 39))

    # ---------------------------------------------------------- off mode
    def test_off_mode_keeps_its_own_list_and_offers_no_theme_command(self):
        shown = self.snapshot([key("/th"), wait(600)], env={"PI_DAVINCI_REFERENCE": "0"})
        self.assertNotIn("❯ /theme", screen_text(shown))
        # The shell's own list still draws where it did: `/` still lists its commands.
        shown = self.snapshot([key("/"), wait(600)], env={"PI_DAVINCI_REFERENCE": "0"})
        self.assertIn("/config", screen_text(shown))

    def test_shell_mode_still_runs_a_local_bang_command(self):
        # NO_REFERENCE: the reference has no shell-mode test id. This only checks the
        # shell mode still works; it claims no parity.
        scenario = {"expect_requests": 0, "wait_ms": 45000,
                    "davinci_settings": SETTINGS,
                    "actions": [key("!echo shell-mode-ok"), wait_for("shell-mode-ok"), shot("s"),
                                key(ESC), wait(300)]}
        self.count += 1
        cap = replay.replay(BINARY, scenario, replay.Profile(120, 44, env=ENV),
                            out_dir=self.root / f"run{self.count}")
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        self.assertIn("shell-mode-ok", screen_text(load(cap.snapshots["s"])))


if __name__ == "__main__":
    unittest.main()
