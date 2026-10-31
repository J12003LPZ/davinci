"""Offline tests for the study task sources, Droid arm identity, warm-up rows and task selection."""
import importlib.util
import json
import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

SPEC = importlib.util.spec_from_file_location("bench", Path(__file__).resolve().parents[1] / "bench.py")
bench = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(bench)

import campaign  # noqa: E402
import droid_arm  # noqa: E402
import droid_usage  # noqa: E402
import task_sources  # noqa: E402
from test_campaign import row as campaign_row  # noqa: E402


def fake_verified(root, command="cargo test --quiet", expected_exit=0):
    """A one-scenario corpus plus its fixture directory under ``root``."""
    repos = Path(root) / "repos"
    fixture = repos / "fx-fixture"
    (fixture / "src").mkdir(parents=True, exist_ok=True)
    (fixture / "src" / "lib.rs").write_text("pub fn f() -> i32 { 1 }\n")
    (fixture / "Cargo.toml").write_text("[package]\nname = \"fx\"\n")
    corpus = Path(root) / "verified-core.json"
    corpus.write_text(json.dumps([{
        "id": "verified-range", "request": "Fix f.", "repo_fixture": "fx-fixture",
        "requirements": [{"type": "file_changed", "path": "src/lib.rs"}],
        "verification_commands": [{"command": command, "timeout_seconds": 77,
                                   "expected_exit": expected_exit}]}]))
    return corpus, repos


class VerifiedCoreTests(unittest.TestCase):
    def test_real_corpus_parses_to_the_six_study_tasks(self):
        scenarios = json.loads(task_sources.CORPUS.read_text(encoding="utf-8"))
        self.assertEqual([s["id"] for s in scenarios], list(task_sources.VERIFIED))
        self.assertEqual(len(scenarios), 6)
        for tid in task_sources.VERIFIED:
            spec = task_sources.load_spec(tid)
            self.assertTrue(spec["prompt"])
            self.assertEqual(spec["allowed"], ["src/lib.rs"])
            self.assertTrue(task_sources.fixture_files(tid))

    def test_prepare_baseline_ignores_build_output_and_flags_real_edits(self):
        with tempfile.TemporaryDirectory() as tmp:
            corpus, repos = fake_verified(tmp)
            dest = Path(tmp) / "ws"
            with patch.object(task_sources, "CORPUS", corpus), patch.object(task_sources, "REPOS", repos):
                bench.prepare("verified-range", str(dest))
                self.assertTrue((dest / ".git").is_dir())
                self.assertEqual(bench.changed_files(str(dest)), [])
                (dest / "target").mkdir()
                (dest / "target" / "debug.o").write_text("x")
                (dest / "Cargo.lock").write_text("lock")
                self.assertEqual(bench.changed_files(str(dest)), [])
                (dest / "src" / "lib.rs").write_text("pub fn f() -> i32 { 2 }\n")
                (dest / "extra.txt").write_text("x")
                self.assertEqual(bench.changed_files(str(dest)), ["extra.txt", "src/lib.rs"])
                self.assertEqual(bench.load("verified-range")["allowed"], ["src/lib.rs"])

    def test_grade_runs_each_command_with_its_timeout_and_expected_exit(self):
        calls = []

        def fake_check(command, workdir, timeout=120):
            calls.append((command, timeout))
            return {"pass": True, "exit": 0, "cleanup_complete": True,
                    "stdout": "test result: ok. 3 passed\n", "stderr": ""}

        with tempfile.TemporaryDirectory() as tmp:
            corpus, repos = fake_verified(tmp)
            workdir = Path(tmp) / "ws"
            workdir.mkdir()
            with (patch.object(task_sources, "CORPUS", corpus), patch.object(task_sources, "REPOS", repos),
                  patch.object(bench, "run_check", side_effect=fake_check)):
                ok, passed, failed, tail = bench.grade("verified-range", str(workdir))
            self.assertEqual((ok, passed, failed), (True, 1, 0))
            self.assertEqual(tail, "test result: ok. 3 passed")
            self.assertEqual(calls, [(["cargo", "test", "--quiet"], 77)])
            # A nonzero exit that differs from expected_exit fails the task.
            corpus, repos = fake_verified(tmp)
            with (patch.object(task_sources, "CORPUS", corpus), patch.object(task_sources, "REPOS", repos),
                  patch.object(bench, "run_check", return_value={
                      "pass": False, "exit": 101, "cleanup_complete": True, "stdout": "", "stderr": "boom\n"})):
                ok, passed, failed, tail = bench.grade("verified-range", str(workdir))
            self.assertEqual((ok, passed, failed, tail), (False, 0, 1, "boom"))

    def test_forbidden_artifacts_needs_no_hidden_dir_for_study_tasks(self):
        with tempfile.TemporaryDirectory() as tmp:
            (Path(tmp) / "ws").mkdir()
            with patch.object(bench, "TASKS", str(Path(tmp) / "tasks")):
                found = bench.forbidden_artifacts(str(Path(tmp) / "ws"), "verified-range")
            self.assertFalse(found["artifact_leak"])


