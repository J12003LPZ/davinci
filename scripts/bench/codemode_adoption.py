"""Unsteered Codemode adoption and answer-quality eval.

Runs the same read-only survey questions with Codemode on and off (no appended
system prompt), then grades every final answer against facts computed from the
repository, so a pass means "correct", not "plausible".

    python scripts/bench/codemode_adoption.py run --binary davinci.exe --repo . \
        --credential-source ~/.pi/agent --out /absolute/new/dir --reps 2
    python scripts/bench/codemode_adoption.py report --out /absolute/new/dir --repo .

`report` exits 1 when the Codemode-on arm falls below the adoption or
correctness threshold. Live runs use the model account behind
`--credential-source`; `report` and the unit tests are offline.
"""
import argparse
import collections
import csv
import json
import os
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

CODEMODE_DIR = "crates/davinci-agent/src/codemode"
PROMPTS = {
    "p1-codemode-dir": "In crates/davinci-agent/src/codemode, list each .rs file with its line count and the first doc comment line it has. Answer as a short table.",
    "p2-constructors": "Find every place in crates/ that constructs a CodeModeChildOutcome and report file and line for each.",
    "p3-root-config": "Which of Cargo.toml, rust-toolchain.toml, deny.toml and rustfmt.toml exist at the repo root, and what are the first 3 lines of each that exists?",
    "p4-crate-versions": "For each crate under crates/, report its version from its Cargo.toml and the number of files directly in its src directory.",
    "p5-docs-summary": "Read the headings of docs/codemode.md and the first 5 lines of CHANGELOG.md, then summarize both in two sentences.",
    "p6-worker-slots": "Find where WorkerSlotCapacity is defined and list every file that calls acquire_exclusive.",
}
ROOT_FILES = ["Cargo.toml", "rust-toolchain.toml", "deny.toml", "rustfmt.toml"]
MIN_ADOPTION = 0.5  # share of Codemode-on runs that call codemode at least once
MIN_CORRECT = 0.8  # share of Codemode-on answers that pass their check


def _text(repo, rel):
    return Path(repo, rel).read_text(encoding="utf-8", errors="replace")


def _git(repo, *args):
    done = subprocess.run(["git", *args], cwd=repo, capture_output=True, text=True, encoding="utf-8", errors="replace")
    return done.stdout if done.returncode == 0 else ""


def _rust_files(repo):
    for path in sorted(Path(repo, "crates").rglob("*.rs")):
        yield path.relative_to(repo).as_posix(), path


def truth(repo):
    """Facts the answers are graded against, read from the working tree."""
    repo = Path(repo)
    files = sorted(p.name for p in Path(repo, CODEMODE_DIR).glob("*.rs"))
    counts = {}
    for name in files:
        seen = {len(_text(repo, f"{CODEMODE_DIR}/{name}").splitlines())}
        # Accept any version of the file from recent history: a long run can span commits.
        for rev in _git(repo, "log", "-n", "8", "--format=%H", "--", f"{CODEMODE_DIR}/{name}").split():
            old = _git(repo, "show", f"{rev}:{CODEMODE_DIR}/{name}")
            if old:
                seen.add(len(old.splitlines()))
        counts[name] = seen
    constructors, callers = [], set()
    for rel, path in _rust_files(repo):
        for number, line in enumerate(path.read_text(encoding="utf-8", errors="replace").splitlines(), 1):
            if "CodeModeChildOutcome {" in line and "pub struct" not in line:
                constructors.append((Path(rel).name, number))
            if "acquire_exclusive" in line and Path(rel).name != "capacity.rs":
                callers.add(Path(rel).name)
    version = re.search(r'^version\s*=\s*"([^"]+)"', _text(repo, "Cargo.toml"), re.M)
    src = {}
    for crate in sorted(p for p in Path(repo, "crates").iterdir() if (p / "src").is_dir()):
        entries = list((crate / "src").iterdir())
        src[crate.name] = (sum(e.is_file() for e in entries), len(entries))
    first3 = {
        name: [l.strip() for l in _text(repo, name).splitlines()[:3] if l.strip()]
        for name in ROOT_FILES
        if Path(repo, name).exists()
    }
    return {"files": files, "counts": counts, "constructors": constructors, "callers": sorted(callers),
            "version": version.group(1) if version else None, "src": src, "first3": first3}


