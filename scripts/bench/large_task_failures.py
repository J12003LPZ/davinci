"""Classify recorded large-task runs: which requirements failed, and how each run ended.

    python scripts/bench/large_task_failures.py <campaign-dir> [<campaign-dir> ...] --out <dir>

Offline and read-only. For each campaign directory it reads `results.jsonl`,
`<harness>/<task>-r<rep>.stdout.jsonl` and, when the campaign recorded it,
`<harness>/<task>-r<rep>.grader.json` (the hidden grader's pytest output). It
makes no model calls, runs no grader or test, and writes only to `--out`,
which must lie outside every campaign directory. A directory without
`results.jsonl` stands for its immediate variant subdirectories that have one.
A selected campaign with no large-task rows is an error unless `--allow-empty`.

The output is sanitized: run identity, requirement IDs, event codes, counts and
repository-relative changed paths. Prompts, model text, command lines, command
output, assertion messages, file contents and absolute paths are never copied;
identifiers outside a conservative character set become "redacted", and changed
paths that are absolute, UNC, drive-relative, contain `..` or unusual characters
are withheld and counted under `unknowns`.

Each hidden test is categorized (`REQUIREMENTS`, `CASES`) as specified, mixed,
underspecified or contradictory. `failure_basis` says what a run's failed tests
can establish by themselves: only a specified failure is evidence against the
agent; mixed and unmapped failures leave the cause indeterminate. Results are
grouped by model, then harness, and never pooled across models.

Each run gets three kinds of field, kept apart on purpose:
  observed   read directly from the logs (edits, checks and their exit status,
             reminders, failed hidden tests);
  hypothesis one label from a fixed rule over the observed fields; it names a
             likely cause, it does not prove one;
  unknowns   evidence that was missing or unreadable for this run.
Exploration quality and the model's reasoning are not visible in event codes;
the summary says so rather than guessing.
"""
import argparse
import csv
import json
import os
from pathlib import Path
import re
import sys

from make_large_tasks_v2 import REQUIREMENTS as V2_REQUIREMENTS
from turn_cache_audit import EDITORS, SHELLS, events, leaves

# Hidden grader test -> (requirement, category, what it checks). Categories say
# what a failure of that test can establish, judged against the prompt, the
# starter code and its public test (scripts/bench/make_large_tasks.py) and
# confirmed by running variants of the reference solution against the hidden
# tests (docs/perf/harness-gap/large-task-contracts.md):
#   specified       every assertion follows from the prompt or starter behavior;
#                   a failure is a failure to do what was asked.
#   mixed           specified assertions share the test with unstated behavior
#                   or an unstated mechanism; a failure alone cannot say which.
#   underspecified  the assertions check only behavior the prompt leaves open.
#   contradictory   no solution satisfies the prompt, the starter public test and
#                   this test together.
# Only "specified" failures are evidence about the agent by themselves. The
# others are not false failures either: they are not attributable either way.
REQUIREMENTS = {
    "m1-config-parser": {
        "test_whitespace_comments_bom_and_duplicate_last_wins":
            ("m1.bom_trim_duplicates", "mixed", "trim, comments, last duplicate wins (stated); BOM (not stated)"),
        "test_invalid_syntax": ("m1.reject_malformed", "specified", "malformed or empty keys raise ValueError"),
        "test_empty_input_and_precedence_do_not_mutate_callers":
            ("m1.precedence_no_mutation", "specified", "override order; caller mappings not mutated"),
        "test_values_keep_equals_after_first_separator":
            ("m1.equals_in_value", "specified", "starter behavior: split on the first '=' only"),
    },
    "m2-public-api-migration": {
        "test_new_signature_and_compatibility_shim":
            ("m2.signature_and_shim", "mixed",
             "signature, shim, no package re-export (stated); trimming, name-only uppercase (not stated)"),
        "test_all_callers_use_new_behavior": ("m2.callers_migrated", "specified", "every internal caller works"),
        "test_validation_errors_propagate":
            ("m2.blank_name_rejected", "underspecified", "whitespace-only name rejected; the starter accepts it"),
        "test_no_stale_internal_imports_or_exports":
            ("m2.no_stale_imports", "mixed",
             "no stale internal imports, __all__ (stated); also bans tests importing the promised shim"),
    },
    "m3-state-persistence": {
        "test_idempotence_scope_and_ordering":
            ("m3.idempotence_scope_order", "specified", "idempotent per scope; event order kept"),
        "test_restart_and_caller_immutability":
            ("m3.restart_no_mutation", "specified", "restart reloads state; caller dict not mutated"),
        "test_failed_replace_rolls_back":
            ("m3.atomic_rollback", "mixed",
             "prior file intact when replace fails (stated); json_store must expose `os` (not stated)"),
        "test_invalid_inputs": ("m3.invalid_ids", "specified", "empty scope or request id rejected"),
    },
    "m4-cli-feature": {
        "test_report_json_and_name_order": ("m4.json_name_sort", "specified", "JSON output; name sort"),
        "test_total_sort_and_tax": ("m4.total_sort_tax", "specified", "descending total sort; INVENTORY_TAX"),
        "test_invalid_inputs_exit_two":
            ("m4.invalid_exit_two", "mixed", "invalid input exits 2 (stated); via SystemExit from main() (not stated)"),
        "test_legacy_total_still_works":
            ("m4.legacy_total", "contradictory", "starter and public test add arguments; hidden multiplies pairs"),
    },
}
# v2 (make_large_tasks_v2.py) traces every hidden test to a prompt clause or
# starter behavior and was validated offline, so all of its tests are specified.
REQUIREMENTS.update({task: {test: (requirement, "specified", source) for test, (requirement, source) in tests.items()}
                     for task, tests in V2_REQUIREMENTS.items()})
