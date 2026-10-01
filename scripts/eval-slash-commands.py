"""Offline ConPTY command smoke eval with disposable configuration and sessions.

Requires the same winpty/pyte dependencies as eval-terminal-input.py. Sharing
uses the built-in dry-run fixture; it never publishes a gist or changes auth.
"""
import importlib.util
import json
import os
from pathlib import Path
import sys
import tempfile


def main():
    spec = importlib.util.spec_from_file_location("terminal_eval", Path(__file__).with_name("eval-terminal-input.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    executable = str(Path(sys.argv[1]).resolve())
    artifact = Path(sys.argv[2]).resolve() if len(sys.argv) > 2 else None
    checks = [
        ("/help", "/setup", False),
        ("/settings", "Search settings", 2),
        ("/config", "Search settings", 2),
        ("/model", "Switch between configured models", True),
        ("/model openai-codex/gpt-6-luna", "model openai-codex / gpt-6-luna", False),
        ("/thinking low", "thinking level low", False),
        ("/effort low", "thinking level low", False),
        ("/tree", "no session tree yet", False),
        ("/rewind", "No prompt checkpoints", False),
        ("/export absent.jsonl", "no session to export", False),
        ("/import absent.jsonl", "Session file not found", False),
        ("/name Audit", "no session to name", False),
        ("/fork", "no session to fork", False),
        ("/clone", "no session to clone", False),
        ("/copy", "no agent messages to copy", False),
        ("/share", "Share URL:", False),
        ("/hotkeys", "Help", True),
        ("/login", "Providers", True),
        ("/logout nonexistent", "removed nonexistent", False),
        ("/context", "Context Usage", True),
        ("/context inspect", "Context & Memory Inspector", True),
        ("/reload", "Native extensions", True),
        ("/mcp", "No MCP servers configured", False),
        ("/cost", "input 0", False),
        ("/status", "restores", False),
        ("/doctor", "securityConfiguration", False),
        ("/permissions plan", "Plan Mode", False),
        ("/plan show", "No structured plan yet", False),
        ("/act", "planning ended", False),
        ("/agents", "No workers active", True),
        ("/plugin list", "No plugins installed", False),
        ("/tasks", "No tasks currently running", True),
        ("/setup check", "vector memory", False),
        ("/workflow list", "workflows are not available in this session", False),
        ("/security-scan --report", "no security report is available yet", False),
    ]
    rows = []
    # These settings affect only the child started by this standalone eval.
    os.environ.update(PI_SHARE_DRY_RUN="1", DAVINCI_EXPERIMENTAL_WORKFLOWS="1")
    with tempfile.TemporaryDirectory(prefix="davinci-slash-eval-") as cwd:
        config = Path(cwd) / "eval-config"
        config.mkdir()
        (config / "vector-memory.json").write_text(json.dumps({"enabled": False}), encoding="utf-8")
        terminal = module.Terminal(executable, cwd, None, 180, 55)

        def check(command, expected, overlay=False, *, reset=False):
            text = terminal.command(command)
            # Some diagnostics traverse the environment before returning.
            for _ in range(8):
                if expected.lower() in text.lower():
                    break
                text = terminal.pump(0.5)
            passed = expected.lower() in text.lower()
            rows.append({"command": command, "passed": passed, "screen": text})
            if artifact:
                artifact.write_text(json.dumps(rows, indent=2), encoding="utf-8")
            assert passed, (command, expected, text)
            assert terminal.process.isalive(), command
            for _ in range(2 if reset else int(overlay)):
                terminal.send("\x1b")
            if reset:
                terminal.send("\x15")

        try:
            terminal.pump(8)
            for item in checks:
                check(*item, reset=True)
            check("/new", "started a new session")
            check("Session audit sentinel", "offline")
            check("/name Audit", "named this session Audit")
            check("/tree", "Session", True)
            check("/export audit.jsonl", "audit.jsonl", True)
            exported = Path(cwd) / "audit.jsonl"
            assert exported.is_file() and "Session audit sentinel" in exported.read_text(encoding="utf-8")
            check("/clone", "cloned to")
            check("/fork", "forked to")
            check("/import audit.jsonl", "imported")
            check("/resume", "Resume session", True)
            check("/compact", "Summarization failed: offline")
            terminal.send("\x1b")
            terminal.command("/quit")
            terminal.pump(1)
            assert not terminal.process.isalive(), "/quit did not exit"
            rows.append({"command": "/quit", "passed": True})
        finally:
            terminal.close()
            if artifact:
                artifact.write_text(json.dumps(rows, indent=2), encoding="utf-8")
    print(json.dumps({"passed": len(rows), "provider_calls": 0, "sharing": "dry-run"}))


if __name__ == "__main__":
    main()
