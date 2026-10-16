"""The WebSocket confirmation study is prepared, balanced, and judged by a rule fixed in advance. No model runs here."""
import contextlib
import copy
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import ab_ws_confirm as ab
import runner

bs = chr(92)
LABELS = {"http": {"http": 14}, "ws": {"websocket": 14}, "wsc": {"websocket": 4, "websocket-continuation": 10}}
# Per-arm defaults: a study in which both candidates clear every threshold and continuation adds little.
DEFAULTS = {
    "http": dict(unc=100_000, wall=150.0, requests=14, ratio=0.60),
    "ws": dict(unc=50_000, wall=130.0, requests=14, ratio=0.85),
    "wsc": dict(unc=45_000, wall=125.0, requests=14, ratio=0.88),
}
# Continuation clearly better than plain ws: lower uncached input and lower wall time by more than the margins.
STRONG_WSC = dict(unc=30_000, wall=100.0, requests=14, ratio=0.90)
PLAN = {run["seq"]: run for run in ab.schedule()}


def row(run, unc, wall, requests, ratio, pass_=True, first=8704, output=3000, **extra):
    settings = copy.deepcopy(runner.BASE_SETTINGS)
    settings["transport"] = ab.TRANSPORT[run["arm"]]
    jitter = (run["block"] or 0) * 100
    out = {"input_tokens": unc + jitter + 100_000, "cached_tokens": 100_000, "requests": requests, "wall_s": wall,
           "pass": pass_, "cache_write_tokens": 0, "later_request_input_tokens": 100_000,
           "later_request_cached_tokens": int(100_000 * ratio), "first_request_cached_tokens": first,
           "output_tokens": output, "usage_unknown_attempts": 0, "effective_settings": settings,
           "provider_transports": dict(LABELS[run["arm"]]), "binary_sha256": ab.BINARY_SHA256,
           "source_sha": ab.SOURCE_SHA, "model": ab.MODEL, "effort_policy": ab.EFFORT, "service_tier": ab.SERVICE_TIER,
           "fixture_hash": "fixture-" + run["task"]}
    out.update(extra)
    return out


def study(mutate=None, **arms):
    """All 36 measured rows. Keyword arguments override that arm's defaults."""
    specs = {arm: dict(DEFAULTS[arm], **arms.get(arm, {})) for arm in ab.ARMS}
    rows = {s: row(r, **specs[r["arm"]]) for s, r in PLAN.items() if r["role"] == "measured"}
    if mutate:
        mutate(rows)
    return rows


def seqs(arm=None, task=None, rep=None, role="measured"):
    return [s for s, r in PLAN.items() if r["role"] == role and (arm is None or r["arm"] == arm)
            and (task is None or r["task_index"] == task) and (rep is None or r["rep"] == rep)]


