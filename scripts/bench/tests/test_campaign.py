"""Campaign contract tests; all fixture content is synthetic and public."""
import importlib.util
from pathlib import Path
import tempfile
import unittest


SPEC = importlib.util.spec_from_file_location(
    "campaign", Path(__file__).resolve().parents[1] / "campaign.py")
campaign = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(campaign)


def row(harness, rep=0):
    return {"schema_version": 2, "campaign": "screen", "variant": "B0",
            "harness": harness, "task": "t1-intervals", "rep": rep,
            "fixture_hash": "frozen", "model": "fixed", "effort_policy": "medium",
            "service_tier": "default", "exit": 0, "grader_pass": True,
            "unrelated": [], "artifact_leak": False, "transaction_leak": False,
            "cleanup_complete": True, "pass": True,
            "grading_isolation": "diagnostic-only",
            "wall_s": 2.125, "binary_sha256": "binary-" + harness,
            "effective_settings": {"fixed": True}}


def feature_row(vm=None, cm=None, tools=None, context_vm="__none__", calls="__none__", rep=0,
                cm_seen=None, events=None):
    """A davinci result row. Settings carry the REQUESTED modes; `context_vm["mode"]`
    and `codemode_mode` (`cm_seen`) carry the OBSERVED ones, as bench.py fills them."""
    settings = {"fixed": True}
    if vm is not None:
        settings["contextVm"] = vm
    if cm is not None:
        settings["codemode"] = {"mode": cm}
    value = dict(row("davinci", rep), effective_settings=settings)
    if tools is not None:
        value["tools"] = tools
    if context_vm != "__none__":
        value["context_vm"] = context_vm
    if calls != "__none__":
        value["codemode_calls"] = calls
    if cm_seen is not None:
        value["codemode_mode"] = cm_seen
        value["codemode_available"] = cm_seen != "off"
    if events is not None:
        value["codemode_tool_events"] = events
    return value


def vm_block(folds, retrieve=0, mode="hybrid", **more):
    block = {"mode": mode, "folds": folds, "compactions": folds,
             "retrieve_context_calls": retrieve, "retrieval_offered": bool(folds)}
    block.update(more)
    return block


