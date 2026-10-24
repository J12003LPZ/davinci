"""Offline release provenance gates; every CI record is a synthetic fixture."""
import importlib.util
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SPEC = importlib.util.spec_from_file_location("release_identity", Path(__file__).parents[2] / "release_identity.py")
release_identity = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release_identity)


class ReleaseIdentityTests(unittest.TestCase):
    def test_harness_snapshot_includes_nested_source_named_target(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "src/target").mkdir(parents=True)
            (root / "src/target/mod.rs").write_text("source")
            self.assertIn("src/target/mod.rs", release_identity.export_snapshot(root)["files"])

    def test_harness_local_identity_cli_is_separate_from_release(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            repo = root / "source"
            repo.mkdir()
            (repo / "main.rs").write_text("fixture source")
            binary = root / "davinci.exe"
            binary.write_bytes(b"fixture binary")
            config = root / "config.json"
            config.write_text('{"mode":"fixture"}')
            snapshot, identity = root / "snapshot.json", root / "identity.json"
            for args in (["snapshot", "--repo", str(repo), "--output", str(snapshot)],
                         ["record-local", "--repo", str(repo), "--snapshot", str(snapshot),
                          "--binary", str(binary), "--configuration", str(config),
                          "--feature", "test-fixtures", "--output", str(identity)]):
                result = subprocess.run([sys.executable, SPEC.origin, *args], capture_output=True, text=True)
                self.assertEqual(result.returncode, 0, result.stderr)
            record = json.loads(identity.read_text())
            self.assertFalse(record["release_eligible"])
            self.assertEqual(record["build_features"], ["test-fixtures"])
            self.assertIsNone(record["source"]["source_sha"])

    def test_harness_export_snapshot_never_invents_git_revision(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "Cargo.toml").write_text('[workspace]\n')
            (root / "src").mkdir()
            source = root / "src/main.rs"
            source.write_text("old")
            (root / ".env").write_text("PRIVATE=not-source")
            (root / "target").mkdir()
            (root / "target/output").write_text("generated")
            before = release_identity.export_snapshot(root)
            self.assertEqual(before["source_kind"], "exported_snapshot")
            self.assertIsNone(before["source_sha"])
            self.assertEqual(set(before["files"]), {"Cargo.toml", "src/main.rs"})
            stamp = source.stat()
            source.write_text("new")
            os.utime(source, ns=(stamp.st_atime_ns, stamp.st_mtime_ns))
            self.assertNotEqual(before["snapshot_sha256"], release_identity.export_snapshot(root)["snapshot_sha256"])
            with self.assertRaisesRegex(ValueError, "Git|committed"):
                release_identity.source_identity(root)

    def test_harness_local_build_identity_checks_source_and_configuration(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            (root / "src").mkdir()
            (root / "src/main.rs").write_text("source")
            (root / "target").mkdir()
            binary = root / "target/davinci.exe"
            binary.write_bytes(b"fixture")
            before = release_identity.export_snapshot(root)
            identity = release_identity.local_build_identity(
                root, binary, before, {"mode": "offline"}, ["test-fixtures"])
            self.assertEqual(identity["binary_sha256"], release_identity.file_hash(binary))
            self.assertEqual(identity["launch_path"], str(binary.resolve()))
            self.assertEqual(identity["build_features"], ["test-fixtures"])
            self.assertFalse(identity["release_eligible"])
            self.assertNotIn("mode", identity)
            (root / "src/main.rs").write_text("changed")
            with self.assertRaisesRegex(ValueError, "source changed"):
                release_identity.local_build_identity(root, binary, before, {}, [])

    def evidence(self):
        sha = "a" * 40
        return sha, {"repository": "fixture/repo", "source_sha": sha, "ci_run": 12,
                     "ci_url": "https://github.com/fixture/repo/actions/runs/12",
                     "status": "completed", "conclusion": "success", "event": "push",
                     "workflow_path": ".github/workflows/ci.yml",
                     "jobs": [{"name": name, "status": "completed", "conclusion": "success"}
                              for name in sorted(release_identity.EXPECTED_CI_JOBS)],
                     "workflow_lint": {"status": "completed", "conclusion": "success", "run_id": 13}}

    def test_green_ci_requires_exact_commit_and_full_jobs(self):
        sha, record = self.evidence()
        release_identity.validate_ci(record, "fixture/repo", sha)
        for patch in ({"source_sha": "b" * 40}, {"status": "in_progress"},
                      {"conclusion": "failure"}, {"jobs": record["jobs"][:-1]},
                      {"jobs": [dict(job, conclusion="skipped") for job in record["jobs"]]},
                      {"workflow_lint": None}, {"repository": "other/repo"}, {"event": "pull_request"}):
            with self.subTest(patch=patch), self.assertRaises(ValueError):
                release_identity.validate_ci(dict(record, **patch), "fixture/repo", sha)

    def test_identity_binds_tag_ci_source_and_binary_bytes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            binary = root / "davinci"
            binary.write_bytes(b"synthetic binary")
            sha, ci = self.evidence()
            source = {"source_sha": sha, "source_tree": "b" * 40, "source_clean": True,
                      "dirty_diff_hash": release_identity.CLEAN_DIFF_HASH}
            identity = release_identity.make_identity(binary, source, ci, "v1.1.7")
            self.assertEqual(identity["schema_version"], 3)
            self.assertEqual(identity["release_tag"], "v1.1.7")
            self.assertEqual(identity["ci_run"], 12)
            self.assertEqual(identity["binary_sha256"], release_identity.file_hash(binary))
            for bad in (None, "v0.0.0", "1.0.71"):
                with self.subTest(tag=bad), self.assertRaises(ValueError):
                    release_identity.make_identity(binary, source, ci, bad, version="1.0.71")

    def test_dirty_source_and_source_changed_during_build_are_rejected(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for args in (["init", "-q"], ["config", "user.name", "Fixture"], ["config", "user.email", "fixture@example.invalid"]):
                subprocess.run(["git", *args], cwd=root, check=True, capture_output=True)
            (root / "a.txt").write_text("before")
            subprocess.run(["git", "add", "a.txt"], cwd=root, check=True)
            subprocess.run(["git", "commit", "-qm", "fixture"], cwd=root, check=True)
            before = release_identity.source_identity(root)
            release_identity.require_same_source(root, before)
            (root / "a.txt").write_text("after")
            with self.assertRaises(ValueError):
                release_identity.require_same_source(root, before)

    def test_latest_failed_ci_cannot_fall_back_to_older_success(self):
        sha, _ = self.evidence()
        old = {"id": 1, "head_sha": sha, "status": "completed", "conclusion": "success", "created_at": "2026-09-29T00:00:00Z"}
        new = dict(old, id=2, conclusion="failure", created_at="2026-09-30T00:00:00Z")
        self.assertEqual(release_identity.latest_run([old, new], sha)["id"], 2)
        with self.assertRaises(ValueError):
            release_identity.latest_run([old], "b" * 40)


class CommitVerificationTests(unittest.TestCase):
    """verify-commit and publish-tag against a throwaway repository and a fake gh."""

    REPOSITORY = "fixture/repo"
    CARGO = '[workspace.package]\nversion = "{}"\n'

    def setUp(self):
        self.temporary = tempfile.TemporaryDirectory()
        self.addCleanup(self.temporary.cleanup)
        self.repo = Path(self.temporary.name) / "repo"
        self.repo.mkdir()
        self.git("init", "-q", "-b", "main")

    def git(self, *args, **options):
        env = {**os.environ, "GIT_AUTHOR_NAME": "Fixture", "GIT_AUTHOR_EMAIL": "fixture@example.invalid",
               "GIT_COMMITTER_NAME": "Fixture", "GIT_COMMITTER_EMAIL": "fixture@example.invalid",
               **options.pop("env", {})}
        result = subprocess.run(["git", *args], cwd=self.repo, check=True, capture_output=True,
                                text=True, env=env, **options)
        return result.stdout.strip()

    def commit(self, files, parent=None):
        """Commit `files` ({path: (mode, text)}) with plumbing; the worktree is not touched."""
        index = str(Path(self.temporary.name) / "index")
        if os.path.exists(index):
            os.remove(index)
        env = {"GIT_INDEX_FILE": index}
        for path, (mode, text) in files.items():
            blob = self.git("hash-object", "-w", "--stdin", input=text)
            self.git("update-index", "--add", "--cacheinfo", f"{mode},{blob},{path}", env=env)
        tree = self.git("write-tree", env=env)
        args = ["commit-tree", tree, "-m", "fixture"] + (["-p", parent] if parent else [])
        return self.git(*args)

    def cargo(self, version):
        return ("100644", self.CARGO.format(version))

    def history(self):
        """candidate B (hostile files, 1.2.3) <- trusted A (checked out, 1.2.4)."""
        marker = self.repo / "pwned"
        hostile = f"import pathlib\npathlib.Path({str(marker)!r}).write_text('ran')\n"
        candidate = self.commit({"Cargo.toml": self.cargo("1.2.3"),
                                 "scripts/release_identity.py": ("100644", hostile)})
        trusted = self.commit({"Cargo.toml": self.cargo("1.2.4")}, parent=candidate)
        self.git("update-ref", "refs/heads/main", trusted)
        self.git("checkout", "-q", "main")
        for hook in ("post-checkout", "post-merge", "post-rewrite"):
            path = self.repo / ".git" / "hooks" / hook
            path.write_text(f"#!/bin/sh\necho ran > '{marker.as_posix()}'\n")
            path.chmod(0o755)
        return candidate, trusted, marker

    def verify(self, commit, trusted, version="1.2.3", **options):
        calls = options.pop("calls", [])

        def fetch(repository, sha, branch=None):
            calls.append((repository, sha, branch))
            return {"ci_url": "https://github.com/fixture/repo/actions/runs/12", "head_branch": branch}

        return release_identity.verify_commit(self.repo, commit, self.REPOSITORY, "main", trusted,
                                              version, fetch=fetch, **options)

    def test_candidate_is_read_without_checkout_or_execution(self):
        candidate, trusted, marker = self.history()
        calls = []
        proof = self.verify(candidate, trusted, calls=calls)
        self.assertEqual(proof["commit"], candidate)
        self.assertEqual(proof["verifier_sha"], trusted)
        self.assertEqual(proof["version"], "1.2.3")  # the candidate blob, not the 1.2.4 worktree
        self.assertEqual(proof["tree"], self.git("rev-parse", f"{candidate}^{{tree}}"))
        self.assertEqual(calls, [(self.REPOSITORY, candidate, "main")])
        self.assertEqual(self.git("rev-parse", "HEAD"), trusted)
        self.assertEqual(self.git("status", "--porcelain"), "")
        self.assertIn("1.2.4", (self.repo / "Cargo.toml").read_text())
        self.assertFalse((self.repo / "scripts").exists())
        self.assertFalse(marker.exists())

    def test_malformed_commits_are_rejected_before_git_or_ci(self):
        candidate, trusted, _ = self.history()
        calls = []
        for bad in (candidate[:12], candidate.upper(), "--output=x", f"{candidate} ", f"{candidate}\n",
                    " " + candidate, "", "HEAD", "main"):
            with self.subTest(commit=bad), self.assertRaises(ValueError):
                self.verify(bad, trusted, calls=calls)
        self.assertEqual(calls, [])

    def test_non_ancestor_candidate_is_rejected(self):
        candidate, trusted, _ = self.history()
        stray = self.commit({"Cargo.toml": self.cargo("1.2.3")})
        calls = []
        with self.assertRaisesRegex(ValueError, "ancestor"):
            self.verify(stray, trusted, calls=calls)
        with self.assertRaisesRegex(ValueError, "ancestor"):
            self.verify(trusted, candidate, version="1.2.4", calls=calls)  # descendant of main
        self.assertEqual(calls, [])

    def test_missing_object_and_non_commit_are_rejected(self):
        candidate, trusted, _ = self.history()
        with self.assertRaises(ValueError):
            self.verify("f" * 40, trusted)
        with self.assertRaises(ValueError):
            self.verify(self.git("rev-parse", f"{candidate}^{{tree}}"), trusted)

    def test_version_must_match_and_be_strict_semver(self):
        candidate, trusted, _ = self.history()
        with self.assertRaisesRegex(ValueError, "does not match"):
            self.verify(candidate, trusted, version="1.2.4")
        for bad in ("1.2", "01.2.3", "v1.2.3", "1.2.3\n", " 1.2.3", ""):
            with self.subTest(version=bad), self.assertRaises(ValueError):
                self.verify(candidate, trusted, version=bad)
        loose = self.commit({"Cargo.toml": self.cargo("1.2")}, parent=candidate)
        top = self.commit({"Cargo.toml": self.cargo("1.2.4")}, parent=loose)
        with self.assertRaisesRegex(ValueError, "semver"):
            self.verify(loose, top, version="1.2")

    def test_cargo_toml_must_be_a_regular_committed_file(self):
        base = self.commit({"README": ("100644", "x")})
        link = self.commit({"Cargo.toml": ("120000", "elsewhere/Cargo.toml")}, parent=base)
        executable = self.commit({"Cargo.toml": ("100755", self.CARGO.format("1.2.3"))}, parent=link)
        absent = self.commit({"README": ("100644", "y")}, parent=executable)
        nested = self.commit({"sub/Cargo.toml": ("100644", self.CARGO.format("1.2.3"))}, parent=absent)
        garbage = self.commit({"Cargo.toml": ("100644", "not = [toml")}, parent=nested)
        no_version = self.commit({"Cargo.toml": ("100644", "[workspace]\n")}, parent=garbage)
        self.git("update-ref", "refs/heads/main", no_version)
        for candidate in (base, link, executable, absent, nested, garbage, no_version):
            with self.subTest(candidate=candidate), self.assertRaises(ValueError):
                self.verify(candidate, no_version)

    def test_replace_refs_cannot_swap_the_candidate_manifest(self):
        candidate, trusted, _ = self.history()
        forged = self.commit({"Cargo.toml": self.cargo("9.9.9")})
        self.git("replace", candidate, forged)
        self.assertEqual(self.verify(candidate, trusted)["version"], "1.2.3")

    def test_ci_must_be_push_ci_of_the_default_branch(self):
        pages = {}

        def api(endpoint):
            return pages[endpoint.split("/workflows/")[1].split("/")[0]]

        sha, record = ReleaseIdentityTests.evidence(self)
        run = {"id": 12, "head_sha": sha, "html_url": record["ci_url"], "path": record["workflow_path"],
               "status": "completed", "conclusion": "success", "event": "push",
               "created_at": "2026-10-01T00:00:00Z", "head_branch": "main"}
        jobs = {"jobs": record["jobs"]}
        lint = dict(run, id=13, path=".github/workflows/workflow-lint.yml")
        original = release_identity.api_pages
        self.addCleanup(setattr, release_identity, "api_pages", original)

        def serve(runs, lint_runs):
            def fake(endpoint):
                if "/jobs" in endpoint:
                    return [jobs]
                return [{"workflow_runs": runs if "/ci.yml/" in endpoint else lint_runs}]
            release_identity.api_pages = fake

        serve([run], [lint])
        self.assertEqual(release_identity.fetch_ci("fixture/repo", sha, "main")["head_branch"], "main")
        # Green CI that ran on another branch is not evidence for main.
        serve([dict(run, head_branch="feature")], [lint])
        with self.assertRaisesRegex(ValueError, "no CI run"):
            release_identity.fetch_ci("fixture/repo", sha, "main")
        serve([run], [dict(lint, head_branch="feature")])
        with self.assertRaisesRegex(ValueError, "no CI run"):
            release_identity.fetch_ci("fixture/repo", sha, "main")
        serve([dict(run, event="pull_request")], [lint])
        with self.assertRaises(ValueError):
            release_identity.fetch_ci("fixture/repo", sha, "main")
        with self.assertRaisesRegex(ValueError, "push to main"):
            release_identity.validate_ci(dict(record, head_branch="feature"), "fixture/repo", sha, "main")
        for branch in ("-x", "a b", "", "a;b"):
            with self.subTest(branch=branch), self.assertRaises(ValueError):
                release_identity.fetch_ci("fixture/repo", sha, branch)

    def test_cli_writes_proof_and_outputs_only_after_checks_pass(self):
        candidate, trusted, marker = self.history()
        out = Path(self.temporary.name)
        script = Path(release_identity.__file__)

        def run(commit, version):
            return subprocess.run(
                [sys.executable, str(script), "verify-commit", "--repo", str(self.repo), "--commit", commit,
                 "--repository", self.REPOSITORY, "--default-branch", "main", "--main-commit", trusted,
                 "--expected-version", version, "--output", str(out / "proof.json"),
                 "--github-output", str(out / "github-output")], capture_output=True, text=True)

        failed = run(candidate, "1.2.4")
        self.assertEqual(failed.returncode, 1)
        self.assertIn("does not match", failed.stderr)
        self.assertFalse((out / "github-output").exists())
        self.assertFalse((out / "proof.json").exists())
        self.assertFalse(marker.exists())

    def proof(self, **changes):
        sha = "c" * 40
        path = Path(self.temporary.name) / "release-proof.json"
        path.write_text(json.dumps({"kind": "verified_commit", "repository": self.REPOSITORY,
                                    "commit": sha, "version": "1.2.3", **changes}))
        return path, sha

    class FakeGh:
        """Scripted `gh api`: responses per (method, endpoint), recorded calls."""
        def __init__(self, responses):
            self.responses, self.calls = responses, []

        def __call__(self, method, endpoint, payload=None):
            self.calls.append((method, endpoint, payload))
            queue = self.responses.get((method, endpoint))
            if not queue:
                raise AssertionError(f"unexpected gh call {method} {endpoint}")
            code, body, error = queue.pop(0) if len(queue) > 1 else queue[0]
            return subprocess.CompletedProcess([], code, json.dumps(body), error)

    NOT_FOUND = (1, {"message": "Not Found"}, "gh: Not Found (HTTP 404)")

    def endpoints(self):
        base = f"repos/{self.REPOSITORY}/git"
        return f"{base}/ref/tags/v1.2.3", f"{base}/tags", f"{base}/refs", f"{base}/tags/tagsha"

    def test_publish_tag_creates_annotated_tag_then_ref(self):
        proof, sha = self.proof()
        ref, tags, refs, _ = self.endpoints()
        gh = self.FakeGh({("GET", ref): [self.NOT_FOUND], ("POST", tags): [(0, {"sha": "tagsha"}, "")],
                          ("POST", refs): [(0, {}, "")]})
        self.assertEqual(release_identity.publish_tag(proof, gh=gh), "created")
        self.assertEqual([call[:2] for call in gh.calls], [("GET", ref), ("POST", tags), ("POST", refs)])
        self.assertEqual(gh.calls[1][2], {"tag": "v1.2.3", "message": "DaVinci 1.2.3; green full CI verified",
                                          "object": sha, "type": "commit"})
        self.assertEqual(gh.calls[2][2], {"ref": "refs/tags/v1.2.3", "sha": "tagsha"})

    def test_publish_tag_accepts_only_a_matching_existing_tag(self):
        proof, sha = self.proof()
        ref, _, _, annotated = self.endpoints()
        exists = {("GET", ref): [(0, {"object": {"type": "tag", "sha": "tagsha"}}, "")]}

        def tag_object(**changes):
            body = {"tag": "v1.2.3", "object": {"type": "commit", "sha": sha}, **changes}
            return {("GET", annotated): [(0, body, "")]}

        gh = self.FakeGh({**exists, **tag_object()})
        self.assertEqual(release_identity.publish_tag(proof, gh=gh), "exists")
        self.assertTrue(all(call[0] == "GET" for call in gh.calls))
        conflicts = [tag_object(object={"type": "commit", "sha": "d" * 40}), tag_object(tag="v9.9.9"),
                     tag_object(object={"type": "tag", "sha": sha}),
                     {("GET", annotated): [(0, {"tag": "v1.2.3", "object": None}, "")]}]
        for responses in conflicts:
            with self.subTest(responses=responses), self.assertRaisesRegex(ValueError, "never moved"):
                release_identity.publish_tag(proof, gh=self.FakeGh({**exists, **responses}))
        lightweight = self.FakeGh({("GET", ref): [(0, {"object": {"type": "commit", "sha": sha}}, "")]})
        with self.assertRaisesRegex(ValueError, "not an annotated tag"):
            release_identity.publish_tag(proof, gh=lightweight)
        self.assertEqual([call[0] for call in lightweight.calls], ["GET"])

    def test_publish_tag_lookup_errors_other_than_404_fail_without_creating(self):
        proof, _ = self.proof()
        ref, _, _, _ = self.endpoints()
        for failure in ((1, {}, "gh: Server Error (HTTP 500)"), (1, {}, "gh: Forbidden (HTTP 403)")):
            gh = self.FakeGh({("GET", ref): [failure]})
            with self.subTest(failure=failure), self.assertRaisesRegex(ValueError, "lookup failed"):
                release_identity.publish_tag(proof, gh=gh)
            self.assertEqual([call[0] for call in gh.calls], ["GET"])

    def test_publish_tag_race_is_accepted_only_for_the_same_commit(self):
        proof, sha = self.proof()
        ref, tags, refs, annotated = self.endpoints()
        raced = (1, {"message": "Reference already exists"}, "gh: Reference already exists (HTTP 422)")
        found = (0, {"object": {"type": "tag", "sha": "tagsha"}}, "")

        def responses(target):
            return {("GET", ref): [self.NOT_FOUND, found], ("POST", tags): [(0, {"sha": "tagsha"}, "")],
                    ("POST", refs): [raced],
                    ("GET", annotated): [(0, {"tag": "v1.2.3", "object": {"type": "commit", "sha": target}}, "")]}

        self.assertEqual(release_identity.publish_tag(proof, gh=self.FakeGh(responses(sha))), "exists")
        with self.assertRaisesRegex(ValueError, "never moved"):
            release_identity.publish_tag(proof, gh=self.FakeGh(responses("d" * 40)))
        # A 422 with no tag behind it is a real failure, not a race.
        missing = responses(sha)
        missing[("GET", ref)] = [self.NOT_FOUND]
        with self.assertRaisesRegex(ValueError, "rejected"):
            release_identity.publish_tag(proof, gh=self.FakeGh(missing))

    def test_publish_tag_never_force_updates_and_validates_the_proof(self):
        proof, _ = self.proof()
        ref, tags, refs, _ = self.endpoints()
        other = (1, {}, "gh: Server Error (HTTP 500)")
        gh = self.FakeGh({("GET", ref): [self.NOT_FOUND], ("POST", tags): [(0, {"sha": "tagsha"}, "")],
                          ("POST", refs): [other]})
        with self.assertRaisesRegex(ValueError, "ref creation failed"):
            release_identity.publish_tag(proof, gh=gh)
        self.assertEqual({call[0] for call in gh.calls}, {"GET", "POST"})
        for changes in ({"commit": "ABC"}, {"version": "1.2"}, {"repository": "a b"}, {"kind": "other"}):
            bad, _ = self.proof(**changes)
            with self.subTest(changes=changes), self.assertRaisesRegex(ValueError, "verified commit"):
                release_identity.publish_tag(bad, gh=self.FakeGh({}))


if __name__ == "__main__":
    unittest.main()