class ScheduleTests(unittest.TestCase):
    def test_shape_is_three_warmups_then_thirty_six_measured(self):
        runs = ab.schedule()
        self.assertEqual(runs, ab.schedule())
        self.assertEqual([r["role"] for r in runs], ["warmup"] * 3 + ["measured"] * 36)
        self.assertEqual([r["seq"] for r in runs], list(range(1, 40)))
        self.assertEqual(ab.MAX_RUNS, 39)
        self.assertEqual([r["arm"] for r in runs[:3]], ["ws", "wsc", "http"], "a WebSocket failure must stop the run first")
        self.assertEqual({r["task"] for r in runs[:3]}, {ab.WARMUP_TASK})

    def test_every_task_sees_every_arm_three_times_in_three_repetitions(self):
        seen = {}
        for r in ab.schedule():
            if r["role"] == "measured":
                seen.setdefault((r["task"], r["arm"]), []).append(r["rep"])
        self.assertEqual(len(seen), 12)
        for reps in seen.values():
            self.assertEqual(sorted(reps), [0, 1, 2])

    def test_arm_order_is_counterbalanced_by_position(self):
        position = {}
        for r in ab.schedule():
            if r["role"] == "measured":
                position.setdefault(r["block"], []).append(r["arm"])
        self.assertEqual(len(position), 12)
        counts = {}
        for order in position.values():
            self.assertEqual(sorted(order), sorted(ab.ARMS))
            for index, arm in enumerate(order):
                counts[(arm, index)] = counts.get((arm, index), 0) + 1
        self.assertEqual(set(counts.values()), {4}, "each arm sits in each position in exactly 4 blocks")
        orders = [tuple(o) for o in position.values()]
        self.assertEqual({orders.count(o) for o in set(orders)}, {2}, "each of the six orders is used twice")

    def test_a_task_never_repeats_an_arm_order_and_leads_with_each_arm_once(self):
        by_task = {}
        for r in ab.schedule():
            if r["role"] == "measured":
                by_task.setdefault(r["task_index"], {}).setdefault(r["block"], []).append(r["arm"])
        for blocks in by_task.values():
            orders = [tuple(o) for o in blocks.values()]
            self.assertEqual(len(set(orders)), 3)
            self.assertEqual(sorted(o[0] for o in orders), sorted(ab.ARMS))

    def test_blocks_run_repetition_major_so_a_task_never_runs_back_to_back(self):
        measured = [r for r in ab.schedule() if r["role"] == "measured"]
        blocks = [b for i, b in enumerate(r["block"] for r in measured) if i == 0 or measured[i - 1]["block"] != b]
        tasks = [PLAN[next(s for s, r in PLAN.items() if r["block"] == b)]["task_index"] for b in blocks]
        self.assertEqual(tasks, [0, 1, 2, 3] * 3)

    def test_stages_are_one_signin_each_and_fit_inside_a_token_lifetime(self):
        sizes = {}
        for r in ab.schedule():
            sizes[r["stage"]] = sizes.get(r["stage"], 0) + 1
        self.assertEqual(sizes, {1: 15, 2: 12, 3: 12})
        self.assertEqual(ab.STAGES, 3)
        self.assertTrue(all(r["stage"] == r["rep"] + 1 for r in ab.schedule() if r["role"] == "measured"))

    def test_commands_are_single_davinci_runs_and_stage_filters(self):
        lines = ab.commands(r"C:\bench\wc", r"C:\settings", "large_manifest_v2.json")
        self.assertEqual(len(lines), 39)
        self.assertEqual(len({line.split("--variant ")[1].split()[0] for line in lines}), 39)
        for line in lines:
            self.assertIn("--harness davinci", line)
            self.assertNotIn("codex", line)
            self.assertIn("--reps 1", line)
            variant = line.split("--variant ")[1].split()[0]
            self.assertTrue(line.endswith(bs + "%s.json --grading-timeout 120" % variant.rsplit("-", 1)[1]))
        self.assertEqual([len(ab.commands("r", "s", "m", stage=n)) for n in (1, 2, 3)], [15, 12, 12])
        self.assertIn("--rep-start 2", ab.commands("r", "s", "m", stage=3)[0])

    def test_arm_settings_differ_only_in_transport(self):
        base = runner.BASE_SETTINGS
        settings = {arm: ab.settings_for(arm, base) for arm in ab.ARMS}
        self.assertEqual([settings[a]["transport"] for a in ab.ARMS], ["auto", "public-websocket",
                                                                         "public-websocket-continuation"])
        for arm in ab.ARMS:
            self.assertEqual({k: v for k, v in settings[arm].items() if k != "transport"},
                             {k: v for k, v in base.items() if k != "transport"})

    def test_settings_directory_check(self):
        with tempfile.TemporaryDirectory() as tmp:
            for arm in ab.ARMS:
                Path(tmp, arm + ".json").write_text(json.dumps(ab.settings_for(arm, runner.BASE_SETTINGS)))
            self.assertEqual(ab.settings_errors(tmp, runner.BASE_SETTINGS), [])
            drifted = ab.settings_for("ws", runner.BASE_SETTINGS)
            drifted["promptProfile"] = "other"
            Path(tmp, "ws.json").write_text(json.dumps(drifted))
            self.assertEqual(len(ab.settings_errors(tmp, runner.BASE_SETTINGS)), 1)
            Path(tmp, "wsc.json").unlink()
            self.assertEqual(len(ab.settings_errors(tmp, runner.BASE_SETTINGS)), 2)

    def test_the_cli_prints_the_schedule(self):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            ab.main(["schedule"])
        self.assertEqual(len(out.getvalue().splitlines()), 39)


