"""Requirements-to-evidence campaign metrics; no provider or benchmark runs."""
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import readiness_metrics
import bench


def fixture(success=True, **changes):
    row = {"harness": "davinci", "task": "one", "rep": 0, "model": "fixture-model", "effort_policy": "high",
           "size_class": "large", "execution_mode": "print", "exit": 0 if success else 1,
           "grader_pass": success, "unrelated": [], "cleanup_complete": True,
           "transaction_leak": False, "artifact_leak": False, "regression_pass": True,
           "input_tokens": 100, "cached_tokens": 40, "cache_write_tokens": 0, "output_tokens": 10, "wall_s": 2,
           "logical_requests": 2, "request_metrics_complete": True, "tool_calls": 3}
    return dict(row, **changes)


class ReadinessMetricsTests(unittest.TestCase):
    def test_simplification_metrics_keep_overlapping_spans_and_unknown_usage(self):
        for worker_usage in ({"input": 20, "output": 7, "cacheRead": 0, "cacheWrite": 0}, None):
            with self.subTest(worker_usage=worker_usage):
                events = []
                for purpose, usage, status in (
                    ("coding", {"input": 10, "output": 5, "cacheRead": 0, "cacheWrite": 0}, "completed"),
                    ("worker", worker_usage, "failed"),
                ):
                    for kind in ("logical_start", "attempt_start", "attempt_end", "logical_end"):
                        events.append({"type": "provider_observation", "observation": {
                            "schema_version": 1, "logical_request_id": purpose, "purpose": purpose,
                            "kind": kind, "attempt_id": 1 if kind.startswith("attempt") else None,
                            "status": "started" if kind.endswith("start") else status,
                            "usage": usage if kind == "attempt_end" else None,
                            "usage_complete": usage is not None and kind == "attempt_end"}})
                parsed = bench.parse_stream("davinci", "\n".join(map(json.dumps, events)))
                summary = readiness_metrics.summarize([fixture(
                    input_tokens=parsed["input"], output_tokens=parsed["output"], cached_tokens=parsed["cached"],
                    cache_write_tokens=parsed["cache_write"],
                    runtime_stats={"wallMs": 2000, "toolWallMs": 1000, "verificationWorkMs": 4000})], self.prices())
                self.assertEqual(summary["median_wall_s"], 2)
                self.assertEqual(summary["latency_ms"]["wallMs"], 2000)
                self.assertEqual(summary["latency_ms"]["verificationWorkMs"], 4000)
                self.assertIsNone(summary["latency_ms"]["queueMs"])
                if worker_usage is None:
                    self.assertIsNone(summary["estimated_usd"])
                    self.assertIsNone(summary["output_per_verified_success"])
                else:
                    self.assertEqual(summary["output_per_verified_success"], 12)
                    self.assertAlmostEqual(summary["estimated_usd"], .000108)

    def test_simplification_report_keeps_failures_and_cold_warm_separation(self):
        rows = [fixture(startup_state="cold"),
                fixture(False, startup_state="cold", exit="timeout", wall_s=10),
                fixture(False, startup_state="warm", exit="aborted", wall_s=1),
                fixture(False, startup_state="warm", wall_s=3),
                fixture(False, startup_state="unverified", exit=None, input_tokens=None),
                fixture(cached_tokens=99)]
        arm = readiness_metrics.report(rows, self.prices())["arms"]["davinci"]
        self.assertEqual(arm["all"]["runs"], 6)
        self.assertEqual(arm["all"]["composite_successes"], 2)
        self.assertIsNone(arm["all"]["estimated_usd"])
        self.assertEqual(arm["all"]["attempt_outcomes"], {
            "completed": 2, "failed": 1, "timed_out": 1, "aborted": 1, "unknown": 1})
        self.assertEqual({key: value["runs"] for key, value in arm["startup"].items()},
                         {"cold": 2, "warm": 2, "unknown": 2})
        self.assertEqual(arm["startup"]["cold"]["median_wall_s"], 6)
        self.assertEqual(arm["startup"]["warm"]["composite_successes"], 0)
        self.assertIsNone(arm["startup"]["unknown"]["estimated_usd"])
        # Provider cache hits and row order cannot establish process startup state.
        self.assertEqual(readiness_metrics.report(list(reversed(rows)))["arms"]["davinci"]["startup"],
                         readiness_metrics.report(rows)["arms"]["davinci"]["startup"])

    def test_harness_stage_diagnostics_preserve_overlap_and_unknowns(self):
        row = fixture(runtime_stats={"wallMs": 2000, "queueMs": 10, "providerMs": 900,
                                     "verificationWorkMs": 4000, "integrationMs": None})
        summary = readiness_metrics.summarize([row])
        self.assertEqual(summary["latency_ms"]["wallMs"], 2000)
        self.assertEqual(summary["latency_ms"]["verificationWorkMs"], 4000)
        self.assertIsNone(summary["latency_ms"]["integrationMs"])
        self.assertIsNone(readiness_metrics.summarize([row, fixture()])["latency_ms"]["queueMs"])
        self.assertIsNone(readiness_metrics.summarize([fixture(runtime_stats=[])])["latency_ms"]["queueMs"])

    def test_harness_waste_diagnostics_preserve_coverage(self):
        row = fixture(runtime_stats={"diagnosticComparableOperations": 4,
                                     "diagnosticUnknownOperations": 3,
                                     "repeatedReads": 2, "repeatedSearches": 0,
                                     "workerDuplicateOperations": 1, "diagnosticsMs": 7})
        result = readiness_metrics.summarize([row])
        self.assertEqual(result["read_search_diagnostics"]["repeatedReads"], 2)
        self.assertEqual(result["read_search_diagnostics"]["diagnosticUnknownOperations"], 3)
        self.assertEqual(result["latency_ms"]["diagnosticsMs"], 7)
        mixed = readiness_metrics.summarize([row, fixture()])
        self.assertIsNone(mixed["read_search_diagnostics"]["repeatedReads"])

    def test_harness_parallel_time_is_not_summed_as_wall_time(self):
        summary = readiness_metrics.summarize([
            fixture(wall_s=2, worker_wall_s=100, provider_attempts=2,
                    usage_complete_attempts=1, usage_unknown_attempts=1)])
        self.assertEqual(summary["median_wall_s"], 2)
        self.assertEqual(summary["usage_completeness_ratio"], .5)

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
        for change in ({"as_of": ""}, {"as_of": "2026-02-30"}, {"source": " "}):
            with self.subTest(change=change), self.assertRaises(ValueError):
                readiness_metrics.validate_prices(dict(self.prices(), **change))
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

    def test_absent_cache_write_measurement_is_not_a_zero_charge(self):
        row = fixture()
        del row["cache_write_tokens"]
        self.assertIsNone(readiness_metrics.estimate(row, self.prices()))
        row = fixture(cache_write_tokens=10)
        self.assertIsNone(readiness_metrics.estimate(row, self.prices()))
        prices = self.prices()
        prices["models"]["fixture-model"]["cache_write_per_million"] = 3
        self.assertAlmostEqual(readiness_metrics.estimate(row, prices), .00021)


if __name__ == "__main__":
    unittest.main()
