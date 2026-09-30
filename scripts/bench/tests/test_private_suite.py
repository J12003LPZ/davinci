"""Private task provenance and validation using only temporary local Git repos."""
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import bench
import private_suite


class PrivateSuiteTests(unittest.TestCase):
    def fixture_execute(self, command, workdir, env, timeout):
        # The fixtures are known one-shot unittest processes. Exercise their
        # actual exit/output on hosts where native process-tree ownership is
        # unavailable; native cleanup has its own runner tests.
        result = subprocess.run(command, cwd=workdir, env=env, timeout=timeout,
                                capture_output=True, text=True)
        return {"exit": result.returncode, "cleanup_complete": True,
                "stdout": result.stdout, "stderr": result.stderr}

    def git(self, root, *args):
        return subprocess.run(["git", *args], cwd=root, capture_output=True, text=True, check=True).stdout.strip()

    def fixture(self, root):
        repo = root / "source"
        repo.mkdir()
        self.git(repo, "init", "-q")
        self.git(repo, "config", "user.name", "Fixture")
        self.git(repo, "config", "user.email", "fixture@example.invalid")
        (repo / "calc.py").write_text("def add(a, b): return 0\n")
        (repo / "test_calc.py").write_text("import unittest\nfrom calc import add\nclass Calc(unittest.TestCase):\n def test_sum(self): self.assertEqual(add(2, 3), 0)\n")
        self.git(repo, "add", ".")
        self.git(repo, "commit", "-qm", "starter")
        starter = self.git(repo, "rev-parse", "HEAD")
        (repo / "calc.py").write_text("def add(a, b): return a + b\n")
        (repo / "test_calc.py").write_text("import unittest\nfrom calc import add\nclass Calc(unittest.TestCase):\n def test_sum(self): self.assertEqual(add(2, 3), 5)\n def test_negative(self): self.assertEqual(add(-2, 3), 1)\n")
        self.git(repo, "add", ".")
        self.git(repo, "commit", "-qm", "source and tests")
        task = {"id": "private-sum", "repository": "source", "repository_id": "fixture/repo", "language": "python",
                "starter_commit": starter, "reference_commit": self.git(repo, "rev-parse", "HEAD"),
                "split": "dev", "size_class": "large", "visible_tests": True, "requires_existing_test_changes": True,
                "test_paths": ["test_calc.py"], "grader_command": [sys.executable, "-m", "unittest", "test_calc"],
                "regression_command": [sys.executable, "-m", "unittest", "discover"],
                "human_prompt": {"text": "Implement addition and update the obsolete tests.", "author": "Fixture owner",
                                 "source": "fixture PR description", "authored_from": "pr-description", "diff_used": False}}
        manifest = root / "manifest.json"
        manifest.write_text(json.dumps({"schema_version": 1, "kind": "private-repository-suite", "tasks": [task]}))
        return manifest, task

    def test_import_freezes_provenance_labels_and_complete_reference(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest, task = self.fixture(root)
            frozen = private_suite.import_suite(manifest, root / "frozen")
            self.assertFalse(frozen["inventory"]["target_sizes_met"])
            self.assertFalse(frozen["promotion_eligible"])
            loaded, tasks, pinned = private_suite.load_frozen(root / "frozen", "dev")
            self.assertEqual(tasks, [task["id"]])
            self.assertEqual(loaded["labels"][task["id"]]["size_class"], "large")
            self.assertTrue(pinned)
            (root / "frozen" / task["id"] / "repo" / "calc.py").write_text("changed")
            with self.assertRaises(ValueError):
                private_suite.load_frozen(root / "frozen", "dev")

    def test_candidate_and_reference_validation_use_private_grader_argv(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest, task = self.fixture(root)
            private_suite.import_suite(manifest, root / "frozen")
            with patch.object(bench, "TASKS", str(root / "frozen")), patch.object(private_suite, "execute", self.fixture_execute):
                bench.prepare(task["id"], root / "candidate")
                before = bench.grade(task["id"], root / "candidate")
                self.assertFalse(before[0])
                bench.prepare(task["id"], root / "candidate")
                bench.copy_tree(root / "frozen" / task["id"] / "solution", root / "candidate")
                after = bench.grade(task["id"], root / "candidate")
                self.assertTrue(after[0], after)

    def test_provenance_and_unsafe_paths_fail_before_export(self):
        for path in ("../secret", "/secret", "C:/secret", "x\x00y", ".GIT/config", "a/../b", "a\\b"):
            with self.subTest(path=path), self.assertRaises(ValueError):
                private_suite.relative_path(path)
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest, task = self.fixture(root)
            task["human_prompt"]["diff_used"] = True
            manifest.write_text(json.dumps({"schema_version": 1, "kind": "private-repository-suite", "tasks": [task]}))
            with self.assertRaises(ValueError):
                private_suite.import_suite(manifest, root / "frozen")
            self.assertFalse((root / "frozen").exists())

    def test_duplicate_revision_cannot_cross_dev_and_holdout(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            manifest, task = self.fixture(root)
            duplicate = dict(task, id="other-id", split="holdout")
            manifest.write_text(json.dumps({"schema_version": 1, "kind": "private-repository-suite", "tasks": [task, duplicate]}))
            with self.assertRaises(ValueError):
                private_suite.read_suite(manifest)


if __name__ == "__main__":
    unittest.main()