class PinTests(unittest.TestCase):
    def test_registered_pins_are_the_frozen_values(self):
        self.assertEqual(ab.SOURCE_SHA, "5ab199e96c674780db59724f38c8c4e7f05b2ae4")
        self.assertTrue(ab.BINARY_SHA256.startswith("202f539653f0") and ab.BINARY_SHA256.endswith("cac9449"))
        self.assertTrue(ab.MANIFEST_SHA256.startswith("49e83faeb7b3"))
        self.assertEqual((ab.MODEL, ab.EFFORT), ("gpt-6-luna", "high"))

    def test_pin_check_fails_closed(self):
        with tempfile.TemporaryDirectory() as tmp:
            binary, manifest = Path(tmp, "davinci.exe"), Path(tmp, "manifest.json")
            binary.write_bytes(b"not the frozen binary")
            manifest.write_bytes(b"not the frozen manifest")
            errors = ab.pin_errors(str(binary), str(manifest), "a" * 40, "b" * 40)
            self.assertEqual(len(errors), 3)
            self.assertEqual(len(ab.pin_errors(str(binary) + "x", str(manifest), None, None)), 3)
            self.assertTrue(any("harness" in e for e in ab.pin_errors(None, None, "a" * 40, None)))

    def test_pin_check_passes_only_when_every_hash_matches(self):
        with tempfile.TemporaryDirectory() as tmp:
            binary, manifest = Path(tmp, "b"), Path(tmp, "m")
            binary.write_bytes(b"x")
            manifest.write_bytes(b"y")
            original = (ab.BINARY_SHA256, ab.MANIFEST_SHA256)
            try:
                ab.BINARY_SHA256, ab.MANIFEST_SHA256 = ab.file_sha256(binary), ab.file_sha256(manifest)
                self.assertEqual(ab.pin_errors(str(binary), str(manifest), "c" * 40, "c" * 40), [])
                self.assertEqual(len(ab.pin_errors(str(binary), str(manifest), "c" * 40, "d" * 40)), 1)
            finally:
                ab.BINARY_SHA256, ab.MANIFEST_SHA256 = original


