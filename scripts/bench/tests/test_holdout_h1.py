"""The independent holdout study H1 is scheduled, guarded and judged by rules fixed in advance. No model runs here."""
import contextlib
import copy
import hashlib
import io
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import holdout_h1 as h1

TASKS = ["h1-%02d" % i for i in range(1, h1.TASK_COUNT + 1)]
DIGEST = "d" * 64
PINS = {"source_sha": h1.SOURCE_SHA, "davinci_sha256": h1.DAVINCI_SHA256, "codex_sha256": h1.CODEX_SHA256,
        "codex_host_sha256": h1.CODEX_HOST_SHA256, "codex_version": h1.CODEX_VERSION, "model": h1.MODEL,
        "effort": h1.EFFORT, "service_tier": h1.SERVICE_TIER, "private_suite_digest": DIGEST,
        "task_ids": TASKS, "harness_sha": "e" * 40}
UNITS = {(u["arm"], u["rep"]): u for u in h1.schedule()}
CLEAN = {"status": "CLEAN", "files_scanned": 54, "needles": 10, "campaigns_with_hits": {}}
# Per-arm defaults: B clearly cheaper than C, A in between. (uncached, output, wall, requests)
DEFAULTS = {"A": (50_000, 3000, 100.0, 14), "B": (12_000, 3000, 82.0, 14), "C": (40_000, 3000, 100.0, 14)}


def row(arm, task, rep, **extra):
    unc, out, wall, requests = DEFAULTS[arm]
    unit = UNITS[(arm, rep)]
    codex = arm == "C"
    settings = ({"ignore_user_config": True, "effort": h1.EFFORT, "service_tier": h1.SERVICE_TIER,
                 "telemetry": "local-sanitized-otlp-logs-and-traces"} if codex
                else {"transport": h1.ARM_TRANSPORT.get(arm), "tools": "base"})
    result = {"harness": h1.ARM_HARNESS[arm], "task": task, "rep": rep, "pass": True, "exit": 0,
              "input_tokens": unc + 100_000, "cached_tokens": 100_000, "output_tokens": out, "wall_s": wall,
              "requests": requests, "tool_calls": 20, "executed_leaf_operations": None if codex else 20,
              "logical_requests": requests, "provider_attempts": requests, "unrelated": [], "stop_reason": None,
              "usage_unknown_attempts": None if codex else 0, "request_metrics_complete": True,
              "reasoning_included_in_output": True, "model": h1.MODEL, "effort_policy": h1.EFFORT,
              "service_tier": h1.SERVICE_TIER, "private_suite_digest": DIGEST, "split": h1.SPLIT,
              "language": h1.LANGUAGE, "task_set": "private", "variant": unit["name"], "fixture_hash": "fixture",
              "binary_sha256": h1.CODEX_SHA256 if codex else h1.DAVINCI_SHA256,
              "effective_settings": settings, "_unit": unit,
              "started_at": "2026-10-16T10:%02d:%02d+00:00" % (unit["seq"], TASKS.index(task))}
    if codex:
        result.update(version=h1.CODEX_VERSION, source_sha=None, provider_transports=None)
    else:
        result.update(source_sha=h1.SOURCE_SHA,
                      provider_transports={"http": requests} if arm == "A" else {"websocket": requests})
    result.update(extra)
    return result


def study(mutate=None):
    rows = {(arm, task, rep): row(arm, task, rep) for arm in h1.ARMS for task in TASKS for rep in range(h1.REPS)}
    if mutate:
        mutate(rows)
    return rows


def fail_runs(rows, arm, count, per_task=1):
    """Fail `count` runs of one arm, spread over tasks with at most `per_task` per task."""
    done = 0
    for task in TASKS:
        for rep in range(per_task):
            if done < count:
                rows[(arm, task, rep)]["pass"] = False
                done += 1


def verdict(rows, **kw):
    return h1.evaluate(rows, PINS, kw.pop("leak", CLEAN), **kw)


class ScheduleTests(unittest.TestCase):
    def test_nine_campaigns_fifty_four_runs(self):
        units = h1.schedule()
        self.assertEqual(units, h1.schedule())
        self.assertEqual(len(units), 9)
        self.assertEqual(h1.MAX_RUNS, 54)
        self.assertEqual(len({u["name"] for u in units}), 9)
        self.assertEqual([u["seq"] for u in units], list(range(1, 10)))

    def test_every_arm_takes_every_position_once_and_each_rep_has_all_arms(self):
        for rep in range(h1.REPS):
            self.assertEqual(sorted(u["arm"] for u in h1.schedule() if u["rep"] == rep), ["A", "B", "C"])
        for arm in h1.ARMS:
            self.assertEqual(sorted(u["position"] for u in h1.schedule() if u["arm"] == arm), [0, 1, 2])

    def test_stage_is_one_repetition_of_three_campaigns(self):
        for stage in range(1, h1.STAGES + 1):
            self.assertEqual(len([u for u in h1.schedule() if u["stage"] == stage]), 3)

    def test_no_warmups_exist(self):
        self.assertFalse(any("warm" in u["name"] for u in h1.schedule()))
        self.assertEqual(h1.MAX_RUNS, len(TASKS) * h1.REPS * len(h1.ARMS))


class CommandTests(unittest.TestCase):
    def lines(self, stage=None):
        return h1.commands("C:\\r", "C:\\s", "D:\\suite", "C:\\d.exe", "C:\\c.exe", stage)

    def test_every_line_runs_the_whole_split_of_the_private_suite(self):
        for line in self.lines():
            self.assertIn("--private-suite D:\\suite --split holdout --tasks all --reps 1", line)

    def test_davinci_arms_pass_settings_and_codex_does_not(self):
        lines = self.lines()
        self.assertEqual(len(lines), 9)
        for line, unit in zip(lines, h1.schedule()):
            if unit["arm"] == "C":
                self.assertIn("--harness codex", line)
                self.assertIn("BENCH_CODEX", line)
                self.assertNotIn("--settings", line)
            else:
                self.assertIn("--harness davinci", line)
                self.assertIn("--settings C:\\s\\%s.json" % unit["arm"], line)
                self.assertIn("BENCH_DAVINCI", line)

    def test_stage_filter(self):
        self.assertEqual(len(self.lines(2)), 3)
        self.assertTrue(all("-r1" in line for line in self.lines(2)))


