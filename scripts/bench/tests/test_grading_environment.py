"""The agent and the grader must run Python in the same text I/O mode.

Stage 1 (campaign v2-stage1-20261015-run2, DaVinci m1 rep 0) failed one grader check
for a reason outside the agent's control: it wrote and ran a public test that prints
"café" from a child process and reads it back with text=True. That test passed in the
agent's environment (PYTHONUTF8=1) and failed in the grader's, which inherited the
launching shell's PYTHONIOENCODING=utf-8:surrogateescape: the child printed UTF-8 while
the grader's inner pytest, not in UTF-8 mode, decoded with the legacy Windows code page.
A clean shell passes, so the outcome depended on the launching shell. These tests pin the
fix: both environments take PYTHON_TEXT_ENV from one place, whatever the host sets.

Everything runs against throwaway directories; nothing reads a campaign directory.
"""
import json
import locale
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import textwrap
import unittest
from unittest.mock import patch

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import bench
import private_suite
import runner

CLI = textwrap.dedent('''\
    import sys

    def main(path):
        with open(path, encoding="utf-8") as handle:
            for line in sorted(handle.read().splitlines()):
                print(line)

    if __name__ == "__main__":
        main(sys.argv[1])
''')

# The shape of the test DaVinci wrote in Stage 1: a child process prints non-ASCII text
# and the parent decodes it with the interpreter's default (text=True).
CHILD_ROUND_TRIP = textwrap.dedent('''\
    import pathlib
    import subprocess
    import sys
    import tempfile
    import unittest


    class RoundTrip(unittest.TestCase):
        def test_cli_prints_non_ascii_values(self):
            with tempfile.TemporaryDirectory() as directory:
                data = pathlib.Path(directory) / "config.txt"
                data.write_text("z=last\\na=caf\\u00e9\\n", encoding="utf-8")
                cli = pathlib.Path(__file__).resolve().parent / "cli.py"
                result = subprocess.run([sys.executable, str(cli), str(data)],
                                        check=True, capture_output=True, text=True)
            self.assertEqual(result.stdout, "a=caf\\u00e9\\nz=last\\n")
''')

HOST_USES_UTF8_BY_DEFAULT = locale.getpreferredencoding(False).lower().replace("-", "") == "utf8"


def workspace(root):
    root = Path(root)
    (root / "cli.py").write_text(CLI, encoding="utf-8")
    (root / "test_round_trip.py").write_text(CHILD_ROUND_TRIP, encoding="utf-8")
    return root


# The variable a launching shell can set that makes a child print UTF-8 while the
# parent still decodes with the locale code page (what the Stage 1 grader inherited).
SHELL_IO = {"PYTHONIOENCODING": "utf-8:surrogateescape"}


def env_without_utf8(extra=None):
    """The host environment with the interpreter's mode variables removed, then `extra` added."""
    env = {key: value for key, value in os.environ.items() if key not in ("PYTHONUTF8", "PYTHONIOENCODING")}
    env.update(extra or {})
    return env


HOSTS = {"clean shell": {}, "shell exports PYTHONIOENCODING": SHELL_IO}


def normalized(checked):
    """Result fields that must not depend on the host, with timings removed."""
    text = re.sub(r"in \d+(\.\d+)?s", "in <t>s", checked["stdout"] + checked["stderr"])
    return {"pass": checked["pass"], "exit": checked["exit"], "output": text}


COMMAND = [sys.executable, "-m", "unittest", "-q", "test_round_trip"]


