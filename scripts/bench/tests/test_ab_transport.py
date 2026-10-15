"""The HTTP versus WebSocket A/B is prepared, balanced, and judged by a rule fixed in advance. No model runs here."""
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import ab_transport as ab
import runner

bs = chr(92)


def labels(arm, continuation=True):
    if arm == "http":
        return {"http": 14}
    if arm == "ws":
        return {"websocket": 14}
    return {"websocket": 4, "websocket-continuation": 10} if continuation else {"websocket": 14}


def row(arm, unc=100_000, ratio=0.70, requests=14, wall=150.0, pass_=True, first=8704, **extra):
    out = {"input_tokens": unc + 100_000, "cached_tokens": 100_000, "requests": requests, "wall_s": wall,
           "pass": pass_, "cache_write_tokens": 0, "later_request_input_tokens": 100_000,
           "later_request_cached_tokens": int(100_000 * ratio), "first_request_cached_tokens": first,
           "effective_settings": {"transport": ab.TRANSPORT[arm]}, "provider_transports": labels(arm)}
    out.update(extra)
    return out


def rows(ws=None, wsc=None, http=None, mutate=None):
    specs = {"http": http or {}, "ws": ws or {}, "wsc": wsc or {}}
    out = {run["seq"]: row(run["arm"], **specs[run["arm"]]) for run in ab.schedule() if run["role"] == "measured"}
    if mutate:
        mutate(out)
    return out


GOOD = dict(unc=60_000, ratio=0.85)


class ScheduleTests(unittest.TestCase):
    def test_shape_is_three_warmups_then_twelve_measured(self):
        runs = ab.schedule()
        self.assertEqual(runs, ab.schedule())
        self.assertEqual([r["role"] for r in runs], ["warmup"] * 3 + ["measured"] * 12)
        self.assertEqual([r["seq"] for r in runs], list(range(1, 16)))
        self.assertEqual([r["arm"] for r in runs[:3]], ["ws", "wsc", "http"], "a WebSocket failure must stop the run first")
        self.assertEqual({r["task"] for r in runs if r["role"] == "measured"}, set(ab.TASKS))

    def test_every_task_sees_every_arm_once(self):
        by_task = {}
        for r in ab.schedule():
            if r["role"] == "measured":
                by_task.setdefault(r["task"], []).append(r["arm"])
        self.assertEqual(len(by_task), 4)
        for arms in by_task.values():
            self.assertEqual(sorted(arms), sorted(ab.ARMS))

    def test_no_arm_always_runs_first_or_last(self):
        firsts = [order[0] for order in ab.ORDERS]
        lasts = [order[-1] for order in ab.ORDERS]
        self.assertGreaterEqual(len(set(firsts)), 3)
        self.assertGreaterEqual(len(set(lasts)), 3)

    def test_commands_are_fifteen_single_davinci_runs(self):
        lines = ab.commands(r"C:\bench\ws", r"C:\settings", "large_manifest_v2.json")
        self.assertEqual(len(lines), 15)
        self.assertEqual(len({line.split("--variant ")[1].split()[0] for line in lines}), 15)
        for line in lines:
            self.assertIn("--harness davinci", line)
            self.assertNotIn("codex", line)
            self.assertIn("--reps 1", line)
        for line in lines:
            variant = line.split("--variant ")[1].split()[0]
            self.assertTrue(line.endswith(bs + "%s.json --grading-timeout 120" % variant.rsplit("-", 1)[1]))

    def test_arm_settings_differ_only_in_transport(self):
        base = runner.BASE_SETTINGS
        http, ws, wsc = (ab.settings_for(arm, base) for arm in ab.ARMS)
        self.assertEqual(http, base, "the control is the unchanged production baseline")
        for arm_settings in (ws, wsc):
            self.assertEqual({k for k in arm_settings if arm_settings[k] != base.get(k)}, {"transport"})
        self.assertEqual((ws["transport"], wsc["transport"]), ("public-websocket", "public-websocket-continuation"))
        self.assertIsNot(http, base, "settings are copies; the shared base is never mutated")
        self.assertEqual(base["transport"], "auto")

    def test_load_rows_reads_campaign_directories(self):
        with tempfile.TemporaryDirectory() as temporary:
            directory = Path(temporary) / "ws-03-measured-http"
            directory.mkdir()
            (directory / "results.jsonl").write_text(json.dumps(row("http")) + "\n", encoding="utf-8")
            self.assertEqual(list(ab.load_rows(temporary)), [3])