class SettingsTests(unittest.TestCase):
    BASE = {"tools": "base", "transport": "ignored"}

    def write(self, directory, mutate=None):
        for arm in h1.DAVINCI_ARMS:
            settings = h1.settings_for(arm, self.BASE)
            if mutate:
                mutate(arm, settings)
            (Path(directory) / ("%s.json" % arm)).write_text(json.dumps(settings), encoding="utf-8")

    def test_arms_differ_only_in_transport(self):
        a, b = h1.settings_for("A", self.BASE), h1.settings_for("B", self.BASE)
        self.assertEqual({k: v for k, v in a.items() if k != "transport"}, {k: v for k, v in b.items() if k != "transport"})
        self.assertEqual((a["transport"], b["transport"]), ("auto", "public-websocket"))

    def test_verify_settings(self):
        with tempfile.TemporaryDirectory() as d:
            self.write(d)
            self.assertEqual(h1.settings_errors(d, self.BASE), [])
            self.write(d, lambda arm, s: s.update(extra=1) if arm == "B" else None)
            self.assertEqual(len(h1.settings_errors(d, self.BASE)), 1)
        with tempfile.TemporaryDirectory() as d:
            self.assertEqual(len(h1.settings_errors(d, self.BASE)), 2)


class PinTests(unittest.TestCase):
    def test_valid_pins(self):
        self.assertEqual(h1.pins_errors(PINS), [])

    def test_each_registered_constant_is_enforced(self):
        for field in ("source_sha", "davinci_sha256", "codex_sha256", "codex_host_sha256", "codex_version", "model",
                      "effort", "service_tier"):
            pins = dict(PINS, **{field: "x"})
            self.assertTrue(any(field in e for e in h1.pins_errors(pins)), field)

    def test_suite_digest_ids_and_harness_commit_are_checked(self):
        self.assertTrue(h1.pins_errors(dict(PINS, private_suite_digest="nothex")))
        self.assertTrue(h1.pins_errors(dict(PINS, task_ids=TASKS[:5])))
        self.assertTrue(h1.pins_errors(dict(PINS, task_ids=TASKS[:5] + [TASKS[0]])))
        self.assertTrue(h1.pins_errors(dict(PINS, task_ids=TASKS[:5] + ["Bad Id"])))
        self.assertTrue(h1.pins_errors(dict(PINS, harness_sha="abc")))
        self.assertTrue(h1.pins_errors(None))

    def test_binary_hashes_and_harness_commit(self):
        with tempfile.TemporaryDirectory() as d:
            files = {}
            for name, key in (("d", "davinci_sha256"), ("c", "codex_sha256"), ("h", "codex_host_sha256")):
                path = Path(d) / name
                path.write_bytes(name.encode())
                files[name] = (str(path), hashlib.sha256(name.encode()).hexdigest(), key)
            pins = dict(PINS, **{key: digest for _, digest, key in files.values()})
            ok = h1.pin_errors(pins, files["d"][0], files["c"][0], files["h"][0], pins["harness_sha"])
            self.assertEqual([e for e in ok if "registered" not in e], [])
            self.assertTrue(any("davinci binary" in e for e in h1.pin_errors(
                dict(pins, davinci_sha256="0" * 64), files["d"][0], files["c"][0], files["h"][0], pins["harness_sha"])))
            self.assertTrue(any("harness commit" in e for e in h1.pin_errors(
                pins, files["d"][0], files["c"][0], files["h"][0], "f" * 40)))
            self.assertTrue(any("unreadable" in e for e in h1.pin_errors(
                pins, str(Path(d) / "missing"), files["c"][0], files["h"][0], pins["harness_sha"])))
            self.assertTrue(any("harness commit" in e for e in h1.pin_errors(
                pins, files["d"][0], files["c"][0], files["h"][0], None)))


class LocationTests(unittest.TestCase):
    def test_overlaps_are_refused_and_a_separate_directory_is_accepted(self):
        with tempfile.TemporaryDirectory() as d:
            base = Path(d)
            suite, repo, results = base / "vault" / "suite", base / "repo", base / "results"
            for p in (suite, repo, results, base / "home" / ".pi"):
                p.mkdir(parents=True)
            forbidden = {"repo": repo, "results": results, "pi": base / "home" / ".pi"}
            self.assertEqual(h1.location_errors(suite, forbidden), [])
            for label, inside in (("repo", repo / "suite"), ("results", results / "suite"), ("pi", base / "home" / ".pi" / "s")):
                inside.mkdir(parents=True, exist_ok=True)
                self.assertEqual(len(h1.location_errors(inside, forbidden)), 1, label)
            self.assertEqual(len(h1.location_errors(base, forbidden)), 3)  # a parent of everything
            self.assertEqual(h1.location_errors(base / "nope", forbidden), ["suite directory does not exist"])

    def test_default_forbidden_covers_agent_config_temp_and_cwd(self):
        labels = h1.default_forbidden("R", "S", home="H")
        for needle in (".pi", ".codex", ".claude", ".davinci", "temp", "results", "checkout", "current"):
            self.assertTrue(any(needle in label for label in labels), needle)