class GradingEnvironmentTests(unittest.TestCase):
    def test_agent_and_grader_declare_the_same_python_text_mode(self):
        captured = {}

        def execute(command, workdir, env, timeout):
            captured.update(env)
            return {"exit": 0, "stdout": "", "stderr": "", "cleanup_complete": True}

        with tempfile.TemporaryDirectory() as temporary, \
                patch.object(private_suite, "execute", execute), \
                patch.dict(os.environ, {"PYTHONUTF8": "0"}):
            private_suite.run_check(COMMAND, temporary)
            agent = runner.controlled_environment(os.environ, temporary)
        for key, value in runner.PYTHON_TEXT_ENV.items():
            self.assertEqual(captured[key], value, "grader does not set " + key)
            self.assertEqual(agent[key], value, "agent does not set " + key)

    def test_host_value_cannot_override_the_declared_mode(self):
        for host in (None, "0", "1"):
            with self.subTest(host=host), tempfile.TemporaryDirectory() as temporary:
                captured = {}

                def execute(command, workdir, env, timeout):
                    captured.update(env)
                    return {"exit": 0, "stdout": "", "stderr": "", "cleanup_complete": True}

                environment = env_without_utf8({"PYTHONUTF8": host} if host is not None else {})
                with patch.object(private_suite, "execute", execute), patch.dict(os.environ, environment, clear=True):
                    private_suite.run_check(COMMAND, temporary)
                self.assertEqual(captured["PYTHONUTF8"], "1")

    def test_container_grading_receives_the_declared_mode(self):
        self.assertTrue(set(runner.PYTHON_TEXT_ENV) <= runner._CONTAINER_ENV_KEYS)

    def test_grader_evidence_records_the_environment_version(self):
        measured = {"exit": 0, "stdout": "", "stderr": "", "cleanup_complete": True}
        with tempfile.TemporaryDirectory() as temporary, patch.object(private_suite, "execute", return_value=measured):
            workdir = Path(temporary) / "run"
            workdir.mkdir()
            evidence = {}
            bench.run_grading_check(COMMAND, workdir, 5, "grader", evidence)
            stored = json.loads(Path(str(workdir) + ".grader.json").read_text(encoding="utf-8"))
        self.assertEqual(stored["grading_env_version"], runner.GRADING_ENV_VERSION)
        self.assertEqual(stored["python_text_env"], runner.PYTHON_TEXT_ENV)
        self.assertGreaterEqual(runner.GRADING_ENV_VERSION, 2)

    def test_cafe_round_trip_passes_in_the_grader_whatever_the_host_exports(self):
        for label, extra in HOSTS.items():
            with self.subTest(host=label), tempfile.TemporaryDirectory() as temporary:
                root = workspace(temporary)
                with patch.dict(os.environ, env_without_utf8(extra), clear=True):
                    checked = private_suite.run_check(COMMAND, root, timeout=60)
                self.assertTrue(checked["pass"], checked["stdout"] + checked["stderr"])

    def test_identical_workspaces_grade_identically_under_different_host_settings(self):
        results = []
        hosts = [env_without_utf8(), env_without_utf8(SHELL_IO), env_without_utf8({"PYTHONUTF8": "1"}),
                 env_without_utf8({"PYTHONUTF8": "0", **SHELL_IO})]
        for host in hosts:
            with tempfile.TemporaryDirectory() as temporary:
                root = workspace(temporary)
                with patch.dict(os.environ, host, clear=True):
                    results.append(normalized(private_suite.run_check(COMMAND, root, timeout=60)))
        for other in results[1:]:
            self.assertEqual(results[0], other)
        self.assertTrue(results[0]["pass"])

    @unittest.skipIf(HOST_USES_UTF8_BY_DEFAULT, "host default is already UTF-8; the unmatched case cannot be shown here")
    def test_unmatched_environment_reproduces_the_stage_1_failure(self):
        """Guards the tests above against passing vacuously on a host that can fail."""
        with tempfile.TemporaryDirectory() as temporary:
            root = workspace(temporary)
            unmatched = subprocess.run(COMMAND, cwd=root, env=env_without_utf8(SHELL_IO),
                                       capture_output=True, text=True, timeout=60)
            matched = subprocess.run(COMMAND, cwd=root, env=env_without_utf8({**SHELL_IO, **runner.PYTHON_TEXT_ENV}),
                                     capture_output=True, text=True, timeout=60)
        self.assertNotEqual(unmatched.returncode, 0, "expected a UTF-8 child and a locale-decoding parent to disagree")
        self.assertEqual(matched.returncode, 0, matched.stdout + matched.stderr)


if __name__ == "__main__":
    unittest.main()
