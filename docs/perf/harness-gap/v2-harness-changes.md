# Large-task harness changes after the Stage 1 pin

The Stage 1 campaign ran on harness commit `ed85b77a4c564d557538a9fc8197b1dda7487992`. That
pin, its manifest hash and its recorded results do not change. Every change below applies only
to campaigns started from a later harness commit, and each campaign records which one it used
(`runner_source` in `campaign.json`, plus `grading_env_version` in every grader evidence file).

## Grading environment v2: one Python text mode for agent and grader

| | Stage 1 harness (grading env v1) | Grading env v2 |
|---|---|---|
| Agent process | `PYTHONUTF8=1` | `PYTHONUTF8=1` |
| Grader process | host environment plus offline flags, no `PYTHONUTF8` | host environment plus offline flags plus `PYTHONUTF8=1` |
| Where it is set | two unrelated places | one constant, `runner.PYTHON_TEXT_ENV`, used by the agent environment, the no-campaign fallback and `private_suite.run_check` |
| Recorded | nothing | `grading_env_version` and `python_text_env` in each `*.grader.json` |

### Why

In Stage 1, DaVinci m1 rep 0 failed one grader check (14 of 15 hidden tests). The check,
`test_public_tests_pass`, runs the workspace's public test file, which the agent may edit. DaVinci
had added a test that prints `café` from a child process and reads it back with `text=True`, and
ran it four times with 7 of 7 passing in its own environment.

The grader inherited the launching shell's `PYTHONIOENCODING=utf-8:surrogateescape` and ran
without UTF-8 mode, so the child printed UTF-8 while the inner pytest decoded with the legacy
Windows code page, and the text came back as mojibake. A clean shell passes: both sides then use
the legacy code page. So the verdict depended on the launching shell, not on the agent.

### What was verified, offline

* `scripts/bench/tests/test_grading_environment.py` (7 tests). Without the fix, the behavioural
  tests fail whenever the host exports `PYTHONIOENCODING`; with it, they pass under a clean shell,
  a shell that exports it, and with `PYTHONUTF8` set to `0`, `1` or absent.
* A sensitivity guard shows the unmatched case failing and the matched case passing on a Windows
  host, so the other tests cannot pass vacuously. It skips on hosts that default to UTF-8.
* Supplementary regrade of temp copies of all 24 Stage 1 workspaces (the campaign directory was
  not touched):
  * the Stage 1 harness under a campaign-like shell reproduces all 24 recorded rows;
  * the fixed harness gives identical results under a clean shell and a campaign-like shell;
  * the two harnesses differ on one run only: m1 rep 0, 14 of 15 to 15 of 15.

### What this does not change

* The recorded Stage 1 result stays 11 of 12 for DaVinci and 12 of 12 for Codex. The regrade is a
  check on copies, not a replacement row.
* The evidence run for the Stage 1 harness pin, its manifest SHA-256 and the fixtures are untouched.
* Legacy (v1) task grading also takes the new environment. It only matters for tests whose result
  depends on the interpreter's text mode; ASCII-only tests are unaffected. Do not pool rows from
  campaigns with different `grading_env_version` without checking this first.

### Not fixed here

`large_task_failures.py` still keys its roll-up by requirement name only (see section 7 of
`v2-stage1-launch.md`). Changing it moves the harness pin again and is a separate decision.