# One case of a parametrized test that differs from the rest of it.
CASES = {
    ("m3-state-persistence", "test_invalid_inputs", "a-r-delta2"):
        ("m3.empty_delta", "underspecified", "empty delta rejected rather than a no-op"),
}
CATEGORIES = ("specified", "mixed", "underspecified", "contradictory", "unknown")
SAFE_TOKEN = re.compile(r"[A-Za-z0-9][A-Za-z0-9._+-]{0,79}")
# Leading dots are allowed (.github); an all-dot part or a trailing dot is not,
# because Windows strips trailing dots and "..." would no longer name what it says.
SAFE_PART = re.compile(r"(?!\.+$)[A-Za-z0-9_.][A-Za-z0-9_.+-]{0,99}(?<!\.)")
PYTHON = r"\bpython[\d.]*(?:\.exe)?"
ADHOC = re.compile(PYTHON + r"\s+-c\b|" + PYTHON + r"\s+-\s*<<")
# A test runner, not a mention of a test file (`cat tests/test_x.py` is a read).
RUNNER = re.compile(r"\b(?:pytest|unittest)\b|" + PYTHON + r"\s+\S*test_[\w.-]+\.py\b|\b(?:cargo|go|npm|pnpm)\s+test\b")
COMPILE = re.compile(r"\b(?:compileall|py_compile)\b")
SHELL_WRITE = re.compile(r"(?<![<>&\d])>{1,2}\s*(?!/dev/null|NUL\b)[\w./\\-]|\bsed\s+-i\b|\btee\b"
                         r"|\b(?:mv|cp|rm|Set-Content|Out-File)\b")
FAILED_LINE = re.compile(r"^(FAILED|ERROR) (.+?)(?: - .*)?$")
PATCH_PATH = re.compile(r"^\*\*\* (?:Add|Update|Delete) File: (.+)$", re.M)


def safe_token(value):
    """A short identifier from a log or row, or "redacted". Never free text."""
    if isinstance(value, bool) or value is None:
        return value
    if isinstance(value, int):
        return value
    text = str(value)
    return text if SAFE_TOKEN.fullmatch(text) else "redacted"


