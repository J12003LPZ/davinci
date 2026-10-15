"""The cache-key A/B is prepared, balanced, and judged by a rule fixed in advance. No model runs here."""
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import ab_cache_key as ab


def row(unc, pass_=True, requests=18, ratio_later=0.7, wall=150.0):
    return {"input_tokens": unc + 100_000, "cached_tokens": 100_000, "requests": requests, "wall_s": wall,
            "pass": pass_, "cache_write_tokens": 0, "later_request_input_tokens": 100_000,
            "later_request_cached_tokens": int(100_000 * ratio_later), "first_request_cached_tokens": 8704}


def rows(session_unc=60_000, shared_unc=100_000, session_ratio=0.85, shared_ratio=0.70,
         session_pass=True, session_requests=18):
    out = {}
    for run in ab.schedule():
        if run["role"] != "measured":
            continue
        session = run["arm"] == "session"
        out[run["seq"]] = row(session_unc if session else shared_unc,
                              pass_=session_pass if session else True,
                              requests=session_requests if session else 18,
                              ratio_later=session_ratio if session else shared_ratio)
    return out


class ScheduleTests(unittest.TestCase):
    def test_shape_and_determinism(self):
        runs = ab.schedule()
        self.assertEqual(runs, ab.schedule())
        self.assertEqual([r["seq"] for r in runs], list(range(1, 21)))
        self.assertEqual([r["role"] for r in runs], ["warmup"] * 4 + ["measured"] * 16)
        self.assertEqual({r["task"] for r in runs if r["role"] == "measured"}, set(ab.TASKS))

    def test_every_pair_has_both_arms_and_order_is_balanced(self):
        measured = [r for r in ab.schedule() if r["role"] == "measured"]
        pairs = {}
        for r in measured:
            pairs.setdefault(r["pair"], []).append(r)
        self.assertEqual(len(pairs), 8)
        firsts = []
        for pair in pairs.values():
            self.assertEqual(sorted(r["arm"] for r in pair), sorted(ab.ARMS))
            self.assertEqual(len({r["task"] for r in pair}), 1)
            firsts.append(pair[0]["arm"])
        self.assertEqual(firsts.count("shared"), 4)
        self.assertEqual(firsts.count("session"), 4)
        for task in ab.TASKS:  # each task sees both arms first, once per repetition
            self.assertEqual(len({p[0]["arm"] for p in pairs.values() if p[0]["task"] == task}), 2)

    def test_warmups_are_discarded_and_alternate(self):
        warmups = [r for r in ab.schedule() if r["role"] == "warmup"]
        self.assertEqual([r["arm"] for r in warmups], ["shared", "session", "shared", "session"])
        with tempfile.TemporaryDirectory() as temporary:
            for seq in (1, 2, 3, 4):
                directory = Path(temporary) / ("ab-%02d-warmup-x" % seq)
                directory.mkdir()
                (directory / "results.jsonl").write_text(json.dumps(row(1)) + "\n", encoding="utf-8")
            self.assertEqual(ab.evaluate(ab.load_rows(temporary))["verdict"], "INCOMPLETE")

    def test_commands_only_describe_single_davinci_runs(self):
        lines = ab.commands(r"C:\bench\ab", r"C:\settings", "large_manifest_v2.json")
        self.assertEqual(len(lines), 20)
        self.assertEqual(len({line.split("--variant ")[1].split()[0] for line in lines}), 20)
        for line in lines:
            self.assertIn("--harness davinci", line)
            self.assertNotIn("codex", line)
            self.assertIn("--reps 1", line)
        self.assertTrue(all(("session.json" in l) == ("-session" in l.split("--variant ")[1].split()[0]) for l in lines))


