"""Exercise native progressive discovery through the real offline print-mode CLI.

No provider requests, extension packages, or MCP servers are used. Real language
servers are optional and must already be provisioned; this script never installs
or downloads them.
"""
import argparse
import json
import os
import shutil
from pathlib import Path
import subprocess
import sys
import tempfile


def server_settings(root: Path, languages: set[str]) -> tuple[dict, list[tuple[str, str]]]:
    settings: dict = {"languageIntelligence": {"enabled": True}}
    checks: list[tuple[str, str]] = []
    if "rust" in languages and os.environ.get("DAVINCI_TEST_RUST_ANALYZER"):
        rust = root / "rust"
        (rust / "src").mkdir(parents=True)
        (rust / "Cargo.toml").write_text(
            '[package]\nname="native_eval"\nversion="0.1.0"\nedition="2021"\n',
            encoding="utf-8",
        )
        (rust / "src/lib.rs").write_text(
            "pub trait Marker {}\npub struct Item;\nimpl Marker for Item {}\n",
            encoding="utf-8",
        )
        profile = {
            "server": {"program": os.environ["DAVINCI_TEST_RUST_ANALYZER"], "args": []},
            "requestTimeoutMs": 30000,
            "initializationTimeoutMs": 60000,
            "coldRequestTimeoutMs": 120000,
        }
        if os.environ.get("DAVINCI_TEST_RUST_SYSROOT"):
            profile["sysroot"] = os.environ["DAVINCI_TEST_RUST_SYSROOT"]
        if os.environ.get("DAVINCI_TEST_RUST_SRC"):
            profile["sysrootSrc"] = os.environ["DAVINCI_TEST_RUST_SRC"]
        settings["languageIntelligence"]["rust"] = profile
        checks.append(("rust/src/lib.rs", "rust/src/lib.rs"))

    python_server = os.environ.get("DAVINCI_TEST_BASEDPYRIGHT") or os.environ.get("DAVINCI_TEST_PYRIGHT")
    if "python" in languages and python_server and os.environ.get("DAVINCI_TEST_PYTHON"):
        py = root / "python"
        py.mkdir()
        (py / "pyrightconfig.json").write_text('{"include":["."]}', encoding="utf-8")
        (py / "models.py").write_text("class User:\n    name: str\n", encoding="utf-8")
        (py / "app.py").write_text(
            "from models import User\n\ndef greeting(user: User) -> str:\n    return user.name\n",
            encoding="utf-8",
        )
        based = bool(os.environ.get("DAVINCI_TEST_BASEDPYRIGHT"))
        settings["languageIntelligence"]["python"] = {
            "backend": "basedpyright" if based else "pyright",
            "interpreter": os.environ["DAVINCI_TEST_PYTHON"],
            "server": {"program": python_server, "args": ["--stdio"]},
            "requestTimeoutMs": 30000,
            "initializationTimeoutMs": 60000,
            "coldRequestTimeoutMs": 120000,
        }
        checks.append(("python/app.py", "python/models.py"))
    return settings, checks


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("executable")
    parser.add_argument("--lsp", action="store_true")
    parser.add_argument("--languages", default="typescript,rust,python")
    args = parser.parse_args()
    executable = str(Path(args.executable).resolve())
    languages = {item.strip() for item in args.languages.split(",") if item.strip()}

    with tempfile.TemporaryDirectory(prefix="davinci-native-eval-") as directory:
        root = Path(directory)
        config = root / "agent"
        config.mkdir()
        mcp = config / "mcp.json"
        marker = root / "unexpected-mcp-start"
        mcp.write_text(
            json.dumps(
                {
                    "mcpServers": {
                        "unused": {
                            "command": sys.executable,
                            "args": [
                                "-c",
                                "from pathlib import Path; Path('unexpected-mcp-start').touch()",
                            ],
                        }
                    }
                }
            ),
            encoding="utf-8",
        )
        env = {key: value for key, value in os.environ.items() if not key.startswith("PI_GRAPH_")}
        env.update(
            DAVINCI_CODING_AGENT_DIR=str(config),
            PI_CODING_AGENT_DIR=str(config),
            PI_MCP_CONFIG=str(mcp),
            PI_OFFLINE="1",
            DAVINCI_OFFLINE="1",
            PI_DISABLE_NETWORK="1",
            PI_HOOKS_DRY_RUN="1",
            OPENAI_API_KEY="offline-eval-placeholder",
        )
        queries = {
            "lsp_definition": "lsp_definition",
            "memory": "memory_search",
            "governor": "retrieve_output",
            "graph_run": "graph_run",
        }
        calls = [{"name": "tool_search", "arguments": {"query": query}} for query in queries]
        expected_targets: list[str] = []

        if args.lsp:
            settings, extra_checks = server_settings(root, languages)
            if extra_checks:
                (config / "settings.json").write_text(json.dumps(settings), encoding="utf-8")
            if "typescript" in languages:
                tls = os.environ.get("DAVINCI_TEST_LANGUAGE_SERVER")
                typescript = os.environ.get("DAVINCI_TEST_TYPESCRIPT")
                if tls and typescript:
                    source_node_modules = Path(tls).resolve().parent
                    shutil.copytree(
                        source_node_modules,
                        root / "node_modules",
                        dirs_exist_ok=True,
                    )
                (root / "package.json").write_text('{"private":true}', encoding="utf-8")
                (root / "tsconfig.json").write_text(
                    json.dumps(
                        {
                            "compilerOptions": {
                                "strict": True,
                                "allowJs": True,
                                "checkJs": True,
                                "noEmit": True,
                            },
                            "include": ["*.ts", "*.js"],
                        }
                    ),
                    encoding="utf-8",
                )
                (root / "api.ts").write_text(
                    "export function greet(name: string): string { return name; }\n",
                    encoding="utf-8",
                )
                (root / "use.ts").write_text(
                    "import { greet } from './api';\ngreet('DaVinci');\n",
                    encoding="utf-8",
                )
                calls.append(
                    {
                        "name": "lsp_definition",
                        "arguments": {"path": "use.ts", "line": 2, "column": 2},
                    }
                )
                expected_targets.append("api.ts")
            for source, expected in extra_checks:
                if source.endswith(".rs"):
                    arguments = {"path": source, "line": 3, "column": 7}
                else:
                    arguments = {"path": source, "line": 1, "column": 20}
                calls.append({"name": "lsp_definition", "arguments": arguments})
                expected_targets.append(expected)

        calls.append(
            {
                "name": "graph_run",
                "arguments": {"goal": "Native print-mode graph fixture", "dryRun": True},
            }
        )
        env["PI_OFFLINE_TOOL_CALL"] = json.dumps(calls)
        command = [
            executable,
            "--offline",
            "--no-session",
            "--no-extensions",
            "--no-mcp",
            "--no-skills",
            "--no-context-files",
            "--no-prompt-templates",
            "--permission-mode",
            "always-approve",
            "--provider",
            "openai",
            "--model",
            "gpt-5",
            "--mode",
            "json",
            "-p",
            "-a",
            "Check native intelligence discovery",
        ]
        result = subprocess.run(
            command,
            cwd=root,
            env=env,
            capture_output=True,
            text=True,
            encoding="utf-8",
            timeout=180,
        )
        assert result.returncode == 0, result.stderr
        assert not marker.exists(), "--no-mcp started an unrelated server"
        events = [json.loads(line) for line in result.stdout.splitlines() if line.startswith("{")]
        results = [event for event in events if event.get("type") == "tool_execution_end"]
        assert len(results) == len(calls), results
        states = list((root / ".davinci/graph/runs").glob("*/state.json"))
        assert len(states) == 1, states
        state = json.loads(states[0].read_text(encoding="utf-8"))
        assert state["phase"] == "done", state["phase"]
        for (query, expected), event in zip(queries.items(), results):
            assert not event.get("isError"), event
            assert expected in json.dumps(event.get("result", {})), event

        semantic_results = results[len(queries) : len(queries) + len(expected_targets)]
        for event, target in zip(semantic_results, expected_targets):
            assert not event.get("isError"), event
            assert target in json.dumps(event.get("result", {})), event

        if semantic_results:
            one_call = calls[len(queries)]
            env["PI_OFFLINE_TOOL_CALL"] = json.dumps([one_call])
            denied_command = ["--no-approve" if arg == "-a" else arg for arg in command]
            denied = subprocess.run(
                denied_command,
                cwd=root,
                env=env,
                capture_output=True,
                text=True,
                encoding="utf-8",
                timeout=180,
            )
            assert denied.returncode == 0, denied.stderr
            denied_events = [
                json.loads(line) for line in denied.stdout.splitlines() if line.startswith("{")
            ]
            denied_results = [
                event for event in denied_events if event.get("type") == "tool_execution_end"
            ]
            assert len(denied_results) == 1, denied_results
            assert denied_results[0].get("isError"), denied_results
            assert "server_launch_denied" in json.dumps(denied_results), denied_results

        print(
            json.dumps(
                {
                    "print_mode": "passed",
                    "native_discovery": list(queries.values()),
                    "legacy_extensions": "disabled",
                    "mcp_spawned": False,
                    "provider_calls": 0,
                    "print_graph_dry_run": "passed",
                    "real_semantic_targets": expected_targets if args.lsp else [],
                }
            )
        )


if __name__ == "__main__":
    main()
