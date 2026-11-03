"""P08: the composer, measured on a live DaVinci through the replay.

Live tests skip, with a reason, when the binary, pywinpty or the reference package is
missing. They run with ``PI_DAVINCI_REFERENCE=1`` against the loopback fixture, never read
the machine's clipboard (the replay sets ``PI_CLIPBOARD_DRY_RUN``) and use synthetic text,
a generated PNG and local editor scripts only.

Each golden is compared in full. What is left open is named with its owner: the footer row
(the mode label and hints) is P12b's, the rest are findings reported to the reviewer.
"""
import json
import os
import shutil
import struct
import sys
import tempfile
import time
import unittest
import zlib
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "ui"))
import cc2316_compare as compare  # noqa: E402
import cc2316_replay as replay  # noqa: E402

BINARY = Path(os.environ.get("CC2316_BINARY") or HERE.parents[1] / "target" / "debug" / "davinci.exe")
ENV = {"PI_DAVINCI_REFERENCE": "1"}
SETTINGS = {"defaultThinkingLevel": "medium"}
PASTE = "\x1b[200~{}\x1b[201~"
ESC, ENTER, CTRL_G, CTRL_R = "\x1b", "\r", "\x07", "\x12"
CTRL_S = "\x13"
ALT_P = ESC + "p"
FOOTER = 43   # the 120x44 footer row


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


def make_png(path):
    raw = b"".join(b"\x00" + b"\x20\x80\xc0" * 4 for _ in range(4))

    def chunk(kind, data):
        body = struct.pack(">I", len(data)) + kind + data
        return body + struct.pack(">I", zlib.crc32(kind + data) & 0xFFFFFFFF)

    path.write_bytes(b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", 4, 4, 8, 2, 0, 0, 0))
                     + chunk(b"IDAT", zlib.compress(raw)) + chunk(b"IEND", b""))


def shot(name):
    return {"type": "snapshot", "name": name}


def key(text, delay=0):
    return {"type": "key", "text": text, "delay_ms": delay}


def wait(ms):
    return {"type": "wait", "ms": ms}


def wait_for(text, timeout_ms=10000):
    return {"type": "wait_for", "text": text, "timeout_ms": timeout_ms}


# (scenario run, golden, owner of the differences that stay open: {row: reason})
LONG = PASTE.format("A" * 12000)
GOLDENS = {
    "paste_only": ([key(PASTE.format("pasted text")), wait_for("pasted text"),
                    shot("paste_only__01_paste")], {}),
    # A 12,000-character paste reaches the shell as individual key records and the paste
    # filter turns them into one paste after a 500 ms quiet window; the debug build needs
    # 2-3 s to take them in. Waiting for the chip is the real condition, not a fixed sleep.
    "paste_large": ([key(LONG), wait_for("[Pasted text #1]"),
                     shot("paste_large__01_paste_12k"),
                     key(LONG), wait_for("[...Truncated text #2"),
                     key(PASTE.format("你好🙂é")), wait_for("é"),
                     shot("paste_large__03_unicode")], {"vim": True}),
    "esc1": ([key("hello"), wait(300), key(ESC), wait(300), shot("esc_clear__01_esc1")], {}),
    "esc2": ([key("hello"), wait(300), key(ESC), wait(200), key(ESC), wait(500),
              shot("esc_clear__02_esc2")], {}),
    "history": ([key(CTRL_R), wait(700), key("help"), wait(600), shot("keys_panels__06_history")], {}),
    "image": ([key("look at this "), key(ESC + "v"), wait(1200), key("please describe"), wait(600),
               shot("image_paste__02_after_typing")], {"image": True}),
    "vim": ([key("i"), wait(300), key("hello vim"), wait(500), key(ESC), wait(500), key("0"), key("d"),
             key("w"), wait(500), shot("vim_mode__04_dw")], {"vim": True}),
    "editor": ([key("draft text"), wait(400), key(CTRL_G), wait(2500),
                shot("external_editor__01_after_editor")], {"editor": "exit 0"}),
}
# Differences that stay after P08, by golden: (row, first column) -> why. Anything else fails.
OPEN = {
    "paste_large__03_unicode": {
        41: "the emoji of the paste never reaches the shell on this Windows console path "
            "(typed or pasted, with the setting off too): crossterm 0.29.0 drops a surrogate "
            "pair when ConPTY follows each half with a key-up record, so the row is one glyph "
            "short and the caret is two columns early"},
    "image_paste__02_after_typing": {
        41: "the reference's caret sits after the token while the text typed after Alt+V "
            "stands before it; the shell keeps the caret where the text is typed"},
}


