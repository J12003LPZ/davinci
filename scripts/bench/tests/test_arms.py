"""Offline tests for the four-arm pilot layer: settings, preflight, schedule, attribution."""
import contextlib
import hashlib
import io
import json
import sys
import tempfile
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import arms  # noqa: E402
import campaign  # noqa: E402


def build_host(root, extra=None):
    """A tiny host bundle plus a manifest in the real format (pins included)."""
    files = {"host.mjs": b"entry", "bounded-worker.mjs": b"worker", "q/quickjs.wasm": b"wasm"}
    host = Path(root) / "host"
    for relative, data in files.items():
        path = host / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(data)
    manifest = dict(arms.MANIFEST_PINS, entry="host.mjs", worker="bounded-worker.mjs", wasm="q/quickjs.wasm",
                    assets=[{"path": p, "bytes": len(d), "sha256": hashlib.sha256(d).hexdigest()}
                            for p, d in sorted(files.items())])
    manifest.update(extra or {})
    manifest_path = Path(root) / "assets-manifest.json"
    manifest_path.write_text(json.dumps(manifest), encoding="utf-8")
    node = Path(root) / "node.exe"
    node.write_bytes(b"fake")
    return str(node.resolve()), str(host.resolve()), manifest_path


class SettingsTests(unittest.TestCase):
    def test_each_arm_maps_to_a_harness_and_mode(self):
        self.assertEqual({a: (s["harness"], s["mode"]) for a, s in arms.ARM_SPECS.items()}, {
            "davinci_direct": ("davinci", "off"), "davinci_script": ("davinci", "controlled"),
            "droid_direct": ("droid-direct", "direct-only"), "droid_script": ("droid-script", "script-only")})
        with self.assertRaises(ValueError):
            arms.spec("davinci")

    def test_script_arm_uses_trusted_settings_and_flag_with_absolute_paths(self):
        node, host = str(Path("node.exe").resolve()), str(Path("host").resolve())
        base = {"compaction": {"enabled": True}, "codemode": {"mode": "read-only"}}
        settings = arms.arm_settings("davinci_script", base, node, host)
        self.assertEqual(settings["codemode"], {"mode": "controlled", "nodePath": node, "hostPath": host})
        self.assertEqual(settings["compaction"], {"enabled": True})
        self.assertEqual(base["codemode"], {"mode": "read-only"})  # input untouched
        self.assertEqual(arms.davinci_session_flags("davinci_script"), ["--codemode", "controlled"])

    def test_direct_arm_is_forced_off_even_if_base_asks_otherwise(self):
        base = {"codemode": {"mode": "controlled", "nodePath": "x"}}
        self.assertEqual(arms.arm_settings("davinci_direct", base)["codemode"], {"mode": "off"})
        self.assertEqual(arms.davinci_session_flags("davinci_direct"), ["--no-session"])
        self.assertIsNone(arms.arm_settings("droid_script", base))

    def test_relative_or_missing_paths_are_refused(self):
        absolute = str(Path("x").resolve())
        for node, host in (("node", absolute), (absolute, "host"), (None, absolute), (absolute, None)):
            with self.assertRaises(ValueError):
                arms.arm_settings("davinci_script", {}, node, host)


class PreflightTests(unittest.TestCase):
    def run_preflight(self, root, node_version="v24.21.0\n", manifest_extra=None, mutate=None):
        node, host, manifest = build_host(root, manifest_extra)
        if mutate:
            mutate(Path(host))
        return arms.preflight("davinci_script", node, host, manifest_path=manifest,
                              node_probe=lambda path: node_version)

    def test_valid_node_and_bundle_pass(self):
        with tempfile.TemporaryDirectory() as root:
            record = self.run_preflight(root)
        self.assertTrue(record["checked"])
        self.assertEqual((record["node_version"], record["host_files"]), ("v24.21.0", 3))

    def test_other_arms_have_nothing_to_check(self):
        self.assertEqual(arms.preflight("davinci_direct"), {"arm": "davinci_direct", "checked": False})

    def test_wrong_or_malformed_node_version_fails(self):
        for version in ("v24.20.0\n", "v24.21.1", "24.21.0", "v24.21.0 ", "", "v24.21.0\nextra", None):
            with tempfile.TemporaryDirectory() as root, self.assertRaises(arms.PreflightError, msg=repr(version)):
                self.run_preflight(root, node_version=version)

    def test_probe_failure_and_missing_node_fail(self):
        with tempfile.TemporaryDirectory() as root:
            node, host, manifest = build_host(root)
            def boom(path):
                raise OSError("no exec")
            with self.assertRaises(arms.PreflightError):
                arms.preflight("davinci_script", node, host, manifest_path=manifest, node_probe=boom)
            Path(node).unlink()
            with self.assertRaises(arms.PreflightError):
                arms.preflight("davinci_script", node, host, manifest_path=manifest, node_probe=lambda p: "v24.21.0")

    def test_tampered_missing_extra_and_linked_assets_fail(self):
        def tamper(host):
            (host / "host.mjs").write_bytes(b"EVIL!")
        def remove(host):
            (host / "bounded-worker.mjs").unlink()
        def extra(host):
            (host / "stowaway.mjs").write_bytes(b"x")
        for mutate in (tamper, remove, extra):
            with tempfile.TemporaryDirectory() as root, self.assertRaises(arms.PreflightError, msg=mutate.__name__):
                self.run_preflight(root, mutate=mutate)

    def test_manifest_must_match_the_trusted_pins(self):
        for extra in ({"node_version": "24.20.0"}, {"schema_version": 2}, {"surprise": 1}, {"assets": []}):
            with tempfile.TemporaryDirectory() as root, self.assertRaises(arms.PreflightError, msg=str(extra)):
                self.run_preflight(root, manifest_extra=extra)

    def test_relative_paths_fail(self):
        with self.assertRaises(arms.PreflightError):
            arms.preflight("davinci_script", "node", "host")

    def test_repo_manifest_parses_in_the_rust_format(self):
        manifest = arms.load_manifest()
        self.assertEqual(manifest["node_version"], "24.21.0")
        self.assertIn(manifest["entry"], {asset["path"] for asset in manifest["assets"]})

    def test_failed_preflight_never_selects_another_arm(self):
        with tempfile.TemporaryDirectory() as root:
            node, host, manifest = build_host(root)
            with self.assertRaises(arms.PreflightError):
                arms.preflight("davinci_script", node, host, manifest_path=manifest,
                               node_probe=lambda p: "v1.0.0")
        self.assertEqual(arms.davinci_session_flags("davinci_script"), ["--codemode", "controlled"])


