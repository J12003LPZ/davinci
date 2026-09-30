"""Offline paired promotion guardrails over synthetic campaign records."""
import sys
import hashlib
from pathlib import Path
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import readiness_protocol
from campaign import digest
from test_readiness_metrics import fixture


class ReadinessProtocolTests(unittest.TestCase):
    def complete(self):
        prices = {"schema_version": 1, "currency": "USD", "as_of": "2026-09-30",
                  "source": "synthetic fixture", "models": {"fixture-model": {
                  "input_per_million": 2, "cached_input_per_million": 1, "output_per_million": 4}}}
        groups = []
        for split, count in (("dev", 40), ("holdout", 150)):
            baseline, candidate = [], []
            for task in range(count):
                for rep in range(3):
                    name = f"{split}-{task}"
                    common = {"task": name, "rep": rep, "size_class": "small" if task % 2 else "large",
                              "grading_assurance": "independent", "source_sha": "b" * 40, "source_clean": True,
                              "private_suite_digest": "c" * 64, "split": split, "repository_id": f"repo-{task % 3}",
                              "reference_commit": hashlib.sha256(name.encode()).hexdigest()[:40],
                              "service_tier": "default", "price_table_hash": digest(prices)}
                    baseline.append(fixture(task % 3 != 0, **dict(common, variant="baseline", binary_sha256="a" * 64,
                                                                 effective_settings={"requirementReview": False})))
                    candidate.append(fixture(True, **dict(common, variant="candidate", binary_sha256="d" * 64,
                                                          effective_settings={"requirementReview": True})))
            groups.extend((baseline, candidate))
        return groups, prices

    def test_complete_synthetic_evidence_is_only_a_fixture(self):
        groups, prices = self.complete()
        result = readiness_protocol.promotion(*groups, target="composite_success_rate", prices=prices)
        self.assertTrue(result["accepted"], result["rules"])
        self.assertEqual(result["holdout_candidate"]["arms"]["davinci"]["large"]["runs"], 225)

    def test_holdout_unknown_usage_and_excessive_tokens_cannot_confirm(self):
        for patch in ({"input_tokens": None, "cached_tokens": None, "output_tokens": None}, {"input_tokens": 10000}):
            groups, prices = self.complete()
            groups[3] = [dict(row, **patch) for row in groups[3]]
            result = readiness_protocol.promotion(*groups, target="composite_success_rate", prices=prices)
            self.assertFalse(result["accepted"])

    def test_frozen_configuration_and_prices_cannot_drift(self):
        groups, prices = self.complete()
        groups[1][2]["effective_settings"] = {"requirementReview": False}
        with self.assertRaises(ValueError):
            readiness_protocol.validate_pairs(*groups[:2])
        groups, prices = self.complete()
        groups[3] = [dict(row, effective_settings={"requirementReview": False}) for row in groups[3]]
        with self.assertRaises(ValueError):
            readiness_protocol.promotion(*groups, target="composite_success_rate", prices=prices)
        groups, prices = self.complete()
        groups[3] = [dict(row, price_table_hash="f" * 64) for row in groups[3]]
        with self.assertRaises(ValueError):
            readiness_protocol.promotion(*groups, target="composite_success_rate", prices=prices)

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
