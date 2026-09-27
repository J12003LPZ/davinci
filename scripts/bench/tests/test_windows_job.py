"""Windows Job Object lifetime tests. They launch only tiny local Python children."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import time
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))

if sys.platform == "win32":
    import windows_job


def _pid_alive(pid):
    result = subprocess.run(["tasklist", "/FI", f"PID eq {pid}", "/NH"],
                            capture_output=True, text=True)
    return str(pid) in result.stdout


@unittest.skipUnless(sys.platform == "win32", "Windows Job Objects only")
class WindowsJobTests(unittest.TestCase):
    def test_normal_exit_captures_output_and_kills_detached_descendants(self):
        with tempfile.TemporaryDirectory() as tmp:
            pid_file = Path(tmp) / "grandchild.pid"
            script = (
                "import subprocess, sys\n"
                "child = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(60)'],"
                " creationflags=0x00000008)\n"  # DETACHED_PROCESS
                f"open({str(pid_file)!r}, 'w').write(str(child.pid))\n"
                "print('parent done')\n")
            result = windows_job.run([sys.executable, "-c", script], tmp, dict(os.environ), 30)
            self.assertEqual(result["exit"], 0, result)
            self.assertTrue(result["cleanup_complete"], result)
            self.assertIn("parent done", result["stdout"])
            grandchild = int(pid_file.read_text())
            self.assertFalse(_pid_alive(grandchild), "a detached descendant escaped the job")

    def test_timeout_terminates_the_tree_and_reports_timeout(self):
        with tempfile.TemporaryDirectory() as tmp:
            started = time.monotonic()
            result = windows_job.run(
                [sys.executable, "-c", "import time; time.sleep(60)"], tmp, dict(os.environ), 1)
            self.assertEqual(result["exit"], "timeout")
            self.assertTrue(result["cleanup_complete"], result)
            self.assertLess(time.monotonic() - started, 20)

    def test_nonzero_exit_is_preserved(self):
        with tempfile.TemporaryDirectory() as tmp:
            result = windows_job.run(
                [sys.executable, "-c", "import sys; sys.stderr.write('bad'); sys.exit(3)"],
                tmp, dict(os.environ), 30)
            self.assertEqual(result["exit"], 3)
            self.assertEqual(result["stderr"], "bad")
            self.assertTrue(result["cleanup_complete"])

    def test_launch_error_is_not_a_process_exit(self):
        with tempfile.TemporaryDirectory() as tmp:
            result = windows_job.run([str(Path(tmp) / "missing.exe")], tmp, dict(os.environ), 5)
            self.assertEqual(result["exit"], "launch_error")


if __name__ == "__main__":
    unittest.main()
