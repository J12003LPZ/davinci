"""Contract checks for generated large and specialist fixtures."""
import importlib.util
import json
from pathlib import Path
import unittest


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("make_large_tasks", ROOT / "make_large_tasks.py")
generator = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(generator)


class LargeFixtureTests(unittest.TestCase):
    def test_manifest_is_explicit_and_matches_task_metadata(self):
        manifest = json.loads((ROOT / "large_manifest.json").read_text(encoding="utf-8"))
        self.assertEqual(manifest["task_set"], "large")
        self.assertEqual(manifest["tasks"], [
            "m1-config-parser", "m2-public-api-migration",
            "m3-state-persistence", "m4-cli-feature",
        ])
        for task in manifest["tasks"]:
            directory = ROOT / "tasks" / task
            metadata = json.loads((directory / "task.json").read_text(encoding="utf-8"))
            entry = manifest["fixtures"][task]
            self.assertEqual(metadata["allowed"], entry["allowed"])
            self.assertEqual(generator.tree_hash(directory / "repo"), entry["public_hash"])
            self.assertEqual(generator.tree_hash(directory / "solution"), entry["reference_solution_hash"])
            self.assertEqual(generator.tree_hash(directory / "hidden"), entry["hidden_grader_hash"])

    def test_specialist_fixtures_are_offline_and_preference_neutral(self):
        manifest = json.loads((ROOT / "specialist_fixtures" / "manifest.json").read_text(encoding="utf-8"))
        self.assertFalse(manifest["preference_is_functional_requirement"])
        self.assertEqual(set(manifest["fixtures"]), {"discovery", "lsp", "browser"})
        for entry in manifest["fixtures"].values():
            self.assertTrue(entry["offline"])
            self.assertFalse(entry["preference_required"])


if __name__ == "__main__":
    unittest.main()
