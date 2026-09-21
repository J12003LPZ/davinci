"""Offline Windows ConPTY regression eval; requires installed winpty and pyte.

Pass the davinci executable as the first argument. No provider calls are made.
"""
import json
import os
import queue
import sys
import tempfile
import threading
import time
from pathlib import Path

import pyte
from winpty import PtyProcess


class Terminal:
    def __init__(self, executable, cwd, fixture, width, height):
        args = [executable, "--davinci", "--offline", "--no-animation",
                "--no-session", "--no-extensions", "--no-skills", "--no-context-files"]
        if fixture:
            args.extend(["--screen", fixture])
        env = dict(os.environ)
        config = Path(cwd) / "eval-config"
        config.mkdir(exist_ok=True)
        env.update(PI_CODING_AGENT_DIR=str(config), DAVINCI_CODING_AGENT_DIR=str(config))
        if not fixture:
            env["OPENAI_API_KEY"] = "offline-eval-placeholder"
            args.extend(["--provider", "openai", "--model", "gpt-5"])
        self.process = PtyProcess.spawn(
            args, cwd=cwd, dimensions=(height, width), env=env,
        )
        self.screen = pyte.Screen(width, height)
        self.stream = pyte.Stream(self.screen)
        self.chunks = queue.Queue()
        threading.Thread(target=self.read, daemon=True).start()

    def read(self):
        try:
            while self.process.isalive():
                self.chunks.put(self.process.read(65536))
        except EOFError:
            pass

    def pump(self, seconds=0.4):
        deadline = time.monotonic() + seconds
        while time.monotonic() < deadline:
            try:
                self.stream.feed(self.chunks.get(timeout=0.05))
            except queue.Empty:
                pass
        return "\n".join(self.screen.display)

    def send(self, text, seconds=0.4):
        self.process.write(text)
        return self.pump(seconds)

    def close(self):
        self.process.terminate(force=True)


