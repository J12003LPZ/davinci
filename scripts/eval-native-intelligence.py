"""Exercise native progressive discovery through the real offline print-mode CLI.

No provider requests, user settings, extension packages or MCP servers are used.
Pass a freshly built davinci executable; add --lsp to test installed JS/TS servers.
"""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile


def main():
    executable = str(Path(sys.argv[1]).resolve())
    with tempfile.TemporaryDirectory(prefix="davinci-native-eval-") as directory:
        root = Path(directory)
        config = root / "agent"
        config.mkdir()
        mcp = config / "mcp.json"
        marker = root / "unexpected-mcp-start"
        mcp.write_text(json.dumps({"mcpServers": {"unused": {
            "command": sys.executable, "args": ["-c",
                "from pathlib import Path; Path('unexpected-mcp-start').touch()"]
        }}}), encoding="utf-8")
        env = {key: value for key, value in os.environ.items()
               if not key.startswith("PI_GRAPH_")}
        env.update(DAVINCI_CODING_AGENT_DIR=str(config), PI_CODING_AGENT_DIR=str(config),
                   PI_MCP_CONFIG=str(mcp), PI_OFFLINE="1", DAVINCI_OFFLINE="1",
                   PI_DISABLE_NETWORK="1", PI_HOOKS_DRY_RUN="1",
                   OPENAI_API_KEY="offline-eval-placeholder")
        queries = {"lsp_definition": "lsp_definition", "memory": "memory_search",
                   "governor": "retrieve_output", "graph_run": "graph_run"}
        calls = [{"name": "tool_search", "arguments": {"query": query}}
                 for query in queries]
        lsp = "--lsp" in sys.argv[2:]
        if lsp:
            (root / "package.json").write_text('{"private":true}', encoding="utf-8")
            (root / "tsconfig.json").write_text(json.dumps({"compilerOptions": {
                "strict": True, "allowJs": True, "checkJs": True, "noEmit": True},
                "include": ["*.ts", "*.js"]}), encoding="utf-8")
            (root / "api.ts").write_text("export function greet(name: string): string { return name; }\n", encoding="utf-8")
            (root / "use.ts").write_text("import { greet } from './api';\ngreet('DaVinci');\n", encoding="utf-8")
            (root / "js-api.js").write_text("export function twice(value) { return value * 2; }\n", encoding="utf-8")
            (root / "use.js").write_text("import { twice } from './js-api';\ntwice(2);\n", encoding="utf-8")
            calls.extend({"name": "lsp_definition", "arguments": {
                "path": path, "line": 2, "column": 2}} for path in ["use.ts", "use.js"])
        calls.append({"name": "graph_run", "arguments": {
            "goal": "Native print-mode graph fixture", "dryRun": True}})
        env["PI_OFFLINE_TOOL_CALL"] = json.dumps(calls)
        command = [executable, "--offline", "--no-session", "--no-extensions", "--no-mcp", "--no-skills",
             "--no-context-files", "--no-prompt-templates", "--permission-mode", "always-approve",
             "--provider", "openai", "--model", "gpt-5", "--mode", "json", "-p",
             "-a",
             "Check native intelligence discovery"]
        result = subprocess.run(command,
            cwd=root, env=env, capture_output=True, text=True, encoding="utf-8", timeout=60)
        assert result.returncode == 0, result.stderr
        assert not marker.exists(), "--no-mcp started an unrelated server"
        events = [json.loads(line) for line in result.stdout.splitlines() if line.startswith("{")]
        results = [event for event in events if event.get("type") == "tool_execution_end"]
        assert len(results) == len(calls), results
        assert all(not event.get("isError") for event in results), results
        states = list((root / ".davinci/graph/runs").glob("*/state.json"))
        assert len(states) == 1, states
        state = json.loads(states[0].read_text(encoding="utf-8"))
        assert state["phase"] == "done", state["phase"]
        for (query, expected), event in zip(queries.items(), results):
            assert not event.get("isError"), event
            assert expected in json.dumps(event.get("result", {})), event
        if lsp:
            for event, target in zip(results[len(queries):], ["api.ts", "js-api.js"]):
                assert not event.get("isError"), event
                assert target in json.dumps(event.get("result", {})), event
            env["PI_OFFLINE_TOOL_CALL"] = json.dumps([calls[len(queries)]])
            denied = subprocess.run(["--no-approve" if arg == "-a" else arg for arg in command],
                                    cwd=root, env=env, capture_output=True, text=True,
                                    encoding="utf-8", timeout=60)
            assert denied.returncode == 0, denied.stderr
            denied_events = [json.loads(line) for line in denied.stdout.splitlines()
                             if line.startswith("{")]
            denied_results = [event for event in denied_events
                              if event.get("type") == "tool_execution_end"]
            assert len(denied_results) == 1, denied_results
            assert denied_results[0].get("isError"), denied_results
            assert "server_launch_denied" in json.dumps(denied_results), denied_results
        print(json.dumps({"print_mode": "passed", "native_discovery": list(queries.values()),
                          "legacy_extensions": "disabled", "mcp_spawned": False, "provider_calls": 0,
                          "print_graph_dry_run": "passed",
                          "real_js_ts_definitions": "passed" if lsp else "not requested"}))


if __name__ == "__main__":
    main()
