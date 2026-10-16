"""The agent-account boundary: what is built offline. Real account launches need the account and run in the spike."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import agent_account as aa

WINDOWS = sys.platform == "win32"


class EnvironmentTests(unittest.TestCase):
    def test_the_operator_profile_is_replaced(self):
        env = {"USERPROFILE": r"C:\Users\op", "HOME": r"C:\Users\op", "TEMP": r"C:\Users\op\T", "tmp": r"C:\Users\op\T",
               "APPDATA": "x", "LOCALAPPDATA": "x", "CODEX_HOME": r"C:\Users\op\.codex", "PATH": "p", "KEEP": "1"}
        out = aa.agent_environment(env, "h1agent", r"C:\Users\h1agent")
        self.assertEqual(out["USERPROFILE"], r"C:\Users\h1agent")
        self.assertEqual(out["TEMP"], os.path.join(r"C:\Users\h1agent", "AppData", "Local", "Temp"))
        self.assertEqual(out["USERNAME"], "h1agent")
        self.assertNotIn("CODEX_HOME", out)
        self.assertNotIn("tmp", out)  # no lower-case leftover pointing at the operator
        self.assertEqual((out["PATH"], out["KEEP"]), ("p", "1"))
        for value in out.values():
            self.assertNotIn(r"C:\Users\op", value)

    def test_the_agent_path_replaces_the_operator_path_when_declared(self):
        env = {"PATH": r"C:\Users\op\bin;C:\Windows"}
        with mock.patch.dict(os.environ, {aa.ENV_PATH: r"C:\Windows\System32;C:\Python"}):
            self.assertEqual(aa.agent_environment(env, "h1agent")["PATH"], r"C:\Windows\System32;C:\Python")
        with mock.patch.dict(os.environ, {}):
            os.environ.pop(aa.ENV_PATH, None)
            self.assertEqual(aa.agent_environment(env, "h1agent")["PATH"], env["PATH"])

    def test_configured_reads_only_the_account_name(self):
        with mock.patch.dict(os.environ, {aa.ENV_USER: "  "}):
            self.assertIsNone(aa.configured())
        with mock.patch.dict(os.environ, {aa.ENV_USER: "h1agent"}):
            self.assertEqual(aa.configured(), "h1agent")

    def test_a_configured_account_needs_its_credential_file(self):
        with mock.patch.dict(os.environ, {aa.ENV_USER: "h1agent", aa.ENV_CREDENTIAL: "missing.bin"}):
            with mock.patch.object(aa.sys, "platform", "win32"):
                with self.assertRaises(ValueError):
                    aa.active_launcher()

    def test_no_account_means_no_launcher(self):
        with mock.patch.dict(os.environ, {}, clear=False):
            os.environ.pop(aa.ENV_USER, None)
            self.assertIsNone(aa.active_launcher())

    def test_a_configured_account_is_refused_off_windows(self):
        with mock.patch.dict(os.environ, {aa.ENV_USER: "h1agent"}):
            with mock.patch.object(aa.sys, "platform", "linux"):
                with self.assertRaises(ValueError):
                    aa.active_launcher()


class PermissionCommandTests(unittest.TestCase):
    def test_lock_removes_inheritance_before_it_grants(self):
        first, second = aa.lock_commands(r"C:\v", r"PC\op")
        self.assertEqual(first[-1], "/inheritance:r")
        self.assertIn("PC\\op:(OI)(CI)F", second)
        self.assertIn("SYSTEM:(OI)(CI)F", second)
        self.assertNotIn("Users", " ".join(second))

    def test_deny_covers_the_whole_tree(self):
        self.assertEqual(aa.deny_command(r"C:\v", "h1agent")[-2:], ["/deny", "h1agent:(OI)(CI)F"])

    def test_a_file_grant_carries_no_inheritance_flags(self):
        self.assertEqual(aa.grant_command("f.exe", "h1agent", "RX", directory=False)[-1], "h1agent:RX")
        self.assertEqual(aa.grant_command("d", "h1agent", "M")[-1], "h1agent:(OI)(CI)M")

    def test_revoke_removes_only_the_grant(self):
        self.assertEqual(aa.revoke_command("d", "h1agent"), ["icacls", "d", "/remove:g", "h1agent"])

    def test_a_surviving_entry_after_revocation_is_an_error(self):
        with mock.patch.object(aa, "_run"), mock.patch.object(aa, "has_entry", return_value=True):
            with self.assertRaises(OSError):
                aa.revoke_run_directory("d", "h1agent")
        with mock.patch.object(aa, "_run"), mock.patch.object(aa, "has_entry", return_value=False):
            aa.revoke_run_directory("d", "h1agent")

    def test_an_unreadable_access_list_counts_as_still_present(self):
        with mock.patch.object(aa.subprocess, "run", return_value=subprocess.CompletedProcess([], 5, "", "")):
            self.assertTrue(aa.has_entry("d", "h1agent"))


class SelfTestTests(unittest.TestCase):
    """The self-test must not call a launch failure a denial, and must fail when the agent can read the vault."""

    def run_selftest(self, results_for):
        with tempfile.TemporaryDirectory() as vault, tempfile.TemporaryDirectory() as results:
            def probe(user, credential, command, workdir, timeout=60):
                return results_for(command)
            with mock.patch.object(aa, "probe", probe), mock.patch.object(aa, "grant_run_directory"), \
                    mock.patch.object(aa, "revoke_run_directory"), mock.patch.object(aa, "has_entry", return_value=False):
                return aa.selftest("h1agent", "cred", vault, results, ["agent.exe"], ["python"])

    @staticmethod
    def result(code):
        return {"exit": code, "stdout": "", "stderr": "", "cleanup_complete": True}

    def decide(self, command):
        denied = command[:2] == ["cmd.exe", "/c"] and command[2] in ("type", "dir")
        return self.result(1 if denied else 0)

    def test_a_working_boundary_passes(self):
        passed, rows = self.run_selftest(self.decide)
        self.assertTrue(passed, rows)
        self.assertTrue(any(r["check"] == "read the vault" and r["expect"] == "denied" for r in rows))

    def test_a_readable_vault_fails(self):
        passed, rows = self.run_selftest(lambda command: self.result(0))
        self.assertFalse(passed)

    def test_a_launch_failure_is_not_a_denial(self):
        passed, rows = self.run_selftest(lambda command: {"exit": "launch_error", "stdout": "", "stderr": "",
                                                          "cleanup_complete": True})
        self.assertFalse(passed)
        self.assertFalse([r for r in rows if r["expect"] == "denied" and r["ok"]])

    def test_unfinished_cleanup_fails_the_check(self):
        passed, _ = self.run_selftest(lambda c: {**self.decide(c), "cleanup_complete": False})
        self.assertFalse(passed)

    def test_a_tool_the_agent_cannot_run_fails(self):
        def decide(command):
            return self.result(1 if command[0] == "python" else self.decide(command)["exit"])
        passed, rows = self.run_selftest(decide)
        self.assertFalse(passed)
        self.assertTrue([r for r in rows if r["check"] == "run tool python" and not r["ok"]])


@unittest.skipUnless(WINDOWS, "Windows job objects and DPAPI")
class WindowsTests(unittest.TestCase):
    def test_the_job_runner_uses_an_injected_launcher(self):
        import windows_job
        seen = []

        def launcher(command, workdir, env, out, err):
            seen.append(command)
            return windows_job._popen(command, workdir, env, out, err)
        with tempfile.TemporaryDirectory() as cwd:
            result = windows_job.run(["cmd.exe", "/c", "echo hi"], cwd, dict(os.environ), 30, launcher)
        self.assertEqual(seen, [["cmd.exe", "/c", "echo hi"]])
        self.assertEqual((result["exit"], result["cleanup_complete"]), (0, True))
        self.assertIn("hi", result["stdout"])

    def test_a_launcher_error_is_a_launch_error_with_clean_job(self):
        import windows_job

        def launcher(*args):
            raise OSError(1326, "logon failure")
        with tempfile.TemporaryDirectory() as cwd:
            result = windows_job.run(["cmd.exe"], cwd, dict(os.environ), 30, launcher)
        self.assertEqual(result["exit"], "launch_error")
        self.assertTrue(result["cleanup_complete"])

    def test_the_credential_round_trips_and_is_not_stored_in_clear(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory, "credential.bin")
            aa.store_credential(path, "s3cret-Pass!")
            self.assertNotIn(b"s3cret", path.read_bytes())
            self.assertEqual(aa.load_credential(path), "s3cret-Pass!")

    def test_the_environment_block_is_sorted_and_double_terminated(self):
        block = aa.environment_block({"b": "2", "A": "1"})
        self.assertEqual(block[:].rstrip("\0"), "A=1\0b=2")
        self.assertTrue(block[:].endswith("\0\0"))

    def test_a_process_object_reports_exit_codes_and_timeouts(self):
        child = subprocess.Popen(["cmd.exe", "/c", "exit 7"])
        proc = aa.AgentProcess(int(child._handle), _duplicate_thread())
        self.assertEqual(proc.wait(10), 7)
        child.wait()
        slow = subprocess.Popen(["cmd.exe", "/c", "ping -n 30 127.0.0.1 >nul"])
        proc = aa.AgentProcess(int(slow._handle), _duplicate_thread())
        with self.assertRaises(subprocess.TimeoutExpired):
            proc.wait(0.2)
        proc.kill()
        self.assertIsNotNone(proc.wait(10))
        slow.wait()


def _duplicate_thread():
    """A throwaway handle for AgentProcess to close, standing in for the suspended thread handle."""
    import ctypes
    from ctypes import wintypes
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel32.CreateEventW.restype = wintypes.HANDLE
    return kernel32.CreateEventW(None, True, False, None)


if __name__ == "__main__":
    unittest.main()