def main():
    executable = str(Path(sys.argv[1]).resolve())
    results = []
    with tempfile.TemporaryDirectory(prefix="davinci-terminal-eval-") as cwd:
        for width in (40, 80, 109, 120, 240):
            terminal = Terminal(executable, cwd, "blueprint", width, 30)
            try:
                text = terminal.pump(8)
                assert "GRAPH RUN" in text, text
                if width >= 100:
                    heading = next(line for line in terminal.screen.display if "WORKER ACTIVITY" in line)
                    assert heading.index("WORKER ACTIVITY") > width // 2
                text = terminal.send("g")
                assert "MAIN GOAL" in text, text
                assert "Validate parallel execution" in " ".join(text.split()), text
                terminal.send("\x1b")
                results.append({"graph_width": width, "goal": "passed"})
            finally:
                terminal.close()
        terminal = Terminal(executable, cwd, "1a", 120, 30)
        try:
            terminal.pump(8)
            payload = "PASTE_SENTINEL " + "x" * 1100 + "\nsecond line\n"
            terminal.send("\x1b[200~" + payload[:600], 0.3)
            text = terminal.send(payload[600:] + "\x1b[20", 0.3)
            assert "PASTE_SENTINEL" not in text, "Paste submitted before closing marker"
            text = terminal.send("1~", 0.7)
            marker = f"[paste #1 {len(payload)} chars]"
            deadline = time.monotonic() + 5
            while marker not in text and time.monotonic() < deadline:
                text = terminal.pump()
            assert marker in text, text
            assert "PASTE_SENTINEL" not in text, "Paste submitted automatically"
            text = terminal.send("\r", 0.7)
            assert marker not in text, "Explicit Enter did not submit\n" + text
            results.append({"delayed_multiline_paste": "passed", "explicit_enter": "passed"})
        finally:
            terminal.close()
        terminal = Terminal(executable, cwd, None, 120, 36)
        try:
            terminal.pump(8)
            terminal.send("/graph --dry-run test role configuration", 0.8)
            text = terminal.send("\r", 0.8)
            assert "GRAPH SETUP" in text, text
            assert not list(Path(cwd).glob(".davinci/graph/runs/*")), "Started before confirmation"
            terminal.send("\x1b[B")  # researcher
            text = terminal.send("\r")
            assert "GRAPH MODEL: RESEARCHER" in text, text
            text = terminal.send("openai/gpt-5", 0.8)
            assert "openai/gpt-5" in text, text
            text = terminal.send("\r")
            assert "GRAPH SETUP" in text, text
            assert "researcher: openai/gpt-5" in text, text
            terminal.send("\x1b")
            assert not list(Path(cwd).glob(".davinci/graph/runs/*")), "Cancel launched workers"
            terminal.send("/graph --dry-run test role configuration", 0.8)
            text = terminal.send("\r", 0.8)
            assert "GRAPH SETUP" in text, text
            for _ in range(7):
                terminal.send("\x1b[B", 0.1)
            text = terminal.send("\r", 3)
            assert "GRAPH RUN" in text, text
            assert list(Path(cwd).glob(".davinci/graph/runs/*")), "Start did not launch graph"
            deadline = time.monotonic() + 10
            while "COMPLETED" not in text and time.monotonic() < deadline:
                text = terminal.pump()
            assert "COMPLETED" in text, text
            text = terminal.send("\x1b")
            assert "Graph COMPLETED" in text, text
            results.append({"graph_role_setup": "passed", "cancel_before_start": "passed"})
        finally:
            terminal.close()
        # Reproduce an older saved blocked run whose lifecycle incorrectly says running.
        state_path = next(Path(cwd).glob(".davinci/graph/runs/*/state.json"))
        state = json.loads(state_path.read_text(encoding="utf-8"))
        verifying = dict(state, phase="verify", lifecycle="running", verification={
            "passed": False, "commands": [], "progress": {
                "name": "test", "command": "cargo test --workspace", "index": 3,
                "total": 8, "startedAt": int(time.time() * 1000),
            },
        })
        state_path.write_text(json.dumps(verifying), encoding="utf-8")
        terminal = Terminal(executable, cwd, None, 160, 36)
        try:
            terminal.pump(8)
            terminal.send("/graph-status", 0.8)
            terminal.send("\r", 1)
            text = terminal.send("g")
            assert "Verification running" in text, text
            assert "3/8" in text, text
            assert "cargo test --workspace" in text, text
            assert "Verification failed" not in text, text
            results.append({"verification_command_progress": "passed"})
        finally:
            terminal.close()
        state.update(phase="blocked", lifecycle="running",
                     blockedReason="verification still failing after 3 revision cycles")
        state["counters"]["revisionCycles"] = 4
        state["budgets"]["maxRevisionCycles"] = 3
        state_path.write_text(json.dumps(state), encoding="utf-8")
        terminal = Terminal(executable, cwd, None, 160, 36)
        try:
            terminal.pump(8)
            terminal.send("/graph-status", 0.8)
            text = terminal.send("\r", 1)
            assert "BLOCKED - goal not completed" in text, text
            assert "4 total (limit 3/milestone)" in text, text
            assert "s resume graph" in text, text
            text = terminal.send("\x1b")
            assert "Graph BLOCKED" in text, text
            assert "verification still failing" in text, text
            terminal.send("/graph-status", 0.8)
            terminal.send("\r", 0.8)
            text = terminal.send("s", 3)
            deadline = time.monotonic() + 10
            while "COMPLETED" not in text and time.monotonic() < deadline:
                text = terminal.pump()
            assert "COMPLETED" in text, text
            resumed = json.loads(state_path.read_text(encoding="utf-8"))
            assert resumed["runId"] == state["runId"]
            assert resumed["phase"] == "done"
            assert resumed["lifecycle"] == "stopped"
            assert resumed["counters"]["revisionCycles"] == 4
            results.append({"blocked_result_message": "passed", "resume_graph_key": "passed"})
        finally:
            terminal.close()
    print(json.dumps(results))


if __name__ == "__main__":
    main()