class DecisionRuleTests(unittest.TestCase):
    def test_a_transport_gain_is_promising_and_attributed_to_the_transport(self):
        result = ab.evaluate(rows(ws=GOOD, wsc=GOOD))
        self.assertEqual(result["verdict"], "PROMISING")
        self.assertEqual(result["ws_vs_http"]["verdict"], "PROMISING")
        self.assertEqual(result["wsc_vs_ws"]["verdict"], "NOT_PROMISING")
        self.assertEqual(result["attribution"], "persistent transport")
        self.assertIn("direction only", result["note"])

    def test_a_continuation_only_gain_is_attributed_to_continuation(self):
        result = ab.evaluate(rows(wsc=GOOD))
        self.assertEqual(result["verdict"], "PROMISING")
        self.assertEqual(result["ws_vs_http"]["verdict"], "NOT_PROMISING")
        self.assertEqual(result["wsc_vs_ws"]["verdict"], "PROMISING")
        self.assertEqual(result["attribution"], "continuation (given a persistent transport)")

    def test_both_effects_are_named_when_both_contribute(self):
        result = ab.evaluate(rows(ws=dict(unc=80_000, ratio=0.82), wsc=dict(unc=40_000, ratio=0.95)))
        self.assertEqual(result["wsc_vs_ws"]["verdict"], "PROMISING")
        self.assertEqual(result["attribution"], "persistent transport and continuation each contribute")

    def test_no_change_is_not_promising(self):
        result = ab.evaluate(rows())
        self.assertEqual(result["verdict"], "NOT_PROMISING")
        self.assertEqual(result["attribution"], "none")

    def test_worse_caching_is_not_promising(self):
        self.assertEqual(ab.evaluate(rows(ws=dict(unc=130_000, ratio=0.55), wsc=dict(unc=130_000, ratio=0.55)))["verdict"],
                         "NOT_PROMISING")

    def test_a_small_gain_is_mixed(self):
        self.assertEqual(ab.evaluate(rows(ws=dict(unc=90_000, ratio=0.74), wsc=dict(unc=90_000, ratio=0.74)))["verdict"], "MIXED")

    def test_a_success_regression_blocks_promising(self):
        result = ab.evaluate(rows(ws=dict(GOOD, pass_=False), wsc=dict(GOOD, pass_=False)))
        self.assertNotEqual(result["verdict"], "PROMISING")

    def test_request_growth_blocks_promising(self):
        result = ab.evaluate(rows(ws=dict(GOOD, requests=20), wsc=dict(GOOD, requests=20)))
        self.assertNotEqual(result["verdict"], "PROMISING")

    def test_a_slower_arm_blocks_promising(self):
        result = ab.evaluate(rows(ws=dict(GOOD, wall=400.0), wsc=dict(GOOD, wall=400.0)))
        self.assertNotEqual(result["verdict"], "PROMISING")

    def test_lower_uncached_without_a_later_gain_is_not_promising(self):
        result = ab.evaluate(rows(ws=dict(unc=60_000, ratio=0.70), wsc=dict(unc=60_000, ratio=0.70)))
        self.assertNotEqual(result["verdict"], "PROMISING")

    def test_the_first_request_cost_is_reported(self):
        result = ab.evaluate(rows(ws=dict(GOOD, first=0), wsc=dict(GOOD, first=0)))
        self.assertEqual(result["ws_vs_http"]["median_first_request_cached_lost"], 8704)


