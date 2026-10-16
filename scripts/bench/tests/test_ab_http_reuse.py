"""The HTTP connection-reuse A/B is prepared, balanced, and judged by a rule fixed in advance. No model runs here."""
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import ab_http_reuse as ab
import ab_transport
import runner

bs = chr(92)


def labels(arm, reuse=True):
    if arm == "http":
        return {"http": 14}
    return {"http-reuse-open": 1, "http-reuse": 13} if reuse else {"http-reuse-open": 14}


def row(arm, unc=100_000, ratio=0.70, requests=14, wall=150.0, pass_=True, first=8704, **extra):
    out = {"input_tokens": unc + 100_000, "cached_tokens": 100_000, "requests": requests, "wall_s": wall,
           "pass": pass_, "cache_write_tokens": 0, "later_request_input_tokens": 100_000,
           "later_request_cached_tokens": int(100_000 * ratio), "first_request_cached_tokens": first,
           "effective_settings": {"transport": ab.TRANSPORT[arm]}, "provider_transports": labels(arm)}
    out.update(extra)
    return out


def rows(reuse=None, http=None, mutate=None):
    specs = {"http": http or {}, "reuse": reuse or {}}
    out = {run["seq"]: row(run["arm"], **specs[run["arm"]]) for run in ab.schedule() if run["role"] == "measured"}
    if mutate:
        mutate(out)
    return out


GOOD = dict(unc=60_000, ratio=0.85)


class ScheduleTests(unittest.TestCase):
    def test_shape_is_two_warmups_then_eight_measured(self):
        runs = ab.schedule()
        self.assertEqual(runs, ab.schedule())
        self.assertEqual([r["role"] for r in runs], ["warmup"] * 2 + ["measured"] * 8)
        self.assertEqual([r["seq"] for r in runs], list(range(1, 11)))
        self.assertEqual(ab.MAX_RUNS, 10)
        self.assertEqual([r["arm"] for r in runs[:2]], ["reuse", "http"], "a reuse failure must stop the run first")
        self.assertEqual({r["task"] for r in runs if r["role"] == "measured"}, set(ab.TASKS))

    def test_every_task_sees_both_arms_once(self):
        by_task = {}
        for r in ab.schedule():
            if r["role"] == "measured":
                by_task.setdefault(r["task"], []).append(r["arm"])
        self.assertEqual(len(by_task), 4)
        for arms in by_task.values():
            self.assertEqual(sorted(arms), sorted(ab.ARMS))

    def test_each_arm_goes_first_on_two_tasks(self):
        firsts = [order[0] for order in ab.ORDERS]
        self.assertEqual(sorted(firsts), ["http", "http", "reuse", "reuse"])

    def test_commands_are_ten_single_davinci_runs(self):
        lines = ab.commands(r"C:\bench\ru", r"C:\settings", "large_manifest_v2.json")
        self.assertEqual(len(lines), 10)
        self.assertEqual(len({line.split("--variant ")[1].split()[0] for line in lines}), 10)
        for line in lines:
            self.assertIn("--harness davinci", line)
            self.assertNotIn("codex", line)
            self.assertIn("--reps 1", line)
            variant = line.split("--variant ")[1].split()[0]
            self.assertTrue(line.endswith(bs + "%s.json --grading-timeout 120" % variant.rsplit("-", 1)[1]))

    def test_arm_settings_differ_only_in_transport(self):
        base = runner.BASE_SETTINGS
        http, reuse = (ab.settings_for(arm, base) for arm in ab.ARMS)
        self.assertEqual(http["transport"], "auto")
        self.assertEqual(reuse["transport"], "http-reuse")
        self.assertEqual({k: v for k, v in http.items() if k != "transport"},
                         {k: v for k, v in reuse.items() if k != "transport"})
        self.assertEqual({k: v for k, v in base.items() if k != "transport"},
                         {k: v for k, v in http.items() if k != "transport"})

    def test_the_cli_prints_the_schedule(self):
        import contextlib
        import io
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            ab.main(["schedule"])
        self.assertEqual(len(out.getvalue().splitlines()), 10)


