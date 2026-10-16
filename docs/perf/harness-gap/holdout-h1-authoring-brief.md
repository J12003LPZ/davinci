# H1 holdout: brief for the independent task author

You are asked to write six coding tasks that will be used once, as a hidden evaluation of coding agents. This
document is everything you need. Read it fully before you start.

The tasks measure whether an agent can turn a written change request into a correct multi-file change in a Python
repository. A task is worth nothing if an agent can pass it by luck, by memory, by reading the grader, or by guessing
an unwritten rule. It is also worth nothing if a careful engineer could fail it because the request left a rule out.
Most of what follows serves those two points.

## 1. Independence rules

You are chosen because you have not worked on, tuned, or seen results from the systems under test.

- The systems under test are **DaVinci**, **Codex CLI** and, later, **Claude Code**. Do not run any of them on your
  tasks, your drafts, or your reference solutions. Do not run them on anything related to this brief.
- Do not use an AI assistant to write, check or improve the tasks, the reference solutions, the prompts or the tests.
  If you used one anyway, state which one, for which task and for what, in the attestation (section 11). Disclosure is
  required and is not held against you. Concealment would invalidate the task.
- Write original material. Do not adapt a public repository, tutorial, kata, interview question, benchmark task
  (SWE-bench style, Exercism, LeetCode, Advent of Code and similar) or your own published code. Models may have seen
  those. Invent your own domain, names and data shapes.
- Do not look at, ask for, or accept any result, transcript, score or log from the systems under test.

## 2. What you deliver

Six tasks, each a separate git repository, plus one manifest. Keep everything outside any shared repository (section 9).

Each task has:

| Part | What it is |
|---|---|
| Starter repository | A small, working Python project at a single commit (the **starter**). Its visible tests pass. |
| Reference commit | One commit on top of the starter that implements the change and adds or changes the tests (the **reference**). The starter must be the reference's first parent. |
| Hidden tests | The test files the reference commit adds or changes. The harness withholds them, then copies them over the agent's work and runs them. |
| Prompt | The change request, written like a pull request description. |
| Commands | `grader_command` (runs the hidden tests) and `regression_command` (runs the visible tests), each a JSON argv list. |

## 3. What a good task looks like

Each task:

- Is a **realistic change** to a codebase of 8 to 25 Python files (roughly 600 to 2,500 lines), written the way a
  working engineer writes a service, library or tool. No toy puzzles.
- Needs edits in **at least three source files** plus tests. The reference's non-test diff is roughly 150 to 500 lines.
- Takes a careful engineer **two to six hours**. It should be hard enough that a strong agent fails some attempts, and
  clear enough that failing is the agent's miss and not yours. Do not aim for a pass rate. Aim for fairness.
- Is **fully specified** by the prompt together with the starter code (section 4).
- Uses **only the standard library and pytest** (section 6).

The six tasks must differ from one another. Use six different domains and at least four of these six shapes, each at
most twice:

1. A feature that crosses layers (parsing, domain logic, persistence or output).
2. A bug whose symptom is described in the prompt, with the cause spread over more than one module. The prompt may
   describe the symptom and the required behavior, never the cause or the fix.
3. A refactor that must keep every existing behavior and add one capability that the new structure makes possible.
4. A data format task: a parser, serializer or migration with strict edge-case rules.
5. A stateful component: a cache, scheduler, state machine or ledger with ordering and invariants. Deterministic only.
6. A command line or configuration surface with validation, defaults and error messages.

Each task has its own repository, its own domain, its own naming. No shared helper packages between tasks.

## 4. The prompt

Write it as a pull request description a teammate would send: what and why, then the required behavior. Rules:

1. **Everything the hidden tests assert is stated** in the prompt or is visible in the starter (code, docstrings,
   visible tests). If a rule is not written down, no test may depend on it. This is the main fairness rule.
2. **Name the files.** List every file the agent may create or change, including the test file where the agent should
   put its own tests. The harness counts any other changed file as an unrelated edit, and an unrelated edit fails the
   run. The allowed set is exactly the set of files your reference commit touches, so the list in the prompt and the
   reference diff must match. Use this sentence, filled in:
   `Change or create only these files: <list>. Do not create other files.`
