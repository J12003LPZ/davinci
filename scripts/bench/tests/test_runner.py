"""Offline runner boundary tests, using public synthetic files only."""
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import runner
import bench


class RunnerTests(unittest.TestCase):
    def test_codex_priority_tier_is_an_explicit_benchmark_arm(self):
        with patch.object(bench, "SERVICE_TIER", "fast"):
            command = bench.command("codex", "prompt", "workdir")
            self.assertIn('service_tier="fast"', command)
        with patch.object(bench, "SERVICE_TIER", "default"):
            command = bench.command("codex", "prompt", "workdir")
            self.assertNotIn('service_tier="fast"', command)

    def test_pin_model_store_selects_exact_public_profile(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "models-store.json"
            model = {"id": "fixture", "provider": "openai-codex",
                     "api": "openai-codex-responses", "baseUrl": "https://chatgpt.com/backend-api"}
            path.write_text(json.dumps({"providers": {"openai-codex": {"models": [model]}}}))
            pinned = runner.pin_model_store(path, "fixture")
            self.assertEqual(pinned["providers"]["openai-codex"]["models"], [model])
            with self.assertRaises(ValueError):
                runner.pin_model_store(path, "missing")
            model["headers"] = {}
            path.write_text(json.dumps({"providers": {"openai-codex": {"models": [model]}}}))
            self.assertNotIn("headers", runner.pin_model_store(path, "fixture")["providers"]["openai-codex"]["models"][0])
            model["headers"] = {"Authorization": "synthetic-secret"}
            path.write_text(json.dumps({"providers": {"openai-codex": {"models": [model]}}}))
            with self.assertRaises(ValueError):
                runner.pin_model_store(path, "fixture")

    def test_model_mismatch_stops_on_observed_identity_only(self):
        event = {"type": "provider_observation", "observation": {
            "kind": "logical_start", "purpose": "coding", "model": "openai-codex/other"}}
        self.assertEqual(runner.model_stop_reason(json.dumps(event), "fixture"), "model_mismatch")
        event["observation"]["model"] = "openai-codex/fixture"
        self.assertIsNone(runner.model_stop_reason(json.dumps(event), "fixture"))

    def test_measurement_failure_preserves_row_before_stopping(self):
        with tempfile.TemporaryDirectory() as tmp:
            measured = {"exit": 0, "stdout": "", "stderr": "", "wall_s": 1.25,
                        "started_at": "start", "finished_at": "end"}
            with (patch.object(bench, "RUNS", tmp),
                  patch.object(bench, "load", return_value={"prompt": "fixture", "allowed": []}),
                  patch.object(bench, "prepare", side_effect=lambda tid, dest: Path(dest).mkdir(parents=True)),
                  patch.object(bench, "changed_files", side_effect=RuntimeError("git status failed")),
                  patch.object(bench, "grade", return_value=(True, 1, 0, "fixture")),
                  patch.object(bench, "execute", return_value=measured), patch("builtins.print")):
                with self.assertRaises(RuntimeError):
                    bench.run_one("davinci", "t1-intervals", 0)
                rows = bench.load_rows(str(Path(tmp) / "results.jsonl"))
                self.assertEqual(len(rows), 1)
                self.assertFalse(rows[0]["pass"])
                self.assertIsNone(rows[0]["unrelated"])
                self.assertIsNone(rows[0]["artifact_leak"])
                self.assertEqual(rows[0]["stop_reason"], "change_inventory_failed")

    def test_checkpoint_identity_is_bound_to_binary_bytes(self):
        with tempfile.TemporaryDirectory() as temporary:
            binary = Path(temporary) / "fixture.exe"
            binary.write_bytes(b"public synthetic executable")
            with self.assertRaises(ValueError):
                runner.checkpoint_identity(binary)
            identity = {"schema_version": 1, "binary_sha256": runner.file_hash(binary),
                        "source_sha": "a" * 40, "dirty_diff_hash": "b" * 64}
            binary.with_suffix(".exe.identity.json").write_text(json.dumps(identity))
            self.assertEqual(runner.checkpoint_identity(binary)["source_sha"], "a" * 40)
            binary.write_bytes(b"changed")
            with self.assertRaises(ValueError):
                runner.checkpoint_identity(binary)

    def test_checkpoint_identity_allows_recorded_clean_parent(self):
        with tempfile.TemporaryDirectory() as temporary:
            binary = Path(temporary) / "parent.exe"
            binary.write_bytes(b"public parent executable")
            identity = {"schema_version": 1, "binary_sha256": runner.file_hash(binary),
                        "source_sha": "c" * 40, "dirty_diff_hash": None}
            binary.with_suffix(".exe.identity.json").write_text(json.dumps(identity))
            self.assertEqual(runner.checkpoint_identity(binary),
                             {"source_sha": "c" * 40, "dirty_diff_hash": None})

    def test_credentials_and_limits_stop_but_model_prose_does_not(self):
        for text, expected in [("Usage limit reached", "usage_limit"),
                               ("401 Unauthorized", "credentials")]:
            self.assertEqual(runner.stop_reason(json.dumps({"type": "error", "message": text}), ""), expected)
        self.assertIsNone(runner.stop_reason(json.dumps({"type": "item.completed",
            "item": {"type": "agent_message", "text": "Usage limit reached"}}), ""))

    def test_run_row_preserves_failed_process_and_full_precision(self):
        with tempfile.TemporaryDirectory() as tmp:
            measured = {"exit": 7, "stdout": "", "stderr": "",
                        "wall_s": 1.23456789, "started_at": "start", "finished_at": "end"}
            with (patch.object(bench, "RUNS", tmp),
                  patch.object(bench, "load", return_value={"prompt": "fixture", "allowed": []}),
                  patch.object(bench, "prepare", side_effect=lambda tid, dest: Path(dest).mkdir(parents=True)),
                  patch.object(bench, "changed_files", return_value=[]),
                  patch.object(bench, "grade", return_value=(True, 1, 0, "fixture")),
                  patch.object(bench, "execute", return_value=measured),
                  patch("builtins.print")):
                row = bench.run_one("davinci", "t1-intervals", 0)
                self.assertFalse(row["pass"])
                self.assertTrue(row["grader_pass"])
                self.assertEqual(row["wall_s"], 1.23456789)
                self.assertEqual(len(bench.load_rows(str(Path(tmp) / "results.jsonl"))), 1)
                with self.assertRaises(FileExistsError):
                    bench.run_one("davinci", "t1-intervals", 0)

    def test_process_measurement_and_failure_are_preserved(self):
        result = runner.execute([sys.executable, "-c", "print('fixture'); raise SystemExit(7)"],
                                Path.cwd(), {}, 10)
        self.assertEqual(result["exit"], 7)
        self.assertEqual(result["stdout"].strip(), "fixture")
        self.assertGreater(result["wall_s"], 0)
        self.assertIn("+00:00", result["started_at"])

    def test_timeout_is_preserved(self):
        result = runner.execute([sys.executable, "-c", "import time; time.sleep(10)"],
                                Path.cwd(), {}, 0.02)
        self.assertEqual(result["exit"], "timeout")

    def test_existing_campaign_is_never_reused(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp) / "campaign"
            runner.create_campaign(root, {"variant": "B0"})
            sentinel = root / "keep.txt"
            sentinel.write_text("preserved", encoding="utf-8")
            with self.assertRaises(FileExistsError):
                runner.create_campaign(root, {"variant": "V1"})
            self.assertEqual(sentinel.read_text(), "preserved")
            self.assertEqual(json.loads((root / "campaign.json").read_text()),
                             {"variant": "B0"})

    def test_isolated_settings_copy_only_auth_and_never_report_it(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            source = root / "owner"
            source.mkdir()
            (source / "auth.json").write_text('{"fixture":"synthetic-secret"}')
            (source / "settings.json").write_text('{"uncontrolled":true}')
            target = root / "isolated"
            settings = {"autoVerify": True, "decisionIntelligence": {"enabled": False}}
            record = runner.isolate_settings(source, target, settings)
            self.assertEqual(sorted(p.name for p in target.iterdir()),
                             ["auth.json", "settings.json"])
            self.assertEqual(json.loads((target / "settings.json").read_text()), settings)
            self.assertNotIn("synthetic-secret", json.dumps(record))
            self.assertEqual(record["settings"], settings)

    def test_environment_drops_inherited_product_overrides(self):
        inherited = {"PATH": "system", "DAVINCI_TOOL_SURFACE": "lean",
                     "PI_CODING_AGENT_DIR": "owner", "DAVINCI_MODEL": "wrong",
                     "OPENAI_API_KEY": "synthetic-key", "PYTHONPATH": "hostile",
                     "BENCH_CONTAINER_IMAGE": "private-image"}
        result = runner.controlled_environment(inherited, Path("isolated").resolve())
        self.assertEqual(result["PATH"], "system")
        self.assertNotIn("DAVINCI_TOOL_SURFACE", result)
        self.assertNotIn("DAVINCI_MODEL", result)
        self.assertNotIn("OPENAI_API_KEY", result)
        self.assertNotIn("PYTHONPATH", result)
        self.assertEqual(result["DAVINCI_CODING_AGENT_DIR"],
                         str(Path("isolated").resolve()))
        self.assertEqual(result["PYTHONUTF8"], "1")

    def test_grading_isolation_accepts_harness_override_and_rejects_unknown(self):
        with patch.dict("os.environ", {"BENCH_GRADING_ISOLATION": "diagnostic-only",
                                        "BENCH_DAVINCI_GRADING_ISOLATION": "container"}, clear=False):
            self.assertEqual(runner.grading_isolation("davinci"), "container")
            self.assertEqual(runner.grading_isolation("codex"), "diagnostic-only")
        with patch.dict("os.environ", {"BENCH_GRADING_ISOLATION": "unsafe"}, clear=False):
            with self.assertRaises(ValueError):
                runner.grading_isolation("davinci")

    def test_container_command_mounts_public_worktree_and_agent_only(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            workdir, agent, binary = root / "work", root / "agent", root / "davinci"
            workdir.mkdir()
            agent.mkdir()
            binary.write_bytes(b"synthetic executable")
            campaign = {
                "grading_isolation": {"davinci": "container"},
                "containers": {"davinci": {"image": "rust:fixture", "image_id": "sha256:fixture",
                                             "network": "bridge"}},
                "executables": {"davinci": str(binary)}, "agent_dir": str(agent),
            }
            env = {"PATH": "/usr/bin", "DAVINCI_CODING_AGENT_DIR": str(agent),
                   "OPENAI_API_KEY": "must-not-forward"}
            with (patch.object(runner, "_container_engine", return_value="docker"),
                  patch.object(runner, "_container_image_id", return_value="sha256:fixture")):
                command = runner.container_command([str(binary), "--mode", "json"],
                                                    workdir, env, campaign, "davinci")
            self.assertEqual(command[0:3], ["docker", "run", "--rm"])
            self.assertIn("--read-only", command)
            self.assertIn("--cap-drop=ALL", command)
            self.assertIn("type=bind,source=" + str(workdir.resolve()) + ",target=/workspace", command)
            self.assertIn("type=bind,source=" + str(agent.resolve()) + ",target=/agent,readonly", command)
            self.assertIn("type=bind,source=" + str(binary.resolve()) + ",target=/opt/harness,readonly", command)
            self.assertNotIn("OPENAI_API_KEY=must-not-forward", command)
            self.assertIn("DAVINCI_CODING_AGENT_DIR=/agent", command)
            self.assertIn("PI_CODING_AGENT_DIR=/agent", command)
            self.assertNotIn("DAVINCI_CODING_AGENT_DIR=" + str(agent.resolve()), command)
            self.assertEqual(command[-4:], ["rust:fixture", "/opt/harness", "--mode", "json"])

    def test_fixture_selection_is_explicit(self):
        self.assertEqual(runner.select_tasks("legacy", None), list(runner.LEGACY_TASKS))
        with self.assertRaises(ValueError):
            runner.select_tasks("large", None)
        with self.assertRaises(ValueError):
            runner.select_tasks("legacy", ["../outside"])
        with self.assertRaises(ValueError):
            runner.select_tasks("legacy", ["t1-intervals", "t1-intervals"])

    def test_large_selection_requires_frozen_metadata(self):
        manifest = {
            "schema_version": 1,
            "task_set": "large",
            "tasks": ["m-fixture"],
            "fixtures": {"m-fixture": {
                "task_set": "large", "allowed": ["main.py"],
                "public_verification": "python -m pytest -q",
                "public_hash": "a" * 64,
                "reference_solution_hash": "b" * 64,
                "hidden_grader_hash": "c" * 64,
            }},
        }
        self.assertEqual(runner.select_tasks("large", ["all"], manifest), ["m-fixture"])
        broken = dict(manifest, fixtures={})
        with self.assertRaises(ValueError):
            runner.select_tasks("large", ["all"], broken)


if __name__ == "__main__":
    unittest.main()
