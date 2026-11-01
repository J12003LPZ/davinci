"""Task sources for the Stage 0 gap study beyond the generated public tasks.

Public tasks (``t1-intervals`` ...) keep their existing path in ``bench.py``.
This module adds the other two corpora of the 17-task study set:

* ``verified-*`` (6 tasks): ``crates/davinci-evals/fixtures/behavior/verified-core.json``.
  The prompt is the scenario ``request``; the workspace is a copy of
  ``fixtures/repos/<repo_fixture>``; grading runs each ``verification_commands``
  entry with its stated timeout and expected exit code.
* ``research-01..03``: the external research fixtures
  (``BENCH_RESEARCH_FIXTURES``). The prompt is the ``## Instruction`` section of
  ``tasks/<id>.md``, the workspace is made by ``setup.ps1`` and graded by
  ``grade.ps1 -Json`` through ``pwsh``.

Research layout. The verbatim instruction says "Write workspace/answer.json" and
names ``workspace/notes``, so the fixture is materialised at
``<run dir>/workspace`` (``setup.ps1 -Path`` that directory) while the agent runs
with cwd ``<run dir>``, where those ``workspace/...`` paths resolve. The git
baseline, the changed-files and unrelated-changes checks all use ``<run dir>``,
so paths are ``workspace/...``; the grader is called with
``-Workspace <run dir>/workspace``. That directory holds no ``.git``, so the
grader's "no extra files" check no longer depends on ``.git`` being a hidden
directory. (Stage 0 first ran the fixture at the run dir root and every research
task failed for that reason; see ``docs/perf/droid-gap/stage0-report.md``.)

Allowed (not "unrelated") changed paths, per source:

* verified: exactly the scenario's ``file_changed`` requirement paths, matching
  its own ``max_unrelated_files_changed: 0`` limit. ``target/`` and ``Cargo.lock``
  are build output and are excluded through ``.git/info/exclude``, not tracked.
* research: only the outputs the task asks for (``workspace/answer.json``,
  ``workspace/edit-report.json``, ``workspace/results.json``) plus, where the
  task edits or runs files, the exact families it names (``workspace/pkg/m*.ts``,
  ``workspace/pkg/__tests__/*``, ``workspace/jobs/out/*.txt``). The fixture's own
  ``.pre/`` setup artefacts are committed into the baseline, so they never show
  as changed. A change to ``workspace/pkg/other/*`` is deliberately NOT allowed:
  the grader asserts it untouched.

Nothing here makes a model call. Missing prerequisites fail loudly.
"""
import json
import os
import re
import shlex
import shutil
import subprocess
from pathlib import Path

from campaign import LEGACY_TASKS, digest, file_hash

REPO = Path(__file__).resolve().parents[2]
CORPUS = REPO / "crates" / "davinci-evals" / "fixtures" / "behavior" / "verified-core.json"
REPOS = REPO / "crates" / "davinci-evals" / "fixtures" / "repos"
DEFAULT_RESEARCH_ROOT = r"C:\Users\sergi\Desktop\lean-droid-1.0\droid-re-research\fixtures"

VERIFIED = ("verified-range", "verified-slice", "verified-parser",
            "verified-dedupe", "verified-errors", "verified-path")
RESEARCH = {"research-01": "01-read-many-filter",
            "research-02": "02-bulk-edit-verify",
            "research-03": "03-independent-parallel"}
# The fixture lives in <run dir>/workspace (see the module docstring), so every
# allowed path carries the "workspace/" prefix.
WORKSPACE_DIR = "workspace"
RESEARCH_ALLOWED = {
    "research-01": {"allowed": ["workspace/answer.json"], "allowed_globs": []},
    "research-02": {"allowed": ["workspace/edit-report.json"],
                    "allowed_globs": ["workspace/pkg/m*.ts", "workspace/pkg/__tests__/*"]},
    "research-03": {"allowed": ["workspace/results.json"], "allowed_globs": ["workspace/jobs/out/*.txt"]},
}
INSTRUCTION_HEADING = "## Instruction (give this verbatim)"
STUDY_TASKS = VERIFIED + tuple(LEGACY_TASKS) + tuple(RESEARCH)