def safe_relative_path(value):
    """A repository-relative path in "/" form, or None when it could leak or escape.

    Rejects absolute Windows, Unix and UNC paths, drive-relative and `~` forms,
    any `..` or all-dot segment, a trailing dot, URLs and alternate data
    streams (`:`), control characters, and segments outside a conservative
    character set.
    """
    if not isinstance(value, str) or not value or len(value) > 260:
        return None
    text = value.replace("\\", "/")
    if text.startswith(("/", "~")) or ":" in text or any(ord(c) < 32 or ord(c) == 127 for c in text):
        return None
    parts = [part for part in text.split("/") if part not in ("", ".")]
    if not parts or any(part == ".." or not SAFE_PART.fullmatch(part) for part in parts):
        return None
    return "/".join(parts)


def is_test_path(path):
    name = path.replace("\\", "/").rsplit("/", 1)[-1]
    return name.startswith("test_") or name.endswith("_test.py") or "/tests/" in "/" + path.replace("\\", "/")


def public_test_file(manifest, task):
    command = (manifest.get(task) or {}).get("public_verification") or ""
    found = re.search(r"\S*test\S*\.py", command)
    return found.group(0).replace("\\", "/") if found else None


def check_kind(command, public_file):
    """None for a command that is not a check, else what it checks."""
    text = command.replace("\\", "/")
    if ADHOC.search(text):
        return "adhoc"
    if not RUNNER.search(text):
        return "compile" if COMPILE.search(text) else None
    if public_file and public_file in text:
        return "public"
    if re.search(r"\bpytest\b", text) and not re.search(r"\S+\.py\b|\s-k\s", text.split("pytest", 1)[1]):
        return "suite"  # the whole suite: public tests plus any the agent added
    return "targeted"


def edit_paths(name, args):
    if name == "apply_patch":
        return PATCH_PATH.findall(str(args.get("input") or args.get("patch") or ""))
    path = args.get("path") or args.get("file_path")
    return [path] if isinstance(path, str) else []


class Timeline:
    """Ordered, sanitized steps: ("edit", is_test) | ("check", kind, status) | ("reminder", reason) | ...."""

    def __init__(self):
        self.steps, self.shell_writes, self.unknown_check_status = [], 0, 0

    def edit(self, path):
        self.steps.append(("edit", is_test_path(path)))

    def shell(self, command, public_file):
        kind = check_kind(command, public_file)
        if kind is None:
            if SHELL_WRITE.search(command):
                self.shell_writes += 1
            return None
        self.steps.append(["check", kind, "unknown"])
        return self.steps[-1]


def davinci_timeline(path, public_file):
    timeline, pending, stats, final = Timeline(), {}, {}, False
    for event in events(path):
        kind = event.get("type")
        if kind == "tool_execution_start":
            call = event.get("toolCallId")
            checks = []
            for name, args in leaves(event.get("toolName"), event.get("args")):
                if name in EDITORS:
                    for edited in edit_paths(name, args):
                        timeline.edit(edited)
                    checks.append(None)
                elif name in SHELLS:
                    checks.append(timeline.shell(str(args.get("command", "")), public_file))
                else:
                    checks.append(None)
            if isinstance(call, str):
                pending[call] = checks
        elif kind == "tool_execution_end":
            checks = pending.pop(event.get("toolCallId"), None) if isinstance(event.get("toolCallId"), str) else None
            if not checks:
                continue
            details = event.get("details") if isinstance(event.get("details"), dict) else {}
            operations = details.get("operations") if details.get("batch") is True else None
            for index, step in enumerate(checks):
                if step is None:
                    continue
                if isinstance(operations, list):
                    child = operations[index] if index < len(operations) else None
                    status = child.get("status") if isinstance(child, dict) else None
                    step[2] = {"ok": "pass", "error": "fail"}.get(status, "unknown")
                elif type(event.get("isError")) is bool:
                    step[2] = "fail" if event["isError"] else "pass"
        elif kind == "completion_reminder":
            reason = event.get("reason_code")
            timeline.steps.append(("reminder", safe_token(reason) if isinstance(reason, str) and reason else "unknown"))
        elif kind == "message_end":
            message = event.get("message") if isinstance(event.get("message"), dict) else {}
            reason = message.get("davinciCapabilityReminder")
            if isinstance(reason, str) and reason:
                timeline.steps.append(("reminder", "capability"))
        elif kind == "verification_notice":
            timeline.steps.append(("notice", safe_token(event.get("status") or "unknown")))
        elif kind == "harness_stats":
            stats = event.get("runtime") if isinstance(event.get("runtime"), dict) else {}
        elif kind == "agent_end":
            final = True
    timeline.unknown_check_status = sum(1 for s in timeline.steps if s[0] == "check" and s[2] == "unknown")
    return timeline, stats, final


