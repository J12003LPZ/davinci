"""Offline tests for the Codemode adoption eval's grader. No model calls."""
import contextlib
import importlib.util
import io
import os
import re
import subprocess
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
        tokens = sorted({w for l in t["changelog"] for w in re.findall(r"\d+\.\d+\.\d+|Unreleased", l)})
        return f"The doc covers {t['headings'][0]} and {t['headings'][1]}. The changelog lists {' and '.join(tokens)}."
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

    def test_a_listed_file_that_does_not_exist_is_penalized(self):
        absent = adoption.ROOT_FILES[-1]
        truth = dict(TRUTH, first3={k: v for k, v in TRUTH["first3"].items() if k != absent})
        base = "\n".join(f"{n}\n" + "\n".join(lines) for n, lines in truth["first3"].items())
        self.assertEqual(adoption.check("p3-root-config", base, truth), [])
        invented = base + f"\n{absent}\nedition = 2021"
        self.assertTrue(adoption.check("p3-root-config", invented, truth))
        honest = base + f"\n{absent} does not exist"
        self.assertEqual(adoption.check("p3-root-config", honest, truth), [])

    def test_the_file_count_must_be_the_number_tied_to_files(self):
        crate, (files, _) = next(iter(TRUTH["src"].items()))
        good = next(l for l in perfect("p4-crate-versions").splitlines() if l.startswith(f"- {crate}:"))
        swapped = f"- {crate}: {TRUTH['version']}, 999 files, {files} modules"
        answer = perfect("p4-crate-versions").replace(good, swapped)
        self.assertIn(f"{crate} src count", adoption.check("p4-crate-versions", answer, TRUTH))

    def test_a_thin_docs_summary_fails(self):
        t = TRUTH
        one_heading = f"The doc covers {t['headings'][0]}. The changelog lists Unreleased."
        self.assertTrue(adoption.check("p5-docs-summary", one_heading, t))

    def test_callers_include_every_call_expression_and_skip_the_definition(self):
        with tempfile.TemporaryDirectory() as tmp:
            crate = Path(tmp, "crates", "x", "src")
            crate.mkdir(parents=True)
            Path(crate, "capacity.rs").write_text(
                "pub struct WorkerSlotCapacity;\nimpl WorkerSlotCapacity {\n"
                "    pub fn acquire_exclusive(&self) {}\n}\n"
                "#[test]\nfn t() { WorkerSlotCapacity.acquire_exclusive(); }\n", encoding="utf-8")
            Path(crate, "user.rs").write_text("fn f(c: C) { c.acquire_exclusive(); }\n", encoding="utf-8")
            for name in ("Cargo.toml", "CHANGELOG.md"):
                Path(tmp, name).write_text('version = "1.0.0"\n', encoding="utf-8")
            Path(tmp, "docs").mkdir()
            Path(tmp, "docs", "codemode.md").write_text("# T\n", encoding="utf-8")
            Path(tmp, "crates", "davinci-agent", "src", "codemode").mkdir(parents=True)
            t = adoption.truth(tmp)
        self.assertEqual(t["callers"], ["capacity.rs", "user.rs"])
        self.assertEqual(t["definition"], ["capacity.rs"])

    def test_a_crashed_run_fails_even_with_a_correct_answer(self):
        with tempfile.TemporaryDirectory() as out:
            prompts = list(adoption.PROMPTS)
            for prompt in prompts:
                for arm in ("on", "off"):
                    run = Path(out, "runs", f"{arm}-{prompt}-r0")
                    run.mkdir(parents=True)
                    events = [{"type": "tool_execution_start", "toolName": "codemode"},
                              {"type": "agent_end", "messages": [{"role": "assistant", "content": [{"type": "text", "text": perfect(prompt)}]}]}]
                    Path(run, "out.jsonl").write_text("\n".join(json.dumps(e) for e in events), encoding="utf-8")
            rows = "arm,prompt,rep,exit,wall_s\n" + "".join(
                f"{arm},{p},0,{'timeout' if (arm, p) == ('on', prompts[0]) else 0},1\n" for p in prompts for arm in ("on", "off"))
            Path(out, "runs.csv").write_text(rows, encoding="utf-8")
            buffer = io.StringIO()
            with contextlib.redirect_stdout(buffer):
                adoption.report(Namespace(out=out, repo=REPO))
        self.assertIn("process exit timeout", buffer.getvalue())
        self.assertIn("crashed=1", buffer.getvalue())

    def test_credentials_never_land_in_the_results_directory(self):
        seen = []

        real_run = subprocess.run

        def fake_run(cmd, cwd=None, env=None, stdout=None, stderr=None, timeout=None, **kw):
            if cmd[0] == "git":
                return real_run(cmd, cwd=cwd, env=env, **kw)
            agent = Path(env["DAVINCI_CODING_AGENT_DIR"])
            seen.append((agent, (agent / "auth.json").exists()))
            return subprocess.CompletedProcess(cmd, 0)

        with tempfile.TemporaryDirectory() as tmp:
            source = Path(tmp, "src")
            source.mkdir()
            Path(source, "auth.json").write_text("{}", encoding="utf-8")
            out = Path(tmp, "results")
            args = Namespace(out=str(out), binary="davinci", repo=str(REPO), credential_source=str(source),
                             reps=1, arms=["on"], model="m", timeout=5, node="n", host="h")
            real = adoption.subprocess.run
            adoption.subprocess.run = fake_run
            try:
                with contextlib.redirect_stdout(io.StringIO()):
                    adoption.run(args)
            finally:
                adoption.subprocess.run = real
            self.assertTrue(seen and all(has_auth for _, has_auth in seen))
            for agent, _ in seen:
                self.assertFalse(str(agent).startswith(str(out)), "agent dir is outside --out")
                self.assertFalse(agent.exists(), "temp credentials are removed afterwards")
            self.assertEqual([p for p in out.rglob("auth.json")], [])
            self.assertTrue((out / "commit.txt").exists())

    def test_the_default_node_matches_the_platform(self):
        node = adoption.default_node()
        self.assertEqual(node.name, "node.exe" if os.name == "nt" else "node")

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