def _numbers(row):
    return {int(n) for n in re.findall(r"(?<![\w.])\d+(?![\w.])", row)}


def check(prompt, answer, t):
    """Reasons the answer is wrong; an empty list means it passed."""
    if not answer.strip():
        return ["no final answer"]
    miss = []
    if prompt == "p1-codemode-dir":
        for name in t["files"]:
            row = next((l for l in answer.splitlines() if name in l), None)
            if row is None:
                miss.append(f"{name} absent")
            elif not _numbers(row.replace(name, "")) & t["counts"][name]:
                miss.append(f"{name} line count")
    elif prompt == "p2-constructors":
        for base, number in t["constructors"]:
            if base not in answer or not re.search(rf"(?<![\w.]){number}(?![\w.])", answer):
                miss.append(f"{base}:{number}")
    elif prompt == "p3-root-config":
        for name in ROOT_FILES:
            if name in t["first3"]:
                miss += [f"{name}: {l!r}" for l in [name] + t["first3"][name] if l not in answer]
    elif prompt == "p4-crate-versions":
        for crate, (files, entries) in t["src"].items():
            row = next((l for l in answer.splitlines() if re.search(re.escape(crate) + r"(?![-\w])", l)), None)
            if row is None:
                miss.append(f"{crate} absent")
                continue
            if t["version"] and t["version"] not in row:
                miss.append(f"{crate} version")
            if not _numbers(row.replace(crate, "")) & {files, entries}:
                miss.append(f"{crate} src count")
    elif prompt == "p5-docs-summary":
        if "codemode" not in answer.lower():
            miss.append("no codemode doc content")
        if not re.search(r"changelog|release|unreleased|\d+\.\d+\.\d+", answer, re.I):
            miss.append("no changelog content")
        if len(re.findall(r"[.!?](\s|$)", answer)) < 2:
            miss.append("not two sentences")
    elif prompt == "p6-worker-slots":
        if "capacity.rs" not in answer:
            miss.append("definition file")
        if t["callers"]:
            miss += [f"caller {c}" for c in t["callers"] if c not in answer]
        elif not re.search(r"\b(no|none|not found|zero|0|nothing|does not|doesn't)\b", answer, re.I):
            miss.append("did not report that nothing calls it")
    return miss


def read_run(jsonl):
    """Final answer and tool-start counts from one run's `--mode json` stream."""
    answer, tools = "", collections.Counter()
    with open(jsonl, encoding="utf-8", errors="replace") as stream:
        for line in stream:
            try:
                event = json.loads(line)
            except ValueError:
                continue
            if event.get("type") == "tool_execution_start":
                tools[event.get("toolName")] += 1
            elif event.get("type") == "agent_end":
                for message in reversed(event.get("messages", [])):
                    if message.get("role") == "assistant":
                        answer = "".join(c.get("text", "") for c in message.get("content", []) if c.get("type") == "text")
                        break
    return answer, tools


def settings(arm, node, host):
    return {
        "compaction": {"enabled": True, "reserveTokens": 16384, "keepRecentTokens": 20000},
        "showCacheMissNotices": False, "transport": "auto", "serviceTier": None, "effortPolicy": "fixed",
        "toolSurface": "full", "autoVerify": True, "promptProfile": "stable", "contextVm": "off",
        "codemode": {"mode": "read-only", "nodePath": str(node), "hostPath": str(host)} if arm == "on" else {"mode": "off"},
    }


