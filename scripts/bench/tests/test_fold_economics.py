"""Offline checks for the fold economics report. No model calls."""
import contextlib
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import fold_economics as economics  # noqa: E402


def attempt(purpose, input_total, cache_read=0, output=5, duration_ms=100.4, usage=True,
            status="completed", parent=None):
    return {"type": "provider_observation",
            "observation": {"kind": "attempt_end", "purpose": purpose, "duration_ms": duration_ms,
                            "status": status, "parent_actor_id": parent,
                            "raw_usage": {"input_total": input_total, "cache_read": cache_read,
                                          "output": output} if usage else None}}


def turn():
    return {"type": "turn_start"}


def stats(hybrid=None, compactions=0):
    vm = {"mode": "hybrid", "folds": 3}
    if hybrid is not None:
        vm["hybridCompaction"] = hybrid
    return {"type": "harness_stats", "contextVm": vm, "runtime": {"compactions": compactions}}


class FoldEconomicsTests(unittest.TestCase):
    def analyze(self, events, tail=""):
        with tempfile.TemporaryDirectory() as temporary:
            path = Path(temporary) / "run.stdout.jsonl"
            path.write_text("".join(json.dumps(event) + "\n" for event in events) + tail, encoding="utf-8")
            return economics.analyze(path)

    def test_a_compaction_that_shrinks_the_prompt_pays_back_over_later_requests(self):
        run = self.analyze([
            turn(), attempt("coding", 20000, 15000),
            turn(), attempt("compaction", 3000, output=400, duration_ms=2500.6),
            attempt("coding", 9000, 2000), attempt("coding", 9500, 8000), attempt("coding", 9800, 9000)])
        (row,) = run["rows"]
        self.assertEqual((row["turn"], row["summarizer_input"], row["summarizer_output"], row["summarizer_ms"]),
                         (2, 3000, 400, 2501))
        self.assertEqual((row["before_input"], row["before_cache_read"]), (20000, 15000))
        self.assertEqual((row["after_input"], row["after_cache_read"]), (9000, 2000))
        self.assertEqual((row["reduction"], row["subsequent_coding_requests"]), (11000, 3))
        self.assertEqual((row["est_saved"], row["net"]), (33000, 30000))
        summary = run["summary"]
        self.assertEqual(summary["compactions_with_positive_reduction"], 1)
        self.assertEqual((summary["est_saved"], summary["net"]), (33000, 30000))
        self.assertAlmostEqual(summary["cached_share_before_first"], 0.75)
        self.assertAlmostEqual(summary["cached_share_after_first"], 19000 / 28300)

    def test_a_compaction_that_leaves_the_prompt_growing_is_a_loss(self):
        run = self.analyze([
            attempt("coding", 14843), attempt("compaction", 14000, output=900),
            attempt("coding", 15500), attempt("compaction", 14500), attempt("coding", 16000)])
        first, second = run["rows"]
        self.assertEqual((first["reduction"], first["est_saved"], first["net"]), (-657, 0, -14000))
        self.assertEqual(first["subsequent_coding_requests"], 1)
        self.assertEqual((second["reduction"], second["net"]), (-500, -14500))
        summary = run["summary"]
        self.assertEqual(summary["compaction_requests"], 2)
        self.assertEqual(summary["compactions_with_positive_reduction"], 0)
        self.assertEqual((summary["summarizer_input"], summary["summarizer_output"]), (28500, 905))
        self.assertEqual(summary["net"], -28500)

    def test_requests_until_the_next_compaction_are_counted_for_each_one(self):
        run = self.analyze([
            attempt("coding", 10000), attempt("compaction", 100), attempt("coding", 6000),
            attempt("coding", 6100), attempt("compaction", 100), attempt("coding", 3000)])
        first, second = run["rows"]
        self.assertEqual((first["subsequent_coding_requests"], first["est_saved"]), (2, 8000))
        self.assertEqual((second["subsequent_coding_requests"], second["reduction"]), (1, 3100))

    def test_missing_usage_is_unknown_never_zero(self):
        run = self.analyze([
            attempt("coding", 10000), attempt("compaction", 0, usage=False),
            attempt("coding", 0, usage=False), attempt("coding", 7000)])
        (row,) = run["rows"]
        self.assertIsNone(row["summarizer_input"])
        self.assertIsNone(row["summarizer_output"])
        self.assertIsNone(row["after_input"])
        self.assertIsNone(row["reduction"])
        self.assertIsNone(row["est_saved"])
        self.assertIsNone(row["net"])
        self.assertEqual(row["subsequent_coding_requests"], 2)
        summary = run["summary"]
        self.assertIsNone(summary["summarizer_input"])
        self.assertIsNone(summary["est_saved"])
        self.assertEqual(summary["summarizer_usage_unknown"], 1)
        self.assertIn("unknown", economics.markdown(run))

    def test_a_compaction_with_no_coding_request_around_it_has_unknown_sizes(self):
        run = self.analyze([attempt("compaction", 500)])
        (row,) = run["rows"]
        self.assertIsNone(row["before_input"])
        self.assertIsNone(row["after_input"])
        self.assertIsNone(row["reduction"])
        self.assertEqual(row["subsequent_coding_requests"], 0)
        self.assertIsNone(run["summary"]["cached_share_after_first"])

    def test_a_run_without_compactions_reports_none(self):
        run = self.analyze([attempt("coding", 100, 50), attempt("coding", 120, 100)])
        self.assertEqual(run["rows"], [])
        summary = run["summary"]
        self.assertEqual((summary["compaction_requests"], summary["coding_requests"]), (0, 2))
        self.assertIsNone(summary["net"])
        self.assertIsNone(summary["cached_share_after_first"])
        self.assertIn("No compaction requests.", economics.markdown(run))

    def test_harness_counters_are_reported_when_present_and_absent_otherwise(self):
        counters = {"attempts": 13, "applied": 0, "skippedNoFit": 0, "skippedBackoff": 0,
                    "notAppliedAfterFold": 13, "contextTokensBefore": 0, "contextTokensAfter": 0}
        run = self.analyze([attempt("coding", 100), stats(counters, compactions=0)])
        self.assertEqual(run["reported"]["attempts"], 13)
        self.assertEqual(run["reported"]["notAppliedAfterFold"], 13)
        self.assertEqual(run["reported"]["compactions"], 0)
        self.assertEqual(run["reported"]["mode"], "hybrid")
        self.assertIn("notAppliedAfterFold=13", economics.markdown(run))
        # A log from before the counters existed is missing them, not zero.
        old = self.analyze([attempt("coding", 100), stats()])
        self.assertIsNone(old["reported"])
        self.assertIn("not reported", economics.markdown(old))
        self.assertIsNone(self.analyze([attempt("coding", 100)])["reported"])

    def test_a_truncated_final_line_and_foreign_events_are_ignored(self):
        run = self.analyze([attempt("coding", 100), {"type": "message_update"}, attempt("compaction", 50)],
                           tail='{"type": "provider_observation", "observ')
        self.assertEqual(run["summary"]["compaction_requests"], 1)
        self.assertEqual(run["summary"]["coding_requests"], 1)

    def test_attempt_starts_are_not_requests(self):
        start = {"type": "provider_observation", "observation": {"kind": "attempt_start", "purpose": "compaction"}}
        run = self.analyze([start, attempt("coding", 100)])
        self.assertEqual(run["summary"]["compaction_requests"], 0)

    def test_only_completed_attempts_of_the_root_actor_count(self):
        run = self.analyze([
            attempt("coding", 10000), attempt("compaction", 900, status="failed"),
            attempt("coding", 7000, parent="sub-1"), attempt("compaction", 400, parent="sub-1"),
            attempt("compaction", 1000), attempt("coding", 6000)])
        (row,) = run["rows"]
        self.assertEqual((row["summarizer_input"], row["before_input"], row["after_input"]), (1000, 10000, 6000))
        self.assertEqual(run["summary"]["coding_requests"], 2)
        self.assertEqual(run["summary"]["compaction_requests"], 1)

    def test_a_missing_parent_field_is_the_root_actor(self):
        event = attempt("coding", 50)
        del event["observation"]["parent_actor_id"]
        self.assertEqual(self.analyze([event])["summary"]["coding_requests"], 1)

    def test_consecutive_summarizer_requests_are_one_compaction(self):
        run = self.analyze([
            turn(), attempt("coding", 20000), turn(),
            attempt("compaction", 3000, output=300, duration_ms=1000), attempt("compaction", 2000, output=200,
                                                                              duration_ms=500),
            attempt("compaction", 1000, output=100, duration_ms=250),
            attempt("coding", 8000), attempt("coding", 8100)])
        (row,) = run["rows"]
        self.assertEqual((row["requests"], row["summarizer_input"], row["summarizer_output"], row["summarizer_ms"]),
                         (3, 6000, 600, 1750))
        self.assertEqual((row["reduction"], row["subsequent_coding_requests"], row["est_saved"], row["net"]),
                         (12000, 2, 24000, 18000))
        summary = run["summary"]
        self.assertEqual((summary["compactions"], summary["compaction_requests"]), (1, 3))

    def test_a_refused_request_between_two_compactions_keeps_them_apart(self):
        run = self.analyze([
            turn(), attempt("coding", 20000), turn(), attempt("compaction", 3000),
            attempt("coding", 0, usage=False, status="failed"),
            attempt("compaction", 2500), attempt("coding", 9000)])
        first, second = run["rows"]
        self.assertEqual((first["requests"], first["summarizer_input"]), (1, 3000))
        self.assertEqual((second["requests"], second["summarizer_input"]), (1, 2500))
        # The refused request is not a size to compare with: the first row has no later request.
        self.assertEqual((first["after_input"], second["before_input"]), (None, 20000))
        self.assertEqual((second["after_input"], second["reduction"]), (9000, 11000))
        self.assertEqual(run["summary"]["compactions"], 2)
        self.assertEqual(run["summary"]["coding_requests"], 2)

    def test_a_group_of_only_failed_attempts_measures_nothing_but_still_separates(self):
        run = self.analyze([
            attempt("coding", 9000), attempt("compaction", 0, usage=False, status="failed"),
            attempt("coding", 8000)])
        self.assertEqual(run["rows"], [])
        self.assertEqual(run["summary"]["coding_requests"], 2)

    def test_usage_is_read_from_the_usage_object_when_raw_usage_is_missing(self):
        def with_usage(purpose, usage):
            event = attempt(purpose, 0, usage=False)
            event["observation"]["usage"] = usage
            return event
        run = self.analyze([
            with_usage("coding", {"input": 250, "cacheRead": 10624, "cacheWrite": 0, "output": 85}),
            with_usage("compaction", {"input": 1009, "cacheRead": 0, "cacheWrite": 0, "output": 451}),
            with_usage("coding", {"input": 100, "cacheRead": 900, "cacheWrite": 50, "output": 5}),
            with_usage("coding", {"input": 0, "cacheRead": 0, "cacheWrite": 0, "output": 0,
                                  "inputUnreported": True})])
        (row,) = run["rows"]
        self.assertEqual((row["before_input"], row["before_cache_read"]), (10874, 10624))
        self.assertEqual((row["summarizer_input"], row["summarizer_output"]), (1009, 451))
        self.assertEqual((row["after_input"], row["after_cache_read"]), (1050, 900))
        # An input the provider did not report stays unknown.
        unreported = {"usage": {"input": 0, "cacheRead": 5, "output": 1, "inputUnreported": True}}
        self.assertIsNone(economics.usage_figures(unreported)[0])

    def test_a_group_with_one_unknown_request_has_an_unknown_cost(self):
        run = self.analyze([attempt("coding", 9000), attempt("compaction", 1000),
                            attempt("compaction", 0, usage=False), attempt("coding", 5000)])
        (row,) = run["rows"]
        self.assertIsNone(row["summarizer_input"])
        self.assertIsNone(row["net"])
        self.assertEqual(row["reduction"], 4000)
        self.assertEqual(run["summary"]["summarizer_usage_unknown"], 1)

    def test_odd_json_values_never_crash_or_become_numbers(self):
        weird = [
            {"type": "provider_observation", "observation": ["not", "a", "dict"]},
            {"type": "provider_observation", "observation": {"kind": "attempt_end", "status": "completed",
                                                              "purpose": "coding", "raw_usage": [1, 2]}},
            attempt("compaction", 100, duration_ms=float("nan")),
            attempt("compaction", 100, duration_ms=float("inf")),
            attempt("compaction", 100, duration_ms=-5),
            attempt("compaction", 100, duration_ms=True),
            {"type": "harness_stats", "contextVm": ["hybrid"], "runtime": "x"},
            {"type": "harness_stats", "contextVm": {"hybridCompaction": {"attempts": True, "applied": "2"}},
             "runtime": ["compactions"]},
        ]
        run = self.analyze(weird, tail="[" * 200000 + "]" * 200000 + "\n" + "[1, 2\n")
        self.assertEqual(run["summary"]["coding_requests"], 1)
        self.assertEqual(run["summary"]["compaction_requests"], 4)
        self.assertEqual(run["summary"]["summarizer_ms"], None)
        self.assertEqual(run["reported"]["attempts"], None)
        self.assertEqual(run["reported"]["applied"], None)
        self.assertIsNone(run["reported"]["compactions"])
        json.dumps(run)  # no NaN or inf survives into the output

    def test_the_combined_section_sums_the_reported_counters(self):
        one = {"attempts": 5, "applied": 1, "notAppliedAfterFold": 4}
        two = {"attempts": 3, "applied": 0, "notAppliedAfterFold": 1, "skippedNoFit": 2}
        with tempfile.TemporaryDirectory() as temporary:
            paths = []
            for name, events in (("a", [attempt("coding", 10), stats(one, compactions=1)]),
                                 ("b", [attempt("coding", 10), stats(two)]),
                                 ("c", [attempt("coding", 10), stats()])):
                path = Path(temporary) / f"{name}.stdout.jsonl"
                path.write_text("".join(json.dumps(event) + "\n" for event in events), encoding="utf-8")
                paths.append(path)
            runs = [economics.analyze(path) for path in paths]
        combined = economics.combine(runs)["reported"]
        self.assertEqual(combined["runs_reporting"], 2)
        self.assertEqual((combined["attempts"], combined["applied"], combined["notAppliedAfterFold"]), (8, 1, 5))
        # skippedNoFit is missing from one reporting run: unknown, not the other run's value.
        self.assertIsNone(combined["skippedNoFit"])
        self.assertIn("runs_reporting=2", economics.markdown(economics.combine(runs)))
        self.assertIsNone(economics.combine([runs[2]])["reported"])

    def test_main_prints_markdown_and_json_and_combines_runs(self):
        with tempfile.TemporaryDirectory() as temporary:
            paths = []
            for name, events in (("a", [attempt("coding", 9000), attempt("compaction", 1000),
                                        attempt("coding", 5000)]),
                                 ("b", [attempt("coding", 4000), attempt("compaction", 500),
                                        attempt("coding", 4500)])):
                path = Path(temporary) / f"{name}.stdout.jsonl"
                path.write_text("".join(json.dumps(event) + "\n" for event in events), encoding="utf-8")
                paths.append(str(path))
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                economics.main(paths)
            text = output.getvalue()
            self.assertIn("| # | turn |", text)
            self.assertIn("### all runs", text)
            self.assertEqual(text.count("| # | turn |"), 2)  # the combined section has no table
            output = io.StringIO()
            with contextlib.redirect_stdout(output):
                economics.main(paths + ["--json"])
            runs = json.loads(output.getvalue())
            self.assertEqual([run["run"] == "all runs" for run in runs], [False, False, True])
            total = runs[-1]["summary"]
            self.assertEqual((total["compaction_requests"], total["summarizer_input"]), (2, 1500))
            self.assertEqual(total["net"], 4000 - 1000 + 0 - 500)


if __name__ == "__main__":
    unittest.main()
