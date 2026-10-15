"""Contract checks for the v2 large-task fixtures. Offline; no model calls."""
import importlib.util
import json
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

    def test_new_test_files_are_allowed_only_where_declared(self):
        spec = {"allowed": ["cli.py"], "allowed_patterns": ["tests/test_*.py"]}
        self.assertEqual(bench.unrelated_paths(
            ["cli.py", "tests/test_extra.py", "tests/deep/test_x.py", "tests/helper.py", "test_hidden_cli.py"], spec),
            ["tests/deep/test_x.py", "tests/helper.py", "test_hidden_cli.py"])
        self.assertEqual(bench.unrelated_paths(["cli.py", "tests/test_extra.py"], {"allowed": ["cli.py"]}),
                         ["tests/test_extra.py"])

    @unittest.skipUnless(HAS_PYTEST, "pytest is needed to run the hidden tests")
    def test_starters_references_alternatives_and_broken_implementations(self):
        report, errors = validation.validate()
        self.assertEqual(errors, [])
        self.assertEqual(set(report), {spec["id"] for spec in v2.TASK_SPECS})


if __name__ == "__main__":
    unittest.main()
