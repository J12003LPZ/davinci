"""Reproducible DaVinci language-intelligence benchmark.

The harness never installs servers. Provision them separately, point the product at
those installations through normal trusted settings, and record those identities
next to the JSON artifact. Unknown RSS/child-process metrics are written as null.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
from pathlib import Path
import platform
import shutil
import statistics
import subprocess
import tempfile
import time


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def percentile(values: list[float], q: float) -> float | None:
    if not values:
        return None
    ordered = sorted(values)
    index = int(round((len(ordered) - 1) * q))
    return ordered[index]


def fixture(root: Path) -> None:
    agent = root / "agent"
    agent.mkdir()
    (root / "package.json").write_text('{"private":true}', encoding="utf-8")
    (root / "tsconfig.json").write_text('{"include":["web/*.ts"]}', encoding="utf-8")
    (root / "web").mkdir()
    (root / "web/api.ts").write_text("export function greet(name: string): string { return name; }\n", encoding="utf-8")
    (root / "web/use.ts").write_text("import { greet } from './api';\ngreet('DaVinci');\n", encoding="utf-8")
    (root / "rust/src").mkdir(parents=True)
    (root / "rust/Cargo.toml").write_text(
        '[package]\nname="bench"\nversion="0.1.0"\nedition="2021"\n', encoding="utf-8"
    )
    (root / "rust/src/lib.rs").write_text("pub fn answer() -> u8 { 42 }\n", encoding="utf-8")
    (root / "python").mkdir()
    (root / "python/pyrightconfig.json").write_text('{"include":["."]}', encoding="utf-8")
    (root / "python/app.py").write_text("def answer() -> int:\n    return 42\n", encoding="utf-8")

    settings: dict = {"languageIntelligence": {"enabled": True}}
    rust_server = os.environ.get("DAVINCI_TEST_RUST_ANALYZER")
    if rust_server:
        profile = {
            "server": {"program": rust_server, "args": []},
            "requestTimeoutMs": 30000,
            "initializationTimeoutMs": 60000,
            "coldRequestTimeoutMs": 120000,
        }
        if os.environ.get("DAVINCI_TEST_RUST_SYSROOT"):
            profile["sysroot"] = os.environ["DAVINCI_TEST_RUST_SYSROOT"]
        if os.environ.get("DAVINCI_TEST_RUST_SRC"):
            profile["sysrootSrc"] = os.environ["DAVINCI_TEST_RUST_SRC"]
        settings["languageIntelligence"]["rust"] = profile

    python_server = os.environ.get("DAVINCI_TEST_BASEDPYRIGHT") or os.environ.get("DAVINCI_TEST_PYRIGHT")
    if python_server and os.environ.get("DAVINCI_TEST_PYTHON"):
        based = bool(os.environ.get("DAVINCI_TEST_BASEDPYRIGHT"))
        settings["languageIntelligence"]["python"] = {
            "backend": "basedpyright" if based else "pyright",
            "interpreter": os.environ["DAVINCI_TEST_PYTHON"],
            "server": {"program": python_server, "args": ["--stdio"]},
            "requestTimeoutMs": 30000,
            "initializationTimeoutMs": 60000,
            "coldRequestTimeoutMs": 120000,
        }

    ts = os.environ.get("DAVINCI_TEST_TYPESCRIPT")
    tls = os.environ.get("DAVINCI_TEST_LANGUAGE_SERVER")
    if ts and tls:
        node_modules = root / "node_modules"
        node_modules.mkdir(exist_ok=True)
        shutil.copytree(Path(ts), node_modules / "typescript", dirs_exist_ok=True)
        shutil.copytree(Path(tls), node_modules / "typescript-language-server", dirs_exist_ok=True)

    (agent / "settings.json").write_text(json.dumps(settings), encoding="utf-8")


def calls_for(scenario: str) -> list[dict]:
    if scenario == "startup":
        return [{"name": "tool_search", "arguments": {"query": "lsp_definition"}}]
    if scenario == "warm_queries":
        return [
            {"name": "lsp_hover", "arguments": {"path": "web/use.ts", "line": 2, "column": 2}},
            {"name": "lsp_hover", "arguments": {"path": "web/use.ts", "line": 2, "column": 2}},
        ]
    if scenario == "polyglot":
        return [
            {"name": "lsp_document_symbols", "arguments": {"path": "web/api.ts"}},
            {"name": "lsp_document_symbols", "arguments": {"path": "rust/src/lib.rs"}},
            {"name": "lsp_document_symbols", "arguments": {"path": "python/app.py"}},
        ]
    if scenario == "graph_workers":
        return [
            {
                "name": "graph_run",
                "arguments": {
                    "goal": "Language intelligence benchmark dry run",
                    "dryRun": True,
                },
            }
        ]
    raise ValueError(scenario)


def run_once(binary: Path, scenario: str, root: Path) -> dict:
    env = {k: v for k, v in os.environ.items() if not k.startswith("PI_GRAPH_")}
    env.update(
        DAVINCI_CODING_AGENT_DIR=str(root / "agent"),
        PI_CODING_AGENT_DIR=str(root / "agent"),
        PI_OFFLINE="1",
        DAVINCI_OFFLINE="1",
        PI_DISABLE_NETWORK="1",
        PI_HOOKS_DRY_RUN="1",
        OPENAI_API_KEY="offline-benchmark-placeholder",
        PI_OFFLINE_TOOL_CALL=json.dumps(calls_for(scenario)),
    )
    command = [
        str(binary),
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
        "Run the deterministic benchmark calls.",
    ]
    started = time.perf_counter()
    completed = subprocess.run(
        command,
        cwd=root,
        env=env,
        capture_output=True,
        text=True,
        encoding="utf-8",
        timeout=180,
    )
    elapsed_ms = (time.perf_counter() - started) * 1000.0
    events = []
    for line in completed.stdout.splitlines():
        if line.startswith("{"):
            try:
                events.append(json.loads(line))
            except json.JSONDecodeError:
                pass
    tool_results = [event for event in events if event.get("type") == "tool_execution_end"]
    return {
        "wall_ms": elapsed_ms,
        "exit_code": completed.returncode,
        "tool_results": len(tool_results),
        "tool_errors": sum(bool(event.get("isError")) for event in tool_results),
        "rss_bytes": None,
        "child_count": None,
        "stderr_tail": completed.stderr[-2000:],
    }


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--output", required=True, type=Path)
    parser.add_argument("--repetitions", type=int, default=20)
    parser.add_argument(
        "--scenario",
        required=True,
        choices=["startup", "warm_queries", "polyglot", "graph_workers"],
    )
    args = parser.parse_args()
    binary = args.binary.resolve()
    if args.repetitions < 1:
        raise SystemExit("--repetitions must be positive")
    if not binary.is_file():
        raise SystemExit(f"binary not found: {binary}")

    samples = []
    with tempfile.TemporaryDirectory(prefix="davinci-lsp-benchmark-") as directory:
        root = Path(directory)
        fixture(root)
        # One unreported warmup keeps startup distributions comparable.
        run_once(binary, args.scenario, root)
        for _ in range(args.repetitions):
            samples.append(run_once(binary, args.scenario, root))

    elapsed = [sample["wall_ms"] for sample in samples]
    report = {
        "schema_version": 1,
        "scenario": args.scenario,
        "repetitions": args.repetitions,
        "binary": str(binary),
        "binary_sha256": sha256(binary),
        "os": platform.platform(),
        "python": platform.python_version(),
        "wall_ms": {
            "samples": elapsed,
            "median": statistics.median(elapsed),
            "p95": percentile(elapsed, 0.95),
        },
        "timeout_count": 0,
        "restart_count": None,
        "freshness_failures": None,
        "runs": samples,
        "server_environment": {
            name: os.environ.get(name)
            for name in [
                "DAVINCI_TEST_RUST_ANALYZER",
                "DAVINCI_TEST_BASEDPYRIGHT",
                "DAVINCI_TEST_PYRIGHT",
                "DAVINCI_TEST_PYTHON",
            ]
        },
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"output": str(args.output), "median_ms": report["wall_ms"]["median"], "p95_ms": report["wall_ms"]["p95"]}))


if __name__ == "__main__":
    main()
