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


def completion(code="completion.requirements"):
    return {"type": "completion_reminder", "reason_code": code}


def verification(kind="message_start", value="verification_required"):
    return {"type": kind, "message": {"role": "user", "davinciCapabilityReminder": value,
                                      "davinciCapabilityReminderReason": "masked_status"}}


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

    def test_requests_before_any_gate_have_none(self):
        rows, _ = self.audit([*attempt(100, 0), tool("read", path="a"), *attempt(200, 100)])
        self.assertEqual([r["gate"] for r in rows], [None, None])
        self.assertEqual([r["gates_between"] for r in rows], ["", ""])

    def test_verification_marker_counts_once_and_attributes_following_requests(self):
        rows, _ = self.audit([
            *attempt(100, 0), verification(), verification("message_end"),
            *attempt(200, 100), tool("read", path="a"), *attempt(300, 200),
        ])
        self.assertEqual([r["gate"] for r in rows], [None, "verification_required", "verification_required"])
        self.assertEqual([r["gates_between"] for r in rows], ["", "verification_required", ""])
        self.assertEqual(rows[1]["trigger"], "verification-required")
        summary = audit.summarize({"a": (rows, None)})
        self.assertEqual(summary["per_gate"]["verification_required"]["requests"], 2)
        self.assertEqual(summary["per_gate"]["verification_required"]["runs_fired"], 1)

    def test_both_gates_one_run_later_marker_wins(self):
        rows, _ = self.audit([
            *attempt(100, 0), completion(), *attempt(200, 100), tool("read", path="a"),
            *attempt(300, 200), verification(), *attempt(400, 300),
            completion(), verification(), *attempt(500, 400),
        ])
        self.assertEqual([r["gate"] for r in rows], [None, "completion.requirements", "completion.requirements",
                                                      "verification_required", "verification_required"])
        self.assertEqual(rows[4]["gates_between"], "completion.requirements;verification_required")
        summary = audit.summarize({"a": (rows, None)})
        self.assertEqual(summary["runs_with_both_gates"], 1)
        self.assertEqual(summary["gate_order"], {"completion.requirements_first": 1, "verification_required_first": 0})
        self.assertEqual((summary["requests_total"], summary["requests_after_first_gate"]), (5, 4))
        self.assertEqual(summary["per_gate"]["completion.requirements"]["requests"], 2)

    def test_trailing_marker_after_the_last_request_still_counts_as_fired(self):
        rows, _ = self.audit([*attempt(100, 0), verification()])
        summary = audit.summarize({"a": (rows, None)})
        self.assertEqual(summary["per_gate"]["verification_required"]["runs_fired"], 1)
        self.assertEqual(summary["per_gate"]["verification_required"]["requests"], 0)

    def test_unknown_usage_in_a_gated_row_is_not_zero(self):
        rows, _ = self.audit([*attempt(100, 0), completion(), *attempt(1000, 400),
                              *attempt(0, 0, usage=False), *attempt(900, 900)])
        gate = audit.summarize({"a": (rows, None)})["per_gate"]["completion.requirements"]
        self.assertEqual((gate["requests"], gate["uncached_input"], gate["uncached_input_known_requests"]),
                         (3, 600, 2))

    def test_mutations_are_counted_per_gate_including_batch_children(self):
        rows, _ = self.audit([
            *attempt(100, 0), tool("edit", path="before.py"),
            completion(), *attempt(200, 100),
            tool("batch", operations=[{"tool": "edit", "args": {}}, {"tool": "read", "args": {}},
                                      {"tool": "write", "args": {}}]),
            *attempt(300, 200), tool("read", path="a"), *attempt(400, 300),
        ])
        gate = audit.summarize({"a": (rows, None)})["per_gate"]["completion.requirements"]
        self.assertEqual(gate["mutations"], 2)

    def test_results_join_gives_runs_fired_passed(self):
        fired, _ = self.audit([*attempt(100, 0), completion(), *attempt(200, 100)])
        quiet, _ = self.audit([*attempt(100, 0)])
        summary = audit.summarize({"d/t1-r0": (fired, None), "d/t2-r0": (quiet, None)},
                                  passed={"d/t1-r0": True, "d/t2-r0": False})
        self.assertEqual(summary["per_gate"]["completion.requirements"]["runs_fired_passed"], 1)
        self.assertNotIn("runs_fired_passed", audit.summarize({"d/t1-r0": (fired, None)})["per_gate"][
            "completion.requirements"])

    def test_load_passed_reads_results_by_task_and_rep(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "results.jsonl").write_text(
                json.dumps({"harness": "davinci", "task": "t1", "rep": 2, "grader_pass": True}) + "\n"
                + json.dumps({"harness": "codex", "task": "t1", "rep": 3, "grader_pass": True}) + "\n",
                encoding="utf-8")
            self.assertEqual(audit.load_passed(root), {("t1", 2): True})
            self.assertIsNone(audit.load_passed(root / "missing"))


if __name__ == "__main__":
    unittest.main()
