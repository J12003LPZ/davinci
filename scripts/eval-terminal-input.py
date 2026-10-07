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


OFFLINE_CODEX_LOGIN = {
    "type": "oauth",
    "access": "offline-eval-access",
    "expires": 4102444800000,
    "env": {
        "OPENAI_SIWC_CLIENT_ID": "oaiapp_offline_eval",
        "OPENAI_SIWC_EXT_AGENT_HOST_ID": "urn:uuid:00000000-0000-4000-8000-000000000001",
        "OPENAI_SIWC_ISSUER": "https://auth.openai.com",
        "OPENAI_SIWC_SUBJECT": "offline-eval-subject",
        "OPENAI_SIWC_EMAIL": "offline-eval@example.test",
        "OPENAI_SIWC_ID_TOKEN": "offline-eval-id-token",
        "OPENAI_SIWC_SCOPES": "openid profile email offline_access resource.invoke chatgpt.tokens.use.direct",
        "OPENAI_SIWC_RESOURCE": "https://api.openai.com/v1",
    },
}


class Terminal:
    def __init__(self, executable, cwd, fixture, width, height, *, offline=True, extra_args=(), skills=False):
        args = [executable, "--davinci", "--no-animation",
                "--no-session", "--no-extensions", "--no-mcp", "--no-context-files",
                "--provider", "openai-codex", "--model", "gpt-6-luna", "--thinking", "low"]
        # Skills stay off unless a check is about them (the slash menu).
        if not skills:
            args.insert(6, "--no-skills")
        if offline:
            args.append("--offline")
        args.extend(extra_args)
        if fixture:
            args.extend(["--screen", fixture])
        env = dict(os.environ)
        config = Path(cwd) / "eval-config"
        config.mkdir(exist_ok=True)
        env.update(PI_CODING_AGENT_DIR=str(config), DAVINCI_CODING_AGENT_DIR=str(config))
        if offline and not fixture:
            env["OPENAI_API_KEY"] = "offline-eval-placeholder"
            # openai-codex accepts only Sign in with ChatGPT credentials, so an
            # API key no longer makes its models selectable. Offline runs never
            # send this synthetic login; it only satisfies the catalog check.
            auth = config / "auth.json"
            if not auth.exists():
                auth.write_text(json.dumps({"openai-codex": OFFLINE_CODEX_LOGIN}), encoding="utf-8")
        self.process = PtyProcess.spawn(
            args, cwd=cwd, dimensions=(height, width), env=env,
        )
        self.screen = pyte.Screen(width, height)
        self.stream = pyte.Stream(self.screen)
        self.chunks = queue.Queue()
        self.reader = threading.Thread(target=self.read, daemon=True)
        self.reader.start()

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

    def command(self, text):
        # ConPTY can split a write into bursts. Wait beyond the Windows paste
        # detector's quiet window before sending the submit key.
        self.send(text, 1.2)
        return self.send("\r", 1.2)

    def close(self):
        if self.process is None:
            return
        if not self.process.terminate(force=True):
            raise RuntimeError("Could not terminate the eval terminal")
        self.process.close(force=True)
        self.reader.join(timeout=5)
        if self.reader.is_alive():
            raise RuntimeError("Eval terminal reader did not stop")
        # Release the ConPTY handle before TemporaryDirectory removes its cwd.
        self.process = None


def main():
    executable = str(Path(sys.argv[1]).resolve())
    results = []
    with tempfile.TemporaryDirectory(prefix="davinci-terminal-eval-") as cwd:
        for width in (40, 80, 109, 120, 240):
            terminal = Terminal(executable, cwd, "blueprint", width, 30)
            try:
                text = terminal.pump(8)
                assert "Agent command center" in text, text
                if width >= 100:
                    assert "Working 2" in text and "Attention 2" in text, text
                text = terminal.send("g")
                assert "Original prompt" in text, text
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
            text = terminal.command("/graph --dry-run test role configuration")
            assert "Graph setup" in text, text
            assert not list(Path(cwd).glob(".davinci/graph/runs/*")), "Started before confirmation"
            terminal.send("\x1b[B")  # researcher
            text = terminal.send("\r")
            assert "Graph model: researcher" in text, text
            text = terminal.send("openai-codex/gpt-6-luna", 0.8)
            assert "openai-codex/gpt-6-luna" in text, text
            text = terminal.send("\r")
            assert "Graph setup" in text, text
            assert "researcher: openai-codex/gpt-6-luna" in text, text
            terminal.send("\x1b")
            assert not list(Path(cwd).glob(".davinci/graph/runs/*")), "Cancel launched workers"
            text = terminal.command("/graph --dry-run test role configuration")
            assert "Graph setup" in text, text
            for _ in range(7):
                terminal.send("\x1b[B", 0.1)
            text = terminal.send("\r", 3)
            # A fast dry run may already return to the conversation.
            assert "Agent command center" in text or "Graph COMPLETED" in text, text
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
            terminal.command("/graph-status")
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
            text = terminal.command("/graph-status")
            assert "BLOCKED - goal not completed" in text, text
            # Revision counts belong to an agent's details in the new canvas.
            text = terminal.send("\x1b[B")
            text = terminal.send("\r")
            assert "Revisions:" in text and "4 of 3" in text, text
            assert "s resume" in text, text
            terminal.send("\x1b")
            text = terminal.send("\x1b")
            assert "Graph BLOCKED" in text, text
            assert "verification still failing" in text, text
            terminal.command("/graph-status")
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
