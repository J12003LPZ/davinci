"""Live GPT-6 Luna/low compaction and recall eval in a disposable session.

Usage: python scripts/eval-live-session.py <davinci-executable> <artifact-dir>
Uses existing OpenAI Codex credentials without changing the original auth store.
"""
import live_auth
import importlib.util
import json
from pathlib import Path
import sys
import tempfile
import time


def main():
    spec = importlib.util.spec_from_file_location("terminal_eval", Path(__file__).with_name("eval-terminal-input.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    executable = str(Path(sys.argv[1]).resolve())
    output = Path(sys.argv[2]).resolve()
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="davinci-live-session-") as temporary:
        root = Path(temporary)
        config = root / "eval-config"
        config.mkdir()
        (config / "vector-memory.json").write_text(json.dumps({"enabled": False}), encoding="utf-8")
        auth = config / "auth.json"
        lent = live_auth.lend(auth)
        terminal = None

        def entries():
            rows = []
            for path in config.rglob("*.jsonl"):
                if "." in path.stem:  # Runtime journals hold exclusive Windows locks.
                    continue
                try:
                    lines = path.read_text(encoding="utf-8").splitlines()
                except (FileNotFoundError, PermissionError):
                    continue  # Retry while the session writer is appending.
                for line in lines:
                    try:
                        rows.append(json.loads(line))
                    except json.JSONDecodeError:
                        pass  # A concurrent append may not yet be complete.
            return rows

        def replies():
            return [r["message"] for r in entries()
                    if r.get("type") == "message" and r.get("message", {}).get("role") == "assistant"]

        def wait_for(predicate, label):
            deadline = time.monotonic() + 180
            while time.monotonic() < deadline:
                screen = terminal.pump(0.5)
                (output / "live-session-screen.txt").write_text(screen, encoding="utf-8")
                assert "Provider request failed:" not in screen, screen
                assert "Summarization failed:" not in screen, screen
                if predicate():
                    return screen
            raise AssertionError(label)

        try:
            terminal = module.Terminal(executable, temporary, None, 160, 45, offline=False,
                                      extra_args=("--permission-mode", "always-approve", "--no-prompt-templates"))
            terminal.pump(8)
            terminal.command("/new")
            terminal.command("Remember this audit code: amber-9364. Reply with a short acknowledgment. "
                             "Do not use tools, agents, or graph.")
            wait_for(lambda: bool(replies()), "initial response was not persisted")
            terminal.pump(1)
            terminal.command("/compact Preserve the audit code exactly.")
            wait_for(lambda: any(r.get("type") == "compaction" for r in entries()), "compaction was not persisted")
            compacted = [r for r in entries() if r.get("type") == "compaction"]
            assert "amber-9364" in json.dumps(compacted), compacted
            terminal.send("\x1b")
            before = len(replies())
            terminal.command("What was the audit code? Reply with only that code. Do not use tools or agents.")
            wait_for(lambda: len(replies()) > before, "post-compaction response was not persisted")
            assert "amber-9364" in json.dumps(replies()[-1]), replies()[-1]
            summary = {"model": "gpt-6-luna", "effort": "low", "compaction_persisted": True,
                       "summary_preserved_code": True, "post_compaction_recall": True}
            (output / "live-session-summary.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")
            print(json.dumps(summary))
        finally:
            try:
                if terminal:
                    terminal.close()
            finally:
                live_auth.give_back(auth, lent)
                auth.unlink(missing_ok=True)


if __name__ == "__main__":
    main()