def codex_timeline(path, public_file):
    timeline, final = Timeline(), False
    for event in events(path):
        item = event.get("item") if isinstance(event.get("item"), dict) else {}
        if event.get("type") == "item.completed" and item.get("type") == "command_execution":
            step = timeline.shell(str(item.get("command", "")), public_file)
            code = item.get("exit_code")
            if step is not None and type(code) is int:
                step[2] = "pass" if code == 0 else "fail"
        elif event.get("type") == "item.completed" and item.get("type") == "file_change":
            for change in item.get("changes") or []:
                if isinstance(change, dict) and isinstance(change.get("path"), str):
                    timeline.edit(change["path"])
        elif event.get("type") == "turn.completed":
            final = True
    timeline.unknown_check_status = sum(1 for s in timeline.steps if s[0] == "check" and s[2] == "unknown")
    return timeline, None, final


def failed_tests(grader_path):
    """(status, [(test function, case id)]) from the stored hidden-grader output."""
    try:
        record = json.loads(grader_path.read_text(encoding="utf-8"))
    except (OSError, ValueError):
        return "unavailable", []
    output = "\n".join(str(record.get(key) or "") for key in ("stdout", "stderr"))
    names = []
    for line in output.splitlines():
        found = FAILED_LINE.match(line.rstrip())
        if not found:
            continue
        node = found.group(2)
        if "::" not in node:
            names.append(("<collection>", None))
            continue
        name = node.split("::", 1)[1]
        case = name[name.index("[") + 1:name.rindex("]")] if "[" in name and name.endswith("]") else None
        names.append((name.split("[", 1)[0], case))
    return "parsed", names


def token(step):
    if step[0] == "edit":
        return "T" if step[1] else "E"
    if step[0] == "check":
        return {"public": "P", "suite": "S", "targeted": "X", "adhoc": "A", "compile": "C"}[step[1]] + \
            {"pass": "+", "fail": "-", "unknown": "?"}[step[2]]
    return {"reminder": "R", "notice": "N"}[step[0]]


def observe(timeline, stats, final):
    steps = timeline.steps
    edits = [i for i, s in enumerate(steps) if s[0] == "edit"]
    last_edit = edits[-1] if edits else -1
    # A compile check shows the code imports, not that it behaves.
    after_edit = [s for s in steps[last_edit + 1:] if s[0] == "check" and s[1] != "compile"]
    last = after_edit[-1] if after_edit else None
    reminders = [i for i, s in enumerate(steps) if s[0] == "reminder"]
    tail = steps[reminders[0] + 1:] if reminders else []
    checks = [s for s in steps if s[0] == "check"]
    count = lambda rows, test: sum(1 for s in rows if test(s))
    return {
        "timeline": " ".join(token(s) for s in steps) + (" F" if final else ""),
        "final_answer": final,
        "edits": len(edits),
        "test_edits": count(steps, lambda s: s[0] == "edit" and s[1]),
        "checks": len(checks),
        "check_passes": count(checks, lambda s: s[2] == "pass"),
        "check_failures": count(checks, lambda s: s[2] == "fail"),
        "adhoc_checks": count(checks, lambda s: s[1] == "adhoc"),
        "checks_after_last_edit": len(after_edit),
        "last_check_after_last_edit": f"{last[1]}:{last[2]}" if last else "none",
        "reminders": len(reminders),
        "reminder_reasons": ";".join(sorted({s[1] for s in steps if s[0] == "reminder"})),
        "requirement_reviews_skipped": (stats or {}).get("completionRequirementReviewsSkipped") if stats is not None
            else "n/a",
        "verification_notices": ";".join(s[1] for s in steps if s[0] == "notice"),
        "edits_after_first_reminder": count(tail, lambda s: s[0] == "edit") if reminders else None,
        "test_edits_after_first_reminder": count(tail, lambda s: s[0] == "edit" and s[1]) if reminders else None,
        "checks_after_first_reminder": count(tail, lambda s: s[0] == "check") if reminders else None,
        "check_passes_after_first_reminder": count(tail, lambda s: s[0] == "check" and s[2] == "pass")
            if reminders else None,
    }


