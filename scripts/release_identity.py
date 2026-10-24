#!/usr/bin/env python3
"""Bind installed/benchmarked bytes to clean source and completed green CI.

Only the preflight, verify-commit and publish-tag commands contact GitHub, through
the existing gh authentication. Export snapshots and local identities do not
satisfy release gates.
"""
import argparse
from datetime import datetime, timezone
import hashlib
import json
import os
from pathlib import Path
import platform
import re
import subprocess
import tomllib
from urllib.parse import quote

CLEAN_DIFF_HASH = hashlib.sha256(b"").hexdigest()
PACKAGES = ("davinci-agent", "davinci-ai", "davinci-client", "davinci-coding-agent",
            "davinci-evals", "davinci-mcp", "davinci-protocol", "davinci-server",
            "davinci-session", "davinci-session-sqlite", "davinci-sys", "davinci-telemetry",
            "davinci-tui")
EXPECTED_CI_JOBS = {"quality", "contracts", "workspace-tests"} | {
    f"workspace-test ({os}, {package})"
    for os in ("ubuntu-latest", "windows-latest") for package in PACKAGES
} | {f"{job} ({os})" for job in ("test-impact-native", "p12-live-browser")
     for os in ("ubuntu-latest", "windows-latest", "macos-latest")} | {
    "python-tests (ubuntu-latest)", "python-tests (windows-latest)"}


def file_hash(path):
    hasher = hashlib.sha256()
    with Path(path).open("rb") as source:
        for chunk in iter(lambda: source.read(1024 * 1024), b""):
            hasher.update(chunk)
    return hasher.hexdigest()


# A replace ref would let a candidate substitute objects the verifier reads.
GIT_ENV = {**os.environ, "GIT_NO_REPLACE_OBJECTS": "1", "GIT_TERMINAL_PROMPT": "0"}
COMMIT_SHA = re.compile(r"[0-9a-f]{40}")
SEMVER = re.compile(r"(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)"
                    r"(-((0|[1-9][0-9]*|[0-9]*[a-zA-Z-][0-9a-zA-Z-]*)(\.(0|[1-9][0-9]*|[0-9]*[a-zA-Z-][0-9a-zA-Z-]*))*))?"
                    r"(\+[0-9a-zA-Z-]+(\.[0-9a-zA-Z-]+)*)?")