class SuccessfulStudyTests(unittest.TestCase):
    def test_both_candidates_clear_the_rule_and_the_simpler_ws_wins(self):
        result = ab.evaluate(study())
        self.assertEqual(result["evidence"], "complete")
        self.assertEqual((result["arms"]["ws"]["verdict"], result["arms"]["wsc"]["verdict"]), ("PASS", "PASS"))
        self.assertFalse(result["continuation_vs_ws"]["earns_its_keep"])
        self.assertEqual(result["verdict"], "GO_WS")
        self.assertIn("never a default change", result["note"])

    def test_continuation_wins_only_when_it_clears_every_margin_over_ws(self):
        result = ab.evaluate(study(wsc=STRONG_WSC))
        self.assertTrue(result["continuation_vs_ws"]["earns_its_keep"])
        self.assertEqual(result["verdict"], "GO_WSC")

    def test_one_margin_short_keeps_the_simpler_arm(self):
        for key, value in (("unc", 41_000), ("wall", 118.0)):
            result = ab.evaluate(study(wsc=dict(STRONG_WSC, **{key: value})))
            self.assertFalse(result["continuation_vs_ws"]["earns_its_keep"], key)
            self.assertEqual(result["verdict"], "GO_WS", key)

    def test_continuation_must_win_on_three_of_four_tasks(self):
        def worse_on_two_tasks(rows):
            for task in (0, 1):
                for s in seqs("wsc", task=task):
                    rows[s] = row(PLAN[s], unc=70_000, wall=160.0, requests=14, ratio=0.9)
        result = ab.evaluate(study(wsc=STRONG_WSC, mutate=worse_on_two_tasks))
        self.assertFalse(result["continuation_vs_ws"]["checks"]["tasks_better_on_both"])

    def test_wsc_wins_when_ws_misses_the_wall_time_bar(self):
        result = ab.evaluate(study(ws=dict(wall=145.0), wsc=STRONG_WSC))
        self.assertEqual(result["arms"]["ws"]["failed"], ["wall_total"])
        self.assertEqual(result["verdict"], "GO_WSC")

    def test_ws_wins_when_wsc_fails_a_gate(self):
        result = ab.evaluate(study(wsc=dict(unc=45_000, wall=125.0, requests=20, ratio=0.9)))
        self.assertEqual(result["arms"]["wsc"]["failed"], ["requests"])
        self.assertEqual(result["verdict"], "GO_WS")

    def test_no_arm_clearing_the_rule_is_no_go(self):
        result = ab.evaluate(study(ws=dict(wall=149.0), wsc=dict(wall=149.0, unc=60_000)))
        self.assertEqual(result["verdict"], "NO_GO")
        self.assertIn("wall_total", result["arms"]["ws"]["failed"])

    def test_the_cache_gain_alone_is_not_enough(self):
        result = ab.evaluate(study(ws=dict(wall=150.0), wsc=dict(wall=150.0)))
        self.assertEqual(result["verdict"], "NO_GO")

    def test_each_threshold_is_enforced(self):
        cases = {"uncached_total": dict(unc=62_000), "requests": dict(requests=16),
                 "later_ratio_gain": dict(ratio=0.65), "wall_total": dict(wall=136.0)}
        for failed, override in cases.items():
            result = ab.evaluate(study(ws=override))
            self.assertIn(failed, result["arms"]["ws"]["failed"], failed)
            self.assertEqual(result["arms"]["ws"]["verdict"], "FAIL")

    def test_fewer_total_tokens_is_required_so_output_cannot_hide_the_cost(self):
        def heavy_output(rows):
            for s in seqs("ws"):
                rows[s]["output_tokens"] = 60_000
        result = ab.evaluate(study(mutate=heavy_output))
        self.assertIn("total_tokens_lower", result["arms"]["ws"]["failed"])
        self.assertEqual(result["verdict"], "GO_WSC")

    def test_block_and_task_consistency_is_required(self):
        def two_blocks_regress(rows):
            for rep, task in ((0, 0), (1, 0), (2, 0), (0, 1)):
                for s in seqs("ws", task=task, rep=rep):
                    rows[s] = row(PLAN[s], unc=130_000, wall=130.0, requests=14, ratio=0.85)
        result = ab.evaluate(study(mutate=two_blocks_regress, ws=dict(unc=30_000)))
        ws = result["analysis"]["vs_http"]["ws"]
        self.assertEqual(ws["blocks_uncached_lower"], 8)
        self.assertIn("uncached_blocks_lower", result["arms"]["ws"]["failed"])

    def test_one_slow_task_blocks_adoption_even_when_the_total_is_fast(self):
        def one_task_slow(rows):
            for s in seqs("ws", task=0):
                rows[s]["wall_s"] = 190.0
        result = ab.evaluate(study(mutate=one_task_slow, ws=dict(wall=100.0)))
        self.assertIn("wall_worst_task", result["arms"]["ws"]["failed"])
        self.assertEqual(result["arms"]["ws"]["verdict"], "FAIL")

    def test_effects_are_reported_per_task_and_per_repetition(self):
        result = ab.evaluate(study())
        ws = result["analysis"]["vs_http"]["ws"]
        self.assertEqual([t["task"] for t in ws["per_task"]], list(ab.TASKS))
        self.assertEqual([t["reps"] for t in ws["per_task"]], [3, 3, 3, 3])
        self.assertEqual([r["rep"] for r in ws["per_rep"]], [0, 1, 2])
        self.assertEqual(result["analysis"]["blocks_analyzed"], 12)

    def test_the_sign_test_is_informational_and_labelled(self):
        ws = ab.evaluate(study())["analysis"]["vs_http"]["ws"]
        self.assertEqual(ws["blocks_uncached_lower"], 12)
        self.assertAlmostEqual(ws["nominal_block_sign_p"], 1 / 4096)
        self.assertIn("not independent", ws["nominal_block_sign_p_note"])
        self.assertNotIn("nominal_block_sign_p", json.dumps(ab.decide(ab.evaluate(study())["analysis"])["arms"]))

    def test_sign_p_helper(self):
        self.assertAlmostEqual(ab.sign_p(10, 12), 79 / 4096)
        self.assertEqual(ab.sign_p(0, 0), None)


