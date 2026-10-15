"""Tests for scripts/collect-v2-evidence.ps1 and the v2 manifest's checkout bytes.

The script is Windows-only (it builds a Windows virtual environment), so its
tests run only on Windows with Windows PowerShell or PowerShell 7 available.
Each test builds a throwaway Git repository holding a copy of the script and a
stand-in scripts/bench, so nothing here reads C:\\bench or the real harness.
"""
import hashlib
import os
import shutil
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parents[2]
SCRIPT = REPO / "scripts" / "collect-v2-evidence.ps1"
MANIFEST = REPO / "scripts" / "bench" / "large_manifest_v2.json"
PINNED_MANIFEST_SHA256 = "49e83faeb7b3091db5ea2952dbc8d180f5acb1b17c44abfd8080ce91e52309e0"
# V2_EVIDENCE_SHELL=powershell runs the suite under Windows PowerShell 5.1.
SHELL = shutil.which(os.environ.get("V2_EVIDENCE_SHELL", "pwsh")) or shutil.which("powershell")


def git(cwd, *args):
    return subprocess.run(["git", "-C", str(cwd), *args], check=True,
                          capture_output=True, text=True).stdout.strip()


class ManifestCheckoutBytes(unittest.TestCase):
    def test_v2_manifest_is_pinned_to_lf_on_every_platform(self):
        attrs = git(REPO, "check-attr", "text", "eol", "--", "scripts/bench/large_manifest_v2.json")
        self.assertIn("eol: lf", attrs, "a CRLF checkout changes the pinned manifest SHA-256 on Windows")

    def test_checked_out_manifest_has_the_pinned_hash(self):
        self.assertEqual(hashlib.sha256(MANIFEST.read_bytes()).hexdigest(), PINNED_MANIFEST_SHA256)


@unittest.skipUnless(os.name == "nt" and SHELL, "Windows with PowerShell only")
class CollectV2Evidence(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="v2-evidence-test-"))
        self.addCleanup(shutil.rmtree, self.tmp, ignore_errors=True)
        self.repo = self.tmp / "clone"
        bench = self.repo / "scripts" / "bench"
        (bench / "tests").mkdir(parents=True)
        shutil.copy(SCRIPT, self.repo / "scripts" / SCRIPT.name)
        (bench / "large_manifest_v2.json").write_bytes(b'{"tasks": []}\n')
        (bench / "v2-validation-requirements.txt").write_text("pip\n", encoding="utf-8")
        (bench / "large_v2_validation.py").write_text(
            "import pathlib, sys\npathlib.Path(sys.argv[0]).with_name('validator-ran').touch()\nsys.exit(3)\n",
            encoding="utf-8")
        (self.repo / ".gitattributes").write_text("* -text\n", encoding="utf-8")
        git(self.tmp, "init", "-q", str(self.repo))
        git(self.repo, "-c", "user.email=t@example.invalid", "-c", "user.name=t", "add", "-A")
        git(self.repo, "-c", "user.email=t@example.invalid", "-c", "user.name=t", "commit", "-qm", "pin")
        self.pin = git(self.repo, "rev-parse", "HEAD")
        self.sha = hashlib.sha256(b'{"tasks": []}\n').hexdigest()
        self.bench_root = self.tmp / "bench"
        (self.bench_root / "campaign").mkdir(parents=True)
        (self.bench_root / "campaign" / "results.jsonl").write_text("", encoding="utf-8")
        self.out = self.tmp / "work" / "evidence"
        self.venv = self.tmp / "work" / "venv"

    def run_script(self, *extra, **overrides):
        args = {"OutDir": self.out, "VenvDir": self.venv, "BenchRoot": self.bench_root,
                "HarnessPin": self.pin, "ManifestSha256": self.sha, "Python": sys.executable}
        args.update(overrides)
        command = [SHELL, "-NoProfile", "-ExecutionPolicy", "Bypass", "-File",
                   str(self.repo / "scripts" / SCRIPT.name)]
        for key, value in args.items():
            command += [f"-{key}", str(value)]
        result = subprocess.run(command + list(extra), capture_output=True, text=True, timeout=600)
        return result.returncode, result.stdout + result.stderr

    def commit(self, message):
        git(self.repo, "-c", "user.email=t@example.invalid", "-c", "user.name=t", "commit", "-qam", message)

    def assert_stop(self, needle, *extra, **overrides):
        code, output = self.run_script(*extra, **overrides)
        self.assertEqual(code, 1, output)
        self.assertIn("STOP:", output)
        self.assertIn(needle, output)
        return output

    def test_preflight_passes_and_creates_nothing(self):
        code, output = self.run_script("-PreflightOnly")
        self.assertEqual(code, 0, output)
        self.assertIn("PREFLIGHT OK", output)
        self.assertFalse(self.out.exists() or self.venv.exists())

    def test_dirty_clone_stops(self):
        (self.repo / "scripts" / "bench" / "large_v2_validation.py").write_text("pass\n", encoding="utf-8")
        self.assert_stop("uncommitted changes", "-PreflightOnly")

    def test_bench_code_after_the_pin_stops(self):
        (self.repo / "scripts" / "bench" / "large_v2_validation.py").write_text("pass\n", encoding="utf-8")
        self.commit("drift")
        self.assert_stop("differs from harness pin", "-PreflightOnly")

    def test_change_outside_scripts_bench_keeps_the_pin(self):
        (self.repo / "scripts" / SCRIPT.name).write_text(SCRIPT.read_text(encoding="utf-8") + "\n", encoding="utf-8")
        self.commit("docs-only change")
        code, output = self.run_script("-PreflightOnly")
        self.assertEqual(code, 0, output)

    def test_manifest_hash_mismatch_stops(self):
        self.assert_stop("SHA-256 is", "-PreflightOnly", ManifestSha256="0" * 64)

    def test_existing_output_folder_stops(self):
        self.out.mkdir(parents=True)
        self.assert_stop("already exists", "-PreflightOnly")

    def test_output_inside_bench_root_stops(self):
        self.assert_stop("must stay untouched", "-PreflightOnly", OutDir=self.bench_root / "evidence")

    def test_missing_bench_root_stops(self):
        self.assert_stop("does not exist", "-PreflightOnly", BenchRoot=self.tmp / "no-such-bench")

    def test_missing_python_stops(self):
        self.assert_stop("missing or older than 3.10", "-PreflightOnly", Python="no-such-python-zz")

    def test_failed_dependency_install_stops_before_validation(self):
        (self.repo / "scripts" / "bench" / "v2-validation-requirements.txt").write_text(
            "-e ./no-such-directory\n", encoding="utf-8")
        self.commit("bad requirement")
        self.assert_stop("Install pinned test tooling failed", HarnessPin=git(self.repo, "rev-parse", "HEAD"))
        self.assertFalse((self.repo / "scripts" / "bench" / "validator-ran").exists())
        self.assertFalse((self.out / "v2-validation.json").exists())

    def test_failed_validation_stops_before_tests_and_classifier(self):
        output = self.assert_stop("v2 behavioral validation failed with exit code 3")
        self.assertTrue((self.repo / "scripts" / "bench" / "validator-ran").exists())
        self.assertNotIn("Regression tests", output)
        self.assertNotIn("Locate large-task campaigns", output)
        self.assertEqual(sorted(p.name for p in self.bench_root.rglob("*")), ["campaign", "results.jsonl"])


if __name__ == "__main__":
    unittest.main()
