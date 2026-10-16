"""Run benchmark agents under a separate Windows account, so the private suite is unreadable to them.

The operator account owns the suite vault, the results root and all grading. The agent account is a
standard local user that only ever runs the agent processes (DaVinci, Codex). File permissions then
enforce what the transcript scan can only detect afterwards.

    python agent_account.py store-credential --file F          # DPAPI blob, readable by this operator only
    python agent_account.py lock DIR [DIR ...]                  # operator and SYSTEM only, no inheritance
    python agent_account.py grant-read PATH [PATH ...]          # toolchain and pinned binaries
    python agent_account.py grant-write PATH [PATH ...]         # the campaign agent directory
    python agent_account.py selftest --vault V --results R --binary B [--binary B ...]

Selected by BENCH_AGENT_USER (account name) and BENCH_AGENT_CREDENTIAL (the DPAPI file). Without
BENCH_AGENT_USER nothing changes and the old diagnostic-only behaviour applies.
"""
import argparse
import ctypes
from ctypes import wintypes
import getpass
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

ENV_USER = "BENCH_AGENT_USER"
ENV_CREDENTIAL = "BENCH_AGENT_CREDENTIAL"
ENV_PATH = "BENCH_AGENT_PATH"
# Profile locations that belong to the operator and are unreadable to the agent account.
PROFILE_KEYS = frozenset({
    "USERPROFILE", "HOME", "HOMEDRIVE", "HOMEPATH", "APPDATA", "LOCALAPPDATA", "TEMP", "TMP",
    "USERNAME", "ONEDRIVE", "ONEDRIVECONSUMER", "CODEX_HOME", "XDG_CONFIG_HOME", "XDG_CACHE_HOME"})
CREATE_SUSPENDED = 0x00000004
CREATE_UNICODE_ENVIRONMENT = 0x00000400
CREATE_NEW_PROCESS_GROUP = 0x00000200
LOGON_WITH_PROFILE = 1
STARTF_USESTDHANDLES = 0x00000100
STILL_ACTIVE = 259
WAIT_TIMEOUT = 0x102
INFINITE = 0xFFFFFFFF


def configured():
    """The account name when this campaign must run its agents under another account, else None."""
    user = os.environ.get(ENV_USER, "").strip()
    return user or None


def operator_name():
    return os.environ.get("USERDOMAIN", ".") + "\\" + getpass.getuser()


def agent_environment(env, user, profile_root=None):
    """Replace the operator's profile variables with the agent account's, so no tool looks inside the operator's profile."""
    profile = Path(profile_root) if profile_root else Path(os.environ.get("SystemDrive", "C:") + "\\", "Users", user)
    local = profile / "AppData" / "Local"
    result = {k: v for k, v in env.items() if k.upper() not in PROFILE_KEYS}
    result.update({
        "USERPROFILE": str(profile), "HOME": str(profile), "USERNAME": user,
        "APPDATA": str(profile / "AppData" / "Roaming"), "LOCALAPPDATA": str(local),
        "TEMP": str(local / "Temp"), "TMP": str(local / "Temp")})
    # The operator's PATH names dozens of tools inside the operator's profile. The agent gets only
    # the directories that were granted to it, plus the system ones.
    path = os.environ.get(ENV_PATH, "").strip()
    if path:
        result["PATH"] = path
    return result


# ---- permissions (icacls argument builders are pure so they test on every OS) ----

def _entry(principal, rights, directory=True):
    return "%s:%s%s" % (principal, "(OI)(CI)" if directory else "", rights)


def lock_commands(path, operator):
    """Operator, SYSTEM and Administrators only, with inheritance removed: nobody else keeps an implied right."""
    return [["icacls", str(path), "/inheritance:r"],
            ["icacls", str(path), "/grant:r", _entry(operator, "F"), _entry("SYSTEM", "F")]]


def deny_command(path, user):
    return ["icacls", str(path), "/deny", _entry(user, "F")]


def grant_command(path, user, rights, directory=True):
    return ["icacls", str(path), "/grant", _entry(user, rights, directory)]


