"""Live GPT-6 Luna/low governor eval using existing OpenAI Codex credentials.

Usage: python scripts/eval-live-governor.py <davinci-executable> <artifact-dir>
Creates only disposable workspace/configuration state. Does not log credentials.
"""
import live_auth
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile


def main():
    executable = str(Path(sys.argv[1]).resolve())
    output = Path(sys.argv[2]).resolve()
    output.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="davinci-governor-eval-") as temporary:
        root = Path(temporary)
        config = root / "config"
        config.mkdir()
        work = root / "work"
        work.mkdir()
        lines = [f"row {i:04d} routine measurement {i * 37:08d}" for i in range(900)]
        lines[451] = "AUDIT_HIDDEN_VALUE=sequoia-4827"
        (work / "large-log.txt").write_text("\n".join(lines) + "\n", encoding="utf-8")
        auth = config / "auth.json"
        lent = live_auth.lend(auth)
        (config / "settings.json").write_text(json.dumps({"maxModelTurns": 10}), encoding="utf-8")
        (config / "vector-memory.json").write_text(json.dumps({"enabled": False}), encoding="utf-8")
        (config / "token-governor.json").write_text(json.dumps({
            "enabled": True, "contentAware": False, "compressThresholdBytes": 512,
            "compressThresholdLines": 30, "keepHeadLines": 3, "keepTailLines": 3,
            "maxImportantLines": 1, "retrieveMaxBytes": 512}), encoding="utf-8")
        env = {k: v for k, v in os.environ.items() if not k.startswith(("PI_", "DAVINCI_"))}
        env.update(PI_CODING_AGENT_DIR=str(config), DAVINCI_CODING_AGENT_DIR=str(config))
        command = [executable, "--no-session", "--no-extensions", "--no-mcp", "--no-skills",
                   "--no-context-files", "--no-prompt-templates", "--provider", "openai-codex",
                   "--model", "gpt-6-luna", "--thinking", "low", "--permission-mode", "always-approve",
                   "--mode", "json", "-p", "-a",
                   "Test governor recovery alone; do not use agents or graph. "
                   "First use powershell to run Get-Content -LiteralPath large-log.txt, without filtering. "
                   "Then call retrieve_output with only its saved output id to read the first page. "
                   "Next call retrieve_output with the continuation cursor shown in that page, "
                   "to read the next page. Finally make a separate retrieve_output call with that id "
                   "and grep AUDIT_HIDDEN_VALUE, searching the entire output without a line range. "
                   "Reply GOVERNOR_OK and the retrieved value only after all three retrievals succeed. "
                   "Do not modify files."]
        try:
            result = subprocess.run(command, cwd=work, env=env, capture_output=True, text=True,
                                    encoding="utf-8", timeout=180)
            (output / "live-governor-paging.jsonl").write_text(result.stdout, encoding="utf-8")
            (output / "live-governor-paging.stderr").write_text(result.stderr, encoding="utf-8")
            events = [json.loads(line) for line in result.stdout.splitlines() if line.startswith("{")]
            ends = [e for e in events if e.get("type") == "tool_execution_end"]
            compressed = [e for e in ends if e.get("details", {}).get("tokenGovernor", {}).get("compressed")]
            retrieved = [e for e in ends if e.get("toolName") == "retrieve_output"]
            starts = [e.get("args", {}) for e in events
                      if e.get("type") == "tool_execution_start" and e.get("toolName") == "retrieve_output"]
            summary = {"model": "gpt-6-luna", "effort": "low", "returncode": result.returncode,
                       "compressed": len(compressed), "retrievals": len(retrieved),
                       "continuation_used": any(a.get("startLine", 1) > 1 for a in starts),
                       "hidden_value_recovered": any("452: AUDIT_HIDDEN_VALUE=sequoia-4827" in e.get("result", "") for e in retrieved)}
            (output / "live-governor-paging-summary.json").write_text(json.dumps(summary, indent=2), encoding="utf-8")
            assert result.returncode == 0, result.stderr
            assert compressed and len(retrieved) >= 3 and not any(e.get("isError") for e in ends), summary
            assert summary["continuation_used"] and summary["hidden_value_recovered"], summary
            assert "GOVERNOR_OK" in result.stdout
            print(json.dumps(summary))
        finally:
            live_auth.give_back(auth, lent)
            auth.unlink(missing_ok=True)


if __name__ == "__main__":
    main()
