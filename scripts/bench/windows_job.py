"""Windows native benchmark lifetime owner, using a kill-on-close Job Object.

The command starts suspended, is placed in a fresh job, and only then resumes,
so every descendant it creates belongs to the job. After the command exits or
times out the whole job is terminated, and cleanup counts as complete only when
the kernel reports zero active processes in it. If the orchestrator dies, the
job handle closes and the kernel kills the tree (JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE).

This is process cleanup, not a security sandbox.
"""
import ctypes
from ctypes import wintypes
import subprocess
import tempfile
import time
from pathlib import Path

CREATE_SUSPENDED = 0x00000004
CREATE_NEW_PROCESS_GROUP = 0x00000200
JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE = 0x00002000
JOB_OBJECT_BASIC_ACCOUNTING_INFORMATION = 1
JOB_OBJECT_EXTENDED_LIMIT_INFORMATION = 9


class _BasicLimit(ctypes.Structure):
    _fields_ = [("PerProcessUserTimeLimit", ctypes.c_int64),
                ("PerJobUserTimeLimit", ctypes.c_int64),
                ("LimitFlags", wintypes.DWORD),
                ("MinimumWorkingSetSize", ctypes.c_size_t),
                ("MaximumWorkingSetSize", ctypes.c_size_t),
                ("ActiveProcessLimit", wintypes.DWORD),
                ("Affinity", ctypes.c_size_t),
                ("PriorityClass", wintypes.DWORD),
                ("SchedulingClass", wintypes.DWORD)]


class _IoCounters(ctypes.Structure):
    _fields_ = [(name, ctypes.c_uint64) for name in (
        "ReadOperationCount", "WriteOperationCount", "OtherOperationCount",
        "ReadTransferCount", "WriteTransferCount", "OtherTransferCount")]


class _ExtendedLimit(ctypes.Structure):
    _fields_ = [("BasicLimitInformation", _BasicLimit),
                ("IoInfo", _IoCounters),
                ("ProcessMemoryLimit", ctypes.c_size_t),
                ("JobMemoryLimit", ctypes.c_size_t),
                ("PeakProcessMemoryUsed", ctypes.c_size_t),
                ("PeakJobMemoryUsed", ctypes.c_size_t)]


class _Accounting(ctypes.Structure):
    _fields_ = [("TotalUserTime", ctypes.c_int64),
                ("TotalKernelTime", ctypes.c_int64),
                ("ThisPeriodTotalUserTime", ctypes.c_int64),
                ("ThisPeriodTotalKernelTime", ctypes.c_int64),
                ("TotalPageFaultCount", wintypes.DWORD),
                ("TotalProcesses", wintypes.DWORD),
                ("ActiveProcesses", wintypes.DWORD),
                ("TotalTerminatedProcesses", wintypes.DWORD)]


def _api():
    kernel32 = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel32.CreateJobObjectW.restype = wintypes.HANDLE
    kernel32.CreateJobObjectW.argtypes = [ctypes.c_void_p, wintypes.LPCWSTR]
    kernel32.SetInformationJobObject.argtypes = [
        wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD]
    kernel32.QueryInformationJobObject.argtypes = [
        wintypes.HANDLE, ctypes.c_int, ctypes.c_void_p, wintypes.DWORD, ctypes.c_void_p]
    kernel32.AssignProcessToJobObject.argtypes = [wintypes.HANDLE, wintypes.HANDLE]
    kernel32.TerminateJobObject.argtypes = [wintypes.HANDLE, wintypes.UINT]
    kernel32.CloseHandle.argtypes = [wintypes.HANDLE]
    ntdll = ctypes.WinDLL("ntdll")
    ntdll.NtResumeProcess.argtypes = [wintypes.HANDLE]
    ntdll.NtResumeProcess.restype = ctypes.c_long
    return kernel32, ntdll


def _create_job(kernel32):
    job = kernel32.CreateJobObjectW(None, None)
    if not job:
        raise OSError(ctypes.get_last_error(), "CreateJobObjectW failed")
    info = _ExtendedLimit()
    info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
    if not kernel32.SetInformationJobObject(
            job, JOB_OBJECT_EXTENDED_LIMIT_INFORMATION, ctypes.byref(info), ctypes.sizeof(info)):
        error = ctypes.get_last_error()
        kernel32.CloseHandle(job)
        raise OSError(error, "SetInformationJobObject failed")
    return job


def _active_processes(kernel32, job):
    info = _Accounting()
    if not kernel32.QueryInformationJobObject(
            job, JOB_OBJECT_BASIC_ACCOUNTING_INFORMATION, ctypes.byref(info),
            ctypes.sizeof(info), None):
        return None
    return info.ActiveProcesses


def _terminate_and_confirm(kernel32, job, timeout=10):
    kernel32.TerminateJobObject(job, 1)
    deadline = time.monotonic() + timeout
    while True:
        active = _active_processes(kernel32, job)
        if active == 0:
            return True
        if active is None or time.monotonic() >= deadline:
            return False
        time.sleep(0.02)


def run(command, workdir, env, timeout):
    """Run ``command`` inside a job; return the runner's measured-result shape."""
    kernel32, ntdll = _api()
    job = _create_job(kernel32)
    process = None
    code = "launch_error"
    stdout = stderr = ""
    clean = False
    # Files, not pipes: a descendant that inherits a pipe would otherwise keep
    # the parent's read blocked after the command itself has exited.
    with tempfile.TemporaryDirectory(prefix="davinci-bench-job-") as temporary:
        out_path, err_path = Path(temporary) / "stdout", Path(temporary) / "stderr"
        try:
            with open(out_path, "wb") as out, open(err_path, "wb") as err:
                try:
                    process = subprocess.Popen(
                        command, cwd=workdir, env=env, stdin=subprocess.DEVNULL,
                        stdout=out, stderr=err,
                        creationflags=CREATE_SUSPENDED | CREATE_NEW_PROCESS_GROUP)
                except OSError as error:
                    stderr = type(error).__name__
                    clean = _terminate_and_confirm(kernel32, job)
                    return {"exit": "launch_error", "stdout": "", "stderr": stderr,
                            "cleanup_complete": clean}
                handle = int(process._handle)
                if not kernel32.AssignProcessToJobObject(job, handle):
                    # Never resume a process whose descendants would escape.
                    process.kill()
                    process.wait(timeout=10)
                    return {"exit": "unsupported_native_lifecycle", "stdout": "",
                            "stderr": "AssignProcessToJobObject failed",
                            "cleanup_complete": True}
                if ntdll.NtResumeProcess(handle) != 0:
                    clean = _terminate_and_confirm(kernel32, job)
                    return {"exit": "launch_error", "stdout": "",
                            "stderr": "NtResumeProcess failed", "cleanup_complete": clean}
                try:
                    code = process.wait(timeout=timeout)
                except subprocess.TimeoutExpired:
                    code = "timeout"
                except KeyboardInterrupt:
                    code = "interrupted"
                    raise
                finally:
                    # Descendants may outlive a normal exit; the run owns them all.
                    clean = _terminate_and_confirm(kernel32, job)
                    if process.poll() is None:
                        try:
                            process.wait(timeout=5)
                        except subprocess.TimeoutExpired:
                            clean = False
            stdout = out_path.read_text(encoding="utf-8", errors="replace")
            stderr = err_path.read_text(encoding="utf-8", errors="replace")
        finally:
            kernel32.CloseHandle(job)
    return {"exit": code, "stdout": stdout, "stderr": stderr, "cleanup_complete": clean}