def is_study_source(tid):
    """True for ids served by this module (not the generated public tasks)."""
    return tid in VERIFIED or tid in RESEARCH


def select_study(requested):
    """Resolve ``--tasks`` for the study: ``["study"]`` or explicit unique study ids.

    Returns None when ``requested`` does not name a study-source task, so the
    caller falls back to the normal ``select_tasks`` (``all`` stays public-only).
    """
    if requested == ["study"]:
        return list(STUDY_TASKS)
    if requested and any(is_study_source(task) for task in requested):
        if any(task not in STUDY_TASKS for task in requested) or len(set(requested)) != len(requested):
            raise ValueError("requested tasks must be unique members of the study task set")
        return list(requested)
    return None


# --- verified-core -----------------------------------------------------------

def _scenario(tid):
    scenarios = json.loads(Path(CORPUS).read_text(encoding="utf-8"))
    for scenario in scenarios:
        if scenario.get("id") == tid:
            return scenario
    raise RuntimeError("verified-core corpus has no scenario " + tid)


def _fixture_dir(scenario):
    return Path(REPOS) / scenario["repo_fixture"]


def _verified_spec(tid):
    scenario = _scenario(tid)
    allowed = [req["path"] for req in scenario.get("requirements", [])
               if req.get("type") == "file_changed" and req.get("path")]
    return {"prompt": scenario["request"], "allowed": allowed, "allowed_globs": [],
            "task_set": "study", "source": "verified-core"}


def _verified_files(tid):
    scenario = _scenario(tid)
    directory = _fixture_dir(scenario)
    if not directory.is_dir():
        raise RuntimeError("missing verified-core fixture directory: " + str(directory))
    files = {path.relative_to(directory).as_posix(): file_hash(path)
             for path in sorted(directory.rglob("*")) if path.is_file()}
    files["scenario.json"] = digest(scenario)
    return files


def _verified_copy(tid, dest):
    directory = _fixture_dir(_scenario(tid))
    if not directory.is_dir():
        raise RuntimeError("missing verified-core fixture directory: " + str(directory))
    for path in directory.rglob("*"):
        if path.is_file():
            out = Path(dest) / path.relative_to(directory)
            out.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(path, out)
    return ("target/", "Cargo.lock")


def _verified_grade(tid, workdir, timeout, phase, evidence, run_check_fn):
    commands = _scenario(tid).get("verification_commands") or []
    if not commands:
        raise RuntimeError("verified-core scenario has no verification command: " + tid)
    passed = failed = 0
    tail = ""
    for index, entry in enumerate(commands):
        limit = entry.get("timeout_seconds") or timeout
        expected = entry.get("expected_exit", 0)
        name = phase if len(commands) == 1 else f"{phase}-{index}"
        checked = run_check_fn(shlex.split(entry["command"]), workdir, limit, name, evidence)
        ok = checked["exit"] == expected and checked["cleanup_complete"] is True
        passed += ok
        failed += not ok
        lines = (checked["stdout"] or checked["stderr"]).strip().splitlines()
        tail = lines[-1] if lines else str(checked["exit"])
    return failed == 0, passed, failed, tail


# --- research fixtures ---------------------------------------------------------

def _research_root():
    root = Path(os.environ.get("BENCH_RESEARCH_FIXTURES") or DEFAULT_RESEARCH_ROOT)
    for name in ("setup.ps1", "grade.ps1", "tasks"):
        if not (root / name).exists():
            raise RuntimeError(f"research fixtures root is missing {name}: {root} "
                               "(set BENCH_RESEARCH_FIXTURES)")
    return root


def _pwsh():
    found = shutil.which(os.environ.get("BENCH_PWSH") or "pwsh")
    if found is None:
        raise RuntimeError("pwsh is required for the research fixtures and was not found "
                           "(set BENCH_PWSH to its path)")
    return found