def revoke_command(path, user):
    return ["icacls", str(path), "/remove:g", user]


def _run(args):
    result = subprocess.run(args, capture_output=True, text=True)
    if result.returncode != 0:
        raise OSError("permission change failed (exit %s)" % result.returncode)


def lock(path, user=None):
    for command in lock_commands(path, operator_name()):
        _run(command)
    if user:
        _run(deny_command(path, user))


def grant(path, user, rights):
    _run(grant_command(path, user, rights, Path(path).is_dir()))


def has_entry(path, user):
    """True when the agent account still appears in the directory's access list."""
    result = subprocess.run(["icacls", str(path)], capture_output=True, text=True)
    return result.returncode != 0 or user.lower() in result.stdout.lower()


def grant_run_directory(workdir, user):
    grant(workdir, user, "M")


def revoke_run_directory(workdir, user):
    """Take the run directory back before the hidden tests are copied into it; fail loudly if the entry survives."""
    _run(revoke_command(workdir, user))
    if has_entry(workdir, user):
        raise OSError("agent account still has access to a graded run directory")


# ---- credential, kept outside the repository and bound to the operator by DPAPI ----

class _Blob(ctypes.Structure):
    _fields_ = [("cbData", wintypes.DWORD), ("pbData", ctypes.POINTER(ctypes.c_char))]


def _blob(data):
    buffer = ctypes.create_string_buffer(data, len(data))
    return _Blob(len(data), ctypes.cast(buffer, ctypes.POINTER(ctypes.c_char))), buffer


def _protect(data, decrypt):
    crypt32 = ctypes.WinDLL("crypt32", use_last_error=True)
    kernel32 = ctypes.WinDLL("kernel32")
    source, keep = _blob(data)
    out = _Blob()
    call = crypt32.CryptUnprotectData if decrypt else crypt32.CryptProtectData
    if not call(ctypes.byref(source), None, None, None, None, 0, ctypes.byref(out)):
        raise OSError(ctypes.get_last_error(), "DPAPI failed")
    try:
        return ctypes.string_at(out.pbData, out.cbData)
    finally:
        kernel32.LocalFree(out.pbData)
        del keep


def store_credential(path, password):
    Path(path).write_bytes(_protect(password.encode("utf-8"), False))


def load_credential(path):
    return _protect(Path(path).read_bytes(), True).decode("utf-8")


# ---- launching ----

class _StartupInfo(ctypes.Structure):
    _fields_ = [("cb", wintypes.DWORD), ("lpReserved", wintypes.LPWSTR), ("lpDesktop", wintypes.LPWSTR),
                ("lpTitle", wintypes.LPWSTR), ("dwX", wintypes.DWORD), ("dwY", wintypes.DWORD),
                ("dwXSize", wintypes.DWORD), ("dwYSize", wintypes.DWORD),
                ("dwXCountChars", wintypes.DWORD), ("dwYCountChars", wintypes.DWORD),
                ("dwFillAttribute", wintypes.DWORD), ("dwFlags", wintypes.DWORD),
                ("wShowWindow", wintypes.WORD), ("cbReserved2", wintypes.WORD),
                ("lpReserved2", ctypes.c_void_p), ("hStdInput", wintypes.HANDLE),
                ("hStdOutput", wintypes.HANDLE), ("hStdError", wintypes.HANDLE)]


class _ProcessInfo(ctypes.Structure):
    _fields_ = [("hProcess", wintypes.HANDLE), ("hThread", wintypes.HANDLE),
                ("dwProcessId", wintypes.DWORD), ("dwThreadId", wintypes.DWORD)]


def environment_block(env):
    """UTF-16 block of NAME=value entries sorted case-insensitively, ended by two NULs."""
    entries = sorted(("%s=%s" % (k, v) for k, v in env.items()), key=lambda item: item.split("=", 1)[0].upper())
    return ctypes.create_unicode_buffer("\0".join(entries) + "\0\0", len("\0".join(entries)) + 2)


