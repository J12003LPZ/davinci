"""Mutation checks for holdout_h1.py: break one rule at a time and demand that the tests notice.

    python scripts/bench/mutation_check_holdout_h1.py

Each mutation rewrites one line of the evaluator in a scratch copy of the bench directory, runs
tests/test_holdout_h1.py there, and reports CAUGHT (tests fail) or SURVIVED (tests still pass).
The real file is never touched. Exit status 1 if any mutation survives.
"""
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile

HERE = Path(__file__).resolve().parent
TARGET = "holdout_h1.py"
# The fast modules only: the real-git suite tests would make forty runs take minutes.
TESTS = ["tests.test_holdout_h1", "tests.test_holdout_h1_suite.SuiteErrorTests"]

MUTATIONS = [
    ("success tolerance widened", "SUCCESS_TOLERANCE = 2 ", "SUCCESS_TOLERANCE = 3 "),
    ("task deficit guard widened", "MAX_TASK_PASS_DEFICIT = 1 ", "MAX_TASK_PASS_DEFICIT = 2 "),
    ("guard dropped from AHEAD", "if mine > base and guard:", "if mine > base:"),
    ("guard dropped from MATCHES", "if mine >= base - SUCCESS_TOLERANCE and guard:", "if mine >= base - SUCCESS_TOLERANCE:"),
    ("token bar loosened", "MAX_TOKENS_PER_SUCCESS_RATIO = 0.90", "MAX_TOKENS_PER_SUCCESS_RATIO = 1.50"),
    ("wall bar loosened", "MAX_MEDIAN_TASK_WALL_RATIO = 1.00", "MAX_MEDIAN_TASK_WALL_RATIO = 1.50"),
    ("faster-tasks bar removed", "MIN_TASKS_FASTER = TASK_COUNT // 2", "MIN_TASKS_FASTER = 0"),
    ("safety lag loosened", "SAFETY_STOP_LAG = 2 ", "SAFETY_STOP_LAG = 3 "),
    ("total unknown attempts ignored", "if unknown_so_far + unknown > MAX_UNKNOWN_ATTEMPTS_TOTAL:", "if False:"),
    ("per-run unknown attempts ignored", "if unknown > MAX_UNKNOWN_ATTEMPTS_RUN:", "if False:"),
    ("websocket usage check removed", 'if arm == "B" and (row.get("input_tokens")', 'if arm == "Z" and (row.get("input_tokens")'),
    ("codex telemetry check removed", 'if arm == "C" and row.get("request_metrics_complete") is not True:',
     'if arm == "Z" and row.get("request_metrics_complete") is not True:'),
    ("codex usage completeness ignored", 'return row.get("request_metrics_complete") is True', "return True"),
    ("output tokens may be missing", 'if row.get("output_tokens") is None:\n        return', "if False:\n        return"),
    ("failed attempts leave the denominator", "_tokens(ms) / wins", '_tokens([m for m in ms if m["pass"]]) / wins'),
    ("reasoning accounting ignored", 'if row.get("reasoning_included_in_output") is not True:', "if False:"),
    ("registered pins ignored", "if pins.get(field) != pin:", "if False:"),
    ("a and b settings may differ", "elif rest != base_settings:", "elif False:"),
    ("suite digest ignored", '("private_suite_digest", pins["private_suite_digest"]), ', ""),
    ("binary pin ignored", 'if row.get("binary_sha256") != binary:', "if False:"),
    ("results readable before the leak scan", "if leak is None:", "if False:"),
    ("fallback with a failed run acceptable", 'or not e["passed"] for e in events', "for e in events"),
    ("zero fallbacks bound misstated", "3 / b_runs", "1 / b_runs"),
    ("overlap in one direction only", "if _inside(suite, directory) or _inside(directory, suite):", "if _inside(suite, directory):"),
    ("generic short lines become needles", "if len(line) >= LEAK_MIN_LINE and", "if True and"),
    ("prompt lines become needles", " and line not in prompt:", ":"),
    ("credential directory scanned", 'if "_agent" not in p.parts]', "]"),
    ("all campaigns run in the same order", "REP_ORDERS = ((\"A\", \"B\", \"C\"), (\"B\", \"C\", \"A\"), (\"C\", \"A\", \"B\"))",
     "REP_ORDERS = ((\"A\", \"B\", \"C\"), (\"A\", \"B\", \"C\"), (\"A\", \"B\", \"C\"))"),
    ("suite size unchecked", "or len(tasks) != TASK_COUNT or len(set(tasks)) != TASK_COUNT:", "or False:"),
    ("suite language unchecked", 'if str(label.get("language", "")).lower() != LANGUAGE:', "if False:"),
    ("suite split unchecked", 'if label.get("split") != SPLIT:', "if False:"),
    ("suite size class unchecked", 'if label.get("size_class") != "large":', "if False:"),
    ("shared repositories allowed", "if len(repositories) != TASK_COUNT:", "if False:"),
    ("task ids may be descriptive", "if not TASK_ID.fullmatch(str(tid)):", "if False:"),
    ("harness stop ignored", 'if row.get("stop_reason") is not None:', "if False:"),
    ("falsy stop reason ignored", 'if row.get("stop_reason") is not None:', 'if row.get("stop_reason"):'),
    ("failed run with unknown attempts tolerated", 'if row.get("pass") is not True and unknown > 0:', "if False:"),
    ("arm A may use websocket", 'elif arm == "A" and (set(labels) != {"http"} or labels["http"] < 1):', "elif False:"),
    ("arm B need not use websocket", ' or labels.get("websocket", 0) < 1):', "):"),
    ("leak ignored", 'if leak is not None and leak.get("status") != "CLEAN":', "if False:"),
    ("duplicated tool calls tolerated", 'integrity["duplicated"] or integrity["orphaned_ends"] or integrity["unfinished"]',
     'integrity["orphaned_ends"] or integrity["unfinished"]'),
    ("unfinished tool calls tolerated", 'integrity["orphaned_ends"] or integrity["unfinished"]\n',
     'integrity["orphaned_ends"]\n'),
    ("operation count mismatch tolerated", "              or not counts_agree):", "              or False):"),
    ("missing transcript treated as verified",
     'if integrity is None or not (_count(leaf) and _count(calls)) or integrity["starts"] != calls:', "if False:"),
    ("start count need not match tool calls", ' or integrity["starts"] != calls:', ":"),
    ("empty transcript counts as verified", "if not starts and not ends:\n        return None", "if False:\n        return None"),
    ("duplicate end events ignored", " + sum(1 for n in ends.values() if n > 1)", ""),
    ("json escapes not undone", "documents = [text] + text.splitlines()", "documents = []"),
    ("scan is case sensitive", "low = needle.lower()", "low = needle"),
    ("msys path form missed", "        variants.add(msys)", "        pass"),
    ("msys form keeps the drive colon", 'return "/%s/%s" % (match.group(1).lower(),', 'return "/%s:/%s" % (match.group(1).lower(),'),
    ("reference solution not scanned", 'for tree in ("hidden", "solution"):', 'for tree in ("hidden",):'),
    ("empty scan is clean", "elif not files or (expected_runs is not None and stdout_files < expected_runs):", "elif False:"),
    ("expected transcripts not counted", "stdout_files < expected_runs", "False"),
    ("wrong suite accepted", "if digest != expected_digest:", "if False:",
     "tests.test_holdout_h1_suite.RealReportTests.test_the_wrong_suite_blocks_the_numbers"),
    ("zero wall time accepted", "and math.isfinite(wall) and wall > 0", "and True"),
    ("missing unrelated list accepted", 'and isinstance(row.get("unrelated"), list))', "and True)"),
    ("cached above input accepted", ' or row["cached_tokens"] > row["input_tokens"]', ""),
    ("pass need not be boolean", 'if type(row.get("pass")) is not bool:', "if False:"),
    ("pass need not agree with its parts", 'elif row["pass"] and (row.get("exit") != 0', 'elif False and (row.get("exit") != 0'),
    ("unknown attempts need not be a count", "if not _count(unknown):", "if False:"),
    ("codex attempts counted for davinci", "if key[0] in DAVINCI_ARMS:\n            unknown_total", "if True:\n            unknown_total"),
    ("transport counts need not add up",
     'elif _count(row.get("provider_attempts")) and sum(labels.values()) != row["provider_attempts"]:', "elif False:"),
    ("transport counts need not be counts", "not all(_count(v) for v in labels.values())", "False"),
    ("no signal is a match", 'if base == 0 and mine == 0:\n        return "NO_SIGNAL"',
     'if False:\n        return "NO_SIGNAL"'),
    ("undefined efficiency is reported as a verdict", 'if c["tokens_per_success_ratio"] is None:\n            out["outcome"] =',
     'if False:\n            out["outcome"] ='),
    ("rows with odd types accepted", ' or type(row.get("rep")) is not int:', ":"),
    ("paths unquoted", "return \"'\" + str(value).replace(\"'\", \"''\") + \"'\"", "return str(value)"),
    ("stale binary variable left set", "$env:BENCH_CODEX = %s; Remove-Item Env:BENCH_DAVINCI -ErrorAction SilentlyContinue; ",
     "$env:BENCH_CODEX = %s; "),
    ("a subset of the split is launched", "--tasks all --reps 1", "--tasks first --reps 1"),
    ("gate needs no passes", 'default_gate(b_vs_a["passes"][1] >= b_vs_a["passes"][0], fallback["verdict"])',
     'default_gate(True, fallback["verdict"])'),
    ("gate ignores the fallback verdict", 'if not passes_ok or fallback == "NOT_ACCEPTABLE":', "if not passes_ok:"),
    ("unverified fallback passes the gate", 'return "UNVERIFIED" if fallback == "UNVERIFIED" else "PASS"', 'return "PASS"'),
    ("passes shown wrongly", '"passes_gate": b_vs_a["passes"][1] >= b_vs_a["passes"][0],', '"passes_gate": True,'),
    ("unpaired blocks count for the safety stop", "} & {(t, r) for (a, t, r) in rows if a == right},", "} | {(t, r) for (a, t, r) in rows if a == right},"),
]


