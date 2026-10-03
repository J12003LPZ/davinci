#!/usr/bin/env python3
"""Run every design integration target explicitly, without provider requests."""
import argparse
import json
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
REQUIRED = {
    "design_acceptance", "design_baseline", "design_budget", "design_commands",
    "design_controller", "design_export", "design_generation", "design_handoff",
    "design_host", "design_interaction", "design_limits", "design_model",
    "design_profiles", "design_quality", "design_recovery", "design_runtime",
    "design_security", "design_store", "design_sync", "design_types",
}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--offline", action="store_true")
    parser.add_argument("--inventory-only", action="store_true")
    args = parser.parse_args()
    targets = sorted(path.stem for path in
                     (ROOT / "crates/davinci-coding-agent/tests").glob("design_*.rs"))
    missing = REQUIRED - set(targets)
    if missing:
        raise SystemExit("Missing design targets: " + ", ".join(sorted(missing)))
    print(json.dumps({"coding_agent_targets": targets,
                      "eval_targets": ["design_quality"],
                      "native_ignored_tests_included": False}), flush=True)
    if args.inventory_only:
        return
    common = ["cargo", "test", "--locked"] + (["--offline"] if args.offline else [])
    commands = [
        common + ["-p", "davinci-coding-agent"]
        + [item for target in targets for item in ["--test", target]],
        common + ["-p", "davinci-evals", "--test", "design_quality"],
    ]
    for command in commands:
        result = subprocess.run(command, cwd=ROOT, check=False)
        if result.returncode:
            raise SystemExit(result.returncode)


if __name__ == "__main__":
    main()
