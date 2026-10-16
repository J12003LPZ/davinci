"""The restricted-token boundary: pure parts everywhere, the real token on Windows."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import agent_boundary as ab

WINDOWS = sys.platform == "win32"
OPERATOR_HOME = r"C:\Users\op"


class EnvironmentTests(unittest.TestCase):
    def test_every_profile_variable_moves_into_the_agent_directory(self):
        env = {"USERPROFILE": OPERATOR_HOME, "HOME": OPERATOR_HOME, "TEMP": OPERATOR_HOME + r"\T", "tmp": OPERATOR_HOME + r"\T",
               "APPDATA": OPERATOR_HOME, "LOCALAPPDATA": OPERATOR_HOME, "CODEX_HOME": OPERATOR_HOME + r"\.codex",
               "DAVINCI_CODING_AGENT_DIR": r"C:\run\_agent", "PATH": "p", "KEEP": "1"}
        out = ab.agent_environment(env)
        for key in ("USERPROFILE", "HOME", "APPDATA", "LOCALAPPDATA", "TEMP", "TMP", "CODEX_HOME"):
            self.assertTrue(out[key].startswith(os.path.join(r"C:\run\_agent", "home")), key)
        self.assertNotIn("tmp", out)  # no lower-case duplicate left pointing at the operator
        self.assertEqual((out["PATH"], out["KEEP"]), ("p", "1"))
        for value in out.values():
            self.assertNotIn(OPERATOR_HOME, value)

    def test_the_agent_needs_its_directory(self):
        with self.assertRaises(ValueError):
            ab.agent_environment({"PATH": "p"})

    def test_the_agent_path_replaces_the_operator_path_when_declared(self):
        env = {"PATH": OPERATOR_HOME + r"\bin;C:\Windows", "DAVINCI_CODING_AGENT_DIR": "a"}
        with mock.patch.dict(os.environ, {ab.ENV_PATH: r"C:\Windows\System32;C:\Python"}):
            self.assertEqual(ab.agent_environment(env)["PATH"], r"C:\Windows\System32;C:\Python")
        with mock.patch.dict(os.environ, {}):
            os.environ.pop(ab.ENV_PATH, None)
            self.assertEqual(ab.agent_environment(env)["PATH"], env["PATH"])

    def test_prepare_home_creates_the_directories_the_tools_expect(self):
        with tempfile.TemporaryDirectory() as directory:
            home = ab.prepare_home(directory)
            for sub in ("AppData/Roaming", "AppData/Local/Temp", ".codex"):
                self.assertTrue((home / sub).is_dir(), sub)

    def test_the_codex_login_is_copied_into_the_stand_in_home_and_never_linked(self):
        with tempfile.TemporaryDirectory() as root:
            source = Path(root, "operator", ".codex")
            source.mkdir(parents=True)
            Path(source, "auth.json").write_text('{"k": "v"}', encoding="utf-8")
            agent = Path(root, "agent")
            with mock.patch.dict(os.environ, {"CODEX_HOME": str(source)}):
                target = ab.copy_codex_login(agent)
            self.assertEqual(target, ab.agent_home(agent) / ".codex" / "auth.json")
            self.assertEqual(target.read_text(encoding="utf-8"), '{"k": "v"}')
            Path(source, "auth.json").unlink()
            with mock.patch.dict(os.environ, {"CODEX_HOME": str(source)}):
                with self.assertRaises(ValueError):
                    ab.copy_codex_login(agent)

    def test_the_boundary_is_selected_only_by_its_one_value(self):
        with mock.patch.dict(os.environ, {ab.ENV_BOUNDARY: ""}):
            self.assertFalse(ab.configured())
        with mock.patch.dict(os.environ, {ab.ENV_BOUNDARY: ab.MODE}):
            self.assertTrue(ab.configured())
        with mock.patch.dict(os.environ, {ab.ENV_BOUNDARY: "yes"}):
            with self.assertRaises(ValueError):
                ab.configured()

    def test_no_boundary_means_no_launcher_and_a_boundary_is_refused_off_windows(self):
        with mock.patch.dict(os.environ, {}):
            os.environ.pop(ab.ENV_BOUNDARY, None)
            self.assertIsNone(ab.active_launcher())
        with mock.patch.dict(os.environ, {ab.ENV_BOUNDARY: ab.MODE}), mock.patch.object(ab.sys, "platform", "linux"):
            with self.assertRaises(ValueError):
                ab.active_launcher()


class PermissionTests(unittest.TestCase):
    def test_lock_removes_inheritance_before_it_grants(self):
        first, second = ab.lock_commands(r"C:\v", r"PC\op")
        self.assertEqual(first[-1], "/inheritance:r")
        self.assertIn("PC\\op:(OI)(CI)F", second)
        self.assertIn("SYSTEM:(OI)(CI)F", second)
        self.assertNotIn("Users", " ".join(second))

    def test_grants_name_users_by_sid_not_by_a_localized_name(self):
        self.assertEqual(ab.grant_command("d", "M")[-1], "*S-1-5-32-545:(OI)(CI)M")
        self.assertEqual(ab.grant_command("f.exe", "RX", directory=False)[-1], "*S-1-5-32-545:RX")
        self.assertEqual(ab.revoke_command("d"), ["icacls", "d", "/remove:g", "*S-1-5-32-545"])

    def test_the_access_list_reader_spots_users_and_everyone(self):
        base = "O:S-1-5-21-1G:S-1-5-21-2D:PAI(A;OICI;FA;;;S-1-5-21-1)(A;OICI;FA;;;SY)"
        self.assertFalse(ab.grants_users(base))
        for sid in ("BU", "WD", "S-1-5-32-545", "S-1-1-0"):
            self.assertTrue(ab.grants_users(base + "(A;OICI;0x1200a9;;;%s)" % sid), sid)
        self.assertFalse(ab.grants_users(base + "(D;OICI;FA;;;BU)"))  # a deny is not a grant
        self.assertFalse(ab.grants_users(base + "S:AI(A;;FA;;;BU)"))  # the audit list is not the access list

    def test_a_surviving_entry_after_revocation_is_an_error(self):
        with mock.patch.object(ab, "_run"), mock.patch.object(ab, "has_entry", return_value=True):
            with self.assertRaises(OSError):
                ab.revoke_run_directory("d")
        with mock.patch.object(ab, "_run") as run, mock.patch.object(ab, "has_entry", return_value=False):
            ab.revoke_run_directory("d")
        run.assert_called_once_with(ab.revoke_command("d"))

    def test_an_unreadable_access_list_counts_as_still_present(self):
        with mock.patch.object(ab, "dacl_string", side_effect=OSError(5, "denied")):
            self.assertTrue(ab.has_entry("d"))


class SelfTestTests(unittest.TestCase):
    """The self-test must not call a launch failure a denial, and must fail when the agent can read the vault."""

    def run_selftest(self, results_for):
        with tempfile.TemporaryDirectory() as vault, tempfile.TemporaryDirectory() as results:
            def probe(command, workdir, agent_dir, timeout=60):
                return results_for(command)
            with mock.patch.object(ab, "probe", probe), mock.patch.object(ab, "grant"), \
                    mock.patch.object(ab, "revoke_run_directory"), mock.patch.object(ab, "has_entry", return_value=False):
                return ab.selftest(vault, results, ["agent.exe"], ["python"])

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
        self.assertFalse(self.run_selftest(lambda command: self.result(0))[0])

    def test_a_launch_failure_is_not_a_denial(self):
        passed, rows = self.run_selftest(lambda c: {"exit": "launch_error", "stdout": "", "stderr": "", "cleanup_complete": True})
        self.assertFalse(passed)
        self.assertFalse([r for r in rows if r["expect"] == "denied" and r["ok"]])

    def test_unfinished_cleanup_fails_the_check(self):
        self.assertFalse(self.run_selftest(lambda c: {**self.decide(c), "cleanup_complete": False})[0])

    def test_a_tool_the_agent_cannot_run_fails(self):
        def decide(command):
            return self.result(1 if command[0] == "python" else self.decide(command)["exit"])
        passed, rows = self.run_selftest(decide)
        self.assertFalse(passed)
        self.assertTrue([r for r in rows if r["check"] == "run tool python" and not r["ok"]])


@unittest.skipUnless(WINDOWS, "restricted tokens and job objects")
class WindowsTests(unittest.TestCase):
    def run_restricted(self, command, workdir, agent_dir):
        return self.run_with_path(command, workdir, agent_dir, r"C:\Windows\System32;C:\Windows")

    def run_with_path(self, command, workdir, agent_dir, path):
        import windows_job
        env = dict(os.environ, DAVINCI_CODING_AGENT_DIR=str(agent_dir))
        with mock.patch.dict(os.environ, {ab.ENV_PATH: path}):  # the harness strips BENCH_* from the child env
            return windows_job.run(command, str(workdir), env, 60, ab.launch)

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

    def test_a_launcher_error_is_a_launch_error_with_a_clean_job(self):
        import windows_job

        def launcher(*args):
            raise OSError(5, "denied")
        with tempfile.TemporaryDirectory() as cwd:
            result = windows_job.run(["cmd.exe"], cwd, dict(os.environ), 30, launcher)
        self.assertEqual((result["exit"], result["cleanup_complete"]), ("launch_error", True))

    def test_a_command_missing_from_the_agent_path_does_not_launch(self):
        with tempfile.TemporaryDirectory() as directory:
            ab.prepare_home(directory)
            # python.exe is on the operator's PATH but not on the agent's: the agent's PATH must decide.
            result = self.run_restricted(["python.exe", "--version"], directory, directory)
        self.assertEqual(result["exit"], "launch_error")

    def test_the_token_carries_users_everyone_and_the_logon_session(self):
        import ctypes
        token = ab.restricted_token()
        try:
            self.assertEqual(ab.restricting_sid_count(token), 3)
        finally:
            ctypes.WinDLL("kernel32").CloseHandle(token)

    @unittest.skipUnless(os.path.isfile(r"C:\Program Files\Git\cmd\git.exe"), "needs the machine-wide Git")
    def test_a_program_that_initialises_the_desktop_starts(self):
        # Without the logon SID git dies with 0xC0000142 (DLL initialisation failed).
        with tempfile.TemporaryDirectory() as directory:
            ab.prepare_home(directory)
            result = self.run_with_path(["git", "--version"], directory, directory,
                                        r"C:\Windows\System32;C:\Program Files\Git\cmd")
        self.assertEqual(result["exit"], 0, result)

    def test_the_restricted_process_runs_inside_the_job_and_cleans_up(self):
        with tempfile.TemporaryDirectory() as directory:
            ab.prepare_home(directory)
            ok = self.run_restricted(["cmd.exe", "/c", "echo restricted-ok"], directory, directory)
            slow = self.run_restricted(["cmd.exe", "/c", "ping -n 30 127.0.0.1 >nul"], directory, directory) \
                if False else None
        self.assertEqual((ok["exit"], ok["cleanup_complete"]), (0, True))
        self.assertIn("restricted-ok", ok["stdout"])

    def test_the_restricted_token_cannot_read_what_only_the_operator_may(self):
        with tempfile.TemporaryDirectory() as directory:
            secret = Path(directory, "secret.txt")
            secret.write_text("canary", encoding="utf-8")
            if ab.grants_users(ab.dacl_string(directory)):
                self.skipTest("this temporary directory is readable by Users, so it proves nothing")
            workdir = Path(directory, "run")
            workdir.mkdir()
            ab.prepare_home(directory)
            ab.grant(directory, "M")  # lets the child start; the secret keeps only its inherited access
            ab._run(["icacls", str(secret), "/inheritance:r"])
            ab._run(["icacls", str(secret), "/grant:r", ab.operator_name() + ":F"])
            result = self.run_restricted(["cmd.exe", "/c", "type", str(secret)], directory, directory)
            ab._run(["icacls", str(directory), "/remove:g", ab.USERS])
        self.assertIsInstance(result["exit"], int)
        self.assertNotEqual(result["exit"], 0)
        self.assertNotIn("canary", result["stdout"])

    def test_a_process_object_reports_exit_codes_and_timeouts(self):
        child = subprocess.Popen(["cmd.exe", "/c", "exit 7"])
        proc = ab.AgentProcess(int(child._handle), _event())
        self.assertEqual(proc.wait(10), 7)
        child.wait()
        slow = subprocess.Popen(["cmd.exe", "/c", "ping -n 30 127.0.0.1 >nul"])
        proc = ab.AgentProcess(int(slow._handle), _event())
        with self.assertRaises(subprocess.TimeoutExpired):
            proc.wait(0.2)
        proc.kill()
        self.assertIsNotNone(proc.wait(10))
        slow.wait()

    def test_the_environment_block_is_sorted_and_double_terminated(self):
        block = ab.environment_block({"b": "2", "A": "1"})
        self.assertEqual(block[:].rstrip("\0"), "A=1\0b=2")
        self.assertTrue(block[:].endswith("\0\0"))


def _event():
    """A throwaway handle for AgentProcess to close, standing in for the suspended thread handle."""
    import ctypes
    from ctypes import wintypes
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel32.CreateEventW.restype = wintypes.HANDLE
    return kernel32.CreateEventW(None, True, False, None)


if __name__ == "__main__":
    unittest.main()
