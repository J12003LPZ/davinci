"""Linux native benchmark lifetime owner, including detached descendants.

This is process cleanup, not a security sandbox. A dedicated child subreaper
owns only this command's tree, so ECHILD after termination proves it has no
remaining descendants without inspecting or killing unrelated host processes.
"""
import argparse
import ctypes
import json
import os
from pathlib import Path
import signal
import subprocess
import sys
import time


def children_path_available():
    try:
        # Some environments expose host /proc with virtualized process IDs.
        # Never signal IDs from a different PID view.
        return (Path("/proc/self").resolve().name == str(os.getpid())
                and Path(f"/proc/{os.getpid()}/task/{os.getpid()}/children").read_text() is not None)
    except OSError:
        return False


def reap_descendants(timeout=10):
    deadline = time.monotonic() + timeout
    children_path = Path(f"/proc/{os.getpid()}/task/{os.getpid()}/children")
    while True:
        # With subreaping enabled before launch, orphaned children are adopted
        # here even if they used setsid or double-forked out of the original group.
        try:
            while os.waitpid(-1, os.WNOHANG)[0]:
                pass
        except ChildProcessError:
            return True
        if time.monotonic() >= deadline:
            return False
        try:
            children = [int(pid) for pid in children_path.read_text().split()]
            for pid in children:
                try:
                    os.kill(pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
        except OSError:
            return False
        time.sleep(0.01)


def supervise(command, timeout):
    if not children_path_available():
        return {"exit": "unsupported_native_lifecycle", "cleanup_complete": False}
    libc = ctypes.CDLL(None, use_errno=True)
    # PR_SET_CHILD_SUBREAPER. Refuse to launch if the kernel cannot own orphans.
    if libc.prctl(36, 1, 0, 0, 0) != 0:
        return {"exit": "unsupported_native_lifecycle", "cleanup_complete": False}
    def interrupted(signum, frame):
        raise InterruptedError("supervisor interrupted")
    signal.signal(signal.SIGTERM, interrupted)
    signal.signal(signal.SIGINT, interrupted)
    process = None
    code = "launch_error"
    try:
        process = subprocess.Popen(command, stdin=subprocess.DEVNULL, start_new_session=True)
        try:
            code = process.wait(timeout=timeout)
        except subprocess.TimeoutExpired:
            code = "timeout"
    except InterruptedError:
        code = "interrupted"
    except OSError as error:
        print(type(error).__name__, file=sys.stderr)
    finally:
        # Cleanup must not itself be interrupted by repeated Ctrl+C/terminate.
        signal.signal(signal.SIGTERM, signal.SIG_IGN)
        signal.signal(signal.SIGINT, signal.SIG_IGN)
        if process is not None and process.poll() is None:
            process.kill()
            try:
                process.wait(timeout=2)
            except subprocess.TimeoutExpired:
                pass
        clean = reap_descendants()
    return {"exit": code, "cleanup_complete": clean}


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--report", required=True)
    parser.add_argument("--timeout", required=True, type=float)
    parser.add_argument("command", nargs=argparse.REMAINDER)
    args = parser.parse_args()
    command = args.command[1:] if args.command[:1] == ["--"] else args.command
    if sys.platform != "linux" or not command or args.timeout <= 0:
        raise SystemExit("native lifetime supervision requires Linux and a command")
    result = supervise(command, args.timeout)
    # The report is outside the public worktree. Its absence, truncation, or an
    # abnormal supervisor exit is never interpreted as successful cleanup.
    Path(args.report).write_text(json.dumps({"schema_version": 1, **result}), encoding="utf-8")


if __name__ == "__main__":
    main()