class IdentityTests(unittest.TestCase):
    def check(self, mutate, expect):
        errors = h1.identity_errors(study(mutate), PINS)
        self.assertTrue(any(expect in e for e in errors), (expect, errors))

    def test_clean_study_has_no_errors(self):
        self.assertEqual(h1.identity_errors(study(), PINS), [])

    def test_wrong_transport_label_per_arm(self):
        self.check(lambda r: r[("A", TASKS[0], 0)].update(provider_transports={"websocket": 3}), "arm A used")
        self.check(lambda r: r[("B", TASKS[0], 0)].update(provider_transports={"http": 3}), "arm B used")
        self.check(lambda r: r[("B", TASKS[0], 0)].update(provider_transports={"websocket": 1, "x": 1}), "arm B used")
        self.check(lambda r: r[("B", TASKS[0], 0)].update(provider_transports=None), "transports not recorded")

    def test_arm_transport_setting_must_match(self):
        self.check(lambda r: r[("B", TASKS[0], 0)]["effective_settings"].update(transport="auto"), "ran with transport")

    def test_a_and_b_may_differ_only_in_transport(self):
        self.check(lambda r: r[("B", TASKS[0], 1)]["effective_settings"].update(tools="other"), "beyond transport")

    def test_binary_source_and_codex_pins(self):
        self.check(lambda r: r[("A", TASKS[0], 0)].update(binary_sha256="0" * 64), "binary")
        self.check(lambda r: r[("B", TASKS[0], 0)].update(source_sha="0" * 40), "source")
        self.check(lambda r: r[("C", TASKS[0], 0)].update(version="codex-cli 9.9"), "codex version")
        self.check(lambda r: r[("C", TASKS[0], 0)]["effective_settings"].update(effort="low"), "codex settings")
        self.check(lambda r: r[("C", TASKS[0], 0)]["effective_settings"].update(ignore_user_config=False), "codex settings")

    def test_model_digest_task_and_fixture(self):
        self.check(lambda r: r[("C", TASKS[0], 0)].update(model="other"), "model")
        self.check(lambda r: r[("A", TASKS[0], 0)].update(private_suite_digest="1" * 64), "private_suite_digest")
        self.check(lambda r: r[("A", TASKS[0], 0)].update(split="dev"), "split")
        self.check(lambda r: r.update({("A", "x-unknown", 0): dict(r.pop(("A", TASKS[0], 0)), task="x-unknown")}),
                   "pinned ids")
        self.check(lambda r: r[("A", TASKS[0], 0)].update(fixture_hash="other"), "fixture")
        self.check(lambda r: r[("A", TASKS[0], 0)].update(model=None), "not recorded")

    def test_campaign_must_match_its_arm_and_rep(self):
        self.check(lambda r: r[("A", TASKS[0], 0)].update(harness="codex"), "harness")
        self.check(lambda r: r[("A", TASKS[0], 0)].update(variant="other"), "variant")

        def moved_rep(rows):  # a row filed under repetition 5 inside the repetition 0 campaign
            rows[("A", TASKS[0], 5)] = dict(rows.pop(("A", TASKS[0], 0)), rep=5)
        self.check(moved_rep, "repetition")

    def test_reasoning_must_be_counted_in_output_for_every_arm(self):
        self.check(lambda r: r[("C", TASKS[0], 0)].update(reasoning_included_in_output=False), "reasoning")


class SuccessRuleTests(unittest.TestCase):
    def label(self, arm, mutate):
        return h1.vs_codex({k: h1.metric(v) for k, v in study(mutate).items()}, arm)["success"]

    def test_equal_passes_match(self):
        self.assertEqual(self.label("B", None), "MATCHES")

    def test_two_behind_matches_three_behind_does_not(self):
        def behind(n):
            return lambda rows: fail_runs(rows, "B", n)
        self.assertEqual(self.label("B", behind(2)), "MATCHES")
        self.assertEqual(self.label("B", behind(3)), "BEHIND")

    def test_more_passes_is_ahead(self):
        self.assertEqual(self.label("B", lambda rows: fail_runs(rows, "C", 1)), "AHEAD")

    def test_task_deficit_guard_beats_the_total(self):
        def one_task_loses_two(rows):
            rows[("B", TASKS[0], 0)]["pass"] = False
            rows[("B", TASKS[0], 1)]["pass"] = False
        self.assertEqual(self.label("B", one_task_loses_two), "BEHIND")

    def test_guard_also_blocks_ahead(self):
        def ahead_but_unbalanced(rows):
            for task in TASKS[1:]:
                for rep in (0, 1):
                    rows[("C", task, rep)]["pass"] = False  # codex 8 of 18
            rows[("B", TASKS[0], 0)]["pass"] = False
            rows[("B", TASKS[0], 1)]["pass"] = False  # B 16 of 18, but task 0 lost two against a clean codex task
        self.assertEqual(self.label("B", ahead_but_unbalanced), "BEHIND")