def run(args):
    out = Path(args.out).resolve()
    if out.exists():
        sys.exit("--out must be a new directory")
    out.mkdir(parents=True)
    rows = []
    for rep in range(args.reps):
        for name, prompt in PROMPTS.items():
            for arm in args.arms:
                agent = out / f"agent-{arm}"
                agent.mkdir(exist_ok=True)
                (agent / "settings.json").write_text(json.dumps(settings(arm, args.node, args.host)), encoding="utf-8")
                # Copied fresh each run so a refreshed token carries forward; never printed.
                shutil.copy(Path(args.credential_source) / "auth.json", agent / "auth.json")
                run_dir = out / "runs" / f"{arm}-{name}-r{rep}"
                run_dir.mkdir(parents=True)
                started = time.time()
                try:
                    with open(run_dir / "out.jsonl", "w", encoding="utf-8") as stdout, open(run_dir / "err.txt", "w", encoding="utf-8") as stderr:
                        code = subprocess.run(
                            [args.binary, "--provider", "openai-codex", "--model", args.model, "--permission-mode", "plan-mode",
                             "-p", "--mode", "json", "--session-dir", str(run_dir), prompt],
                            cwd=args.repo, env=dict(os.environ, DAVINCI_CODING_AGENT_DIR=str(agent)),
                            stdout=stdout, stderr=stderr, timeout=args.timeout).returncode
                except subprocess.TimeoutExpired:
                    code = "timeout"
                rows.append({"arm": arm, "prompt": name, "rep": rep, "exit": code, "wall_s": round(time.time() - started, 1)})
                print(json.dumps(rows[-1]), flush=True)
    with open(out / "runs.csv", "w", newline="", encoding="utf-8") as f:
        writer = csv.DictWriter(f, fieldnames=list(rows[0]))
        writer.writeheader()
        writer.writerows(rows)


def report(args):
    out, t = Path(args.out), truth(args.repo)
    arms = collections.defaultdict(lambda: {"runs": 0, "correct": 0, "adopted": 0, "tools": 0, "failures": []})
    for run_dir in sorted((out / "runs").iterdir()):
        arm, prompt = run_dir.name.split("-", 1)[0], re.sub(r"-r\d+$", "", run_dir.name.split("-", 1)[1])
        answer, tools = read_run(run_dir / "out.jsonl")
        miss = check(prompt, answer, t)
        a = arms[arm]
        a["runs"] += 1
        a["correct"] += not miss
        a["adopted"] += tools.get("codemode", 0) > 0
        a["tools"] += sum(tools.values())
        if miss:
            a["failures"].append(f"{run_dir.name}: {'; '.join(miss)[:160]}")
    for arm, a in sorted(arms.items()):
        print(f"{arm:4} runs={a['runs']} correct={a['correct']}/{a['runs']} using_codemode={a['adopted']}/{a['runs']} tool_calls={a['tools']}")
        for failure in a["failures"]:
            print("   ", failure)
    on = arms.get("on")
    if not on or not on["runs"]:
        sys.exit("no Codemode-on runs to judge")
    adoption, correct = on["adopted"] / on["runs"], on["correct"] / on["runs"]
    print(f"adoption {adoption:.0%} (min {MIN_ADOPTION:.0%}), correct {correct:.0%} (min {MIN_CORRECT:.0%})")
    return adoption >= MIN_ADOPTION and correct >= MIN_CORRECT


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    sub = parser.add_subparsers(dest="command", required=True)
    live = sub.add_parser("run", help="live runs; consumes model usage")
    live.add_argument("--binary", required=True)
    live.add_argument("--repo", required=True)
    live.add_argument("--credential-source", required=True, help="agent dir holding auth.json")
    live.add_argument("--out", required=True)
    live.add_argument("--reps", type=int, default=2)
    live.add_argument("--arms", nargs="+", choices=["on", "off"], default=["on", "off"])
    live.add_argument("--model", default="gpt-6-luna")
    live.add_argument("--timeout", type=int, default=300)
    live.add_argument("--node", default=Path.home() / ".davinci/agent/codemode/node/node.exe")
    live.add_argument("--host", default=Path.home() / ".davinci/agent/codemode/host")
    rep = sub.add_parser("report", help="offline grading of a finished run directory")
    rep.add_argument("--out", required=True)
    rep.add_argument("--repo", required=True)
    args = parser.parse_args(argv)
    if args.command == "run":
        run(args)
        return 0
    return 0 if report(args) else 1


if __name__ == "__main__":
    sys.exit(main())
