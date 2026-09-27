"""Offline regression tests for comparable benchmark telemetry."""
import importlib.util
import json
from pathlib import Path
import unittest
import sys
from unittest.mock import patch
from types import SimpleNamespace
import tempfile
import contextlib
import io
from test_campaign import row as campaign_row

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))


SPEC = importlib.util.spec_from_file_location(
    "bench", Path(__file__).resolve().parents[1] / "bench.py"
)
bench = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(bench)


class StreamTelemetryTests(unittest.TestCase):
    def test_uninstrumented_jev_is_unavailable_when_coding_telemetry_exists(self):
        event = {"type": "provider_observation", "observation": {
            "schema_version": 1, "logical_request_id": "coding", "purpose": "coding",
            "kind": "logical_start", "status": "started"}}
        stats = bench.parse_stream("davinci", json.dumps(event))
        self.assertEqual(stats["logical_requests"], 1)
        self.assertIsNone(stats["jev_attempts"])

    def test_reports_separate_harnesses_strata_real_uncached_and_availability(self):
        rows = [dict(campaign_row("davinci"), input_tokens=100, cached_tokens=90, requests=9,
                     logical_requests=9, request_metrics_complete=True),
                dict(campaign_row("codex"), input_tokens=300, cached_tokens=290, requests=6,
                     logical_requests=6, request_metrics_complete=True),
                dict(campaign_row("davinci"), task="m-fixture", wall_s=20, input_tokens=200,
                     cached_tokens=150, requests=None, logical_requests=None, request_metrics_complete=False)]
        summary = bench.summarize(rows)
        self.assertEqual(summary["davinci"]["runs"], 2)
        self.assertEqual(summary["davinci"]["uncached_input"], 60)
        self.assertEqual(summary["codex"]["uncached_input"], 10)
        self.assertIsNone(summary["davinci"]["median_logical_requests"])
        self.assertEqual(summary["davinci"]["complete_request_telemetry_runs"], 1)
        strata = bench.stratified_summary(rows)
        self.assertEqual(strata["legacy"]["davinci"]["runs"], 1)
        self.assertEqual(strata["legacy"]["codex"]["runs"], 1)
        self.assertEqual(strata["large"]["davinci"]["runs"], 1)
        self.assertNotIn("codex", strata["large"])

    def test_missing_artifact_and_transaction_evidence_is_not_zero_leaks(self):
        row = dict(campaign_row("davinci"), artifact_leak=None, transaction_leak=False)
        summary = bench.summarize([row])["davinci"]
        self.assertIsNone(summary["artifact_leak_runs"])
        self.assertEqual(summary["transaction_leak_runs"], 0)

    def test_both_successful_latency_is_separate_and_failed_runs_remain(self):
        left = [dict(campaign_row("davinci", rep), task=task, wall_s=10)
                for task in ("t1-intervals", "t2-duration") for rep in range(2)]
        right = [dict(row, wall_s=5) for row in left]
        right[0].update({"pass": False, "grader_pass": False, "wall_s": 1})
        summary = bench.paired_report(left, right)
        self.assertEqual(summary["all_runs"]["pairs"], 4)
        self.assertEqual(summary["both_successful"]["pairs"], 3)

    def test_unknown_attempt_outcome_is_not_complete_telemetry(self):
        events = [{"type": "provider_observation", "observation": {
            "schema_version": 1, "logical_request_id": "r", "purpose": "coding",
            "kind": kind, "attempt_id": attempt, "status": status}}
            for kind, attempt, status in (("logical_start", None, "started"),
                ("attempt_start", 1, "started"), ("attempt_end", 1, "unknown"),
                ("logical_end", None, "failed"))]
        stats = bench.parse_stream("davinci", "\n".join(map(json.dumps, events)))
        self.assertEqual(stats["provider_attempts"], 1)
        self.assertFalse(stats["request_metrics_complete"])

    def test_cache_groups_sum_tokens_and_deduplicate_terminal_usage(self):
        def done(identity, uncached, cached, reasoning):
            return {"type": "message_update", "assistantMessageEvent": {
                "type": "done", "message": {"id": identity}}, "usage": {
                    "input": uncached, "cacheRead": cached, "cacheWrite": 0,
                    "output": 10, "reasoning": reasoning}}
        first, later = done("a", 20, 80, 4), done("b", 100, 100, 6)
        stats = bench.parse_stream("davinci", "\n".join(map(json.dumps, [first, first, later])))
        self.assertEqual(stats["first_request_input_tokens"], 100)
        self.assertEqual(stats["first_request_cached_tokens"], 80)
        self.assertEqual(stats["later_request_input_tokens"], 200)
        self.assertEqual(stats["reasoning_tokens"], 10)
        self.assertEqual(stats["output"], 20)
        missing = done("c", None, 10, None)
        stats = bench.parse_stream("davinci", "\n".join(map(json.dumps, [first, missing])))
        self.assertIsNone(stats["later_request_input_tokens"])
        self.assertIsNone(stats["reasoning_tokens"])
        stats = bench.parse_stream("codex", json.dumps({"type": "turn.completed", "usage": {
            "input_tokens": 300, "cached_input_tokens": 180, "output_tokens": 20}}))
        self.assertIsNone(stats["first_request_input_tokens"])

    def test_executed_leaves_use_dispatch_counter_not_tool_results(self):
        observation = lambda count: {"type": "mutation_observation", "schema_version": 1,
                                     "generation": 0, "executed_leaf_operations": count}
        events = [observation(5), observation(8), observation(7), observation(8)]
        parse = lambda values: bench.parse_stream("davinci", "\n".join(map(json.dumps, values)))
        self.assertEqual(parse(events)["executed_leaf_operations"], 3)
        self.assertIsNone(parse([*events, observation(None)])["executed_leaf_operations"])

    def test_post_reminder_mutations_use_observed_generations(self):
        observation = lambda generation: {"type": "mutation_observation", "schema_version": 1, "generation": generation}
        events = [observation(0), observation(2),
                  {"type": "message_end", "message": {"davinciCapabilityReminder": "verification_required"}},
                  observation(3), observation(3), observation(4)]
        result = bench.parse_stream("davinci", "\n".join(map(json.dumps, events)))
        self.assertEqual(result["mutations_after_reminder"], 2)
        self.assertEqual(result["mutation_metric_scope"], "completion-ledger generations")
        result = bench.parse_stream("davinci", json.dumps(events[2]))
        self.assertIsNone(result["mutations_after_reminder"])

    def test_gate_checks_complete_evidence_not_legacy_absolute_targets(self):
        rows = [campaign_row("davinci"), campaign_row("codex")]
        manifest = {key: rows[0][key] for key in ("schema_version", "campaign", "variant", *bench.COMPARABLE)}
        manifest.update(tasks=["t1-intervals"], repetitions=[0], harnesses=["davinci", "codex"],
                        grading_isolation={"davinci": "diagnostic-only", "codex": "diagnostic-only"},
                        identities={r["harness"]: {key: r[key] for key in ("binary_sha256", "effective_settings")}
                                    for r in rows})
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "campaign.json").write_text(json.dumps(manifest))
            (root / "results.jsonl").write_text("\n".join(map(json.dumps, rows)))
            output = io.StringIO()
            with patch.object(bench, "RUNS", directory), contextlib.redirect_stdout(output):
                with self.assertRaises(SystemExit) as ended:
                    bench.gate()
            self.assertEqual(ended.exception.code, 0)
            self.assertIn("completeness", output.getvalue())
            self.assertIn("not checkpoint acceptance", output.getvalue())

    def test_duplicate_usage_is_counted_once_and_conflicts_are_unavailable(self):
        event = {"type": "message_update", "assistantMessageEvent": {
            "type": "done", "message": {"id": "response1"}},
            "usage": {"input": 20, "cacheRead": 80, "output": 5}}
        result = bench.parse_stream("davinci", "\n".join(map(json.dumps, [event, event])))
        self.assertEqual(result["input"], 100)
        self.assertEqual(result["completed_assistant_messages"], 1)
        conflicting = dict(event, usage={"input": 50, "cacheRead": 50, "output": 5})
        result = bench.parse_stream("davinci", "\n".join(map(json.dumps, [event, conflicting])))
        self.assertFalse(result["usage_available"])
        self.assertIsNone(result["input"])

    def test_cache_cannot_exceed_total_input(self):
        event = {"type": "turn.completed", "usage": {
            "input_tokens": 10, "cached_input_tokens": 20, "output_tokens": 5}}
        result = bench.parse_stream("codex", json.dumps(event))
        self.assertFalse(result["usage_available"])
        self.assertIsNone(result["input"])
        self.assertIsNone(result["cached"])

    def test_changed_paths_include_both_sides_of_renames(self):
        status = "R  allowed.py\0unrelated.py\0?? odd -> name.py\0?? x__pycache__/source.py\0?? __pycache__/cache.pyc\0"
        with patch.object(bench, "git", return_value=SimpleNamespace(stdout=status, returncode=0)):
            self.assertEqual(bench.changed_files("fixture"), [
                "allowed.py", "odd -> name.py", "unrelated.py", "x__pycache__/source.py"])

    def test_failed_git_status_cannot_establish_no_unrelated_changes(self):
        with patch.object(bench, "git", return_value=SimpleNamespace(stdout="", returncode=1)):
            with self.assertRaises(RuntimeError):
                bench.changed_files("fixture")

    def test_malformed_nested_events_do_not_crash(self):
        events = [{"type": "item.completed", "item": ["invalid"]},
                  {"type": "message_update", "assistantMessageEvent": ["invalid"]},
                  {"type": "tool_execution_start", "toolCallId": [], "toolName": {}},
                  {"type": "message_end", "message": {"davinciHarnessVerification": True,
                                                       "content": None}},
                  {"type": "tool_execution_end", "toolCallId": [],
                   "details": {"batch": True, "operations": None}}]
        for harness in ("davinci", "codex"):
            result = bench.parse_stream(harness, "\n".join(map(json.dumps, events)))
            self.assertIsNone(result["requests"])

    def test_codex_turn_is_not_provider_request(self):
        stream = json.dumps({
            "type": "turn.completed",
            "usage": {"input_tokens": 100, "cached_input_tokens": 40,
                      "output_tokens": 12},
        })
        result = bench.parse_stream("codex", stream)
        self.assertIsNone(result["requests"])
        self.assertEqual(result["user_turns"], 1)
        self.assertEqual(result["input"], 100)
        self.assertEqual(result["cached"], 40)

    def test_missing_usage_is_not_zero(self):
        for harness, event in (
            ("codex", {"type": "turn.completed"}),
            ("davinci", {"type": "message_update",
                         "assistantMessageEvent": {"type": "done"}}),
        ):
            with self.subTest(harness=harness):
                result = bench.parse_stream(harness, json.dumps(event))
                for field in ("input", "cached", "output"):
                    self.assertIsNone(result[field])

    def test_partial_usage_remains_unavailable(self):
        stream = "\n".join(json.dumps(event) for event in (
            {"type": "turn.completed", "usage": {
                "input_tokens": 100, "cached_input_tokens": 0,
                "output_tokens": 12}},
            {"type": "turn.completed"},
        ))
        result = bench.parse_stream("codex", stream)
        self.assertIsNone(result["input"])
        self.assertIsNone(result["cached"])
        self.assertIsNone(result["output"])

    def test_summary_preserves_unavailable_metrics(self):
        row = {"harness": "codex", "pass": False, "wall_s": 2.5,
               "input_tokens": None, "cached_tokens": None,
               "output_tokens": None, "tool_calls": 0,
               "requests": None, "unrelated": []}
        result = bench.summarize([row])["codex"]
        for field in ("input_tokens", "cached_tokens", "uncached_input",
                      "output_tokens", "cache_ratio", "median_requests"):
            self.assertIsNone(result[field])

    def test_retry_and_batch_counts_are_distinct(self):
        def observation(kind, attempt=None):
            return {"type": "provider_observation", "observation": {
                "schema_version": 1, "kind": kind, "logical_request_id": "r1",
                "attempt_id": attempt, "purpose": "coding", "status": "completed"}}
        events = [observation("logical_start"), observation("attempt_start", 1),
                  observation("attempt_end", 1), observation("attempt_start", 2),
                  observation("attempt_end", 2), observation("logical_end"),
                  {"type": "tool_execution_start", "toolCallId": "batch1", "toolName": "batch"},
                  {"type": "tool_execution_end", "toolCallId": "batch1", "toolName": "batch",
                   "details": {"batch": True, "operations": [
                       {"tool": "read", "status": "ok"},
                       {"tool": "read", "status": "skipped"}]}}]
        # Duplicate terminal observations must not double request/attempt counts.
        result = bench.parse_stream("davinci", "\n".join(map(json.dumps, events + events[:6])))
        self.assertEqual(result["logical_requests"], 1)
        self.assertEqual(result["provider_attempts"], 2)
        self.assertEqual(result["tool_calls"], 1)
        self.assertEqual(result["batch_children_reported"], 1)
        self.assertIsNone(result["executed_leaf_operations"])

    def test_gate_and_auto_verification_are_separate(self):
        events = [
            {"type": "message_end", "message": {"davinciCapabilityReminder": "verification_required"}},
            {"type": "message_end", "message": {"davinciHarnessVerification": True,
                "content": [{"type": "toolCall", "id": "davinci_verify_1"}]}},
            {"type": "tool_execution_start", "toolCallId": "davinci_verify_1", "toolName": "bash"},
        ]
        result = bench.parse_stream("davinci", "\n".join(map(json.dumps, events)))
        self.assertEqual(result["gate_reminders"], {"verification_required": 1})
        self.assertEqual(result["auto_verify_runs"], 1)
        self.assertEqual(result["tool_calls"], 0)


if __name__ == "__main__":
    unittest.main()
