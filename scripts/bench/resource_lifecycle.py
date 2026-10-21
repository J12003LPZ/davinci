#!/usr/bin/env python3
"""Compare local release fixtures or explicitly offline CLI runs. No provider calls."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import threading
import time


def parse_stat(text):
    """Linux stat: command names may contain spaces and closing parentheses."""
    fields = text[text.rindex(")") + 1:].split()
    if len(fields) < 22:
        raise ValueError("short proc stat")
    return {"state": fields[0], "parent": int(fields[1]),
            "started": int(fields[19]), "pages": max(0, int(fields[21]))}


def tree_members(records, root, started):
    if root not in records or records[root]["started"] != started:
        return None  # Root vanished or PID was reused; never count its replacement.
    children = {}
    for pid, entry in records.items():
        children.setdefault(entry["parent"], []).append(pid)
    pending, seen, live = [root], set(), []
    while pending:
        pid = pending.pop()
        if pid in seen:
            continue
        seen.add(pid)
        entry = records[pid]
        if entry["started"] < started:
            continue
        if entry["state"] not in ("Z", "X"):
            live.append(pid)
        pending.extend(children.get(pid, []))
    return live


def tree_rss(records, root, started, page_size):
    members = tree_members(records, root, started)
    return None if members is None else sum(records[pid]["pages"] for pid in members) * page_size


def proc_snapshot():
    records = {}
    for path in Path("/proc").iterdir():
        if path.name.isdigit():
            try:
                records[int(path.name)] = parse_stat((path / "stat").read_text())
            except (OSError, ValueError, IndexError):
                pass  # Processes can exit during collection.
    return records


def sha256(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            digest.update(chunk)
    return digest.hexdigest()


def run(binary, workload, kind, entries, interval):
    with tempfile.TemporaryDirectory(prefix="davinci-resource-run-") as temporary:
        root = Path(temporary)
        env = {key: value for key, value in os.environ.items()
               if not key.startswith(("PI_", "DAVINCI_", "OPENAI_", "ANTHROPIC_", "CLAUDE_"))}
        env.update(PI_OFFLINE="1", DAVINCI_OFFLINE="1", PI_DISABLE_NETWORK="1",
                   PI_SKIP_VERSION_CHECK="1", PI_CODING_AGENT_DIR=str(root / "agent"),
                   DAVINCI_CODING_AGENT_DIR=str(root / "agent"))
        if kind == "cli":
            command = [str(binary), "--offline", "--mode", "json", "--no-extensions",
                       "--no-skills", "--no-prompt-templates", "--no-mcp", "--no-session",
                       "--model", "openai-codex/gpt-6-astra", "--thinking", "high",
                       "-p", "local resource benchmark"]
        else:
            command = [str(binary), workload, str(entries)]
        began = time.perf_counter_ns()
        child = subprocess.Popen(command, cwd=root, env=env, stdin=subprocess.DEVNULL,
                                 stdout=subprocess.PIPE, stderr=subprocess.PIPE, start_new_session=True)
        outputs, overflow = {}, []

        def drain(name, stream):
            data = bytearray()
            while chunk := stream.read(16384):
                remaining = 1024 * 1024 - len(data)
                data.extend(chunk[:max(0, remaining)])
                if len(chunk) > remaining:
                    overflow.append(name)
            outputs[name] = bytes(data)

        readers = [threading.Thread(target=drain, args=(name, stream))
                   for name, stream in (("stdout", child.stdout), ("stderr", child.stderr))]
        for reader in readers:
            reader.start()
        peak, observations, identity, peak_children = None, 0, None, None
        try:
            while child.poll() is None:
                if time.perf_counter_ns() - began > 60_000_000_000:
                    raise TimeoutError("local workload exceeded 60 seconds")
                if sys.platform.startswith("linux"):
                    records = proc_snapshot()
                    if identity is None and child.pid in records:
                        identity = records[child.pid]["started"]
                    if identity is not None:
                        rss = tree_rss(records, child.pid, identity, os.sysconf("SC_PAGE_SIZE"))
                        if rss is not None:
                            observations += 1
                            peak = max(peak or 0, rss)
                            peak_children = max(peak_children or 0, len(tree_members(records, child.pid, identity)) - 1)
                time.sleep(interval)
            child.wait()
        finally:
            if child.poll() is None:
                if os.name == "posix":
                    import signal
                    os.killpg(child.pid, signal.SIGKILL)
                else:
                    child.kill()
                child.wait()
            for reader in readers:
                reader.join(timeout=3)
        if any(reader.is_alive() for reader in readers):
            raise RuntimeError("fixture left inherited output pipes open")
        if child.returncode or overflow:
            raise RuntimeError(f"workload failed ({child.returncode}, overflow={overflow}): "
                               + outputs.get("stderr", b"").decode(errors="replace")[-3000:])
        stdout = outputs["stdout"].decode()
        if kind == "cli":
            if "(offline)" not in stdout:
                raise RuntimeError("CLI did not produce the expected offline stub")
            metrics = {"offlineStubObserved": True}
        else:
            metrics = json.loads(stdout)
        return {"elapsedNs": time.perf_counter_ns() - began, "peakTreeRssBytes": peak,
                "rssObservations": observations, "peakObservedChildren": peak_children, "metrics": metrics}


def prepare(source, destination, candidate):
    source, destination = source.resolve(), destination.resolve()
    destination.mkdir(parents=True, exist_ok=True)
    graph = destination / "src/graph"
    graph.mkdir(parents=True, exist_ok=True)
    production = source / "crates/davinci-coding-agent/src/native_extensions/graph"
    shutil.copyfile(Path(__file__).with_name("resource_lifecycle_fixture.rs"), destination / "src/main.rs")
    for name in ["process.rs"] + (["process_output.rs"] if candidate else []):
        shutil.copyfile(production / name, graph / name)
    runtime_source = source / "crates/davinci-coding-agent/src"
    shutil.copyfile(runtime_source / "js_host.rs", destination / "src/js_host.rs")
    shutil.copyfile(runtime_source / "extension_runner.js", destination / "src/extension_runner.js")
    shutil.copyfile(source / "crates/davinci-coding-agent/tests/fixtures/language-server.cjs", destination / "src/language-server.cjs")
    if candidate:
        (destination / "src/js_host").mkdir(exist_ok=True)
        shutil.copyfile(runtime_source / "js_host/lifecycle.rs", destination / "src/js_host/lifecycle.rs")
    dependencies = {name: {"path": str(source / "crates" / name)}
                    for name in ("davinci-agent", "davinci-coding-agent", "davinci-sys", "davinci-tui", "davinci-ai", "davinci-session")}
    manifest = '[package]\nname = "davinci-resource-fixture"\nversion = "0.0.0"\nedition = "2021"\n'
    manifest += '[features]\ncandidate = []\n[dependencies]\n'
    for name, dependency in dependencies.items():
        manifest += f'{name} = {{ path = {json.dumps(dependency["path"])} }}\n'
    manifest += 'serde_json = "=1.0.154"\ntempfile = "=3.27.0"\nlibc = "=0.2.189"\n'
    manifest += 'serde = { version = "=1.0.229", features = ["derive"] }\nuuid = { version = "=1.11.0", features = ["v4"] }\n'
    manifest += '[profile.release]\nopt-level = 3\ndebug = false\nstrip = true\n'
    (destination / "Cargo.toml").write_text(manifest)
    shutil.copyfile(source / "Cargo.lock", destination / "Cargo.lock")
    identity = hashlib.sha256()
    for path in sorted((source / "crates").rglob("*")):
        if path.is_file() and path.suffix in (".rs", ".js", ".toml"):
            identity.update(str(path.relative_to(source)).encode() + b"\0")
            identity.update(bytes.fromhex(sha256(path)))
    copied = {str(path.relative_to(destination)): sha256(path)
              for path in sorted((destination / "src").rglob("*")) if path.is_file()}
    (destination / "source-manifest.json").write_text(json.dumps({
        "crateSourceDigest": identity.hexdigest(),
        "digestDefinition": "sorted crates/**/*.{rs,js,toml}: relative UTF-8 path, NUL, raw SHA256 bytes",
        "workspaceLockSha256": sha256(source / "Cargo.lock"),
        "copiedFiles": copied,
    }, indent=2) + "\n")
    print(destination / "Cargo.toml")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    prep = sub.add_parser("prepare")
    prep.add_argument("source", type=Path)
    prep.add_argument("destination", type=Path)
    prep.add_argument("--candidate", action="store_true")
    compare = sub.add_parser("compare")
    compare.add_argument("--baseline", required=True, type=Path)
    compare.add_argument("--candidate", required=True, type=Path)
    compare.add_argument("--kind", choices=("cli", "component"), default="component")
    compare.add_argument("--samples", type=int, default=5)
    compare.add_argument("--entries", type=int, default=50000)
    compare.add_argument("--interval-ms", type=float, default=2)
    compare.add_argument("--workloads", default="transcript,cache,output,host,js_idle,js_active,lsp_idle,lsp_active")
    compare.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    if args.command == "prepare":
        return prepare(args.source, args.destination, args.candidate)
    if not 1 <= args.samples <= 100 or not 1 <= args.interval_ms <= 100:
        parser.error("samples must be 1..100; interval-ms must be 1..100")
    report = {"platform": sys.platform, "samplesPerVariant": args.samples,
              "samplingIntervalMs": args.interval_ms, "kind": args.kind,
              "rssDefinition": "sum of observed process-tree RSS; shared pages counted per process; short peaks may be missed",
              "providerCalls": 0, "variants": {}}
    workloads = ["cli"] if args.kind == "cli" else args.workloads.split(",")
    for label in ("baseline", "candidate"):
        binary = getattr(args, label).resolve(strict=True)
        report["variants"][label] = {"binarySha256": sha256(binary), "workloads": {}}
        for workload in workloads:
            samples = [run(binary, workload, args.kind, args.entries, args.interval_ms / 1000)
                       for _ in range(args.samples)]
            report["variants"][label]["workloads"][workload] = samples
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n")
    print(args.output)


if __name__ == "__main__":
    main()
