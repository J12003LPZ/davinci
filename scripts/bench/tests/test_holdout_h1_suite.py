"""The H1 suite helpers against the importer's real on-disk layout. Slow (real git), so kept out of the mutation loop."""
import contextlib
import io
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, str(Path(__file__).resolve().parents[1]))
sys.path.insert(0, str(Path(__file__).resolve().parent))
import holdout_h1 as h1
from test_holdout_h1 import HIDDEN_LINE, PROMPT_LINE, TASKS


def git(repo, *args):
    done = subprocess.run(["git", "-c", "core.autocrlf=false", "-c", "user.name=t", "-c", "user.email=t@example.invalid",
                           "-C", str(repo), *args], check=True, capture_output=True, text=True)
    return done.stdout.strip()


def real_suite(base, count=h1.TASK_COUNT, language="python", split="holdout", size="large", shared_repo=False):
    """A frozen private suite built by the harness's own importer from tiny throwaway git repositories."""
    import private_suite
    base = Path(base)
    tasks = []
    for index in range(count):
        repo = base / "src" / ("r%d" % index)
        (repo / "pkg").mkdir(parents=True)
        (repo / "tests").mkdir()
        git(repo, "init", "-q")
        (repo / "pkg" / "core.py").write_bytes(b"def value():\n    return 0\n")
        (repo / "tests" / "test_visible.py").write_bytes(b"def test_ok():\n    assert True\n")
        git(repo, "add", "-A")
        git(repo, "commit", "-qm", "starter")
        starter = git(repo, "rev-parse", "HEAD")
        (repo / "pkg" / "core.py").write_bytes(("def value():\n    return %d\n" % (index + 1)).encode())
        hidden_test = "from pkg.core import value\n\n\ndef test_value():\n    assert value() == %d\n    assert %r\n"
        (repo / "tests" / "test_hidden.py").write_bytes((hidden_test % (index + 1, HIDDEN_LINE)).encode())
        git(repo, "add", "-A")
        git(repo, "commit", "-qm", "reference")
        reference = git(repo, "rev-parse", "HEAD")
        tasks.append({"id": "h1-%02d" % (index + 1),
                      "repository_id": "shared" if shared_repo else "repo-%d" % index,
                      "repository": "src/r%d" % index, "reference_commit": reference, "starter_commit": starter,
                      "language": language, "split": split, "size_class": size, "visible_tests": True,
                      "requires_existing_test_changes": False,
                      "human_prompt": {"authored_from": "pr-description", "diff_used": False,
                                       "text": PROMPT_LINE, "author": "author-%d" % index, "source": "synthetic"},
                      "test_paths": ["tests/test_hidden.py"],
                      "grader_command": ["python", "-m", "pytest", "-q", "tests/test_hidden.py"],
                      "regression_command": ["python", "-m", "pytest", "-q", "tests/test_visible.py"]})
    manifest = base / "manifest.json"
    manifest.write_text(json.dumps({"schema_version": 1, "kind": "private-repository-suite", "tasks": tasks}),
                        encoding="utf-8")
    destination = base / "vault" / "suite"
    private_suite.import_suite(manifest, destination)
    return destination


