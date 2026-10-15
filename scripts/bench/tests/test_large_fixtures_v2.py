"""Contract checks for the v2 large-task fixtures. Offline; no model calls."""
import importlib.util
import json
import os
from pathlib import Path
import re
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
import make_large_tasks as v1  # noqa: E402
import make_large_tasks_v2 as v2  # noqa: E402
import large_v2_validation as validation  # noqa: E402
import bench  # noqa: E402
from runner import LEGACY_TASKS, validate_large_manifest  # noqa: E402

HAS_PYTEST = importlib.util.find_spec("pytest") is not None
# Set in the dedicated CI job: there a missing pytest is a failure, not a skip.
REQUIRE_BEHAVIORAL = os.environ.get("BENCH_REQUIRE_BEHAVIORAL") == "1"


def hidden_tests(spec):
    (text,) = spec["hidden"].values()
    return set(re.findall(r"^def (test_\w+)\(", text, re.M))


class LargeFixturesV2Tests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.temporary = tempfile.TemporaryDirectory()
        cls.task_root = Path(cls.temporary.name) / "tasks"
        cls.manifest_path = Path(cls.temporary.name) / "large_manifest_v2.json"
        with patch.object(v1, "TASK_ROOT", cls.task_root), patch.object(v2, "MANIFEST", cls.manifest_path), \
                patch("builtins.print"):
            v2.main()

    @classmethod
    def tearDownClass(cls):
        cls.temporary.cleanup()

    def test_committed_manifest_matches_generated_fixtures(self):
        committed = json.loads((ROOT / "large_manifest_v2.json").read_text(encoding="utf-8"))
        self.assertEqual(committed, json.loads(self.manifest_path.read_text(encoding="utf-8")))
        self.assertTrue(validate_large_manifest(committed))
        self.assertEqual((committed["fixture_version"], committed["independent_holdout"]), ("v2", False))
        for task in committed["tasks"]:
            entry, directory = committed["fixtures"][task], self.task_root / task
            for part, field in (("repo", "public_hash"), ("solution", "reference_solution_hash"),
                                ("hidden", "hidden_grader_hash")):
                self.assertEqual(v1.tree_hash(directory / part), entry[field], (task, part))
            metadata = json.loads((directory / "task.json").read_text(encoding="utf-8"))
            self.assertEqual((metadata["fixture_version"], metadata["allowed_patterns"]), ("v2", ["tests/test_*.py"]))

    def test_v1_is_untouched_and_v2_starters_are_v1_starters(self):
        v1_manifest = json.loads((ROOT / "large_manifest.json").read_text(encoding="utf-8"))
        v2_manifest = json.loads((ROOT / "large_manifest_v2.json").read_text(encoding="utf-8"))
        self.assertNotIn("fixture_version", v1_manifest)
        self.assertTrue(set(v2_manifest["tasks"]).isdisjoint(v1_manifest["tasks"]))
        self.assertTrue(set(v2_manifest["tasks"]).isdisjoint(LEGACY_TASKS))
        for spec in v2.TASK_SPECS:
            base = v1_manifest["fixtures"][spec["base"]]
            entry = v2_manifest["fixtures"][spec["id"]]
            self.assertEqual(entry["public_hash"], base["public_hash"], spec["id"])
            self.assertEqual((entry["allowed"], entry["public_verification"]),
                             (base["allowed"], base["public_verification"]))
            self.assertNotEqual(entry["hidden_grader_hash"], base["hidden_grader_hash"])

    def test_every_hidden_test_traces_to_the_prompt_or_starter(self):
        for spec in v2.TASK_SPECS:
            mapping = v2.REQUIREMENTS[spec["id"]]
            self.assertEqual(set(mapping), hidden_tests(spec), spec["id"])
            for test, (requirement, source) in mapping.items():
                self.assertTrue(requirement.startswith(spec["id"][:2] + "."), requirement)
                if source.startswith("starter:"):
                    continue
                self.assertIn(source, spec["prompt"], f"{spec['id']} {test}: not stated in the prompt")

    def test_every_hidden_test_is_failed_by_some_known_defect(self):
        for spec in v2.TASK_SPECS:
            expected = v2.VALIDATION[spec["id"]]
            failing = list(expected["starter_fails"])
            for _, names in expected["broken"].values():
                failing += names
            bases = {name.split("[", 1)[0] for name in failing}
            self.assertEqual(bases - hidden_tests(spec), set(), spec["id"])
            self.assertEqual(hidden_tests(spec) - bases, set(), f"{spec['id']}: tests no defect fails")
            self.assertTrue(expected["alternatives"], spec["id"])

    def test_variants_apply_cleanly(self):
        for spec in v2.TASK_SPECS:
            expected = v2.VALIDATION[spec["id"]]
            variants = list(expected["alternatives"].values()) + [v for v, _ in expected["broken"].values()]
            for variant in variants:
                validation.implementation(spec, variant)

    def test_starters_references_alternatives_and_broken_implementations(self):
        if not HAS_PYTEST:
            if REQUIRE_BEHAVIORAL:
                self.fail("BENCH_REQUIRE_BEHAVIORAL=1 but pytest is not installed")
            self.skipTest("pytest is needed to run the hidden tests")
        report, errors = validation.validate()
        self.assertEqual(errors, [])
        self.assertEqual(set(report), {spec["id"] for spec in v2.TASK_SPECS})



