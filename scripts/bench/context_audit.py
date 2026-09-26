"""Audit public recorded tool calls using the real verification classifier.

No model calls, recorded command execution, or hidden grader input. Recognition
counts describe attempted command forms, not execution success or fresh evidence.
"""
import argparse
import json
import re
import subprocess
from pathlib import Path

SHELLS = {"exec_command", "bash", "shell", "powershell"}
CHECK_MARKER = re.compile(r"\b(assert|pytest|unittest|compileall|py_compile)\b|\b(cargo|go|npm|pnpm)\s+test\b|\btest_[\w.-]+\.py\b")
LISTING = re.compile(r"^(pwd|ls|dir|find|Get-Location|Get-ChildItem)(?:\s|$)", re.I)


def leaves(name, args):
    name = name.removeprefix("functions.")
    if name == "batch":
        for op in args.get("operations", []):
            yield from leaves(op.get("tool", ""), op.get("args", {}))
    else:
        yield name, args


def listing_only(name, args):
    if name in {"ls", "list_directory", "find"}:
        return True
    if name not in SHELLS:
        return False
    command = args.get("command", "").strip()
    # Conservative: uncertain control flow, substitutions, and redirection are
    # not called listing-only. This measures initial discovery, not all reads.
    if not command or any(c in command for c in "`$><|\n"):
        return False
    parts = re.split(r"\s*(?:&&|;)\s*", command)
    return all(LISTING.match(part) and not re.search(r"-(exec|delete)\b", part) for part in parts)


def calls(path):
    for line in path.read_text(encoding="utf-8").splitlines():
        try:
            event = json.loads(line)
        except ValueError:
            continue
        message = event.get("message", {})
        if event.get("type") != "message_end" or message.get("role") != "assistant":
            continue
        content = message.get("content", [])
        yield [leaf for block in content if block.get("type") == "toolCall"
               for leaf in leaves(block.get("name", ""), block.get("arguments", {}))]


def audit(campaign, binary):
    rows = [json.loads(line) for line in (campaign / "results.jsonl").read_text(encoding="utf-8").splitlines() if line]
    inputs, records = [], []
    listing_requests = 0
    runs = 0
    for row in rows:
        if row["harness"] != "davinci":
            continue
        runs += 1
        stem = f'{row["task"]}-r{row["rep"]}'
        cwd = campaign / "davinci" / stem
        stream = campaign / "davinci" / (stem + ".stdout.jsonl")
        for request, batch in enumerate(calls(stream)):
            if request == 0 and batch and all(listing_only(*leaf) for leaf in batch):
                listing_requests += 1
            for name, args in batch:
                command = args.get("command")
                if name not in SHELLS or not isinstance(command, str):
                    continue
                identity = f"{stem}:{len(inputs)}"
                effective_cwd = Path(args.get("workdir") or args.get("cwd") or cwd)
                if not effective_cwd.is_absolute():
                    effective_cwd = cwd / effective_cwd
                inputs.append(dict(id=identity, tool=name, command=command,
                                   cwd=str(effective_cwd), paths=[str(cwd / path) for path in row["changed"]]))
                records.append(dict(id=identity, task=row["task"], rep=row["rep"], request=request,
                                    tool=name, command=command, check_marker=bool(CHECK_MARKER.search(command))))
    process = subprocess.run([str(binary)], input="".join(json.dumps(i) + "\n" for i in inputs),
                             text=True, encoding="utf-8", capture_output=True, check=True)
    results = [json.loads(line) for line in process.stdout.splitlines()]
    if [r["id"] for r in records] != [r["id"] for r in results]:
        raise ValueError("classifier output identity mismatch")
    for record, result in zip(records, results):
        record.update(result)
    checks = [r for r in records if r["check_marker"]]
    recognized = lambda r: r["kind"] in {"suite", "targeted_script"}
    return dict(runs=runs, initial_listing_only_requests=listing_requests,
                shell_commands=len(records), recognized_shell_commands=sum(map(recognized, records)),
                check_marker_commands=len(checks), recognized_check_marker_commands=sum(map(recognized, checks)),
                denominator_note="Check markers are a reviewable syntactic proxy. Review commands for verification intent; recognition is not successful execution or complete coverage. Final changed paths are used, not historical mutation generations.",
                commands=records)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--campaign", type=Path, required=True)
    parser.add_argument("--classifier", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    result = audit(args.campaign.resolve(), args.classifier.resolve())
    args.output.write_text(json.dumps(result, indent=2), encoding="utf-8")
    print(json.dumps({k: v for k, v in result.items() if k != "commands"}))


if __name__ == "__main__":
    main()