class RuleTests(unittest.TestCase):
    def test_a_large_consistent_gain_is_promising(self):
        verdict = ab.evaluate(rows(reuse=GOOD))
        self.assertEqual(verdict["verdict"], "PROMISING")
        self.assertGreater(verdict["reuse_vs_http"]["median_later_ratio_gain"], 0.10)

    def test_no_change_is_not_promising(self):
        self.assertEqual(ab.evaluate(rows())["verdict"], "NOT_PROMISING")

    def test_worse_caching_is_not_promising(self):
        self.assertEqual(ab.evaluate(rows(reuse=dict(unc=120_000, ratio=0.5)))["verdict"], "NOT_PROMISING")

    def test_a_small_gain_is_mixed(self):
        self.assertEqual(ab.evaluate(rows(reuse=dict(unc=90_000, ratio=0.75)))["verdict"], "MIXED")

    def test_a_success_regression_blocks_promising(self):
        def fail_one(out):
            seq = next(s for s, r in out.items() if r["effective_settings"]["transport"] == "http-reuse")
            out[seq]["pass"] = False
        self.assertEqual(ab.evaluate(rows(reuse=GOOD, mutate=fail_one))["verdict"], "MIXED")

    def test_request_growth_blocks_promising(self):
        self.assertEqual(ab.evaluate(rows(reuse=dict(GOOD, requests=16)))["verdict"], "MIXED")

    def test_a_slower_arm_blocks_promising(self):
        self.assertEqual(ab.evaluate(rows(reuse=dict(GOOD, wall=200.0)))["verdict"], "MIXED")

    def test_the_thresholds_are_the_ones_of_the_websocket_experiment(self):
        for name in ("MIN_RATIO_GAIN", "MIN_TASKS", "MAX_REQUEST_GROWTH", "MAX_WALL_GROWTH"):
            self.assertEqual(getattr(ab_transport, name), {"MIN_RATIO_GAIN": 0.10, "MIN_TASKS": 3,
                                                           "MAX_REQUEST_GROWTH": 1.10, "MAX_WALL_GROWTH": 1.10}[name])
        self.assertEqual((ab.MAX_UNKNOWN_ATTEMPTS_RUN, ab.MAX_UNKNOWN_ATTEMPTS_TOTAL), (1, 3))
        self.assertEqual(ab.MIN_REUSE_SHARE, 0.5)


class ValidityTests(unittest.TestCase):
    def test_the_http_arm_must_be_pure_http(self):
        def mix(out):
            seq = next(s for s, r in out.items() if r["effective_settings"]["transport"] == "auto")
            out[seq]["provider_transports"] = {"http": 13, "http-reuse": 1}
        self.assertEqual(ab.evaluate(rows(mutate=mix))["verdict"], "INVALID")

    def test_the_reuse_arm_must_not_use_other_paths(self):
        for bad in ({"http": 14}, {"http-reuse": 10, "websocket": 4}, {"http-reuse": 10, "unknown": 4},
                    {"http-reuse": 10, "conflict": 4}):
            def swap(out, bad=bad):
                seq = next(s for s, r in out.items() if r["effective_settings"]["transport"] == "http-reuse")
                out[seq]["provider_transports"] = bad
            self.assertEqual(ab.evaluate(rows(mutate=swap))["verdict"], "INVALID", bad)

    def test_a_reuse_arm_that_never_expected_a_pooled_connection_is_invalid(self):
        def never(out):
            seq = next(s for s, r in out.items() if r["effective_settings"]["transport"] == "http-reuse")
            out[seq]["provider_transports"] = labels("reuse", reuse=False)
        verdict = ab.evaluate(rows(mutate=never))
        self.assertEqual(verdict["verdict"], "INVALID")
        self.assertIn("pooled connection", verdict["errors"][0])

    def test_the_reuse_share_boundary(self):
        # 14 attempts, so the first is exempt and 13 are judged: 6.5 needed, 7 passes and 6 fails.
        for reused, expected in ((7, "NOT_PROMISING"), (6, "INVALID")):
            def share(out, reused=reused):
                seq = next(s for s, r in out.items() if r["effective_settings"]["transport"] == "http-reuse")
                out[seq]["provider_transports"] = {"http-reuse-open": 14 - reused, "http-reuse": reused}
            self.assertEqual(ab.evaluate(rows(mutate=share))["verdict"], expected, reused)

    def test_an_arm_that_ran_with_the_wrong_transport_setting_is_invalid(self):
        def wrong(out):
            seq = next(s for s, r in out.items() if r["effective_settings"]["transport"] == "http-reuse")
            out[seq]["effective_settings"] = {"transport": "auto"}
        self.assertEqual(ab.evaluate(rows(mutate=wrong))["verdict"], "INVALID")

    def test_unrecorded_labels_or_settings_are_invalid(self):
        def no_labels(out):
            out[next(iter(out))].pop("provider_transports")
        def no_settings(out):
            out[next(iter(out))].pop("effective_settings")
        self.assertEqual(ab.evaluate(rows(mutate=no_labels))["verdict"], "INVALID")
        self.assertEqual(ab.evaluate(rows(mutate=no_settings))["verdict"], "INVALID")

    def test_a_harness_stop_ends_the_experiment_without_stitching(self):
        def stop(out):
            out[next(iter(out))]["stop_reason"] = "credentials"
        verdict = ab.evaluate(rows(mutate=stop))
        self.assertEqual(verdict["verdict"], "STOPPED")
        self.assertIn("no reruns", verdict["note"])

    def test_missing_runs_are_incomplete(self):
        def drop(out):
            out.pop(next(iter(out)))
        self.assertEqual(ab.evaluate(rows(mutate=drop))["verdict"], "INCOMPLETE")

    def test_unknown_usage_is_unscorable_never_zero(self):
        def unknown(out):
            out[next(iter(out))].update(input_tokens=None, cached_tokens=None, later_request_input_tokens=None,
                                        later_request_cached_tokens=None, first_request_cached_tokens=None,
                                        cache_write_tokens=None)
        verdict = ab.evaluate(rows(reuse=GOOD, mutate=unknown))
        self.assertEqual(verdict["verdict"], "INCOMPLETE")
        self.assertEqual(len(verdict["unscorable_runs"]), 1)

    def test_invalid_outranks_stopped_outranks_incomplete(self):
        def both(out):
            first = next(iter(out))
            out[first]["stop_reason"] = "credentials"
            out[first]["effective_settings"] = {"transport": "wrong"}
        self.assertEqual(ab.evaluate(rows(mutate=both))["verdict"], "INVALID")


