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
        if stream is not None or grade is not None:
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
        self.assertEqual(result["failed_requirement_categories"], "specified;specified")
        self.assertEqual((result["failure_basis"], result["failed_specified"]), ("specified_failure", 2))
        self.assertEqual(result["timeline"], "E P+ F")
        self.assertEqual(result["last_check_after_last_edit"], "public:pass")
        self.assertEqual(result["requirement_reviews_skipped"], 1)
        self.assertEqual(result["reminders"], 0)
        self.assertIsNone(result["edits_after_first_reminder"])
        self.assertEqual(summary["per_model"]["m"]["davinci"]["runs_with_review_skipped"], 1)
        self.assertEqual(summary["per_model"]["m"]["davinci"]["failed_requirement_runs"]["m1.equals_in_value"],
                         {"runs": 1, "category": "specified"})

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
        self.assertEqual(result["failed_requirement_categories"], "mixed")
        self.assertEqual(result["failure_basis"], "cause_indeterminate")

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

    def test_a_campaign_without_large_runs_is_an_error_unless_allowed(self):
        self.campaign.add(row("davinci", "t1-intervals"), [final()])
        with self.assertRaisesRegex(ValueError, r"no large-task \(m\*\) runs in: campaign-01 \(1 rows\)"):
            self.campaign.run(self.out)
        self.assertFalse(self.out.exists())
        rows, summary = classifier.run([self.campaign.root], self.out, MANIFEST, allow_empty=True)
        self.assertEqual((rows, summary["runs"]), ([], 0))
        self.assertEqual(summary["campaigns_without_large_runs"], [{"campaign": "campaign-01", "rows": 1}])

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


    def test_unsafe_changed_paths_are_withheld_and_safe_ones_kept(self):
        unsafe = [f"C:\\Users\\{SECRET}\\a.py", f"C:/{SECRET}/a.py", f"c:{SECRET}.py",
                  f"\\\\server\\{SECRET}\\a.py", f"//server/{SECRET}/a.py", f"/home/{SECRET}/a.py",
                  f"~/{SECRET}.py", f"../{SECRET}.py", f"a/../../{SECRET}.py", f"a\\..\\{SECRET}.py",
                  f"file:///{SECRET}", f"a.py:{SECRET}", f"a\x00{SECRET}.py", f"a\n{SECRET}.py",
                  f"{SECRET} space.py", f"caf\u00e9{SECRET}.py", "x" * 300, "", ".", 42, None, {"p": SECRET}]
        safe = ["config_parser.py", "tests\\test_public_config.py", "./README.md", "pkg//sub/./mod.py"]
        self.campaign.add(row("davinci", "m1-config-parser", changed=safe + unsafe), [final()],
                          grader("test_hidden_config.py::test_invalid_syntax[broken]"))
        result = self.campaign.run(self.out)[0][0]
        self.assertEqual(result["changed_files"],
                         "README.md;config_parser.py;pkg/sub/mod.py;tests/test_public_config.py")
        self.assertIn(f"changed_paths_withheld:{len(unsafe)}", result["unknowns"])
        self.assertOutputHasNo(SECRET, "C:", "server", "/home", "~/", "..")

    def test_safe_relative_path_contract(self):
        cases = {"a.py": "a.py", "src\\pkg\\m.py": "src/pkg/m.py", "./x/./y.py": "x/y.py",
                 "C:\\x.py": None, "C:x.py": None, "\\\\h\\s\\x.py": None, "//h/s": None, "/x": None,
                 "~/x": None, "../x": None, "x/..": None, "x/.../y": None, "a./b": None, "-rf": None, ".github/ci.yml": ".github/ci.yml",
                 "x\ty": None, "x\x7fy": None, "": None, "/": None, ".": None, b"a.py": None, 3: None}
        for value, expected in cases.items():
            self.assertEqual(classifier.safe_relative_path(value), expected, repr(value))

    def test_free_text_fields_are_redacted(self):
        self.campaign.add(row("davinci", "m1-config-parser", variant=f"C:\\{SECRET}", model=f"gpt {SECRET}",
                              stop_reason=f"not run: {SECRET}", binary_sha256=SECRET, tests_passed=SECRET,
                              exit=SECRET, unrelated=SECRET), [
            *edit("e1", "config_parser.py"),
            {"type": "completion_reminder", "reason_code": f"{SECRET} please write tests"},
            {"type": "verification_notice", "status": f"/home/{SECRET}"},
            final(),
        ], {"stdout": f"FAILED test_hidden_config.py::{SECRET}/../x - boom\n1 failed", "stderr": ""})
        result = self.campaign.run(self.out)[0][0]
        self.assertEqual((result["variant"], result["model"], result["stop_reason"], result["binary_sha256"],
                          result["tests_passed"], result["exit"], result["unrelated_changes"]),
                         ("redacted", "redacted", "redacted", None, None, None, None))
        self.assertEqual((result["reminder_reasons"], result["verification_notices"]), ("redacted", "redacted"))
        self.assertEqual(result["failed_requirements"], "unmapped:redacted")
        self.assertOutputHasNo(SECRET, "/home")

    def test_unsafe_run_identity_never_reads_outside_the_campaign(self):
        outside = self.campaign.root.parent / "m1-config-parser-r0"
        Path(str(outside) + ".stdout.jsonl").write_text(json.dumps(start("x", "edit", path="a.py")) + "\n",
                                                         encoding="utf-8")
        self.campaign.add(dict(row("davinci", "m1-config-parser"), harness=".."))
        self.campaign.add(dict(row("davinci", "m1-config-parser", rep=1), task="m1/../../x"))
        rows = self.campaign.run(self.out)[0]
        for result in rows:
            self.assertIn("unsafe_run_identity", result["unknowns"])
            self.assertEqual((result["timeline"], result["failed_requirements_source"]), ("", "unavailable"))
        self.assertEqual(sorted(str(r["harness"]) for r in rows), ["davinci", "redacted"])

    def test_failure_basis_separates_what_the_grader_output_can_establish(self):
        cases = {
            # (task, failed nodes) -> (requirements, categories, basis)
            "m4-cli-feature": (["test_hidden_cli.py::test_legacy_total_still_works"],
                               "m4.legacy_total", "contradictory", "only_underspecified_or_contradictory"),
            "m3-state-persistence": (["test_hidden_state.py::test_invalid_inputs[a-r-delta2]",
                                      "test_hidden_state.py::test_failed_replace_rolls_back"],
                                     "m3.empty_delta;m3.atomic_rollback", "underspecified;mixed",
                                     "cause_indeterminate"),
            "m1-config-parser": (["test_hidden_config.py::test_invalid_syntax[  = value]",
                                  "test_hidden_config.py::test_whitespace_comments_bom_and_duplicate_last_wins"],
                                 "m1.reject_malformed;m1.bom_trim_duplicates", "specified;mixed",
                                 "specified_failure"),
            "m2-public-api-migration": (["test_hidden_api.py::test_validation_errors_propagate"],
                                        "m2.blank_name_rejected", "underspecified",
                                        "only_underspecified_or_contradictory"),
        }
        for task, (nodes, _, _, _) in cases.items():
            self.campaign.add(row("davinci", task, changed=["cli.py"]), [final()], grader(*nodes))
        rows = self.by_task(self.campaign.run(self.out)[0])
        for task, (_, requirements, categories, basis) in cases.items():
            result = rows[("davinci", task)]
            self.assertEqual((result["failed_requirements"], result["failed_requirement_categories"],
                              result["failure_basis"]), (requirements, categories, basis), task)
        self.assertEqual(rows[("davinci", "m3-state-persistence")]["failed_mixed"], 1)

    def test_other_cases_of_a_parametrized_test_keep_its_category(self):
        self.campaign.add(row("davinci", "m3-state-persistence", changed=["a.py"]), [final()],
                          grader("test_hidden_state.py::test_invalid_inputs[-r-delta0]",
                                 "test_hidden_state.py::test_invalid_inputs[a--delta1]"))
        result = self.campaign.run(self.out)[0][0]
        self.assertEqual((result["failed_requirements"], result["failure_basis"]),
                         ("m3.invalid_ids", "specified_failure"))

    def test_unknown_grader_detail_never_counts_as_a_category(self):
        self.campaign.add(row("davinci", "m1-config-parser"), [final()])
        self.campaign.add(row("davinci", "m4-cli-feature"), [final()],
                          {"stdout": "ERROR test_hidden_cli.py - ImportError", "stderr": ""})
        rows = self.by_task(self.campaign.run(self.out)[0])
        self.assertEqual(rows[("davinci", "m1-config-parser")]["failure_basis"], "unknown")
        collection = rows[("davinci", "m4-cli-feature")]
        self.assertEqual((collection["failure_basis"], collection["failed_unknown"]), ("cause_indeterminate", 1))

    def test_models_are_never_pooled(self):
        self.campaign.add(row("davinci", "m1-config-parser", model="gpt-5.6-luna"), [final()],
                          grader("test_hidden_config.py::test_invalid_syntax[broken]"))
        self.campaign.add(row("davinci", "m2-public-api-migration", model="gpt-6-luna"), [final()],
                          grader("test_hidden_api.py::test_validation_errors_propagate"))
        _, summary = self.campaign.run(self.out)
        self.assertEqual(set(summary["per_model"]), {"gpt-5.6-luna", "gpt-6-luna"})
        self.assertEqual(summary["per_model"]["gpt-5.6-luna"]["davinci"]["failure_basis"], {"specified_failure": 1})
        self.assertEqual(summary["per_model"]["gpt-6-luna"]["davinci"]["failure_basis"],
                         {"only_underspecified_or_contradictory": 1})

    def test_every_category_is_known_and_every_case_names_a_real_test(self):
        for task, mapping in classifier.REQUIREMENTS.items():
            for requirement, category, _ in mapping.values():
                self.assertIn(category, classifier.CATEGORIES, requirement)
        for (task, name, _), (_, category, _) in classifier.CASES.items():
            self.assertIn(name, classifier.REQUIREMENTS[task])
            self.assertIn(category, classifier.CATEGORIES)

    def test_variant_subdirectories_are_expanded(self):
        parent = Path(self.temporary.name) / "luna-x"
        for variant, harness in (("arm-a", "davinci"), ("arm-b", "codex")):
            (parent / variant).mkdir(parents=True)
            (parent / variant / "results.jsonl").write_text(
                json.dumps(row(harness, "m1-config-parser")) + "\n", encoding="utf-8")
        rows, summary = classifier.run([parent], self.out, MANIFEST)
        self.assertEqual(summary["campaigns"], ["luna-x/arm-a", "luna-x/arm-b"])
        self.assertEqual(sorted(r["campaign"] for r in rows), ["luna-x/arm-a", "luna-x/arm-b"])
        with self.assertRaisesRegex(ValueError, "no results.jsonl"):
            classifier.run([Path(self.temporary.name) / "missing"], self.out, MANIFEST)

    def test_an_output_directory_that_looks_like_a_campaign_is_refused(self):
        self.campaign.add(row("davinci", "m1-config-parser"), [final()])
        self.out.mkdir()
        (self.out / "results.jsonl").write_text("", encoding="utf-8")
        with self.assertRaisesRegex(ValueError, "looks like a campaign"):
            self.campaign.run(self.out)

    def test_no_process_network_or_grader_execution(self):
        import os
        import socket
        import subprocess
        source = Path(classifier.__file__).read_text(encoding="utf-8")
        for name in ("subprocess", "socket", "urllib", "http.client", "os.system", "exec(", "eval(",
                     "import bench", "importlib"):
            self.assertNotIn(name, source)

        def refuse(*args, **kwargs):
            raise AssertionError("the classifier must not start processes or open connections")

        self.campaign.add(row("davinci", "m1-config-parser"), [
            *edit("e1", "config_parser.py"), *shell("s1", "pytest -q", False), final()],
            grader("test_hidden_config.py::test_invalid_syntax[broken]"))
        patches = [(subprocess, "Popen"), (subprocess, "run"), (os, "system"), (socket, "socket"),
                   (socket, "create_connection")]
        saved = [(module, name, getattr(module, name)) for module, name in patches]
        try:
            for module, name in patches:
                setattr(module, name, refuse)
            self.campaign.run(self.out)
        finally:
            for module, name, value in saved:
                setattr(module, name, value)

    def test_locate_finds_large_campaigns_without_entering_private_directories(self):
        root = Path(self.temporary.name) / "bench"

        def campaign(relative, rows, graders=()):
            directory = root / relative
            directory.mkdir(parents=True)
            (directory / "results.jsonl").write_text("".join(json.dumps(r) + "\n" for r in rows), encoding="utf-8")
            for name in graders:
                (directory / "davinci").mkdir(exist_ok=True)
                (directory / "davinci" / name).write_text("{}", encoding="utf-8")

        campaign("old-h2h", [dict(row("davinci", "m1-config-parser", model="gpt-5.6-luna",
                                      source_sha="e8a276ec" + "0" * 32, started_at="2026-10-01T00:00:00Z")),
                             row("codex", "m1-config-parser", model="gpt-5.6-luna"),
                             row("davinci", "t1-intervals")])
        campaign("luna-high-recovery-02/vm-off-cm-off+codex",
                 [dict(row("davinci", "m4-cli-feature", model="gpt-6-luna", started_at="2026-10-10T00:00:00Z"))], graders=["m4-cli-feature-r0.grader.json", "t1-intervals-r0.grader.json"])
        campaign("luna-high-wire-01", [row("davinci", "t1-intervals")])
        campaign("x/_agent", [row("davinci", "m1-config-parser", model=SECRET)])
        campaign("y/davinci/m1-config-parser-r0", [row("davinci", "m1-config-parser", model=SECRET)])
        before = snapshot(root)
        found = classifier.locate([root, Path(self.temporary.name) / "missing"])
        self.assertEqual(before, snapshot(root))
        self.assertEqual([Path(item["path"]).relative_to(root).as_posix() for item in found],
                         ["old-h2h", "luna-high-recovery-02/vm-off-cm-off+codex"])
        old, new = found
        self.assertEqual((old["rows"], old["large_runs"], old["models"], old["source_sha"],
                          old["original_baseline_candidate"]),
                         (3, {"codex": 1, "davinci": 1}, ["gpt-5.6-luna"], ["e8a276ec0000"], True))
        self.assertEqual((new["grader_logs"], new["models"], new["original_baseline_candidate"]),
                         (1, ["gpt-6-luna"], False))
        self.assertNotIn(SECRET, json.dumps(found))

    def assertOutputHasNo(self, *needles):
        for path in self.out.iterdir():
            text = path.read_text(encoding="utf-8")
            for needle in needles:
                self.assertNotIn(needle, text, f"{needle!r} in {path.name}")


if __name__ == "__main__":
    unittest.main()