class AgentProcess:
    """The slice of subprocess.Popen that windows_job uses: _handle, wait, poll, kill."""

    def __init__(self, process_handle, thread_handle):
        self._handle = int(process_handle)
        self._thread = int(thread_handle)
        self._kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
        self._kernel32.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
        self._kernel32.GetExitCodeProcess.argtypes = [wintypes.HANDLE, ctypes.POINTER(wintypes.DWORD)]
        self._kernel32.TerminateProcess.argtypes = [wintypes.HANDLE, wintypes.UINT]
        self._kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
        self.returncode = None
        self._kernel32.CloseHandle(self._thread)

    def poll(self):
        if self.returncode is None:
            code = wintypes.DWORD()
            if self._kernel32.GetExitCodeProcess(self._handle, ctypes.byref(code)) and code.value != STILL_ACTIVE:
                self.returncode = code.value
        return self.returncode

    def wait(self, timeout=None):
        milliseconds = INFINITE if timeout is None else int(timeout * 1000)
        if self._kernel32.WaitForSingleObject(self._handle, milliseconds) == WAIT_TIMEOUT:
            raise subprocess.TimeoutExpired("agent process", timeout)
        return self.poll()

    def kill(self):
        self._kernel32.TerminateProcess(self._handle, 1)


def _inheritable(path, mode):
    import msvcrt
    descriptor = os.open(path, mode)
    os.set_handle_inheritable(msvcrt.get_osfhandle(descriptor), True)
    return descriptor


def launcher(user, credential_file):
    """A windows_job launcher that starts the command suspended under the agent account."""
    def launch(command, workdir, env, out, err):
        import msvcrt
        password = load_credential(credential_file)
        advapi32 = ctypes.WinDLL("advapi32", use_last_error=True)
        advapi32.CreateProcessWithLogonW.argtypes = [
            wintypes.LPCWSTR, wintypes.LPCWSTR, wintypes.LPCWSTR, wintypes.DWORD, wintypes.LPCWSTR,
            wintypes.LPWSTR, wintypes.DWORD, ctypes.c_void_p, wintypes.LPCWSTR,
            ctypes.POINTER(_StartupInfo), ctypes.POINTER(_ProcessInfo)]
        for stream in (out, err):
            os.set_handle_inheritable(msvcrt.get_osfhandle(stream.fileno()), True)
        stdin = _inheritable(os.devnull, os.O_RDONLY)
        try:
            info = _StartupInfo()
            info.cb = ctypes.sizeof(info)
            info.dwFlags = STARTF_USESTDHANDLES
            info.hStdInput = msvcrt.get_osfhandle(stdin)
            info.hStdOutput = msvcrt.get_osfhandle(out.fileno())
            info.hStdError = msvcrt.get_osfhandle(err.fileno())
            created = _ProcessInfo()
            block = environment_block(agent_environment(env, user))
            line = ctypes.create_unicode_buffer(subprocess.list2cmdline(command))
            ok = advapi32.CreateProcessWithLogonW(
                user, ".", password, LOGON_WITH_PROFILE, None, line,
                CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT | CREATE_NEW_PROCESS_GROUP,
                ctypes.cast(block, ctypes.c_void_p), str(workdir), ctypes.byref(info), ctypes.byref(created))
            password = None
            if not ok:
                raise OSError(ctypes.get_last_error(), "CreateProcessWithLogonW failed")
        finally:
            os.close(stdin)
        return AgentProcess(created.hProcess, created.hThread)
    return launch


def active_launcher():
    user = configured()
    if user is None:
        return None
    if sys.platform != "win32":
        raise ValueError("%s needs Windows" % ENV_USER)
    credential = os.environ.get(ENV_CREDENTIAL, "").strip()
    if not credential or not Path(credential).is_file():
        raise ValueError("%s must name the DPAPI credential file" % ENV_CREDENTIAL)
    return launcher(user, credential)


# ---- self-test: the proof that the boundary holds, before any model is paid for ----

def probe(user, credential, command, workdir, timeout=60):
    """Run one command under the agent account inside a job object, exactly as the harness does."""
    import windows_job
    environment = dict(os.environ)
    return windows_job.run(command, str(workdir), environment, timeout, launcher(user, credential))