def run(args, cwd):
    return subprocess.run(args, cwd=cwd, capture_output=True, text=True)


def main():
    survivors = []
    with tempfile.TemporaryDirectory() as scratch:
        shutil.copytree(HERE.parent, Path(scratch) / "scripts",
                        ignore=shutil.ignore_patterns("__pycache__", "tasks", "specialist_fixtures"))
        copy = Path(scratch) / "scripts" / "bench"
        docs = HERE.parents[1] / "docs" / "perf" / "harness-gap"
        shutil.copytree(docs, Path(scratch) / "docs" / "perf" / "harness-gap")  # the preregistration tests read it
        original = (copy / TARGET).read_text(encoding="utf-8")
        baseline = run([sys.executable, "-m", "unittest", *TESTS], copy)
        if baseline.returncode != 0:
            print("BASELINE FAILS, mutation results would mean nothing\n" + baseline.stderr[-2000:])
            return 2
        for name, old, new, *extra in MUTATIONS:
            if original.count(old) != 1:
                print("BAD MUTATION (target occurs %d times): %s" % (original.count(old), name))
                survivors.append(name)
                continue
            (copy / TARGET).write_text(original.replace(old, new, 1), encoding="utf-8", newline="\n")
            result = run([sys.executable, "-m", "unittest", *TESTS, *extra], copy)
            caught = result.returncode != 0
            print("%-8s %s" % ("CAUGHT" if caught else "SURVIVED", name), flush=True)
            if not caught:
                survivors.append(name)
        (copy / TARGET).write_text(original, encoding="utf-8", newline="\n")
    print("\n%d mutations, %d survived" % (len(MUTATIONS), len(survivors)))
    for name in survivors:
        print("  survivor:", name)
    return 1 if survivors else 0


if __name__ == "__main__":
    sys.exit(main())