class AllowedPatternBoundaryTests(unittest.TestCase):
    """`allowed_patterns` may admit new test files and nothing else."""

    SPEC = {"allowed": ["cli.py", "tests/test_public_cli.py"], "allowed_patterns": ["tests/test_*.py"]}

    def test_admits_only_plain_new_test_files_in_the_named_directory(self):
        admitted = ["tests/test_extra.py", "tests/test_report_2.py", "tests/test_X.py"]
        refused = [
            "tests/deep/test_x.py", "tests/../cli.py", "tests/test_x.py/../../cli.py", "../tests/test_x.py",
            "/tmp/tests/test_x.py", "C:/w/tests/test_x.py", "tests\\test_x.py", "tests/test_.py",
            "tests/test_x.pyc", "tests/test_x.py.bak", "tests/test_x.PY", "tests/test x.py", "tests/test_-x.py",
            "tests/test_\u00e9.py", "tests/test_x\n.py", "tests/conftest.py", "tests/helper.py",
            "tests/__init__.py", "test_root.py", "test_hidden_cli.py", "Tests/test_x.py", "./tests/test_x.py",
            "inventory/test_x.py", "tests/test_x.py:stream", "",
        ]
        changed = ["cli.py", "tests/test_public_cli.py", *admitted, *refused]
        self.assertEqual(bench.unrelated_paths(changed, self.SPEC), refused)

    def test_broad_or_unsafe_patterns_are_rejected_before_use(self):
        manifest = json.loads((ROOT / "large_manifest_v2.json").read_text(encoding="utf-8"))
        task = manifest["tasks"][0]
        for pattern in ["*", "**", "*.py", "tests/*", "tests/*.py", "test_*.py", "tests/**/test_*.py",
                        "../tests/test_*.py", "/tests/test_*.py", "tests/../test_*.py", "./tests/test_*.py",
                        "tests\\test_*.py", "tests/test_?.py", "tests/test_[a-z].py", "tests/test_*",
                        "tests/test_*.py*", "C:/tests/test_*.py", "", None, 3]:
            with self.subTest(pattern=pattern):
                with self.assertRaises(ValueError):
                    bench.unrelated_paths(["cli.py"], {"allowed": ["cli.py"], "allowed_patterns": [pattern]})
                broken = json.loads(json.dumps(manifest))
                broken["fixtures"][task]["allowed_patterns"] = [pattern]
                with self.assertRaises(ValueError):
                    validate_large_manifest(broken)
        for patterns in ([], "tests/test_*.py", ["tests/test_*.py"] * 5):
            with self.assertRaises(ValueError):
                bench.unrelated_paths([], {"allowed": [], "allowed_patterns": patterns})

    def test_v1_specs_keep_the_exact_allowlist(self):
        v1_manifest = json.loads((ROOT / "large_manifest.json").read_text(encoding="utf-8"))
        self.assertTrue(validate_large_manifest(v1_manifest))
        for task, entry in v1_manifest["fixtures"].items():
            self.assertNotIn("allowed_patterns", entry)
            changed = [*entry["allowed"], "tests/test_extra.py", "notes.md"]
            self.assertEqual(bench.unrelated_paths(changed, entry), ["tests/test_extra.py", "notes.md"], task)

    def test_a_real_workspace_admits_a_new_test_file_but_not_a_symlinked_one(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            work, outside, tasks = root / "work", root / "outside.py", root / "tasks"
            (work / "tests").mkdir(parents=True)
            (work / "cli.py").write_text("x = 1\n", encoding="utf-8")
            (tasks / "m-fixture" / "hidden").mkdir(parents=True)
            (tasks / "m-fixture" / "hidden" / "test_hidden_cli.py").write_text("", encoding="utf-8")
            for args in (["init", "-q"], ["config", "user.email", "b@e"], ["config", "user.name", "b"],
                         ["config", "core.hooksPath", ""], ["add", "-A"], ["commit", "-q", "-m", "base"]):
                self.assertEqual(bench.git(str(work), *args).returncode, 0, args)
            (work / "cli.py").write_text("x = 2\n", encoding="utf-8")
            (work / "tests" / "test_new.py").write_text("def test_x():\n    pass\n", encoding="utf-8")
            (work / "tests" / "helper.py").write_text("", encoding="utf-8")
            (work / "test_hidden_cli.py").write_text("", encoding="utf-8")
            outside.write_text("secret = 1\n", encoding="utf-8")
            changed = bench.changed_files(str(work))
            self.assertEqual(bench.unrelated_paths(changed, {"allowed": ["cli.py"],
                                                             "allowed_patterns": ["tests/test_*.py"]}),
                             ["test_hidden_cli.py", "tests/helper.py"])
            with patch.object(bench, "TASKS", str(tasks)):
                self.assertEqual(bench.forbidden_artifacts(work, "m-fixture")["artifact_paths"], ["test_hidden_cli.py"])
                try:
                    os.symlink(outside, work / "tests" / "test_link.py")
                except (OSError, NotImplementedError):
                    self.skipTest("symlinks unavailable")
                artifacts = bench.forbidden_artifacts(work, "m-fixture")
            self.assertTrue(artifacts["artifact_leak"])
            self.assertIn("tests/test_link.py", artifacts["artifact_paths"])


if __name__ == "__main__":
    unittest.main()