def hypothesis(row, seen):
    """One label from a fixed rule. A likely cause, not a proven one."""
    if row.get("pass"):
        return "passed"
    if row.get("stop_reason"):
        return "not_graded"
    if not row.get("changed"):
        return "no_change"
    last = seen["last_check_after_last_edit"]
    if last == "none":
        return "finished_unverified"
    if last.endswith(":fail"):
        return "finished_with_failing_check"
    if last.endswith(":unknown"):
        return "last_check_outcome_unknown"
    if seen["test_edits"] == 0 and seen["adhoc_checks"] == 0:
        return "stopped_on_existing_checks"
    return "own_checks_passed_hidden_failed"


def failure_basis(requirements, status):
    """What the failed hidden tests can establish about the agent, by themselves."""
    if status != "parsed" or not requirements:
        return "unknown"
    categories = {category for _, category in requirements}
    if "specified" in categories:
        return "specified_failure"
    if categories & {"mixed", "unknown"}:
        return "cause_indeterminate"
    return "only_underspecified_or_contradictory"


def classify(campaign, label, row, manifest):
    harness, task, rep = (safe_token(row.get(key)) for key in ("harness", "task", "rep"))
    # These name the log files, so an unsafe value must not reach a path.
    readable = "redacted" not in (harness, task, rep) and None not in (harness, task, rep)
    base = campaign / str(harness) / f"{task}-r{rep}"
    public_file = public_test_file(manifest, task)
    unknowns = []
    log = Path(str(base) + ".stdout.jsonl")
    if not readable:
        timeline, stats, final = Timeline(), {}, False
        unknowns.append("unsafe_run_identity")
    elif not log.is_file():
        timeline, stats, final = Timeline(), {}, False
        unknowns.append("stdout_log_missing")
    elif harness == "codex":
        timeline, stats, final = codex_timeline(log, public_file)
    else:
        timeline, stats, final = davinci_timeline(log, public_file)
    if timeline.unknown_check_status:
        unknowns.append(f"check_status_unknown:{timeline.unknown_check_status}")
    if timeline.shell_writes:
        unknowns.append(f"shell_may_have_written:{timeline.shell_writes}")
    seen = observe(timeline, stats, final)
    mapping = REQUIREMENTS.get(task, {})
    failed_status, names = failed_tests(Path(str(base) + ".grader.json")) if readable else ("unavailable", [])
    if failed_status == "unavailable" and not row.get("pass"):
        unknowns.append("grader_log_missing")
    elif failed_status == "parsed" and not names and row.get("tests_failed"):
        failed_status = "unparsed"
        unknowns.append("grader_failures_unparsed")
    requirements = []
    for name, case in names:
        if name == "<collection>":
            found = (f"{task}.grader_collection", "unknown")
        elif (task, name, case) in CASES:
            found = CASES[(task, name, case)][:2]
        elif name in mapping:
            found = mapping[name][:2]
        else:
            found = (f"unmapped:{safe_token(name)}", "unknown")
        if found not in requirements:
            requirements.append(found)
    if failed_status == "unavailable" or failed_status == "unparsed":
        requirements = []
    changed = row.get("changed") if isinstance(row.get("changed"), list) else []
    paths = [safe_relative_path(p) for p in changed]
    if None in paths:
        unknowns.append(f"changed_paths_withheld:{paths.count(None)}")
    paths = sorted({p for p in paths if p})
    numeric = lambda value: value if isinstance(value, (int, float)) and not isinstance(value, bool) else None
    flag = lambda value: value if isinstance(value, bool) else None
    unrelated = row.get("unrelated")
    return {
        "campaign": label, "variant": safe_token(row.get("variant")), "model": safe_token(row.get("model")),
        "harness": harness, "task": task, "rep": rep,
        "binary_sha256": row.get("binary_sha256") if re.fullmatch(r"[0-9a-f]{64}", str(row.get("binary_sha256")))
            else None,
        "pass": flag(row.get("pass")), "grader_pass": flag(row.get("grader_pass")),
        "tests_passed": numeric(row.get("tests_passed")), "tests_failed": numeric(row.get("tests_failed")),
        "stop_reason": safe_token(row.get("stop_reason")) if row.get("stop_reason") else None,
        "exit": numeric(row.get("exit")),
        "failed_requirements_source": failed_status,
        "failed_requirements": ";".join(r for r, _ in requirements),
        "failed_requirement_categories": ";".join(c for _, c in requirements),
        **{f"failed_{category}": sum(1 for _, c in requirements if c == category) for category in CATEGORIES},
        "failure_basis": "passed" if row.get("pass") else failure_basis(requirements, failed_status),
        "changed_files": ";".join(paths),
        "public_test_changed": public_file in changed if public_file else None,
        "unrelated_changes": len(unrelated) if isinstance(unrelated, list) else None,
        **seen,
        "hypothesis": hypothesis(row, seen),
        "unknowns": ";".join(unknowns),
    }


