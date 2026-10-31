"""Droid 1.1.0 session-file shapes as observed live, the 1.0.0 fallbacks, and rederive."""
import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

SPEC = importlib.util.spec_from_file_location("bench", Path(__file__).resolve().parents[1] / "bench.py")
bench = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(bench)

import droid_usage  # noqa: E402

REAL_1_1_0 = {
    "allowSubagentsInScripts": True, "assistantActiveTimeMs": 1234, "autonomyLevel": "high",
    "autonomyMode": "auto-high", "childInclusiveTokenUsageBySessionId": {},
    "compactionThresholdCheckEnabled": True, "inclusiveTokenUsage": {}, "interactionMode": "auto",
    "lastCallTokenUsage": {}, "model": "custom:Codex-via-ChatGPT-(gpt-6.1-sol)-0",
    "providerLock": "openai", "providerLockTimestamp": "2026-10-31T00:00:00Z",
    "reasoningEffort": "high", "tags": [],
    "tokenUsage": {"inputTokens": 12593, "outputTokens": 1088, "cacheCreationTokens": 0,
                   "cacheReadTokens": 25088, "thinkingTokens": 34, "factoryCredits": 15431},
    "toolExecutionMode": "script_only", "toolExecutionModeModelId": "gpt-6.1-sol",
    "toolExecutionModeOverride": "script_only"}


def log_line(sid, count):
    return '[t] INFO: Streaming result | Context: {"sessionId":"%s","count":%d}' % (sid, count)


def write_session(home, sid, body):
    folder = Path(home) / "sessions" / "proj"
    folder.mkdir(parents=True, exist_ok=True)
    (folder / f"{sid}.settings.json").write_text(json.dumps(body))


def write_log(home, lines):
    (Path(home) / "logs").mkdir(parents=True, exist_ok=True)
    (Path(home) / "logs" / "droid-log-single.log").write_text("\n".join(lines))


class RealShapeTests(unittest.TestCase):
    def test_usage_uses_the_1_1_0_names(self):
        with tempfile.TemporaryDirectory() as tmp:
            write_session(tmp, "s1", REAL_1_1_0)
            path = next(iter(droid_usage.droid_session_files(tmp)))
            self.assertEqual(droid_usage.droid_usage_from_session(path),
                             {"input": 12593 + 25088, "cached": 25088, "output": 1088, "reasoning": 34})

    def test_1_0_0_names_are_still_accepted_and_new_names_win(self):
        with tempfile.TemporaryDirectory() as tmp:
            old = {"tokenUsage": {"inputTokens": 100, "cacheReadInputTokens": 50,
                                  "outputTokens": 20, "reasoningTokens": 5}}
            write_session(tmp, "old", old)
            write_session(tmp, "both", {"tokenUsage": {"inputTokens": 1, "cacheReadTokens": 7,
                                                       "cacheReadInputTokens": 99, "thinkingTokens": 3,
                                                       "reasoningTokens": 99, "outputTokens": 2}})
            write_session(tmp, "none", {"tokenUsage": {"inputTokens": 1, "outputTokens": 2}})
            by_stem = {Path(p).name.split(".")[0]: p for p in droid_usage.droid_session_files(tmp)}
            self.assertEqual(droid_usage.droid_usage_from_session(by_stem["old"]),
                             {"input": 150, "cached": 50, "output": 20, "reasoning": 5})
            self.assertEqual(droid_usage.droid_usage_from_session(by_stem["both"]),
                             {"input": 8, "cached": 7, "output": 2, "reasoning": 3})
            # Missing cache and reasoning fields are None, never zero; so input is unknown too.
            self.assertEqual(droid_usage.droid_usage_from_session(by_stem["none"]),
                             {"input": None, "cached": None, "output": 2, "reasoning": None})

    def test_underscore_mode_matches_and_raw_value_is_recorded(self):
        with tempfile.TemporaryDirectory() as tmp:
            write_session(tmp, "s1", REAL_1_1_0)
            write_log(tmp, [log_line("s1", 12593)])
            stats = bench.droid_usage_for_run({}, "w", set(), tmp, requested_mode="script-only")
            self.assertEqual(stats["droid_resolved_tool_execution_mode"], "script_only")
            self.assertIs(stats["droid_mode_mismatch"], False)
            self.assertEqual(stats["droid_session_reasoning_effort"], "high")
            self.assertEqual(stats["droid_session_model"], "custom:Codex-via-ChatGPT-(gpt-6.1-sol)-0")
            self.assertEqual(stats["droid_session_id"], "s1")
            self.assertTrue(stats["droid_session_settings"].endswith("s1.settings.json"))
            self.assertEqual((stats["input"], stats["cached"], stats["output"], stats["reasoning_tokens"]),
                             (37681, 25088, 1088, 34))
            self.assertEqual((stats["requests"], stats["droid_log_uncached_input"]), (1, 12593))
            self.assertTrue(stats["usage_available"])
            self.assertIsNone(stats["droid_usage_error"])
            # A genuinely different mode is still a mismatch, in either spelling.
            self.assertIs(bench.droid_usage_for_run({}, "w", set(), tmp,
                                                    requested_mode="direct-only")["droid_mode_mismatch"], True)
            self.assertIs(droid_usage.normalize_mode("direct_only") == droid_usage.normalize_mode("direct-only"), True)

    def test_rederive_recomputes_from_session_files_without_rewriting_rows(self):
        with tempfile.TemporaryDirectory() as tmp:
            home, runs = Path(tmp) / "factory", Path(tmp) / "runs"
            write_session(home, "sid-real-1", REAL_1_1_0)
            write_session(home, "sid-other", REAL_1_1_0)
            write_log(home, [log_line("sid-real-1", 12593), log_line("sid-other", 5)])
            (runs / "droid-script").mkdir(parents=True)
            (runs / "droid-script" / "t1-r0.stdout.jsonl").write_text('{"session_id":"sid-real-1"}\n')
            (runs / "droid-direct").mkdir()
            (runs / "droid-direct" / "t1-r0.stdout.jsonl").write_text("{}\n")
            rows = [
                {"harness": "droid-script", "task": "t1", "rep": 0, "droid_tool_execution_mode": "script-only",
                 "input_tokens": None, "usage_available": False},
                {"harness": "droid-direct", "task": "t1", "rep": 0, "droid_tool_execution_mode": "direct-only"},
                {"harness": "davinci", "task": "t1", "rep": 0}]
            results = runs / "results.jsonl"
            results.write_text("\n".join(json.dumps(r) for r in rows) + "\n")
            before = results.read_bytes()
            out = droid_usage.rederive(runs, home)
            self.assertEqual(len(out), 2)  # the davinci row is skipped
            good = out[0]
            self.assertEqual((good["session_id"], good["input"], good["cached"], good["output"],
                              good["reasoning"], good["requests"]), ("sid-real-1", 37681, 25088, 1088, 34, 1))
            self.assertEqual(good["droid_log_uncached_input"], 12593)
            self.assertIs(good["droid_mode_mismatch"], False)
            self.assertEqual(good["row"]["usage_available"], False)
            self.assertIn("found 0", out[1]["error"])
            self.assertEqual(results.read_bytes(), before)
            # A recorded session id is used directly.
            rows[1]["droid_session_id"] = "sid-other"
            results.write_text("\n".join(json.dumps(r) for r in rows) + "\n")
            self.assertEqual(droid_usage.rederive(runs, home)[1]["session_id"], "sid-other")
            self.assertIs(droid_usage.rederive(runs, home)[1]["droid_mode_mismatch"], True)


if __name__ == "__main__":
    unittest.main()
