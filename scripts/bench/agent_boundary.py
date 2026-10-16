"""Run benchmark agents with a restricted token, so the private suite is unreadable to them.

No second account and no administrator rights. The agent process runs as the operator, but with a
restricted copy of the operator's token. Windows then checks every file access twice: once as
usual, and once against the restricting SIDs only (Users, Everyone and the logon session). The suite vault and the
results root grant neither, so the second check refuses the agent even where the operator is
allowed. Graders and checks keep the ordinary token.

    python agent_boundary.py lock DIR [DIR ...]       # operator and SYSTEM only, no inheritance
    python agent_boundary.py grant-read PATH [...]    # toolchain and pinned binaries: Users read/execute
    python agent_boundary.py selftest --vault V --results R [--binary B ...] [--tool T ...]

Selected by BENCH_AGENT_BOUNDARY=restricted-token. Unset, nothing changes (diagnostic-only).
"""
import argparse
import ctypes
from ctypes import wintypes
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile

ENV_BOUNDARY = "BENCH_AGENT_BOUNDARY"
ENV_PATH = "BENCH_AGENT_PATH"
MODE = "restricted-token"
USERS = "*S-1-5-32-545"  # icacls spells a SID with a leading star, so no locale-specific names
EVERYONE_SID = "S-1-1-0"
USERS_SID = "S-1-5-32-545"
# Profile locations that belong to the operator and are unreadable once the token is restricted.
PROFILE_KEYS = frozenset({
    "USERPROFILE", "HOME", "HOMEDRIVE", "HOMEPATH", "APPDATA", "LOCALAPPDATA", "TEMP", "TMP",
    "ONEDRIVE", "ONEDRIVECONSUMER", "CODEX_HOME", "XDG_CONFIG_HOME", "XDG_CACHE_HOME"})
CREATE_SUSPENDED = 0x00000004
CREATE_UNICODE_ENVIRONMENT = 0x00000400
CREATE_NEW_PROCESS_GROUP = 0x00000200
STARTF_USESTDHANDLES = 0x00000100
STILL_ACTIVE = 259
WAIT_TIMEOUT = 0x102
INFINITE = 0xFFFFFFFF
TOKEN_GROUPS = 2
TOKEN_RESTRICTED_SIDS = 11
SE_GROUP_LOGON_ID = 0xC0000000
TOKEN_ACCESS = 0x0002 | 0x0001 | 0x0008 | 0x0080  # DUPLICATE | ASSIGN_PRIMARY | QUERY | ADJUST_DEFAULT


def configured():
    """True when this campaign must run its agents under the restricted token."""
    value = os.environ.get(ENV_BOUNDARY, "").strip()
    if value and value != MODE:
        raise ValueError("%s must be %s or unset" % (ENV_BOUNDARY, MODE))
    return bool(value)


def operator_name():
    return os.environ.get("USERDOMAIN", ".") + "\\" + os.environ.get("USERNAME", "")


def agent_home(agent_dir):
    return Path(agent_dir) / "home"


def prepare_home(agent_dir):
    """Create the profile stand-in the agent uses instead of the operator's profile."""
    home = agent_home(agent_dir)
    for sub in ("AppData/Roaming", "AppData/Local/Temp", ".codex"):
        (home / sub).mkdir(parents=True, exist_ok=True)
    return home


def copy_codex_login(agent_dir):
    """Give the Codex arm its own copy of the login in the stand-in home; the operator's profile stays unreadable."""
    source = Path(os.environ.get("CODEX_HOME") or Path.home() / ".codex") / "auth.json"
    if not source.is_file() or source.is_symlink():
        raise ValueError("Codex login file is unavailable")
    target = agent_home(agent_dir) / ".codex" / "auth.json"
    target.parent.mkdir(parents=True, exist_ok=True)
    shutil.copyfile(source, target)
    return target


def agent_environment(env):
    """Point every profile variable at the campaign's agent directory, never at the operator's profile."""
    agent_dir = env.get("DAVINCI_CODING_AGENT_DIR")
    if not agent_dir:
        raise ValueError("the restricted agent needs the campaign agent directory in its environment")
    home = agent_home(agent_dir)
    result = {k: v for k, v in env.items() if k.upper() not in PROFILE_KEYS}
    result.update({
        "USERPROFILE": str(home), "HOME": str(home), "APPDATA": str(home / "AppData" / "Roaming"),
        "LOCALAPPDATA": str(home / "AppData" / "Local"), "TEMP": str(home / "AppData" / "Local" / "Temp"),
        "TMP": str(home / "AppData" / "Local" / "Temp"), "CODEX_HOME": str(home / ".codex")})
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
    """Operator and SYSTEM only, with inheritance removed: Users and Everyone keep no implied right."""
    return [["icacls", str(path), "/inheritance:r"],
            ["icacls", str(path), "/grant:r", _entry(operator, "F"), _entry("SYSTEM", "F")]]


