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

    def test_missing_binary_is_blocked(self):
        with tempfile.TemporaryDirectory() as out:
            cap = replay.replay(Path(out) / "nope.exe", {}, replay.Profile(), out_dir=out)
        self.assertEqual(cap.status, replay.BLOCKED)


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
        self.assertFalse(cache.exists(), "reference package already has a __pycache__")
        replay.load_reference()
        self.assertFalse(cache.exists())
        self.assertFalse(sys.dont_write_bytecode)

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