class EfficiencyRuleTests(unittest.TestCase):
    def result(self, arm="B", mutate=None):
        return h1.vs_codex({k: h1.metric(v) for k, v in study(mutate).items()}, arm)

    def test_cheaper_and_faster_is_efficient(self):
        out = self.result()
        self.assertEqual(out["outcome"], "MATCHES_AND_EFFICIENT")
        self.assertTrue(out["efficiency"]["all"])

    def test_tokens_per_success_over_ninety_percent_is_not_efficient(self):
        def spend(rows):
            for task in TASKS:
                for rep in range(h1.REPS):
                    rows[("B", task, rep)]["input_tokens"] += 31_000  # uncached 43k+3k vs 40k+3k
        out = self.result(mutate=spend)
        self.assertFalse(out["efficiency"]["checks"]["tokens_per_success"])
        self.assertEqual(out["outcome"], "MATCHES_NOT_EFFICIENT")

    def test_slower_is_not_efficient(self):
        def slow(rows):
            for task in TASKS:
                for rep in range(h1.REPS):
                    rows[("B", task, rep)]["wall_s"] = 130.0
        out = self.result(mutate=slow)
        self.assertFalse(out["efficiency"]["checks"]["median_task_wall"])
        self.assertFalse(out["efficiency"]["checks"]["tasks_faster"])

    def test_faster_on_fewer_than_half_the_tasks_is_not_efficient(self):
        def mostly_slow(rows):
            for task in TASKS[:4]:
                for rep in range(h1.REPS):
                    rows[("B", task, rep)]["wall_s"] = 130.0
        self.assertFalse(self.result(mutate=mostly_slow)["efficiency"]["checks"]["tasks_faster"])

    def test_behind_is_not_scored_for_efficiency(self):
        out = self.result(mutate=lambda rows: fail_runs(rows, "B", 4, per_task=1))
        self.assertEqual((out["success"], out["efficiency"], out["outcome"]), ("BEHIND", "NOT_SCORED", "BEHIND"))

    def test_failed_attempts_stay_in_the_denominator(self):
        full = self.result()["comparison"]["tokens_per_success"][1]
        out = self.result(mutate=lambda rows: fail_runs(rows, "B", 2))
        self.assertAlmostEqual(out["comparison"]["tokens_per_success"][1], full * 18 / 16)

    def test_zero_passes_gives_no_tokens_per_success(self):
        def none(rows):
            for key in [k for k in rows if k[0] == "B"]:
                rows[key]["pass"] = False
        out = self.result(mutate=none)
        self.assertIsNone(out["comparison"]["tokens_per_success"][1])
        self.assertEqual(out["outcome"], "BEHIND")


class DefaultGateTests(unittest.TestCase):
    def test_gate_combines_passes_and_fallback(self):
        self.assertEqual(h1.default_gate(True, "NO_FALLBACK_OBSERVED"), "PASS")
        self.assertEqual(h1.default_gate(True, "ACCEPTABLE_PENDING_REVIEW"), "PASS")
        self.assertEqual(h1.default_gate(True, "UNVERIFIED"), "UNVERIFIED")
        self.assertEqual(h1.default_gate(True, "NOT_ACCEPTABLE"), "FAIL")
        self.assertEqual(h1.default_gate(False, "NO_FALLBACK_OBSERVED"), "FAIL")
        self.assertEqual(h1.default_gate(False, "NOT_ACCEPTABLE"), "FAIL")

    def test_a_lost_operation_fails_the_gate_in_a_full_evaluation(self):
        with tempfile.TemporaryDirectory() as d:
            rows = study()
            rows[("B", TASKS[0], 0)].update(provider_transports={"websocket": 10, "http": 4}, executed_leaf_operations=19)
            path = h1.transcript_path(d, rows[("B", TASKS[0], 0)])
            path.parent.mkdir(parents=True)
            path.write_text(json.dumps({"type": "tool_execution_start", "toolCallId": "c"}) + chr(10)
                            + json.dumps({"type": "tool_execution_end", "toolCallId": "c"}), encoding="utf-8")
            out = verdict(rows, root=d)
            self.assertEqual(out["fallback"]["verdict"], "NOT_ACCEPTABLE")
            self.assertEqual((out["b_vs_a"]["passes_gate"], out["b_vs_a"]["gate"]), (True, "FAIL"))

    def test_unverifiable_fallback_leaves_the_gate_unverified(self):
        rows = study()
        rows[("B", TASKS[0], 0)].update(provider_transports={"websocket": 10, "http": 4})
        out = verdict(rows, root=None)
        self.assertEqual(out["b_vs_a"]["gate"], "UNVERIFIED")


class PreregistrationTests(unittest.TestCase):
    """The frozen document and the code must state the same pins, schedule and rules."""

    DOC = Path(__file__).resolve().parents[3] / "docs" / "perf" / "harness-gap" / "holdout-h1-preregistration.md"
    BRIEF = DOC.with_name("holdout-h1-authoring-brief.md")

    @classmethod
    def setUpClass(cls):
        cls.text = cls.DOC.read_text(encoding="utf-8")

    def test_pins_are_in_the_document(self):
        for pin in (h1.SOURCE_SHA, h1.DAVINCI_SHA256, h1.CODEX_SHA256, h1.CODEX_HOST_SHA256, h1.CODEX_VERSION,
                    h1.MODEL, h1.EFFORT, h1.SERVICE_TIER):
            self.assertIn(pin, self.text)

    def test_every_campaign_is_in_the_document_in_order(self):
        positions = [self.text.index(unit["name"]) for unit in h1.schedule()]
        self.assertEqual(positions, sorted(positions))

    def test_thresholds_match_the_code(self):
        self.assertIn("at most %.2f" % h1.MAX_TOKENS_PER_SUCCESS_RATIO, self.text)
        self.assertIn("at most %.2f" % h1.MAX_MEDIAN_TASK_WALL_RATIO, self.text)
        self.assertIn("`d >= c - %d`" % h1.SUCCESS_TOLERANCE, self.text)
        self.assertIn("`c_task - d_task > %d`" % h1.MAX_TASK_PASS_DEFICIT, self.text)
        self.assertIn("at least %d of the %d tasks" % (h1.MIN_TASKS_FASTER, h1.TASK_COUNT), self.text)
        self.assertIn("%d or more verified passes behind" % h1.SAFETY_STOP_LAG, self.text)
        self.assertIn("ore than %d DaVinci attempt" % h1.MAX_UNKNOWN_ATTEMPTS_RUN, self.text)
        self.assertIn("more than %d in total" % h1.MAX_UNKNOWN_ATTEMPTS_TOTAL, self.text)
        self.assertIn("**%d runs, no warm-ups**" % h1.MAX_RUNS, self.text)
        self.assertIn("%d characters or more" % h1.LEAK_MIN_LINE, self.text)

    def test_the_document_states_its_limits(self):
        for phrase in ("operational", "not a statistically established", "HUMAN_REVIEW_REQUIRED", "diagnostic-only",
                       "Not ready", "not launched"):
            self.assertIn(phrase, self.text)

    def test_the_brief_matches_the_code(self):
        brief = self.BRIEF.read_text(encoding="utf-8")
        for fragment in ("20 to 60 test functions", "70%", "%d characters" % h1.LEAK_MIN_LINE, "Python 3.12 and 3.13",
                         "pytest 8.3.5", "make-pins", "private-import", "validate", "diff_used", "pr-description",
                         "Change or create only these files"):
            self.assertIn(fragment, brief)
        self.assertNotIn(chr(0x2014), brief + self.text)  # no em dashes


