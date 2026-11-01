"""Offline tests for the Stage 0 gap-study analysis (preregistration sections 4-6)."""
import json
import math
import sys
import tempfile
import unittest
from contextlib import redirect_stdout
from io import StringIO
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import gap_study  # noqa: E402


def row(arm, task, requests=4, input_tokens=1000, cached=400, output=100, reasoning=20,
        wall_s=10.0, passed=True, **extra):
    base = {"harness": arm, "task": task, "rep": 0, "pass": passed, "usage_available": True,
            "logical_requests": requests, "input_tokens": input_tokens, "cached_tokens": cached,
            "output_tokens": output, "reasoning_tokens": reasoning,
            "reasoning_included_in_output": True, "wall_s": wall_s, "stop_reason": None}
    base.update(extra)
    return base


class MetricTests(unittest.TestCase):
    def test_five_metrics_are_read_from_the_row(self):
        r = row("davinci", "t", requests=5, input_tokens=1000, cached=400, output=100, reasoning=20, wall_s=1.2345)
        self.assertEqual(gap_study.metric_value(r, "requests"), 5)
        self.assertEqual(gap_study.metric_value(r, "input_uncached"), 600)
        self.assertEqual(gap_study.metric_value(r, "input_total"), 1000)
        self.assertEqual(gap_study.metric_value(r, "output_plus_reasoning"), 100)
        self.assertEqual(gap_study.metric_value(r, "wall_ms"), 1234)

    def test_reasoning_rule_follows_the_flag_and_the_droid_switch(self):
        separate = row("droid-direct", "t", output=100, reasoning=20, reasoning_included_in_output=False)
        self.assertEqual(gap_study.metric_value(separate, "output_plus_reasoning"), 120)
        included = row("droid-direct", "t", output=100, reasoning=20)
        self.assertEqual(gap_study.metric_value(included, "output_plus_reasoning"), 100)
        self.assertEqual(gap_study.metric_value(included, "output_plus_reasoning",
                                                droid_reasoning_excluded=True), 120)
        davinci = row("davinci", "t", output=100, reasoning=20)
        self.assertEqual(gap_study.metric_value(davinci, "output_plus_reasoning",
                                                droid_reasoning_excluded=True), 100)
        missing = row("droid-direct", "t", reasoning=None, reasoning_included_in_output=False)
        self.assertIsNone(gap_study.metric_value(missing, "output_plus_reasoning"))

    def test_zero_missing_or_invalid_values_are_none(self):
        self.assertIsNone(gap_study.metric_value(row("davinci", "t", cached=1000), "input_uncached"))
        self.assertIsNone(gap_study.metric_value(row("davinci", "t", requests=0), "requests"))
        self.assertIsNone(gap_study.metric_value(row("davinci", "t", input_tokens=None), "input_total"))
        self.assertIsNone(gap_study.metric_value(row("davinci", "t", wall_s=float("nan")), "wall_ms"))

    def test_plain_droid_rows_map_to_an_arm_by_recorded_mode(self):
        self.assertEqual(gap_study.arm_of({"harness": "droid", "droid_tool_execution_mode": "direct-only"}),
                         "droid-direct")
        self.assertIsNone(gap_study.arm_of({"harness": "droid"}))
        self.assertIsNone(gap_study.arm_of({"harness": "codex"}))