class FeatureUseTests(unittest.TestCase):
    def test_old_log_without_the_fields_is_unknown_not_unused(self):
        use = campaign.feature_use([feature_row(vm="hybrid", cm="read-only")])
        self.assertEqual(use["contextVm"]["status"], "unknown")
        self.assertIsNone(use["contextVm"]["configured_but_unused"])
        self.assertEqual(use["contextVm"]["runs_unknown"], 1)
        self.assertEqual(use["codemode"]["status"], "unknown")
        self.assertIsNone(use["codemode"]["configured_but_unused"])
        # Nothing observed: the mode is unknown, not the requested one.
        self.assertEqual(use["contextVm"]["observed_mode"], "unknown")
        self.assertEqual(use["contextVm"]["requested_mode"], "hybrid")
        self.assertEqual(use["contextVm"]["mode_match"]["unknown"], 1)

    def test_old_log_tool_events_still_count_as_use(self):
        use = campaign.feature_use([feature_row(
            vm="hybrid", cm="read-only", tools={"retrieve_context": 2, "codemode": 1})])
        self.assertEqual(use["contextVm"]["status"], "used")
        self.assertEqual(use["codemode"]["status"], "used")

    def test_settings_that_say_off_do_not_hide_a_run_that_reported_hybrid(self):
        rows = [feature_row(vm="off", context_vm=vm_block(3), rep=0),
                feature_row(vm="hybrid", context_vm=vm_block(1), rep=1)]
        use = campaign.feature_use(rows)["contextVm"]
        self.assertEqual(use["observed_mode"], "hybrid")
        self.assertEqual(use["requested_mode"], "mixed")
        self.assertEqual(use["status"], "used")
        self.assertEqual((use["mode_match"]["matched"], use["mode_match"]["mismatched"]), (1, 1))
        self.assertEqual(use["mode_match"]["mismatched_runs"], ["B0/t1-intervals/r0"])
        self.assertEqual(sorted(use["by_mode"]), ["hybrid"])

    def test_requested_on_but_observed_off_is_a_mismatch_and_reported_off(self):
        use = campaign.feature_use([feature_row(
            vm="hybrid", cm="read-only", context_vm=vm_block(0, mode="off"), cm_seen="off", calls=0)])
        for name in ("contextVm", "codemode"):
            self.assertEqual(use[name]["status"], "off")
            self.assertEqual(use[name]["observed_mode"], "off")
            self.assertEqual(use[name]["mode_match"]["mismatched"], 1)

    def test_missing_settings_does_not_invent_an_off_request(self):
        row_ = feature_row(vm="hybrid", context_vm=vm_block(2))
        del row_["effective_settings"]
        use = campaign.feature_use([row_])["contextVm"]
        self.assertEqual(use["requested_mode"], "unknown")
        self.assertEqual(use["observed_mode"], "hybrid")
        self.assertEqual(use["mode_match"], {
            "matched": 0, "mismatched": 0, "unknown": 1, "mismatched_runs": []})
        row_["effective_settings"] = {}
        self.assertEqual(campaign.feature_use([row_])["contextVm"]["mode_match"]["mismatched"], 1)

    def test_images_with_no_folds_or_retrieval_are_still_activity(self):
        use = campaign.feature_use([feature_row(
            vm="active", context_vm=vm_block(0, mode="active", images_compiled=3))])["contextVm"]
        self.assertEqual(use["status"], "used")
        self.assertFalse(use["configured_but_unused"])
        self.assertEqual(use["fold_status"], "no_folds_observed")
        self.assertEqual(use["images_compiled"]["total"], 3)

    def test_a_missing_observation_is_unknown_never_assumed_from_the_request(self):
        use = campaign.feature_use([feature_row(vm="hybrid", cm="read-only", context_vm={"folds": 0, "retrieve_context_calls": 0})])
        self.assertEqual(use["contextVm"]["observed_mode"], "unknown")
        self.assertIsNone(use["contextVm"]["mode_match"]["mismatched"] or None)
        self.assertEqual(use["contextVm"]["mode_match"]["unknown"], 1)
        self.assertEqual(use["codemode"]["observed_mode"], "unknown")

    def test_a_string_codemode_setting_is_off_as_bench_parses_it(self):
        row = feature_row(vm=None)
        row["effective_settings"]["codemode"] = "read-only"
        self.assertEqual(campaign.requested_mode(row, "codemode"), "off")
        self.assertEqual(campaign.requested_codemode({"codemode": "read-only"}), "off")
        self.assertEqual(campaign.requested_codemode({"codemode": {"mode": "read-only"}}), "read-only")

    def test_hybrid_with_zero_folds_does_not_prove_no_vm_activity(self):
        use = campaign.feature_use([feature_row(
            vm="hybrid", context_vm=vm_block(0), tools={})])["contextVm"]
        self.assertEqual((use["observed_mode"], use["runs"], use["runs_with_activity"]),
                         ("hybrid", 1, 0))
        self.assertIsNone(use["configured_but_unused"])
        self.assertEqual(use["status"], "unknown")
        self.assertEqual((use["fold_status"], use["retrieval_status"]),
                         ("no_folds_observed", "no_retrieval_observed"))

    def test_folds_make_it_used(self):
        use = campaign.feature_use([feature_row(vm="hybrid", context_vm=vm_block(2)),
                                    feature_row(vm="hybrid", context_vm=vm_block(0), rep=1)])["contextVm"]
        self.assertEqual((use["runs"], use["runs_with_activity"], use["status"]), (2, 1, "used"))
        self.assertFalse(use["configured_but_unused"])
        self.assertEqual(use["fold_status"], "folds_observed")
        self.assertEqual(use["retrieval_status"], "no_retrieval_observed")

    def test_active_with_a_fold_is_used_and_shadow_without_one_is_unused(self):
        use = campaign.feature_use([feature_row(vm="active", context_vm=vm_block(1, mode="active"))])
        self.assertEqual(use["contextVm"]["status"], "used")
        use = campaign.feature_use([feature_row(vm="shadow", context_vm=vm_block(0, mode="shadow"))])
        self.assertIsNone(use["contextVm"]["configured_but_unused"])

    def test_retrieve_context_call_alone_counts_as_use(self):
        use = campaign.feature_use([feature_row(vm="active", context_vm=vm_block(0, retrieve=1, mode="active"))])
        self.assertEqual(use["contextVm"]["status"], "used")
        self.assertEqual(use["contextVm"]["retrieval_status"], "retrieval_observed")
        self.assertEqual(use["contextVm"]["fold_status"], "no_folds_observed")

    def test_an_applied_hybrid_compaction_counts_as_use(self):
        block = vm_block(0, hybrid_compaction={"attempts": 1, "applied": 1, "skippedNoFit": 0})
        use = campaign.feature_use([feature_row(vm="hybrid", context_vm=block)])["contextVm"]
        self.assertEqual(use["status"], "used")
        self.assertEqual(use["hybrid_applied"], {"total": 1, "runs_unknown": 0})

    def test_images_lookups_and_shadow_are_tracked_apart_and_unknown_when_missing(self):
        block = vm_block(0, mode="shadow", images_compiled=4, retrieval_hits=2, retrieval_misses=1,
                         shadow_comparisons=None)
        use = campaign.feature_use([feature_row(vm="shadow", context_vm=block),
                                    feature_row(vm="shadow", context_vm=vm_block(0, mode="shadow"), rep=1)])["contextVm"]
        self.assertEqual(use["images_compiled"], {"total": 4, "runs_unknown": 1})
        self.assertEqual(use["retrieval_lookups"], {"total": 3, "runs_unknown": 1})
        self.assertEqual(use["shadow_comparisons"], {"total": None, "runs_unknown": 2})
        self.assertEqual(use["hybrid_applied"], {"total": None, "runs_unknown": 2})
        # Compiled images count as activity even without folds or retrieval.
        self.assertIsNone(use["configured_but_unused"])
        self.assertEqual(use["status"], "used")

    def test_every_mode_seen_in_an_arm_is_reported(self):
        rows = [feature_row(vm="hybrid", cm="read-only", context_vm=vm_block(0), calls=0, cm_seen="read-only"),
                feature_row(vm="active", cm="controlled", context_vm=vm_block(2, mode="active"), calls=1, rep=1,
                            cm_seen="controlled"),
                feature_row(vm=None, cm=None, context_vm=vm_block(0, mode="off"), calls=0, rep=2, cm_seen="off")]
        use = campaign.feature_use(rows)
        self.assertEqual(sorted(use["contextVm"]["by_mode"]), ["active", "hybrid", "off"])
        self.assertEqual(use["contextVm"]["observed_mode"], "mixed")
        self.assertIsNone(use["contextVm"]["by_mode"]["hybrid"]["configured_but_unused"])
        self.assertEqual(use["contextVm"]["by_mode"]["active"]["status"], "used")
        self.assertEqual(use["contextVm"]["by_mode"]["off"]["status"], "off")
        self.assertEqual(sorted(use["codemode"]["by_mode"]), ["controlled", "off", "read-only"])
        self.assertTrue(use["codemode"]["by_mode"]["read-only"]["configured_but_unused"])
        self.assertEqual(use["codemode"]["by_mode"]["controlled"]["status"], "used")

    def test_unavailable_codemode_is_observed_off_even_when_mode_is_read_only(self):
        row_ = feature_row(cm="read-only", cm_seen="read-only", calls=0)
        row_["codemode_available"] = False
        use = campaign.feature_use([row_])["codemode"]
        self.assertEqual(use["observed_mode"], "off")
        self.assertEqual(use["mode_match"]["mismatched_runs"], ["B0/t1-intervals/r0"])
        del row_["codemode_available"]
        self.assertEqual(campaign.feature_use([row_])["codemode"]["observed_mode"], "unknown")

    def test_tool_event_overrules_a_reported_zero_and_flags_disagreement(self):
        row_ = feature_row(cm="read-only", cm_seen="read-only", calls=0, events=1)
        use = campaign.feature_use([row_])["codemode"]
        self.assertEqual(use["status"], "used")
        self.assertFalse(use["configured_but_unused"])
        self.assertEqual(use["telemetry_mismatch"], {"count": 1, "run_keys": ["B0/t1-intervals/r0"]})

    def test_observed_off_with_positive_activity_is_used_and_listed(self):
        row_ = feature_row(cm="read-only", cm_seen="off", calls=2, context_vm=vm_block(3, mode="off"))
        use = campaign.feature_use([row_])
        for name in ("contextVm", "codemode"):
            self.assertEqual(use[name]["observed_mode"], "off")
            self.assertEqual(use[name]["status"], "used")
            self.assertEqual(use[name]["off_with_activity"], ["B0/t1-intervals/r0"])

    def test_conflicting_codemode_counts_are_unknown_and_exposed_separately(self):
        row_ = feature_row(cm="read-only", cm_seen="read-only", calls=0, events=1)
        use = campaign.feature_use([row_])["codemode"]
        self.assertEqual(use["calls"], {"total": None, "runs_unknown": 1})
        self.assertEqual(use["calls_reported"], {"total": 0, "runs_unknown": 0})
        self.assertEqual(use["calls_tool_events"], {"total": 1, "runs_unknown": 0})
        self.assertEqual(use["telemetry_mismatch"]["count"], 1)

    def test_reported_calls_above_tool_events_are_not_a_contradiction(self):
        use = campaign.feature_use([feature_row(cm="read-only", cm_seen="read-only", calls=2, events=0)])["codemode"]
        self.assertEqual(use["calls"], {"total": 2, "runs_unknown": 0})
        self.assertEqual(use["telemetry_mismatch"], {"count": 0, "run_keys": []})

    def test_equal_codemode_counts_have_no_mismatch(self):
        use = campaign.feature_use([feature_row(cm="read-only", cm_seen="read-only", calls=3, events=3)])["codemode"]
        self.assertEqual(use["calls"], {"total": 3, "runs_unknown": 0})
        self.assertEqual(use["telemetry_mismatch"]["count"], 0)

    def test_an_applied_hybrid_compaction_alone_counts_as_activity(self):
        block = vm_block(0, mode="hybrid", hybrid_compaction={"applied": 1}, images_compiled=None)
        use = campaign.feature_use([feature_row(vm="hybrid", context_vm=block)])["contextVm"]
        self.assertEqual((use["status"], use["runs_with_activity"]), ("used", 1))

    def test_codemode_on_with_a_reported_zero_is_configured_but_unused(self):
        use = campaign.feature_use([feature_row(cm="read-only", cm_seen="read-only", calls=0)])["codemode"]
        self.assertTrue(use["configured_but_unused"])
        self.assertEqual((use["requested_mode"], use["observed_mode"]), ("read-only", "read-only"))

    def test_codemode_with_unknown_usage_is_unknown_not_unused(self):
        for row_ in (feature_row(cm="read-only", cm_seen="read-only", calls=None, events=0),
                     feature_row(cm="read-only", cm_seen="read-only", calls=None)):
            use = campaign.feature_use([row_])["codemode"]
            self.assertEqual(use["status"], "unknown")
            self.assertIsNone(use["configured_but_unused"])

    def test_codemode_tool_events_prove_use_without_a_harness_record(self):
        use = campaign.feature_use([feature_row(cm="read-only", cm_seen="read-only", calls=None, events=2)])["codemode"]
        self.assertEqual(use["status"], "used")

    def test_codemode_call_is_counted(self):
        use = campaign.feature_use([feature_row(cm="read-only", cm_seen="read-only", calls=3)])["codemode"]
        self.assertEqual((use["runs_with_activity"], use["status"]), (1, "used"))
        self.assertFalse(use["configured_but_unused"])
        self.assertEqual(use["calls"], {"total": 3, "runs_unknown": 0})

    def test_off_is_never_reported_unused(self):
        use = campaign.feature_use([feature_row(context_vm=vm_block(0, mode="off"), cm_seen="off", calls=0)])
        for name in ("contextVm", "codemode"):
            self.assertEqual(use[name]["observed_mode"], "off")
            self.assertEqual(use[name]["status"], "off")
            self.assertFalse(use[name]["configured_but_unused"])


