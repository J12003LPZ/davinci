"""Tests for the loopback mock and the DaVinci replay (P03). Synthetic prompts only."""
import json
import os
import shutil
import sys
import tempfile
import unittest
import urllib.request
from pathlib import Path

HERE = Path(__file__).resolve().parent
sys.path.insert(0, str(HERE.parent / "ui"))
import cc2316_mock as mock  # noqa: E402
import cc2316_replay as replay  # noqa: E402

BINARY = Path(os.environ.get("CC2316_BINARY") or HERE.parents[1] / "target" / "debug" / "davinci.exe")


def post(server, body=b"{}"):
    req = urllib.request.Request(server.base_url + "/responses", data=body, method="POST")
    with urllib.request.urlopen(req, timeout=10) as resp:
        return resp.read().decode("utf-8")


def event_types(stream):
    return [json.loads(line[6:])["type"] for line in stream.splitlines() if line.startswith("data: ")]


class MockTest(unittest.TestCase):
    def test_text_event_order(self):
        with mock.MockServer([mock.text_turn("abcdefghij", chunk=4)]) as s:
            types = event_types(post(s))
        self.assertEqual(types, ["response.created", "response.output_item.added",
                                 "response.output_text.delta", "response.output_text.delta",
                                 "response.output_text.delta", "response.output_item.done",
                                 "response.completed"])

    def test_tool_event_order_and_usage(self):
        with mock.MockServer([mock.tool_turn("read", {"path": "a.txt"})]) as s:
            stream = post(s)
        types = event_types(stream)
        self.assertEqual(types[0:2], ["response.created", "response.output_item.added"])
        self.assertEqual(types[-3:], ["response.function_call_arguments.done",
                                      "response.output_item.done", "response.completed"])
        done = [json.loads(x[6:]) for x in stream.splitlines() if x.startswith("data: ")][-1]
        self.assertEqual(done["response"]["usage"]["total_tokens"], 17)
        item = done["response"]["output"][0]
        self.assertEqual((item["name"], json.loads(item["arguments"])), ("read", {"path": "a.txt"}))

    def test_unknown_tool_rejected(self):
        with self.assertRaises(ValueError):
            mock.tool_turn("rm", {})

    def test_records_every_request_and_count_is_exact(self):
        with mock.MockServer([mock.text_turn("a"), mock.text_turn("b")]) as s:
            post(s, b'{"input": "one"}')
            post(s, b'{"input": "two"}')
            self.assertEqual([r["body"]["input"] for r in s.post_requests()], ["one", "two"])
            s.assert_request_count(2)
            with self.assertRaises(AssertionError):
                s.assert_request_count(1)
            with self.assertRaises(AssertionError):
                s.assert_request_count(3)

    def test_binds_loopback_only(self):
        with mock.MockServer() as s:
            self.assertEqual(s.host, "127.0.0.1")
            self.assertTrue(s.base_url.startswith("http://127.0.0.1:"))
            self.assertNotEqual(s.port, 0)

    def test_held_turn_waits_for_release(self):
        with mock.MockServer([mock.text_turn("hi", hold=True)]) as s:
            import threading
            result = []
            t = threading.Thread(target=lambda: result.append(post(s)))
            t.start()
            self.assertTrue(s.wait_for_requests(1))
            t.join(0.5)
            self.assertTrue(t.is_alive(), "the stream ended before release")
            s.release()
            t.join(5)
            self.assertEqual(event_types(result[0])[-1], "response.completed")


    def test_release_does_not_complete_a_later_held_turn(self):
        import threading
        import time
        with mock.MockServer([mock.text_turn("one", hold=True), mock.text_turn("two", hold=True)]) as s:
            done = {}

            def call(key):
                done[key] = post(s)

            first = threading.Thread(target=call, args=("one",))
            first.start()
            self.assertTrue(s.wait_for_requests(1))
            deadline = time.time() + 5
            while s.held < 1 and time.time() < deadline:
                time.sleep(0.01)
            releasing = threading.Thread(target=s.release)
            releasing.start()
            time.sleep(0.01)
            second = threading.Thread(target=call, args=("two",))
            second.start()
            releasing.join(5)
            first.join(5)
            second.join(0.4)
            self.assertIn("one", done)
            self.assertTrue(second.is_alive(), "release() completed a hold registered after it")
            s.release()
            second.join(5)
            self.assertIn("two", done)