def summarize(rows):
    def tally(values):
        result = {}
        for value in values:
            result[value] = result.get(value, 0) + 1
        return dict(sorted(result.items()))

    categories = {req: category for task in REQUIREMENTS.values() for req, category, _ in task.values()}
    categories.update({req: category for req, category, _ in CASES.values()})

    def group(mine):
        failed = [r for r in mine if r["pass"] is not True]
        reminded = [r for r in mine if r["reminders"]]
        return {
            "runs": len(mine), "passes": sum(1 for r in mine if r["pass"] is True),
            "failure_basis": tally(r["failure_basis"] for r in mine),
            "hypothesis": tally(r["hypothesis"] for r in mine),
            "failed_requirement_runs": {
                req: {"runs": n, "category": categories.get(req, "unknown")}
                for req, n in tally(req for r in failed for req in filter(None, r["failed_requirements"].split(";"))).items()},
            "failed_runs_by_requirement_source": tally(r["failed_requirements_source"] for r in failed),
            "runs_with_reminder": len(reminded),
            "reminded_runs_adding_test_edits_after": sum(1 for r in reminded if r["test_edits_after_first_reminder"]),
            "reminded_runs_passing": sum(1 for r in reminded if r["pass"] is True),
            "runs_with_review_skipped": sum(1 for r in mine if isinstance(r["requirement_reviews_skipped"], int)
                                            and r["requirement_reviews_skipped"] > 0),
            "runs_editing_tests": sum(1 for r in mine if r["test_edits"]),
            "runs_with_unknowns": sum(1 for r in mine if r["unknowns"]),
        }

    # Models are never pooled: a GPT-5.6 baseline and GPT-6 campaigns stay apart.
    per_model = {}
    for model in sorted({str(r["model"]) for r in rows}):
        of_model = [r for r in rows if str(r["model"]) == model]
        per_model[model] = {harness: group([r for r in of_model if str(r["harness"]) == harness])
                            for harness in sorted({str(r["harness"]) for r in of_model})}
    # Same task and repetition under different harnesses: where the failed
    # requirement sets differ, the trajectories diverged even if both failed.
    divergences = []
    keys = sorted({(r["campaign"], r["task"], r["rep"]) for r in rows}, key=str)
    for campaign, task, rep in keys:
        group = {str(r["harness"]): r for r in rows if (r["campaign"], r["task"], r["rep"]) == (campaign, task, rep)}
        if len(group) < 2:
            continue
        sets = {h: set(filter(None, r["failed_requirements"].split(";"))) for h, r in group.items()}
        if len({frozenset(s) for s in sets.values()}) > 1 or len({r["hypothesis"] for r in group.values()}) > 1:
            divergences.append({"campaign": campaign, "task": task, "rep": rep,
                                "failed_requirements": {h: sorted(s) for h, s in sets.items()},
                                "hypothesis": {h: r["hypothesis"] for h, r in group.items()}})
    return {
        "runs": len(rows), "per_model": per_model, "divergences": divergences,
        "requirement_categories": categories,
        "failure_basis_rules": {
            "specified_failure": "at least one failed hidden test checks only stated or starter behavior: "
                                 "the run did not do something it was asked to do",
            "cause_indeterminate": "no specified failure; a failed test mixes stated and unstated behavior, or is "
                                   "unmapped, so the grader output cannot say which part failed",
            "only_underspecified_or_contradictory": "every failure is on unstated or contradictory behavior: "
                                                    "not evidence against the agent, and not proof it was right",
            "unknown": "failed tests unavailable or unparsed",
        },
        "hypothesis_rules": {
            "stopped_on_existing_checks": "last check after the last edit passed; no test edits and no ad hoc checks",
            "own_checks_passed_hidden_failed": "last check after the last edit passed; the run edited tests or ran ad hoc checks",
            "finished_with_failing_check": "last check after the last edit failed",
            "finished_unverified": "no check after the last edit",
            "last_check_outcome_unknown": "last check after the last edit has no recorded exit status",
            "not_graded": "the run stopped (stop_reason) before or during grading",
            "no_change": "no changed files recorded",
        },
        "not_observable": [
            "exploration quality: which files were read is not weighed against what the task needed",
            "model reasoning and misunderstanding: no model text is analysed",
            "whether a passing agent-written test exercised the failed requirement",
        ],
    }


