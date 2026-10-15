"""Offline checks for the large-task failure classifier. Synthetic fixtures only; no model calls."""
import hashlib
import json
from pathlib import Path
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
import large_task_failures as classifier  # noqa: E402

MANIFEST = Path(__file__).resolve().parents[1] / "large_manifest.json"
# Markers that must never reach the output: model text, command text and
# output, assertion messages, file contents and absolute paths.
SECRET = "SECRET-7f3a"


def start(call, name, **args):
    return {"type": "tool_execution_start", "toolCallId": call, "toolName": name, "args": args}


def end(call, error, **extra):
    return {"type": "tool_execution_end", "toolCallId": call, "isError": error, "result": f"{SECRET} output", **extra}


def edit(call, path):
    return [start(call, "edit", path=path, oldText=SECRET, newText=SECRET), end(call, False)]


def shell(call, command, error):
    return [start(call, "bash", command=command), end(call, error)]


def final(text=f"done {SECRET}"):
    return {"type": "agent_end", "messages": [{"role": "assistant", "content": [{"type": "text", "text": text}]}]}


def row(harness, task, rep=0, **extra):
    base = {"harness": harness, "task": task, "rep": rep, "pass": False, "grader_pass": False,
            "tests_passed": 2, "tests_failed": 2, "stop_reason": None, "exit": 0,
            "changed": ["config_parser.py"], "unrelated": [], "campaign": "c", "variant": "v",
            "model": "m", "task_set": "large", "pytest": f"2 failed {SECRET}"}
    base.update(extra)
    return base


def grader(*nodes):
    lines = [f"FAILED {node} - AssertionError: {SECRET}" for node in nodes]
    return {"schema_version": 1, "phase": "grader", "command": [SECRET], "cwd": f"C:/bench/{SECRET}",
            "stdout": "\n".join(["..F", *lines, f"{len(nodes)} failed, 2 passed in 0.1s"]), "stderr": ""}


class Campaign:
    def __init__(self, root):
        self.root = Path(root) / "campaign-01"
        self.root.mkdir()
        self.rows = []

    def add(self, result, stream=None, grade=None):
        self.rows.append(result)
        base = self.root / result["harness"] / f"{result['task']}-r{result['rep']}"
        base.parent.mkdir(exist_ok=True)
        if stream is not None:
            Path(str(base) + ".stdout.jsonl").write_text(
                "".join(json.dumps(e) + "\n" for e in stream), encoding="utf-8")
        if grade is not None:
            Path(str(base) + ".grader.json").write_text(json.dumps(grade), encoding="utf-8")

    def run(self, out):
        (self.root / "results.jsonl").write_text(
            "".join(json.dumps(r) + "\n" for r in self.rows), encoding="utf-8")
        return classifier.run([self.root], Path(out), MANIFEST)


def snapshot(directory):
    return {p.relative_to(directory).as_posix(): hashlib.sha256(p.read_bytes()).hexdigest()
            for p in sorted(Path(directory).rglob("*")) if p.is_file()}


