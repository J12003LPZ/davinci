"""Offline ConPTY command smoke eval with disposable configuration and sessions.

Requires the same winpty/pyte dependencies as eval-terminal-input.py. Sharing
uses the built-in dry-run fixture; it never publishes a gist or changes auth.
"""
import importlib.util
import json
import os
from pathlib import Path
import sys
import struct
import tempfile
import zlib


def tiny_png():
    """A valid 2x1 RGB PNG, built here so no binary lives in the repo."""
    def chunk(kind, data):
        body = kind + data
        return struct.pack(">I", len(data)) + body + struct.pack(">I", zlib.crc32(body))
    raw = b"\x00" + b"\xff\x00\x00" + b"\x00\x00\xff"
    return (
        b"\x89PNG\r\n\x1a\n"
        + chunk(b"IHDR", struct.pack(">IIBBBBB", 2, 1, 8, 2, 0, 0, 0))
        + chunk(b"IDAT", zlib.compress(raw))
        + chunk(b"IEND", b"")
    )


def main():
    spec = importlib.util.spec_from_file_location("terminal_eval", Path(__file__).with_name("eval-terminal-input.py"))
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    executable = str(Path(sys.argv[1]).resolve())
    artifact = Path(sys.argv[2]).resolve() if len(sys.argv) > 2 else None
    checks = [
        ("/help", "/setup", False),
        ("/settings", "is not a command", False),
        ("/config", "Search settings", 2),
        ("/model", "Switch between configured models", True),
        ("/model openai-codex/gpt-6-luna", "model openai-codex / gpt-6-luna", False),
        ("/thinking low", "is not a command", False),
        ("/fast", "is not a command", False),
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
        ("/mcp", "No MCP servers configured", True),
        ("/skills", "eval-own-skill", True),
        ("/skill-list", "is not a command", False),
        ("/skill-view pdf", "is not a command", False),
        ("/cost", "input 0", False),
        ("/status", "restores", False),
        ("/doctor", "securityConfiguration", False),
        ("/permissions plan", "Plan Mode", False),
        ("/plan show", "No structured plan yet", False),
        ("/act", "is not a command", False),
        ("/permissions manual", "Manual", False),
        ("/agents", "No workers active", True),
        ("/plugin list", "is not a command", False),
        ("/plugins list", "No plugins installed", False),
        ("/tasks", "No tasks currently running", True),
        ("/setup check", "vector memory", False),
        ("/workflow list", "workflows are not available in this session", False),
        ("/security-scan --report", "no security report is available yet", False),
    ]
    rows = []
    # These settings affect only the child started by this standalone eval.
    # Discover's online directories point at a closed port: the eval stays
    # offline and checks that an unreachable directory is reported.
    os.environ.update(
        PI_SHARE_DRY_RUN="1",
        DAVINCI_EXPERIMENTAL_WORKFLOWS="1",
        DAVINCI_MCP_REGISTRY_URL="http://127.0.0.1:9/v0/servers",
        DAVINCI_SKILLS_SEARCH_URL="http://127.0.0.1:9/api/search",
        DAVINCI_BUILTIN_MARKETPLACES="off",
    )
    with tempfile.TemporaryDirectory(prefix="davinci-slash-eval-") as cwd:
        config = Path(cwd) / "eval-config"
        config.mkdir()
        (config / "vector-memory.json").write_text(json.dumps({"enabled": False}), encoding="utf-8")
        # A skill of the user's own, and a local marketplace with a plugin
        # that ships a skill and a command, for the slash menu checks.
        own = config / "skills" / "eval-own-skill"
        own.mkdir(parents=True)
        (own / "SKILL.md").write_text(
            "---\nname: eval-own-skill\ndescription: The user's own eval skill\n---\nOwn body.\n",
            encoding="utf-8",
        )
        market = Path(cwd) / "eval-market"
        plugin = market / "plugins" / "evalkit"
        (market / ".claude-plugin").mkdir(parents=True)
        (market / ".claude-plugin" / "marketplace.json").write_text(json.dumps({
            "name": "eval-market",
            "plugins": [{"name": "evalkit", "source": "./plugins/evalkit", "description": "eval"}],
        }), encoding="utf-8")
        (plugin / ".claude-plugin").mkdir(parents=True)
        (plugin / ".claude-plugin" / "plugin.json").write_text(json.dumps({"name": "evalkit"}), encoding="utf-8")
        (plugin / "skills" / "plan-it").mkdir(parents=True)
        (plugin / "skills" / "plan-it" / "SKILL.md").write_text(
            "---\nname: plan-it\ndescription: Plan before touching code\n---\nPlan body.\n",
            encoding="utf-8",
        )
        (plugin / "commands").mkdir()
        (plugin / "commands" / "ship.md").write_text("Ship it: $ARGUMENTS\n", encoding="utf-8")
        # alt+v reads this instead of the system clipboard (2x1 PNG).
        clip = Path(cwd) / "clip.png"
        clip.write_bytes(tiny_png())
        os.environ["PI_CLIPBOARD_IMAGE"] = str(clip)
        terminal = module.Terminal(executable, cwd, None, 180, 55, skills=True)

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
            text = terminal.pump(8)
            # The context bar is on by default, with the auto-compact point.
            assert "◆ context" in text and "compacts at" in text, text
            rows.append({"command": "context bar", "passed": True})
            for item in checks:
                check(*item, reset=True)
            # Discover: → opens it, typing searches, esc clears then closes.
            text = terminal.command("/mcp")
            assert "No MCP servers configured" in text, text
            text = terminal.send("\x1b[C", 1.0)
            assert "Search the MCP Registry" in text, text
            for ch in "git":
                text = terminal.send(ch, 0.3)
            text = terminal.pump(2.5)
            assert "\u2315 git" in text, text
            assert "Could not reach the MCP Registry" in text, text
            text = terminal.send("\x1b", 0.8)
            assert "Search the MCP Registry" in text, text
            terminal.send("\x1b", 0.8)
            rows.append({"command": "/mcp discover", "passed": True})
            # A double esc clears a draft; one esc only arms it.
            for ch in "half a draft":
                terminal.send(ch, 0.05)
            text = terminal.pump(1.0)
            assert "half a draft" in text, text
            text = terminal.send("\x1b", 0.3)
            assert "esc again to clear" in text.lower(), text
            text = terminal.send("\x1b", 1.0)
            assert "half a draft" not in text, text
            rows.append({"command": "esc esc", "passed": True})
            # Skills and plugin commands are slash commands, named as Claude
            # Code names them: `/name`, `/plugin:name`.
            check("/plugins marketplace add " + str(market), "eval-market")
            check("/plugins install evalkit@eval-market", "Loaded now")
            for typed, listed in [
                ("/eval-own", "/eval-own-skill"),
                ("/evalkit:", "/evalkit:plan-it"),
                ("/evalkit:sh", "/evalkit:ship"),
            ]:
                for ch in typed:
                    terminal.send(ch, 0.05)
                text = terminal.pump(1.0)
                assert listed in text, (typed, text)
                assert "/skill:" not in text, text
                terminal.send("", 0.3)
                rows.append({"command": f"menu {typed}", "passed": True})
            text = terminal.pump(0.5)
            # alt+v pastes the clipboard image as an [Image #1] chip; clearing
            # the draft drops it.
            def composer(screen):
                return [line for line in screen.splitlines() if line.lstrip().startswith("❯")][-1]
            text = terminal.send("v", 2.0)
            assert "[Image #1]" in composer(text), text
            assert "1 image attached" in text, text
            assert "Pasted [Image #1]" in text, text
            terminal.send("", 0.3)
            text = terminal.send("", 1.0)
            assert "[Image #1]" not in composer(text), text
            assert "image attached" not in text, text
            rows.append({"command": "alt+v image chip", "passed": True})
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