FIELDS = ("campaign", "variant", "model", "harness", "task", "rep", "pass", "grader_pass", "tests_passed",
          "tests_failed", "stop_reason", "exit", "failed_requirements_source", "failed_requirements",
          "failed_requirement_categories", "failure_basis", "failed_specified", "failed_mixed",
          "failed_underspecified", "failed_contradictory", "failed_unknown", "hypothesis", "unknowns", "timeline", "final_answer", "edits",
          "test_edits", "checks", "check_passes", "check_failures", "adhoc_checks", "checks_after_last_edit",
          "last_check_after_last_edit", "reminders", "reminder_reasons", "requirement_reviews_skipped",
          "verification_notices", "edits_after_first_reminder", "test_edits_after_first_reminder",
          "checks_after_first_reminder", "check_passes_after_first_reminder", "changed_files",
          "public_test_changed", "unrelated_changes", "binary_sha256")


def load_rows(campaign):
    """(large-task rows, total rows) from one results.jsonl."""
    rows, total = [], 0
    for line in (campaign / "results.jsonl").read_text(encoding="utf-8", errors="replace").splitlines():
        try:
            row = json.loads(line)
        except ValueError:
            continue
        if not isinstance(row, dict):
            continue
        total += 1
        if str(row.get("task", "")).startswith("m"):
            rows.append(row)
    return rows, total


def expand(campaigns):
    """(label, directory) for each campaign; a directory without results.jsonl
    stands for its immediate subdirectories that have one (campaign variants)."""
    found = []
    for campaign in campaigns:
        if (campaign / "results.jsonl").is_file():
            found.append((safe_token(campaign.resolve().name), campaign))
            continue
        variants = sorted(p for p in campaign.iterdir() if (p / "results.jsonl").is_file()) \
            if campaign.is_dir() else []
        if not variants:
            raise ValueError(f"no results.jsonl in {campaign.name} or its immediate subdirectories")
        found += [(f"{safe_token(campaign.resolve().name)}/{safe_token(p.name)}", p) for p in variants]
    return found


def run(campaigns, out, manifest_path, allow_empty=False):
    out = out.resolve()
    selected = expand(campaigns)
    for _, campaign in selected:
        if out == campaign.resolve() or campaign.resolve() in out.parents:
            raise ValueError("--out must be outside every campaign directory")
    if (out / "results.jsonl").exists():
        raise ValueError("--out looks like a campaign directory (it has results.jsonl)")
    manifest = {}
    for path in manifest_path if isinstance(manifest_path, (list, tuple)) else [manifest_path]:
        manifest.update(json.loads(Path(path).read_text(encoding="utf-8")).get("fixtures", {}))
    rows, empty = [], []
    for label, campaign in selected:
        large, total = load_rows(campaign)
        if not large:
            empty.append({"campaign": label, "rows": total})
        rows += [classify(campaign, label, row, manifest) for row in large]
    if empty and not allow_empty:
        raise ValueError("no large-task (m*) runs in: " + ", ".join(
            f"{e['campaign']} ({e['rows']} rows)" for e in empty) + "; pass --allow-empty to continue without them")
    summary = summarize(rows)
    summary["campaigns"] = [label for label, _ in selected]
    summary["campaigns_without_large_runs"] = empty
    out.mkdir(parents=True, exist_ok=True)
    with (out / "large-task-runs.csv").open("w", newline="", encoding="utf-8") as stream:
        writer = csv.DictWriter(stream, FIELDS, extrasaction="ignore")
        writer.writeheader()
        writer.writerows(rows)
    (out / "large-task-runs.json").write_text(json.dumps(rows, indent=2) + "\n", encoding="utf-8")
    (out / "large-task-summary.json").write_text(json.dumps(summary, indent=2) + "\n", encoding="utf-8")
    return rows, summary


