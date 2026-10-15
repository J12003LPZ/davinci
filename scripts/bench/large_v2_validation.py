"""Offline acceptance checks for the v2 large-task fixtures.

    python scripts/bench/large_v2_validation.py

Builds each v2 task's starter, reference, alternative and deliberately broken
implementations in a temporary directory and runs the hidden tests against
them with pytest. No model, campaign or generated `tasks/` tree is used.
Exit status 0 means every implementation failed exactly the hidden tests
`make_large_tasks_v2.VALIDATION` expects, and the reference also passes its
public tests.
"""
import json
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import textwrap

import make_large_tasks_v2 as v2

FAILED = re.compile(r"^(?:FAILED|ERROR) (\S+?\.py)(?:::(.+?))?(?: - .*)?$")


def materialize(directory, files):
    for relative, text in files.items():
        path = directory / relative
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(textwrap.dedent(text).lstrip("\n"), encoding="utf-8", newline="\n")


def implementation(spec, variant=None):
    """Files of the reference, or of the reference changed by one variant."""
    files = {**spec["repo"], **spec["solution"]}
    files = {k: textwrap.dedent(v).lstrip("\n") for k, v in files.items()}
    for relative, text in (variant or {}).get("files", {}).items():
        files[relative] = textwrap.dedent(text).lstrip("\n")
    for relative, old, new in (variant or {}).get("patch", []):
        if files[relative].count(old) != 1:
            raise ValueError(f"variant patch does not match exactly once in {relative}: {old[:60]!r}")
        files[relative] = files[relative].replace(old, new)
    return files


def failures(files, hidden, tests=None, timeout=300):
    """Failing hidden test node names (without file) for one implementation."""
    with tempfile.TemporaryDirectory() as temporary:
        root = Path(temporary)
        materialize(root, {**files, **hidden})
        names = tests or sorted(hidden)
        result = subprocess.run([sys.executable, "-m", "pytest", "-q", "-rfE", "-p", "no:cacheprovider", *names],
                                cwd=root, capture_output=True, text=True, timeout=timeout)
    found = set()
    for line in result.stdout.splitlines():
        match = FAILED.match(line.rstrip())
        if match:
            found.add(match.group(2) or "<collection>")
    if result.returncode not in (0, 1) and not found:
        found.add(f"<pytest exit {result.returncode}>")
    return sorted(found)


def validate(only=None):
    report, errors = {}, []
    for spec in v2.TASK_SPECS:
        task = spec["id"]
        if only and task not in only:
            continue
        expected = v2.VALIDATION[task]
        outcome = {}

        def check(label, files, want):
            got = failures(files, spec["hidden"])
            outcome[label] = got
            if got != sorted(want):
                errors.append({"task": task, "implementation": label, "expected": sorted(want), "got": got})

        check("starter", spec["repo"], expected["starter_fails"])
        check("reference", implementation(spec), [])
        public = failures(implementation(spec), {}, tests=[spec["public_verification"].split()[-1]])
        outcome["reference public tests"] = public
        if public:
            errors.append({"task": task, "implementation": "reference public tests", "expected": [], "got": public})
        for name, variant in expected["alternatives"].items():
            check(f"alternative: {name}", implementation(spec, variant), [])
        for name, (variant, want) in expected["broken"].items():
            check(f"broken: {name}", implementation(spec, variant), want)
        report[task] = outcome
    return report, errors


def main():
    report, errors = validate(sys.argv[1:] or None)
    print(json.dumps({"report": report, "errors": errors}, indent=2))
    raise SystemExit(1 if errors else 0)


if __name__ == "__main__":
    main()