class ResearchFixtureTests(unittest.TestCase):
    def make_root(self, tmp):
        root = Path(tmp) / "fixtures"
        (root / "tasks").mkdir(parents=True)
        (root / "setup.ps1").write_text("# setup")
        (root / "grade.ps1").write_text("# grade")
        (root / "tasks" / "01-read-many-filter.md").write_text(
            "# 01\n\n## Setup\n\nignore\n\n## Instruction (give this verbatim)\n\n"
            "Find tokens.\nWrite answer.json.\n\n## Acceptance\n\nnope\n")
        return root

    def test_prompt_comes_from_the_instruction_section_only(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = self.make_root(tmp)
            with patch.dict(os.environ, {"BENCH_RESEARCH_FIXTURES": str(root)}):
                spec = bench.load("research-01")
                self.assertEqual(spec["prompt"], "Find tokens.\nWrite answer.json.")
                self.assertEqual(spec["allowed"], ["answer.json"])
                self.assertEqual(sorted(task_sources.fixture_files("research-01")),
                                 ["grade.ps1", "setup.ps1", "tasks/01-read-many-filter.md"])

    def test_prepare_calls_setup_then_baselines_the_pre_manifest(self):
        seen = []
        real_run = task_sources.subprocess.run

        def fake_run(command, **kwargs):
            if "-Path" not in command:  # git calls pass through to the real runner
                return real_run(command, **kwargs)
            seen.append(command)
            dest = Path(command[command.index("-Path") + 1])
            (dest / ".pre").mkdir(parents=True)
            (dest / ".pre" / "manifest.json").write_text("[]")
            (dest / "notes").mkdir()
            (dest / "notes" / "note-000.md").write_text("alpha")
            class Done:
                returncode = 0
            return Done()

        with tempfile.TemporaryDirectory() as tmp:
            root = self.make_root(tmp)
            dest = Path(tmp) / "ws"
            with (patch.dict(os.environ, {"BENCH_RESEARCH_FIXTURES": str(root), "BENCH_PWSH": sys.executable}),
                  patch.object(task_sources.subprocess, "run", side_effect=fake_run)):
                bench.prepare("research-01", str(dest))
            self.assertEqual(seen[0][seen[0].index("-Task") + 1], "01-read-many-filter")
            self.assertFalse(dest.joinpath("answer.json").exists())
            # The fixture's own .pre/ files are in the baseline, so they are not "changed".
            self.assertEqual(bench.changed_files(str(dest)), [])
            (dest / "answer.json").write_text("{}")
            self.assertEqual(bench.changed_files(str(dest)), ["answer.json"])

    def test_grade_parses_the_json_verdict_and_fails_loudly_otherwise(self):
        verdict = json.dumps({"task": "01", "pass": True, "passed": 3, "total": 3, "assertions": []})

        def checked(stdout, exit_code=0, cleanup=True):
            return {"pass": exit_code == 0, "exit": exit_code, "cleanup_complete": cleanup,
                    "stdout": stdout, "stderr": ""}

        with tempfile.TemporaryDirectory() as tmp:
            root = self.make_root(tmp)
            workdir = Path(tmp) / "ws"
            workdir.mkdir()
            env = {"BENCH_RESEARCH_FIXTURES": str(root), "BENCH_PWSH": sys.executable}
            with patch.dict(os.environ, env):
                with patch.object(bench, "run_check", return_value=checked("WARN\n" + verdict)):
                    self.assertEqual(bench.grade("research-01", str(workdir)), (True, 3, 0, "3/3 assertions"))
                failing = json.dumps({"pass": False, "passed": 1, "total": 3})
                with patch.object(bench, "run_check", return_value=checked(failing, 1)):
                    self.assertEqual(bench.grade("research-01", str(workdir)), (False, 1, 2, "1/3 assertions"))
                with patch.object(bench, "run_check", return_value=checked("", 2)):
                    with self.assertRaisesRegex(RuntimeError, "exit 2"):
                        bench.grade("research-01", str(workdir))
                with patch.object(bench, "run_check", return_value=checked("no json here", 0)):
                    with self.assertRaisesRegex(RuntimeError, "no verdict"):
                        bench.grade("research-01", str(workdir))

    def test_missing_root_or_pwsh_fails_loudly(self):
        with tempfile.TemporaryDirectory() as tmp:
            with patch.dict(os.environ, {"BENCH_RESEARCH_FIXTURES": str(Path(tmp) / "absent")}):
                with self.assertRaisesRegex(RuntimeError, "research fixtures root"):
                    bench.load("research-02")
            root = self.make_root(tmp)
            with patch.dict(os.environ, {"BENCH_RESEARCH_FIXTURES": str(root),
                                         "BENCH_PWSH": str(Path(tmp) / "no-such-pwsh")}):
                with self.assertRaisesRegex(RuntimeError, "pwsh is required"):
                    bench.prepare("research-01", str(Path(tmp) / "ws"))


class SelectionTests(unittest.TestCase):
    def test_study_set_is_seventeen_and_all_stays_public(self):
        self.assertEqual(len(task_sources.STUDY_TASKS), 17)
        self.assertEqual(task_sources.select_study(["study"]), list(task_sources.STUDY_TASKS))
        self.assertIsNone(task_sources.select_study(["all"]))
        self.assertIsNone(task_sources.select_study(["t1-intervals"]))
        self.assertEqual(task_sources.select_study(["verified-path", "t1-intervals"]),
                         ["verified-path", "t1-intervals"])
        with self.assertRaises(ValueError):
            task_sources.select_study(["verified-path", "verified-path"])
        with self.assertRaises(ValueError):
            task_sources.select_study(["verified-path", "t9-unknown"])
        self.assertEqual(bench.task_ids(), list(campaign.LEGACY_TASKS))

    def test_fixture_manifest_covers_mixed_sets_and_keeps_public_only_unchanged(self):
        with tempfile.TemporaryDirectory() as tmp:
            tasks = Path(tmp) / "tasks"
            (tasks / "t1-intervals" / "repo").mkdir(parents=True)
            (tasks / "t1-intervals" / "repo" / "a.py").write_text("x")
            corpus, repos = fake_verified(tmp)
            with (patch.object(bench, "TASKS", str(tasks)), patch.object(task_sources, "CORPUS", corpus),
                  patch.object(task_sources, "REPOS", repos)):
                public_only = bench.study_fixture_manifest(["t1-intervals"])
                self.assertEqual(public_only, campaign.fixture_manifest(str(tasks), ["t1-intervals"]))
                mixed = bench.study_fixture_manifest(["verified-range", "t1-intervals"])
                again = bench.study_fixture_manifest(["verified-range", "t1-intervals"])
            self.assertEqual(mixed["tasks"], ["verified-range", "t1-intervals"])
            self.assertEqual(mixed["files"]["t1-intervals"], public_only["files"]["t1-intervals"])
            self.assertIn("scenario.json", mixed["files"]["verified-range"])
            self.assertEqual(mixed["fixture_hash"], again["fixture_hash"])
            self.assertNotEqual(mixed["fixture_hash"], public_only["fixture_hash"])


class DroidArmTests(unittest.TestCase):
    def test_harness_choices_and_fixed_modes(self):
        parser = bench.build_parser()
        args = parser.parse_args(["run", "--harness", "davinci", "droid-direct", "droid-script", "--warmup"])
        self.assertEqual(args.harness, ["davinci", "droid-direct", "droid-script"])
        self.assertTrue(args.warmup)
        with (tempfile.TemporaryDirectory() as tmp,
              patch.object(bench, "DROID_TOOL_EXECUTION_MODE", "script-only")):
            direct = bench.command("droid-direct", "hi", str(Path(tmp) / "wd"))
            script = bench.command("droid-script", "hi", str(Path(tmp) / "wd"))
        self.assertEqual(direct[direct.index("--tool-execution-mode") + 1], "direct-only")
        self.assertEqual(script[script.index("--tool-execution-mode") + 1], "script-only")

    def test_model_and_autonomy_are_independent_of_bench_effort(self):
        with tempfile.TemporaryDirectory() as tmp:
            workdir = str(Path(tmp) / "run")
            with (patch.object(bench, "MODEL", "gpt-6.1-sol"), patch.object(bench, "EFFORT", "medium"),
                  patch.dict(os.environ, {"BENCH_DROID_MODEL": "custom:alias", "BENCH_DROID_AUTO": "low"})):
                args = bench.command("droid-script", "hi", workdir)
            self.assertEqual(args[args.index("--model") + 1], "custom:alias")
            self.assertEqual(args[args.index("--auto") + 1], "low")
            with (patch.object(bench, "MODEL", "gpt-6.1-sol"), patch.object(bench, "EFFORT", "medium"),
                  patch.dict(os.environ, {}, clear=False)):
                os.environ.pop("BENCH_DROID_MODEL", None)
                os.environ.pop("BENCH_DROID_AUTO", None)
                args = bench.command("droid-script", "hi", workdir)
            self.assertEqual(args[args.index("--model") + 1], "gpt-6.1-sol")
            self.assertEqual(args[args.index("--auto") + 1], "high")
            with patch.dict(os.environ, {"BENCH_DROID_AUTO": "max"}):
                with self.assertRaises(ValueError):
                    bench.command("droid-script", "hi", workdir)

    def test_every_droid_arm_uses_bench_droid_and_records_its_pinned_config(self):
        for harness in droid_arm.DROID_HARNESSES:
            self.assertEqual(droid_arm.binary_env(harness), "BENCH_DROID")
        self.assertEqual(droid_arm.binary_env("codex"), "BENCH_CODEX")
        import runner
        settings = runner.droid_arm_settings("droid-direct", "gpt-6.1-sol")
        self.assertEqual(settings["tool_execution_mode"], "direct-only")
        self.assertEqual(settings["reasoning_effort"], "not forced")
        self.assertEqual(settings["auto"], "high")

    def test_gate_and_integrity_accept_the_named_arms(self):
        for harnesses, code in ((["davinci", "droid-direct", "droid-script"], 0),
                                (["davinci", "droid-script"], 0),
                                (["davinci", "codex", "droid-direct"], 0),
                                (["droid-direct", "droid-script"], 1),
                                (["davinci"], 1)):
            with tempfile.TemporaryDirectory() as tmp:
                (Path(tmp) / "campaign.json").write_text(json.dumps({"harnesses": harnesses}))
                with (patch.object(bench, "RUNS", tmp), patch.object(bench, "load_rows", return_value=[]),
                      patch.object(bench, "campaign_errors", return_value=[]), patch("builtins.print")):
                    with self.assertRaises(SystemExit) as raised:
                        bench.gate()
                self.assertEqual(raised.exception.code, code, harnesses)
        rows = [campaign_row(h) for h in ("davinci", "droid-direct", "droid-script")]
        self.assertEqual(campaign.integrity_errors(rows, ["t1-intervals"], [0], ["davinci", "droid-direct", "droid-script"]), [])
        order = campaign.schedule(["t1-intervals", "t2-duration", "t3-lru"], [0],
                                  ["davinci", "droid-direct", "droid-script"])
        self.assertEqual(len(order), 9)


class DroidRunRowTests(unittest.TestCase):
    """One fully patched run_one: session discovery, resolved mode, row fields, warm-up file."""

    def run_row(self, tmp, harness, resolved_mode, warmup=False, allowed_globs=()):
        home = Path(tmp) / "factory"
        runs = Path(tmp) / "runs"
        runs.mkdir()
        measured = {"exit": 0, "stdout": "", "stderr": "", "wall_s": 1,
                    "started_at": "s", "finished_at": "f", "cleanup_complete": True}

        def execute(*args):
            session = home / "sessions" / "proj"
            session.mkdir(parents=True)
            body = {"tokenUsage": {"inputTokens": 10, "cacheReadInputTokens": 5,
                                   "outputTokens": 3, "reasoningTokens": 1}}
            if resolved_mode is not None:
                body["toolExecutionMode"] = resolved_mode
            (session / "sid1.settings.json").write_text(json.dumps(body))
            (home / "logs").mkdir()
            (home / "logs" / "droid-log-single.log").write_text(
                '[t] INFO: Streaming result | Context: {"sessionId":"sid1","count":7}')
            return measured

        spec = {"prompt": "p", "allowed": ["a.txt"], "allowed_globs": list(allowed_globs)}
        with (patch.dict(os.environ, {"BENCH_DROID_HOME": str(home)}),
              patch.object(bench, "RUNS", str(runs)), patch.object(bench, "load", return_value=spec),
              patch.object(bench, "prepare", side_effect=lambda tid, dest: Path(dest).mkdir(parents=True)),
              patch.object(bench, "execute", side_effect=execute),
              patch.object(bench, "changed_files", return_value=["a.txt", "pkg/m1.ts", "pkg/other/x.ts"]),
              patch.object(bench, "forbidden_artifacts", return_value={"artifact_leak": False}),
              patch.object(bench, "grade", return_value=(True, 1, 0, "ok")),
              patch("builtins.print")):
            row = bench.run_one(harness, "case", 0, warmup=warmup)
        return row, runs

    def test_row_records_arm_mode_model_and_resolved_mode(self):
        with tempfile.TemporaryDirectory() as tmp:
            with patch.dict(os.environ, {"BENCH_DROID_MODEL": "custom:alias"}):
                row, runs = self.run_row(tmp, "droid-direct", "direct-only")
            self.assertEqual(row["droid_tool_execution_mode"], "direct-only")
            self.assertEqual(row["droid_resolved_tool_execution_mode"], "direct-only")
            self.assertIs(row["droid_mode_mismatch"], False)
            self.assertEqual(row["droid_model"], "custom:alias")
            self.assertEqual(row["droid_auto"], "high")
            self.assertEqual(row["droid_reasoning_effort"], "not forced")
            self.assertEqual(row["logical_requests"], 1)
            self.assertIsNone(row["provider_attempts"])
            self.assertTrue((runs / "results.jsonl").is_file())
            self.assertFalse((runs / "warmup.jsonl").exists())
            self.assertNotIn("scored", row)

    def test_mode_mismatch_and_absent_resolved_mode(self):
        with tempfile.TemporaryDirectory() as tmp:
            row, _ = self.run_row(tmp, "droid-script", "direct-only")
        self.assertEqual(row["droid_resolved_tool_execution_mode"], "direct-only")
        self.assertIs(row["droid_mode_mismatch"], True)
        with tempfile.TemporaryDirectory() as tmp:
            row, _ = self.run_row(tmp, "droid-script", None)
        self.assertIsNone(row["droid_resolved_tool_execution_mode"])
        self.assertIsNone(row["droid_mode_mismatch"])
        self.assertIsNone(droid_usage.droid_session_mode(Path(tmp) / "missing.json"))

    def test_allowed_globs_keep_task_outputs_out_of_unrelated(self):
        with tempfile.TemporaryDirectory() as tmp:
            row, _ = self.run_row(tmp, "droid-direct", "direct-only", allowed_globs=["pkg/m*.ts"])
        self.assertEqual(row["unrelated"], ["pkg/other/x.ts"])

    def test_warmup_rows_go_to_warmup_jsonl_only_and_are_never_read_as_results(self):
        import gap_study
        with tempfile.TemporaryDirectory() as tmp:
            row, runs = self.run_row(tmp, "droid-direct", "direct-only", warmup=True)
            self.assertIs(row["scored"], False)
            self.assertTrue((runs / "warmup.jsonl").is_file())
            self.assertFalse((runs / "results.jsonl").exists())
            with self.assertRaises(FileNotFoundError):
                bench.load_rows(str(runs / "results.jsonl"))
            (runs / "results.jsonl").write_text(json.dumps({"harness": "davinci", "task": "t", "scored": False})
                                                + "\n" + json.dumps({"harness": "davinci", "task": "u"}) + "\n")
            self.assertEqual([r["task"] for r in gap_study.load_study_rows(runs)], ["u"])


if __name__ == "__main__":
    unittest.main()