class UsageKnownTests(unittest.TestCase):
    """The scoring layer refuses unknown usage by itself, whatever the stop rules above it do."""

    def test_complete_rows_are_known(self):
        self.assertTrue(h1.usage_known(row("A", TASKS[0], 0)))
        self.assertTrue(h1.usage_known(row("C", TASKS[0], 0)))

    def test_missing_totals_are_unknown(self):
        for field in ("input_tokens", "cached_tokens", "output_tokens"):
            self.assertFalse(h1.usage_known(row("B", TASKS[0], 0, **{field: None})), field)

    def test_davinci_attempts_without_usage_are_unknown(self):
        self.assertFalse(h1.usage_known(row("A", TASKS[0], 0, usage_unknown_attempts=1)))
        self.assertFalse(h1.usage_known(row("A", TASKS[0], 0, usage_unknown_attempts=None)))

    def test_codex_needs_complete_request_telemetry(self):
        self.assertFalse(h1.usage_known(row("C", TASKS[0], 0, request_metrics_complete=False)))
        self.assertFalse(h1.usage_known(row("C", TASKS[0], 0, request_metrics_complete=None)))


class EvaluateTests(unittest.TestCase):
    def test_successful_study(self):
        out = verdict(study())
        self.assertEqual(out["verdict"], "COMPLETE")
        self.assertEqual(out["passes"], {"A": 18, "B": 18, "C": 18})
        self.assertEqual(out["b_vs_codex"]["outcome"], "MATCHES_AND_EFFICIENT")
        self.assertEqual(out["b_vs_a"]["gate"], "PASS")
        self.assertEqual(out["decision"], "HUMAN_REVIEW_REQUIRED")
        self.assertEqual(out["fallback"]["verdict"], "NO_FALLBACK_OBSERVED")
        self.assertIn("not a statistically established", out["note"])

    def test_failed_success_gate(self):
        def both_davinci_arms_three_behind(rows):
            fail_runs(rows, "A", 3)
            fail_runs(rows, "B", 3)
        out = verdict(study(both_davinci_arms_three_behind))
        self.assertEqual(out["verdict"], "COMPLETE")
        self.assertEqual(out["b_vs_codex"]["outcome"], "BEHIND")
        self.assertEqual(out["a_vs_codex"]["outcome"], "BEHIND")
        self.assertEqual(out["b_vs_a"]["gate"], "PASS")

    def test_websocket_one_pass_behind_http_fails_the_gate_without_stopping(self):
        out = verdict(study(lambda rows: fail_runs(rows, "B", 1)))
        self.assertEqual(out["verdict"], "COMPLETE")
        self.assertEqual(out["b_vs_a"]["gate"], "FAIL")
        self.assertFalse(out["b_vs_a"]["passes_gate"])
        self.assertEqual(out["b_vs_codex"]["success"], "MATCHES")

    def test_gate_is_paired_with_a_not_with_codex(self):
        out = verdict(study(lambda rows: fail_runs(rows, "A", 1)))
        self.assertEqual(out["b_vs_a"]["gate"], "PASS")

    def test_no_verdict_before_the_leak_scan(self):
        out = h1.evaluate(study(), PINS, None)
        self.assertEqual(out["verdict"], "NEEDS_LEAK_SCAN")
        self.assertNotIn("passes", out)

    def test_leak_means_contamination_review(self):
        leak = {"status": "LEAK", "files_scanned": 54, "needles": 10, "campaigns_with_hits": {"h1-02-B-r0": 1}}
        out = verdict(study(), leak=leak)
        self.assertEqual(out["verdict"], "CONTAMINATION_REVIEW")
        self.assertNotIn("passes", out)

    def test_invalid_pins_and_load_errors(self):
        self.assertEqual(h1.evaluate(study(), dict(PINS, model="x"), CLEAN)["verdict"], "INVALID")
        self.assertEqual(verdict(study(), load_errors=["duplicate row"])["verdict"], "INVALID")

    def test_identity_problem_stops_the_study(self):
        rows = study(lambda r: r[("B", TASKS[2], 1)].update(provider_transports={"http": 14}))
        out = verdict(rows)
        self.assertEqual(out["verdict"], "STOPPED")
        self.assertIn("arm B used", out["stop"])


class IncompleteTests(unittest.TestCase):
    def test_missing_runs_make_the_study_incomplete(self):
        rows = study(lambda r: [r.pop(("C", t, 2)) for t in TASKS])
        out = verdict(rows)
        self.assertEqual(out["verdict"], "INCOMPLETE")
        self.assertEqual(len(out["missing_runs"]), 6)
        self.assertNotIn("passes", out)

    def test_unknown_usage_is_never_imputed(self):
        out = verdict(study(lambda r: r[("C", TASKS[0], 0)].update(input_tokens=None)))
        self.assertEqual(out["verdict"], "INCOMPLETE")
        self.assertEqual(out["unscorable_runs"], ["C %s r0" % TASKS[0]])

    def test_codex_run_without_complete_telemetry_stops(self):
        out = verdict(study(lambda r: r[("C", TASKS[0], 0)].update(request_metrics_complete=False)))
        self.assertEqual(out["verdict"], "STOPPED")
        self.assertIn("telemetry", out["stop"])