class Editor:
    """A local editor script for ``externalEditor``."""

    def __init__(self, root: Path):
        self.root = root

    def command(self, body: str, name="editor") -> str:
        script = self.root / f"{name}.py"
        script.write_text(body, encoding="utf-8")
        return f'"{sys.executable}" "{script.as_posix()}"'


@unittest.skipIf(live_skip_reason(), live_skip_reason() or "")
class LiveEditorTest(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp(prefix="cc2316_p08_"))
        self.addCleanup(shutil.rmtree, self.root, True)
        self.count = 0
        self.png = self.root / "clip.png"
        make_png(self.png)
        self.editor = Editor(self.root)

    def run_actions(self, actions, settings=None, env=None, cols=120, rows=44, wait_ms=45000):
        self.count += 1
        scenario = {"expect_requests": 0, "wait_ms": wait_ms, "actions": actions,
                    "davinci_settings": {**SETTINGS, **(settings or {})}}
        cap = replay.replay(BINARY, scenario, replay.Profile(cols, rows, env={**ENV, **(env or {})}),
                            out_dir=self.root / f"run{self.count}")
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        return cap

    def snapshot(self, actions, **kw):
        cap = self.run_actions([*actions, wait(400), shot("s")], **kw)
        return load(cap.snapshots["s"])

    # ------------------------------------------------------------- goldens
    def test_every_golden_matches_outside_the_footer_and_the_findings_named_above(self):
        for key_, (actions, how) in GOLDENS.items():
            settings, env = dict(), dict()
            if how.get("vim"):
                settings["editorMode"] = "vim"
            if how.get("image"):
                env["PI_CLIPBOARD_IMAGE"] = str(self.png)
            if "editor" in how:
                settings["externalEditor"] = self.editor.command("import sys\nsys.exit(0)\n")
            cap = self.run_actions(actions, settings=settings, env=env, wait_ms=6000)
            for name, path in cap.snapshots.items():
                with self.subTest(golden=name):
                    expected = golden(name)
                    if expected is None:
                        self.skipTest(f"no golden {name}")
                    report = compare.compare_golden(expected, load(path))
                    open_rows = OPEN.get(name, {})
                    by = {"footer (P12b)": [], "named finding": [], "unowned": []}
                    for d in report.differences:
                        if d.row == FOOTER:
                            by["footer (P12b)"].append(d)
                        elif d.row in open_rows:
                            by["named finding"].append(d)
                        else:
                            by["unowned"].append(d)
                    print(f"\n{name}: {len(report.differences)} differences; "
                          + ", ".join(f"{k} {len(v)}" for k, v in by.items()))
                    self.assertEqual(by["unowned"], [], "\n" + "\n".join(
                        f"row {d.row} col {d.column} {d.field}: {d.expected!r} != {d.actual!r}"
                        for d in by["unowned"][:20]))

    # ------------------------------------------------------- external editor
    def draft_survives(self, command, draft_steps, expect, toast=None):
        shown = self.snapshot([*draft_steps, wait(300), key(CTRL_G), wait(2500)],
                              settings={"externalEditor": command})
        self.assertIn(expect, screen_text(shown))
        if toast:
            self.assertIn(toast, screen_text(shown))
        return shown

    def test_an_editor_that_changes_nothing_leaves_the_draft_and_what_follows_works(self):
        draft = "naïve 你好 draft"
        command = self.editor.command("import sys\nsys.exit(0)\n")
        steps = [key(draft)]
        shown = self.snapshot([*steps, wait(300), key(CTRL_G), wait(2500), key(" more"), wait(400)],
                              settings={"externalEditor": command})
        self.assertIn(draft + " more", screen_text(shown))

    def test_an_editor_that_edits_the_file_replaces_the_draft(self):
        command = self.editor.command(
            "import sys\np = sys.argv[1]\nt = open(p, encoding='utf-8').read()\n"
            "open(p, 'w', encoding='utf-8').write(t.rstrip() + ' EDITED\\n')\n")
        self.draft_survives(command, [key("before")], "before EDITED")

    def test_an_editor_that_fails_keeps_the_draft(self):
        command = self.editor.command("import sys\nsys.exit(3)\n")
        self.draft_survives(command, [key("keep this")], "keep this", toast="editor:")

    def test_an_editor_that_is_missing_keeps_the_draft(self):
        command = f'"{(self.root / "no-such-editor.exe").as_posix()}"'
        self.draft_survives(command, [key("keep this too")], "keep this too", toast="editor:")

    def test_a_multiline_draft_with_wide_text_comes_back_byte_for_byte(self):
        # Lines made by a bracketed paste (CRLF becomes LF in the draft), CJK and a mark.
        draft_steps = [key(PASTE.format("first 你好\r\nsecond é")), wait(300)]
        command = self.editor.command("import sys\nsys.exit(0)\n")
        shown = self.snapshot([*draft_steps, key(CTRL_G), wait(2500)], settings={"externalEditor": command})
        text = screen_text(shown)
        self.assertIn("first 你好", text)
        self.assertIn("second é", text.replace("é", "é"))

    # ---------------------------------------------------------------- stash
    def test_ctrl_s_stashes_the_draft_and_a_second_ctrl_s_brings_it_back(self):
        stashed = self.snapshot([key("draft text"), wait(300), key(CTRL_S), wait(300)])
        screen = screen_text(stashed)
        self.assertIn("› stashed", screen)
        self.assertNotIn("draft text", screen)
        restored = self.snapshot([key("draft text"), wait(300), key(CTRL_S), wait(300),
                                  key(CTRL_S), wait(300)])
        screen = screen_text(restored)
        self.assertIn("draft text", screen)
        self.assertNotIn("› stashed", screen)

    def test_a_wide_paste_and_an_image_survive_a_modal_and_a_resize(self):
        # The plan's P08 adverse case: a pasted draft with wide text, an image attachment, a
        # modal over it and a resize. Enter inside the modal must not send; the token, the
        # text and the attachment are still there and send once when the modal is closed.
        self.count += 1
        scenario = {"expect_requests": 1, "wait_ms": 45000,
                    "turns": [{"kind": "text", "text": "Synthetic reply."}],
                    "davinci_settings": SETTINGS,
                    "actions": [key("look "), key(ESC + "v"), wait_for("[Image #1]"),
                                key(PASTE.format("你好🙂é")), wait_for("é"),
                                key(ALT_P), wait(700), key(ENTER), wait(400),
                                {"type": "resize", "cols": 100, "rows": 30}, wait(900),
                                {"type": "resize", "cols": 120, "rows": 44}, wait(900),
                                key(ESC), wait(500), shot("draft"),
                                key(ENTER), wait(6000), shot("sent")]}
        cap = replay.replay(BINARY, scenario,
                            replay.Profile(120, 44, env={**ENV, "PI_CLIPBOARD_IMAGE": str(self.png)}),
                            out_dir=self.root / f"run{self.count}")
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        draft = screen_text(load(cap.snapshots["draft"]))
        self.assertIn("[Image #1]", draft)
        self.assertIn("你好", draft)
        self.assertEqual(len(cap.requests), 1, "the modal's Enter sent a turn")
        strings, images = [], 0

        def walk(node):
            nonlocal images
            if isinstance(node, str):
                strings.append(node)
            elif isinstance(node, dict):
                if node.get("type") == "input_image":
                    images += 1
                for value in node.values():
                    walk(value)
            elif isinstance(node, list):
                for value in node:
                    walk(value)

        walk(cap.requests[0])
        self.assertEqual(images, 1, "the image attachment was lost")
        self.assertTrue(any("look" in text and "你好" in text for text in strings),
                        "the draft text did not reach the request intact")

    # ------------------------------------------------------------- paste
    def test_a_hundred_thousand_character_paste_collapses_and_the_shell_answers(self):
        big = PASTE.format(("word " * 20000))
        started = time.time()
        # The console hands a paste this big over in pieces: the first characters arrive as
        # keys and the bulk as two pastes, each its own chip, because the burst filter holds
        # 65,536 keys. Wait for the *second* chip before typing: a key written while the
        # burst is still open joins the paste instead of landing after it.
        shown = self.snapshot([key(big), wait_for("[Pasted text #2]", timeout_ms=300000),
                               key("after"), wait_for("after", timeout_ms=120000)])
        text = screen_text(shown)
        self.assertIn("[Pasted text #1]", text)
        self.assertIn("after", text)
        print(f"\n100,000-character paste then a key, both on screen after "
              f"{time.time() - started:.1f} s including startup and the waits")

    def test_escape_sequences_in_a_paste_do_not_reach_the_terminal_or_the_draft(self):
        # The console strips the ESC and BEL bytes and hands the rest over as keys; the paste
        # filter reassembles them after its 500 ms window, so the screen is the condition.
        cap = self.run_actions([key(PASTE.format("a\x1b]0;owned\x07b\x1b[2Jc")),
                                wait_for("a]0;ownedb"), shot("s")])
        raw = cap.raw_path.read_bytes()
        # The escape bytes are dropped before the draft sees the paste (the unit tests cover
        # the sequences in full); what is left is plain text, and no title is ever written.
        self.assertNotIn(b"\x1b]0;owned", raw)
        self.assertNotIn(b"owned\x07", raw)
        self.assertIn("a]0;ownedb", screen_text(load(cap.snapshots["s"])))

    def test_two_escapes_clear_the_draft_and_one_only_arms(self):
        armed = self.snapshot([key("hello"), wait(300), key(ESC), wait(300)])
        self.assertIn("hello", screen_text(armed))
        cleared = self.snapshot([key("hello"), wait(300), key(ESC), wait(200), key(ESC), wait(400)])
        self.assertNotIn("hello", screen_text(cleared))

    def test_the_second_escape_clears_the_draft_inside_the_window_in_both_modes(self):
        # The window is 800 ms with the setting on and 500 ms off, so 700 ms clears only on.
        for mode, env, gaps in (("on", {}, (400, 700)),
                                ("off", {"PI_DAVINCI_REFERENCE": "0"}, (400,))):
            for gap in gaps:
                with self.subTest(setting=mode, gap_ms=gap):
                    shown = self.snapshot([key("hello"), wait(300), key(ESC), wait(gap), key(ESC), wait(400)],
                                          env=env)
                    self.assertNotIn("hello", screen_text(shown))
        shown = self.snapshot([key("hello"), wait(300), key(ESC), wait(700), key(ESC), wait(400)],
                              env={"PI_DAVINCI_REFERENCE": "0"})
        self.assertIn("hello", screen_text(shown), "off: 700 ms is past its 500 ms window")

    @unittest.expectedFailure
    def test_a_typed_or_pasted_emoji_reaches_the_draft_in_both_modes(self):
        # KNOWN FAULT, not in this repository: ConPTY hands the console a key-down and a key-up
        # record for each UTF-16 unit of an emoji, and crossterm 0.29.0 (event/sys/windows/parse.rs)
        # does not look at key_down for a surrogate, so the high half is paired with its own
        # release, fails to decode and is dropped, and so is the low half. No event reaches the
        # shell at all. Proven with a bare console reader (U+D83D down/up, U+DE42 down/up) and a
        # trace of the events the shell is fed; this test is the same path a real terminal uses
        # (pywinpty -> ConPTY -> console input -> crossterm). It does not prove Windows Terminal,
        # which was not run. When crossterm is fixed this stops failing: drop the decorator.
        for mode, env in (("on", {}), ("off", {"PI_DAVINCI_REFERENCE": "0"})):
            for how, text in (("typed", "a\U0001F642b"), ("pasted", PASTE.format("a\U0001F642b"))):
                with self.subTest(setting=mode, how=how):
                    shown = self.snapshot([key(text), wait(600)], env=env)
                    self.assertIn("a\U0001F642b", screen_text(shown))

    def test_a_large_multiline_paste_is_sent_whole_once(self):
        # 100,000 characters over 3,700 lines, newlines as a terminal pastes them (a carriage
        # return, so each is an Enter key to the console) and a first line of two characters:
        # its Enter is the early-submit case. The console hands the shell one key record per
        # key (two chips come of the shell's 65,536-key burst limit), and a debug build needs
        # more than a minute to take them all; an Enter sent before it has would join the
        # paste, as it would from a real terminal with the keys still queued. The text ends on
        # a letter because a send drops trailing whitespace, as it always has.
        lines = ["hi"] + [f"line {i:05d} " + "word " * 3 for i in range(1, 5000)]
        typed = "\r".join(lines)[:99_995] + "!end!"
        self.assertEqual(len(typed), 100_000)
        cleaned = typed.replace("\r", "\n")
        self.count += 1
        scenario = {"expect_requests": 1, "wait_ms": 6000, "timeout": 400,
                    "turns": [{"kind": "text", "text": "Synthetic reply."}],
                    "davinci_settings": SETTINGS,
                    "actions": [key(PASTE.format(typed)), wait(150000), shot("draft"), key(ENTER),
                                wait(8000)]}
        started = time.time()
        cap = replay.replay(BINARY, scenario, replay.Profile(120, 44, env=ENV),
                            out_dir=self.root / f"run{self.count}")
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        self.assertEqual(len(cap.requests), 1)
        strings = []

        def walk(node):
            if isinstance(node, str):
                strings.append(node)
            elif isinstance(node, dict):
                for value in node.values():
                    walk(value)
            elif isinstance(node, list):
                for value in node:
                    walk(value)
        walk(cap.requests[0])
        self.assertIn(cleaned, strings, "the text sent is not the cleaned paste, byte for byte")
        draft = screen_text(load(cap.snapshots["draft"]))
        self.assertIn("[Pasted text #1", draft, "the paste was held as chips, not sent early")
        print(f"\n{len(cleaned)} characters, {cleaned.count(chr(10))} newlines: one request, text "
              f"identical ({time.time() - started:.0f} s including the waits, a debug build)")

    def test_keys_during_a_running_turn_edit_the_draft(self):
        # A human types; one write of 18 keys is a paste to the burst filter and is held for
        # its 500 ms window, which is not what this case is about. Each word is under the
        # eight-key threshold, so it lands as typing, then the screen is awaited.
        scenario = {"expect_requests": 1, "wait_ms": 45000,
                    "turns": [{"kind": "text", "text": "Synthetic held reply.", "hold": True}],
                    "davinci_settings": SETTINGS,
                    "actions": [key("first"), key(ENTER, 400), wait(1200),
                                key("typed ", 120), key("while ", 120), key("running", 120),
                                wait_for("typed while running"), shot("s"),
                                {"type": "release"}, wait(500)]}
        self.count += 1
        cap = replay.replay(BINARY, scenario, replay.Profile(120, 44, env=ENV),
                            out_dir=self.root / f"run{self.count}")
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        self.assertIn("typed while running", screen_text(load(cap.snapshots["s"])))


class OpenFindingsTest(unittest.TestCase):
    def test_every_named_finding_belongs_to_a_golden_in_this_file(self):
        names = {"paste_large__03_unicode", "image_paste__02_after_typing"}
        self.assertEqual(set(OPEN), names)
        for rows in OPEN.values():
            self.assertTrue(all(isinstance(why, str) and why for why in rows.values()))


if __name__ == "__main__":
    unittest.main()