def _research_prompt(tid):
    text = (_research_root() / "tasks" / (RESEARCH[tid] + ".md")).read_text(encoding="utf-8")
    if INSTRUCTION_HEADING not in text:
        raise RuntimeError("research task file has no instruction section: " + tid)
    section = text.split(INSTRUCTION_HEADING, 1)[1]
    section = re.split(r"^## ", section, maxsplit=1, flags=re.MULTILINE)[0]
    prompt = section.strip()
    if not prompt:
        raise RuntimeError("empty research instruction: " + tid)
    return prompt


def _research_spec(tid):
    return {"prompt": _research_prompt(tid), "task_set": "study", "source": "research-fixtures",
            **{key: list(value) for key, value in RESEARCH_ALLOWED[tid].items()}}


def _research_files(tid):
    root = _research_root()
    return {name: file_hash(root / name) for name in ("setup.ps1", "grade.ps1")} | {
        "tasks/" + RESEARCH[tid] + ".md": file_hash(root / "tasks" / (RESEARCH[tid] + ".md"))}


def _research_setup(tid, dest):
    root = _research_root()
    Path(dest).mkdir(parents=True, exist_ok=True)
    fixture = Path(dest) / WORKSPACE_DIR
    result = subprocess.run(
        [_pwsh(), "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(root / "setup.ps1"),
         "-Task", RESEARCH[tid], "-Path", str(fixture)],
        capture_output=True, text=True, encoding="utf-8", timeout=300)
    if result.returncode != 0:
        raise RuntimeError(f"research fixture setup failed for {tid}: exit {result.returncode}")
    return ()


def _research_grade(tid, workdir, timeout, phase, evidence, run_check_fn):
    root = _research_root()
    checked = run_check_fn(
        [_pwsh(), "-NoProfile", "-ExecutionPolicy", "Bypass", "-File", str(root / "grade.ps1"),
         "-Task", RESEARCH[tid], "-Workspace", str(Path(workdir) / WORKSPACE_DIR), "-Json"],
        workdir, timeout, phase, evidence)
    if checked["exit"] == 2:
        raise RuntimeError(f"research grader rejected the invocation for {tid} (exit 2)")
    text = checked["stdout"]
    start = text.find("{")
    try:
        report = json.JSONDecoder().raw_decode(text[start:])[0] if start >= 0 else None
    except ValueError:
        report = None
    if not isinstance(report, dict) or type(report.get("pass")) is not bool:
        raise RuntimeError(f"research grader produced no verdict for {tid} (exit {checked['exit']})")
    passed, total = report.get("passed"), report.get("total")
    passed = passed if type(passed) is int else int(report["pass"])
    failed = total - passed if type(total) is int else int(not report["pass"])
    return (report["pass"] and checked["cleanup_complete"] is True, passed, failed,
            f"{passed}/{total} assertions")


# --- dispatch --------------------------------------------------------------------

def load_spec(tid):
    return _verified_spec(tid) if tid in VERIFIED else _research_spec(tid)


def fixture_files(tid):
    """Relative path -> SHA-256 for the inputs that define the task."""
    return _verified_files(tid) if tid in VERIFIED else _research_files(tid)


def prepare(tid, dest, baseline):
    """Create the workspace at ``dest`` and call ``baseline(dest, exclude)``."""
    dest = Path(dest)
    if dest.exists():
        shutil.rmtree(dest)
    if tid in VERIFIED:
        dest.mkdir(parents=True)
        exclude = _verified_copy(tid, dest)
    else:
        exclude = _research_setup(tid, dest)  # setup.ps1 recreates <dest>/workspace itself
        if not (dest / WORKSPACE_DIR).is_dir():
            raise RuntimeError("research setup did not create the workspace: " + str(dest / WORKSPACE_DIR))
    baseline(str(dest), exclude=exclude)


def grade(tid, workdir, timeout, phase, evidence, run_check_fn):
    if tid in VERIFIED:
        return _verified_grade(tid, workdir, timeout, phase, evidence, run_check_fn)
    return _research_grade(tid, workdir, timeout, phase, evidence, run_check_fn)