class ScenarioCheckTest(unittest.TestCase):
    def test_unknown_action_is_blocked_without_a_process(self):
        with tempfile.TemporaryDirectory() as out:
            cap = replay.replay(BINARY, {"actions": [{"type": "teleport"}]}, replay.Profile(), out_dir=out)
        self.assertEqual(cap.status, replay.BLOCKED)
        self.assertIn("teleport", cap.reason)

    def test_unsupported_claude_keys_are_blocked(self):
        for scenario in ({"state": {"x": 1}}, {"onboarding": True}, {"args": ["--resume"]}):
            self.assertIn("not supported", replay.check_scenario(scenario))
        self.assertEqual(replay.check_scenario({"args": [], "env": {}, "actions": []}), "")

    def test_snapshot_names_and_files_are_checked(self):
        self.assertTrue(replay.check_scenario({"actions": [{"type": "snapshot", "name": "../x"}]}))
        self.assertTrue(replay.check_scenario({"files": {"../x": "y"}}))

    def test_request_count_is_mandatory(self):
        turn = {"kind": "text", "text": "x"}
        self.assertIn("expect_requests", replay.check_scenario({"turns": [turn]}))
        self.assertIn("expect_requests", replay.check_scenario({"turns": [turn], "expect_requests": "1"}))
        self.assertIn("expect_requests", replay.check_scenario({"turns": [turn], "expect_requests": True}))
        self.assertEqual(replay.check_scenario({"turns": [turn], "expect_requests": 1}), "")
        self.assertEqual(replay.check_scenario({"actions": []}), "")
        self.assertEqual(replay.check_scenario({"expect_requests": 0}), "")
        self.assertIn("exactly 0", replay.check_scenario({"expect_requests": 2}))

    def test_replay_blocks_scenario_without_count(self):
        with tempfile.TemporaryDirectory() as out:
            cap = replay.replay(BINARY, {"turns": [{"kind": "text"}]}, replay.Profile(), out_dir=out)
        self.assertEqual(cap.status, replay.BLOCKED)
        self.assertIn("expect_requests", cap.reason)

    def test_expected_requests(self):
        self.assertEqual(replay.expected_requests({}), 0)
        self.assertEqual(replay.expected_requests({"turns": [{}], "expect_requests": 2}), 2)

    def test_duplicate_request_is_counted_against_the_expectation(self):
        # The mock repeats its last turn, so a duplicate request still gets an answer.
        with mock.MockServer([mock.text_turn("a")]) as s:
            post(s)
            post(s)
            self.assertEqual(len(s.post_requests()), 2)
            self.assertNotEqual(len(s.post_requests()), replay.expected_requests(
                {"turns": [{}], "expect_requests": 1}))

    def test_out_dir_inside_repository_is_blocked(self):
        inside = replay.REPO_ROOT / "target" / "cc2316_out_should_not_exist"
        cap = replay.replay(BINARY, {}, replay.Profile(), out_dir=inside)
        self.assertEqual(cap.status, replay.BLOCKED)
        self.assertIn("inside the repository", cap.reason)
        self.assertFalse(inside.exists())
        self.assertTrue(replay.inside_repo(replay.REPO_ROOT / "x" / ".." / "y"))
        with tempfile.TemporaryDirectory() as out:
            self.assertFalse(replay.inside_repo(out))

    def test_profile_env_only_allows_presentation_variables(self):
        for key in ("PI_CATALOG_DRY_RUN", "HOME", "PI_DISABLE_NETWORK", "HTTP_PROXY", "USERPROFILE"):
            self.assertIn(key, replay.check_profile(replay.Profile(env={key: "0"})))
        self.assertEqual(replay.check_profile(replay.Profile(
            env={"NO_COLOR": "1", "FORCE_COLOR": "1", "COLORFGBG": "15;0"})), "")
        with self.assertRaises(ValueError):
            replay.build_env(Path("h"), Path("a"), 1, {"PI_CATALOG_DRY_RUN": "0"})
        env = replay.build_env(Path("h"), Path("a"), 1, {"NO_COLOR": "1"})
        self.assertEqual((env["NO_COLOR"], env["PI_CATALOG_DRY_RUN"]), ("1", "1"))

    def test_a_first_run_leaves_the_agent_dir_to_the_shell_but_keeps_every_isolation_variable(self):
        # P07b: the shell asks its first-run question only without a directory override.
        normal = replay.build_env(Path("h"), Path("a"), 1, {})
        first = replay.build_env(Path("h"), Path("a"), 1, {}, first_run=True)
        self.assertEqual({k for k in normal if k not in first},
                         {"PI_CODING_AGENT_DIR", "DAVINCI_CODING_AGENT_DIR"})
        self.assertEqual(first["PI_EXPERIMENTAL"], "1")
        self.assertNotIn("PI_EXPERIMENTAL", normal)
        for key in ("HOME", "USERPROFILE", "PI_DISABLE_NETWORK", "PI_CATALOG_DRY_RUN", "HTTP_PROXY"):
            self.assertEqual(first[key], normal[key])
        self.assertEqual(replay.check_scenario({"first_run": True, "expect_requests": 0}), "")

    def test_replay_blocks_a_profile_that_changes_isolation(self):
        with tempfile.TemporaryDirectory() as out:
            cap = replay.replay(Path(out) / "nope.exe", {}, replay.Profile(env={"HOME": "x"}),
                                out_dir=out)
        self.assertEqual(cap.status, replay.BLOCKED)
        self.assertIn("HOME", cap.reason)

    def test_rooted_and_drive_paths_are_rejected(self):
        with tempfile.TemporaryDirectory() as tmp:
            outside = Path(tmp).resolve() / "outside" / "escape.txt"
            rooted = str(outside)[len(outside.drive):]       # a rooted path without a drive
            for rel in (rooted, "/x/y.txt", "\\x\\y.txt", "C:y.txt", "C:\\y.txt", "a/../b"):
                self.assertIn("relative", replay.check_scenario({"files": {rel: "x"}}), rel)
            work = Path(tmp) / "work"
            work.mkdir()
            self.assertIn("relative", replay.write_files(work, {rooted: "x"}))
            self.assertFalse(outside.exists())
            self.assertEqual(replay.write_files(work, {"sub/ok.txt": "x"}), "")
            self.assertTrue((work / "sub" / "ok.txt").is_file())

    def test_a_target_resolving_outside_work_is_rejected(self):
        import subprocess
        with tempfile.TemporaryDirectory() as tmp:
            work, away = Path(tmp) / "work", Path(tmp) / "away"
            work.mkdir()
            away.mkdir()
            made = subprocess.run(["cmd", "/c", "mklink", "/J", str(work / "link"), str(away)],
                                  capture_output=True)
            if made.returncode != 0:
                self.skipTest("cannot create a junction here")
            self.assertEqual(replay.check_scenario({"files": {"link/x.txt": "x"}}), "")
            self.assertIn("outside", replay.write_files(work, {"link/x.txt": "x"}))
            self.assertFalse((away / "x.txt").exists())

    def test_missing_binary_is_blocked(self):
        with tempfile.TemporaryDirectory() as out:
            cap = replay.replay(Path(out) / "nope.exe", {}, replay.Profile(), out_dir=out)
        self.assertEqual(cap.status, replay.BLOCKED)