class CampaignTests(unittest.TestCase):
    def test_cluster_uncertainty_is_reproducible_and_does_not_treat_repetitions_as_tasks(self):
        left = [dict(row("davinci", rep), task=task, wall_s=10)
                for task in ("t1-intervals", "t2-duration") for rep in range(4)]
        right = [dict(value, wall_s=5 if value["task"] == "t1-intervals" else 20) for value in left]
        result = campaign.latency_uncertainty(left, right, samples=200)
        self.assertEqual(result, campaign.latency_uncertainty(left, right, samples=200))
        self.assertEqual(result["clusters"], 2)
        self.assertTrue(result["ratio_of_medians"]["includes_no_change"])
        self.assertEqual(result["ratio_of_medians"]["ci95"], [0.5, 2.0])
        self.assertFalse(campaign.latency_uncertainty(left[:4], right[:4], samples=200)["available"])

    def test_campaign_overlap_uses_full_envelope_and_requires_timestamps(self):
        def timed(start, end):
            return dict(row("davinci"), started_at=f"2026-09-27T{start}:00+00:00", finished_at=f"2026-09-27T{end}:00+00:00")
        self.assertFalse(campaign.serial_campaigns([timed("01:00", "01:10"), timed("03:00", "03:10")],
                                                   [timed("02:00", "02:10")]))
        self.assertTrue(campaign.serial_campaigns([timed("01:00", "01:10")], [timed("02:00", "02:10")]))
        self.assertIsNone(campaign.serial_campaigns([row("davinci")], [row("davinci")]))

    def test_promotion_rules_fail_quality_latency_and_missing_telemetry_independently(self):
        left = [dict(row("davinci", rep), task=task, wall_s=100, input_tokens=100, cached_tokens=90, output_tokens=10)
                for task in ("t1-intervals", "m-fixture") for rep in range(2)]
        right = [dict(value, wall_s=95, input_tokens=90, cached_tokens=70) for value in left]
        right[0].update({"pass": False, "grader_pass": False})
        report = campaign.promotion_gates(left, right, {}, {}, campaign.latency_uncertainty(left, right, samples=100),
                                          {"verified": False})
        self.assertFalse(report["accepted"])
        rules = report["rules"]
        for name in ("candidate_passes_at_least_parent", "every_correctness_regression_reviewed",
                     "median_at_least_ten_percent_faster", "untouched_holdout", "independent_grading_boundary",
                     "uncached_input_tokens_no_task_regression"):
            self.assertEqual(rules[name]["status"], "fail", name)
        self.assertEqual(rules["p90_regression_at_most_ten_percent"]["status"], "pass")
        self.assertEqual(rules["logical_request_reduction"]["status"], "unavailable")
        self.assertEqual(rules["two_independent_windows"]["status"], "unavailable")

    def test_promotion_overlap_includes_codex_without_combining_arm_metrics(self):
        def timed(harness, start, end):
            return dict(row(harness), started_at=f"2026-09-27T00:{start}:00+00:00",
                        finished_at=f"2026-09-27T00:{end}:00+00:00")
        parent = [timed("davinci", "00", "01"), timed("codex", "01", "05")]
        candidate = [timed("davinci", "03", "04"), timed("codex", "05", "06")]
        parent[1]["pass"] = False
        self.assertTrue(campaign.serial_campaigns(parent[:1], candidate[:1]))
        report = campaign.promotion_gates(parent[:1], candidate[:1], {}, {}, {"method": "fixture"}, {},
                                          baseline_campaign=parent, candidate_campaign=candidate)
        self.assertEqual(report["rules"]["sequential_campaigns"]["status"], "fail")
        self.assertEqual(report["rules"]["candidate_passes_at_least_parent"]["evidence"],
                         {"parent": 1, "candidate": 1})
        missing = campaign.promotion_gates(parent[:1], candidate[:1], {}, {}, {"method": "fixture"}, {})
        self.assertEqual(missing["rules"]["sequential_campaigns"]["status"], "unavailable")

    def test_paired_report_includes_uncached_input_and_gate_counts(self):
        left = dict(row("davinci"), input_tokens=100, cached_tokens=80,
                    gate_reminders={"verification_required": 2})
        right = dict(row("codex"), input_tokens=80, cached_tokens=70,
                     gate_reminders={})
        paired = campaign.paired_metrics([left], [right])
        self.assertEqual(paired["uncached_input_tokens"]["total_delta"], -10)
        self.assertEqual(paired["gate_reminder_count"]["total_delta"], -2)

    def test_paired_metrics_preserve_missingness_and_pair_membership(self):
        left = [dict(row("davinci", rep), wall_s=wall, logical_requests=4)
                for rep, wall in enumerate([10, 20, 40])]
        right = [dict(row("codex", rep), wall_s=wall, logical_requests=2)
                 for rep, wall in enumerate([5, 10, 20])]
        paired = campaign.paired_metrics(left, right)
        self.assertEqual(paired["pairs"], 3)
        self.assertEqual(paired["wall_s"]["median_ratio"], 0.5)
        self.assertEqual(paired["wall_s"]["total_delta"], -35)
        right[1]["logical_requests"] = None
        paired = campaign.paired_metrics(left, right)
        self.assertIsNone(paired["logical_requests"]["median_ratio"])
        self.assertEqual(paired["logical_requests"]["unavailable_pairs"], 1)
        with self.assertRaises(ValueError):
            campaign.paired_metrics(left, right[:-1])

    def test_per_success_counts_failed_run_cost(self):
        # Two runs, one fails: its cost stays in the numerator, only the pass is the divisor.
        def cost(harness, rep, passed, tokens):
            return dict(row(harness, rep), input_tokens=tokens, cached_tokens=0, output_tokens=tokens // 10,
                        logical_requests=4, provider_attempts=5, wall_s=10, **{"pass": passed})
        left = [cost("davinci", 0, True, 100), cost("davinci", 1, False, 300)]
        right = [cost("codex", 0, True, 50), cost("codex", 1, True, 50)]
        per = campaign.paired_metrics(left, right)["per_verified_success"]
        self.assertEqual(per["baseline"]["passes"], 1)
        self.assertEqual(per["baseline"]["uncached_input_tokens"]["value"], 400)
        self.assertEqual(per["baseline"]["output_tokens"]["value"], 40)
        self.assertEqual(per["baseline"]["logical_requests"]["value"], 8)
        self.assertEqual(per["baseline"]["provider_attempts"]["value"], 10)
        self.assertEqual(per["baseline"]["wall_s"]["value"], 20)
        self.assertEqual(per["candidate"]["uncached_input_tokens"]["value"], 50)

    def test_per_success_unknown_usage_is_none_not_zero(self):
        left = [dict(row("davinci", rep), input_tokens=100, cached_tokens=10) for rep in range(2)]
        right = [dict(row("codex", rep), input_tokens=100, cached_tokens=10) for rep in range(2)]
        del right[1]["cached_tokens"]
        per = campaign.paired_metrics(left, right)["per_verified_success"]
        self.assertEqual(per["baseline"]["uncached_input_tokens"]["value"], 90)
        self.assertIsNone(per["candidate"]["uncached_input_tokens"]["value"])
        self.assertEqual(per["candidate"]["uncached_input_tokens"]["reason"], "unavailable_usage")

    def test_per_success_zero_passes_is_none(self):
        left = [dict(row("davinci"), input_tokens=100, cached_tokens=0, **{"pass": False})]
        right = [dict(row("codex"), input_tokens=100, cached_tokens=0)]
        per = campaign.paired_metrics(left, right)["per_verified_success"]
        self.assertEqual(per["baseline"]["passes"], 0)
        self.assertEqual(per["baseline"]["uncached_input_tokens"],
                         {"value": None, "reason": "no_passes"})

    def test_per_success_does_not_double_count_cached(self):
        left = [dict(row("davinci"), input_tokens=1000, cached_tokens=900)]
        right = [dict(row("codex"), input_tokens=1000, cached_tokens=0)]
        per = campaign.paired_metrics(left, right)["per_verified_success"]
        self.assertEqual(per["baseline"]["uncached_input_tokens"]["value"], 100)
        self.assertEqual(per["candidate"]["uncached_input_tokens"]["value"], 1000)

    def test_manifest_is_authoritative_even_when_rows_agree(self):
        rows = [row("davinci"), row("codex")]
        manifest = {key: rows[0][key] for key in ("schema_version", "campaign", "variant", *campaign.COMPARABLE)}
        manifest.update(tasks=["t1-intervals"], repetitions=[0], harnesses=["davinci", "codex"],
                        grading_isolation={"davinci": "diagnostic-only", "codex": "diagnostic-only"},
                        identities={r["harness"]: {key: r[key] for key in ("binary_sha256", "effective_settings")}
                                    for r in rows})
        self.assertEqual(campaign.manifest_errors(manifest, rows), [])
        self.assertTrue(campaign.manifest_errors(dict(manifest, model="different"), rows))
        manifest["identities"]["davinci"]["binary_sha256"] = "different"
        self.assertTrue(campaign.manifest_errors(manifest, rows))

    def test_malformed_rows_are_rejected_without_crashing(self):
        for malformed in (None, [], dict(row("davinci"), harness=[]),
                          dict(row("davinci"), rep=True),
                          dict(row("davinci"), task={})):
            with self.subTest(row=malformed):
                self.assertTrue(campaign.integrity_errors(
                    [malformed, row("codex")], ["t1-intervals"], [0]))

    def test_complete_pair(self):
        self.assertEqual(campaign.integrity_errors(
            [row("davinci"), row("codex")], ["t1-intervals"], [0]), [])

    def test_duplicate_and_incomplete_pairs_rejected(self):
        errors = campaign.integrity_errors(
            [row("davinci"), row("davinci")], ["t1-intervals"], [0])
        self.assertTrue(any("duplicate" in e for e in errors))
        self.assertTrue(any("missing" in e for e in errors))

    def test_incompatible_configs_rejected(self):
        for field in ("fixture_hash", "model", "effort_policy", "service_tier"):
            with self.subTest(field=field):
                candidate = dict(row("codex"), **{field: "different"})
                self.assertTrue(campaign.integrity_errors(
                    [row("davinci"), candidate], ["t1-intervals"], [0]))

    def test_failed_completion_cannot_pass(self):
        for changes in ({"exit": 1}, {"exit": "timeout"},
                        {"unrelated": ["outside.py"]}, {"artifact_leak": True},
                        {"transaction_leak": True}, {"cleanup_complete": False},
                        {"grader_pass": False}):
            with self.subTest(changes=changes):
                self.assertTrue(campaign.integrity_errors(
                    [dict(row("davinci"), **changes), row("codex")],
                    ["t1-intervals"], [0]))

    def test_failures_and_timeouts_are_valid_rows_when_reported(self):
        failed = dict(row("davinci"), exit="timeout", grader_pass=False, **{"pass": False})
        self.assertEqual(campaign.integrity_errors(
            [failed, row("codex")], ["t1-intervals"], [0]), [])

    def test_balanced_order_and_frozen_manifest(self):
        tasks = ["t1-intervals", "t2-duration"]
        schedule = campaign.schedule(tasks, range(3), ["davinci", "codex"], 42)
        self.assertEqual(schedule, campaign.schedule(tasks, range(3), ["davinci", "codex"], 42))
        starts = [schedule[i][0] for i in range(0, len(schedule), 2)]
        self.assertEqual(starts.count("davinci"), starts.count("codex"))
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            for task in tasks:
                (root / task / "repo").mkdir(parents=True)
                (root / task / "repo" / "public.txt").write_text("fixture", encoding="utf-8")
            manifest = campaign.fixture_manifest(root, tasks)
            self.assertEqual(manifest, campaign.fixture_manifest(root, tasks))
            (root / tasks[0] / "repo" / "public.txt").write_text("changed", encoding="utf-8")
            self.assertNotEqual(manifest["fixture_hash"],
                                campaign.fixture_manifest(root, tasks)["fixture_hash"])


if __name__ == "__main__":
    unittest.main()