def grant_command(path, rights, directory=True):
    return ["icacls", str(path), "/grant", _entry(USERS, rights, directory)]


def revoke_command(path):
    return ["icacls", str(path), "/remove:g", USERS]


def _run(args):
    result = subprocess.run(args, capture_output=True, text=True)
    if result.returncode != 0:
        raise OSError("permission change failed (exit %s)" % result.returncode)


def lock(path):
    for command in lock_commands(path, operator_name()):
        _run(command)


def grant(path, rights):
    _run(grant_command(path, rights, Path(path).is_dir()))


def grants_users(sddl):
    """True when a DACL string has an allow entry for Users or Everyone (inherited or not)."""
    dacl = sddl.split("D:", 1)[-1].split("S:", 1)[0]
    for ace in re.findall(r"\(([^)]*)\)", dacl):
        fields = ace.split(";")
        if len(fields) >= 6 and fields[0] in ("A", "OA") and fields[5] in ("BU", "WD", USERS_SID, EVERYONE_SID):
            return True
    return False


def dacl_string(path):
    """SDDL text of the directory's DACL, read without any external tool."""
    advapi32 = ctypes.WinDLL("advapi32", use_last_error=True)
    kernel32 = ctypes.WinDLL("kernel32")
    descriptor = ctypes.c_void_p()
    advapi32.GetNamedSecurityInfoW.argtypes = [wintypes.LPCWSTR, ctypes.c_int, wintypes.DWORD, ctypes.c_void_p,
                                               ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p]
    code = advapi32.GetNamedSecurityInfoW(str(path), 1, 4, None, None, None, None, ctypes.byref(descriptor))
    if code:
        raise OSError(code, "cannot read the access list")
    text = wintypes.LPWSTR()
    advapi32.ConvertSecurityDescriptorToStringSecurityDescriptorW.argtypes = [
        ctypes.c_void_p, wintypes.DWORD, wintypes.DWORD, ctypes.POINTER(wintypes.LPWSTR), ctypes.c_void_p]
    try:
        if not advapi32.ConvertSecurityDescriptorToStringSecurityDescriptorW(descriptor, 1, 4, ctypes.byref(text), None):
            raise OSError(ctypes.get_last_error(), "cannot format the access list")
        return text.value
    finally:
        kernel32.LocalFree(descriptor)


def has_entry(path):
    try:
        return grants_users(dacl_string(path))
    except OSError:
        return True  # an unreadable access list is never evidence of revocation


def grant_run_directory(workdir):
    grant(workdir, "M")


def revoke_run_directory(workdir):
    """Take the run directory back before the hidden tests are copied in; fail loudly if the entry survives."""
    _run(revoke_command(workdir))
    if has_entry(workdir):
        raise OSError("the restricted agent can still reach a graded run directory")


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


class _SidAndAttributes(ctypes.Structure):
    _fields_ = [("Sid", ctypes.c_void_p), ("Attributes", wintypes.DWORD)]


def environment_block(env):
    """UTF-16 block of NAME=value entries sorted case-insensitively, ended by two NULs."""
    entries = sorted(("%s=%s" % (k, v) for k, v in env.items()), key=lambda item: item.split("=", 1)[0].upper())
    body = "\0".join(entries)
    return ctypes.create_unicode_buffer(body + "\0\0", len(body) + 2)


def _logon_sid(advapi32, token):
    """Pointer into a buffer holding this session's logon SID, which guards the desktop and window station."""
    needed = wintypes.DWORD()
    advapi32.GetTokenInformation(token, TOKEN_GROUPS, None, 0, ctypes.byref(needed))
    buffer = ctypes.create_string_buffer(needed.value)
    if not advapi32.GetTokenInformation(token, TOKEN_GROUPS, buffer, needed, ctypes.byref(needed)):
        raise OSError(ctypes.get_last_error(), "GetTokenInformation failed")
    count = ctypes.cast(buffer, ctypes.POINTER(wintypes.DWORD))[0]
    groups = ctypes.cast(ctypes.addressof(buffer) + ctypes.sizeof(ctypes.c_void_p),
                         ctypes.POINTER(_SidAndAttributes))
    for index in range(count):
        if groups[index].Attributes & SE_GROUP_LOGON_ID == SE_GROUP_LOGON_ID:
            length = advapi32.GetLengthSid(ctypes.c_void_p(groups[index].Sid))
            return ctypes.create_string_buffer(ctypes.string_at(groups[index].Sid, length), length)
    raise OSError(0, "the token has no logon SID")


