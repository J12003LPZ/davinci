"""Offline tests for the Droid bench arm, usage parser, and campaign defaults."""
import importlib.util
import json
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

BENCH_SPEC = importlib.util.spec_from_file_location(
    "bench", Path(__file__).resolve().parents[1] / "bench.py"
)
bench = importlib.util.module_from_spec(BENCH_SPEC)
BENCH_SPEC.loader.exec_module(bench)

import droid_usage  # noqa: E402  (sys.path was just extended)

CAMPAIGN_SPEC = importlib.util.spec_from_file_location(
    "campaign", Path(__file__).resolve().parents[1] / "campaign.py"
)
campaign = importlib.util.module_from_spec(CAMPAIGN_SPEC)
CAMPAIGN_SPEC.loader.exec_module(campaign)

from test_campaign import row as campaign_row  # noqa: E402


class DroidCommandTests(unittest.TestCase):
    def test_droid_command_shape(self):
        with tempfile.TemporaryDirectory() as tmp:
            workdir = str(Path(tmp) / "t1-intervals-r0")
            with (patch.object(bench, "MODEL", "gpt-6-luna"),
                  patch.object(bench, "EFFORT", "high"),
                  patch.object(bench, "DROID_TOOL_EXECUTION_MODE", "script-only")):
                args = bench.command("droid", "hi", workdir)
            self.assertIsInstance(args, list)
            self.assertIn("exec", args)
            self.assertEqual(args[args.index("--model") + 1], "gpt-6-luna")
            self.assertEqual(args[args.index("--auto") + 1], "high")
            self.assertEqual(args[args.index("--tool-execution-mode") + 1], "script-only")
            self.assertEqual(args[args.index("--output-format") + 1], "stream-json")
            self.assertEqual(args[args.index("--settings") + 1], workdir + ".droid-settings.json")
            self.assertEqual(args[args.index("--cwd") + 1], workdir)
            self.assertEqual(args[-1], "hi")
            # No ~-expanded user settings path, ever.
            self.assertNotIn(str(Path.home() / ".factory" / "settings.json"), args)
            self.assertFalse(any(".factory" in arg for arg in args if isinstance(arg, str)))

    def test_droid_settings_file_content_and_determinism(self):
        with tempfile.TemporaryDirectory() as tmp:
            workdir = str(Path(tmp) / "run")
            for mode in ("direct-only", "script-only"):
                with patch.object(bench, "DROID_TOOL_EXECUTION_MODE", mode):
                    path, first = bench.droid_settings(workdir)
                    _, second = bench.droid_settings(workdir)
                self.assertEqual(first, second)
                self.assertEqual(json.loads(Path(path).read_text()), {"toolExecutionMode": mode})
            with self.assertRaises(ValueError):
                bench.droid_settings(workdir, "bogus")

    def test_harness_argparse_accepts_droid(self):
        parser = bench.build_parser()
        self.assertEqual(parser.parse_args(["run", "--harness", "droid"]).harness, ["droid"])
        self.assertEqual(parser.parse_args(["run", "--harness", "davinci", "codex", "droid"]).harness,
                         ["davinci", "codex", "droid"])
        with self.assertRaises(SystemExit):
            parser.parse_args(["run", "--harness", "bogus"])

    def test_droid_stream_counts_tool_calls_and_excludes_text(self):
        events = [
            {"type": "tool_call", "name": "Read"},
            {"type": "tool_call", "toolName": "Grep"},
            {"type": "assistant_message"},
            {"type": "reasoning"},
            {"type": "tool_call", "name": "Read"},
        ]
        result = bench.parse_stream("droid", "\n".join(json.dumps(event) for event in events))
        self.assertEqual(result["tool_calls"], 3)
        self.assertEqual(result["tools"], {"Read": 2, "Grep": 1})
        for field in ("input", "cached", "output", "requests"):
            self.assertIsNone(result[field])


