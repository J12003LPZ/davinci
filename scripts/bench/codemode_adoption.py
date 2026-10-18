"""Unsteered Codemode adoption and answer-quality eval.

Runs the same read-only survey questions with Codemode on and off (no appended
system prompt), then grades every final answer against facts computed from the
repository, so a pass means "correct", not "plausible".

    python scripts/bench/codemode_adoption.py run --binary davinci.exe --repo . \
        --credential-source ~/.pi/agent --out /absolute/new/dir --reps 2
    python scripts/bench/codemode_adoption.py report --out /absolute/new/dir --repo .

`report` exits 1 when the Codemode-on arm falls below the adoption or strict
correctness threshold, or answers fewer questions correctly than the
Codemode-off arm. Partial credit (facts matched) is printed, never gated on. Live runs use the model account behind
`--credential-source`; `report` and the unit tests are offline.
"""
import argparse
import collections
import contextlib
import csv
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile
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
NOTHING = re.compile(r"\b(no|none|not found|zero|0|nothing|does not|doesn't|not defined|no such)\b", re.I)
NEGATION = re.compile(r"\b(no|not|none|missing|absent|doesn't|does not|n't|without|nonexistent)\b|n't", re.I)
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


def _first_doc(source):
    """Text of the first `///` or `//!` line in a Rust file, markers removed."""
    for line in source.splitlines():
        match = re.match(r"\s*//[/!]\s?(.*)$", line)
        if match and match.group(1).strip():
            return match.group(1).strip()
    return ""


def _squash(text):
    """Comparable form: no backticks or asterisks, single spaces, lower case."""
    return re.sub(r"\s+", " ", re.sub(r"[`*]", "", text)).strip().lower()


def truth(repo):
    """Facts the answers are graded against, read from the working tree."""
    repo = Path(repo)
    files = sorted(p.name for p in Path(repo, CODEMODE_DIR).glob("*.rs"))
    counts, docs = {}, {}
    for name in files:
        docs[name] = _first_doc(_text(repo, f"{CODEMODE_DIR}/{name}"))
        # The tree the run was made against, never an older revision of a file.
        counts[name] = {len(_text(repo, f"{CODEMODE_DIR}/{name}").splitlines())}
    constructors, callers, definition = [], set(), set()
    for rel, path in _rust_files(repo):
        for number, line in enumerate(path.read_text(encoding="utf-8", errors="replace").splitlines(), 1):
            if "CodeModeChildOutcome {" in line and "pub struct" not in line:
                constructors.append((Path(rel).name, number))
            if re.search(r"\bacquire_exclusive\s*\(", line) and not re.search(r"\bfn\s+acquire_exclusive\b", line):
                callers.add(Path(rel).name)
            if re.search(r"\b(struct|enum|type)\s+WorkerSlotCapacity\b", line):
                definition.add(Path(rel).name)
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
    headings = [h.strip() for h in re.findall(r"^#{1,6}\s+(.+)$", _text(repo, "docs/codemode.md"), re.M)]
    changelog = [l.strip() for l in _text(repo, "CHANGELOG.md").splitlines()[:5] if l.strip()]
    return {"files": files, "counts": counts, "docs": docs, "headings": headings, "changelog": changelog, "constructors": constructors, "callers": sorted(callers), "definition": sorted(definition),
            "version": version.group(1) if version else None, "src": src, "first3": first3}


def _numbers(row):
    return {int(n) for n in re.findall(r"(?<![\w.])\d+(?![\w.])", row)}


def _has_number(row, number):
    return bool(re.search(rf"(?<![\w.]){number}(?![\w.])", row))