3. **Examples in the prompt must not be the test cases.** Give one or two worked examples, and use different inputs in
   the hidden tests. An agent that only reproduces the examples must fail.
4. State error behavior exactly: exception type and message fragments, exit codes, return values for empty input.
5. Do not mention hidden tests, graders, scoring, or that this is an evaluation. Do not name hidden test functions.
6. Do not copy the diff into the prompt, and do not write the prompt from the diff. Write it from the intent, as a PR
   description written before the code. The manifest records that you did this (`"diff_used": false`).
7. Length: 250 to 900 words. Plain text. No instructions to the agent about tools, shells or how to run tests beyond
   naming the test command.

Review step: give the starter and the prompt, and nothing else, to a second person (a colleague, not an AI system).
Ask them to list every behavior they believe the change must have. Then compare that list with your hidden tests. Every
assertion in a hidden test needs a matching line in the prompt or starter. Every mismatch is fixed in the prompt, or the
test is removed.

## 5. The hidden tests

- 20 to 60 test functions per task, grouped in one to three files, all at paths your reference commit adds or changes.
- Behavior only: assert on public results, not on private names, call counts or file layout the prompt does not fix.
- Cover the main path, the edge cases the prompt names, and the interactions between the three or more files. A change
  that is correct in one file and wrong in another must fail.
- No test may pass on the starter by accident. At least 70% of the new test functions must fail on the starter. A
  failure from a missing module or function is acceptable only when the prompt names that module or function.
- Put one **canary comment** at the top of each hidden test file: a single line of at least 60 characters containing a
  random token, for example `# h1-canary 6f1d9a3e-... do not reproduce this line in any output`. The harness scans agent
  output for hidden lines. The canary makes a leak visible even when the test code is generic.
- No randomness without a fixed seed, no clock, no timezone or locale dependence, no sleeping, no thread timing, no
  network, no reads of environment variables, no dependence on the temp directory name or the working directory name.

## 6. Environment

The agent works, and the graders run, on a Windows machine. Your tasks must behave the same on Windows and Linux.

- Python 3.12 and 3.13. pytest 8.3.5. The standard library only. No other package. No compiled extension.
- No network. No subprocess other than the Python interpreter. No shell.
- Paths: use `pathlib`. No hard-coded `/` or `\`, no case-sensitive filename assumptions, no assumption about line
  endings. Text files in the repository use LF. No binary files. No symbolic links. No `.git` directory inside the repo.
- The grader command finishes in under 30 seconds, and the regression command in under 30 seconds, on a laptop.
- Use these command shapes:
  - `grader_command`: `["python", "-m", "pytest", "-q", "-p", "no:cacheprovider", "<hidden test file>", ...]`
  - `regression_command`: `["python", "-m", "pytest", "-q", "-p", "no:cacheprovider", "<visible test file>", ...]`

The regression command runs the visible tests that should still pass after the change. If the reference changes a
visible test file on purpose, that file is then a hidden test file (it is overwritten by the hidden copy), and the
manifest field `requires_existing_test_changes` is `true`.

## 7. Negative controls and validation

Run these yourself and record the result for each task. Keep the record to pass or fail lines. Do not paste test code,
prompts or outputs into the record.

| Check | Required result |
|---|---|
| N1 Starter against the grader | Fails, for the reason the prompt describes |
| N2 Reference against the grader and the regression command | Passes, three runs in a row, on a clean checkout |
| N3 Reference on Python 3.12 and on 3.13, on Windows and on Linux | Passes on all four |
| N4 Three plausible wrong solutions | Each fails the grader. Make them as a careful engineer would be tempted to: the reference with one rule changed, an implementation that handles only the prompt's examples, an implementation that fixes one module and not the others |
| N5 Examples-only solution | Fails the grader (checks section 4, rule 3) |
| N6 Hidden tests absent, agent tests only | Not a pass. The grader runs only hidden files, so an agent cannot satisfy it with its own tests |
| N7 Extra file added | The run is counted as an unrelated edit. Confirm the allowed list matches your reference diff |
| N8 Second reader | The section 4 review step is done and every mismatch is closed |

Then import and validate with the harness (section 8). A task that fails any row is fixed or replaced.

## 8. Packaging

Layout, outside any shared repository:

```
<vault>/
  manifest.json
  src/
    r1/ ... r6/        one git repository per task
