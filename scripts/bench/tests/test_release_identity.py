"""Offline release provenance gates; every CI record is a synthetic fixture."""
import importlib.util
import json
from pathlib import Path
import subprocess
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location("release_identity", Path(__file__).parents[2] / "release_identity.py")
release_identity = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release_identity)


class ReleaseIdentityTests(unittest.TestCase):
    def evidence(self):
        sha = "a" * 40
        return sha, {"repository": "fixture/repo", "source_sha": sha, "ci_run": 12,
                     "ci_url": "https://github.com/fixture/repo/actions/runs/12",
                     "status": "completed", "conclusion": "success",
                     "workflow_path": ".github/workflows/ci.yml",
                     "jobs": [{"name": name, "status": "completed", "conclusion": "success"}
                              for name in sorted(release_identity.EXPECTED_CI_JOBS)],
                     "workflow_lint": {"status": "completed", "conclusion": "success", "run_id": 13}}

    def test_green_ci_requires_exact_commit_and_full_jobs(self):
        sha, record = self.evidence()
        release_identity.validate_ci(record, "fixture/repo", sha)
        for patch in ({"source_sha": "b" * 40}, {"status": "in_progress"},
                      {"conclusion": "failure"}, {"jobs": record["jobs"][:-1]},
                      {"jobs": [dict(job, conclusion="skipped") for job in record["jobs"]]},
                      {"workflow_lint": None}, {"repository": "other/repo"}):
            with self.subTest(patch=patch), self.assertRaises(ValueError):
                release_identity.validate_ci(dict(record, **patch), "fixture/repo", sha)

    def test_identity_binds_tag_ci_source_and_binary_bytes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            binary = root / "davinci"
            binary.write_bytes(b"synthetic binary")
            sha, ci = self.evidence()
            source = {"source_sha": sha, "source_tree": "b" * 40, "source_clean": True,
                      "dirty_diff_hash": release_identity.CLEAN_DIFF_HASH}
            identity = release_identity.make_identity(binary, source, ci, "v1.0.71")
            self.assertEqual(identity["schema_version"], 3)
            self.assertEqual(identity["release_tag"], "v1.0.71")
            self.assertEqual(identity["ci_run"], 12)
            self.assertEqual(identity["binary_sha256"], release_identity.file_hash(binary))
            for bad in (None, "v0.0.0", "1.0.71"):
                with self.subTest(tag=bad), self.assertRaises(ValueError):
                    release_identity.make_identity(binary, source, ci, bad, version="1.0.71")

    def test_dirty_source_and_source_changed_during_build_are_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for args in (["init", "-q"], ["config", "user.name", "Fixture"], ["config", "user.email", "fixture@example.invalid"]):
                subprocess.run(["git", *args], cwd=root, check=True, capture_output=True)
            (root / "a.txt").write_text("before")
            subprocess.run(["git", "add", "a.txt"], cwd=root, check=True)
            subprocess.run(["git", "commit", "-qm", "fixture"], cwd=root, check=True)
            before = release_identity.source_identity(root)
            release_identity.require_same_source(root, before)
            (root / "a.txt").write_text("after")
            with self.assertRaises(ValueError):
                release_identity.require_same_source(root, before)

    def test_latest_failed_ci_cannot_fall_back_to_older_success(self):
        sha, _ = self.evidence()
        old = {"id": 1, "head_sha": sha, "status": "completed", "conclusion": "success", "created_at": "2026-09-29T00:00:00Z"}
        new = dict(old, id=2, conclusion="failure", created_at="2026-09-30T00:00:00Z")
        self.assertEqual(release_identity.latest_run([old, new], sha)["id"], 2)
        with self.assertRaises(ValueError):
            release_identity.latest_run([old], "b" * 40)


if __name__ == "__main__":
    unittest.main()