class FakeBase:
    """Stands in for the reference Harness, with its offset-based find_text and click_text."""
    def __init__(self, cols, rows, *args):
        self.cols, self.rows, self.clicks = cols, rows, []

    def text(self):
        return ""

    def event(self, *args):
        pass

    def mouse(self, action, x, y, button=0):
        self.clicks.append((action, x, y))

    def find_text(self, needle):
        for y, line in enumerate(self.display().splitlines()):
            x = line.split("|", 1)[1].find(needle)
            if x >= 0:
                return x + 1, y + 1
        return None

    def click_text(self, needle, dx=0):
        pos = self.find_text(needle)
        if not pos:
            return False
        x, y = pos
        self.mouse("left", x + dx, y)
        self.mouse("release", x + dx, y)
        return True


class ClickTextTest(unittest.TestCase):
    def harness(self, rows):
        import types
        dump = {"rows": [{"runs": runs} for runs in rows]}
        xt = types.SimpleNamespace(
            trim_to_current_screen=lambda raw: raw, dump=lambda *a: dump,
            styled_capture=types.SimpleNamespace(sanitize=lambda raw: raw))
        tmp = tempfile.TemporaryDirectory()
        self.addCleanup(tmp.cleanup)
        cls = replay.make_harness_class(types.SimpleNamespace(Harness=FakeBase), xt, Path(tmp.name))
        return cls(20, len(rows))

    def test_text_after_a_wide_glyph_maps_to_its_real_column(self):
        h = self.harness([[{"x": 0, "text": "\u4f60Z"}]])
        self.assertEqual(h.find_text("Z"), (3, 1))      # columns 0-1 are the glyph, Z is column 2

    def test_text_after_a_combining_mark_maps_to_its_real_column(self):
        h = self.harness([[{"x": 0, "text": "e\u0301Z"}]])
        self.assertEqual(h.find_text("Z"), (2, 1))
        self.assertEqual(h.find_text("e\u0301Z"), (1, 1))

    def test_click_uses_the_cell_column_and_row(self):
        h = self.harness([[{"x": 0, "text": "ab"}],
                          [{"x": 1, "text": "\u4f60\u4f60Q"}]])
        self.assertTrue(h.click_text("Q"))
        self.assertEqual(h.clicks, [("left", 6, 2), ("release", 6, 2)])
        self.assertFalse(h.click_text("missing"))