class RealSuiteTests(unittest.TestCase):
    """The scanner and the pins builder against the importer's real on-disk layout, not a hand-made imitation."""

    def test_pins_for_a_valid_suite_are_built_and_pass_the_shape_check(self):
        with tempfile.TemporaryDirectory() as d:
            suite = real_suite(d)
            pins, errors = h1.make_pins(suite, "e" * 40)
            self.assertEqual(errors, [])
            self.assertEqual(pins["task_ids"], sorted(TASKS))
            self.assertRegex(pins["private_suite_digest"], r"^[0-9a-f]{64}$")
            self.assertEqual(h1.pins_errors(pins), [])
            self.assertEqual(pins["private_suite_digest"], h1.make_pins(suite, "e" * 40)[0]["private_suite_digest"])

    def test_wrong_shape_is_refused(self):
        for kwargs, expect in (({"count": 5}, "exactly 6"), ({"language": "go"}, "language"),
                               ({"split": "dev"}, "split"), ({"size": "small"}, "size class"),
                               ({"shared_repo": True}, "own repository id")):
            with tempfile.TemporaryDirectory() as d:
                try:
                    suite = real_suite(d, **kwargs)
                except ValueError:
                    continue  # the importer itself refused it
                pins, errors = h1.make_pins(suite, "e" * 40)
                self.assertTrue(errors, kwargs)
                if pins is not None:
                    self.assertTrue(any(expect in e for e in errors), (kwargs, errors))
                else:
                    self.assertIn("refused by the loader", errors[0])

    def test_unreadable_suite_is_an_error_not_a_traceback(self):
        with tempfile.TemporaryDirectory() as d:
            pins, errors = h1.make_pins(Path(d) / "missing", "e" * 40)
            self.assertIsNone(pins)
            self.assertIn("refused by the loader", errors[0])

    def test_needles_come_from_the_real_hidden_overlay_and_catch_a_leak(self):
        with tempfile.TemporaryDirectory() as d:
            suite = real_suite(d)
            self.assertIn("assert %r" % HIDDEN_LINE, h1.needles(suite).values())
            root = Path(d) / "results" / "h1-01-A-r0" / "davinci"
            root.mkdir(parents=True)
            (root / "x-r0.stdout.jsonl").write_text(json.dumps({"t": "assert %r" % HIDDEN_LINE}), encoding="utf-8")
            self.assertEqual(h1.leak_scan(Path(d) / "results", suite)["status"], "LEAK")

    def test_a_clean_transcript_over_the_real_suite_is_clean(self):
        with tempfile.TemporaryDirectory() as d:
            suite = real_suite(d)
            root = Path(d) / "results" / "h1-01-A-r0" / "davinci"
            root.mkdir(parents=True)
            (root / "x-r0.stdout.jsonl").write_text(json.dumps({"t": "edited pkg/core.py"}), encoding="utf-8")
            self.assertEqual(h1.leak_scan(Path(d) / "results", suite)["status"], "CLEAN")

    def test_tampering_with_a_frozen_suite_is_detected(self):
        import private_suite
        with tempfile.TemporaryDirectory() as d:
            suite = real_suite(d)
            hidden = next(suite.glob("*/hidden/tests/test_hidden.py"))
            hidden.write_bytes(hidden.read_bytes() + b"# changed\n")
            with self.assertRaises(ValueError):
                private_suite.load_frozen(suite, h1.SPLIT)


class MakePinsCliTests(unittest.TestCase):
    def run_main(self, *argv):
        out = io.StringIO()
        code = 0
        with contextlib.redirect_stdout(out):
            try:
                h1.main(list(argv))
            except SystemExit as exit_:
                code = exit_.code
        return code, out.getvalue()

    def test_make_pins_prints_ids_and_digest_only(self):
        with tempfile.TemporaryDirectory() as d:
            suite = real_suite(d)
            code, out = self.run_main("make-pins", "--suite", str(suite), "--harness-sha", "e" * 40)
            report = json.loads(out)
            self.assertEqual((code, report["ok"]), (0, True))
            for secret in (HIDDEN_LINE.strip(), PROMPT_LINE, str(suite), "core.py"):
                self.assertNotIn(secret, out)

    def test_make_pins_exit_status_on_a_refused_suite(self):
        with tempfile.TemporaryDirectory() as d:
            code, out = self.run_main("make-pins", "--suite", str(Path(d) / "none"), "--harness-sha", "e" * 40)
            self.assertEqual((code, json.loads(out)["ok"]), (1, False))