class DecisionRuleTests(unittest.TestCase):
    def test_clear_improvement_supports_further_testing_only(self):
        result = ab.evaluate(rows())
        self.assertEqual(result["verdict"], "SUPPORTS_FURTHER_TESTING")
        self.assertIn("direction only", result["note"])

    def test_no_change_is_no_benefit(self):
        self.assertEqual(ab.evaluate(rows(session_unc=100_000, session_ratio=0.70))["verdict"], "NO_BENEFIT")

    def test_loss_of_shared_prefix_reuse_is_no_benefit(self):
        self.assertEqual(ab.evaluate(rows(session_unc=130_000, session_ratio=0.55))["verdict"], "NO_BENEFIT")

    def test_small_improvement_is_inconclusive(self):
        self.assertEqual(ab.evaluate(rows(session_unc=90_000, session_ratio=0.74))["verdict"], "INCONCLUSIVE")

    def test_a_success_regression_blocks_support(self):
        result = ab.evaluate(rows(session_pass=False))
        self.assertNotEqual(result["verdict"], "SUPPORTS_FURTHER_TESTING")
        self.assertFalse(result["conditions_met"][3])

    def test_request_growth_blocks_support(self):
        self.assertNotEqual(ab.evaluate(rows(session_requests=25))["verdict"], "SUPPORTS_FURTHER_TESTING")

    def test_missing_runs_are_incomplete_not_guessed(self):
        partial = rows()
        partial.pop(next(iter(partial)))
        self.assertEqual(ab.evaluate(partial)["verdict"], "INCOMPLETE")


def pilot_row(arm, unc=100_000, first_cached=8704, ratio_later=0.70, pass_=True, requests=18, mode="arm", stop=None):
    out = row(unc, pass_=pass_, requests=requests, ratio_later=ratio_later)
    out["first_request_cached_tokens"] = first_cached
    mode = arm if mode == "arm" else mode
    if mode is not None:
        out["effective_settings"] = {"experimentalSessionCacheKey": True} if mode == "session" else {}
    if stop:
        out["stop_reason"] = stop
    return out


def pilot_rows(session=None, shared=None, mutate=None):
    out = {}
    for run in ab.schedule("pilot"):
        if run["role"] != "measured":
            continue
        kwargs = dict((session if run["arm"] == "session" else shared) or {})
        out[run["seq"]] = pilot_row(run["arm"], **kwargs)
    if mutate:
        mutate(out)
    return out


