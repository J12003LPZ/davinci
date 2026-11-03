"""P13: pickers and panels, measured on a live DaVinci.

The reference fixes the frame; the content is DaVinci's own (model names,
settings, commands, context figures). Behaviour is checked live: tab order,
search, selection, effort, session-only vs persisted, real values, drafts and
unknown settings keys surviving a close.
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
ESC, ENTER, TAB, UP, DOWN, RIGHT = "\x1b", "\r", "\t", "\x1b[A", "\x1b[B", "\x1b[C"


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


@unittest.skipIf(live_skip_reason(), live_skip_reason() or "")
class LivePanelTest(unittest.TestCase):
    def setUp(self):
        self.root = Path(tempfile.mkdtemp(prefix="cc2316_p13p_"))
        self.addCleanup(shutil.rmtree, self.root, True)
        self.count = 0

    def run_actions(self, actions, env=None, turns=None, expect=0, settings=None):
        self.count += 1
        scenario = {"expect_requests": expect, "wait_ms": 45000, "actions": actions,
                    "turns": turns or [], "davinci_settings": {**SETTINGS, **(settings or {})}}
        return replay.replay(BINARY, scenario,
                             replay.Profile(120, 46, env={**ENV, **(env or {})}),
                             out_dir=self.root / f"run{self.count}")

    def open_panel(self, command, before=None):
        actions = [key(command), wait(800)]
        if before:
            actions += before
        actions += [key(ENTER, 200), wait(1400)]
        return actions

    def test_the_model_picker_matches_the_frame_and_changes_effort(self):
        actions = self.open_panel("/model") + [shot("open"),
                   key(RIGHT, 250), wait(500), shot("right"),
                   key("s", 250), wait(900), shot("session")]
        cap = self.run_actions(actions)
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        opened = load(cap.snapshots["open"])
        screen = screen_text(opened)
        self.assertIn("Select model", screen)
        row = next((row_text(opened, y) for y in range(46) if "1. demo" in row_text(opened, y)), "")
        self.assertTrue(row.startswith("   ❯ 1. demo"), row)
        at_31 = "".join(row)[31:35]
        self.assertEqual(at_31, "loca", f"description starts at 31: {row!r}")
        self.assertIn("◐ Medium effort (default) ←/→ to adjust", screen)
        self.assertIn("Enter to set as default · s to use this session only · Esc to cancel", screen)
        self.assertNotIn("/ to search", screen)
        right = screen_text(load(cap.snapshots["right"]))
        self.assertIn("● High effort ←/→ to adjust", right)
        session = screen_text(load(cap.snapshots["session"]))
        self.assertIn("Set model to Demo for this session only with high effort", session)
        self.assertIn("● high · /effort", session)

    def test_config_tabs_search_and_close(self):
        actions = self.open_panel("/config") + [shot("open"),
                   key("theme", 100), wait(600), shot("search"),
                   key("\x7f", 80), key("\x7f", 80), key("\x7f", 80), key("\x7f", 80),
                   key("\x7f", 80), wait(500), shot("cleared"),
                   key(UP, 250), wait(350), key(RIGHT, 250), wait(500), shot("status"),
                   key(RIGHT, 250), wait(500), shot("usage"),
                   key(RIGHT, 250), wait(500), shot("stats"),
                   key(ESC, 250), wait(500), shot("closed")]
        cap = self.run_actions(actions)
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        opened = screen_text(load(cap.snapshots["open"]))
        self.assertIn("Settings  Status   Config   Usage   Stats", opened)
        self.assertIn("Type to filter · Enter/↓ to select · ↑ to tabs · Esc to clear", opened)
        for name, marker in [("status", "Context used:"), ("usage", "User messages:"),
                             ("stats", "Permission mode:")]:
            self.assertIn(marker, screen_text(load(cap.snapshots[name])), name)
        status = screen_text(load(cap.snapshots["status"]))
        self.assertIn("account balances are not available", status)
        search = screen_text(load(cap.snapshots["search"]))
        self.assertIn("Theme", search)
        self.assertNotIn("Auto-compact", search, "the filter did not narrow the list")
        cleared = screen_text(load(cap.snapshots["cleared"]))
        self.assertIn("Auto-compact", cleared)
        closed = screen_text(load(cap.snapshots["closed"]))
        self.assertNotIn("Settings  Status", closed)
        self.assertIn("manual mode on", closed)

    def test_help_has_the_three_reference_tabs(self):
        actions = self.open_panel("/help") + [shot("general"),
                   key(TAB, 250), wait(500), shot("commands"),
                   key(TAB, 250), wait(500), shot("custom"),
                   key(ESC, 250), wait(500), shot("closed")]
        cap = self.run_actions(actions)
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        general = screen_text(load(cap.snapshots["general"]))
        self.assertIn("Help  General   Commands   Custom commands", general)
        self.assertIn("Esc to cancel", general)
        commands = screen_text(load(cap.snapshots["commands"]))
        self.assertIn("Browse default commands", commands)
        self.assertIn("❯ /", commands)
        custom = screen_text(load(cap.snapshots["custom"]))
        self.assertIn("Custom commands", custom)
        self.assertIn("No custom commands found", custom)
        closed = screen_text(load(cap.snapshots["closed"]))
        self.assertNotIn("Help  General", closed)

    def test_help_off_the_reference_is_still_the_command_list(self):
        actions = self.open_panel("/help") + [shot("help")]
        cap = self.run_actions(actions, env={"PI_DAVINCI_REFERENCE": "0"})
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        screen = screen_text(load(cap.snapshots["help"]))
        self.assertNotIn("Help  General", screen)
        self.assertIn("- /export", screen, "the off-mode command list is gone")

    def test_context_and_context_all_draw_the_reference_grid(self):
        actions = self.open_panel("/context") + [shot("context"),
                   key("/context all"), wait(800), key(ENTER, 200), wait(1600), shot("all")]
        cap = self.run_actions(actions)
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        screen = screen_text(load(cap.snapshots["context"]))
        self.assertIn("Context Usage", screen)
        self.assertIn("⛶", screen)
        self.assertNotIn("usage: /context", screen)
        row = next(row_text(load(cap.snapshots["context"]), y) for y in range(46)
                   if "⛶" in row_text(load(cap.snapshots["context"]), y))
        glyphs = sum(row.count(glyph) for glyph in "⛀⛁⛶⛃")
        self.assertEqual(glyphs, 20, row)
        all_rows = screen_text(load(cap.snapshots["all"]))
        self.assertIn("Context Usage", all_rows)
        self.assertNotIn("usage: /context", all_rows)

    def test_rewind_opens_from_double_escape_and_closes(self):
        actions = [key("first turn"), wait(800), key(ENTER, 200), wait(3000),
                   key(ESC, 150), wait(250), key(ESC, 150), wait(1400), shot("rewind"),
                   key(DOWN, 250), wait(400), shot("down"),
                   key(ESC, 250), wait(500), shot("closed")]
        cap = self.run_actions(actions, turns=[{"kind": "text", "text": "An answer."}], expect=1)
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        self.assertIn("Rewind", screen_text(load(cap.snapshots["rewind"])))
        self.assertIn("first turn", screen_text(load(cap.snapshots["rewind"])))
        self.assertNotIn("Rewind", screen_text(load(cap.snapshots["closed"])))

    def test_a_panel_close_keeps_the_draft_and_an_unknown_setting(self):
        # A keybinding-opened sheet can be entered with a draft already typed;
        # a slash command replaces the composer, so it cannot test this.
        actions = [key("keep me"), wait(400), key(ESC + "p", 200), wait(900), shot("open"),
                   key(ESC, 200), wait(500), shot("closed")]
        cap = self.run_actions(actions, settings={"keepMe": 123})
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        self.assertIn("Select model", screen_text(load(cap.snapshots["open"])))
        self.assertIn("keep me", screen_text(load(cap.snapshots["closed"])))
        self.assertEqual(cap.agent_settings.get("keepMe"), 123,
                         "an unknown settings key was lost on close")

        # The settings sheet can be opened and closed with the same key intact.
        actions = self.open_panel("/config") + [key(ESC, 200), wait(500), shot("closed")]
        cap = self.run_actions(actions, settings={"keepMe": 456})
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        self.assertEqual(cap.agent_settings.get("keepMe"), 456,
                         "closing the settings sheet rewrote unknown keys")

    def test_the_permissions_sheet_tabs_filter_and_are_read_only(self):
        actions = self.open_panel("/permissions") + [shot("open"),
                   key(RIGHT, 200), wait(400), shot("blocked"),
                   key(RIGHT, 200), wait(400), shot("allow"),
                   key("bash", 100), wait(500), shot("search"),
                   key(ENTER, 200), wait(400), shot("enter"),
                   key(ESC, 200), wait(300), shot("cleared"),
                   key(ESC, 200), wait(400), shot("closed")]
        cap = self.run_actions(actions)
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        opened = screen_text(load(cap.snapshots["open"]))
        self.assertIn("Permissions", opened)
        self.assertIn("Search…", opened)
        self.assertIn("←/→ to switch · ↓ to select · Esc to cancel", opened)
        self.assertIn("Recently blocked", opened)
        self.assertIn("No recently blocked calls.", screen_text(load(cap.snapshots["blocked"])))
        allow = screen_text(load(cap.snapshots["allow"]))
        self.assertIn("Tools that run without asking.", allow)
        search = screen_text(load(cap.snapshots["search"]))
        self.assertIn("bash", search)
        entered = screen_text(load(cap.snapshots["enter"]))
        self.assertIn("No rules match.", entered, "Enter applied a filtered tab")
        cleared = screen_text(load(cap.snapshots["cleared"]))
        self.assertIn("Tools that run without asking.", cleared, "Esc did not clear the search")
        closed = screen_text(load(cap.snapshots["closed"]))
        self.assertIn("manual mode on", closed, "a filtered tab changed the mode")
        self.assertNotIn("Search…", closed)

    def test_the_permissions_sheet_off_the_reference_is_the_old_list(self):
        actions = self.open_panel("/permissions") + [shot("open")]
        cap = self.run_actions(actions, env={"PI_DAVINCI_REFERENCE": "0"})
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        screen = screen_text(load(cap.snapshots["open"]))
        self.assertNotIn("Search…", screen)
        self.assertNotIn("Recently blocked   Allow", screen)
        self.assertIn("Modes", screen)


if __name__ == "__main__":
    unittest.main()