REPOSITORY = re.compile(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+")
BRANCH = re.compile(r"[A-Za-z0-9_.][A-Za-z0-9_./-]{0,254}")


def git(repo, *args):
    result = subprocess.run(["git", *args], cwd=repo, check=True, capture_output=True, text=True, env=GIT_ENV)
    return result.stdout.strip()


def source_identity(repo):
    try:
        status = git(repo, "status", "--porcelain=v1", "--untracked-files=all")
    except subprocess.CalledProcessError as error:
        raise ValueError("release/benchmark source requires verifiable committed Git provenance") from error
    if status:
        raise ValueError("release/benchmark source must be a clean committed checkout")
    return {"source_sha": git(repo, "rev-parse", "HEAD"),
            "source_tree": git(repo, "rev-parse", "HEAD^{tree}"),
            "source_clean": True, "dirty_diff_hash": CLEAN_DIFF_HASH}


# Export identities are local evidence, never a replacement for the release gate.
SNAPSHOT_EXCLUDES = frozenset({".git", "target", "node_modules", "__pycache__",
                              ".venv", ".superpowers", ".pytest_cache"})
NESTED_EXCLUDES = frozenset({".git", "node_modules", "__pycache__", ".pytest_cache"})


def export_snapshot(repo):
    root = Path(repo).resolve(strict=True)
    files = {}
    for directory, names, filenames in os.walk(root, followlinks=False):
        excluded = SNAPSHOT_EXCLUDES if Path(directory) == root else NESTED_EXCLUDES
        names[:] = sorted(name for name in names if name not in excluded
                          and name != ".env" and not name.startswith(".env."))
        for name in names + sorted(filenames):
            path = Path(directory) / name
            if name == ".env" or name.startswith(".env."):
                continue
            if path.is_symlink() or (hasattr(path, "is_junction") and path.is_junction()):
                raise ValueError("source snapshot cannot follow linked paths")
            if path.is_file():
                files[path.relative_to(root).as_posix()] = file_hash(path)
    if not files:
        raise ValueError("source snapshot is empty")
    files = dict(sorted(files.items()))
    digest = hashlib.sha256(json.dumps(files, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
    return {"source_kind": "exported_snapshot", "source_sha": None,
            "snapshot_sha256": digest, "files": files,
            "excluded_root": sorted(SNAPSHOT_EXCLUDES),
            "excluded_anywhere": sorted(NESTED_EXCLUDES | {".env", ".env.*"})}


def local_build_identity(repo, binary, before, configuration, build_features):
    """Bind a caller-observed local build to unchanged content; no CI attestation."""
    if export_snapshot(repo) != before:
        raise ValueError("source changed during the local build")
    binary = Path(binary).resolve(strict=True)
    return {"schema_version": 1, "identity_kind": "local_build", "source": before,
            "release_eligible": False, "binary_sha256": file_hash(binary),
            "launch_path": str(binary), "build_features": sorted(set(build_features)),
            "platform": platform.platform(),
            "configuration_sha256": hashlib.sha256(json.dumps(
                configuration, sort_keys=True, separators=(",", ":"), allow_nan=False).encode()).hexdigest()}


def require_same_source(repo, before):
    if source_identity(repo) != before:
        raise ValueError("source changed after CI preflight or during the build")


def product_version(repo):
    text = (Path(repo) / "Cargo.toml").read_text(encoding="utf-8")
    section = re.search(r"(?ms)^\[workspace\.package\]\s*(.*?)(?=^\[|\Z)", text)
    version = re.search(r'^version\s*=\s*"([0-9]+\.[0-9]+\.[0-9]+)"', section[1], re.M) if section else None
    if not version:
        raise ValueError("workspace product version is unavailable")
    return version[1]


def latest_run(runs, sha):
    matches = [run for run in runs if isinstance(run, dict) and run.get("head_sha") == sha]
    if not matches:
        raise ValueError("no CI run exists for this exact source commit")
    return max(matches, key=lambda run: (run.get("created_at", ""), run.get("id", 0), run.get("run_attempt", 0)))


def validate_ci(record, repository, sha, branch=None):
    if (not isinstance(record, dict) or record.get("repository") != repository
            or record.get("source_sha") != sha or not re.fullmatch(r"[0-9a-f]{40}", sha)
            or record.get("event") != "push"
            or record.get("workflow_path") != ".github/workflows/ci.yml"
            or type(record.get("ci_run")) is not int or record["ci_run"] <= 0
            or record.get("ci_url") != f"https://github.com/{repository}/actions/runs/{record['ci_run']}"):
        raise ValueError("CI proof is not bound to the exact repository and source commit")
    if branch is not None and record.get("head_branch") != branch:
        raise ValueError(f"CI proof does not come from a push to {branch}")
    if record.get("status") != "completed" or record.get("conclusion") != "success":
        raise ValueError("the latest full CI run for this commit is not green")
    jobs = record.get("jobs")
    if not isinstance(jobs, list) or not all(isinstance(job, dict) for job in jobs):
        raise ValueError("CI job evidence is missing")
    names = [job.get("name") for job in jobs]
    if len(names) != len(set(names)) or not EXPECTED_CI_JOBS.issubset(names):
        raise ValueError("full CI evidence does not contain every required shard and quality job")
    if any(job.get("status") != "completed" or job.get("conclusion") != "success" for job in jobs):
        raise ValueError("CI contains a failed, pending, cancelled or skipped job")
    lint = record.get("workflow_lint")
    if (not isinstance(lint, dict) or lint.get("status") != "completed"
            or lint.get("conclusion") != "success" or type(lint.get("run_id")) is not int):
        raise ValueError("workflow lint evidence is missing or not green for this commit")


def api_pages(endpoint):
    result = subprocess.run(["gh", "api", "--paginate", "--slurp", endpoint],
                            check=True, capture_output=True, text=True, timeout=120)
    value = json.loads(result.stdout)
    if not isinstance(value, list) or not all(isinstance(page, dict) for page in value):
        raise ValueError("invalid GitHub CI response")
    return value


def fetch_ci(repository, sha, branch=None):
    """CI evidence for a commit. With `branch`, only push runs of that branch count."""
    if not REPOSITORY.fullmatch(repository):
        raise ValueError("invalid repository name")
    if branch is not None and not BRANCH.fullmatch(branch):
        raise ValueError("invalid branch name")
    prefix = f"repos/{repository}/actions"
    # PR workflows check out a synthetic merge by default; head_sha alone
    # cannot prove these exact source bytes were tested. Require push CI.
    query = f"head_sha={sha}&event=push&per_page=100" + (f"&branch={quote(branch, safe='')}" if branch else "")

    def runs_of(workflow):
        return [run for page in api_pages(f"{prefix}/workflows/{workflow}/runs?{query}")
                for run in page.get("workflow_runs", [])
                if branch is None or (run.get("head_branch") == branch and run.get("event") == "push")]

    run = latest_run(runs_of("ci.yml"), sha)
    jobs = [job for page in api_pages(f"{prefix}/runs/{run['id']}/jobs?filter=latest&per_page=100")
            for job in page.get("jobs", [])]
    lint = latest_run(runs_of("workflow-lint.yml"), sha)
    record = {"repository": repository, "source_sha": sha, "ci_run": run["id"],
              "ci_url": run["html_url"], "workflow_path": run.get("path"),
              "status": run["status"], "conclusion": run["conclusion"],
              "event": run["event"],
              "jobs": [{key: job[key] for key in ("name", "status", "conclusion")} for job in jobs],
              "workflow_lint": {"run_id": lint["id"], "status": lint["status"], "conclusion": lint["conclusion"]}}
    if branch is not None:
        record["head_branch"] = branch
    validate_ci(record, repository, sha, branch)
    return record


def git_bytes(repo, *args):
    result = subprocess.run(["git", *args], cwd=repo, check=True, capture_output=True, env=GIT_ENV)
    return result.stdout


def committed_version(repo, tree):
    """Workspace version from the Cargo.toml blob of a tree; the worktree is never read."""
    entry = git_bytes(repo, "ls-tree", "-z", tree, "Cargo.toml").split(b"\0")[0]
    meta, _, name = entry.partition(b"\t")
    fields = meta.split()
    if name != b"Cargo.toml" or len(fields) != 3 or fields[0] != b"100644" or fields[1] != b"blob":
        raise ValueError("candidate Cargo.toml must be a regular file")
    blob = fields[2].decode("ascii")
    if not COMMIT_SHA.fullmatch(blob):
        raise ValueError("candidate Cargo.toml has an unexpected object id")
    try:
        manifest = tomllib.loads(git_bytes(repo, "cat-file", "blob", blob).decode("utf-8"))
        version = manifest["workspace"]["package"]["version"]
    except (UnicodeDecodeError, tomllib.TOMLDecodeError, KeyError, TypeError) as error:
        raise ValueError("candidate workspace product version is unavailable") from error
    if not isinstance(version, str) or not SEMVER.fullmatch(version):
        raise ValueError("candidate workspace version is not strict semver")
    return version


def verify_commit(repo, commit, repository, default_branch, main_commit, expected_version, fetch=fetch_ci):
    """Prove a candidate from the trusted checkout's object database alone.

    Nothing from the candidate is checked out or executed: its tree, version and
    ancestry are read with Git plumbing, and its CI must be push CI of the
    default branch. `main_commit` is the trusted commit the verifier came from.
    """
    for label, value in (("commit", commit), ("main commit", main_commit)):
        if not isinstance(value, str) or not COMMIT_SHA.fullmatch(value):
            raise ValueError(f"{label} must be a full lowercase 40-hex SHA")
    if not isinstance(expected_version, str) or not SEMVER.fullmatch(expected_version):
        raise ValueError("expected version must be strict semver")
    if not REPOSITORY.fullmatch(repository):
        raise ValueError("invalid repository name")
    if not BRANCH.fullmatch(default_branch):
        raise ValueError("invalid default branch name")
    repo = Path(repo).resolve()
    try:
        for label, value in (("candidate", commit), ("main", main_commit)):
            if git(repo, "cat-file", "-t", value) != "commit":
                raise ValueError(f"{label} object is not a commit")
        ancestor = subprocess.run(["git", "merge-base", "--is-ancestor", commit, main_commit],
                                  cwd=repo, capture_output=True, env=GIT_ENV)
        if ancestor.returncode != 0:
            raise ValueError(f"candidate is not an ancestor of {default_branch} at {main_commit}")
        header = git_bytes(repo, "cat-file", "commit", commit).split(b"\n", 1)[0].decode("ascii", "replace")
        tree = header.removeprefix("tree ")
        if not header.startswith("tree ") or not COMMIT_SHA.fullmatch(tree):
            raise ValueError("candidate commit object has no tree")
        version = committed_version(repo, tree)
    except subprocess.CalledProcessError as error:
        raise ValueError("candidate is not readable in the trusted object database") from error
    if version != expected_version:
        raise ValueError(f"candidate Cargo.toml version {version} does not match requested {expected_version}")
    ci = fetch(repository, commit, default_branch)
    return {"kind": "verified_commit", "repository": repository, "commit": commit, "tree": tree,
            "version": version, "verifier_sha": main_commit, "default_branch": default_branch,
            "ci": ci, "checked_at": datetime.now(timezone.utc).isoformat()}


def gh_call(method, endpoint, payload=None):
    command = ["gh", "api", "-X", method, endpoint] + (["--input", "-"] if payload is not None else [])
    return subprocess.run(command, input=None if payload is None else json.dumps(payload),
                          capture_output=True, text=True, timeout=120)


def gh_json(result, what):
    if result.returncode != 0:
        raise ValueError(f"GitHub {what} failed: {result.stderr.strip()}")
    try:
        value = json.loads(result.stdout)
    except ValueError as error:
        raise ValueError(f"GitHub {what} returned invalid JSON") from error
    if not isinstance(value, dict):
        raise ValueError(f"GitHub {what} returned an unexpected response")
    return value


def read_tag(gh, repository, tag):
    """The ref object of `tag`, or None when it does not exist (404 only)."""
    result = gh("GET", f"repos/{repository}/git/ref/tags/{tag}")
    if result.returncode != 0 and "HTTP 404" in result.stderr:
        return None
    return gh_json(result, "tag lookup").get("object")


def require_tag_matches(gh, repository, tag, commit, ref_object):
    """An existing tag is accepted only if it is an annotated tag of exactly `commit`."""
    if not isinstance(ref_object, dict) or ref_object.get("type") != "tag":
        raise ValueError(f"{tag} exists but is not an annotated tag; it is never moved")
    annotated = gh_json(gh("GET", f"repos/{repository}/git/tags/{ref_object.get('sha')}"), "tag object lookup")
    target = annotated.get("object")
    if (annotated.get("tag") != tag or not isinstance(target, dict)
            or target.get("type") != "commit" or target.get("sha") != commit):
        raise ValueError(f"{tag} exists and does not identify the verified commit; it is never moved")


def publish_tag(proof_path, gh=gh_call):
    proof = json.loads(Path(proof_path).read_text(encoding="utf-8"))
    repository, commit, version = proof.get("repository"), proof.get("commit"), proof.get("version")
    if (proof.get("kind") != "verified_commit" or not isinstance(repository, str)
            or not REPOSITORY.fullmatch(repository) or not isinstance(commit, str)
            or not COMMIT_SHA.fullmatch(commit) or not isinstance(version, str)
            or not SEMVER.fullmatch(version)):
        raise ValueError("release proof is not a verified commit")
    tag = "v" + version
    existing = read_tag(gh, repository, tag)
    if existing is not None:
        require_tag_matches(gh, repository, tag, commit, existing)
        return "exists"
    annotated = gh_json(gh("POST", f"repos/{repository}/git/tags", {
        "tag": tag, "message": f"DaVinci {version}; green full CI verified",
        "object": commit, "type": "commit"}), "tag creation")
    created = gh("POST", f"repos/{repository}/git/refs", {"ref": f"refs/tags/{tag}", "sha": annotated.get("sha")})
    if created.returncode == 0:
        return "created"
    if "HTTP 422" not in created.stderr:
        raise ValueError(f"GitHub tag ref creation failed: {created.stderr.strip()}")
    # Lost a race: the ref now exists. Accept it only if it is the identical tag.
    raced = read_tag(gh, repository, tag)
    if raced is None:
        raise ValueError(f"GitHub rejected tag ref creation: {created.stderr.strip()}")
    require_tag_matches(gh, repository, tag, commit, raced)
    return "exists"


def preflight(repo, require_tag=False, repository="J12003LPZ/davinci"):
    repo = Path(repo).resolve()
    source = source_identity(repo)
    version = product_version(repo)
    tag = None
    if require_tag:
        tag = "v" + version
        if git(repo, "rev-parse", "--verify", f"refs/tags/{tag}^{{commit}}") != source["source_sha"]:
            raise ValueError("installation requires the product version tag at the exact green commit")
    ci = fetch_ci(repository, source["source_sha"])
    require_same_source(repo, source)
    return {"repo": str(repo), "source": source, "version": version,
            "release_tag": tag, "ci": ci, "checked_at": datetime.now(timezone.utc).isoformat()}


def make_identity(binary, source, ci, tag, version="1.1.7", require_tag=True, build_features=()):
    if not isinstance(build_features, (list, tuple)) or any(
            not isinstance(feature, str) or not feature or feature.split("/")[-1] == "test-fixtures"
            for feature in build_features):
        raise ValueError("fixture-enabled builds are not release or benchmark artifacts")
    features = sorted(set(build_features))
    if source.get("source_clean") is not True or source.get("dirty_diff_hash") != CLEAN_DIFF_HASH:
        raise ValueError("binary identity requires clean committed source")
    validate_ci(ci, ci.get("repository"), source.get("source_sha", ""))
    if require_tag and tag != "v" + version:
        raise ValueError("installed binary must map to the product's version tag")
    return {"schema_version": 3, **source, "release_tag": tag,
            "repository": ci["repository"], "ci_run": ci["ci_run"], "ci_url": ci["ci_url"],
            "ci_verified": True, "ci": ci, "ci_evidence_sha256": hashlib.sha256(json.dumps(ci, sort_keys=True).encode()).hexdigest(),
            "binary_sha256": file_hash(binary), "build_features": features,
            "build_platform": platform.platform(),
            "built_at": datetime.now(timezone.utc).isoformat()}


def record_identity(proof, binary, output, require_tag=True):
    proof = json.loads(Path(proof).read_text(encoding="utf-8"))
    require_same_source(proof["repo"], proof["source"])
    identity = make_identity(binary, proof["source"], proof["ci"], proof["release_tag"], proof["version"], require_tag=require_tag)
    Path(output).write_text(json.dumps(identity, indent=2) + "\n", encoding="utf-8")
    return identity


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    before = commands.add_parser("preflight")
    before.add_argument("--repo", required=True)
    before.add_argument("--require-tag", action="store_true")
    before.add_argument("--output", required=True)
    after = commands.add_parser("record")
    after.add_argument("--proof", required=True)
    after.add_argument("--binary", required=True)
    after.add_argument("--output", required=True)
    after.add_argument("--benchmark", action="store_true", help="green committed benchmark, not a tagged installation")
    snapshot = commands.add_parser("snapshot", help="local exported-source evidence; not release provenance")
    snapshot.add_argument("--repo", required=True)
    snapshot.add_argument("--output", required=True)
    local = commands.add_parser("record-local", help="bind a local build to an unchanged snapshot")
    local.add_argument("--repo", required=True)
    local.add_argument("--snapshot", required=True)
    local.add_argument("--binary", required=True)
    local.add_argument("--configuration", required=True, help="JSON configuration; only its digest is recorded")
    local.add_argument("--feature", action="append", default=[])
    local.add_argument("--output", required=True)
    verify = commands.add_parser("verify-commit", help="prove a candidate commit without checking it out")
    verify.add_argument("--repo", required=True)
    verify.add_argument("--commit", required=True)
    verify.add_argument("--repository", required=True)
    verify.add_argument("--default-branch", required=True)
    verify.add_argument("--main-commit", required=True)
    verify.add_argument("--expected-version", required=True)
    verify.add_argument("--output", required=True)
    verify.add_argument("--github-output")
    publish = commands.add_parser("publish-tag", help="create the annotated version tag through the GitHub API")
    publish.add_argument("--proof", required=True)
    args = parser.parse_args()
    try:
        if args.command == "verify-commit":
            proof = verify_commit(args.repo, args.commit, args.repository, args.default_branch,
                                  args.main_commit, args.expected_version)
            Path(args.output).write_text(json.dumps(proof, indent=2) + "\n", encoding="utf-8")
            if args.github_output:
                with Path(args.github_output).open("a", encoding="utf-8") as output:
                    output.write(f"sha={proof['commit']}\nversion={proof['version']}\n"
                                 f"verifier_sha={proof['verifier_sha']}\n")
            print(f"Verified {proof['commit']} v{proof['version']}; green CI: {proof['ci']['ci_url']}")
        elif args.command == "publish-tag":
            print(f"Version tag {publish_tag(args.proof)}.")
        elif args.command in {"snapshot", "record-local"}:
            # Evidence must not become a changing input to its own identity.
            if Path(args.output).resolve().is_relative_to(Path(args.repo).resolve()):
                raise ValueError("local evidence output must be outside the source directory")
            if args.command == "snapshot":
                identity = export_snapshot(args.repo)
            else:
                identity = local_build_identity(
                    args.repo, args.binary, json.loads(Path(args.snapshot).read_text(encoding="utf-8")),
                    json.loads(Path(args.configuration).read_text(encoding="utf-8")), args.feature)
            Path(args.output).write_text(json.dumps(identity, indent=2) + "\n", encoding="utf-8")
            print(f"Recorded local evidence (not release eligible): {args.output}")
        elif args.command == "preflight":
            proof = preflight(args.repo, args.require_tag)
            Path(args.output).write_text(json.dumps(proof, indent=2) + "\n", encoding="utf-8")
            print(f"Green CI: {proof['ci']['ci_url']}; source: {proof['source']['source_sha']}")
        else:
            record_identity(args.proof, args.binary, args.output, require_tag=not args.benchmark)
            print(f"Recorded installation identity: {args.output}")
    except (ValueError, OSError, subprocess.SubprocessError, KeyError) as error:
        parser.exit(1, f"davinci release gate: {error}\n")


if __name__ == "__main__":
    main()
