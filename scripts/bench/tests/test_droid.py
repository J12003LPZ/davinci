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
