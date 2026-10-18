"""Offline tests for the Codemode adoption eval's grader. No model calls."""
import contextlib
import importlib.util
import io
import json
import tempfile
import unittest
from argparse import Namespace
from pathlib import Path

BENCH = Path(__file__).resolve().parents[1]
REPO = BENCH.parents[1]
SPEC = importlib.util.spec_from_file_location("codemode_adoption", BENCH / "codemode_adoption.py")
adoption = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(adoption)

TRUTH = adoption.truth(REPO)


def perfect(prompt):
    t = TRUTH
    if prompt == "p1-codemode-dir":
        return "\n".join(f"| `{n}` | {min(t['counts'][n])} | {t['docs'][n]} |" for n in t["files"])
    if prompt == "p2-constructors":
        return "\n".join(f"- {base} line {number}" for base, number in t["constructors"])
    if prompt == "p3-root-config":
        return "\n".join(f"{n}\n" + "\n".join(lines) for n, lines in t["first3"].items())
    if prompt == "p4-crate-versions":
        return "\n".join(f"- {c}: {t['version']}, {files} files" for c, (files, _) in t["src"].items())
    if prompt == "p5-docs-summary":
        return "The doc covers " + t["headings"][0] + " and more. The changelog starts with " + t["changelog"][-1] + "."
    callers = ", ".join(t["callers"]) or "nothing calls it"
    return f"WorkerSlotCapacity is defined in capacity.rs. acquire_exclusive callers: {callers}."


