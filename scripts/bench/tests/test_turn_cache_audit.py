"""Offline checks for the per-request cache and turn audit. No model calls."""
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import turn_cache_audit as audit  # noqa: E402


def attempt(input_total, cache_read, attempt_id=1, purpose="coding", usage=True):
    start = {"type": "provider_observation",
             "observation": {"kind": "attempt_start", "attempt_id": attempt_id, "purpose": purpose}}
    end = {"type": "provider_observation",
           "observation": {"kind": "attempt_end", "attempt_id": attempt_id, "purpose": purpose,
                           "transport": "http", "schema_hash": "s", "status": "completed",
                           "raw_usage": {"input_total": input_total, "cache_read": cache_read, "cache_write": 0,
                                         "output": 5, "reasoning": 0} if usage else None}}
    return [start, end]


def tool(name, **args):
    return {"type": "tool_execution_start", "toolName": name, "args": args}


class TurnCacheAuditTests(unittest.TestCase):
    def audit(self, events, tail=""):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "run.stdout.jsonl"
            path.write_text("".join(json.dumps(event) + "\n" for event in events) + tail, encoding="utf-8")
            return audit.audit_run(path)

    def test_shortfall_and_reuse_follow_the_previous_request(self):
        rows, stats = self.audit([
            *attempt(10000, 9000), tool("read", path="a.py"),
            *attempt(12000, 9984), tool("edit", path="a.py"),      # full: within the block slack of 10000
            *attempt(14000, 4096), tool("bash", command="python -m pytest -q"),  # partial
            *attempt(16000, 0),                                      # zero
            {"type": "harness_stats", "runtime": {"modelTurns": 4}},
        ])
        self.assertEqual([r["reuse"] for r in rows], ["first", "full", "partial", "zero"])
        self.assertEqual([r["prefix_shortfall"] for r in rows], [None, 16, 7904, 14000])
        self.assertEqual([r["action"] for r in rows], ["explore", "edit", "verify", "final-answer"])
        self.assertEqual([r["trigger"] for r in rows], ["start", "tool-results", "tool-results", "tool-results"])
        self.assertEqual(stats, {"modelTurns": 4})

    def test_harness_injected_turns_and_retries_are_labelled(self):
        rows, _ = self.audit([
            *attempt(100, 0), {"type": "completion_reminder", "reason_code": "completion.requirements"},
            *attempt(200, 0), tool("todo", items=[]),
            {"type": "verification_notice", "status": "unverified"},
            *attempt(300, 0), {"type": "auto_retry_start", "attempt": 1},
            *attempt(400, 0, attempt_id=2),
        ])
        self.assertEqual([r["trigger"] for r in rows],
                         ["start", "completion-reminder", "verification-notice", "retry"])
        self.assertEqual(rows[1]["action"], "todo-only")

    def test_batch_children_decide_the_action(self):
        rows, _ = self.audit([
            *attempt(100, 0),
            tool("batch", operations=[{"tool": "read", "args": {"path": "a"}},
                                      {"tool": "exec_command", "args": {"command": "git status --short"}}]),
            *attempt(200, 0),
        ])
        self.assertEqual(rows[0]["action"], "shell")
        self.assertEqual(rows[0]["tools"], "read exec_command")

    def test_missing_usage_is_unknown_and_never_becomes_a_zero_cache_read(self):
        rows, _ = self.audit([*attempt(1000, 900), *attempt(0, 0, usage=False), *attempt(3000, 2900)])
        self.assertEqual([r["reuse"] for r in rows], ["first", "unknown", "first"])
        self.assertEqual([r["prefix_shortfall"] for r in rows], [None, None, None])
        summary = audit.summarize({"run": (rows, None)})
        self.assertEqual((summary["requests"], summary["requests_with_usage"], summary["later_requests"]), (3, 2, 0))
        self.assertIsNone(summary["prefix_shortfall_fraction"])

    def test_helper_requests_and_a_truncated_last_line_are_ignored(self):
        rows, _ = self.audit([*attempt(1000, 0), *attempt(50, 0, purpose="compaction"), *attempt(2000, 1000)],
                             tail='{"type": "provider_observation", "observ')
        self.assertEqual([r["input_total"] for r in rows], [1000, 2000])

    def test_summary_counts_cache_regressions_and_reminder_runs(self):
        first, _ = self.audit([*attempt(10000, 0), *attempt(20000, 10000), *attempt(30000, 5000)])
        second, _ = self.audit([*attempt(10000, 0), {"type": "completion_reminder"}, *attempt(20000, 20000)])
        summary = audit.summarize({"a": (first, {"modelTurns": 3}), "b": (second, {"modelTurns": 2})})
        self.assertEqual(summary["cache_read_lower_than_previous"], 1)
        self.assertEqual(summary["runs_with_completion_reminder"], 1)
        self.assertEqual(summary["prefix_tokens"], 10000 + 20000 + 10000)
        self.assertEqual(summary["prefix_shortfall"], 0 + (20000 - 5000) + 0)
        self.assertEqual(summary["per_run_mean"]["modelTurns"], 2.5)
        # Run b: the reminder-triggered request and everything after it.
        self.assertEqual(summary["requests_from_first_completion_reminder"], 1)
        self.assertEqual(summary["uncached_input_from_first_completion_reminder"], 0)
        self.assertEqual(summary["uncached_input"], 10000 + 10000 + 25000 + 10000 + 0)
        self.assertEqual(summary["reuse"], {"full": 2, "partial": 1})


if __name__ == "__main__":
    unittest.main()