class ScreenSequence:
    """A harness whose screen is the next of ``frames`` on each read, then the last for good."""

    def __init__(self, frames):
        self.frames, self.reads = list(frames), 0

    def display(self):
        self.reads += 1
        return self.frames[min(self.reads, len(self.frames)) - 1]


class WaitForTest(unittest.TestCase):
    def test_it_is_a_known_action_with_a_checked_shape(self):
        self.assertIn("wait_for", replay.ACTIONS)
        ok = {"type": "wait_for", "text": "[Pasted text #1]", "timeout_ms": 5000}
        self.assertEqual(replay.check_scenario({"actions": [ok]}), "")
        self.assertEqual(replay.check_scenario({"actions": [{"type": "wait_for", "text": "x"}]}), "")
        for bad in ({"type": "wait_for"}, {"type": "wait_for", "text": ""},
                    {"type": "wait_for", "text": 3}, {"type": "wait_for", "text": "x", "timeout_ms": 0},
                    {"type": "wait_for", "text": "x", "timeout_ms": "9"},
                    {"type": "wait_for", "text": "x", "timeout_ms": True}):
            self.assertIn("wait_for", replay.check_scenario({"actions": [bad]}), bad)

    def test_it_returns_as_soon_as_the_text_is_on_the_screen(self):
        screen = ScreenSequence(["00|", "00|", "00|[Pasted text #1]", "00|never read"])
        found, last, waited = replay.wait_for(screen, "[Pasted text #1]", 5000, poll_ms=1)
        self.assertTrue(found)
        self.assertEqual(screen.reads, 3)
        self.assertIn("[Pasted text #1]", last)
        self.assertLess(waited, 5000)

    def test_it_gives_up_at_the_ceiling_and_hands_back_the_last_screen(self):
        screen = ScreenSequence(["00|first", "00|the last screen"])
        found, last, waited = replay.wait_for(screen, "absent", 60, poll_ms=5)
        self.assertFalse(found)
        self.assertEqual(last, "00|the last screen")
        self.assertGreaterEqual(waited, 60)

    def test_the_replay_reports_a_timeout_as_a_failure_with_the_screen(self):
        reason = replay.wait_failure(2, "x", 60, "00|screen")
        self.assertIn("action 2", reason)
        self.assertIn("'x'", reason)
        self.assertIn("60 ms", reason)
        self.assertIn("00|screen", reason)


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


def screen_text(path):
    data = json.loads(Path(path).read_text(encoding="utf-8"))
    return "\n".join("".join(r["text"] for r in row) for row in data["rows"])