class SuccessGateTests(unittest.TestCase):
    def test_a_candidate_with_fewer_verified_passes_cannot_win(self):
        def lose_one(rows):
            rows[seqs("ws", task=0, rep=0)[0]]["pass"] = False
        result = ab.evaluate(study(mutate=lose_one, wsc=STRONG_WSC))
        self.assertIn("safety_passes_not_below_http", result["arms"]["ws"]["failed"])
        self.assertEqual(result["verdict"], "GO_WSC")

    def test_equal_passes_with_failures_on_both_sides_is_allowed(self):
        def both_lose_one(rows):
            rows[seqs("ws", task=0, rep=0)[0]]["pass"] = False
            rows[seqs("http", task=1, rep=0)[0]]["pass"] = False
        result = ab.evaluate(study(mutate=both_lose_one))
        self.assertEqual(result["arms"]["ws"]["verdict"], "PASS")

    def test_a_task_that_loses_two_repetitions_fails_even_if_the_total_is_level(self):
        def shuffle(rows):
            for rep in (0, 1):
                rows[seqs("ws", task=0, rep=rep)[0]]["pass"] = False
            for rep in (0, 1):
                rows[seqs("http", task=1, rep=rep)[0]]["pass"] = False
        result = ab.evaluate(study(mutate=shuffle))
        self.assertTrue(result["analysis"]["vs_http"]["ws"]["passes"][1] >= result["analysis"]["vs_http"]["ws"]["passes"][0])
        self.assertIn("safety_no_task_loses_more_than_one", result["arms"]["ws"]["failed"])

    def test_all_failing_is_no_go_even_with_huge_savings(self):
        def fail_everything(rows):
            for s in seqs("ws") + seqs("wsc"):
                rows[s]["pass"] = False
        result = ab.evaluate(study(mutate=fail_everything))
        self.assertEqual(result["verdict"], "NO_GO")
        for arm in ab.CANDIDATES:
            self.assertIn("safety_passes_not_below_http", result["arms"][arm]["failed"])


class MissingUsageTests(unittest.TestCase):
    def test_unknown_usage_makes_the_study_incomplete_with_no_verdict(self):
        def unknown(rows):
            seq = seqs("ws", task=2, rep=1)[0]
            rows[seq]["input_tokens"] = None
        result = ab.evaluate(study(mutate=unknown))
        self.assertEqual(result["verdict"], "INCOMPLETE")
        self.assertEqual(result["evidence"], "partial_directional")
        self.assertEqual(len(result["unscorable_runs"]), 1)
        self.assertNotIn("arms", result)
        self.assertIn("Partial directional", result["note"])

    def test_the_excluded_block_is_named_and_the_rest_is_directional_only(self):
        seq = seqs("wsc", task=2, rep=1)[0]
        result = ab.evaluate(study(mutate=lambda rows: rows[seq].update(cached_tokens=None)))
        self.assertEqual(result["blocks_excluded_from_directional"], [PLAN[seq]["block"]])
        self.assertEqual(result["directional"]["blocks_analyzed"], 11)
        self.assertNotIn("verdict", result["directional"])
        self.assertNotIn("checks", json.dumps(result["directional"]))

    def test_unknown_usage_is_never_counted_as_zero_even_when_the_rest_is_perfect(self):
        seq = seqs("http", task=0, rep=0)[0]
        result = ab.evaluate(study(mutate=lambda rows: rows[seq].update(input_tokens=None, cached_tokens=None)))
        self.assertEqual(result["verdict"], "INCOMPLETE")
        self.assertEqual(result["unscorable_runs"][seq]["uncached_lower_bound"], None)
        self.assertNotIn("GO", result["verdict"])

    def test_a_labelled_lower_bound_is_shown_for_information_only(self):
        seq = seqs("ws", task=0, rep=0)[0]
        result = ab.evaluate(study(mutate=lambda rows: rows[seq].update(
            input_tokens=None, cached_tokens=None, known_usage={"input": 50_000, "cached": 20_000})))
        entry = result["unscorable_runs"][seq]
        self.assertEqual(entry["uncached_lower_bound"], 30_000)
        self.assertIn("lower bound", entry["label"])

    def test_missing_output_tokens_make_the_run_unscorable_not_zero(self):
        seq = seqs("wsc", task=1, rep=2)[0]
        result = ab.evaluate(study(mutate=lambda rows: rows[seq].update(output_tokens=None)))
        self.assertEqual(result["verdict"], "INCOMPLETE")
        self.assertEqual(list(result["unscorable_runs"]), [seq])

    def test_a_very_favourable_partial_result_still_gets_no_verdict(self):
        keep = set(seqs("http", rep=0) + seqs("ws", rep=0) + seqs("wsc", rep=0))
        rows = {s: r for s, r in study(wsc=STRONG_WSC).items() if s in keep}
        result = ab.evaluate(rows)
        self.assertEqual(result["verdict"], "INCOMPLETE")
        self.assertEqual(result["measured_runs_present"], 12)
        self.assertEqual(result["directional"]["blocks_analyzed"], 4)

    def test_passes_are_still_visible_in_an_incomplete_study(self):
        seq = seqs("ws", task=0, rep=0)[0]
        result = ab.evaluate(study(mutate=lambda rows: (rows[seq].update(input_tokens=None, pass_=False),
                                                       rows[seq].update(**{"pass": False}))))
        self.assertEqual(result["safety_observed"]["passes"]["ws"], 11)
        self.assertEqual(result["safety_observed"]["passes"]["http"], 12)


