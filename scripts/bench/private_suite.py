"""Private repository fixtures. No provider calls and no prompt generation.

The operator supplies reviewed commits, test paths, commands and human prompts.
These inputs are trusted executable code, not an untrusted archive service.
"""
import json
import os
from pathlib import Path, PurePosixPath
import re
import shutil
import subprocess
import tempfile

from campaign import digest, fixture_manifest
from runner import execute


def relative_path(value):
    if (not isinstance(value, str) or not value or "\\" in value
            or ":" in value or any(ord(ch) < 32 for ch in value)
            or PurePosixPath(value).is_absolute()
            or any(part.lower() in ("", ".", "..", ".git") for part in value.split("/"))):
        raise ValueError("expected a safe repository-relative path")
    return value


def argv(value):
    if not isinstance(value, list) or not value or any(not isinstance(v, str) or not v for v in value):
        raise ValueError("test command must be a nonempty argv list")
    return value


def git(repo, *args):
    result = subprocess.run(["git", "-C", str(repo), *args], capture_output=True, check=True)
    return result.stdout


def read_suite(path):
    suite = json.loads(Path(path).read_text(encoding="utf-8"))
    if not isinstance(suite, dict) or suite.get("schema_version") != 1 or suite.get("kind") != "private-repository-suite":
        raise ValueError("unsupported private suite schema")
    tasks = suite.get("tasks")
    if not isinstance(tasks, list) or not tasks:
        raise ValueError("private suite needs tasks supplied by its owner")
    ids, revisions = set(), set()
    for task in tasks:
        if not isinstance(task, dict):
            raise ValueError("private task must be an object")
        tid = task.get("id")
        if not isinstance(tid, str) or not re.fullmatch(r"[a-zA-Z0-9][a-zA-Z0-9_-]*", tid) or tid in ids:
            raise ValueError("invalid or duplicate private task id")
        ids.add(tid)
        for field in ("repository_id", "repository", "reference_commit", "starter_commit", "language"):
            if not isinstance(task.get(field), str) or not task[field]:
                raise ValueError(f"{tid}: missing {field}")
        for field in ("reference_commit", "starter_commit"):
            if not re.fullmatch(r"[0-9a-f]{40}", task[field]):
                raise ValueError(f"{tid}: {field} must be a full immutable commit")
        revision = (task["repository_id"], task["reference_commit"])
        if revision in revisions:
            raise ValueError("a source revision cannot appear in multiple tasks or splits")
        revisions.add(revision)
        if task.get("split") not in ("dev", "holdout") or task.get("size_class") not in ("small", "large"):
            raise ValueError(f"{tid}: missing split or size class")
        for field in ("visible_tests", "requires_existing_test_changes"):
            if type(task.get(field)) is not bool:
                raise ValueError(f"{tid}: {field} must be boolean")
        prompt = task.get("human_prompt", {})
        if (not isinstance(prompt, dict) or prompt.get("authored_from") != "pr-description"
                or prompt.get("diff_used") is not False
                or any(not isinstance(prompt.get(key), str) or not prompt[key].strip()
                       for key in ("text", "author", "source"))):
            raise ValueError(f"{tid}: human prompt and provenance attestation required")
        paths = task.get("test_paths")
        if not isinstance(paths, list) or not paths or any(not isinstance(p, str) for p in paths) or len(set(paths)) != len(paths):
            raise ValueError(f"{tid}: unique changed test paths required")
        for value in paths:
            relative_path(value)
        argv(task.get("grader_command"))
        argv(task.get("regression_command"))
    return suite


def suite_inventory(suite):
    tasks = suite["tasks"]
    counts = {split: sum(task["split"] == split for task in tasks) for split in ("dev", "holdout")}
    repos = sorted({task["repository_id"] for task in tasks})
    return {"tasks": len(tasks), "splits": counts, "repositories": repos,
            "languages": sorted({task["language"] for task in tasks}),
            "target_sizes_met": 3 <= len(repos) <= 5 and 40 <= counts["dev"] <= 60 and counts["holdout"] >= 150}


def export_tree(repo, commit, destination):
    """Read committed blobs directly; never run checkout filters or repository hooks."""
    destination.mkdir(parents=True)
    for entry in git(repo, "ls-tree", "-rz", commit).split(b"\0"):
        if not entry:
            continue
        metadata, encoded_path = entry.split(b"\t", 1)
        mode, kind, oid = metadata.split()
        name = relative_path(encoded_path.decode("utf-8"))
        if kind != b"blob" or mode not in (b"100644", b"100755"):
            raise ValueError(f"unsupported linked/submodule fixture path: {name}")
        path = destination / name
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_bytes(git(repo, "cat-file", "blob", oid.decode("ascii")))
        path.chmod(0o755 if mode == b"100755" else 0o644)