def grade(prompt, answer, t):
    """(reasons the answer is wrong, number of facts checked) for one answer.

    Every fact is checked on the row that has to carry it, so a name here and
    a number elsewhere never add up to a pass.
    """
    if not answer.strip():
        return ["no final answer"], 1
    miss, total = [], 0

    def expect(ok, reason):
        nonlocal total
        total += 1
        if not ok:
            miss.append(reason)

    lines = answer.splitlines()
    if prompt == "p1-codemode-dir":
        for name in t["files"]:
            row = next((l for l in lines if name in l), None)
            expect(row is not None, f"{name} absent")
            row = row or ""
            expect(bool(_numbers(row.replace(name, "")) & t["counts"][name]), f"{name} line count")
            doc = _squash(t["docs"][name])
            expect(not doc or doc in _squash(row), f"{name} doc comment")
    elif prompt == "p2-constructors":
        # One expected location per construction site: its file and its line
        # number on the same row of the answer.
        for base, number in t["constructors"]:
            expect(any(base in l and _has_number(l, number) for l in lines), f"{base}:{number}")
    elif prompt == "p3-root-config":
        for name in ROOT_FILES:
            if name in t["first3"]:
                for fact in [name] + t["first3"][name]:
                    expect(fact in answer, f"{name}: {fact!r}")
            else:
                # A file that does not exist may be named only to say so.
                claimed = [l for l in lines if name in l and not NEGATION.search(l)]
                expect(not claimed, f"{name} does not exist but the answer lists it")
    elif prompt == "p4-crate-versions":
        for crate, (files, _entries) in t["src"].items():
            row = next((l for l in lines if re.search(re.escape(crate) + r"(?![-\w])", l)), None)
            expect(row is not None, f"{crate} absent")
            row = row or ""
            expect(bool(t["version"]) and t["version"] in row, f"{crate} version")
            # Version digits never count; the number must be the one tied to "files".
            bare = row.replace(crate, "").replace(t["version"] or "\0", "")
            tied = re.findall(r"(\d+)\s*(?:\w+\s+)?files?\b|\bfiles?\b\s*[:=]?\s*(\d+)", bare, re.I)
            stated = {int(n) for pair in tied for n in pair if n}
            if not stated:
                stated = _numbers(bare)  # a table cell: the only bare number is the count
            expect(stated == {files}, f"{crate} src count")
    elif prompt == "p5-docs-summary":
        text = _squash(answer)
        need = min(2, len(t["headings"]))
        seen = sum(_squash(h) in text for h in t["headings"])
        expect(seen >= need, f"only {seen} of the docs/codemode.md headings mentioned (need {need})")
        tokens = sorted({w.lower() for l in t["changelog"] for w in re.findall(r"\d+\.\d+\.\d+|unreleased", l, re.I)})
        for token in tokens:
            expect(token in text, f"changelog entry {token!r} missing")
        expect(len(re.findall(r"[.!?](\s|$)", answer)) >= 2, "not two sentences")
    elif prompt == "p6-worker-slots":
        for name in t["definition"]:
            expect(name in answer, f"definition file {name}")
        if not t["definition"]:
            expect(bool(NOTHING.search(answer)), "did not report that WorkerSlotCapacity is not defined")
        if t["callers"]:
            for c in t["callers"]:
                expect(c in answer, f"caller {c}")
        else:
            expect(bool(NOTHING.search(answer)), "did not report that nothing calls it")
    return miss, max(total, 1)


def check(prompt, answer, t):
    """Reasons the answer is wrong; an empty list means it passed."""
    return grade(prompt, answer, t)[0]


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
    (out / "commit.txt").write_text(_git(args.repo, "rev-parse", "HEAD").strip() + "\n", encoding="utf-8")
    # Agent dirs hold auth.json, so they live outside --out (which gets archived
    # and shared) in a private temp dir that is removed when the runs end.
    scratch = Path(tempfile.mkdtemp(prefix="codemode-adoption-"))
    try:
        _run_all(args, out, scratch, rows)
    finally:
        shutil.rmtree(scratch, ignore_errors=True)
    with open(out / "runs.csv", "w", newline="", encoding="utf-8") as f:
        writer = csv.DictWriter(f, fieldnames=list(rows[0]))
        writer.writeheader()
        writer.writerows(rows)


def _run_all(args, out, scratch, rows):
    for arm in args.arms:
        (out / f"settings-{arm}.json").write_text(json.dumps(settings(arm, args.node, args.host)), encoding="utf-8")
    for rep in range(args.reps):
        for name, prompt in PROMPTS.items():
            for arm in args.arms:
                agent = scratch / f"agent-{arm}"
                agent.mkdir(exist_ok=True)
                (agent / "settings.json").write_text(json.dumps(settings(arm, args.node, args.host)), encoding="utf-8")
                # Copied fresh each run; never printed, never left under --out.
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


@contextlib.contextmanager
def tree_at(repo, sha):
    """A clean checkout of `sha`, so the facts are those of the tree the runs saw."""
    scratch = Path(tempfile.mkdtemp(prefix="codemode-truth-"))
    tree = scratch / "tree"
    added = subprocess.run(["git", "worktree", "add", "--detach", str(tree), sha], cwd=repo,
                           capture_output=True, text=True, encoding="utf-8", errors="replace")
    if added.returncode != 0:
        shutil.rmtree(scratch, ignore_errors=True)
        sys.exit(f"cannot check out recorded commit {sha[:10]} to grade against: {added.stderr.strip()[:200]}")
    try:
        yield tree
    finally:
        subprocess.run(["git", "worktree", "remove", "--force", str(tree)], cwd=repo, capture_output=True)
        shutil.rmtree(scratch, ignore_errors=True)