class IncompleteCampaignTests(unittest.TestCase):
    def test_no_rows_is_incomplete(self):
        result = ab.evaluate({})
        self.assertEqual(result["verdict"], "INCOMPLETE")
        self.assertEqual(result["measured_runs_present"], 0)
        self.assertNotIn("directional", result)

    def test_missing_runs_are_listed_and_never_invented(self):
        rows = study()
        gone = seqs("wsc", task=3, rep=2)[0]
        del rows[gone]
        result = ab.evaluate(rows)
        self.assertEqual(result["missing_runs"], [gone])
        self.assertEqual(result["verdict"], "INCOMPLETE")

    def test_rows_outside_the_schedule_are_invalid(self):
        rows = study()
        rows[40] = copy.deepcopy(rows[4])
        self.assertEqual(ab.evaluate(rows)["verdict"], "INVALID")

    def test_warmup_rows_are_checked_but_never_scored(self):
        rows = study()
        for s in seqs(role="warmup"):
            rows[s] = row(PLAN[s], unc=1, wall=1.0, requests=1, ratio=0.99)
        self.assertEqual(ab.evaluate(rows)["verdict"], "GO_WS")
        rows[1]["binary_sha256"] = "0" * 64
        self.assertEqual(ab.evaluate(rows)["verdict"], "INVALID")


class ValidityTests(unittest.TestCase):
    def test_a_transport_label_mismatch_is_invalid(self):
        cases = {
            "http arm used websocket": ("http", {"websocket": 14}),
            "ws arm fell back to http": ("ws", {"websocket": 12, "http": 2}),
            "ws arm sent a continuation": ("ws", {"websocket": 4, "websocket-continuation": 10}),
            "wsc arm never continued": ("wsc", {"websocket": 14}),
            "labels missing": ("ws", {}),
        }
        for name, (arm, labels) in cases.items():
            rows = study()
            rows[seqs(arm, task=0, rep=0)[0]]["provider_transports"] = labels
            result = ab.evaluate(rows)
            self.assertEqual(result["verdict"], "INVALID", name)

    def test_a_settings_transport_mismatch_is_invalid(self):
        rows = study()
        rows[seqs("ws", task=1, rep=1)[0]]["effective_settings"]["transport"] = "auto"
        self.assertEqual(ab.evaluate(rows)["verdict"], "INVALID")
        rows = study()
        del rows[seqs("wsc", task=1, rep=1)[0]]["effective_settings"]
        self.assertEqual(ab.evaluate(rows)["verdict"], "INVALID")

    def test_any_difference_beyond_transport_is_invalid(self):
        rows = study()
        rows[seqs("wsc", task=0, rep=0)[0]]["effective_settings"]["promptProfile"] = "other"
        result = ab.evaluate(rows)
        self.assertEqual(result["verdict"], "INVALID")
        self.assertTrue(any("beyond transport" in e for e in result["errors"]))

    def test_pinned_identity_fields_are_enforced_on_every_run(self):
        for field, bad in (("binary_sha256", "0" * 64), ("source_sha", "1" * 40), ("model", "gpt-6-other"),
                           ("effort_policy", "low"), ("service_tier", "priority")):
            rows = study()
            rows[seqs("http", task=2, rep=2)[0]][field] = bad
            self.assertEqual(ab.evaluate(rows)["verdict"], "INVALID", field)
            rows = study()
            del rows[seqs("http", task=2, rep=2)[0]][field]
            self.assertEqual(ab.evaluate(rows)["verdict"], "INVALID", field + " missing")

    def test_task_fixtures_must_not_change_between_arms_or_repetitions(self):
        rows = study()
        rows[seqs("wsc", task=0, rep=1)[0]]["fixture_hash"] = "changed"
        result = ab.evaluate(rows)
        self.assertEqual(result["verdict"], "INVALID")
        self.assertTrue(any("fixture" in e for e in result["errors"]))

    def test_a_harness_stop_ends_the_study_with_no_stitching(self):
        rows = study()
        rows[seqs("ws", task=3, rep=2)[0]]["stop_reason"] = "credentials expired"
        result = ab.evaluate(rows)
        self.assertEqual(result["verdict"], "STOPPED")
        self.assertIn("no reruns", result["note"])