@unittest.skipIf(live_skip_reason(), live_skip_reason() or "")
class LiveReplayTest(unittest.TestCase):
    def setUp(self):
        self.out = tempfile.mkdtemp(prefix="cc2316_test_")
        self.addCleanup(shutil.rmtree, self.out, True)

    def run_scenario(self, scenario):
        # ``wait_ms`` is the ceiling on the wait for the first screen to hold still, not a
        # pause: the wait ends as soon as two reads agree. Six seconds was short of a start
        # on a loaded machine ("the screen did not settle within 6 s"), which is a slow
        # start rather than a fault, so the ceiling sits well above any start.
        scenario = {"wait_ms": 45000, **scenario}
        cap = replay.replay(BINARY, scenario, replay.Profile(120, 44), out_dir=self.out)
        self.assertEqual(cap.status, replay.PASS, cap.reason)
        self.assertEqual(cap.proxy_attempts, [])
        self.assertTrue({"startup", "mid", "exit", "watch"} <= {s["label"] for s in cap.connection_samples})
        self.assertTrue(cap.raw_path.is_file() and cap.raw_path.stat().st_size > 0)
        return cap

    def test_boot_reaches_a_settled_screen_without_a_request(self):
        cap = self.run_scenario({"expect_requests": 0, "actions": [{"type": "snapshot", "name": "settled"}]})
        data = json.loads(cap.snapshots["settled"].read_text(encoding="utf-8"))
        self.assertEqual(data["schema"], "claude-tui-styled-capture/2")
        self.assertEqual(data["target"]["name"], "DaVinci")
        self.assertEqual((data["dimensions"]["cols"], data["dimensions"]["rows"]), (120, 44))
        self.assertIn("DaVinci", screen_text(cap.snapshots["settled"]))
        html = (Path(self.out) / "settled.html").read_text(encoding="utf-8")
        self.assertNotIn("Claude Code", html)

    def test_text_turn(self):
        cap = self.run_scenario({
            "expect_requests": 1, "turns": [{"kind": "text", "text": "Synthetic reply one."}],
            "actions": [{"type": "key", "text": "hello"}, {"type": "key", "text": "\r", "delay_ms": 400},
                        {"type": "wait", "ms": 2500}, {"type": "snapshot", "name": "after"}]})
        self.assertEqual(cap.request_count, 1)
        self.assertIn("Synthetic reply one.", screen_text(cap.snapshots["after"]))

    def test_read_tool_turn(self):
        cap = self.run_scenario({
            "expect_requests": 2, "files": {"note.txt": "synthetic file body\n"},
            "turns": [{"kind": "tool", "tool": "read", "arguments": {"path": "note.txt"}},
                      {"kind": "text", "text": "Synthetic done."}],
            "actions": [{"type": "key", "text": "read it"}, {"type": "key", "text": "\r", "delay_ms": 400},
                        {"type": "wait", "ms": 3500}, {"type": "snapshot", "name": "after"}]})
        self.assertIn("Synthetic done.", screen_text(cap.snapshots["after"]))

    def test_import_leaves_the_reference_package_untouched(self):
        cache = replay.ref_root() / "scripts" / "__pycache__"

        def snapshot():
            return sorted((p.name, p.stat().st_size, p.stat().st_mtime_ns)
                          for p in cache.iterdir()) if cache.exists() else None

        before, flag = snapshot(), sys.dont_write_bytecode
        replay.load_reference()
        self.assertEqual(snapshot(), before)
        self.assertEqual(sys.dont_write_bytecode, flag)

    def test_duplicate_provider_request_fails_the_run(self):
        cap = replay.replay(BINARY, {
            "turns": [{"kind": "text", "text": "Synthetic reply."}], "expect_requests": 1,
            "actions": [{"type": "key", "text": "one"}, {"type": "key", "text": "\r", "delay_ms": 400},
                        {"type": "wait", "ms": 2000},
                        {"type": "key", "text": "two"}, {"type": "key", "text": "\r", "delay_ms": 400},
                        {"type": "wait", "ms": 2500}]}, replay.Profile(120, 44), out_dir=self.out)
        self.assertEqual(cap.status, replay.FAIL)
        self.assertEqual(cap.request_count, 2)
        self.assertIn("expected exactly 1", cap.reason)

    def test_wrong_request_count_fails_the_run(self):
        cap = replay.replay(BINARY, {"turns": [{"kind": "text"}], "expect_requests": 1, "actions": []},
                            replay.Profile(120, 44), out_dir=self.out)
        self.assertEqual(cap.status, replay.FAIL)
        self.assertIn("expected exactly 1", cap.reason)


if __name__ == "__main__":
    unittest.main()