def restricted_token():
    """A primary token for this process whose restricting SIDs are Users, Everyone and the logon session.

    The logon SID is not for files: without it the process cannot open its own desktop and window
    station, and any DLL that initialises user32 fails with 0xC0000142.
    """
    advapi32 = ctypes.WinDLL("advapi32", use_last_error=True)
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel32.GetCurrentProcess.restype = wintypes.HANDLE
    advapi32.OpenProcessToken.argtypes = [wintypes.HANDLE, wintypes.DWORD, ctypes.POINTER(wintypes.HANDLE)]
    advapi32.ConvertStringSidToSidW.argtypes = [wintypes.LPCWSTR, ctypes.POINTER(ctypes.c_void_p)]
    advapi32.GetTokenInformation.argtypes = [wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD,
                                             ctypes.POINTER(wintypes.DWORD)]
    advapi32.GetLengthSid.argtypes = [ctypes.c_void_p]
    advapi32.GetLengthSid.restype = wintypes.DWORD
    advapi32.CreateRestrictedToken.argtypes = [
        wintypes.HANDLE, wintypes.DWORD, wintypes.DWORD, ctypes.c_void_p, wintypes.DWORD, ctypes.c_void_p,
        wintypes.DWORD, ctypes.c_void_p, ctypes.POINTER(wintypes.HANDLE)]
    base = wintypes.HANDLE()
    if not advapi32.OpenProcessToken(kernel32.GetCurrentProcess(), TOKEN_ACCESS, ctypes.byref(base)):
        raise OSError(ctypes.get_last_error(), "OpenProcessToken failed")
    sids = []
    token = wintypes.HANDLE()
    try:
        logon = _logon_sid(advapi32, base)
        restricting = (_SidAndAttributes * 3)()
        for index, text in enumerate((USERS_SID, EVERYONE_SID)):
            pointer = ctypes.c_void_p()
            if not advapi32.ConvertStringSidToSidW(text, ctypes.byref(pointer)):
                raise OSError(ctypes.get_last_error(), "ConvertStringSidToSidW failed")
            sids.append(pointer)
            restricting[index].Sid = pointer
        restricting[2].Sid = ctypes.cast(logon, ctypes.c_void_p)
        if not advapi32.CreateRestrictedToken(base, 0, 0, None, 0, None, 3, restricting, ctypes.byref(token)):
            raise OSError(ctypes.get_last_error(), "CreateRestrictedToken failed")
    finally:
        kernel32.CloseHandle(base)
        for pointer in sids:
            kernel32.LocalFree(pointer)
    return token


def restricting_sid_count(token):
    """How many restricting SIDs a token carries; the tests pin it so no SID is silently dropped."""
    advapi32 = ctypes.WinDLL("advapi32", use_last_error=True)
    advapi32.GetTokenInformation.argtypes = [wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD,
                                             ctypes.POINTER(wintypes.DWORD)]
    needed = wintypes.DWORD()
    advapi32.GetTokenInformation(token, TOKEN_RESTRICTED_SIDS, None, 0, ctypes.byref(needed))
    buffer = ctypes.create_string_buffer(needed.value)
    if not advapi32.GetTokenInformation(token, TOKEN_RESTRICTED_SIDS, buffer, needed, ctypes.byref(needed)):
        raise OSError(ctypes.get_last_error(), "GetTokenInformation failed")
    return ctypes.cast(buffer, ctypes.POINTER(wintypes.DWORD))[0]


class AgentProcess:
    """The slice of subprocess.Popen that windows_job uses: _handle, wait, poll, kill."""

    def __init__(self, process_handle, thread_handle):
        self._handle = int(process_handle)
        self._kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
        self._kernel32.WaitForSingleObject.argtypes = [wintypes.HANDLE, wintypes.DWORD]
        self._kernel32.GetExitCodeProcess.argtypes = [wintypes.HANDLE, ctypes.POINTER(wintypes.DWORD)]
        self._kernel32.TerminateProcess.argtypes = [wintypes.HANDLE, wintypes.UINT]
        self._kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
        self.returncode = None
        self._kernel32.CloseHandle(int(thread_handle))

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


