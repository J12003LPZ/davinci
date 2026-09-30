"""Offline paired promotion guardrails over synthetic campaign records."""
import sys
from pathlib import Path
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import readiness_protocol
from test_readiness_metrics import fixture


class ReadinessProtocolTests(unittest.TestCase):
    def rows(self, tasks=4, winner=False):
        baseline, candidate = [], []
        for task in range(tasks):
            for rep in range(3):
                common = {"task": f"task-{task}", "rep": rep, "size_class": "small" if task % 2 else "large",
                          "grading_assurance": "diagnostic-only", "binary_sha256": "a" * 64,
                          "source_sha": "b" * 40, "source_clean": True, "private_suite_digest": "c" * 64,
                          "split": "dev", "variant": "baseline"}
                baseline.append(fixture(not winner, **common))
                candidate.append(fixture(True, **dict(common, variant="candidate", binary_sha256="d" * 64)))
        return baseline, candidate

    def test_task_cluster_bootstrap_reports_difference_without_pseudoreplication(self):
        baseline, candidate = self.rows(winner=True)
        interval = readiness_protocol.success_interval(baseline, candidate)
        self.assertEqual(interval["clusters"], 4)
        self.assertEqual(interval["ci95"], [1.0, 1.0])
        baseline, candidate = self.rows()
        interval = readiness_protocol.success_interval(baseline, candidate)
        self.assertTrue(interval["includes_no_change"])
        self.assertEqual(interval["conclusion"], "no measurable difference")

    def test_missing_duplicates_and_wrong_effort_fail_pair_validation(self):
        baseline, candidate = self.rows()
        for invalid in (candidate[:-1], candidate + candidate[:1], [dict(row, effort_policy="low") for row in candidate]):
            with self.subTest(count=len(invalid)), self.assertRaises(ValueError):
                readiness_protocol.validate_pairs(baseline, invalid)

    def test_diagnostic_and_unconfirmed_results_cannot_promote(self):
        baseline, candidate = self.rows(winner=True)
        result = readiness_protocol.promotion(baseline, candidate, None, None, target="composite_success_rate")
        self.assertFalse(result["accepted"])
        self.assertEqual(result["rules"]["private_dev_sizes"]["status"], "fail")
        self.assertEqual(result["rules"]["independent_grading"]["status"], "fail")
        self.assertEqual(result["rules"]["holdout_confirmation"]["status"], "unavailable")

    def test_any_size_regression_and_token_ceiling_are_visible(self):
        baseline, candidate = self.rows()
        candidate[0] = dict(candidate[0], grader_pass=False, input_tokens=200)
        result = readiness_protocol.promotion(baseline, candidate, None, None, target="composite_success_rate")
        self.assertEqual(result["rules"]["no_size_class_success_regression"]["status"], "fail")
        for row in candidate:
            row["input_tokens"] = 200
        result = readiness_protocol.promotion(baseline, candidate, None, None, target="composite_success_rate")
        self.assertEqual(result["rules"]["uncached_token_ceiling"]["status"], "fail")

    def test_protocol_schedule_has_three_repetitions_and_rotates_arms(self):
        prepared = readiness_protocol.prepare(["one", "two"], ["stable", "requirements", "preview"], seed=7)
        self.assertEqual(len(prepared["schedule"]), 18)
        self.assertEqual(len({tuple(item) for item in prepared["schedule"]}), 18)
        first = [prepared["schedule"][index][0] for index in range(0, 18, 3)]
        self.assertGreater(len(set(first)), 1)
        self.assertFalse(prepared["model_calls_performed"])


if __name__ == "__main__":
    unittest.main()