def ground_truth(out, repo):
    """Facts from the commit the runs were made at (HEAD when none is recorded)."""
    recorded = out / "commit.txt"
    sha = recorded.read_text(encoding="utf-8").strip() if recorded.exists() else ""
    if sha and sha != _git(repo, "rev-parse", "HEAD").strip():
        with tree_at(repo, sha) as tree:
            return truth(tree)
    return truth(repo)


def report(args):
    out = Path(args.out)
    t = ground_truth(out, args.repo)
    exits = {}
    if (out / "runs.csv").exists():
        with open(out / "runs.csv", newline="", encoding="utf-8") as f:
            exits = {(r["arm"], r["prompt"], r["rep"]): r["exit"] for r in csv.DictReader(f)}
    elif getattr(args, "allow_unverified_exits", False):
        print("warning: no runs.csv; process exit codes are UNVERIFIED (--allow-unverified-exits)")
    else:
        sys.exit("no runs.csv: process exit codes cannot be verified. Pass --allow-unverified-exits to grade anyway.")
    arms = collections.defaultdict(
        lambda: {"crashed": 0, "runs": 0, "correct": 0, "facts": 0, "facts_ok": 0, "adopted": 0, "tools": 0, "failures": []})
    for run_dir in sorted((out / "runs").iterdir()):
        arm, prompt = run_dir.name.split("-", 1)[0], re.sub(r"-r\d+$", "", run_dir.name.split("-", 1)[1])
        answer, tools = read_run(run_dir / "out.jsonl")
        miss, total = grade(prompt, answer, t)
        rep_no = re.search(r"-r(\d+)$", run_dir.name)
        status = exits.get((arm, prompt, rep_no.group(1) if rep_no else ""), "0" if not exits else "missing")
        a = arms[arm]
        # A run that did not exit cleanly fails strictly whatever its output says,
        # but never costs the answer's own facts: those stay a measure of the answer.
        crashed = status != "0"
        a["crashed"] += crashed
        a["runs"] += 1
        a["correct"] += not miss and not crashed
        a["facts"] += total
        a["facts_ok"] += total - len(miss)
        a["adopted"] += tools.get("codemode", 0) > 0
        a["tools"] += sum(tools.values())
        if miss or crashed:
            reasons = ([f"process exit {status}"] if crashed else []) + miss
            a["failures"].append(f"{run_dir.name}: {'; '.join(reasons)[:160]}")
    for arm, a in sorted(arms.items()):
        print(f"{arm:4} runs={a['runs']} crashed={a['crashed']} strict_correct={a['correct']}/{a['runs']} "
              f"facts={a['facts_ok']}/{a['facts']} using_codemode={a['adopted']}/{a['runs']} tool_calls={a['tools']}")
        for failure in a["failures"]:
            print("   ", failure)
    on, off = arms.get("on"), arms.get("off")
    if not on or not on["runs"]:
        sys.exit("no Codemode-on runs to judge")
    adoption, correct = on["adopted"] / on["runs"], on["correct"] / on["runs"]
    print(f"adoption {adoption:.0%} (min {MIN_ADOPTION:.0%}), strict correct {correct:.0%} (min {MIN_CORRECT:.0%})")
    ok = adoption >= MIN_ADOPTION and correct >= MIN_CORRECT
    if off and off["runs"]:
        # Codemode must not make answers worse than the same questions without it.
        baseline = off["correct"] / off["runs"]
        facts_on, facts_off = on["facts_ok"] / on["facts"], off["facts_ok"] / off["facts"]
        print(f"baseline strict correct {baseline:.0%}, facts on {facts_on:.0%} vs off {facts_off:.0%}")
        ok = ok and correct >= baseline
    return ok


def default_node():
    """The Node runtime Codemode installs: node.exe on Windows, bin/node elsewhere."""
    base = Path.home() / ".davinci/agent/codemode/node"
    return base / "node.exe" if os.name == "nt" else base / "bin" / "node"


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
    live.add_argument("--node", default=default_node())
    live.add_argument("--host", default=Path.home() / ".davinci/agent/codemode/host")
    rep = sub.add_parser("report", help="offline grading of a finished run directory")
    rep.add_argument("--out", required=True)
    rep.add_argument("--repo", required=True)
    rep.add_argument("--allow-unverified-exits", action="store_true",
                     help="grade a legacy run directory that has no runs.csv")
    args = parser.parse_args(argv)
    if args.command == "run":
        run(args)
        return 0
    return 0 if report(args) else 1


if __name__ == "__main__":
    sys.exit(main())
