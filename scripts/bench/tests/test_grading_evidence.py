"""Grader diagnostics survive summary selection and failed process cleanup."""
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import bench


class GradingEvidenceTests(unittest.TestCase):
    def checked(self, passed=False, cleanup=True):
        return {"pass": passed, "exit": 0 if passed else 1,
                "cleanup_complete": cleanup, "stdout": '{"command": "tests"}\n',
                "stderr": "AssertionError: missing evidence\n" if not passed else "OK\n"}

    def spec(self):
        return {"prompt": "Fix the regression", "allowed": [],
                "private_provenance": {key: "fixture" for key in
                    ("size_class", "split", "language", "repository_id", "reference_commit",
                     "visible_tests", "requires_existing_test_changes")},
                "regression_command": [sys.executable, "visible.py"],
                "grader_command": [sys.executable, "hidden.py"]}

    def test_run_retains_separate_full_receipts_before_hidden_overlay(self):
        for cleanup in (True, False):
            with self.subTest(cleanup=cleanup), tempfile.TemporaryDirectory() as temporary:
                workdir = Path(temporary) / "davinci/case-r0"
                measured = {"exit": 0, "stdout": "", "stderr": "", "wall_s": 0,
                            "started_at": "start", "finished_at": "end", "cleanup_complete": True}
                spec = self.spec()
                regression, grader = self.checked(True), self.checked(cleanup=cleanup)

                def overlay(*args):
                    self.assertTrue(Path(str(workdir) + ".regression.json").is_file())

                with (patch.object(bench, "RUNS", temporary),
                      patch.object(bench, "load", return_value=spec),
                      patch.object(bench, "prepare", side_effect=lambda tid, dest: Path(dest).mkdir(parents=True)),
                      patch.object(bench, "execute", return_value=measured),
                      patch.object(bench, "changed_files", return_value=[]),
                      patch.object(bench, "forbidden_artifacts", return_value={"artifact_leak": False}),
                      patch.object(bench, "apply_hidden", side_effect=overlay),
                      patch.object(bench, "run_check", side_effect=[regression, grader]),
                      patch("builtins.print")):
                    if cleanup:
                        row = bench.run_one("davinci", "case", 0)
                    else:
                        with self.assertRaisesRegex(RuntimeError, "grader_cleanup_failed"):
                            bench.run_one("davinci", "case", 0)
                        row = bench.load_rows(str(Path(temporary) / "results.jsonl"))[0]
                self.assertFalse(row["pass"])
                self.assertFalse(row["grader_pass"])
                self.assertEqual(row["regression_pass"], True)
                for phase, checked in (("regression", regression), ("grader", grader)):
                    ref = row["grading_evidence"][phase]
                    path = Path(ref["path"])
                    # Windows CI may spell its temp root using an 8.3 alias.
                    self.assertEqual(path.parent, workdir.parent.resolve())
                    self.assertEqual(ref["sha256"], hashlib.sha256(path.read_bytes()).hexdigest())
                    receipt = json.loads(path.read_text(encoding="utf-8"))
                    self.assertEqual(receipt["command"], spec[phase + "_command"])
                    self.assertEqual(receipt["cwd"], str(workdir.resolve()))
                    self.assertEqual(receipt["timeout_seconds"], 120)
                    for key, value in checked.items():
                        self.assertEqual(receipt[key], value)
                self.assertNotIn("AssertionError", json.dumps(row))

    def test_public_grader_also_retains_stderr(self):
        with tempfile.TemporaryDirectory() as temporary:
            workdir = Path(temporary) / "candidate"
            workdir.mkdir()
            with (patch.object(bench, "load", return_value={}),
                  patch.object(bench, "copy_tree"),
                  patch.object(bench, "run_check", return_value=self.checked())):
                result = bench.grade("case", str(workdir))
            self.assertFalse(result[0])
            receipt = json.loads(Path(str(workdir) + ".grader.json").read_text(encoding="utf-8"))
            self.assertEqual(receipt["stderr"], self.checked()["stderr"])
            self.assertEqual(receipt["command"][:3], [sys.executable, "-m", "pytest"])

    def test_validation_preserves_starter_reference_and_regression_receipts(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "tasks/case/solution").mkdir(parents=True)
            workdir = root / "runs/_validate/case"
            with (patch.object(bench, "RUNS", str(root / "runs")),
                  patch.object(bench, "TASKS", str(root / "tasks")),
                  patch.object(bench, "load", return_value=self.spec()),
                  patch.object(bench, "prepare", side_effect=lambda tid, dest: Path(dest).mkdir(parents=True, exist_ok=True)),
                  patch.object(bench, "apply_hidden"),
                  patch.object(bench, "run_check", side_effect=[self.checked(), self.checked(True), self.checked(True)]),
                  patch("builtins.print"), self.assertRaises(SystemExit) as result):
                bench.validate(private_tasks=["case"])
            self.assertEqual(result.exception.code, 0)
            for phase, passed in (("starter-grader", False), ("reference-grader", True), ("reference-regression", True)):
                receipt = json.loads(Path(str(workdir) + "." + phase + ".json").read_text(encoding="utf-8"))
                self.assertEqual(receipt["pass"], passed)


if __name__ == "__main__":
    unittest.main()