class PilotTests(unittest.TestCase):
    def test_shape_is_two_warmups_and_four_balanced_pairs(self):
        runs = ab.schedule("pilot")
        self.assertEqual(runs, ab.schedule("pilot"))
        self.assertEqual([r["role"] for r in runs], ["warmup"] * 2 + ["measured"] * 8)
        self.assertEqual([r["arm"] for r in runs if r["role"] == "warmup"], ["shared", "session"])
        pairs = {}
        for r in runs:
            if r["role"] == "measured":
                pairs.setdefault(r["pair"], []).append(r)
        self.assertEqual(len(pairs), 4)
        self.assertEqual({p[0]["task"] for p in pairs.values()}, set(ab.TASKS))
        firsts = [p[0]["arm"] for p in pairs.values()]
        self.assertEqual((firsts.count("shared"), firsts.count("session")), (2, 2))
        for pair in pairs.values():
            self.assertEqual(sorted(r["arm"] for r in pair), sorted(ab.ARMS))

    def test_the_full_schedule_is_unchanged_by_the_pilot(self):
        self.assertEqual(len(ab.schedule()), 20)
        self.assertEqual(ab.schedule(), ab.schedule("full"))

    def test_pilot_commands_are_ten_single_davinci_runs(self):
        lines = ab.commands(r"C:\bench\ab", r"C:\settings", "large_manifest_v2.json", profile="pilot")
        self.assertEqual(len(lines), 10)
        self.assertEqual(len({line.split("--variant ")[1].split()[0] for line in lines}), 10)
        self.assertTrue(all("--harness davinci" in l and "--reps 1" in l and "codex" not in l for l in lines))

    def test_a_clear_gain_is_promising_and_reports_the_first_request_cost(self):
        result = ab.evaluate(
            pilot_rows(session=dict(unc=70_000, first_cached=0, ratio_later=0.88),
                       shared=dict(unc=100_000, first_cached=8704, ratio_later=0.70)), "pilot")
        self.assertEqual(result["verdict"], "PROMISING")
        self.assertEqual(result["median_first_request_cached_lost"], 8704)
        self.assertAlmostEqual(result["median_later_ratio_gain"], 0.18, places=2)
        self.assertIn("never supports changing a default", result["note"])

    def test_losing_the_shared_prefix_without_a_later_gain_is_not_promising(self):
        result = ab.evaluate(
            pilot_rows(session=dict(unc=112_000, first_cached=0, ratio_later=0.70),
                       shared=dict(unc=100_000)), "pilot")
        self.assertEqual(result["verdict"], "NOT_PROMISING")
        self.assertGreater(result["net_uncached_change"], 0)

    def test_a_later_gain_that_does_not_pay_for_the_first_request_is_not_promising(self):
        result = ab.evaluate(
            pilot_rows(session=dict(unc=104_000, first_cached=0, ratio_later=0.78),
                       shared=dict(unc=100_000)), "pilot")
        self.assertEqual(result["verdict"], "NOT_PROMISING")

    def test_a_small_gain_is_mixed(self):
        result = ab.evaluate(
            pilot_rows(session=dict(unc=96_000, first_cached=0, ratio_later=0.74),
                       shared=dict(unc=100_000)), "pilot")
        self.assertEqual(result["verdict"], "MIXED")

    def test_a_success_regression_blocks_promising(self):
        result = ab.evaluate(
            pilot_rows(session=dict(unc=70_000, ratio_later=0.88, pass_=False), shared=dict(unc=100_000)), "pilot")
        self.assertNotEqual(result["verdict"], "PROMISING")

    def test_request_growth_blocks_promising(self):
        result = ab.evaluate(
            pilot_rows(session=dict(unc=70_000, ratio_later=0.88, requests=25), shared=dict(unc=100_000)), "pilot")
        self.assertNotEqual(result["verdict"], "PROMISING")

    def test_an_arm_that_ran_with_the_wrong_key_mode_is_invalid(self):
        def wrong(rows_):
            seq = next(s for s, r in rows_.items() if "effective_settings" in r and not r["effective_settings"])
            rows_[seq]["effective_settings"] = {"experimentalSessionCacheKey": True}
        result = ab.evaluate(pilot_rows(mutate=wrong), "pilot")
        self.assertEqual(result["verdict"], "INVALID")
        self.assertIn("cache-key mode", result["errors"][0])

    def test_an_unrecorded_key_mode_is_invalid_for_the_pilot(self):
        result = ab.evaluate(pilot_rows(session=dict(mode=None)), "pilot")
        self.assertEqual(result["verdict"], "INVALID")
        self.assertIn("not recorded", result["errors"][0])

    def test_a_harness_stop_ends_the_pilot_without_stitching(self):
        def stop(rows_):
            rows_[next(iter(rows_))]["stop_reason"] = "provider_error"
        result = ab.evaluate(pilot_rows(mutate=stop), "pilot")
        self.assertEqual(result["verdict"], "STOPPED")

    def test_missing_pilot_runs_are_incomplete(self):
        partial = pilot_rows()
        partial.pop(next(iter(partial)))
        self.assertEqual(ab.evaluate(partial, "pilot")["verdict"], "INCOMPLETE")

    def test_the_full_study_rejects_a_mismatched_mode_too(self):
        bad = rows()
        seq = next(s for s in bad if ab.schedule()[s - 1]["arm"] == "shared")
        bad[seq]["effective_settings"] = {"experimentalSessionCacheKey": True}
        self.assertEqual(ab.evaluate(bad)["verdict"], "INVALID")


if __name__ == "__main__":
    unittest.main()
