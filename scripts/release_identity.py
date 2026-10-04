#!/usr/bin/env python3
"""Bind installed/benchmarked bytes to clean source and completed green CI.

Only the preflight command contacts GitHub, through the user's existing gh
authentication. Export snapshots and local identities do not satisfy release gates.
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


def git(repo, *args):
    result = subprocess.run(["git", *args], cwd=repo, check=True, capture_output=True, text=True)
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


def validate_ci(record, repository, sha):
    if (not isinstance(record, dict) or record.get("repository") != repository
            or record.get("source_sha") != sha or not re.fullmatch(r"[0-9a-f]{40}", sha)
            or record.get("event") != "push"
            or record.get("workflow_path") != ".github/workflows/ci.yml"
            or type(record.get("ci_run")) is not int or record["ci_run"] <= 0
            or record.get("ci_url") != f"https://github.com/{repository}/actions/runs/{record['ci_run']}"):
        raise ValueError("CI proof is not bound to the exact repository and source commit")
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


def fetch_ci(repository, sha):
    if not re.fullmatch(r"[A-Za-z0-9_.-]+/[A-Za-z0-9_.-]+", repository):
        raise ValueError("invalid repository name")
    prefix = f"repos/{repository}/actions"
    # PR workflows check out a synthetic merge by default; head_sha alone
    # cannot prove these exact source bytes were tested. Require push CI.
    runs = [run for page in api_pages(f"{prefix}/workflows/ci.yml/runs?head_sha={sha}&event=push&per_page=100")
            for run in page.get("workflow_runs", [])]
    run = latest_run(runs, sha)
    jobs = [job for page in api_pages(f"{prefix}/runs/{run['id']}/jobs?filter=latest&per_page=100")
            for job in page.get("jobs", [])]
    lint = latest_run([item for page in api_pages(f"{prefix}/workflows/workflow-lint.yml/runs?head_sha={sha}&event=push&per_page=100")
                       for item in page.get("workflow_runs", [])], sha)
    record = {"repository": repository, "source_sha": sha, "ci_run": run["id"],
              "ci_url": run["html_url"], "workflow_path": run.get("path"),
              "status": run["status"], "conclusion": run["conclusion"],
              "event": run["event"],
              "jobs": [{key: job[key] for key in ("name", "status", "conclusion")} for job in jobs],
              "workflow_lint": {"run_id": lint["id"], "status": lint["status"], "conclusion": lint["conclusion"]}}
    validate_ci(record, repository, sha)
    return record


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


def make_identity(binary, source, ci, tag, version="1.0.71", require_tag=True, build_features=()):
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
    args = parser.parse_args()
    try:
        if args.command in {"snapshot", "record-local"}:
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
