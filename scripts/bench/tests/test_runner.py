"""Offline runner boundary tests, using public synthetic files only."""
import json
import ctypes
import os
from pathlib import Path
import sys
import tempfile
import unittest
import subprocess
import signal
import time
from unittest.mock import patch, MagicMock
from types import SimpleNamespace

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import runner
import bench
import native_supervisor

NATIVE_LIFETIME_AVAILABLE = (sys.platform == "win32"
                             or (sys.platform == "linux"
                                 and native_supervisor.children_path_available()))


class RunnerTests(unittest.TestCase):
    def repository(self, root):
        repo = root / "repo"
        repo.mkdir()
        for args in (["init", "-q"], ["config", "user.name", "Fixture"],
                     ["config", "user.email", "fixture@example.invalid"]):
            subprocess.run(["git", *args], cwd=repo, check=True, capture_output=True)
        (repo / "source.txt").write_text("committed source")
        subprocess.run(["git", "add", "source.txt"], cwd=repo, check=True)
        subprocess.run(["git", "commit", "-qm", "fixture"], cwd=repo, check=True)
        return repo

    def test_codex_priority_tier_is_an_explicit_benchmark_arm(self):
        with patch.object(bench, "SERVICE_TIER", "fast"):
            command = bench.command("codex", "prompt", "workdir")
            self.assertIn('service_tier="fast"', command)
        with patch.object(bench, "SERVICE_TIER", "default"):
            command = bench.command("codex", "prompt", "workdir")
            self.assertNotIn('service_tier="fast"', command)

    def test_prepared_fixture_contract_preserves_task_and_repository_rules(self):
        task = "Fix the real issue.\nKeep compatibility and add a regression test."
        instructions = b"Before edits create a worktree using CLAUDE_CODE_SESSION_ID.\nKeep tests.\n"
        prompts = []
        collector = MagicMock()
        collector.__enter__.return_value = collector
        collector.records, collector.spans, collector.errors, collector.error_categories = [], [], [], []
        collector.overrides.return_value = []
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            starter = root / "tasks/case/repo"
            starter.mkdir(parents=True)
            (starter.parent / "hidden").mkdir()
            (starter / "CLAUDE.md").write_bytes(instructions)
            (starter / "source.txt").write_text("original", encoding="utf-8")
            (starter.parent / "task.json").write_text(json.dumps({"prompt": task, "allowed": ["source.txt"]}))

            def execute(args, cwd, env, timeout):
                workdir = Path(cwd)
                self.assertEqual(workdir.parent.parent, root / "runs")
                self.assertTrue((workdir / ".git").is_dir())
                self.assertEqual(bench.checked_git(cwd, "status", "--porcelain").stdout, "")
                self.assertEqual((workdir / "CLAUDE.md").read_bytes(), instructions)
                prompt = args[-1]
                self.assertIn("isolated, single-owner", prompt)
                self.assertIn("setup is already complete", prompt)
                self.assertIn("Do not create another checkout or worktree", prompt)
                self.assertIn("Keep repository coding and testing instructions", prompt)
                self.assertTrue(prompt.endswith(task))
                prompts.append(prompt)
                return {"exit": 0, "stdout": "", "stderr": "", "wall_s": 0,
                        "started_at": "start", "finished_at": "end", "cleanup_complete": True}

            with (patch.object(bench, "TASKS", str(root / "tasks")),
                  patch.object(bench, "RUNS", str(root / "runs")),
                  patch.object(bench, "execute", side_effect=execute),
                  patch.object(bench, "grade", return_value=(True, 1, 0, "offline fixture")),
                  patch.object(bench, "Collector", return_value=collector),
                  patch.object(bench, "request_metrics", return_value={"request_telemetry_complete": False}),
                  patch("builtins.print")):
                for harness in ("davinci", "codex"):
                    bench.run_one(harness, "case", 0)
            self.assertEqual(prompts[0], prompts[1])
            self.assertEqual((starter / "CLAUDE.md").read_bytes(), instructions)

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
                        "started_at": "start", "finished_at": "end", "cleanup_complete": True}
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
            repo = self.repository(Path(temporary))
            with self.assertRaises(ValueError):
                runner.checkpoint_identity(binary, repo)

            source = runner.source_identity(repo)
            identity = self.green_identity(binary, source)
            binary.with_suffix(".exe.identity.json").write_text(json.dumps(identity))
            validated = runner.checkpoint_identity(binary, repo)
            self.assertEqual({key: validated[key] for key in source}, source)
            binary.write_bytes(b"changed")
            with self.assertRaises(ValueError):
                runner.checkpoint_identity(binary, repo)

    def green_identity(self, binary, source):
        ci = {"repository": "fixture/repo", "source_sha": source["source_sha"], "ci_run": 12,
              "ci_url": "https://github.com/fixture/repo/actions/runs/12", "status": "completed", "conclusion": "success",
              "event": "push", "workflow_path": ".github/workflows/ci.yml", "jobs": [{"name": name, "status": "completed", "conclusion": "success"}
              for name in sorted(runner.release_identity.EXPECTED_CI_JOBS)],
              "workflow_lint": {"status": "completed", "conclusion": "success", "run_id": 13}}
        return runner.release_identity.make_identity(binary, source, ci, None, require_tag=False)

    def test_checkpoint_refuses_provenance_without_green_ci(self):
        with tempfile.TemporaryDirectory() as temporary:
            binary = Path(temporary) / "unverified.exe"
            binary.write_bytes(b"public fixture binary")
            repo = self.repository(Path(temporary))
            source = runner.source_identity(repo)
            for schema in (2, 3):
                binary.with_suffix(".exe.identity.json").write_text(json.dumps({
                    "schema_version": schema, "binary_sha256": runner.file_hash(binary), **source}))
                with self.subTest(schema=schema), self.assertRaises(ValueError):
                    runner.checkpoint_identity(binary, repo)

    def test_checkpoint_identity_rejects_old_dirty_or_unavailable_source(self):
        with tempfile.TemporaryDirectory() as temporary:
            binary = Path(temporary) / "parent.exe"
            binary.write_bytes(b"public parent executable")
            repo = self.repository(Path(temporary))
            identity = self.green_identity(binary, runner.source_identity(repo))
            for changes in ({"schema_version": 1}, {"dirty_diff_hash": None},
                            {"dirty_diff_hash": "b" * 64}, {"source_clean": False},
                            {"source_sha": "c" * 40}, {"source_tree": "d" * 40},
                            {"build_features": ["test-fixtures"]}, {"build_features": None},
                            {"build_platform": None}):
                binary.with_suffix(".exe.identity.json").write_text(json.dumps(dict(identity, **changes)))
                with self.subTest(changes=changes), self.assertRaises(ValueError):
                    runner.checkpoint_identity(binary, repo)

    def test_credentials_and_limits_stop_but_model_prose_does_not(self):
        for text, expected in [("Usage limit reached", "usage_limit"),
                               ("401 Unauthorized", "credentials")]:
            self.assertEqual(runner.stop_reason(json.dumps({"type": "error", "message": text}), ""), expected)
        self.assertIsNone(runner.stop_reason(json.dumps({"type": "item.completed",
            "item": {"type": "agent_message", "text": "Usage limit reached"}}), ""))

    def test_run_row_preserves_failed_process_and_full_precision(self):
        with tempfile.TemporaryDirectory() as tmp:
            measured = {"exit": 7, "stdout": "", "stderr": "",
                        "wall_s": 1.23456789, "started_at": "start", "finished_at": "end", "cleanup_complete": True}
            with (patch.object(bench, "RUNS", tmp),
                  patch.object(bench, "load", return_value={"prompt": "fixture", "allowed": []}),
                  patch.object(bench, "prepare", side_effect=lambda tid, dest: Path(dest).mkdir(parents=True)),
                  patch.object(bench, "changed_files", return_value=[]),
                  patch.object(bench, "forbidden_artifacts", return_value={"transaction_leak": False, "artifact_leak": False}),
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

    @unittest.skipUnless(NATIVE_LIFETIME_AVAILABLE, "matching Linux /proc child ownership view required")
    def test_process_measurement_and_failure_are_preserved(self):
        result = runner.execute([sys.executable, "-c", "print('fixture'); raise SystemExit(7)"],
                                Path.cwd(), {}, 10)
        self.assertEqual(result["exit"], 7)
        self.assertEqual(result["stdout"].strip(), "fixture")
        self.assertGreater(result["wall_s"], 0)
        self.assertIn("+00:00", result["started_at"])
        self.assertTrue(result["cleanup_complete"])

    @unittest.skipUnless(NATIVE_LIFETIME_AVAILABLE, "matching Linux /proc child ownership view required")
    def test_timeout_is_preserved(self):
        result = runner.execute([sys.executable, "-c", "import time; time.sleep(10)"],
                                Path.cwd(), {}, 0.02)
        self.assertEqual(result["exit"], "timeout")
        self.assertTrue(result["cleanup_complete"])

    @unittest.skipUnless(NATIVE_LIFETIME_AVAILABLE and sys.platform == "linux",
                         "fork/setsid fixture; test_windows_job covers Windows descendants")
    def test_successful_native_launcher_cannot_leave_detached_writers(self):
        for detach in (False, True):
            with self.subTest(detach=detach), tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                marker, pidfile = root / "late-write", root / "child.pid"
                child = ("import os,time; from pathlib import Path; "
                         + ("pid=os.fork(); os._exit(0) if pid else os.setsid(); " if detach else "")
                         + f"Path({str(pidfile)!r}).write_text(str(os.getpid())); time.sleep(.5); "
                         + f"Path({str(marker)!r}).write_text('late')")
                launcher = ("import subprocess,sys,time; from pathlib import Path; "
                            f"p=subprocess.Popen([sys.executable,'-c',{child!r}], "
                            "stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL); "
                            f"path=Path({str(pidfile)!r}); deadline=time.monotonic()+3; "
                            "\nwhile not path.exists() and time.monotonic()<deadline: time.sleep(.01)\n"
                            "assert path.exists()")
                result = runner.execute([sys.executable, "-c", launcher], root, dict(os.environ), 5)
                self.assertEqual(result["exit"], 0, result)
                self.assertTrue(result["cleanup_complete"], result)
                self.assertFalse(marker.exists())
                with self.assertRaises(ProcessLookupError):
                    os.kill(int(pidfile.read_text()), 0)

    def test_native_owner_refuses_to_launch_without_its_scoped_kernel_view(self):
        with (patch.object(native_supervisor, "children_path_available", return_value=False),
              patch.object(native_supervisor.subprocess, "Popen") as start):
            result = native_supervisor.supervise(["unused"], 1)
        self.assertFalse(result["cleanup_complete"])
        self.assertEqual(result["exit"], "unsupported_native_lifecycle")
        start.assert_not_called()

    @unittest.skipIf(NATIVE_LIFETIME_AVAILABLE, "this environment supports native supervision")
    def test_actual_unsupported_native_owner_never_runs_the_benchmark_command(self):
        with tempfile.TemporaryDirectory() as tmp:
            marker = Path(tmp) / "should-not-run"
            command = [sys.executable, "-c", f"from pathlib import Path; Path({str(marker)!r}).touch()"]
            result = runner.execute(command, tmp, dict(os.environ), 1)
            self.assertEqual(result["exit"], "unsupported_native_lifecycle", result)
            self.assertFalse(result["cleanup_complete"])
            self.assertFalse(marker.exists())

    def test_native_preflight_checks_ownership_before_launch_or_dirty_intent(self):
        with (patch.object(runner.sys, "platform", "linux"),
              patch.object(native_supervisor, "children_path_available", return_value=False),
              patch.object(runner, "execute") as start):
            with self.assertRaisesRegex(ValueError, "PID namespace before any launch"):
                runner.tooling_preflight()
        start.assert_not_called()

    @unittest.skipUnless(sys.platform == "linux", "waitpid/WNOHANG reaping is Linux-only")
    def test_native_cleanup_proves_echild_and_only_signals_owned_children(self):
        with (patch.object(native_supervisor.os, "waitpid", side_effect=[(0, 0), (123, 0), ChildProcessError]),
              patch.object(native_supervisor.Path, "read_text", return_value="123"),
              patch.object(native_supervisor.os, "kill") as kill,
              patch.object(native_supervisor.time, "sleep")):
            self.assertTrue(native_supervisor.reap_descendants())
        kill.assert_called_once_with(123, signal.SIGKILL)
        with (patch.object(native_supervisor.os, "waitpid", return_value=(0, 0)),
              patch.object(native_supervisor.Path, "read_text", side_effect=OSError("unavailable")),
              patch.object(native_supervisor.os, "kill") as kill):
            self.assertFalse(native_supervisor.reap_descendants())
        kill.assert_not_called()

    def test_abrupt_native_supervisor_exit_has_no_cleanup_proof(self):
        process = MagicMock(returncode=-9)
        process.communicate.return_value = ("", "")
        with (patch.object(runner.sys, "platform", "linux"),
              patch.object(runner.subprocess, "Popen", return_value=process)):
            result = runner.execute(["unused"], Path.cwd(), {}, 1)
        self.assertFalse(result["cleanup_complete"])
        self.assertEqual(result["exit"], "supervisor_failed")

    def test_native_supervisor_watchdog_preserves_ungraded_failure(self):
        process = MagicMock(returncode=0)
        process.communicate.side_effect = [subprocess.TimeoutExpired("supervisor", 16), ("partial", "")]
        with (patch.object(runner.sys, "platform", "linux"),
              patch.object(runner.subprocess, "Popen", return_value=process)):
            result = runner.execute(["unused"], Path.cwd(), {}, 1)
        self.assertEqual(result["exit"], "supervisor_timeout")
        self.assertEqual(result["stdout"], "partial")
        self.assertFalse(result["cleanup_complete"])
        process.terminate.assert_called_once()

    def test_unsupported_native_lifecycle_does_not_launch_a_process(self):
        with patch.object(runner.sys, "platform", "darwin"), patch.object(runner.subprocess, "Popen") as start:
            result = runner.execute(["unused"], Path.cwd(), {}, 1)
        self.assertEqual(result["exit"], "unsupported_native_lifecycle")
        self.assertFalse(result["cleanup_complete"])
        start.assert_not_called()

    def test_container_timeout_removes_named_container_before_client_reap(self):
        events = []
        process = MagicMock(pid=1234, returncode=None)
        def communicate(timeout):
            events.append("communicate")
            if events.count("communicate") == 1:
                raise subprocess.TimeoutExpired("docker", timeout)
            return "partial transcript", ""
        process.communicate.side_effect = communicate
        with (tempfile.TemporaryDirectory() as tmp,
              patch.object(runner.subprocess, "Popen", return_value=process),
              patch.object(runner, "_remove_container", side_effect=lambda target: events.append(target) or True),
              patch.object(runner, "_kill_process", side_effect=lambda p: events.append("kill client"))):
            cidfile = Path(tmp) / "container.cid"
            cidfile.write_text("a" * 64)
            result = runner.execute(["docker", "run", "--name", "davinci-bench-fixture",
                                    "--cidfile", str(cidfile), "image"],
                                    Path.cwd(), {}, 0.01)
        self.assertEqual(events, ["communicate", ("docker", "davinci-bench-fixture"), "kill client", "communicate",
                                  ("docker", "davinci-bench-fixture")])
        self.assertEqual(result["exit"], "timeout")
        self.assertTrue(result["cleanup_complete"])

    def test_container_client_death_needs_creation_acknowledgment_before_grading(self):
        with tempfile.TemporaryDirectory() as tmp:
            cidfile = Path(tmp) / "container.cid"
            for code in (-9, 125, 0):
                for content in (None, "", "invalid", "a" * 64):
                    with self.subTest(code=code, content=content):
                        if cidfile.exists():
                            cidfile.unlink()
                        if content is not None:
                            cidfile.write_text(content)
                        process = MagicMock(returncode=code)
                        process.communicate.return_value = ("", "")
                        with (patch.object(runner.subprocess, "Popen", return_value=process),
                              patch.object(runner, "_remove_container", return_value=True)):
                            result = runner.execute(["docker", "run", "--name", "davinci-bench-fixture",
                                "--cidfile", str(cidfile), "image"], Path.cwd(), {}, 1)
                        self.assertEqual(result["cleanup_complete"], content == "a" * 64)
                        self.assertEqual(result["exit"], code)

    def test_container_cleanup_does_not_confuse_daemon_error_with_absence(self):
        for error, expected in (("Error: No such container: davinci-bench-fixture", True),
                                ("Cannot connect to the Docker daemon", False)):
            with patch.object(runner.subprocess, "run", side_effect=[
                    SimpleNamespace(returncode=1), SimpleNamespace(returncode=1, stderr=error)]) as run:
                self.assertEqual(runner._remove_container(("docker", "davinci-bench-fixture")), expected)
                self.assertEqual(run.call_args_list[0].args[0], ["docker", "rm", "--force", "davinci-bench-fixture"])

    def test_cleanup_failure_stops_before_inspection_or_grading_and_preserves_row(self):
        with tempfile.TemporaryDirectory() as tmp:
            measured = {"exit": "timeout", "stdout": "", "stderr": "", "wall_s": 2,
                        "started_at": "start", "finished_at": "end", "cleanup_complete": False}
            with (patch.object(bench, "RUNS", tmp),
                  patch.object(bench, "load", return_value={"prompt": "fixture", "allowed": []}),
                  patch.object(bench, "prepare", side_effect=lambda tid, dest: Path(dest).mkdir(parents=True)),
                  patch.object(bench, "changed_files") as inspect,
                  patch.object(bench, "grade") as grade,
                  patch.object(bench, "execute", return_value=measured), patch("builtins.print")):
                with self.assertRaisesRegex(RuntimeError, "cleanup_failed"):
                    bench.run_one("davinci", "t1-intervals", 0)
                inspect.assert_not_called()
                grade.assert_not_called()
                rows = bench.load_rows(str(Path(tmp) / "results.jsonl"))
                self.assertFalse(rows[0]["cleanup_complete"])
                self.assertFalse(rows[0]["pass"])

    def test_tooling_preflight_fails_before_live_usage(self):
        failed = {"exit": 127, "stdout": "", "cleanup_complete": True}
        with patch.object(runner, "execute", return_value=failed) as execute:
            with self.assertRaisesRegex(ValueError, "prerequisites missing"):
                runner.tooling_preflight("docker", "sha256:fixture")
            command = execute.call_args.args[0]
            self.assertIn("sha256:fixture", command)
            self.assertIn("--network", command)
            self.assertIn("python -m pytest --version", command[-1])
            self.assertIn("python3 -m pytest --version", command[-1])

    def test_machine_lock_excludes_other_campaign_directories_and_releases(self):
        with (tempfile.TemporaryDirectory() as tmp,
              patch.object(runner, "_campaign_lock_path", return_value=Path(tmp) / "machine.lock")):
            with runner.campaign_lock("campaign-a"):
                with self.assertRaisesRegex(RuntimeError, "another benchmark campaign"):
                    with runner.campaign_lock("campaign-b"):
                        self.fail("overlapping campaign admitted")
            with runner.campaign_lock("campaign-b") as record:
                self.assertEqual(record["scope"], "machine")

    def test_unproven_cleanup_blocks_another_launch_and_the_next_campaign(self):
        with (tempfile.TemporaryDirectory() as tmp,
              patch.object(runner, "_campaign_lock_path", return_value=Path(tmp) / "machine.lock"),
              patch.object(runner, "_execute_native", return_value={"cleanup_complete": False})):
            with runner.campaign_lock("campaign-a"):
                self.assertFalse(runner.execute(["unused"], tmp, {}, 1)["cleanup_complete"])
                with self.assertRaisesRegex(RuntimeError, "unresolved process cleanup"):
                    runner.execute(["unused"], tmp, {}, 1)
            with self.assertRaisesRegex(RuntimeError, "unresolved benchmark campaign ownership"):
                with runner.campaign_lock("campaign-b"):
                    self.fail("unproven cleanup admitted")

    def test_proven_cleanup_clears_durable_launch_intent(self):
        with (tempfile.TemporaryDirectory() as tmp,
              patch.object(runner, "_campaign_lock_path", return_value=Path(tmp) / "machine.lock"),
              patch.object(runner, "_execute_native", return_value={"cleanup_complete": True})):
            lockfile = Path(tmp) / "machine.lock"
            with runner.campaign_lock("campaign-a"):
                self.assertTrue(runner.execute(["unused"], tmp, {}, 1)["cleanup_complete"])
                self.assertEqual(json.loads(lockfile.read_text())["in_flight"], [])
            self.assertEqual(json.loads(lockfile.read_text())["status"], "clean")
            with runner.campaign_lock("campaign-b"):
                pass

    def test_corrupt_and_legacy_lock_ownership_cannot_be_treated_as_clean(self):
        with tempfile.TemporaryDirectory() as tmp:
            lockfile = Path(tmp) / "machine.lock"
            for record in ("{", '{"pid":123,"campaign":"legacy"}',
                           '{"schema_version":1,"status":"active","in_flight":[]}',
                           '{"schema_version":1,"status":"clean","in_flight":[{}]}'):
                lockfile.write_text(record)
                with (self.subTest(record=record),
                      patch.object(runner, "_campaign_lock_path", return_value=lockfile),
                      self.assertRaisesRegex(RuntimeError, "unresolved benchmark campaign ownership")):
                    with runner.campaign_lock("new-campaign"):
                        self.fail("ambiguous ownership admitted")

    def test_dead_owner_is_reconciled_only_when_its_work_is_proven_gone(self):
        def record(kind, container=None):
            return json.dumps({"schema_version": 1, "pid": 424242, "status": "active",
                               "in_flight": [{"id": "x", "kind": kind, "container": container}]})
        cases = [
            (record("windows-job"), False, None, True),
            (record("windows-job"), True, None, False),   # owner still alive
            (record("native"), False, None, False),       # Linux native needs an operator
            (record("container", ["docker", "davinci-bench-abc"]), False, True, True),
            (record("container", ["docker", "davinci-bench-abc"]), False, False, False),
            (record("container", ["docker", "other-name"]), False, True, False),
            (json.dumps({"schema_version": 1, "pid": 424242, "status": "active",
                         "in_flight": []}), False, None, True),
        ]
        with tempfile.TemporaryDirectory() as tmp:
            lockfile = Path(tmp) / "machine.lock"
            for text, alive, removed, admitted in cases:
                lockfile.write_text(text)
                with (self.subTest(text=text, alive=alive, removed=removed),
                      patch.object(runner, "_campaign_lock_path", return_value=lockfile),
                      patch.object(runner, "_pid_alive", return_value=alive),
                      patch.object(runner, "_remove_container", return_value=removed) as remove):
                    if admitted:
                        with runner.campaign_lock("next"):
                            pass
                        self.assertEqual(json.loads(lockfile.read_text())["status"], "clean")
                    else:
                        with self.assertRaisesRegex(RuntimeError, "unresolved benchmark campaign ownership"):
                            with runner.campaign_lock("next"):
                                self.fail("unproven ownership admitted")
                    if removed is not None and "davinci-bench-" in text:
                        remove.assert_called_once_with(("docker", "davinci-bench-abc"))

    def test_fixture_baseline_ignores_operator_git_hooks_and_reports_git_failures(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "tasks" / "t0" / "repo").mkdir(parents=True)
            (root / "tasks" / "t0" / "repo" / "app.py").write_text("x = 1\n")
            hooks = root / "global-hooks"
            hooks.mkdir()
            (hooks / "pre-commit").write_text("#!/bin/sh\nexit 1\n", newline="\n")
            (hooks / "pre-commit").chmod(0o755)
            global_config = root / "gitconfig"
            global_config.write_text(f"[core]\n\thooksPath = {hooks.as_posix()}\n")
            env = {**os.environ, "GIT_CONFIG_GLOBAL": str(global_config)}
            with (patch.object(bench, "TASKS", str(root / "tasks")),
                  patch.dict(os.environ, env)):
                bench.prepare("t0", str(root / "work"))
                self.assertFalse(bench.changed_files(str(root / "work")))
                log = bench.git(str(root / "work"), "log", "--oneline")
                self.assertEqual(len(log.stdout.splitlines()), 1)
                with patch.object(bench, "git", return_value=SimpleNamespace(
                        returncode=128, stdout="", stderr="fatal: boom")):
                    with self.assertRaisesRegex(RuntimeError, "fixture setup failed: git init: fatal: boom"):
                        bench.prepare("t0", str(root / "work2"))

    def test_lock_record_stays_valid_json_when_it_shrinks(self):
        with tempfile.TemporaryDirectory() as tmp:
            lockfile = Path(tmp) / "machine.lock"
            with patch.object(runner, "_campaign_lock_path", return_value=lockfile):
                with runner.campaign_lock("campaign-" + "x" * 400):
                    self.assertEqual(json.loads(lockfile.read_text())["status"], "active")
                with runner.campaign_lock("b"):
                    pass
            self.assertEqual(json.loads(lockfile.read_text())["campaign"],
                             str(Path("b").resolve()))

    def test_compare_passes_all_control_rows_to_overlap_gate(self):
        def timed(harness, start, end):
            return {"harness": harness, "task": "t1-intervals", "rep": 0, "pass": True,
                    "wall_s": 1, "started_at": f"2026-09-27T00:{start}:00+00:00",
                    "finished_at": f"2026-09-27T00:{end}:00+00:00"}
        parent = [timed("davinci", "00", "01"), timed("codex", "01", "05")]
        candidate = [timed("davinci", "03", "04"), timed("codex", "05", "06")]
        with tempfile.TemporaryDirectory() as tmp:
            roots = [Path(tmp) / name for name in ("parent", "candidate")]
            for root in roots:
                root.mkdir()
                (root / "campaign.json").write_text("{}")
            with (patch.object(bench, "load_rows", side_effect=[parent, candidate]),
                  patch.object(bench, "campaign_errors", return_value=[]),
                  patch.object(bench, "paired_report", return_value={"all_run_latency_uncertainty": {"method": "fixture"}}),
                  patch.object(bench, "summarize", return_value={}),
                  patch.object(bench, "stratified_summary", return_value={}),
                  patch("builtins.print") as output):
                self.assertEqual(bench.compare(*roots), 1)
            result = json.loads(output.call_args.args[0])
            self.assertEqual(result["promotion"]["rules"]["sequential_campaigns"]["status"], "fail")

    @unittest.skipUnless(sys.platform == "linux", "Linux crash/reaping regression")
    def test_orchestrator_crash_keeps_durable_launch_ownership(self):
        libc = ctypes.CDLL(None)
        prior = ctypes.c_int()
        self.assertEqual(libc.prctl(37, ctypes.byref(prior), 0, 0, 0), 0)
        self.assertEqual(libc.prctl(36, 1, 0, 0, 0), 0)
        try:
            with tempfile.TemporaryDirectory() as tmp:
                root = Path(tmp)
                lockfile, ready = root / "machine.lock", root / "worker.json"
                worker = ("import os,time,json; from pathlib import Path; "
                          f"p=Path({str(ready)!r}); temporary=p.with_suffix('.tmp'); "
                          "temporary.write_text(json.dumps(os.getpid())); temporary.replace(p); time.sleep(30)")
                launcher = ("import os,sys,subprocess; from pathlib import Path; "
                            f"sys.path.insert(0,{str(Path(runner.__file__).parent)!r}); import runner; "
                            f"runner._campaign_lock_path=lambda:Path({str(lockfile)!r}); "
                            "\ndef fixture_execute(command,workdir,env,timeout):\n"
                            " p=subprocess.Popen(command,cwd=workdir,env=env,start_new_session=True,"
                            "stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)\n"
                            " p.wait(timeout=timeout)\n"
                            " return {'cleanup_complete':True}\n"
                            "runner._execute_native=fixture_execute\n"
                            f"\nwith runner.campaign_lock({str(root / 'first')!r}):\n"
                            f" runner.execute([sys.executable,'-c',{worker!r}],{str(root)!r},dict(os.environ),60)\n")
                outer = subprocess.Popen([sys.executable, "-c", launcher], stdout=subprocess.PIPE,
                                         stderr=subprocess.PIPE, start_new_session=True)
                worker_pid = None
                try:
                    deadline = time.monotonic() + 5
                    while not ready.exists() and outer.poll() is None and time.monotonic() < deadline:
                        time.sleep(.01)
                    self.assertTrue(ready.exists(), "fixture worker did not start")
                    worker_pid = json.loads(ready.read_text())
                    outer.kill()
                    outer.communicate(timeout=3)
                    os.kill(worker_pid, 0)
                    with patch.object(runner, "_campaign_lock_path", return_value=lockfile):
                        with self.assertRaisesRegex(RuntimeError, "unresolved benchmark campaign ownership"):
                            with runner.campaign_lock(root / "second"):
                                self.fail("orphan campaign overlap admitted")
                    self.assertTrue(json.loads(lockfile.read_text())["in_flight"])
                finally:
                    if outer.poll() is None:
                        outer.kill()
                        outer.communicate(timeout=3)
                    if worker_pid is not None:
                        try:
                            os.kill(worker_pid, signal.SIGKILL)
                        except ProcessLookupError:
                            pass
                        # Reap the fixture child adopted after its orchestrator
                        # was killed, leaving no orphan/zombie from this test.
                        os.waitpid(worker_pid, 0)
        finally:
            libc.prctl(36, prior.value, 0, 0, 0)

    def test_source_identity_rejects_tracked_and_untracked_build_inputs(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo = self.repository(Path(tmp))
            self.assertTrue(runner.source_identity(repo)["source_clean"])
            (repo / "new-source.py").write_text("new source")
            with self.assertRaisesRegex(ValueError, "clean committed"):
                runner.source_identity(repo)
            (repo / "new-source.py").unlink()
            (repo / "source.txt").write_text("dirty source")
            with self.assertRaisesRegex(ValueError, "clean committed"):
                runner.source_identity(repo)

    def test_parent_control_must_equal_actual_merge_base_not_old_ancestor(self):
        with tempfile.TemporaryDirectory() as tmp:
            repo = self.repository(Path(tmp))
            old = runner.source_identity(repo)["source_sha"]
            (repo / "source.txt").write_text("target advanced")
            subprocess.run(["git", "commit", "-qam", "target advance"], cwd=repo, check=True)
            actual = runner.source_identity(repo)["source_sha"]
            subprocess.run(["git", "branch", "fixture-base"], cwd=repo, check=True)
            (repo / "source.txt").write_text("candidate")
            subprocess.run(["git", "commit", "-qam", "candidate"], cwd=repo, check=True)
            candidate = runner.source_identity(repo)["source_sha"]
            self.assertEqual(runner.parent_identity(repo, candidate, "fixture-base", actual)["merge_base_sha"], actual)
            with self.assertRaisesRegex(ValueError, "actual merge-base"):
                runner.parent_identity(repo, candidate, "fixture-base", old)

    def test_artifact_inventory_is_distinct_from_transaction_and_sees_ignored_paths(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            work, tasks = root / "work", root / "tasks"
            work.mkdir()
            hidden = tasks / "synthetic" / "hidden"
            hidden.mkdir(parents=True)
            (hidden / "test_private.py").write_text("assert 1 == 1")
            transaction = work / ".davinci-transactions"
            transaction.mkdir()
            with patch.object(bench, "TASKS", str(tasks)):
                result = bench.forbidden_artifacts(work, "synthetic")
                self.assertTrue(result["transaction_leak"])
                self.assertFalse(result["artifact_leak"])
                transaction.rmdir()
                (work / "test_private.py").write_text("copied before grading")
                result = bench.forbidden_artifacts(work, "synthetic")
                self.assertTrue(result["artifact_leak"])
                self.assertFalse(result["transaction_leak"])
                self.assertEqual(result["artifact_paths"], ["test_private.py"])

    def test_hidden_file_shipped_in_the_public_repo_is_not_an_artifact(self):
        # Regression: t8 ships test_calc.py publicly and in hidden/, so the
        # untouched starter file stopped every campaign as forbidden_artifact.
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            work, tasks = root / "work", root / "tasks"
            work.mkdir()
            hidden, public = tasks / "synthetic" / "hidden", tasks / "synthetic" / "repo"
            hidden.mkdir(parents=True)
            public.mkdir(parents=True)
            (hidden / "test_calc.py").write_text("hidden version")
            (hidden / "test_hidden_calc.py").write_text("private")
            (public / "test_calc.py").write_text("public version")
            (work / "test_calc.py").write_text("agent edited the starter tests")
            with patch.object(bench, "TASKS", str(tasks)):
                result = bench.forbidden_artifacts(work, "synthetic")
                self.assertFalse(result["artifact_leak"], result)
                (work / "test_hidden_calc.py").write_text("copied before grading")
                result = bench.forbidden_artifacts(work, "synthetic")
                self.assertEqual(result["artifact_paths"], ["test_hidden_calc.py"])

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
            settings = {"autoVerify": True, "effortPolicy": "fixed"}
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
            self.assertEqual(command[0:3], ["docker", "run", "--name"])
            self.assertTrue(command[command.index("--name") + 1].startswith("davinci-bench-"))
            self.assertIn("--cidfile", command)
            self.assertIn("--read-only", command)
            self.assertIn("--cap-drop=ALL", command)
            self.assertIn("type=bind,source=" + str(workdir.resolve()) + ",target=/workspace", command)
            self.assertIn("type=bind,source=" + str(agent.resolve()) + ",target=/agent,readonly", command)
            self.assertIn("type=bind,source=" + str(binary.resolve()) + ",target=/opt/harness,readonly", command)
            self.assertNotIn("OPENAI_API_KEY=must-not-forward", command)
            self.assertIn("DAVINCI_CODING_AGENT_DIR=/agent", command)
            self.assertIn("PI_CODING_AGENT_DIR=/agent", command)
            self.assertNotIn("DAVINCI_CODING_AGENT_DIR=" + str(agent.resolve()), command)
            self.assertEqual(command[-4:], ["sha256:fixture", "/opt/harness", "--mode", "json"])

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
            "hash_order": "relative-posix-codepoint-v1",
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