class RealReportTests(unittest.TestCase):
    """The report command end to end: real frozen suite, pinned digest, 54 rows and 54 transcripts."""

    def build(self, base, transcript_for=None):
        from test_holdout_h1 import study
        suite = real_suite(base)
        pins, errors = h1.make_pins(suite, "e" * 40)
        self.assertEqual(errors, [])
        root = Path(base) / "results"
        for (arm, task, rep), r in study().items():
            r = dict(r, private_suite_digest=pins["private_suite_digest"])
            unit = r.pop("_unit")
            directory = root / unit["name"]
            directory.mkdir(parents=True, exist_ok=True)
            with open(directory / "results.jsonl", "a", encoding="utf-8") as handle:
                handle.write(json.dumps(r) + chr(10))
            transcript = directory / r["harness"] / ("%s-r%d.stdout.jsonl" % (task, rep))
            transcript.parent.mkdir(parents=True, exist_ok=True)
            transcript.write_text(json.dumps({"t": (transcript_for or (lambda *a: "edited pkg/core.py"))(arm, task, rep)}),
                                  encoding="utf-8")
        pins_path = Path(base) / "pins.json"
        pins_path.write_text(json.dumps(pins), encoding="utf-8")
        return suite, root, pins_path

    def report(self, root, pins_path, suite):
        out = io.StringIO()
        with contextlib.redirect_stdout(out):
            h1.main(["report", "--root", str(root), "--pins", str(pins_path), "--suite", str(suite)])
        return json.loads(out.getvalue())

    def test_a_clean_complete_study(self):
        with tempfile.TemporaryDirectory() as d:
            suite, root, pins = self.build(d)
            report = self.report(root, pins, suite)
            self.assertEqual((report["verdict"], report["leak_scan"]["status"]), ("COMPLETE", "CLEAN"))
            self.assertEqual(report["leak_scan"]["files_scanned"], 54)

    def test_one_leaking_transcript_blocks_the_numbers(self):
        with tempfile.TemporaryDirectory() as d:
            suite, root, pins = self.build(d, lambda arm, task, rep: ("assert %r" % HIDDEN_LINE) if (arm, task, rep) == ("B", "h1-02", 1) else "ok")
            report = self.report(root, pins, suite)
            self.assertEqual(report["verdict"], "CONTAMINATION_REVIEW")
            self.assertEqual(report["leak_scan"]["campaigns_with_hits"], {"h1-04-B-r1": 1})
            self.assertNotIn("passes", report)

    def test_a_missing_transcript_blocks_the_numbers(self):
        with tempfile.TemporaryDirectory() as d:
            suite, root, pins = self.build(d)
            next(root.rglob("h1-01-r0.stdout.jsonl")).unlink()
            self.assertEqual(self.report(root, pins, suite)["leak_scan"]["status"], "INCOMPLETE_SCAN")

    def test_the_wrong_suite_blocks_the_numbers(self):
        with tempfile.TemporaryDirectory() as d, tempfile.TemporaryDirectory() as other:
            suite, root, pins = self.build(d)
            wrong = real_suite(other)  # a valid frozen suite, but not the pinned one
            digest = json.loads(pins.read_text(encoding="utf-8"))["private_suite_digest"]
            self.assertEqual(h1.leak_scan(root, wrong, expected_runs=54, expected_digest=digest)["status"], "WRONG_SUITE")
            self.assertEqual(self.report(root, pins, wrong)["verdict"], "CONTAMINATION_REVIEW")


class SuiteErrorTests(unittest.TestCase):
    def frozen(self, **changes):
        labels = {t: {"split": "holdout", "language": "Python", "size_class": "large", "repository_id": "r-" + t}
                  for t in TASKS}
        out = {"tasks": list(TASKS), "labels": labels}
        out.update(changes)
        return out

    def labelled(self, task, **changes):
        frozen = self.frozen()
        frozen["labels"][task] = dict(frozen["labels"][task], **changes)
        return frozen

    def test_valid_shape(self):
        self.assertEqual(h1.suite_errors(self.frozen()), [])

    def test_structural_problems(self):
        self.assertEqual(h1.suite_errors(None), ["suite.json is not an object"])
        self.assertEqual(h1.suite_errors(self.frozen(labels=None)), ["suite labels missing"])
        for tasks in (TASKS[:5], TASKS[:5] + [TASKS[0]]):
            self.assertEqual(h1.suite_errors(self.frozen(tasks=tasks)), ["the suite must hold exactly 6 distinct tasks"])

    def test_each_label_rule_reports_its_own_error(self):
        cases = ((dict(language="go"), "language is not python"), (dict(split="dev"), "split is not holdout"),
                 (dict(size_class="small"), "size class is not large"))
        for change, message in cases:
            errors = h1.suite_errors(self.labelled(TASKS[2], **change))
            self.assertEqual(errors, ["%s: %s" % (TASKS[2], message)])

    def test_language_comparison_ignores_case(self):
        self.assertEqual(h1.suite_errors(self.labelled(TASKS[0], language="PYTHON")), [])

    def test_repositories_must_be_distinct(self):
        self.assertEqual(h1.suite_errors(self.labelled(TASKS[1], repository_id="r-" + TASKS[0])),
                         ["each task needs its own repository id"])

    def test_descriptive_task_ids_are_refused(self):
        frozen = self.frozen(tasks=["Ledger Task"] + TASKS[1:])
        frozen["labels"]["Ledger Task"] = dict(frozen["labels"][TASKS[0]], repository_id="r-x")
        errors = h1.suite_errors(frozen)
        self.assertEqual(len(errors), 1)
        self.assertIn("not an opaque id", errors[0])


if __name__ == "__main__":
    unittest.main()