def selftest(user, credential, vault, results, binaries, tools):
    """Each check names what must be denied or allowed. Returns (passed, rows). No model is involved."""
    rows = []

    def check(name, expect_ok, result):
        # A launch failure is never evidence of denial: the command must have run and then failed.
        ran = isinstance(result["exit"], int) and result["cleanup_complete"] is True
        ok = ran and (result["exit"] == 0) == expect_ok
        rows.append({"check": name, "expect": "allowed" if expect_ok else "denied", "ok": ok, "exit": result["exit"]})

    with tempfile.TemporaryDirectory(dir=results, prefix="selftest-") as scratch:
        scratch = Path(scratch)
        secret = Path(vault) / "selftest-canary.txt"
        secret.write_text("canary", encoding="utf-8")
        finished = scratch / "finished-run"
        finished.mkdir()
        (finished / "hidden-test.txt").write_text("canary", encoding="utf-8")
        outside = Path(tempfile.gettempdir()) / "selftest-operator-canary.txt"
        outside.write_text("canary", encoding="utf-8")
        live = scratch / "live-run"
        live.mkdir()
        anchor = scratch / "anchor"  # always-readable working directory for the probes that must fail on a path
        anchor.mkdir()
        grant_run_directory(live, user)
        grant_run_directory(anchor, user)
        try:
            for name, target in (("read the vault", secret), ("read a finished run directory", finished / "hidden-test.txt"),
                                 ("read the operator profile", outside)):
                check(name, False, probe(user, credential, ["cmd.exe", "/c", "type", str(target)], anchor))
            check("list the results root", False, probe(user, credential, ["cmd.exe", "/c", "dir", str(results)], anchor))
            check("write the live run directory", True,
                  probe(user, credential, ["cmd.exe", "/c", "echo ok> written.txt"], live))
            for tool in tools:
                check("run tool " + tool, True, probe(user, credential, [tool, "--version"], live))
            for binary in binaries:
                check("run pinned binary " + Path(binary).name, True, probe(user, credential, [binary, "--version"], live))
            revoke_run_directory(live, user)
            check("read a run directory after revocation", False,
                  probe(user, credential, ["cmd.exe", "/c", "dir", str(live)], anchor))
        finally:
            secret.unlink(missing_ok=True)
            outside.unlink(missing_ok=True)
            if has_entry(live, user):
                revoke_run_directory(live, user)
            revoke_run_directory(anchor, user)
    return all(row["ok"] for row in rows), rows


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="mode", required=True)
    sub.add_parser("store-credential").add_argument("--file", required=True)
    lock_parser = sub.add_parser("lock")
    lock_parser.add_argument("directory", nargs="+")
    lock_parser.add_argument("--deny-agent", action="store_true", help="also add an explicit deny for the agent account")
    for name in ("grant-read", "grant-write"):
        sub.add_parser(name).add_argument("path", nargs="+")
    test = sub.add_parser("selftest")
    test.add_argument("--vault", required=True)
    test.add_argument("--results", required=True)
    test.add_argument("--binary", action="append", default=[])
    test.add_argument("--tool", action="append", default=[])
    args = parser.parse_args(argv)
    if args.mode == "store-credential":
        store_credential(args.file, getpass.getpass("Agent account password: "))
        print("stored (DPAPI, readable by this account only)")
        return 0
    user = configured()
    if user is None:
        print("set %s to the agent account name" % ENV_USER, file=sys.stderr)
        return 2
    if args.mode == "lock":
        for directory in args.directory:
            lock(directory, user if args.deny_agent else None)
            print("locked", directory)
        return 0
    if args.mode in ("grant-read", "grant-write"):
        for path in args.path:
            grant(path, user, "RX" if args.mode == "grant-read" else "M")
            print(args.mode, path)
        return 0
    passed, rows = selftest(user, os.environ[ENV_CREDENTIAL], args.vault, args.results, args.binary, args.tool)
    print(json.dumps({"passed": passed, "agent_account": user, "checks": rows}, indent=2))
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