class GraderTests(unittest.TestCase):
    def test_truth_is_populated_from_the_real_repository(self):
        self.assertTrue(TRUTH["files"])
        self.assertTrue(TRUTH["constructors"])
        self.assertTrue(TRUTH["version"])
        self.assertEqual(set(TRUTH["first3"]), set(adoption.ROOT_FILES) & {p.name for p in REPO.iterdir()})

    def test_a_correct_answer_passes_every_question(self):
        for prompt in adoption.PROMPTS:
            self.assertEqual(adoption.check(prompt, perfect(prompt), TRUTH), [], prompt)

    def test_wrong_or_missing_facts_fail(self):
        name = TRUTH["files"][0]
        wrong_count = perfect("p1-codemode-dir").replace(f"| {min(TRUTH['counts'][name])} |", "| 99999 |", 1)
        self.assertTrue(adoption.check("p1-codemode-dir", wrong_count, TRUTH))
        number = TRUTH["constructors"][0][1]
        self.assertTrue(adoption.check("p2-constructors", perfect("p2-constructors").replace(str(number), "1"), TRUTH))
        first = next(iter(TRUTH["first3"].values()))[0]
        self.assertTrue(adoption.check("p3-root-config", perfect("p3-root-config").replace(first, "x"), TRUTH))
        crate = next(iter(TRUTH["src"]))
        self.assertTrue(adoption.check("p4-crate-versions", perfect("p4-crate-versions").replace(TRUTH["version"], "0.0.0"), TRUTH))
        self.assertIn(f"{crate} absent", adoption.check("p4-crate-versions", "no crates", TRUTH))
        self.assertTrue(adoption.check("p6-worker-slots", "It is somewhere.", TRUTH))
        for prompt in adoption.PROMPTS:
            self.assertEqual(adoption.check(prompt, "   ", TRUTH), ["no final answer"])

    def test_strict_grading_rejects_partly_right_answers(self):
        t = TRUTH
        name = t["files"][0]
        no_doc = perfect("p1-codemode-dir").replace(t["docs"][name], "", 1)
        self.assertIn(f"{name} doc comment", adoption.check("p1-codemode-dir", no_doc, t))
        # The file name and its line number must share a row to be a location.
        base, number = t["constructors"][0]
        rows = [f"{b} line {n}" for b, n in t["constructors"] if (b, n) != (base, number)]
        split = "\n".join(rows + [f"{base} is mentioned here", f"and {number} is somewhere else"])
        if sum(1 for b, _ in t["constructors"] if b == base) == 1:
            self.assertIn(f"{base}:{number}", adoption.check("p2-constructors", split, t))
        vague = "The Codemode doc is a guide. The changelog lists some release notes."
        self.assertTrue(adoption.check("p5-docs-summary", vague, t))

    def test_a_version_component_never_counts_as_a_file_count(self):
        crate, (files, _) = next(iter(TRUTH["src"].items()))
        wrong = next(n for n in range(900, 999) if n != files)
        major = TRUTH["version"].split(".")[0]
        good = next(l for l in perfect("p4-crate-versions").splitlines() if l.startswith(f"- {crate}:"))
        bad = f"- {crate}: {TRUTH['version']}, {wrong} files, {major}"
        answer = perfect("p4-crate-versions").replace(good, bad)
        self.assertIn(f"{crate} src count", adoption.check("p4-crate-versions", answer, TRUTH))

    def test_read_run_extracts_the_answer_and_counts_tool_starts(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp, "out.jsonl")
            events = [
                {"type": "tool_execution_start", "toolName": "codemode"},
                {"type": "tool_execution_start", "toolName": "read"},
                {"type": "tool_execution_start", "toolName": "codemode"},
                {"type": "agent_end", "messages": [
                    {"role": "user", "content": [{"type": "text", "text": "q"}]},
                    {"role": "assistant", "content": [{"type": "text", "text": "the answer"}]}]},
            ]
            path.write_text("garbage\n" + "\n".join(json.dumps(e) for e in events), encoding="utf-8")
            answer, tools = adoption.read_run(path)
        self.assertEqual(answer, "the answer")
        self.assertEqual(tools["codemode"], 2)
        self.assertEqual(sum(tools.values()), 3)

    def test_report_enforces_adoption_and_correctness_thresholds(self):
        def write(out, arm, prompt, answer, codemode_calls):
            run = Path(out, "runs", f"{arm}-{prompt}-r0")
            run.mkdir(parents=True)
            events = [{"type": "tool_execution_start", "toolName": "codemode"}] * codemode_calls
            events.append({"type": "agent_end", "messages": [{"role": "assistant", "content": [{"type": "text", "text": answer}]}]})
            Path(run, "out.jsonl").write_text("\n".join(json.dumps(e) for e in events), encoding="utf-8")

        def verdict(answers, calls):
            with tempfile.TemporaryDirectory() as out:
                for prompt in adoption.PROMPTS:
                    write(out, "on", prompt, answers(prompt), calls)
                    write(out, "off", prompt, perfect(prompt), 0)
                with contextlib.redirect_stdout(io.StringIO()):
                    return adoption.report(Namespace(out=out, repo=REPO))

        self.assertTrue(verdict(perfect, 1))
        self.assertFalse(verdict(perfect, 0), "no adoption must fail")
        self.assertFalse(verdict(lambda p: "wrong", 1), "wrong answers must fail")

    def test_codemode_must_not_answer_worse_than_the_baseline(self):
        def write(out, arm, prompt, answer):
            run = Path(out, "runs", f"{arm}-{prompt}-r0")
            run.mkdir(parents=True)
            events = [{"type": "tool_execution_start", "toolName": "codemode"},
                      {"type": "agent_end", "messages": [{"role": "assistant", "content": [{"type": "text", "text": answer}]}]}]
            Path(run, "out.jsonl").write_text("\n".join(json.dumps(e) for e in events), encoding="utf-8")

        prompts = list(adoption.PROMPTS)
        with tempfile.TemporaryDirectory() as out:
            for prompt in prompts:
                write(out, "on", prompt, "wrong" if prompt == prompts[0] else perfect(prompt))
                write(out, "off", prompt, perfect(prompt))
            buffer = io.StringIO()
            with contextlib.redirect_stdout(buffer):
                verdict = adoption.report(Namespace(out=out, repo=REPO))
        # 5/6 strict is above the 80% floor but below the 100% baseline.
        self.assertFalse(verdict)
        self.assertIn("baseline strict correct 100%", buffer.getvalue())
        self.assertIn("facts=", buffer.getvalue())


if __name__ == "__main__":
    unittest.main()
