"""Apply reviewable TUI patches, then publish tested Git objects (never refs).

This bridge supports GitHub-only editing of large existing Rust modules. Staging
patches are removed from the resulting tree; production source is stored as
ordinary files. A separate, explicit GitHub action must commit/update the ref.
"""
import argparse
import base64
import hashlib
import json
import os
from pathlib import Path
import subprocess
import urllib.request

ROOT = Path.cwd().resolve()
PATCH = ".github/terminal-ui/candidate.patch"
BRANCH = "refs/heads/J12003LPZ/claude-terminal-implementation-20260921"


def git(*args):
    return subprocess.check_output(["git", *args], text=True).strip()


def checked_path(name):
    path = Path(name)
    if not name.startswith("crates/davinci-tui/") or ".." in path.parts:
        raise ValueError(f"Candidate path is outside the TUI: {name}")
    resolved = (ROOT / path).resolve()
    if not resolved.is_relative_to(ROOT) or (ROOT / path).is_symlink():
        raise ValueError(f"Unsafe candidate path: {name}")
    return ROOT / path


def candidate_patches():
    patches = [PATCH]
    patches.extend(str(p) for p in sorted(Path(".github/terminal-ui").glob("candidate-*.patch")))
    for name in patches:
        if (ROOT / name).is_symlink() or not (ROOT / name).is_file():
            raise ValueError(f"Unsafe or missing patch: {name}")
    return patches


def apply():
    for patch in candidate_patches():
        entries = subprocess.check_output(
            ["git", "apply", "--numstat", "--", patch], text=True
        ).splitlines()
        if not entries:
            raise ValueError(f"Empty candidate patch: {patch}")
        for entry in entries:
            added, removed, name = entry.split("\t", 2)
            if not added.isdecimal() or not removed.isdecimal():
                raise ValueError("Binary patches are not supported")
            checked_path(name)
        subprocess.run(["git", "apply", "--check", "--index", "--", patch], check=True)
        subprocess.run(["git", "apply", "--index", "--", patch], check=True)
    files = git("diff", "--cached", "--name-only", "-z").split("\0")
    for name in files:
        if name:
            checked_path(name)
    (Path(os.environ["RUNNER_TEMP"]) / "terminal-paths.json").write_text(
        json.dumps([name for name in files if name]), encoding="utf-8"
    )


def api(method, endpoint, payload=None):
    repository = os.environ["GITHUB_REPOSITORY"]
    if repository != "J12003LPZ/davinci":
        raise ValueError("Unexpected repository")
    request = urllib.request.Request(
        f"https://api.github.com/repos/{repository}/{endpoint}",
        data=None if payload is None else json.dumps(payload).encode(),
        headers={
            "Authorization": "Bearer " + os.environ["GH_TOKEN"],
            "Accept": "application/vnd.github+json",
            "Content-Type": "application/json",
            "X-GitHub-Api-Version": "2022-11-28",
        },
        method=method,
    )
    with urllib.request.urlopen(request, timeout=60) as response:
        return json.load(response)


def publish():
    paths = json.loads((Path(os.environ["RUNNER_TEMP"]) / "terminal-paths.json").read_text())
    if not paths:
        raise ValueError("No source files to publish")
    elements, files = [], []
    for name in paths:
        path = checked_path(name)
        if not path.is_file():
            raise ValueError(f"Candidate deleted a source file: {name}")
        mode = git("ls-files", "--stage", "--", name).split()[0]
        if mode != "100644":
            raise ValueError(f"Unexpected source mode: {name}")
        data = path.read_bytes()
        data.decode("utf-8")
        blob = api("POST", "git/blobs", {
            "content": base64.b64encode(data).decode("ascii"), "encoding": "base64"
        })
        elements.append({"path": name, "mode": mode, "type": "blob", "sha": blob["sha"]})
        files.append({"path": name, "blob_sha": blob["sha"], "sha256": hashlib.sha256(data).hexdigest()})
    for patch in candidate_patches():
        elements.append({"path": patch, "mode": "100644", "type": "blob", "sha": None})
    parent = os.environ["GITHUB_SHA"]
    base = api("GET", f"git/commits/{parent}")["tree"]["sha"]
    tree = api("POST", "git/trees", {"base_tree": base, "tree": elements})
    result = {"parent_sha": parent, "tree_sha": tree["sha"], "files": files}
    (Path(os.environ["RUNNER_TEMP"]) / "terminal-candidate.json").write_text(
        json.dumps(result, indent=2) + "\n", encoding="utf-8"
    )
    print(json.dumps(result))


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("mode", choices=["apply", "publish"])
    args = parser.parse_args()
    if os.environ.get("GITHUB_REF") != BRANCH:
        raise SystemExit("Candidate assembly is limited to the dedicated implementation branch")
    {"apply": apply, "publish": publish}[args.mode]()
