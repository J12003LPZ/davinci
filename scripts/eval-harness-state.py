"""Exercise memory/setup persistence through the built CLI in disposable directories.

No provider calls are made. --embedding tests an already installed EmbeddingGemma
through local Ollama; it never downloads a model or invokes memory extraction.
"""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("executable")
    parser.add_argument("--embedding", action="store_true")
    args = parser.parse_args()
    executable = str(Path(args.executable).resolve())
    results = []
    with tempfile.TemporaryDirectory(prefix="davinci-state-eval-") as directory:
        root = Path(directory)
        subprocess.run(["git", "init", "--quiet", str(root)], check=True, capture_output=True)
        config = root / "config"
        config.mkdir()
        memory_config = {"enabled": args.embedding, "promotion": False,
                         "automaticRetrieval": False, "embeddingModel": "embeddinggemma",
                         "embedTimeoutSeconds": 60}
        (config / "vector-memory.json").write_text(json.dumps(memory_config), encoding="utf-8")
        env = {k: v for k, v in os.environ.items() if not k.startswith(("PI_", "DAVINCI_"))}
        env.update(PI_CODING_AGENT_DIR=str(config), DAVINCI_CODING_AGENT_DIR=str(config),
                   PI_OFFLINE="1", DAVINCI_OFFLINE="1", PI_HOOKS_DRY_RUN="1")
        command = [executable, "--offline", "--no-session", "--no-extensions", "--no-mcp",
                   "--no-skills", "--no-context-files", "--no-prompt-templates",
                   "--provider", "openai-codex", "--model", "gpt-6-luna", "--thinking", "low", "-p"]

        def run(slash):
            result = subprocess.run(command + [slash], cwd=root, env=env, capture_output=True,
                                    text=True, encoding="utf-8", timeout=120)
            assert result.returncode == 0, (slash, result.stdout, result.stderr)
            prefix = slash.split()[0] + ": "
            assert result.stdout.startswith(prefix), result.stdout
            return json.loads(result.stdout[len(prefix):])

        store = root / ".davinci/vector-memory/records.jsonl"
        store.parent.mkdir(parents=True)
        repo_id = run("/memory-status")["repoId"]
        record = {"id": "duplicate", "repoId": repo_id, "kind": "fact",
                  "text": "The audit deployment region is eu-west-3", "source": "audit",
                  "contentHash": "audit-region", "importance": 0.9, "createdAt": 1}
        foreign = dict(record, id="foreign", repoId="foreign-repository")
        store.write_text("\n".join(json.dumps(r) for r in [record, record, record, foreign]), encoding="utf-8")
        legacy = root / ".pi/vector-memory/records.jsonl"
        legacy.parent.mkdir(parents=True)
        legacy.write_bytes(store.read_bytes())
        repaired = run("/memory-reindex")
        assert repaired["repairedIds"] == 2, repaired
        stored = [json.loads(line) for line in store.read_text(encoding="utf-8").splitlines()]
        assert len({r["id"] for r in stored}) == 4
        assert run("/memory-reindex")["repairedIds"] == 0
        if args.embedding:
            assert repaired["reembedded"] == 3 and repaired["embeddingError"] is None, repaired
            assert all(len(r["embedding"]) == 768 for r in stored if r["repoId"] == repo_id)
            hits = run("/memory-search deployment region")
            assert "eu-west-3" in json.dumps(hits), hits
        results.append({"memory_duplicate_repair": "passed", "real_embedding": args.embedding})
        run("/memory-clear")
        assert [json.loads(line) for line in store.read_text(encoding="utf-8").splitlines()] == [foreign]
        assert run("/memory-status")["records"] == 0
        results.append({"memory_clear_foreign_and_legacy": "passed"})

        # Disable embedding for setup so setup cannot start or download anything.
        memory_config["enabled"] = False
        (config / "vector-memory.json").write_text(json.dumps(memory_config), encoding="utf-8")
        ignore = root / ".gitignore"
        original = b"# user content\n\xff\xfe\n"
        ignore.write_bytes(original)
        status = run("/setup")
        assert ignore.read_bytes() == original, status
        assert "could not read" in json.dumps(status).lower(), status
        ignore.write_text(".davinci/*\n!/.davinci/vector-memory/\n", encoding="utf-8")
        run("/setup")
        assert "/.davinci/vector-memory/" in ignore.read_text(encoding="utf-8").splitlines()
        status = run("/setup check")
        assert next(step for step in status["steps"] if step["id"] == "gitignore")["state"] == "ready"
        results.append({"setup_preserves_unreadable_and_repairs_negated_ignores": "passed"})
    print(json.dumps(results))


if __name__ == "__main__":
    main()