class StopRuleTests(unittest.TestCase):
    def stop(self, mutate):
        return h1.first_stop(study(mutate), PINS)

    def test_clean_study_never_stops(self):
        self.assertIsNone(self.stop(None))

    def test_harness_stop_reason(self):
        self.assertIn("harness stop: forbidden_artifact",
                      self.stop(lambda r: r[("A", TASKS[0], 0)].update(stop_reason="forbidden_artifact")))

    def test_credential_failure_is_a_harness_stop(self):
        self.assertIn("harness stop", self.stop(lambda r: r[("C", TASKS[1], 0)].update(stop_reason="auth_failed")))

    def test_attempts_without_usage_limits(self):
        self.assertIn("attempts without usage", self.stop(lambda r: r[("A", TASKS[0], 0)].update(usage_unknown_attempts=2)))
        def spread(rows):
            for task in TASKS[:4]:
                rows[("A", task, 0)]["usage_unknown_attempts"] = 1
        self.assertIn("in total", self.stop(spread))

    def test_failed_run_with_unknown_attempts_is_not_separable(self):
        def mutate(rows):
            rows[("A", TASKS[0], 0)].update(usage_unknown_attempts=1, **{"pass": False})
        self.assertIn("cause not separable", self.stop(mutate))

    def test_websocket_run_with_unknown_attempts_stops(self):
        self.assertIn("usage incomplete on a WebSocket run",
                      self.stop(lambda r: r[("B", TASKS[0], 0)].update(usage_unknown_attempts=1)))

    def test_missing_output_tokens_stops(self):
        self.assertIn("output tokens", self.stop(lambda r: r[("A", TASKS[0], 0)].update(output_tokens=None)))

    def test_b_two_passes_behind_a_is_a_safety_stop(self):
        def mutate(rows):
            fail_runs(rows, "B", 2)
        self.assertIn("safety: B is 2 verified passes behind A", self.stop(mutate))

    def test_b_one_pass_behind_a_continues(self):
        self.assertIsNone(self.stop(lambda r: fail_runs(r, "B", 1)))

    def test_safety_stop_ignores_unpaired_blocks(self):
        rows = study(lambda r: [r.pop(("A", t, 0)) for t in TASKS])
        fail_runs(rows, "B", 2, per_task=1)  # reps 0 of A are absent, so those blocks are unpaired
        self.assertIsNone(h1.first_stop(rows, PINS))

    def test_stop_order_follows_the_launch_order(self):
        def mutate(rows):
            rows[("C", TASKS[0], 0)].update(stop_reason="late")
            rows[("A", TASKS[0], 0)].update(stop_reason="early")
        self.assertIn("early", self.stop(mutate))


class FallbackTests(unittest.TestCase):
    def write(self, root, r, calls):
        path = h1.transcript_path(root, r)
        path.parent.mkdir(parents=True, exist_ok=True)
        lines = []
        for call, starts, ends in calls:
            lines += [json.dumps({"type": "tool_execution_start", "toolCallId": call})] * starts
            lines += [json.dumps({"type": "tool_execution_end", "toolCallId": call})] * ends
        path.write_text("\n".join(lines), encoding="utf-8")

    def rows(self, **extra):
        rows = study()
        rows[("B", TASKS[0], 0)].update(provider_transports={"websocket": 10, "http": 4}, provider_attempts=15,
                                        **extra)
        return rows

    def test_clean_transcript_is_recoverable(self):
        with tempfile.TemporaryDirectory() as d:
            rows = self.rows()
            self.write(d, rows[("B", TASKS[0], 0)], [("c1", 1, 1), ("c2", 1, 1)])
            events = h1.fallback_events(rows, d)
            self.assertEqual([e["classification"] for e in events], ["RECOVERABLE"])
            self.assertEqual(events[0]["http_requests"], 4)
            self.assertEqual(h1.fallback_verdict(events, 18)["verdict"], "ACCEPTABLE_PENDING_REVIEW")

    def test_duplicated_tool_call_is_lost_or_duplicated(self):
        with tempfile.TemporaryDirectory() as d:
            rows = self.rows()
            self.write(d, rows[("B", TASKS[0], 0)], [("c1", 2, 2)])
            events = h1.fallback_events(rows, d)
            self.assertEqual(events[0]["classification"], "LOST_OR_DUPLICATED")
            self.assertEqual(h1.fallback_verdict(events, 18)["verdict"], "NOT_ACCEPTABLE")

    def test_operation_count_mismatch_is_lost_or_duplicated(self):
        with tempfile.TemporaryDirectory() as d:
            rows = self.rows(executed_leaf_operations=19)
            self.write(d, rows[("B", TASKS[0], 0)], [("c1", 1, 1)])
            self.assertEqual(h1.fallback_events(rows, d)[0]["classification"], "LOST_OR_DUPLICATED")

    def test_missing_transcript_or_counts_is_unverified(self):
        with tempfile.TemporaryDirectory() as d:
            events = h1.fallback_events(self.rows(), d)
            self.assertEqual(events[0]["classification"], "UNVERIFIED")
            self.assertEqual(h1.fallback_verdict(events, 18)["verdict"], "UNVERIFIED")
            rows = self.rows(executed_leaf_operations=None)
            self.write(d, rows[("B", TASKS[0], 0)], [("c1", 1, 1)])
            self.assertEqual(h1.fallback_events(rows, d)[0]["classification"], "UNVERIFIED")

    def test_failed_fallback_run_is_not_acceptable(self):
        with tempfile.TemporaryDirectory() as d:
            rows = self.rows(**{"pass": False})
            self.write(d, rows[("B", TASKS[0], 0)], [("c1", 1, 1)])
            self.assertEqual(h1.fallback_verdict(h1.fallback_events(rows, d), 18)["verdict"], "NOT_ACCEPTABLE")

    def test_no_fallback_states_the_bound_without_calling_it_acceptable(self):
        out = h1.fallback_verdict([], 18)
        self.assertEqual(out["verdict"], "NO_FALLBACK_OBSERVED")
        self.assertAlmostEqual(out["rule_of_three_upper_95"], 0.167, places=3)
        self.assertIn("does not show", out["note"])

    def test_only_arm_b_runs_with_http_requests_are_events(self):
        rows = study()
        rows[("A", TASKS[0], 0)]["provider_transports"] = {"http": 14}
        self.assertEqual(h1.fallback_events(rows, None), [])

    def test_full_evaluation_lists_fallback_events_without_transcript_content(self):
        with tempfile.TemporaryDirectory() as d:
            rows = self.rows()
            self.write(d, rows[("B", TASKS[0], 0)], [("c1", 1, 1)])
            out = verdict(rows, root=d)
            self.assertEqual(out["fallback"]["verdict"], "ACCEPTABLE_PENDING_REVIEW")
            self.assertEqual(out["fallback"]["event_count"], 1)
            self.assertEqual(set(out["fallback"]["events"][0]) & {"stdout", "args", "output"}, set())


