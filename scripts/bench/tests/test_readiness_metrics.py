"""Requirements-to-evidence campaign metrics; no provider or benchmark runs."""
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import readiness_metrics


def fixture(success=True, **changes):
    row = {"harness": "davinci", "task": "one", "rep": 0, "model": "fixture-model", "effort_policy": "high",
           "size_class": "large", "execution_mode": "print", "exit": 0 if success else 1,
           "grader_pass": success, "unrelated": [], "cleanup_complete": True,
           "transaction_leak": False, "artifact_leak": False, "regression_pass": True,
           "input_tokens": 100, "cached_tokens": 40, "output_tokens": 10, "wall_s": 2,
           "logical_requests": 2, "request_metrics_complete": True, "tool_calls": 3}
    return dict(row, **changes)


class ReadinessMetricsTests(unittest.TestCase):
    def prices(self):
        return {"schema_version": 1, "currency": "USD", "as_of": "2026-09-30",
                "source": "https://example.invalid/fixture-prices", "models": {"fixture-model": {
                "input_per_million": 2, "cached_input_per_million": 1, "output_per_million": 4}}}

    def test_failed_attempt_spend_counts_per_verified_success(self):
        summary = readiness_metrics.summarize([fixture(), fixture(False, rep=1, wall_s=4)], self.prices())
        self.assertEqual(summary["composite_successes"], 1)
        self.assertEqual(summary["regression_free_successes"], 1)
        self.assertEqual(summary["uncached_input_per_verified_success"], 120)
        self.assertEqual(summary["cached_input_per_verified_success"], 80)
        self.assertEqual(summary["output_per_verified_success"], 20)
        self.assertAlmostEqual(summary["estimated_usd_per_verified_success"], .0004)
        self.assertEqual(summary["median_wall_s"], 3)
        self.assertAlmostEqual(summary["p90_wall_s"], 3.8)

    def test_unknown_usage_regression_and_background_are_unavailable(self):
        summary = readiness_metrics.summarize([fixture(input_tokens=None, regression_pass=None)], self.prices())
        self.assertIsNone(summary["uncached_input_per_verified_success"])
        self.assertIsNone(summary["estimated_usd_per_verified_success"])
        self.assertIsNone(summary["regression_free_successes"])
        self.assertIsNone(summary["background_tokens"])
        self.assertEqual(summary["background_scope"], "unmeasured in print mode")
        no_success = readiness_metrics.summarize([fixture(False)], self.prices())
        self.assertIsNone(no_success["output_per_verified_success"])

    def test_report_keeps_arm_size_and_background_separate(self):
        background = {"learning": {"tokens": {"input": 7, "output": 2, "cacheRead": 3, "cacheWrite": 0},
                                    "pendingRequests": 0, "unknownTokenRequests": 0},
                      "securityWatch": {"tokens": {"input": 9, "output": 1, "cacheRead": 0, "cacheWrite": 0},
                                        "pendingRequests": 0, "unknownTokenRequests": 0}}
        rows = [fixture(execution_mode="rpc", background_usage=background),
                fixture(harness="codex", size_class="small", task="two")]
        report = readiness_metrics.report(rows, self.prices())
        self.assertEqual(report["arms"]["davinci"]["all"]["background_tokens"]["learning"]["input"], 7)
        self.assertNotIn("small", report["arms"]["davinci"])
        self.assertEqual(report["arms"]["davinci"]["large"]["output_per_verified_success"], 10)
        self.assertEqual(report["price_label"], "estimate from pinned price table")

    def test_prices_are_pinned_and_invalid_rates_fail(self):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "prices.json"
            path.write_text(json.dumps(self.prices()))
            prices, pinned = readiness_metrics.load_prices(path)
            self.assertTrue(pinned)
            prices["models"]["fixture-model"]["input_per_million"] = -1
            path.write_text(json.dumps(prices))
            with self.assertRaises(ValueError):
                readiness_metrics.load_prices(path)
        self.assertIsNone(readiness_metrics.summarize([fixture()], None)["estimated_usd_per_verified_success"])

    def test_stale_declared_pass_does_not_replace_composite_evidence(self):
        summary = readiness_metrics.summarize([fixture(False, **{"pass": True})], self.prices())
        self.assertEqual(summary["composite_successes"], 0)


if __name__ == "__main__":
    unittest.main()