class DroidUsageTests(unittest.TestCase):
    def test_session_usage_totals_and_missing_output(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "session.settings.json"
            path.write_text(json.dumps({"tokenUsage": {
                "inputTokens": 100, "cacheReadInputTokens": 50,
                "outputTokens": 20, "reasoningTokens": 5}}))
            usage = droid_usage.droid_usage_from_session(str(path))
            self.assertEqual(usage, {"input": 150, "cached": 50, "output": 20, "reasoning": 5})
            path.write_text(json.dumps({"tokenUsage": {"inputTokens": 100, "cacheReadInputTokens": 50}}))
            usage = droid_usage.droid_usage_from_session(str(path))
            self.assertEqual(usage["input"], 150)
            self.assertEqual(usage["cached"], 50)
            self.assertIsNone(usage["output"])
            self.assertIsNone(usage["reasoning"])

    def test_request_count_sums_and_none_without_match(self):
        log = "\n".join([
            '[2026-10-31T17:16:23.913Z] INFO: Streaming result | '
            'Context: {"sessionId":"session-A","count":100}',
            '[2026-10-31T17:16:24.913Z] INFO: Streaming result | '
            'Context: {"sessionId":"session-A","count":250}',
            '[2026-10-31T17:16:25.913Z] INFO: Streaming result | '
            'Context: {"sessionId":"session-A","count":150}',
            '[2026-10-31T17:16:26.913Z] INFO: Streaming result | '
            'Context: {"sessionId":"session-B","count":10}',
            '[2026-10-31T17:16:27.913Z] INFO: Streaming result | '
            'Context: {"sessionId":"session-B","count":20}',
        ])
        result = droid_usage.droid_request_count(log, "session-A")
        self.assertEqual(result["requests"], 3)
        self.assertEqual(result["uncached_input"], 500)
        self.assertIsNone(droid_usage.droid_request_count(log, "session-none"))
        no_count = '[2026-10-31T17:16:23.913Z] INFO: Streaming result | ' \
                   'Context: {"sessionId":"session-A"}'
        result = droid_usage.droid_request_count(no_count, "session-A")
        self.assertEqual(result["requests"], 1)
        self.assertIsNone(result["uncached_input"])


class DroidPerRunTests(unittest.TestCase):
    def _home(self, tmp, sessions, log_lines=()):
        home = Path(tmp)
        for sid in sessions:
            d = home / "sessions" / "proj"
            d.mkdir(parents=True, exist_ok=True)
            (d / f"{sid}.settings.json").write_text(json.dumps({"tokenUsage": {
                "inputTokens": 10, "cacheReadInputTokens": 5,
                "outputTokens": 3, "reasoningTokens": 1}}))
        (home / "logs").mkdir(exist_ok=True)
        (home / "logs" / "droid-log-single.log").write_text("\n".join(log_lines))
        return home

    @staticmethod
    def _line(sid, count):
        return '[t] INFO: Streaming result | Context: {"sessionId":"%s","count":%d}' % (sid, count)

    def test_one_new_session_yields_usage_and_metrics_for_campaign(self):
        with tempfile.TemporaryDirectory() as tmp:
            home = self._home(tmp, ["old"], [self._line("old", 1)])
            before = droid_usage.droid_session_files(home)
            self._home(tmp, ["old", "new"], [self._line("old", 1), self._line("new", 7),
                                             self._line("new", 9)])
            stats = bench.droid_usage_for_run({}, "w", before, home)
            self.assertEqual(stats["input"], 15)
            self.assertEqual(stats["requests"], 2)
            self.assertEqual(stats["logical_requests"], 2)
            self.assertIsNone(stats["provider_attempts"])
            self.assertTrue(stats["request_metrics_available"])
            self.assertTrue(stats["request_metrics_complete"])
            self.assertEqual(stats["droid_log_uncached_input"], 16)
            self.assertIsNone(stats["droid_usage_error"])
            self.assertTrue(stats["usage_available"])
            # campaign.py reads logical_requests per success; a droid row carries it.
            self.assertIn("logical_requests", campaign.PER_SUCCESS_FIELDS)
            r = campaign_row("droid")
            r["logical_requests"] = stats["logical_requests"]
            self.assertIsNotNone(r["logical_requests"])

    def test_zero_or_many_new_sessions_leave_usage_none_with_reason(self):
        with tempfile.TemporaryDirectory() as tmp:
            home = self._home(tmp, [])
            stats = bench.droid_usage_for_run({}, "w", set(), home)
            self.assertIsNone(stats["input"])
            self.assertIsNone(stats["logical_requests"])
            self.assertFalse(stats["request_metrics_available"])
            self.assertFalse(stats["usage_available"])
            self.assertIn("found 0", stats["droid_usage_error"])
            self._home(tmp, ["a", "b"], [self._line("a", 1), self._line("b", 1)])
            stats = bench.droid_usage_for_run({}, "w", set(), home)
            self.assertIsNone(stats["input"])
            self.assertIsNone(stats["requests"])
            self.assertIn("found 2", stats["droid_usage_error"])

    def test_session_id_match_is_exact(self):
        log = "\n".join([self._line("S", 1), self._line("S2", 1), self._line("S2", 1)])
        self.assertEqual(droid_usage.droid_request_count(log, "S")["requests"], 1)
        self.assertEqual(droid_usage.droid_request_count(log, "S2")["requests"], 2)
        bare = "Streaming result session S2 count=4\nStreaming result session S count=3"
        self.assertEqual(droid_usage.droid_request_count(bare, "S")["requests"], 1)


class DroidGateTests(unittest.TestCase):
    def _gate_code(self, harnesses):
        with tempfile.TemporaryDirectory() as tmp:
            (Path(tmp) / "campaign.json").write_text(json.dumps({"harnesses": harnesses}))
            with (patch.object(bench, "RUNS", tmp), patch.object(bench, "load_rows", return_value=[]),
                  patch.object(bench, "campaign_errors", return_value=[]),
                  patch("builtins.print")):
                with self.assertRaises(SystemExit) as raised:
                    bench.gate()
            return raised.exception.code

    def test_gate_harness_sets(self):
        for ok in (["davinci", "codex"], ["davinci", "droid"], ["davinci", "codex", "droid"]):
            self.assertEqual(self._gate_code(ok), 0, ok)
        for bad in (["codex", "droid"], ["davinci"], ["davinci", "bogus"]):
            self.assertEqual(self._gate_code(bad), 1, bad)


class DroidCampaignTests(unittest.TestCase):
    def test_three_harness_schedule_rotates_first_positions(self):
        tasks = ["t1-intervals", "t2-duration", "t3-lru"]
        harnesses = ["davinci", "codex", "droid"]
        order = campaign.schedule(tasks, range(2), harnesses, 0)
        firsts = []
        for rep in range(2):
            for task in tasks:
                block = [harness for (harness, t, r) in order if t == task and r == rep]
                firsts.append(block[0])
        self.assertEqual(set(firsts), set(harnesses))

    def test_integrity_errors_accepts_three_arm_manifest(self):
        rows = [campaign_row("davinci"), campaign_row("codex"), campaign_row("droid")]
        manifest = {key: rows[0][key] for key in
                    ("schema_version", "campaign", "variant", *campaign.COMPARABLE)}
        manifest.update(
            tasks=["t1-intervals"], repetitions=[0],
            harnesses=["davinci", "codex", "droid"],
            grading_isolation={"davinci": "diagnostic-only", "codex": "diagnostic-only",
                               "droid": "diagnostic-only"},
            identities={r["harness"]: {key: r[key] for key in
                                       ("binary_sha256", "effective_settings")} for r in rows})
        self.assertEqual(campaign.manifest_errors(manifest, rows), [])

    def test_integrity_errors_default_includes_droid_arm(self):
        rows = [campaign_row("davinci"), campaign_row("codex"), campaign_row("droid")]
        self.assertEqual(campaign.integrity_errors(rows, ["t1-intervals"], [0]), [])


if __name__ == "__main__":
    unittest.main()