class RowLoadingTests(unittest.TestCase):
    def test_arm_comes_from_the_campaign_directory_and_unknown_directories_are_errors(self):
        with tempfile.TemporaryDirectory() as d:
            for name, task in (("h1-02-B-r0", TASKS[0]), ("h1-99-X-r0", TASKS[1])):
                (Path(d) / name).mkdir()
                (Path(d) / name / "results.jsonl").write_text(json.dumps({"task": task, "rep": 0}) + "\n", encoding="utf-8")
            rows, errors = h1.load_rows(d)
            self.assertEqual(list(rows), [("B", TASKS[0], 0)])
            self.assertEqual(len(errors), 1)

    def test_duplicate_rows_are_errors(self):
        with tempfile.TemporaryDirectory() as d:
            (Path(d) / "h1-01-A-r0").mkdir()
            line = json.dumps({"task": TASKS[0], "rep": 0}) + "\n"
            (Path(d) / "h1-01-A-r0" / "results.jsonl").write_text(line * 2, encoding="utf-8")
            self.assertEqual(len(h1.load_rows(d)[1]), 1)


HIDDEN_LINE = "    assert normalise_ledger(entries, tolerance=0.005) == expected_balances_for_quarter_end"
SHORT_LINE = "    assert True"
PROMPT_LINE = "Implement the ledger normaliser with a tolerance of one half of one percent exactly as described"


def make_suite(base):
    suite = Path(base) / "vault" / "suite"
    task = suite / TASKS[0]
    for sub in ("repo", "hidden"):
        (task / sub).mkdir(parents=True)
    (task / "repo" / "ledger.py").write_text("def normalise_ledger():\n    pass\n", encoding="utf-8")
    (task / "hidden" / "test_ledger.py").write_text(
        "\n".join([SHORT_LINE, HIDDEN_LINE, "    " + PROMPT_LINE]) + "\n", encoding="utf-8")
    (task / "task.json").write_text(json.dumps({"prompt": PROMPT_LINE}), encoding="utf-8")
    return suite


class LeakScanTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.suite = make_suite(self.tmp.name)
        self.root = Path(self.tmp.name) / "results"
        (self.root / "h1-01-A-r0" / "davinci").mkdir(parents=True)

    def transcript(self, text, name="t-r0.stdout.jsonl", directory="davinci"):
        path = self.root / "h1-01-A-r0" / directory / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text, encoding="utf-8")

    def test_clean_transcripts(self):
        self.transcript(json.dumps({"text": "implemented the ledger"}))
        out = h1.leak_scan(self.root, self.suite)
        self.assertEqual((out["status"], out["campaigns_with_hits"]), ("CLEAN", {}))
        self.assertEqual(out["files_scanned"], 1)

    def test_distinctive_hidden_line_is_detected(self):
        self.transcript(json.dumps({"text": "cat output: " + HIDDEN_LINE.strip()}))
        out = h1.leak_scan(self.root, self.suite)
        self.assertEqual((out["status"], out["campaigns_with_hits"]), ("LEAK", {"h1-01-A-r0": 1}))

    def test_suite_path_is_detected_in_either_slash_style(self):
        self.transcript(json.dumps({"cmd": "ls " + str(self.suite.resolve())}))
        self.assertEqual(h1.leak_scan(self.root, self.suite)["status"], "LEAK")
        self.transcript(json.dumps({"cmd": "ls " + self.suite.resolve().as_posix()}))
        self.assertEqual(h1.leak_scan(self.root, self.suite)["status"], "LEAK")

    def test_lines_the_prompt_or_starter_already_contain_are_not_needles(self):
        self.transcript(PROMPT_LINE + "\ndef normalise_ledger():\n" + SHORT_LINE)
        self.assertEqual(h1.leak_scan(self.root, self.suite)["status"], "CLEAN")

    def test_stderr_and_otel_are_scanned_and_credentials_are_not(self):
        self.transcript(HIDDEN_LINE, name="t-r0.stderr.txt")
        self.assertEqual(h1.leak_scan(self.root, self.suite)["status"], "LEAK")
        for path in (self.root / "h1-01-A-r0" / "davinci").iterdir():
            path.unlink()
        agent = self.root / "h1-01-A-r0" / "_agent"
        agent.mkdir()
        (agent / "auth.json").write_text(HIDDEN_LINE, encoding="utf-8")
        (agent / "x.stdout.jsonl").write_text(HIDDEN_LINE, encoding="utf-8")
        out = h1.leak_scan(self.root, self.suite)
        self.assertEqual((out["status"], out["files_scanned"]), ("CLEAN", 0))

    def test_result_carries_counts_and_never_content(self):
        self.transcript(json.dumps({"text": HIDDEN_LINE}))
        text = json.dumps(h1.leak_scan(self.root, self.suite))
        for secret in (HIDDEN_LINE.strip(), str(self.suite), "normalise_ledger", "vault"):
            self.assertNotIn(secret, text)

    def test_needle_table_holds_only_digest_keys_and_skips_short_lines(self):
        table = h1.needles(self.suite)
        self.assertNotIn(SHORT_LINE.strip(), table.values())
        self.assertIn(HIDDEN_LINE.strip(), table.values())
        self.assertTrue(all(len(key) == 12 for key in table))