# Never entered while locating: credentials, wire dumps, Git data, run workspaces.
LOCATE_SKIP = {"_agent", "wire", ".git", "davinci", "codex", "tasks", "node_modules"}


def locate(roots, depth=4):
    """Campaign directories under `roots` with large-task rows, one dict each.

    Reads only `results.jsonl` and lists `<harness>/` for `*.grader.json` names.
    """
    found = []
    for root in roots:
        if not Path(root).is_dir():
            continue
        for directory, subdirs, files in os.walk(root):
            level = len(Path(directory).relative_to(root).parts)
            subdirs[:] = [] if level >= depth else sorted(d for d in subdirs if d not in LOCATE_SKIP)
            if "results.jsonl" not in files:
                continue
            large, total = load_rows(Path(directory))
            if not large:
                continue
            values = lambda key: sorted({safe_token(r.get(key)) for r in large if r.get(key)} - {"redacted"})
            graders = sum(1 for harness in ("davinci", "codex") if (Path(directory) / harness).is_dir()
                          for name in os.listdir(Path(directory) / harness)
                          if name.startswith("m") and name.endswith(".grader.json"))
            started = sorted(str(r["started_at"]) for r in large if isinstance(r.get("started_at"), str))
            shas = sorted({str(r.get("source_sha"))[:12] for r in large
                           if re.fullmatch(r"[0-9a-f]{7,40}", str(r.get("source_sha")))})
            found.append({
                "path": directory, "rows": total,
                "large_runs": {h: sum(1 for r in large if safe_token(r.get("harness")) == h) for h in values("harness")},
                "large_passes": sum(1 for r in large if r.get("pass") is True),
                "models": values("model"), "source_sha": shas, "grader_logs": graders,
                "first_started": started[0] if started else None,
                "original_baseline_candidate": "gpt-5.6-luna" in values("model")
                                               or any(sha.startswith("e8a276ec") for sha in shas),
            })
    return sorted(found, key=lambda item: (item["first_started"] or "", item["path"]))


def main():
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("campaigns", type=Path, nargs="+",
                        help="campaign directories, or search roots with --locate")
    parser.add_argument("--out", type=Path)
    parser.add_argument("--locate", action="store_true",
                        help="list campaigns with large-task rows under the given roots; read-only, writes nothing")
    here = Path(__file__).resolve().parent
    parser.add_argument("--manifest", type=Path, action="append",
                        help="fixture manifest(s); default: large_manifest.json and large_manifest_v2.json")
    parser.add_argument("--allow-empty", action="store_true",
                        help="continue when a selected campaign has no large-task runs (listed in the summary)")
    args = parser.parse_args()
    if args.locate:
        for item in locate(args.campaigns):
            print(json.dumps(item))
        return
    if args.out is None:
        parser.error("--out is required unless --locate")
    try:
        manifests = args.manifest or [here / "large_manifest.json", here / "large_manifest_v2.json"]
        _, summary = run(args.campaigns, args.out, manifests, args.allow_empty)
    except ValueError as error:
        parser.error(str(error))
    for entry in summary["campaigns_without_large_runs"]:
        print(f"warning: no large-task runs in {entry['campaign']} ({entry['rows']} rows)", file=sys.stderr)
    print(json.dumps(summary, indent=2))


if __name__ == "__main__":
    main()