class LargeTaskFailureTests(unittest.TestCase):
    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.campaign = Campaign(self.temporary.name)
        self.out = Path(self.temporary.name) / "out"

    def by_task(self, rows):
        return {(r["harness"], r["task"]): r for r in rows}

    def test_partial_fix_that_passes_the_public_test_stops_on_existing_checks(self):
        self.campaign.add(row("davinci", "m1-config-parser"), [
            *edit("e1", "config_parser.py"),
            *shell("s1", "python -m pytest -q tests/test_public_config.py", False),
            {"type": "harness_stats", "runtime": {"completionRequirementReviewsSkipped": 1}},
            final(),
        ], grader("test_hidden_config.py::test_invalid_syntax[  = value]",
                  "test_hidden_config.py::test_values_keep_equals_after_first_separator"))
        rows, summary = self.campaign.run(self.out)
        result = rows[0]
        self.assertEqual(result["hypothesis"], "stopped_on_existing_checks")
        self.assertEqual(result["failed_requirements"], "m1.reject_malformed;m1.equals_in_value")
        self.assertEqual(result["failed_requirement_prompt_states"], "explicit;implicit")
        self.assertEqual(result["timeline"], "E P+ F")
        self.assertEqual(result["last_check_after_last_edit"], "public:pass")
        self.assertEqual(result["requirement_reviews_skipped"], 1)
        self.assertEqual(result["reminders"], 0)
        self.assertIsNone(result["edits_after_first_reminder"])
        self.assertEqual(summary["per_harness"]["davinci"]["runs_with_review_skipped"], 1)
        self.assertEqual(summary["per_harness"]["davinci"]["failed_requirement_runs"]["m1.equals_in_value"],
                         {"runs": 1, "prompt_states": "implicit"})

    def test_reminder_followed_by_new_tests_is_recorded_and_labelled_as_own_checks(self):
        self.campaign.add(row("davinci", "m3-state-persistence", changed=["state_core/service.py"]), [
            *edit("e1", "state_core/service.py"),
            *shell("s1", "python -m pytest -q tests/test_public_state.py", False),
            {"type": "completion_reminder", "reason_code": "completion.requirements"},
            *edit("e2", "tests/test_public_state.py"),
            *shell("s2", "pytest -q", False),
            final(),
        ], grader("test_hidden_state.py::test_failed_replace_rolls_back"))
        result = self.campaign.run(self.out)[0][0]
        self.assertEqual(result["timeline"], "E P+ R T S+ F")
        self.assertEqual((result["reminders"], result["reminder_reasons"]), (1, "completion.requirements"))
        self.assertEqual((result["edits_after_first_reminder"], result["test_edits_after_first_reminder"],
                          result["checks_after_first_reminder"], result["check_passes_after_first_reminder"]),
                         (1, 1, 1, 1))
        self.assertEqual(result["hypothesis"], "own_checks_passed_hidden_failed")
        self.assertEqual(result["failed_requirements"], "m3.atomic_rollback")

    def test_codex_commands_and_file_changes_build_the_same_timeline(self):
        self.campaign.add(row("codex", "m4-cli-feature", changed=["cli.py"]), [
            {"type": "item.completed", "item": {"type": "file_change",
                                                "changes": [{"path": f"/abs/{SECRET}/cli.py", "kind": "update"}]}},
            {"type": "item.completed", "item": {"type": "command_execution",
                                                "command": "bash -lc 'python -m pytest -q tests/test_public_cli.py'",
                                                "exit_code": 1, "aggregated_output": SECRET}},
            {"type": "item.completed", "item": {"type": "agent_message", "text": SECRET}},
            {"type": "turn.completed", "usage": {}},
        ], grader("test_hidden_cli.py::test_invalid_inputs_exit_two[argv0]"))
        result = self.campaign.run(self.out)[0][0]
        self.assertEqual(result["timeline"], "E P- F")
        self.assertEqual(result["hypothesis"], "finished_with_failing_check")
        self.assertEqual(result["requirement_reviews_skipped"], "n/a")
        self.assertEqual(result["failed_requirement_prompt_states"], "partial")

    def test_batch_children_carry_their_own_status_and_reads_are_not_checks(self):
        self.campaign.add(row("davinci", "m1-config-parser"), [
            *edit("e1", "config_parser.py"),
            start("b1", "batch", operations=[
                {"tool": "exec_command", "args": {"command": "cat tests/test_public_config.py"}},
                {"tool": "exec_command", "args": {"command": "python -m pytest -q tests/test_public_config.py"}},
                {"tool": "exec_command", "args": {"command": "python -c 'import config_parser'"}}]),
            {"type": "tool_execution_end", "toolCallId": "b1", "toolName": "batch", "isError": False,
             "details": {"batch": True, "operations": [{"status": "ok"}, {"status": "ok"}, {"status": "error"}]}},
            final(),
        ], grader("test_hidden_config.py::test_invalid_syntax[broken]"))
        result = self.campaign.run(self.out)[0][0]
        self.assertEqual(result["timeline"], "E P+ A- F")
        self.assertEqual(result["last_check_after_last_edit"], "adhoc:fail")
        self.assertEqual(result["hypothesis"], "finished_with_failing_check")

    def test_no_check_and_compile_only_runs_finish_unverified(self):
        self.campaign.add(row("davinci", "m1-config-parser"), [
            *edit("e1", "config_parser.py"), *shell("s1", "python -m py_compile config_parser.py", False), final(),
        ], grader("test_hidden_config.py::test_invalid_syntax[broken]"))
        result = self.campaign.run(self.out)[0][0]
        self.assertEqual(result["timeline"], "E C+ F")
        self.assertEqual(result["hypothesis"], "finished_unverified")

    def test_missing_evidence_is_reported_as_unknown(self):
        self.campaign.add(row("davinci", "m1-config-parser"))  # no stdout, no grader output
        self.campaign.add(row("davinci", "m2-public-api-migration", tests_failed=3),
                          [*edit("e1", "userlib/core.py"), start("s1", "bash", command="pytest"), final()],
                          {"stdout": "3 failed in 0.1s", "stderr": ""})
        rows = self.by_task(self.campaign.run(self.out)[0])
        missing = rows[("davinci", "m1-config-parser")]
        self.assertEqual(missing["failed_requirements_source"], "unavailable")
        self.assertIn("stdout_log_missing", missing["unknowns"])
        self.assertIn("grader_log_missing", missing["unknowns"])
        unparsed = rows[("davinci", "m2-public-api-migration")]
        self.assertEqual(unparsed["failed_requirements_source"], "unparsed")
        self.assertEqual(unparsed["last_check_after_last_edit"], "suite:unknown")
        self.assertEqual(unparsed["hypothesis"], "last_check_outcome_unknown")
        self.assertIn("check_status_unknown:1", unparsed["unknowns"])

    def test_stopped_unchanged_unmapped_and_collection_failures(self):
        self.campaign.add(row("davinci", "m1-config-parser", stop_reason="provider_error"), [final()])
        self.campaign.add(row("davinci", "m3-state-persistence", changed=[]), [final()],
                          grader("test_hidden_state.py::test_new_case"))
        self.campaign.add(row("davinci", "m4-cli-feature"), [final()],
                          {"stdout": "ERROR test_hidden_cli.py - ImportError", "stderr": ""})
        rows = self.by_task(self.campaign.run(self.out)[0])
        self.assertEqual(rows[("davinci", "m1-config-parser")]["hypothesis"], "not_graded")
        unchanged = rows[("davinci", "m3-state-persistence")]
        self.assertEqual((unchanged["hypothesis"], unchanged["failed_requirements"]),
                         ("no_change", "unmapped:test_new_case"))
        self.assertEqual(rows[("davinci", "m4-cli-feature")]["failed_requirements"], "m4-cli-feature.grader_collection")

    def test_divergence_compares_harnesses_that_both_failed(self):
        events = [*edit("e1", "config_parser.py"),
                  *shell("s1", "python -m pytest -q tests/test_public_config.py", False), final()]
        self.campaign.add(row("davinci", "m1-config-parser"), events,
                          grader("test_hidden_config.py::test_invalid_syntax[broken]"))
        self.campaign.add(row("codex", "m1-config-parser"), [], grader(
            "test_hidden_config.py::test_values_keep_equals_after_first_separator"))
        _, summary = self.campaign.run(self.out)
        self.assertEqual(len(summary["divergences"]), 1)
        self.assertEqual(summary["divergences"][0]["failed_requirements"],
                         {"codex": ["m1.equals_in_value"], "davinci": ["m1.reject_malformed"]})

    def test_legacy_tasks_are_ignored(self):
        self.campaign.add(row("davinci", "t1-intervals"), [final()])
        rows, summary = self.campaign.run(self.out)
        self.assertEqual((rows, summary["runs"]), ([], 0))

    def test_output_is_sanitized_and_campaign_is_untouched(self):
        self.campaign.add(row("davinci", "m1-config-parser"), [
            *edit("e1", f"/abs/{SECRET}/config_parser.py"),
            *shell("s1", f"python -m pytest -q tests/test_public_config.py # {SECRET}", False), final(),
        ], grader(f"test_hidden_config.py::test_invalid_syntax[broken]"))
        self.campaign.run(self.out)  # writes the fixture's results.jsonl
        before = snapshot(self.campaign.root)
        classifier.run([self.campaign.root], self.out, MANIFEST)
        self.assertEqual(before, snapshot(self.campaign.root))
        for path in self.out.iterdir():
            text = path.read_text(encoding="utf-8")
            self.assertNotIn(SECRET, text, path.name)
            self.assertNotIn("C:/bench", text, path.name)

    def test_output_inside_a_campaign_is_refused(self):
        self.campaign.add(row("davinci", "m1-config-parser"), [final()])
        with self.assertRaises(ValueError):
            self.campaign.run(self.campaign.root / "analysis")
        self.assertFalse((self.campaign.root / "analysis").exists())

    def test_every_hidden_grader_test_is_mapped(self):
        import make_large_tasks
        import re
        source = Path(make_large_tasks.__file__).read_text(encoding="utf-8")
        for task, mapping in classifier.REQUIREMENTS.items():
            block = source.split(f'"id": "{task}"', 1)[1].split('"hidden": {', 1)[1].split('"solution": {', 1)[0]
            self.assertEqual(set(re.findall(r"def (test_\w+)\(", block)), set(mapping), task)


if __name__ == "__main__":
    unittest.main()