class ReportPrivacyTests(unittest.TestCase):
    def test_a_complete_report_contains_no_suite_content(self):
        with tempfile.TemporaryDirectory() as d:
            suite = make_suite(d)
            rows = study()
            for row_ in rows.values():
                row_["pytest"] = HIDDEN_LINE
                row_["changed"] = ["src/" + PROMPT_LINE]
                row_["grading_evidence"] = {"stdout": HIDDEN_LINE}
            out = json.dumps(verdict(rows), default=str)
            for secret in list(h1.needles(suite).values()) + [PROMPT_LINE, "ledger.py"]:
                self.assertNotIn(secret, out)


class CliTests(unittest.TestCase):
    def run_main(self, *argv):
        out = io.StringIO()
        code = 0
        with contextlib.redirect_stdout(out):
            try:
                h1.main(list(argv))
            except SystemExit as exit_:
                code = exit_.code
        return code, out.getvalue()

    def results_root(self, base, mutate=None):
        root = Path(base) / "results"
        for (arm, task, rep), r in study(mutate).items():
            directory = root / r["_unit"]["name"]
            directory.mkdir(parents=True, exist_ok=True)
            with open(directory / "results.jsonl", "a", encoding="utf-8") as handle:
                handle.write(json.dumps({k: v for k, v in r.items() if k != "_unit"}) + chr(10))
        return root

    def test_schedule_lists_nine_campaigns(self):
        code, out = self.run_main("schedule")
        self.assertEqual((code, len(out.strip().splitlines())), (0, 9))

    def test_commands_prints_only_text_and_runs_nothing(self):
        code, out = self.run_main("commands", "--root", "R", "--settings-dir", "S", "--suite", "Q",
                                  "--davinci", "D", "--codex", "C", "--stage", "1")
        self.assertEqual((code, len(out.strip().splitlines())), (0, 3))

    def test_verify_pins_exit_status(self):
        with tempfile.TemporaryDirectory() as d:
            pins = Path(d) / "pins.json"
            pins.write_text(json.dumps(PINS), encoding="utf-8")
            files = []
            for name in ("d", "c", "h"):
                (Path(d) / name).write_bytes(b"x")
                files.append(str(Path(d) / name))
            code, out = self.run_main("verify-pins", "--pins", str(pins), "--davinci", files[0], "--codex", files[1],
                                      "--codex-host", files[2], "--harness-sha", "e" * 40)
            self.assertEqual(code, 1)  # the stand-in files do not hash to the registered pins
            self.assertFalse(json.loads(out)["ok"])

    def test_check_location_exit_status(self):
        with tempfile.TemporaryDirectory() as d:
            suite = make_suite(d)
            code, out = self.run_main("check-location", "--suite", str(suite), "--results-root", str(Path(d) / "results"),
                                      "--repo", str(Path(d) / "repo"))
            # the suite sits under the system temp directory in this test, which is refused on purpose
            self.assertEqual(code, 1)
            self.assertIn("system temp", out)

    def test_report_needs_the_leak_scan_then_completes(self):
        with tempfile.TemporaryDirectory() as d:
            root = self.results_root(d)
            pins = Path(d) / "pins.json"
            pins.write_text(json.dumps(PINS), encoding="utf-8")
            suite = make_suite(d)
            _, first = self.run_main("report", "--root", str(root), "--pins", str(pins))
            self.assertEqual(json.loads(first)["verdict"], "NEEDS_LEAK_SCAN")
            _, second = self.run_main("report", "--root", str(root), "--pins", str(pins), "--suite", str(suite))
            report = json.loads(second)
            self.assertEqual((report["verdict"], report["leak_scan"]["status"]), ("COMPLETE", "CLEAN"))

    def test_report_after_a_leak_withholds_the_numbers(self):
        with tempfile.TemporaryDirectory() as d:
            root = self.results_root(d)
            transcript = root / "h1-02-B-r0" / "davinci" / "t-r0.stdout.jsonl"
            transcript.parent.mkdir(parents=True)
            transcript.write_text(HIDDEN_LINE.strip(), encoding="utf-8")
            pins = Path(d) / "pins.json"
            pins.write_text(json.dumps(PINS), encoding="utf-8")
            _, out = self.run_main("report", "--root", str(root), "--pins", str(pins), "--suite", str(make_suite(d)))
            report = json.loads(out)
            self.assertEqual(report["verdict"], "CONTAMINATION_REVIEW")
            self.assertNotIn("passes", report)
            self.assertNotIn(HIDDEN_LINE.strip(), out)


class RunnerCompatibilityTests(unittest.TestCase):
    def test_fields_the_evaluator_reads_exist_in_the_bench_row_writer(self):
        source = (Path(__file__).resolve().parents[1] / "bench.py").read_text(encoding="utf-8")
        for field in ("private_suite_digest", "provider_transports", "usage_unknown_attempts", "executed_leaf_operations",
                      "reasoning_included_in_output", "request_metrics_complete", "provider_attempts",
                      "logical_requests", "binary_sha256", "output_tokens", "cached_tokens", "wall_s", "stop_reason"):
            self.assertIn('"%s"' % field, source, field)

    def test_private_suite_loader_exists(self):
        import private_suite
        self.assertTrue(callable(private_suite.load_frozen))


if __name__ == "__main__":
    unittest.main()