def import_suite(manifest, destination):
    """Freeze reviewed parent/reference snapshots and changed tests outside Git."""
    manifest, destination = Path(manifest).resolve(), Path(destination).resolve()
    if destination.exists():
        raise ValueError("private fixture output must be a new directory")
    repository = Path(__file__).resolve().parents[2]
    if destination.is_relative_to(repository):
        raise ValueError("keep private fixtures outside the runner checkout")
    suite = read_suite(manifest)
    # Validate provenance before creating any output.
    sources, actual_revisions = [], set()
    for task in suite["tasks"]:
        repo = (manifest.parent / task["repository"]).resolve()
        reference, starter = task["reference_commit"], task["starter_commit"]
        parent = git(repo, "rev-parse", reference + "^1").decode().strip()
        if parent != starter:
            raise ValueError(task["id"] + ": starter is not the reference's first parent")
        touched = git(repo, "diff", "--name-only", "--no-renames", "-z", starter, reference).decode().split("\0")
        touched = sorted(relative_path(path) for path in touched if path)
        if not set(task["test_paths"]).issubset(touched) or not set(touched) - set(task["test_paths"]):
            raise ValueError(task["id"] + ": reference must change source and declared grader tests")
        actual_repo = git(repo, "rev-parse", "--show-toplevel").decode().strip()
        actual_revision = (str(Path(actual_repo).resolve()), reference)
        if actual_revision in actual_revisions:
            raise ValueError("repository aliases cannot duplicate a revision across splits")
        actual_revisions.add(actual_revision)
        sources.append((task, Path(actual_repo).resolve(), touched))
    destination.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="private-suite-", dir=destination.parent) as temporary:
        root = Path(temporary)
        for task, repo, touched in sources:
            target = root / task["id"]
            target.mkdir()
            export_tree(repo, task["starter_commit"], target / "repo")
            export_tree(repo, task["reference_commit"], target / "solution")
            hidden = target / "hidden"
            hidden.mkdir()
            deleted = []
            for name in task["test_paths"]:
                source = target / "solution" / name
                if source.is_file():
                    output = hidden / name
                    output.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copy2(source, output)
                else:
                    deleted.append(name)
            # Source path is deliberately omitted from portable campaign metadata.
            provenance = {key: value for key, value in task.items() if key not in ("repository", "human_prompt")}
            provenance["human_prompt"] = {key: value for key, value in task["human_prompt"].items() if key != "text"}
            provenance["prompt_sha256"] = digest(task["human_prompt"]["text"])
            spec = {"prompt": task["human_prompt"]["text"], "allowed": touched,
                    "private_provenance": provenance, "hidden_deleted_paths": deleted,
                    "grader_command": task["grader_command"], "regression_command": task["regression_command"]}
            (target / "task.json").write_text(json.dumps(spec, indent=2), encoding="utf-8")
        frozen = fixture_manifest(root, [task["id"] for task in suite["tasks"]])
        frozen.update(kind="private-repository-suite", inventory=suite_inventory(suite),
                      labels={task["id"]: {key: task[key] for key in
                              ("repository_id", "reference_commit", "split", "size_class", "visible_tests",
                               "requires_existing_test_changes", "language")} for task in suite["tasks"]},
                      promotion_eligible=False,
                      boundary="private inputs do not establish an independent grading boundary")
        frozen["file_modes"] = fixture_modes(root, frozen["tasks"])
        (root / "suite.json").write_text(json.dumps(frozen, indent=2), encoding="utf-8")
        # Directory rename publishes the complete suite, never a partial export.
        root.rename(destination)
    return frozen


def load_frozen(root, split):
    root = Path(root)
    frozen = json.loads((root / "suite.json").read_text(encoding="utf-8"))
    if frozen.get("kind") != "private-repository-suite" or split not in ("dev", "holdout"):
        raise ValueError("private tasks require an explicit dev or holdout split")
    current = fixture_manifest(root, frozen["tasks"])
    if current != {key: frozen[key] for key in current}:
        raise ValueError("private fixture bytes changed after freezing")
    if fixture_modes(root, frozen["tasks"]) != frozen.get("file_modes"):
        raise ValueError("private fixture executable modes changed after freezing")
    tasks = [task for task in frozen["tasks"] if frozen["labels"][task]["split"] == split]
    if not tasks:
        raise ValueError("selected private split is empty")
    # Bind labels too: changing split/size/provenance must change campaign identity.
    return frozen, tasks, digest(frozen)


def fixture_modes(root, tasks):
    modes = {}
    for task in tasks:
        for directory in ("repo", "solution", "hidden"):
            for path in sorted((Path(root) / task / directory).rglob("*")):
                if path.is_symlink():
                    raise ValueError("linked private fixture paths are unsupported")
                if path.is_file():
                    modes[path.relative_to(root).as_posix()] = bool(path.stat().st_mode & 0o111)
    return modes


def apply_hidden(spec, hidden, workdir):
    """Overlay only declared changed tests, including deletion, after measurements."""
    workdir = Path(workdir)
    for name in spec.get("hidden_deleted_paths", []):
        target = workdir / relative_path(name)
        relative = target.relative_to(workdir)
        if any((workdir / Path(*relative.parts[:count])).is_symlink() for count in range(1, len(relative.parts) + 1)):
            raise ValueError("candidate linked a private grader path")
        if target.exists():
            if not target.is_file():
                raise ValueError("candidate replaced a grader file with a directory")
            target.unlink()
    for source in Path(hidden).rglob("*"):
        if source.is_file():
            name = source.relative_to(hidden).as_posix()
            target = workdir / relative_path(name)
            relative = target.relative_to(workdir)
            if any((workdir / Path(*relative.parts[:count])).is_symlink() for count in range(1, len(relative.parts) + 1)):
                raise ValueError("candidate linked a private grader path")
            target.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(source, target)


def run_check(command, workdir, timeout=120):
    env = dict(os.environ, DAVINCI_OFFLINE="1", PI_OFFLINE="1", PYTHONDONTWRITEBYTECODE="1")
    result = execute(argv(command), workdir, env, timeout)
    return {"pass": result["exit"] == 0 and result.get("cleanup_complete") is True,
            "exit": result["exit"], "cleanup_complete": result.get("cleanup_complete"),
            "stdout": result["stdout"], "stderr": result["stderr"]}