class ValidityTests(unittest.TestCase):
    def test_a_websocket_arm_that_fell_back_to_http_is_invalid(self):
        def mutate(out):
            seq = next(s for s, r in out.items() if r["effective_settings"]["transport"] == "public-websocket")
            out[seq]["provider_transports"] = {"http": 1, "websocket": 13}
        result = ab.evaluate(rows(mutate=mutate))
        self.assertEqual(result["verdict"], "INVALID")
        self.assertIn("fallback", result["errors"][0])

    def test_the_http_arm_must_be_pure_http(self):
        def mutate(out):
            seq = next(s for s, r in out.items() if r["effective_settings"]["transport"] == "auto")
            out[seq]["provider_transports"] = {"http": 13, "websocket": 1}
        self.assertEqual(ab.evaluate(rows(mutate=mutate))["verdict"], "INVALID")

    def test_the_persistent_arm_must_never_send_a_continuation(self):
        def mutate(out):
            seq = next(s for s, r in out.items() if r["effective_settings"]["transport"] == "public-websocket")
            out[seq]["provider_transports"] = {"websocket": 4, "websocket-continuation": 10}
        self.assertEqual(ab.evaluate(rows(mutate=mutate))["verdict"], "INVALID")

    def test_the_continuation_arm_must_actually_continue(self):
        def mutate(out):
            seq = next(s for s, r in out.items() if r["effective_settings"]["transport"] == "public-websocket-continuation")
            out[seq]["provider_transports"] = labels("wsc", continuation=False)
        result = ab.evaluate(rows(mutate=mutate))
        self.assertEqual(result["verdict"], "INVALID")
        self.assertIn("never sent a continuation", result["errors"][0])

    def test_an_arm_that_ran_with_the_wrong_transport_setting_is_invalid(self):
        def mutate(out):
            seq = next(iter(out))
            out[seq]["effective_settings"] = {"transport": "sse"}
        self.assertEqual(ab.evaluate(rows(mutate=mutate))["verdict"], "INVALID")

    def test_unrecorded_labels_or_settings_are_invalid(self):
        for key in ("provider_transports", "effective_settings"):
            def mutate(out, key=key):
                out[next(iter(out))].pop(key)
            self.assertEqual(ab.evaluate(rows(mutate=mutate))["verdict"], "INVALID", key)

    def test_a_harness_stop_ends_the_experiment_without_stitching(self):
        def mutate(out):
            out[next(iter(out))]["stop_reason"] = "transport"
        self.assertEqual(ab.evaluate(rows(mutate=mutate))["verdict"], "STOPPED")

    def test_missing_runs_are_incomplete(self):
        partial = rows()
        partial.pop(next(iter(partial)))
        self.assertEqual(ab.evaluate(partial)["verdict"], "INCOMPLETE")

    def test_unknown_usage_is_unscorable_never_zero(self):
        def mutate(out):
            seq = next(iter(out))
            out[seq].update(input_tokens=None, cached_tokens=None, later_request_input_tokens=None,
                            later_request_cached_tokens=None, first_request_cached_tokens=None,
                            known_usage={"input": 500, "cached": 100})
        result = ab.evaluate(rows(mutate=mutate))
        self.assertEqual(result["verdict"], "INCOMPLETE")
        self.assertEqual(len(result["unscorable_runs"]), 1)
        self.assertEqual(next(iter(result["unscorable_runs"].values()))["uncached_lower_bound"], 400)

    def test_stops_outrank_unknown_usage_and_invalid_outranks_both(self):
        def unknown(out):
            out[next(iter(out))].update(input_tokens=None, cached_tokens=None)
        def stopped(out):
            unknown(out)
            out[list(out)[1]]["stop_reason"] = "credentials"
        def invalid(out):
            stopped(out)
            out[list(out)[2]]["provider_transports"] = {"http": 1}
        self.assertEqual(ab.evaluate(rows(mutate=unknown))["verdict"], "INCOMPLETE")
        self.assertEqual(ab.evaluate(rows(mutate=stopped))["verdict"], "STOPPED")
        self.assertEqual(ab.evaluate(rows(mutate=invalid))["verdict"], "INVALID")

    def test_the_registered_thresholds_are_these(self):
        self.assertEqual((ab.MIN_RATIO_GAIN, ab.MIN_TASKS, ab.MAX_REQUEST_GROWTH, ab.MAX_WALL_GROWTH),
                         (0.10, 3, 1.10, 1.10))


if __name__ == "__main__":
    unittest.main()