def launch(command, workdir, env, out, err):
    """A windows_job launcher: start the command suspended under the restricted token."""
    import msvcrt
    advapi32 = ctypes.WinDLL("advapi32", use_last_error=True)
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    advapi32.CreateProcessAsUserW.argtypes = [
        wintypes.HANDLE, wintypes.LPCWSTR, wintypes.LPWSTR, ctypes.c_void_p, ctypes.c_void_p, wintypes.BOOL,
        wintypes.DWORD, ctypes.c_void_p, wintypes.LPCWSTR, ctypes.POINTER(_StartupInfo), ctypes.POINTER(_ProcessInfo)]
    for stream in (out, err):
        os.set_handle_inheritable(msvcrt.get_osfhandle(stream.fileno()), True)
    stdin = os.open(os.devnull, os.O_RDONLY)
    os.set_handle_inheritable(msvcrt.get_osfhandle(stdin), True)
    token = restricted_token()
    try:
        info = _StartupInfo()
        info.cb = ctypes.sizeof(info)
        info.dwFlags = STARTF_USESTDHANDLES
        info.hStdInput = msvcrt.get_osfhandle(stdin)
        info.hStdOutput = msvcrt.get_osfhandle(out.fileno())
        info.hStdError = msvcrt.get_osfhandle(err.fileno())
        created = _ProcessInfo()
        agent_env = agent_environment(env)
        # CreateProcess would search the operator's PATH; the agent's own PATH must decide.
        found = shutil.which(command[0], path=agent_env.get("PATH", ""))
        if found is None:
            raise FileNotFoundError(2, "not on the agent PATH: " + str(command[0]))
        block = environment_block(agent_env)
        line = ctypes.create_unicode_buffer(subprocess.list2cmdline([found, *command[1:]]))
        ok = advapi32.CreateProcessAsUserW(
            token, None, line, None, None, True,
            CREATE_SUSPENDED | CREATE_UNICODE_ENVIRONMENT | CREATE_NEW_PROCESS_GROUP,
            ctypes.cast(block, ctypes.c_void_p), str(workdir), ctypes.byref(info), ctypes.byref(created))
        if not ok:
            raise OSError(ctypes.get_last_error(), "CreateProcessAsUserW failed")
    finally:
        kernel32.CloseHandle(token)
        os.close(stdin)
    return AgentProcess(created.hProcess, created.hThread)


def active_launcher():
    if not configured():
        return None
    if sys.platform != "win32":
        raise ValueError("%s needs Windows" % ENV_BOUNDARY)
    return launch


# ---- self-test: the proof that the boundary holds, before any model is paid for ----

def probe(command, workdir, agent_dir, timeout=60):
    """Run one command under the restricted token inside a job object, exactly as the harness does."""
    import windows_job
    env = dict(os.environ, DAVINCI_CODING_AGENT_DIR=str(agent_dir))
    return windows_job.run(command, str(workdir), env, timeout, launch)


def selftest(vault, results, binaries, tools):
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
        live, anchor = scratch / "live-run", scratch / "anchor"  # anchor: always-readable cwd for the denial probes
        agent_dir = scratch / "agent"
        live.mkdir()
        anchor.mkdir()
        prepare_home(agent_dir)
        for directory in (live, anchor, agent_dir):
            grant(directory, "M")
        try:
            positive = probe(["cmd.exe", "/c", "echo ok> written.txt"], live, agent_dir)
            check("write the live run directory", True, positive)
            for name, target in (("read the vault", secret), ("read a finished run directory", finished / "hidden-test.txt"),
                                 ("read the operator profile", outside)):
                check(name, False, probe(["cmd.exe", "/c", "type", str(target)], anchor, agent_dir))
            check("list the results root", False, probe(["cmd.exe", "/c", "dir", str(results)], anchor, agent_dir))
            for tool in tools:
                check("run tool " + tool, True, probe([tool, "--version"], live, agent_dir))
            for binary in binaries:
                check("run pinned binary " + Path(binary).name, True, probe([binary, "--version"], live, agent_dir))
            revoke_run_directory(live)
            check("read a run directory after revocation", False, probe(["cmd.exe", "/c", "dir", str(live)], anchor, agent_dir))
        finally:
            secret.unlink(missing_ok=True)
            outside.unlink(missing_ok=True)
            for directory in (live, anchor, agent_dir):
                if has_entry(directory):
                    revoke_run_directory(directory)
    return all(row["ok"] for row in rows), rows


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="mode", required=True)
    sub.add_parser("lock").add_argument("directory", nargs="+")
    sub.add_parser("grant-read").add_argument("path", nargs="+")
    test = sub.add_parser("selftest")
    test.add_argument("--vault", required=True)
    test.add_argument("--results", required=True)
    test.add_argument("--binary", action="append", default=[])
    test.add_argument("--tool", action="append", default=[])
    args = parser.parse_args(argv)
    if args.mode == "lock":
        for directory in args.directory:
            lock(directory)
            print("locked", directory)
        return 0
    if args.mode == "grant-read":
        for path in args.path:
            grant(path, "RX")
            print("granted read and execute to Users:", path)
        return 0
    passed, rows = selftest(args.vault, args.results, args.binary, args.tool)
    print(json.dumps({"passed": passed, "boundary": MODE, "checks": rows}, indent=2))
    return 0 if passed else 1


if __name__ == "__main__":
    sys.exit(main())