class StopConditionTests(unittest.TestCase):
    def finished(self, arm="ws", task=0, rep=0, **extra):
        seq = seqs(arm, task=task, rep=rep)[0]
        spec = dict(DEFAULTS[arm], **{k: v for k, v in extra.items() if k in ("unc", "wall", "requests", "ratio", "pass_")})
        rest = {k: v for k, v in extra.items() if k not in spec}
        return PLAN[seq], row(PLAN[seq], **spec, **rest)

    def test_a_clean_run_continues(self):
        run, r = self.finished()
        self.assertIsNone(ab.stop_reason(run, r))

    def test_each_registered_stop_fires(self):
        cases = {
            "harness stop": dict(stop_reason="refused request"),
            "arm mismatch": dict(provider_transports={"http": 14}),
            "binary drift": dict(binary_sha256="0" * 64),
            "model drift": dict(model="gpt-6-other"),
            "source drift": dict(source_sha="2" * 40),
            "usage incomplete on a WebSocket run": dict(input_tokens=None),
            "two attempts without usage": dict(usage_unknown_attempts=2),
            "failed verification with failed attempt": dict(usage_unknown_attempts=1, pass_=False),
            "output tokens missing": dict(output_tokens=None),
        }
        for name, extra in cases.items():
            run, r = self.finished(**extra)
            self.assertIsNotNone(ab.stop_reason(run, r), name)

    def test_total_unknown_attempts_stop_at_four(self):
        run, r = self.finished("http", usage_unknown_attempts=1)
        self.assertIsNone(ab.stop_reason(run, r, unknown_so_far=2))
        self.assertIn("in total", ab.stop_reason(run, r, unknown_so_far=3))

    def test_the_cumulative_safety_stop_needs_a_lag_of_two_over_complete_blocks(self):
        earlier = {}
        for s, r in PLAN.items():
            if r["role"] == "measured" and r["block"] in (0, 1):
                earlier[s] = row(r, **DEFAULTS[r["arm"]])
        for block in (0, 1):
            earlier[seqs("ws", task=block, rep=0)[0]]["pass"] = False
        last = max(seqs(rep=0))
        run = PLAN[last]
        one = copy.deepcopy(earlier)
        one[seqs("ws", task=1, rep=0)[0]]["pass"] = True
        self.assertIsNone(ab.stop_reason(PLAN[max(one)], one[max(one)], earlier=one, plan=PLAN))
        reason = ab.stop_reason(PLAN[max(earlier)], earlier[max(earlier)], earlier=earlier, plan=PLAN)
        self.assertIn("safety", reason)
        self.assertIn("ws", reason)
        self.assertIsNotNone(run)

    def test_a_lag_in_an_unfinished_block_does_not_count(self):
        earlier = {}
        first = [s for s, r in PLAN.items() if r["block"] == 0]
        for s in first[:2]:
            earlier[s] = row(PLAN[s], **DEFAULTS[PLAN[s]["arm"]])
        for s in earlier:
            if PLAN[s]["arm"] == "ws":
                earlier[s]["pass"] = False
        run = PLAN[first[1]]
        self.assertIsNone(ab.stop_reason(run, earlier[first[1]], earlier={first[0]: earlier[first[0]]}, plan=PLAN))

    def test_the_maximum_is_39_runs(self):
        self.assertEqual(len(ab.schedule()), 39)
        self.assertEqual(ab.MAX_RUNS, 39)


class ReportCliTests(unittest.TestCase):
    def test_report_reads_campaign_rows_from_disk(self):
        with tempfile.TemporaryDirectory() as tmp:
            for seq, r in study().items():
                folder = Path(tmp, ab.run_name(PLAN[seq]))
                folder.mkdir()
                (folder / "results.jsonl").write_text(json.dumps(r) + "\n", encoding="utf-8")
            out = io.StringIO()
            with contextlib.redirect_stdout(out):
                ab.main(["report", "--root", tmp])
            self.assertEqual(json.loads(out.getvalue())["verdict"], "GO_WS")


if __name__ == "__main__":
    unittest.main()