class StatisticsTests(unittest.TestCase):
    def test_hand_checkable_geometric_mean(self):
        self.assertAlmostEqual(gap_study.geometric_mean([2.0, 0.5]), 1.0)
        self.assertAlmostEqual(gap_study.geometric_mean([4.0, 1.0, 0.25, 1.0]), 1.0)
        self.assertAlmostEqual(gap_study.geometric_mean([8.0, 2.0]), 4.0)

    def test_bootstrap_is_seeded_bounded_and_degenerate_for_one_ratio(self):
        ratios = [0.5, 0.8, 1.0, 1.25, 2.0]
        first = gap_study.bootstrap_interval(ratios, 7, resamples=2000)
        self.assertEqual(first, gap_study.bootstrap_interval(ratios, 7, resamples=2000))
        self.assertNotEqual(first, gap_study.bootstrap_interval(ratios, 8, resamples=2000))
        self.assertGreaterEqual(first[0], min(ratios))
        self.assertLessEqual(first[1], max(ratios))
        low, high = gap_study.bootstrap_interval([2.0], 1, resamples=100)
        self.assertAlmostEqual(low, 2.0)
        self.assertAlmostEqual(high, 2.0)

    def test_label_boundaries(self):
        label = gap_study.label
        self.assertEqual(label(0.5, 0.85), "GAP")
        self.assertEqual(label(0.5, 0.8500001), "NO_CLEAR_GAP")
        self.assertEqual(label(1.15, 2.0), "AHEAD")
        self.assertEqual(label(1.1499999, 2.0), "NO_CLEAR_GAP")
        self.assertEqual(label(0.8, 1.2), "NO_CLEAR_GAP")
        # GAP wins only on the upper bound, AHEAD only on the lower bound.
        self.assertEqual(label(0.1, 0.84), "GAP")
        self.assertEqual(label(1.2, 3.0), "AHEAD")


class EstimateTests(unittest.TestCase):
    def two_arm(self, factor, tasks=("a", "b", "c")):
        rows = []
        for task in tasks:
            rows.append(row("davinci", task, requests=10))
            rows.append(row("droid-direct", task, requests=10 * factor))
        return gap_study.index_rows(rows)

    def test_constant_ratio_gives_that_ratio_and_the_matching_label(self):
        result = gap_study.estimate(self.two_arm(0.5), "droid-direct", "davinci", "requests",
                                    seed=3, resamples=500, confidence=0.9, droid_reasoning_excluded=False)
        self.assertEqual(result["n"], 3)
        self.assertAlmostEqual(result["geometric_mean"], 0.5)
        self.assertEqual(result["label"], "GAP")
        result = gap_study.estimate(self.two_arm(2.0), "droid-direct", "davinci", "requests",
                                    seed=3, resamples=500, confidence=0.9, droid_reasoning_excluded=False)
        self.assertEqual(result["label"], "AHEAD")
        self.assertEqual(result["pair"], "droid-direct vs davinci")

    def test_exclusions_are_counted_by_reason_and_never_imputed(self):
        rows = [
            row("davinci", "ok", requests=10), row("droid-direct", "ok", requests=5),
            row("davinci", "fail", requests=10), row("droid-direct", "fail", requests=5, passed=False),
            row("davinci", "inc", requests=10), row("droid-direct", "inc", requests=5, usage_available=False),
            row("davinci", "zero", requests=10), row("droid-direct", "zero", requests=5, cached=1000),
            row("davinci", "mode", requests=10), row("droid-direct", "mode", requests=5, droid_mode_mismatch=True),
            row("davinci", "solo", requests=10),
        ]
        result = gap_study.estimate(gap_study.index_rows(rows), "droid-direct", "davinci", "input_uncached",
                                    seed=1, resamples=200, confidence=0.9, droid_reasoning_excluded=False)
        self.assertEqual(result["n"], 1)
        self.assertEqual(result["excluded"], {"missing_arm": 1, "not_both_pass": 1, "incomplete": 1,
                                              "mode_mismatch": 1, "missing_or_zero_metric": 1})
        # The one eligible task has equal uncached input, so the ratio is exactly 1.
        self.assertAlmostEqual(result["geometric_mean"], 1.0)

    def test_no_eligible_task_reports_no_estimate(self):
        index = gap_study.index_rows([row("davinci", "a"), row("droid-direct", "a", passed=False)])
        result = gap_study.estimate(index, "droid-direct", "davinci", "requests", 1, 100, 0.9, False)
        self.assertEqual((result["n"], result["geometric_mean"], result["interval"]), (0, None, None))
        self.assertEqual(result["label"], "NO_CLEAR_GAP")

    def test_a_second_repetition_is_rejected(self):
        with self.assertRaisesRegex(ValueError, "one repetition"):
            gap_study.index_rows([row("davinci", "a"), row("davinci", "a", rep=1)])