class ScheduleTests(unittest.TestCase):
    TASKS = ["t1", "t2", "t3"]

    def test_uses_the_existing_scheduler_and_is_deterministic(self):
        first = arms.schedule_arms(self.TASKS, [0, 1], arms.ARMS, 7)
        self.assertEqual(first, arms.schedule_arms(self.TASKS, [0, 1], arms.ARMS, 7))
        self.assertEqual(first, campaign.schedule(self.TASKS, [0, 1], list(arms.ARMS), 7))
        self.assertEqual(len(first), 3 * 2 * 4)
        self.assertEqual({item[0] for item in first}, set(arms.ARMS))

    def test_every_task_and_rep_is_run_once_per_arm_and_first_arm_rotates(self):
        order = arms.schedule_arms(self.TASKS, [0], arms.ARMS, 0)
        self.assertEqual(len({item for item in order}), 12)
        firsts = [order[i * 4][0] for i in range(3)]
        self.assertEqual(len(set(firsts)), 3)

    def test_unknown_or_duplicate_arms_are_refused(self):
        with self.assertRaises(ValueError):
            arms.schedule_arms(self.TASKS, [0], ["davinci", "droid_direct"], 0)
        with self.assertRaises(ValueError):
            arms.schedule_arms(self.TASKS, [0], ["droid_direct", "droid_direct"], 0)

    def test_plan_reports_the_count_before_anything_runs(self):
        result = arms.plan(self.TASKS, [0, 1], arms.ARMS, 3)
        self.assertEqual(result["planned_runs"], 24)
        self.assertEqual(result["per_arm"], {arm: 6 for arm in arms.ARMS})
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            arms.main(["plan", "--tasks", "t1", "t2", "--reps", "2", "--seed", "3"])
        self.assertTrue(out.getvalue().startswith("planned runs: 16 "))


class AttributionTests(unittest.TestCase):
    def davinci_row(self, mode, available=True, harness="davinci"):
        return {"harness": harness, "codemode_mode": mode, "codemode_available": available}

    def test_confirmed_only_when_the_run_reports_the_arm_mode(self):
        row = arms.stamp_row(self.davinci_row("controlled"), "davinci_script", {"checked": True})
        self.assertEqual((row["arm"], row["arm_requested_mode"], row["arm_observed_mode"],
                          row["arm_attribution"]), ("davinci_script", "controlled", "controlled", "confirmed"))
        self.assertEqual(arms.row_arm(row), "davinci_script")
        direct = arms.stamp_row(self.davinci_row("off", available=False), "davinci_direct")
        self.assertEqual(arms.row_arm(direct), "davinci_direct")

    def test_a_script_arm_that_ran_read_only_or_off_is_a_mismatch(self):
        for mode in ("read-only", "off"):
            row = arms.stamp_row(self.davinci_row(mode), "davinci_script")
            self.assertEqual(row["arm_attribution"], "mismatch")
            self.assertIsNone(arms.row_arm(row))

    def test_missing_report_is_unknown_not_confirmed(self):
        row = arms.stamp_row({"harness": "davinci"}, "davinci_script")
        self.assertEqual((row["arm_attribution"], row["arm_observed_mode"]), ("unknown", None))
        self.assertIsNone(arms.row_arm(row))

    def test_wrong_harness_is_a_mismatch(self):
        row = arms.stamp_row(self.davinci_row("controlled", harness="codex"), "davinci_script")
        self.assertEqual(row["arm_attribution"], "mismatch")

    def test_droid_arms_use_the_resolved_mode(self):
        ok = {"harness": "droid-script", "droid_resolved_tool_execution_mode": "script_only"}
        self.assertEqual(arms.row_arm(arms.stamp_row(ok, "droid_script")), "droid_script")
        bad = {"harness": "droid-script", "droid_resolved_tool_execution_mode": "direct-only"}
        self.assertEqual(arms.stamp_row(bad, "droid_script")["arm_attribution"], "mismatch")
        plain = {"harness": "droid", "droid_tool_execution_mode": "direct-only",
                 "droid_resolved_tool_execution_mode": "direct-only"}
        self.assertEqual(arms.stamp_row(plain, "droid_direct")["arm_attribution"], "confirmed")
        self.assertEqual(arms.stamp_row(plain, "droid_script")["arm_attribution"], "mismatch")

    def test_stamping_does_not_mutate_the_input_row(self):
        original = self.davinci_row("controlled")
        arms.stamp_row(original, "davinci_script")
        self.assertNotIn("arm", original)


if __name__ == "__main__":
    unittest.main()