class StopTests(unittest.TestCase):
    def run_for(self, arm):
        return next(r for r in ab.schedule() if r["arm"] == arm and r["role"] == "measured")

    def test_a_clean_run_continues(self):
        for arm in ab.ARMS:
            self.assertIsNone(ab.stop_reason(self.run_for(arm), row(arm, usage_unknown_attempts=0)))

    def test_a_harness_stop_stops(self):
        self.assertIn("harness stop", ab.stop_reason(self.run_for("reuse"), row("reuse", stop_reason="credentials")))

    def test_a_wrong_path_stops(self):
        self.assertIn("wrong request path", ab.stop_reason(
            self.run_for("reuse"), row("reuse", provider_transports={"http": 14})))
        self.assertIn("http arm used", ab.stop_reason(
            self.run_for("http"), row("http", provider_transports={"http-reuse": 14})))

    def test_one_stall_is_recorded_not_fatal_and_two_in_a_run_stop(self):
        for arm in ab.ARMS:
            self.assertIsNone(ab.stop_reason(self.run_for(arm), row(arm, usage_unknown_attempts=1)))
            self.assertIn("2 attempts", ab.stop_reason(self.run_for(arm), row(arm, usage_unknown_attempts=2)))

    def test_the_fourth_failed_attempt_overall_stops(self):
        self.assertIsNone(ab.stop_reason(self.run_for("reuse"), row("reuse", usage_unknown_attempts=1), unknown_so_far=2))
        self.assertIn("in total", ab.stop_reason(
            self.run_for("reuse"), row("reuse", usage_unknown_attempts=1), unknown_so_far=3))

    def test_missing_usage_without_a_count_stops(self):
        self.assertIn("usage incomplete", ab.stop_reason(
            self.run_for("reuse"), row("reuse", input_tokens=None, cached_tokens=None)))


class DiskTests(unittest.TestCase):
    def test_report_reads_campaign_directories_in_order(self):
        with tempfile.TemporaryDirectory() as root:
            for run in ab.schedule():
                folder = Path(root) / ab.run_name(run)
                folder.mkdir()
                data = row(run["arm"], **(GOOD if run["arm"] == "reuse" else {}))
                (folder / "results.jsonl").write_text(json.dumps(data) + "\n", encoding="utf-8")
            loaded = ab.load_rows(root)
            self.assertEqual(sorted(loaded), list(range(1, 11)))
            self.assertEqual(ab.evaluate({s: r for s, r in loaded.items()})["verdict"], "PROMISING")


if __name__ == "__main__":
    unittest.main()
