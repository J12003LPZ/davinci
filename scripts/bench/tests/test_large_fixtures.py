"""Contract checks for generated large and specialist fixtures."""
import importlib.util
import json
from pathlib import Path
import unittest
import tempfile
from unittest.mock import patch


ROOT = Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location("make_large_tasks", ROOT / "make_large_tasks.py")
generator = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(generator)


class LargeFixtureTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.task_root = Path(self.temporary.name) / "tasks"
        with patch.object(generator, "TASK_ROOT", self.task_root):
            for spec in generator.TASK_SPECS:
                generator.write_task(spec)

    def test_manifest_is_explicit_and_matches_task_metadata(self):
        manifest = json.loads((ROOT / "large_manifest.json").read_text(encoding="utf-8"))
        self.assertEqual(manifest["task_set"], "large")
        self.assertEqual(manifest["tasks"], [
            "m1-config-parser", "m2-public-api-migration",
            "m3-state-persistence", "m4-cli-feature",
            "m5-wide-migration", "m6-requirements-ledger", "m7-log-forensics",
        ])
        for task in manifest["tasks"]:
            directory = self.task_root / task
            metadata = json.loads((directory / "task.json").read_text(encoding="utf-8"))
            entry = manifest["fixtures"][task]
            self.assertEqual(metadata["allowed"], entry["allowed"])
            self.assertEqual(generator.tree_hash(directory / "repo"), entry["public_hash"])
            self.assertEqual(generator.tree_hash(directory / "solution"), entry["reference_solution_hash"])
            self.assertEqual(generator.tree_hash(directory / "hidden"), entry["hidden_grader_hash"])

    def test_long_fixtures_meet_their_size_floors(self):
        sizes = generator.check_size_floors()
        self.assertEqual(sorted(sizes), ["m5-wide-migration", "m6-requirements-ledger", "m7-log-forensics"])
        for task, size in sizes.items():
            on_disk = sum(p.stat().st_size for p in (self.task_root / task / "repo").rglob("*") if p.is_file())
            self.assertEqual(size, on_disk, task)
            self.assertGreaterEqual(size, generator.LARGE_SIZE_FLOORS[task], task)

    def test_size_floor_check_rejects_a_shrunken_fixture(self):
        shrunk = [dict(spec, repo={"a.py": "x = 1"}) for spec in generator.TASK_SPECS
                  if spec["id"] == "m5-wide-migration"]
        with self.assertRaises(ValueError):
            generator.check_size_floors(shrunk)

    def test_requirement_ledger_has_a_test_for_every_requirement(self):
        hidden = (self.task_root / "m6-requirements-ledger" / "hidden" / "test_hidden_warehouse.py").read_text(
            encoding="utf-8")
        for number in range(1, 15):
            self.assertIn(f"def test_req{number:02d}_", hidden)

    def test_migration_fixture_constraints_each_have_a_test(self):
        hidden = (self.task_root / "m5-wide-migration" / "hidden" / "test_hidden_migration.py").read_text(
            encoding="utf-8")
        for marker in ("test_c1_", "test_c2_", "test_c3_", "test_c4_"):
            self.assertIn(marker, hidden)

    def test_specialist_fixtures_are_offline_and_preference_neutral(self):
        manifest = json.loads((ROOT / "specialist_fixtures" / "manifest.json").read_text(encoding="utf-8"))
        self.assertFalse(manifest["preference_is_functional_requirement"])
        self.assertEqual(set(manifest["fixtures"]), {"discovery", "lsp", "browser"})
        for entry in manifest["fixtures"].values():
            self.assertTrue(entry["offline"])
            self.assertFalse(entry["preference_required"])

    def test_generated_specialists_match_committed_bytes_and_both_manifests(self):
        specialist_root = Path(self.temporary.name) / "specialists"
        with patch.object(generator, "SPECIALIST_ROOT", specialist_root):
            generated = generator.write_specialist_fixtures()
        manifest = json.loads((ROOT / "specialist_fixtures" / "manifest.json").read_text(encoding="utf-8"))
        large_manifest = json.loads((ROOT / "large_manifest.json").read_text(encoding="utf-8"))
        self.assertEqual(generated, manifest["fixtures"])
        self.assertEqual(generated, large_manifest["specialist_fixtures"])
        for name, entry in generated.items():
            content = (specialist_root / name / "fixture.json").read_bytes()
            self.assertNotIn(b"\r\n", content)
            self.assertEqual(content, (ROOT / entry["path"]).read_bytes())
            self.assertEqual(generator.tree_hash(specialist_root / name), entry["fixture_hash"])


if __name__ == "__main__":
    unittest.main()
