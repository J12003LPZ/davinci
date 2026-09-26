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
            "unrelated": [], "artifact_leak": False, "pass": True,
            "wall_s": 2.125, "binary_sha256": "binary-" + harness,
            "effective_settings": {"fixed": True}}


class CampaignTests(unittest.TestCase):
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

    def test_manifest_is_authoritative_even_when_rows_agree(self):
        rows = [row("davinci"), row("codex")]
        manifest = {key: rows[0][key] for key in ("schema_version", "campaign", "variant", *campaign.COMPARABLE)}
        manifest.update(tasks=["t1-intervals"], repetitions=[0], harnesses=["davinci", "codex"],
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