class PassGuardTests(unittest.TestCase):
    def test_counts_discordant_tasks_and_the_three_task_same_cause_rule(self):
        rows = []
        for task in ("a", "b", "c", "d"):
            rows.append(row("davinci", task, passed=(task == "d"), stop_reason="x"))
            rows.append(row("droid-script", task))
            rows.append(row("droid-direct", task))
        index = gap_study.index_rows(rows)
        causes = {"a|davinci": "stopped early", "b|davinci": "stopped early", "c|davinci": "claimed done unverified"}
        guard = gap_study.pass_guard(index, causes)
        self.assertEqual(guard["pass_counts"]["davinci"], {"passes": 1, "runs": 4})
        self.assertEqual(guard["pass_counts"]["droid-script"], {"passes": 4, "runs": 4})
        tasks = {(item["pair"], item["task"]) for item in guard["discordant"]}
        self.assertIn(("droid-script vs davinci", "a"), tasks)
        self.assertNotIn(("droid-script vs davinci", "d"), tasks)
        self.assertEqual(guard["discordant"][0]["transcript"], "davinci/a-r0.stdout.jsonl")
        # Two tasks share "stopped early": fewer than three, so no success gap.
        self.assertFalse(guard["success_gap"])
        causes["c|davinci"] = "stopped early"
        guard = gap_study.pass_guard(index, causes)
        self.assertTrue(guard["success_gap"])
        self.assertEqual(guard["success_gap_causes"], {"stopped early": ["a", "b", "c"]})
        # Without human-read causes nothing qualifies.
        self.assertFalse(gap_study.pass_guard(index, None)["success_gap"])


class CliTests(unittest.TestCase):
    def test_cli_requires_a_seed_and_reads_only_results_jsonl(self):
        with tempfile.TemporaryDirectory() as tmp:
            rows = [row(arm, task, requests=r) for task in ("a", "b") for arm, r in
                    (("davinci", 10), ("droid-direct", 5), ("droid-script", 4))]
            Path(tmp, "results.jsonl").write_text("\n".join(json.dumps(r) for r in rows) + "\n")
            Path(tmp, "warmup.jsonl").write_text(json.dumps(row("davinci", "a", requests=999, scored=False)) + "\n")
            with self.assertRaises(SystemExit):
                gap_study.main([tmp])
            out = StringIO()
            with redirect_stdout(out):
                self.assertEqual(gap_study.main([tmp, "--seed", "5", "--resamples", "200", "--json"]), 0)
            report = json.loads(out.getvalue())
            self.assertEqual(len(report["estimates"]), 15)
            requests = [e for e in report["estimates"] if e["metric"] == "requests"]
            self.assertTrue(all(e["n"] == 2 for e in requests))
            first = requests[0]
            self.assertAlmostEqual(first["geometric_mean"], 0.5)
            self.assertEqual(first["label"], "GAP")
            table = StringIO()
            with redirect_stdout(table):
                gap_study.main([tmp, "--seed", "5", "--resamples", "200"])
            self.assertIn("droid-script vs droid-direct", table.getvalue())
            self.assertTrue(math.isfinite(first["interval"][0]))


if __name__ == "__main__":
    unittest.main()


def test_recorded_promotion_decision_matches_the_gate() -> None:
    """The checked-in Stage 1 decision must be the gate's own output."""
    from parity_report import PromotionDecision, rollout_gate

    root = Path(__file__).resolve().parents[3]
    record = json.loads(
        (root / "docs/perf/droid-gap/stage1-promotion.json").read_text(encoding="utf-8")
    )
    decision = PromotionDecision(
        behavior_parity=record["behaviorParity"],
        security_pass=record["securityPass"],
        success_noninferior=record["successNoninferior"],
        resource_benefit=record["resourceBenefit"],
        unknowns=tuple(record["unknowns"]),
    )
    mode, blockers = rollout_gate(decision)
    assert record["decision"] == mode == "off"
    for unknown in record["unknowns"]:
        assert unknown in blockers, unknown
    assert any("behavior parity" in blocker for blocker in blockers)
    assert any("resource benefit" in blocker for blocker in blockers)

    record_doc = (root / "docs/perf/droid-gap/stage1-promotion.md").read_text(
        encoding="utf-8"
    )
    addendum = (root / "docs/perf/droid-gap/stage1-addendum.md").read_text(
        encoding="utf-8"
    )
    assert "off" in record_doc
    assert "rollback" in record_doc.lower()
    assert "No new performance measurement was taken" in addendum
    assert "unchanged" in addendum