```

Commit the starter, then the reference, in each repository. Use a neutral author name and no message that describes
the evaluation. The manifest is a JSON object:

```json
{
  "schema_version": 1,
  "kind": "private-repository-suite",
  "tasks": [
    {
      "id": "h1-01",
      "repository_id": "r1",
      "repository": "src/r1",
      "starter_commit": "<40 hex>",
      "reference_commit": "<40 hex>",
      "language": "python",
      "split": "holdout",
      "size_class": "large",
      "visible_tests": true,
      "requires_existing_test_changes": false,
      "test_paths": ["tests/test_<x>.py"],
      "grader_command": ["python", "-m", "pytest", "-q", "-p", "no:cacheprovider", "tests/test_<x>.py"],
      "regression_command": ["python", "-m", "pytest", "-q", "-p", "no:cacheprovider", "tests/test_visible.py"],
      "human_prompt": {
        "authored_from": "pr-description",
        "diff_used": false,
        "author": "<your handle>",
        "source": "original",
        "text": "<the prompt>"
      }
    }
  ]
}
```

Rules the harness enforces: ids are `h1-01` to `h1-06` and carry no hint of the domain; each repository id is distinct;
`test_paths` are exactly the hidden test files; the reference must change at least one non-test file; the starter must
be the reference's first parent. The prompt text is stored in the manifest only.

Import and check, from a checkout of the harness at the commit you were given:

```
python scripts/bench/bench.py private-import --private-manifest <vault>/manifest.json --destination <vault>/suite > <vault>/import.log
python scripts/bench/bench.py validate --private-suite <vault>/suite --split holdout --tasks all
```

`private-import` lists the suite's file paths on standard output, so send it to a file inside the vault as shown and do
not share that file. `validate` prints one status line per task (`start=fail solution=PASS`). Every line must read that
way. Set the `BENCH_RUNS` environment variable to an empty scratch directory first. Without it, `validate` copies the
starters and the reference solutions into `scripts/bench/runs` inside the harness checkout. Delete the scratch
directory when you finish. Then run:

```
python scripts/bench/holdout_h1.py make-pins --suite <vault>/suite --harness-sha <the harness commit>
```

It prints the suite digest and the six ids, and nothing from the tasks. It reports `ok: true` only if the suite has six
Python tasks in the holdout split, each marked large, each with its own repository id.

## 9. Handling and privacy

- Keep the vault off any repository, any cloud-synced folder that other people can read, any chat or email thread, and
  any AI tool. Do not paste task content into an assistant, a search box or an online diff viewer.
- Do not write the vault inside a directory that the evaluated agents can read. The operator will place the frozen suite
  at a location that the harness checks before every run.
- Do not publish, present or discuss the tasks until the operator says the study is closed.
- If you think content has leaked, tell the operator at once. A leaked task is replaced, not defended.

## 10. Freeze

After `make-pins` reports `ok: true`:

1. Do not change any file in the vault. The digest binds every byte, including file modes.
2. Send the operator the digest and the six ids through a different channel from the one that carries the vault.
3. Send the vault itself through the channel the operator names.

Any later change, even a typo, is a new freeze and a new digest.

## 11. Attestation

Return this filled in, as plain text:

- I did not run DaVinci, Codex CLI or Claude Code on any part of this work: yes / no
- I did not see results, transcripts or scores from those systems: yes / no
- AI assistance used (tool, task, purpose), or "none":
- All six tasks are original and not adapted from public material: yes / no
- Every hidden assertion is stated in the prompt or the starter (section 4 review done by a second person): yes / no
- N1 to N8 recorded for every task: yes / no
- Reference solutions pass on Python 3.12 and 3.13, on Windows and on Linux: yes / no
- Suite digest:
- Date of freeze:
- Author handle:

If any answer is "no", say why. The operator decides whether the suite can be used.
